//! Keeps the animations a ped's tasks still hold alive across a skin change.
//!
//! A skin change (RPC 153) deletes the ped's clump, which frees every animation association in its list, yet a
//! running task (e.g. hands-up) keeps its own pointer to the association it started and writes into it when it is
//! later aborted (death, any event). The original gets away with it because the freed block stays unused; in ours it
//! was reused and the later writes corrupted streaming. So before the clump is deleted we unlink the associations
//! the ped's tasks point at, which leaks one association per skin change during an animation task.
//! Background: docs/phase3/m5-notes.md; docs/ROADMAP.md, "Decisions made autonomously".

use crate::patch::{Memory, PatchError};

/// `CAnimBlendAssociation` link offset (next, prev).
pub const ASSOC_LINK: u32 = 4;
/// Bytes of a task scanned for an association pointer (CTask pool slots are 0x80 bytes).
pub const TASK_SCAN: u32 = 0x40;
/// Safety bound for list walks.
const MAX_LINKS: usize = 256;

/// Associations in a clump's animation list (`data` = the clump's `CAnimBlendClumpData*`, whose first field is
/// the list head link), head first.
pub fn associations<M: Memory>(mem: &M, data: u32) -> Result<Vec<u32>, PatchError> {
    let mut out = Vec::new();
    let mut link = mem.read_u32(data)?;
    while link != 0 && out.len() < MAX_LINKS {
        out.push(link - ASSOC_LINK);
        link = mem.read_u32(link)?;
    }
    Ok(out)
}

/// Removes `assoc` from its list (fixes the neighbours' links) and clears its own link.
pub fn unlink<M: Memory>(mem: &mut M, assoc: u32) -> Result<(), PatchError> {
    let link = assoc + ASSOC_LINK;
    let next = mem.read_u32(link)?;
    let prev = mem.read_u32(link + 4)?;
    if next != 0 {
        mem.write(next + 4, &prev.to_le_bytes())?;
    }
    if prev != 0 {
        mem.write(prev, &next.to_le_bytes())?;
    }
    mem.write(link, &0u32.to_le_bytes())?;
    mem.write(link + 4, &0u32.to_le_bytes())
}

/// The associations of `data`'s list that one of `tasks` points at (first `TASK_SCAN` bytes after the
/// vtable), unlinked from the list so deleting the clump keeps them. Returns them.
pub fn keep_task_held<M: Memory>(mem: &mut M, data: u32, tasks: &[u32]) -> Result<Vec<u32>, PatchError> {
    let list = associations(mem, data)?;
    let mut held = Vec::new();
    for &task in tasks {
        for off in (4..TASK_SCAN).step_by(4) {
            let w = mem.read_u32(task + off)?;
            if w != 0 && list.contains(&w) && !held.contains(&w) {
                held.push(w);
            }
        }
    }
    for &a in &held {
        unlink(mem, a)?;
    }
    Ok(held)
}

