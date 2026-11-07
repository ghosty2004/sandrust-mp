//! The local player in a vehicle: driver sync (200, with 210 while towing), passenger sync (211), the damage
//! status the driver reports (106), and the in-car part of the client tick.

use super::*;

impl Vehicles {
    /// The in-car duplicate gate (shared timestamp of 200/211/210, also used for the in-car 203).
    pub fn dup_offer(&mut self, now: Instant, packet_id: u8, bytes: Vec<u8>) -> bool {
        self.dup.offer(now, packet_id, bytes)
    }

    /// A 200 was sent by the last `local_tick` (armed vehicles send a 203 after every 200).
    pub fn take_driver_sent(&mut self) -> Option<u16> {
        self.driver_sent.take()
    }

    /// Death while driving: one last 200, outside the interval. Mirrors samp.dll 0x10007080.
    pub fn force_driver_send(&mut self, net: &mut NetClient, now: Instant) {
        let ped = local_ped();
        if ped == 0 || !in_vehicle(ped) {
            return;
        }
        let veh = ped_vehicle(ped);
        if driver_of(veh) != ped {
            return;
        }
        if let Some(id) = self.id_of_ptr(veh) {
            self.send_driver(net, ped, veh, id, now);
        }
    }

    /// The local player's in-car part of the tick. Returns true when on-foot sync must not be sent (ped state
    /// not 1/12: driving, passenger, entering a seat or the exit animation).
    pub fn local_tick(&mut self, runner: &mut Runner, net: &mut NetClient, streamed: u32, now: Instant) -> bool {
        let ped = local_ped();
        if ped == 0 {
            return false;
        }
        self.end_train_camera(runner, ped, now);
        let st: u32 = rd(ped + off::PED_STATE);
        let veh = ped_vehicle(ped);
        let interval = vm::in_car_interval(self.in_car_rate.max(10), streamed);
        if (st == PEDSTATE_DRIVING || st == PEDSTATE_PASSENGER) && in_vehicle(ped) && veh != 0 {
            let Some(id) = self.id_of_ptr(veh) else { return true };
            let driver = driver_of(veh) == ped && st == PEDSTATE_DRIVING;
            if driver {
                self.local_driver_tick(net, ped, veh, id, interval, now);
            } else {
                self.local_passenger_tick(net, ped, veh, id, interval, now);
            }
            return true;
        }
        if let Some(prev) = self.driving.take() {
            self.last_driven = Some(prev);
        }
        self.last_rotation = None;
        if st == PEDSTATE_IDLE || st == PEDSTATE_SNIPER {
            if !in_vehicle(ped) {
                self.passenger_poll(runner, net, ped);
            }
            return false;
        }
        true
    }

    /// Five seconds after the train camera was pointed at a rail vehicle: the camera goes behind the player
    /// again when the ped is not in a vehicle.
    fn end_train_camera(&mut self, runner: &mut Runner, ped: u32, now: Instant) {
        if let Some(t) = self.train_camera_since
            && now.duration_since(t) > Duration::from_secs(5)
        {
            self.train_camera_since = None;
            if !in_vehicle(ped) {
                camera_behind(runner);
            }
        }
    }

    /// The local driver: 200 on its interval, then 106 when the damage status changed.
    fn local_driver_tick(
        &mut self,
        net: &mut NetClient,
        ped: u32,
        veh: u32,
        id: u16,
        interval: Duration,
        now: Instant,
    ) {
        self.driving = Some(id);
        if self.driver_timer.due(now, interval) {
            self.send_driver(net, ped, veh, id, now);
        }
        let d = damage_of(veh, kind_of(veh));
        if self.damage.update(id, d) {
            send(
                net,
                &rv::SetVehicleDamageStatus {
                    vehicle_id: id,
                    panels: d.panels,
                    doors: d.doors,
                    lights: d.lights,
                    tyres: d.tyres,
                },
            );
            self.n.sent_106 += 1;
        }
    }

