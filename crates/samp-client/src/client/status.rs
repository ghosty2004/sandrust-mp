//! Control-channel queries and requests (they only touch plain data), and the RPC 119 sender of the Present pass.

use std::time::Instant;

use samp_proto::raknet::Reliability;

use super::Client;

impl Client {
    pub fn status_line(&self) -> String {
        format!(
            "{} class_selection={} class={} spawned={}",
            self.net.status_line(),
            self.class_sel.is_active(),
            self.class_sel.index(),
            self.spawned
        )
    }

    /// Control-channel `remote`: one header line plus one line per remote player.
    pub fn remote_lines(&self) -> String {
        self.players.status(Instant::now(), |p| self.vehicles.remote_fields(p))
    }

    /// Control-channel `synced`: synced health/armour per remote player.
    pub fn synced_lines(&self) -> String {
        self.players.synced_lines()
    }

    /// Control-channel `vehicles`: one header line plus one line per streamed vehicle.
    pub fn vehicle_lines(&self) -> String {
        self.vehicles.status(&self.players)
    }

    /// Control-channel `world`: one header line plus one line per world entity.
    pub fn world_lines(&self) -> String {
        let mut lines = Vec::new();
        self.objects.status_lines(&mut lines);
        self.entities.status_lines(&mut lines);
        let mut out = format!(
            "world count={} {} {} {}",
            lines.len(),
            self.objects.header_fields(),
            self.entities.header_fields(),
            crate::plates::status()
        );
        for l in lines {
            out.push('\n');
            out.push_str(&l);
        }
        out
    }

    /// Control-channel `behaviour`: one line of camera / spectating / bounds / drunk / animation state.
    pub fn behaviour_line(&self) -> String {
        self.behaviour.status_line(self.spawned)
    }

    /// Control-channel `spawn`: run the spawn routine on the next tick.
    pub fn request_debug_spawn(&mut self) -> String {
        if self.spawn_info.is_none() {
            return "error no spawn info yet".into();
        }
        self.debug_spawn = true;
        "spawn queued".into()
    }

    /// RPC 119 (reliability 8), flushed at once (called from the Present pass while the pause menu is up).
    pub fn send_click_map(&mut self, pos: [f32; 3]) {
        crate::log!("client: RPC 119 {pos:?}");
        let p = samp_proto::rpcs::session::OnPlayerClickMap {
            pos: samp_proto::types::Vec3 { x: pos[0], y: pos[1], z: pos[2] },
        };
        self.net.send_rpc(&p, Reliability::Reliable);
        self.net.flush(Instant::now());
    }
}
