//! Surfing: a player standing on a moving vehicle or object is carried along with it.
//!
//! [`LocalSurf`] is the pure, host-tested state machine: what the ped stands on (`client::surf_probe`), a
//! 2-second contact latch and the surf fields of the on-foot sync packet (207). The game side is in [`gta`]:
//! the local glue keeps the ped at a fixed offset in the surface's frame while no movement stick is held, the
//! remote glue puts remote surfers at surface matrix × synced offset. Both run right after
//! `CWorld::ProcessAttachedEntities` inside `CWorld::Process`.
//! Spec: docs/re/subsystems/remote-control-onfoot.md, "3.7 Surfing".

/// Contact latch: after this long without contact the surf state is cleared.
pub const LATCH_MS: u32 = 2000;
/// Vehicles farther than this from the local player are not surfed.
pub const MAX_VEHICLE_DISTANCE: f32 = 30.0;
/// Remote offsets outside [−100, 100] in any component are ignored.
pub const MAX_REMOTE_OFFSET: f32 = 100.0;
/// Surf ids: vehicles 1..1999, objects 2000 + object id.
pub const OBJECT_BASE: u16 = 2000;
pub const NO_SURF: u16 = 0xFFFF;

/// What the probe found under the ped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contact {
    None,
    Vehicle(u16),
    Object(u16),
}

/// The surface the glue follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Vehicle(u16),
    Object(u16),
}

/// The local player's surf state (samp keeps the same fields in its local-player object).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalSurf {
    pub id: u16,
    pub last_contact_ms: u32,
    pub target: Option<Target>,
    pub glue: bool,
    pub latched: bool,
    pub offset: [f32; 3],
}

impl Default for LocalSurf {
    fn default() -> Self {
        Self { id: NO_SURF, last_contact_ms: 0, target: None, glue: false, latched: false, offset: [0.0; 3] }
    }
}

/// What the caller must do after a probe step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeAction {
    None,
    /// First contact: run the glue once with `glue == false` (measures the offset), then latch.
    MeasureOffset,
}

impl LocalSurf {
    /// Spawn and local-player creation: everything cleared.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Teleports (RPC 12/13) and the stream-out of the surfed vehicle (RPC 165): unlatched, no surface, no
    /// glue; the id and offset stay.
    pub fn detach(&mut self) {
        self.latched = false;
        self.target = None;
        self.glue = false;
    }

    /// The surfed object was destroyed (RPC 47): surface and latch cleared.
    pub fn object_destroyed(&mut self, id: u16) {
        if self.target == Some(Target::Object(id)) {
            self.target = None;
            self.latched = false;
        }
    }

    /// A vehicle streamed out (RPC 165); detaches when it is the surfed one.
    pub fn vehicle_streamed_out(&mut self, id: u16) {
        if self.target == Some(Target::Vehicle(id)) {
            self.detach();
        }
    }

    /// One probe step, run on every local on-foot tick. Mirrors samp.dll 0x10003730.
    /// `cancel`: the ped is entering a car or in another task that stops surfing (see `gta::cancel_tasks`);
    /// `exists(t)`: the surface is still in its pool.
    pub fn probe(
        &mut self,
        contact: Contact,
        cancel: bool,
        now_ms: u32,
        exists: impl Fn(Target) -> bool,
    ) -> ProbeAction {
        if cancel {
            self.target = None;
            self.glue = false;
            return ProbeAction::None;
        }
        match contact {
            Contact::None => {
                if !self.latched {
                    self.target = None;
                    self.glue = false;
                    return ProbeAction::None;
                }
                if now_ms.wrapping_sub(self.last_contact_ms) >= LATCH_MS {
                    self.latched = false;
                    self.offset = [0.0; 3];
                    self.id = NO_SURF;
                    self.target = None;
                    self.glue = false;
                }
                if self.latched && now_ms.wrapping_sub(self.last_contact_ms) < LATCH_MS {
                    let t = if self.id < OBJECT_BASE {
                        Target::Vehicle(self.id)
                    } else {
                        Target::Object(self.id.wrapping_sub(OBJECT_BASE))
                    };
                    if exists(t) {
                        self.target = Some(t);
                        self.glue = true;
                    }
                }
                ProbeAction::None
            }
            Contact::Vehicle(id) => {
                self.id = id;
                self.last_contact_ms = now_ms;
                let first = !self.latched;
                let t = exists(Target::Vehicle(id)).then_some(Target::Vehicle(id));
                self.target = t;
                self.latched = true;
                if first {
                    self.glue = false;
                    return ProbeAction::MeasureOffset;
                }
                self.glue = true;
                ProbeAction::None
            }
            Contact::Object(id) => {
                self.id = id.wrapping_add(OBJECT_BASE);
                self.last_contact_ms = now_ms;
                if !exists(Target::Object(id)) {
                    return ProbeAction::None;
                }
                self.target = Some(Target::Object(id));
                let first = !self.latched;
                self.latched = true;
                if first {
                    self.glue = false;
                    return ProbeAction::MeasureOffset;
                }
                self.glue = true;
                ProbeAction::None
            }
        }
    }

