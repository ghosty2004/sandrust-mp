//! samp's two custom script opcodes (CSV rows 2 and 3, gta-patches.md "Other").
//!
//! samp points the jump-table entries of two opcodes GTA leaves unimplemented
//! (0x09C6 and 0x09D3, table of `CRunningScript::ProcessCommands2500To2599` at
//! 0x47BF4C) at its own code. samp.saa's scripts use both, so with samp's
//! main.scm/script.img active they must exist: GTA's "unimplemented" target
//! (0x47BF2B) returns without reading the parameters, which would leave the
//! script's instruction pointer inside the arguments.
//!
//! Found in the R4 archive (byte scan of the decrypted scripts, each a full
//! instruction followed by a valid next opcode):
//! * 0x09C6, 4 parameters `(type, a, b, c)`: main.scm once (type 5),
//!   carmod1.scm 11× (types 1–4), player_parachute.scm 21× (types 22–43);
//! * 0x09D3, condition without parameters: carmod1.scm once (`NOT 09D3`).
//!
//! Behaviour (samp B 0x10002090 / 0x10001CE0 and 0x10001C30), in our words:
//! * 0x09C6 collects 4 parameters (`CRunningScript::CollectParameters`), then
//!   for types 1–4 sends the SCM-event RPC (96) with the vehicle id of handle
//!   `a` and `b`, `c`, the type — only if the vehicle is one of samp's vehicles;
//!   every other type does nothing more. It then continues like the
//!   unimplemented case (result 0, keep running).
//! * 0x09D3 sets the condition flag: false while the local player is in a
//!   vehicle (ped flag bit 8 at +0x46C) as its driver, unless that vehicle is a
//!   train carriage (models 569, 570), and also false when the flag is set but
//!   the ped has no vehicle pointer; true otherwise (on foot, passenger,
//!   carriage). B 0x10001C30 → 0x100ABE80 / 0x100ABFD0 ("in a vehicle, not as
//!   its driver or in a carriage"); measured: carmod1.scm only closes the mod
//!   garage once this is false for the driver (feata-notes.md "FEAT-1").

/// Jump-table slot of opcode 0x09C6 (CSV row 2).
pub const OP_09C6_SLOT: u32 = 0x47BF54;
/// Jump-table slot of opcode 0x09D3 (CSV row 3).
pub const OP_09D3_SLOT: u32 = 0x47BF88;
/// GTA's shared "unimplemented opcode" target both slots hold originally.
pub const UNIMPLEMENTED_TARGET: u32 = 0x47BF2B;

/// SCM event types 0x09C6 turns into RPC 96.
pub fn is_network_event(kind: i32) -> bool {
    (1..=4).contains(&kind)
}

/// Train carriage models samp treats like driving (569 freiflat, 570 streakc).
pub const TRAIN_CARRIAGE_MODELS: [u16; 2] = [569, 570];

/// 0x09D3's result. `vehicle`: the ped's vehicle (+0x58C) if set, as
/// (ped is its driver, model id).
pub fn op_09d3_result(in_vehicle_flag: bool, vehicle: Option<(bool, u16)>) -> bool {
    if !in_vehicle_flag {
        return true;
    }
    matches!(vehicle, Some((driver, model)) if !driver || TRAIN_CARRIAGE_MODELS.contains(&model))
}

#[cfg(windows)]
pub use hooks::install;

#[cfg(windows)]
mod hooks {
    use std::arch::naked_asm;

    use crate::game::{self, off};
    use crate::patch::{Memory, PatchError, Patcher, ProcessMemory};

    /// `CRunningScript::CollectParameters(count)` (thiscall), gta-reversed.
    const COLLECT_PARAMETERS: usize = 0x464080;
    /// `CRunningScript::UpdateCompareFlag(bool)` (thiscall), gta-reversed.
    const UPDATE_COMPARE_FLAG: usize = 0x4859D0;
    /// `ScriptParams[]` (CTheScripts), gta-reversed.
    const SCRIPT_PARAMS: u32 = 0xA43C78;
    /// `CPed::m_pVehicle`.
    const PED_VEHICLE: u32 = 0x58C;
    /// `CVehicle::m_pDriver`.
    const VEHICLE_DRIVER: u32 = 0x460;

    // Jump-table targets run inside ProcessCommands2500To2599's frame with the
    // script in ESI; every register is preserved and control continues at the
    // unimplemented target (result 0), like samp.
    #[unsafe(naked)]
    unsafe extern "C" fn op_09c6_stub() {
        naked_asm!(
            "pushad",
            "push esi",
            "call {h}",
            "add esp, 4",
            "popad",
            "push {back}",
            "ret",
            h = sym op_09c6,
            back = const super::UNIMPLEMENTED_TARGET,
        )
    }

