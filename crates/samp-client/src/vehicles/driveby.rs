//! The drive-by task of local and remote passengers, and the leave-car task test, both read from the ped
//! intelligence task slot samp uses.

use super::*;

/// `CPedIntelligence` task slot samp reads for the drive-by task.
const INTEL_DRIVE_BY_SLOT: u32 = 0x10;
/// `TASK_SIMPLE_GANG_DRIVEBY` task type.
const TASK_GANG_DRIVEBY: u32 = 0x3FE;
/// `CTaskSimpleGangDriveBy` deleting destructor.
const GANG_DRIVEBY_DELETING_DTOR: u32 = 0x6275E0;
/// `TASK_COMPLEX_LEAVE_CAR`.
const TASK_LEAVE_CAR: u32 = 0x2C0;
/// Task vtables are trusted only inside gta_sa.exe's read-only data.
const TASK_VTABLE_RANGE: std::ops::RangeInclusive<u32> = 0x80_0000..=0x90_0000;

/// The task in the intelligence slot +0x10 (the task manager's primary slot 3) and its type, if any.
fn intel_slot_task(ped: u32) -> Option<(u32, u32)> {
    let intel: u32 = rd(ped + off::PED_INTELLIGENCE);
    if intel == 0 {
        return None;
    }
    let task: u32 = rd(intel + INTEL_DRIVE_BY_SLOT);
    if task == 0 {
        return None;
    }
    let vt: u32 = rd(task);
    if !TASK_VTABLE_RANGE.contains(&vt) {
        return None;
    }
    let get_type: u32 = rd(vt + g::VT_TASK_GET_TYPE);
    // SAFETY: the task's own GetTaskType virtual (thiscall), main thread.
    let kind = unsafe {
        let f: extern "thiscall" fn(u32) -> u32 = std::mem::transmute(get_type as usize);
        f(task)
    };
    Some((task, kind))
}

/// The drive-by task is active (task type 0x3FE in the intelligence slot +0x10).
pub(super) fn drive_by_task(ped: u32) -> u32 {
    match intel_slot_task(ped) {
        Some((task, TASK_GANG_DRIVEBY)) => task,
        _ => 0,
    }
}

/// The ped is running the leave-car task (in the intelligence slot samp reads).
pub(super) fn leaving_car(ped: u32) -> bool {
    matches!(intel_slot_task(ped), Some((_, TASK_LEAVE_CAR)))
}

/// Stops the drive-by task through its deleting destructor and clears the slot. Mirrors samp.dll 0x100AD0C0.
pub(crate) fn stop_drive_by(ped: u32) {
    let task = drive_by_task(ped);
    if task == 0 {
        return;
    }
    // SAFETY: the task's deleting destructor (thiscall), main thread.
    unsafe {
        let f: extern "thiscall" fn(u32, u32) = std::mem::transmute(GANG_DRIVEBY_DELETING_DTOR as usize);
        f(task, 1);
    }
    let intel: u32 = rd(ped + off::PED_INTELLIGENCE);
    poke(intel + INTEL_DRIVE_BY_SLOT, 0u32);
}

/// Starts the drive-by task: in a vehicle only; with the parachute the weapon goes to fists and no task starts;
/// otherwise the current weapon is armed again and TASK_DRIVE_BY(ped, −1, −1, 0, 0, 0, 300, 8, 1, 20) runs (same
/// parameters for the local and the remote passenger). Mirrors samp.dll 0x100ACFD0.
pub(crate) fn start_drive_by(runner: &mut Runner, ped: u32, arm: impl FnOnce(&mut Runner, u8)) -> bool {
    if !in_vehicle(ped) || ped_vehicle(ped) == 0 {
        return false;
    }
    let Some(h) = ped_handle(ped) else { return false };
    let w = current_weapon(ped) as u8;
    if w == vm::WEAPON_PARACHUTE {
        arm(runner, 0);
        return false;
    }
    arm(runner, w);
    let f = Arg::Float;
    runner.run(
        op::TASK_DRIVE_BY,
        &[
            Arg::Int(h),
            Arg::Int(-1),
            Arg::Int(-1),
            f(0.0),
            f(0.0),
            f(0.0),
            f(300.0),
            Arg::Int(8),
            Arg::Int(1),
            Arg::Int(20),
        ],
    );
    true
}

/// Remote passenger drive-by bit: start when set and not active, stop when clear.
pub(super) fn remote_drive_by(runner: &mut Runner, ped: u32, slot: u8, on: bool) {
    let active = drive_by_task(ped) != 0;
    if on && !active {
        start_drive_by(runner, ped, |r, w| crate::players::set_weapon(r, ped, slot, w));
    } else if !on && active {
        stop_drive_by(ped);
    }
}