    /// After the first-contact offset measurement: latched and glued.
    pub fn latch_after_measure(&mut self) {
        self.latched = true;
        self.glue = true;
    }

    /// Surf fields of the on-foot sync packet (207): (offset, id) while latched, else zeros.
    pub fn sync_fields(&self) -> ([f32; 3], u16) {
        if self.latched { (self.offset, self.id) } else { ([0.0; 3], 0) }
    }
}

/// Surf target of a remote player from its on-foot sync (207): `None` when the player does not surf or the
/// offset is out of range.
pub fn remote_target(surf_id: u16, offset: [f32; 3]) -> Option<Target> {
    if surf_id == 0 || surf_id == NO_SURF || surf_id >= 3000 {
        return None;
    }
    let t = if surf_id < OBJECT_BASE {
        Target::Vehicle(surf_id)
    } else {
        let o = surf_id - OBJECT_BASE;
        if o > 1000 {
            return None;
        }
        Target::Object(o)
    };
    offset.iter().all(|v| (-MAX_REMOTE_OFFSET..=MAX_REMOTE_OFFSET).contains(v)).then_some(t)
}

/// `out = M × v` with GTA's matrix layout (right +0x00, forward +0x10, up +0x20, pos +0x30).
pub fn transform(m: &[f32; 16], v: [f32; 3]) -> [f32; 3] {
    [
        m[8] * v[2] + m[4] * v[1] + m[0] * v[0] + m[12],
        m[9] * v[2] + m[5] * v[1] + m[1] * v[0] + m[13],
        m[10] * v[2] + m[6] * v[1] + m[2] * v[0] + m[14],
    ]
}

#[cfg(windows)]
pub use gta::*;

#[cfg(windows)]
mod gta {
    use std::sync::Mutex;

    use super::*;

    fn rd<T: Copy + Default>(a: u32) -> T {
        if a < 0x10000 {
            return T::default();
        }
        // SAFETY: addresses inside live game objects or game globals.
        unsafe { std::ptr::read_unaligned(a as usize as *const T) }
    }

    fn wr<T: Copy>(a: u32, v: T) {
        if a < 0x10000 {
            return;
        }
        // SAFETY: writable fields of live game objects.
        unsafe { std::ptr::write_unaligned(a as usize as *mut T, v) }
    }

    const ENTITY_MATRIX: u32 = 0x14;
    const MOVE_SPEED: u32 = 0x44;
    const TURN_SPEED: u32 = 0x50;
    /// `CPad::Pads[0]` NewState LeftStickX / LeftStickY.
    const PAD0: u32 = 0xB73458;
    /// `CPed::m_pIntelligence`, primary task slots of its task manager.
    const PED_INTELLIGENCE: u32 = 0x47C;
    /// First primary task slot inside `CPedIntelligence` (the task manager's array).
    const INTEL_PRIMARY_TASKS: u32 = 0x4;
    const PED_FLAGS: u32 = 0x46C;
    /// `PED_FLAGS` bit `bInVehicle`.
    const PED_IN_VEHICLE: u32 = 0x100;
    /// `CPed::m_nPedType` (0 = player).
    const PED_TYPE: u32 = 0x598;
    /// `CVehicle::m_pDriver`.
    const VEH_DRIVER: u32 = 0x460;
    /// `CTask::GetTaskType` vtable slot.
    const VT_TASK_GET_TYPE: u32 = 0x10;
    /// `RwMatrixInvert(dst, src)`.
    const RW_MATRIX_INVERT: u32 = 0x7F2070;
    /// Task type 211 in a primary slot: the glue does not hold the ped.
    const TASK_NO_GLUE: u32 = 0xD3;

    /// (ped, surface entity, offset) of every remote player to glue this frame.
    static REMOTES: Mutex<Vec<(u32, u32, [f32; 3])>> = Mutex::new(Vec::new());
    /// The local surface entity, offset and glue flag for the hook.
    static LOCAL: Mutex<Option<(u32, [f32; 3], bool)>> = Mutex::new(None);
    /// Offset measured by the hook-side glue (fed back into the state at the next tick).
    static MEASURED: Mutex<Option<[f32; 3]>> = Mutex::new(None);