    #[unsafe(naked)]
    unsafe extern "C" fn op_09d3_stub() {
        naked_asm!(
            "pushad",
            "push esi",
            "call {h}",
            "add esp, 4",
            "popad",
            "push {back}",
            "ret",
            h = sym op_09d3,
            back = const super::UNIMPLEMENTED_TARGET,
        )
    }

    extern "C" fn op_09c6(script: u32) {
        // SAFETY: GTA's CollectParameters on the running script (thiscall).
        let collect: extern "thiscall" fn(u32, i32) = unsafe { std::mem::transmute(COLLECT_PARAMETERS) };
        collect(script, 4);
        let m = ProcessMemory;
        let p = |i: u32| m.read_i32(SCRIPT_PARAMS + 4 * i).unwrap_or(0);
        let (kind, a, b, c) = (p(0), p(1), p(2), p(3));
        if super::is_network_event(kind) {
            // samp sends RPC 96 right here (B 0x10001CE0); the vehicle pool turns the handle into an id and
            // sends it in this frame's client tick (vehicles.rs `hook_events`).
            crate::log!("scm: 09C6 SCM event type {kind} (vehicle handle {a}, {b}, {c})");
            crate::vehhooks::with_shared(|s| {
                s.events.push(crate::vehhooks::HookEvent::Scm { handle: a, arg1: b, arg2: c, kind })
            });
        }
    }

    extern "C" fn op_09d3(script: u32) {
        let m = ProcessMemory;
        let ped = m.read_u32(game::CWORLD_PLAYERS + off::PLAYER_INFO_PED).unwrap_or(0);
        let result = if ped == 0 {
            true
        } else {
            let in_vehicle = m.read_u32(ped + off::PED_FLAGS).unwrap_or(0) & 0x100 != 0;
            let veh = m.read_u32(ped + PED_VEHICLE).unwrap_or(0);
            let vehicle = (veh != 0).then(|| {
                let driver = m.read_u32(veh + VEHICLE_DRIVER).unwrap_or(0) == ped;
                (driver, m.read_u16(veh + off::ENTITY_MODEL).unwrap_or(0))
            });
            super::op_09d3_result(in_vehicle, vehicle)
        };
        // SAFETY: GTA's UpdateCompareFlag on the running script (thiscall).
        let update: extern "thiscall" fn(u32, u32) = unsafe { std::mem::transmute(UPDATE_COMPARE_FLAG) };
        update(script, u32::from(result));
    }

    /// Points both jump-table slots at our handlers (each only if it still
    /// holds GTA's unimplemented target).
    pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Result<(), PatchError> {
        let orig = super::UNIMPLEMENTED_TARGET.to_le_bytes();
        let s1 = op_09c6_stub as unsafe extern "C" fn() as usize as u32;
        let s2 = op_09d3_stub as unsafe extern "C" fn() as usize as u32;
        patcher.write(mem, "SCM opcode 0x09C6", super::OP_09C6_SLOT, &s1.to_le_bytes(), Some(&orig))?;
        patcher.write(mem, "SCM opcode 0x09D3", super::OP_09D3_SLOT, &s2.to_le_bytes(), Some(&orig))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patches::testutil;

    #[test]
    fn slots_match_csv_and_exe() {
        let csv = testutil::csv_rows();
        for (row, slot) in [(2, OP_09C6_SLOT), (3, OP_09D3_SLOT)] {
            let r = &csv[&row];
            assert_eq!(testutil::field_u32(r, "gta_addr"), slot);
            assert_eq!(testutil::unhex(&r["original_bytes_hex"]), UNIMPLEMENTED_TARGET.to_le_bytes());
        }
        // Table at 0x47BF4C, 4 bytes per opcode from 0x09C4.
        assert_eq!(OP_09C6_SLOT, 0x47BF4C + 4 * (0x09C6 - 0x09C4));
        assert_eq!(OP_09D3_SLOT, 0x47BF4C + 4 * (0x09D3 - 0x09C4));
        if let Some(exe) = testutil::Exe::load() {
            for slot in [OP_09C6_SLOT, OP_09D3_SLOT] {
                assert_eq!(exe.read_va(slot, 4), UNIMPLEMENTED_TARGET.to_le_bytes());
            }
        }
    }

    #[test]
    fn op_09d3_rules() {
        assert!(op_09d3_result(false, None), "on foot");
        assert!(op_09d3_result(false, Some((false, 411))));
        assert!(!op_09d3_result(true, Some((true, 411))), "driver");
        assert!(op_09d3_result(true, Some((false, 411))), "passenger");
        assert!(op_09d3_result(true, Some((true, 570))), "train carriage driver");
        assert!(op_09d3_result(true, Some((false, 569))), "train carriage passenger");
        assert!(!op_09d3_result(true, None), "flag without a vehicle");
        assert!(is_network_event(1) && is_network_event(4));
        assert!(!is_network_event(5) && !is_network_event(22) && !is_network_event(0));
    }
}
