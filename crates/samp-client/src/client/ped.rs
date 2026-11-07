//! Operations on the local ped: lookup, teleport, heading, control, camera behind, skin, weapons, phone.

use super::Client;
use super::gta::{self, ped_in_vehicle, ped_in_world, poke};
use crate::game::{self, off};
use crate::patch::{Memory, ProcessMemory};
use crate::script::{Arg, op};

impl Client {
    pub(super) fn ped(&self) -> u32 {
        ProcessMemory.read_u32(game::CWORLD_PLAYERS + game::off::PLAYER_INFO_PED).unwrap_or(0)
    }

    pub(super) fn ped_handle(&self, ped: u32) -> Option<i32> {
        game::ped_handle(&ProcessMemory, ped).ok().flatten()
    }

    /// The local ped's primary task is the hands-up task. Mirrors samp.dll 0x100ACBA0.
    pub(super) fn hands_up(&self, ped: u32) -> bool {
        let m = ProcessMemory;
        if ped_in_vehicle(ped) || !ped_in_world(ped) {
            return false;
        }
        let intel = m.read_u32(ped + off::PED_INTELLIGENCE).unwrap_or(0);
        let task = if intel != 0 { m.read_u32(intel + off::INTELLIGENCE_PRIMARY_TASK).unwrap_or(0) } else { 0 };
        task != 0 && m.read_u32(task).ok() == Some(game::VTBL_TASK_HANDS_UP)
    }

    /// `GET_CURRENT_CHAR_WEAPON` on the local ped. Mirrors samp.dll 0x100ABC90.
    pub(super) fn current_weapon(&mut self, ped: u32) -> u8 {
        let Some(h) = self.ped_handle(ped) else { return 0 };
        self.runner.run(op::GET_CURRENT_CHAR_WEAPON, &[Arg::Int(h), Arg::Local(0)]);
        self.runner.local_i32(0) as u8
    }

    /// Phone flag + `TASK_USE_MOBILE_PHONE(ped, flag)`. Mirrors samp.dll 0x100AE0B0.
    pub(super) fn set_phone(&mut self, handle: i32, on: bool) {
        self.phone = on;
        self.runner.run(op::TASK_USE_MOBILE_PHONE, &[Arg::Int(handle), Arg::Int(i32::from(on))]);
    }

    /// `CPlayerPed::SetInitialState(false)`.
    pub(super) fn reset_ped_state(&mut self, ped: u32) {
        // SAFETY: thiscall on the live local CPlayerPed, main thread.
        unsafe {
            let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(game::PLAYER_PED_SET_INITIAL_STATE as usize);
            f(ped, 0);
        }
    }

    /// Entity `Teleport` virtual (docs/re/subsystems/player-state.md, "12 SetPlayerPosition").
    pub(super) fn teleport(&mut self, x: f32, y: f32, z: f32) {
        let ped = self.ped();
        if ped == 0 {
            return;
        }
        let vtbl = ProcessMemory.read_u32(ped + off::VTABLE).unwrap_or(0);
        let Ok(target) = ProcessMemory.read_u32(vtbl + off::VT_TELEPORT) else { return };
        // SAFETY: the ped's own virtual, thiscall (CVector by value, bool).
        unsafe {
            let f: extern "thiscall" fn(u32, f32, f32, f32, u32) = std::mem::transmute(target as usize);
            f(ped, x, y, z, 0);
        }
    }

    /// RPC 19 behaviour: current and goal heading, then SET_CHAR_HEADING.
    pub(super) fn set_facing(&mut self, deg: f32) {
        let ped = self.ped();
        if ped == 0 {
            return;
        }
        let r = game::heading_rad(deg);
        poke(ped + off::PED_HEADING_GOAL, r);
        poke(ped + off::PED_HEADING, r);
        if let Some(h) = self.ped_handle(ped) {
            self.runner.run(op::SET_CHAR_HEADING, &[Arg::Int(h), Arg::Float(deg)]);
        }
    }

    /// RPC 15 behaviour: player control and freeze; enabling re-sets an on-foot ped to its own position.
    /// Mirrors samp.dll 0x100AC7D0.
    pub(super) fn set_controllable(&mut self, enable: bool) {
        let ped = self.ped();
        let Some(h) = self.ped_handle(ped) else { return };
        self.runner.run(op::SET_PLAYER_CONTROL, &[Arg::Int(0), Arg::Int(i32::from(enable))]);
        self.runner.run(op::FREEZE_CHAR_POSITION, &[Arg::Int(h), Arg::Int(i32::from(!enable))]);
        let in_vehicle = ped_in_vehicle(ped);
        if enable
            && !in_vehicle
            && let Ok(Some(p)) = game::read_player(&ProcessMemory)
        {
            self.teleport(p.pos[0], p.pos[1], p.pos[2]);
        }
    }

    /// RPC 162 behaviour: the attachment ends too. Mirrors samp.dll 0x1009D7A0.
    pub(super) fn camera_behind_player(&mut self) {
        self.runner.run(op::CAMERA_PERSIST_POS, &[Arg::Int(0)]);
        self.runner.run(op::CAMERA_RESET_NEW_SCRIPTABLES, &[]);
        self.behaviour.attached = None;
        self.runner.run(op::SET_CAMERA_BEHIND_PLAYER, &[]);
        self.runner.run(op::RESTORE_CAMERA_JUMPCUT, &[]);
    }

    fn load_model(&mut self, model: i32) -> bool {
        gta::load_model(&mut self.runner, model)
    }

    pub(super) fn set_skin(&mut self, ped: u32, skin: u32) {
        gta::set_ped_skin(&mut self.runner, ped, skin);
    }

    /// `CPed::ClearWeapons` inside the local slot context (`CWorld::PlayerInFocus` only). Mirrors samp.dll 0x100AC050.
    pub(super) fn clear_weapons(&mut self, ped: u32) {
        crate::combatclient::local_context(false, || {
            // SAFETY: thiscall on the live local ped, main thread.
            unsafe {
                let f: extern "thiscall" fn(u32) = std::mem::transmute(game::PED_CLEAR_WEAPONS as usize);
                f(ped);
            }
        });
    }

    /// RPC 22 behaviour: load the weapon model, `CPed::GiveWeapon`, arm it. Mirrors samp.dll 0x100AF7D0.
    pub(super) fn give_weapon(&mut self, ped: u32, weapon: u8, ammo: u32) {
        let Some(model) = game::weapon_model(u32::from(weapon)) else { return };
        if !self.load_model(model) {
            return;
        }
        // GiveWeapon inside the slot context, then set-armed-weapon, not forced.
        crate::combatclient::local_context(true, || {
            // SAFETY: thiscall on the live local ped, main thread.
            unsafe {
                let f: extern "thiscall" fn(u32, u32, u32, u32) -> u32 =
                    std::mem::transmute(game::PED_GIVE_WEAPON as usize);
                f(ped, u32::from(weapon), ammo, 1);
            }
        });
        crate::combatclient::set_armed_local(&mut self.runner, ped, weapon);
    }
}
