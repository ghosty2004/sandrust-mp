//! Diagnostics: the `vehicles` control-channel reply, the vehicle fields of a `remote` line, pool statistics and
//! the rows of the `/dl` labels.

use super::*;

impl Vehicles {
    /// Vehicles in the pool and the distinct models among those with a GTA vehicle.
    pub fn pool_stats(&self) -> (u32, u32) {
        let models: std::collections::BTreeSet<u32> =
            self.pool.values().filter(|v| v.ptr != 0).map(|v| v.model).collect();
        (self.pool.len() as u32, models.len() as u32)
    }

    /// (id, GTA vehicle, stream-in position) of every vehicle with a GTA object, in id order (`/dl` labels).
    pub fn debug_rows(&self) -> Vec<(u16, u32, [f32; 3])> {
        self.pool.values().filter(|v| v.ptr != 0).map(|v| (v.id, v.ptr, v.spawn_pos)).collect()
    }

    /// `vehicles` reply: a header line, then one line per vehicle.
    pub fn status(&self, players: &Players) -> String {
        let local = local_ped();
        let mut out = self.status_header();
        for v in self.pool.values() {
            out.push('\n');
            out.push_str(&status_line(v, local, players));
        }
        out
    }

    fn status_header(&self) -> String {
        let [a, b, c, d] = vehhooks::call_counts();
        let n = &self.n;
        format!(
            "vehicles count={} process_control_calls={a}/{b}/{c}/{d} sent_200={} sent_210={} sent_211={} sent_209={} \
             sent_26={} sent_154={} sent_106={} sent_136={} applied_209={} last_driven={} sent_96={} jacked={}",
            self.pool.len(),
            n.sent_200,
            n.sent_210,
            n.sent_211,
            n.sent_209,
            n.sent_26,
            n.sent_154,
            n.sent_106,
            n.sent_136,
            n.applied_209,
            self.last_driven.map(|i| i.to_string()).unwrap_or_else(|| "-".into()),
            n.sent_96,
            n.jacked,
        )
    }

    /// Extra fields of a `remote` line: the GTA vehicle/seat of the ped and the synced vehicle.
    pub fn remote_fields(&self, p: &remote::RemotePlayer) -> String {
        let (veh, seat) = if p.ped != 0 && in_vehicle(p.ped) {
            let ptr = ped_vehicle(p.ped);
            let id = self.id_of_ptr(ptr).map(|i| i.to_string()).unwrap_or_else(|| "?".into());
            let seat = if driver_of(ptr) == p.ped {
                "0".to_string()
            } else {
                (0..8u32)
                    .find(|&i| passenger_at(ptr, i) == p.ped)
                    .map(|i| (i + 1).to_string())
                    .unwrap_or_else(|| "?".into())
            };
            (id, seat)
        } else {
            ("-".into(), "-".into())
        };
        let r = self.remote.get(&p.id).copied().unwrap_or_default();
        let synced = r
            .driver
            .map(|d| format!("{}:{:.2},{:.2},{:.2}", d.vehicle_id, d.position[0], d.position[1], d.position[2]))
            .or_else(|| r.passenger.map(|d| format!("{}:seat{}", d.vehicle_id, d.seat)))
            .unwrap_or_else(|| "-".into());
        format!(
            " vehicle={veh} seat={seat} veh_sync={synced} veh_corrections={} veh_teleports={} warps_in={} warps_out={}",
            r.corrections, r.teleports, r.warps_in, r.warps_out
        )
    }
}

/// One vehicle line of the `vehicles` reply.
fn status_line(v: &Veh, local: u32, players: &Players) -> String {
    let ptr = if v.ptr != 0 { vehicle_at(v.handle) } else { 0 };
    let (pos, heading, health, driver, occ) = if ptr != 0 {
        let m = matrix(ptr);
        let driver = driver_of(ptr);
        let occ = if local != 0 && driver == local {
            "driver"
        } else if local != 0 && is_passenger(ptr, local) {
            "passenger"
        } else {
            "none"
        };
        (
            (m != 0).then(|| v3(m + off::MATRIX_POS)),
            (m != 0).then(|| vm::heading_deg(v3(m + g::MATRIX_FORWARD))),
            Some(rd::<f32>(ptr + g::VEH_HEALTH)),
            driver_label(driver, local, players),
            occ,
        )
    } else {
        (None, None, None, "-".to_string(), "none")
    };
    vm::format_vehicle(v.id, v.model, v.handle, ptr, pos, heading, health, &driver, occ)
}

/// The driver column: "-", "local", the remote player id or the raw ped.
fn driver_label(driver: u32, local: u32, players: &Players) -> String {
    if driver == 0 {
        "-".to_string()
    } else if driver == local {
        "local".to_string()
    } else {
        players.iter().find(|p| p.ped == driver).map(|p| p.id.to_string()).unwrap_or_else(|| format!("ped:{driver:#x}"))
    }
}
