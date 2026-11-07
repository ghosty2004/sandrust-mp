//! Gates the CJ walk style in `CPlayerPed::ProcessAnimGroups`.
//!
//! GTA picks the player's move-anim group from CJ's fat/muscle stats and stores it in
//! `CPed::m_nAnimGroup`. samp NOPs that store and calls a stub in its place that performs the store
//! only when InitGame's "CJ walk" flag is set. With the flag off every player ped, local and remote,
//! keeps the anim group its skin got from `SetModelIndex`. The stub preserves every register; the
//! caller's flags do not matter. Spec: docs/re/subsystems/remote-control-onfoot.md, "Animation groups".

/// The move-anim-group store in `ProcessAnimGroups` that the stub call replaces.
pub const STORE_SITE: u32 = 0x609A4E;
/// `CPed::m_nAnimGroup`.
pub const PED_ANIM_GROUP: u32 = 0x4D4;

#[cfg(windows)]
pub use hook::{install, set_cj_walk};

#[cfg(windows)]
mod hook {
    use std::arch::naked_asm;
    use std::sync::atomic::{AtomicU8, Ordering};

    use crate::patch::{OP_CALL, OP_NOP, PatchError, Patcher, ProcessMemory, encode_branch};

    /// 1 = InitGame's "CJ walk" flag is set (0 before InitGame, like samp without a netgame).
    static CJ_WALK: AtomicU8 = AtomicU8::new(0);

    pub fn set_cj_walk(on: bool) {
        CJ_WALK.store(u8::from(on), Ordering::SeqCst);
    }

    // Called with the ped in ESI and the new group in EAX. Mirrors samp.dll 0x100A3F60.
    #[unsafe(naked)]
    unsafe extern "C" fn anim_group_stub() {
        naked_asm!(
            "cmp byte ptr [{flag}], 0",
            "je 2f",
            "mov dword ptr [esi + {off}], eax",
            "2:",
            "ret",
            flag = sym CJ_WALK,
            off = const super::PED_ANIM_GROUP,
        )
    }

    /// Writes the call over the store (already NOPed by the startup patch table).
    pub fn install(patcher: &mut Patcher, mem: &mut ProcessMemory) -> Result<(), PatchError> {
        let stub = anim_group_stub as unsafe extern "C" fn() as usize as u32;
        let call = encode_branch(OP_CALL, super::STORE_SITE, stub);
        patcher.write(mem, "ProcessAnimGroups CJ-walk gate", super::STORE_SITE, &call, Some(&[OP_NOP; 5]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patches::testutil;

    #[test]
    fn rows_43_and_187_share_the_store_site() {
        let csv = testutil::csv_rows();
        for row in [43, 187] {
            assert_eq!(testutil::field_u32(&csv[&row], "gta_addr"), STORE_SITE, "row {row}");
        }
        // The original bytes: mov [esi+0x4D4], eax.
        assert_eq!(testutil::unhex(&csv[&187]["original_bytes_hex"]), [0x89, 0x86, 0xd4, 0x04, 0x00, 0x00]);
        assert_eq!(PED_ANIM_GROUP.to_le_bytes(), [0xd4, 0x04, 0x00, 0x00]);
    }
}