#[cfg(windows)]
pub use imp::*;

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::game::off;
    use crate::patch::ProcessMemory;

    /// `ClumpOffset`: the global holding the anim blend plugin's offset inside an RpClump.
    const CLUMP_OFFSET: u32 = 0xB5F878;
    /// `CEntity::m_pRwObject`.
    const ENTITY_RW_OBJECT: u32 = 0x18;
    /// `CPedIntelligence` + 4: `CTaskManager` (5 primary, then 6 secondary task pointers).
    const TASK_MANAGER: u32 = 4;
    const TASK_SLOTS: u32 = 11;
    /// `CTask::GetSubTask` vtable slot.
    const VT_GET_SUB_TASK: u32 = 2 * 4;

    /// Every task of the ped: the 11 task-manager slots and their sub-task chains.
    fn ped_tasks(ped: u32) -> Vec<u32> {
        let mem = ProcessMemory;
        let intel = mem.read_u32(ped + off::PED_INTELLIGENCE).unwrap_or(0);
        let mut out = Vec::new();
        if intel == 0 {
            return out;
        }
        for slot in 0..TASK_SLOTS {
            let mut task = mem.read_u32(intel + TASK_MANAGER + 4 * slot).unwrap_or(0);
            while task != 0 && out.len() < 64 && !out.contains(&task) {
                out.push(task);
                let vt = mem.read_u32(task).unwrap_or(0);
                let Ok(get_sub) = mem.read_u32(vt + VT_GET_SUB_TASK) else { break };
                // SAFETY: the task's own GetSubTask virtual (thiscall, no arguments), main thread.
                task = unsafe {
                    let f: extern "thiscall" fn(u32) -> u32 = std::mem::transmute(get_sub as usize);
                    f(task)
                };
            }
        }
        out
    }

    /// Before the ped's clump is deleted (skin change): keep the associations its tasks hold.
    pub fn keep_before_delete(ped: u32) {
        let mut mem = ProcessMemory;
        let clump = mem.read_u32(ped + ENTITY_RW_OBJECT).unwrap_or(0);
        let plugin = mem.read_u32(CLUMP_OFFSET).unwrap_or(0);
        if clump == 0 || plugin == 0 {
            return;
        }
        let data = mem.read_u32(clump + plugin).unwrap_or(0);
        if data == 0 {
            return;
        }
        let tasks = ped_tasks(ped);
        match keep_task_held(&mut mem, data, &tasks) {
            Ok(held) if !held.is_empty() => {
                crate::log!(
                    "pedanims: ped {ped:#x}: kept {} task-held association(s) {held:x?} across the skin change",
                    held.len()
                );
            }
            Ok(_) => {}
            Err(e) => crate::log!("pedanims: ped {ped:#x}: {e:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::fake::FakeMemory;

    const BASE: u32 = 0x1000;

    fn w(m: &mut FakeMemory, a: u32, v: u32) {
        m.set(a, &v.to_le_bytes());
    }

    /// Clump data at 0x1000 with associations at 0x1100, 0x1200, 0x1300 (links at +4).
    fn list() -> FakeMemory {
        let mut m = FakeMemory::new(BASE, 0x1000);
        let data = 0x1000;
        let a = [0x1100u32, 0x1200, 0x1300];
        w(&mut m, data, a[0] + 4);
        for (i, &x) in a.iter().enumerate() {
            let next = a.get(i + 1).map_or(0, |n| n + 4);
            let prev = if i == 0 { data } else { a[i - 1] + 4 };
            w(&mut m, x + 4, next);
            w(&mut m, x + 8, prev);
        }
        m
    }

    #[test]
    fn walks_the_clump_list() {
        let m = list();
        assert_eq!(associations(&m, 0x1000).unwrap(), vec![0x1100, 0x1200, 0x1300]);
    }

    #[test]
    fn task_held_association_leaves_the_list_and_stays_intact() {
        let mut m = list();
        // A hands-up-like task at 0x1800: vtable, parent, then the association it started at +8.
        w(&mut m, 0x1800, 0x85A29C);
        w(&mut m, 0x1808, 0x1200);
        w(&mut m, 0x1200, 0x85C6D0);
        let held = keep_task_held(&mut m, 0x1000, &[0x1800]).unwrap();
        assert_eq!(held, vec![0x1200]);
        assert_eq!(associations(&m, 0x1000).unwrap(), vec![0x1100, 0x1300]);
        // Back links stay consistent, the kept association is detached but untouched otherwise.
        assert_eq!(m.read_u32(0x1300 + 8).unwrap(), 0x1100 + 4);
        assert_eq!(m.read_u32(0x1204).unwrap(), 0);
        assert_eq!(m.read_u32(0x1208).unwrap(), 0);
        assert_eq!(m.read_u32(0x1200).unwrap(), 0x85C6D0);
        assert_eq!(m.read_u32(0x1808).unwrap(), 0x1200);
    }

    #[test]
    fn head_and_tail_unlink() {
        let mut m = list();
        unlink(&mut m, 0x1100).unwrap();
        assert_eq!(associations(&m, 0x1000).unwrap(), vec![0x1200, 0x1300]);
        assert_eq!(m.read_u32(0x1208).unwrap(), 0x1000);
        unlink(&mut m, 0x1300).unwrap();
        assert_eq!(associations(&m, 0x1000).unwrap(), vec![0x1200]);
    }

    #[test]
    fn tasks_without_animations_keep_nothing() {
        let mut m = list();
        w(&mut m, 0x1800, 0x85A29C);
        w(&mut m, 0x1808, 0x1250);
        assert!(keep_task_held(&mut m, 0x1000, &[0x1800]).unwrap().is_empty());
        assert_eq!(associations(&m, 0x1000).unwrap().len(), 3);
    }
}