    /// The local passenger: the vehicle driven until now becomes "last driven", then 211 on its interval.
    fn local_passenger_tick(
        &mut self,
        net: &mut NetClient,
        ped: u32,
        veh: u32,
        id: u16,
        interval: Duration,
        now: Instant,
    ) {
        if let Some(prev) = self.driving.take() {
            self.last_driven = Some(prev);
        }
        if self.passenger_timer.due(now, interval) {
            self.send_passenger(net, ped, veh, id, now);
        }
    }

    fn send_driver(&mut self, net: &mut NetClient, ped: u32, veh: u32, id: u16, now: Instant) {
        let m = matrix(veh);
        if m == 0 {
            return;
        }
        let Ok(sample) = game::read_local_sample(&ProcessMemory, ped) else { return };
        let kind = kind_of(veh);
        let trailer: u32 = rd(veh + g::VEH_TRAILER);
        let s = vm::DriverSample {
            vehicle_id: id,
            left_right: sample.pad.left_stick_x,
            up_down: sample.pad.left_stick_y,
            keys: footsync::key_word(&sample.pad, sample.key_ctx),
            additional_key: footsync::additional_key(&sample.pad),
            rotation: rows_of(m),
            position: v3(m + off::MATRIX_POS),
            velocity: v3(veh + off::MOVE_SPEED),
            vehicle_health: rd(veh + g::VEH_HEALTH),
            health: sample.health,
            armour: sample.armour,
            weapon: sample.weapon,
            siren: rd::<u8>(veh + g::VEH_SIREN_BYTE) & g::VEH_SIREN_ON != 0,
            landing_gear: kind == Kind::Plane && rd::<f32>(veh + g::PLANE_GEAR) != 0.0,
            trailer_id: self.id_of_ptr(trailer).unwrap_or(0),
            extra: driver_extra(veh, kind),
        };
        let p = vm::build_driver_sync(&s, self.last_rotation);
        if self.dup.offer(now, samp_proto::sync::ids::VEHICLE_SYNC, body(&p)) {
            self.last_rotation = Some(p.rotation);
            net.send_sync(&p);
            self.n.sent_200 += 1;
            self.driver_sent = Some(id);
        }
        self.send_trailer(net, s.trailer_id, now);
    }

    fn send_passenger(&mut self, net: &mut NetClient, ped: u32, veh: u32, id: u16, now: Instant) {
        let Ok(sample) = game::read_local_sample(&ProcessMemory, ped) else { return };
        let seat = (0..8u32).find(|&i| passenger_at(veh, i) == ped).map(|i| i as u8 + 1).unwrap_or(0);
        let mut p = vm::build_passenger_sync(
            id,
            seat,
            sample.weapon,
            footsync::additional_key(&sample.pad),
            sample.health,
            sample.armour,
            sample.pad.left_stick_x,
            sample.pad.left_stick_y,
            footsync::key_word(&sample.pad, sample.key_ctx),
            sample.position,
        );
        // Drive-by bit (byte 2 bit 6): the local drive-by task is active, from its start frame on.
        p.drive_by = self.local_drive_by;
        p.cuffed = self.local_cuffed;
        if self.dup.offer(now, samp_proto::sync::ids::PASSENGER_SYNC, body(&p)) {
            net.send_sync(&p);
            self.n.sent_211 += 1;
        }
    }
}

/// The model-specific `extra` field of a 200: Hydra nozzle angle, train speed or bike lean.
fn driver_extra(veh: u32, kind: Kind) -> u32 {
    let model = model_of_entity(veh);
    if model == vm::MODEL_HYDRA {
        u32::from(rd::<u16>(veh + g::HYDRA_NOZZLE))
    } else if vm::TRAIN_SYNC_MODELS.contains(&model) {
        rd::<f32>(veh + g::TRAIN_SPEED).to_bits()
    } else if matches!(kind, Kind::Bike | Kind::Bmx) {
        rd::<f32>(veh + g::BIKE_LEAN).to_bits()
    } else {
        0
    }
}