    pub fn matrix(ent: u32) -> Option<[f32; 16]> {
        let m: u32 = rd(ent + ENTITY_MATRIX);
        if ent == 0 || m == 0 {
            return None;
        }
        let mut out = [0f32; 16];
        for (i, v) in out.iter_mut().enumerate() {
            // Flags dwords (+0x0C, +0x1C, +0x2C, +0x3C) stay 0, like samp's matrix copies.
            if i % 4 != 3 {
                *v = rd(m + 4 * i as u32);
            }
        }
        Some(out)
    }

    fn v3(a: u32) -> [f32; 3] {
        [rd(a), rd(a + 4), rd(a + 8)]
    }

    fn put3(a: u32, v: [f32; 3]) {
        for (i, x) in v.iter().enumerate() {
            wr(a + 4 * i as u32, *x);
        }
    }

    fn task_type(task: u32) -> u32 {
        if task == 0 {
            return 0;
        }
        // Like samp, trust only tasks whose vtable lies in gta_sa.exe's .rdata (0x800000..=0x900000).
        let vt: u32 = rd(task);
        if !(0x80_0000..=0x90_0000).contains(&vt) {
            return 0;
        }
        let f: u32 = rd(vt + VT_TASK_GET_TYPE);
        if f == 0 {
            return 0;
        }
        // SAFETY: CTask::GetTaskType (vtable +0x10, thiscall) on a live task.
        unsafe {
            let f: extern "thiscall" fn(u32) -> u32 = std::mem::transmute(f as usize);
            f(task)
        }
    }

    fn primary_task(ped: u32, slot: u32) -> u32 {
        let intel: u32 = rd(ped + PED_INTELLIGENCE);
        if intel == 0 { 0 } else { rd(intel + INTEL_PRIMARY_TASKS + 4 * slot) }
    }

    fn in_vehicle(ped: u32) -> bool {
        rd::<u32>(ped + PED_FLAGS) & PED_IN_VEHICLE != 0
    }

    /// The probe's cancel test: physical-response task 208 (not in a vehicle), primary task 704, or an
    /// enter-car task (700/701/712/713).
    pub fn cancel_tasks(ped: u32) -> bool {
        if ped == 0 {
            return true;
        }
        if !in_vehicle(ped) && task_type(primary_task(ped, 0)) == 0xD0 {
            return true;
        }
        matches!(task_type(primary_task(ped, 3)), 0x2C0 | 0x2BC | 0x2BD | 0x2C8 | 0x2C9)
    }

    /// samp's "player-driven" test of a vehicle: a driver that is in the vehicle and has ped type 0 (player).
    /// Mirrors samp.dll 0x100B7120.
    pub fn player_driven(vehicle: u32) -> bool {
        let driver: u32 = rd(vehicle + VEH_DRIVER);
        driver != 0 && in_vehicle(driver) && rd::<u32>(driver + PED_TYPE) == 0
    }

    fn inverse(m: &[f32; 16]) -> [f32; 16] {
        let mut out = [0f32; 16];
        // SAFETY: GTA's RwMatrixInvert(dst, src) on two 0x40-byte buffers (flags 0: general inverse).
        unsafe {
            let f: extern "C" fn(*mut f32, *const f32) -> *mut f32 = std::mem::transmute(RW_MATRIX_INVERT as usize);
            f(out.as_mut_ptr(), m.as_ptr());
        }
        out
    }

    /// The local glue for `ped` on `surface`: holds the ped when `glue` is set and no stick is held, else
    /// returns the re-measured offset (inverse surface matrix × ped position). Mirrors samp.dll 0x10003570.
    pub fn local_glue(ped: u32, surface: u32, offset: [f32; 3], glue: bool) -> Option<[f32; 3]> {
        let (Some(e), Some(p)) = (matrix(surface), matrix(ped)) else { return None };
        let lr: i16 = rd(PAD0);
        let ud: i16 = rd(PAD0 + 2);
        let held = !in_vehicle(ped) && task_type(primary_task(ped, 3)) == TASK_NO_GLUE;
        if glue && lr == 0 && ud == 0 && !held {
            let pos = transform(&e, offset);
            let rows = crate::footsync::Rotation {
                right: [p[0], p[1], p[2]],
                forward: [p[4], p[5], p[6]],
                up: [p[8], p[9], p[10]],
            };
            crate::objects::write_matrix(ped, &rows, pos);
            let ev = v3(surface + MOVE_SPEED);
            let pv = v3(ped + MOVE_SPEED);
            put3(ped + MOVE_SPEED, [ev[0], ev[1], pv[2]]);
            let et = v3(surface + TURN_SPEED);
            let pt = v3(ped + TURN_SPEED);
            put3(ped + TURN_SPEED, [pt[0], pt[1], et[2]]);
            None
        } else {
            Some(transform(&inverse(&e), [p[12], p[13], p[14]]))
        }
    }

