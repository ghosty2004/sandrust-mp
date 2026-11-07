//! S→C RPC dispatch: one handler per RPC module (session, UI, remote players, vehicles, world); the player RPCs are
//! in `rpc_player`.

use std::time::Instant;

use samp_proto::rpcs::{Incoming, remote, session, ui, vehicle, world};

use super::Client;
use crate::script::{Arg, op};

impl Client {
    pub(super) fn on_rpc(&mut self, r: Incoming) {
        crate::crash::note(crate::crash::crumb::RPC, u16::from(r.id()), 0);
        crate::ui::on_rpc(&r);
        match r {
            Incoming::Session(s) => self.on_session_rpc(s),
            Incoming::Ui(u) => on_ui_rpc(u),
            Incoming::Remote(r) => self.on_remote_rpc(r),
            Incoming::Vehicle(v) => self.on_vehicle_rpc(v),
            Incoming::World(w) => self.on_world_rpc(w),
            Incoming::Player(p) => self.on_player_rpc(p),
            _ => {}
        }
    }

    fn on_session_rpc(&mut self, s: session::Incoming) {
        use session::Incoming as S;
        match s {
            // RPC 40 (gmx): the reset routine, state 11 until the server sends InitGame again.
            S::PlayerClose(_) => {
                let actions = self.netgame.on_server_restart();
                self.apply_actions(actions);
            }
            S::ClientCheckRequest(c) => self.behaviour.client_check(&mut self.net, &mut self.runner, &c),
            // RPC 92 / 150: opcode 0x052C / 0x03FD with (player 0, value) when the local player object exists
            // (docs/re/subsystems/session-misc.md).
            S::SetPlayerDrunkVisuals(v) => {
                if self.ped() != 0 {
                    self.runner.run(op::SET_PLAYER_DRUNKENNESS, &[Arg::Int(0), Arg::Int(v.value as i32)]);
                }
            }
            S::SetPlayerDrunkHandling(v) => {
                if self.ped() != 0 {
                    self.runner.run(op::RPC150_PLAYER_COMMAND, &[Arg::Int(0), Arg::Int(v.value as i32)]);
                }
            }
            S::SetVehicleTyreStatus(t) => self.vehicles.on_tyres(t.vehicle_id, t.tyres),
            S::SetVehicleParamsForPlayerOld(p) => {
                self.vehicles.on_old_params(p.vehicle_id, p.objective, p.doors_locked)
            }
            _ => {}
        }
    }

    fn on_remote_rpc(&mut self, r: remote::Incoming) {
        use remote::Incoming as R;
        // Colour table: PlayerJoin sets it only for a non-zero colour and an id below 1004 (the spawn of
        // PlayerStreamIn: players.rs).
        if let R::PlayerJoin(j) = &r
            && j.colour != 0
            && j.player_id < 1004
        {
            let (id, colour) = (j.player_id, j.colour);
            crate::worldhooks::with_shared(|s| {
                s.colours.get_or_insert_with(crate::ui::colours::PlayerColours::new).set(id, colour)
            });
        }
        let (streamed_in, quit) = match &r {
            R::PlayerStreamIn(s) => (Some(s.player_id), None),
            R::PlayerQuit(q) => (None, Some(q.player_id)),
            R::PlayerStreamOut(s) => {
                self.vehicles.reset_remote_vehicle(s.player_id);
                (None, None)
            }
            _ => (None, None),
        };
        match &r {
            R::PlayerJoin(j) => self.behaviour.forget_remote(j.player_id),
            R::PlayerQuit(q) => self.behaviour.forget_remote(q.player_id),
            R::TogglePlayerSpectating(t) => self.on_toggle_spectating(t.enable == 0),
            R::PlayerSpectatePlayer(p) => {
                self.behaviour.spectate_player(&self.players, p.player_id, p.gta_camera_mode())
            }
            R::PlayerSpectateVehicle(v) => {
                self.behaviour.spectate_vehicle(&self.vehicles, v.vehicle_id, v.gta_camera_mode())
            }
            _ => {}
        }
        self.players.on_rpc(&mut self.runner, r);
        if let Some(id) = streamed_in {
            self.vehicles.after_stream_in(&mut self.runner, &mut self.players, id);
        }
        if let Some(id) = quit {
            self.vehicles.forget_player(id);
        }
    }

    /// Spectating off with the class-selection flag (RPC 74 / F4) set leaves the spawned state for the class
    /// selection (the local tick then enters it and clears the flag); without the flag, leaving spectating spawns.
    fn on_toggle_spectating(&mut self, off: bool) {
        let spawn = self.behaviour.toggle_spectating(!off);
        if off && self.force_class_selection {
            crate::log!("client: spectating off with the class-selection flag -> class selection");
            self.spawned = false;
            self.dead = false;
            let count = self.class_count;
            self.enter_class_selection(Instant::now(), count);
            self.force_class_selection = false;
        } else if spawn {
            self.spawn();
        }
    }

    fn on_vehicle_rpc(&mut self, v: vehicle::Incoming) {
        if let vehicle::Incoming::StreamOutVehicle(s) = &v {
            self.surf.vehicle_streamed_out(s.vehicle_id);
        }
        self.vehicles.on_rpc(&mut self.runner, &mut self.players, v)
    }

    /// Objects (and their editing / attachments) go to the object pool, everything else to the entity pools.
    fn on_world_rpc(&mut self, w: world::Incoming) {
        use world::Incoming as W;
        if let W::DestroyObject(d) = &w {
            self.surf.object_destroyed(d.object_id);
        }
        match w {
            W::CreateObject(_)
            | W::SetObjectPosition(_)
            | W::SetObjectRotation(_)
            | W::DestroyObject(_)
            | W::MoveObject(_)
            | W::StopObject(_)
            | W::AttachObjectToPlayer(_)
            | W::SetPlayerObjectMaterial(_)
            | W::SetObjectNoCameraCollision(_)
            | W::SetPlayerAttachedObject(_)
            | W::PlayerBeginObjectSelect(_)
            | W::PlayerCancelObjectEdit(_)
            | W::PlayerBeginObjectEdit(_)
            | W::PlayerBeginAttachedObjectEdit(_) => {
                self.objects.on_rpc(&mut self.runner, &mut self.net, &self.players, w)
            }
            w => self.entities.on_rpc(&mut self.runner, w),
        }
    }
}

/// Chat and game text are logged; RPC 41 / 42 drive the audio streams (audio.rs).
fn on_ui_rpc(u: ui::Incoming) {
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    match u {
        ui::Incoming::SendClientMessage(m) => {
            crate::log!("[chat {:08x}] {}", m.colour, text(&m.message));
        }
        ui::Incoming::SendGameText(g) => crate::log!("[gametext] {}", text(&g.text)),
        ui::Incoming::PlayAudioStreamForPlayer(a) => crate::audio::on_play_rpc(&a),
        ui::Incoming::StopAudioStreamForPlayer(_) => {
            crate::audio::stop(false);
        }
        _ => {}
    }
}