    /// The remote glue: the ped's position := surface matrix × offset. Mirrors samp.dll 0x10014650.
    fn remote_glue(ped: u32, surface: u32, offset: [f32; 3]) {
        let (Some(e), Some(p)) = (matrix(surface), matrix(ped)) else { return };
        let pos = transform(&e, offset);
        let rows = crate::footsync::Rotation {
            right: [p[0], p[1], p[2]],
            forward: [p[4], p[5], p[6]],
            up: [p[8], p[9], p[10]],
        };
        crate::objects::write_matrix(ped, &rows, pos);
    }

    /// Published by the client tick (main thread) for the hook.
    pub fn publish(local: Option<(u32, [f32; 3], bool)>, remotes: Vec<(u32, u32, [f32; 3])>) {
        *LOCAL.lock().unwrap_or_else(|e| e.into_inner()) = local;
        *REMOTES.lock().unwrap_or_else(|e| e.into_inner()) = remotes;
    }

    /// The offset the hook re-measured since the last tick.
    pub fn take_measured() -> Option<[f32; 3]> {
        MEASURED.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// Runs right after `CWorld::ProcessAttachedEntities` (from the hook in `worldhooks`): local glue, then
    /// the remote players.
    pub fn after_process_attached_entities(local_ped: u32) {
        let local = *LOCAL.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((surface, offset, glue)) = local
            && local_ped != 0
            && let Some(o) = local_glue(local_ped, surface, offset, glue)
        {
            *MEASURED.lock().unwrap_or_else(|e| e.into_inner()) = Some(o);
            // Keep the published copy current for the next frame before the tick runs.
            *LOCAL.lock().unwrap_or_else(|e| e.into_inner()) = Some((surface, o, glue));
        }
        let remotes = REMOTES.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for (ped, surface, offset) in remotes {
            remote_glue(ped, surface, offset);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_contact_measures_then_glues() {
        let mut s = LocalSurf::default();
        assert_eq!(s.probe(Contact::Vehicle(5), false, 1000, |_| true), ProbeAction::MeasureOffset);
        s.latch_after_measure();
        assert!(s.latched && s.glue && s.target == Some(Target::Vehicle(5)) && s.id == 5);
        assert_eq!(s.probe(Contact::Vehicle(5), false, 1100, |_| true), ProbeAction::None);
        assert!(s.glue);
        assert_eq!(s.sync_fields().1, 5);
    }

    #[test]
    fn latch_holds_two_seconds() {
        let mut s = LocalSurf::default();
        s.probe(Contact::Object(7), false, 1000, |_| true);
        s.latch_after_measure();
        s.offset = [1.0, 2.0, 3.0];
        s.probe(Contact::None, false, 2999, |_| true);
        assert!(s.latched && s.target == Some(Target::Object(7)) && s.id == 2007);
        s.probe(Contact::None, false, 3000, |_| true);
        assert!(!s.latched && s.target.is_none() && s.id == NO_SURF && s.offset == [0.0; 3]);
        assert_eq!(s.sync_fields(), ([0.0; 3], 0));
    }

    #[test]
    fn cancel_keeps_latch() {
        let mut s = LocalSurf::default();
        s.probe(Contact::Vehicle(3), false, 0, |_| true);
        s.latch_after_measure();
        s.probe(Contact::Vehicle(3), true, 10, |_| true);
        assert!(s.latched && s.target.is_none() && !s.glue);
    }

    #[test]
    fn remote_targets() {
        assert_eq!(remote_target(12, [0.0, 0.0, 1.0]), Some(Target::Vehicle(12)));
        assert_eq!(remote_target(2003, [100.0, -100.0, 0.0]), Some(Target::Object(3)));
        assert_eq!(remote_target(2003, [100.5, 0.0, 0.0]), None);
        assert_eq!(remote_target(0, [0.0; 3]), None);
        assert_eq!(remote_target(0xFFFF, [0.0; 3]), None);
        assert_eq!(remote_target(3001, [0.0; 3]), None);
        assert_eq!(remote_target(2000 + 1001, [0.0; 3]), None);
    }

    #[test]
    fn transform_uses_rows() {
        let mut m = [0f32; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[12] = 10.0;
        m[13] = 20.0;
        m[14] = 30.0;
        assert_eq!(transform(&m, [1.0, 2.0, 3.0]), [11.0, 22.0, 33.0]);
    }
}
