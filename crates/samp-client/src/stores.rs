//! Enlarged model-info stores (gta-patches.md §4.2, CSV rows 132 and 248–261).
//!
//! samp moves `CModelInfo::ms_atomicModelInfoStore`'s entries to a 20000-entry
//! array and `ms_pedModelInfoStore`'s entries to a 319-entry array. Each new
//! entry is zeroed and gets the class vtable, which is all samp writes
//! (B 0x100AAA20 / 0x100AA9D0). The store counts stay where they are; only the
//! listed references to the entry arrays move.
//!
//! The data samp serves from samp.saa needs this: its peds.ide plus SAMP.ide
//! define 311 ped models (the stock store holds 278) and SAMP.ide adds 1433
//! objects to roughly 14000 stock ones (the stock atomic store holds 14000).
//!
//! As with the sector grid, each store moves all-or-nothing.

use crate::patch::{Memory, PatchError, Patcher};

/// One relocated store.
#[derive(Debug)]
pub struct Store {
    pub name: &'static str,
    /// Address of entry 0 in the unmodified executable.
    pub original_entries: u32,
    pub entry_size: u32,
    pub entries: u32,
    /// Class vtable written into every new entry.
    pub vtable: u32,
    /// Code references to `original_entries`: (site, CSV row).
    pub refs: &'static [(u32, u32)],
}

impl Store {
    pub const fn bytes(&self) -> u32 {
        self.entry_size * self.entries
    }
}

/// `CModelInfo::ms_atomicModelInfoStore` (count at 0xAAE950, entries at
/// 0xAAE954, 32-byte `CAtomicModelInfo`, vtable 0x85BBF0); samp's list at
/// B .data 0x10116720.
pub const ATOMIC: Store = Store {
    name: "atomic model-info store",
    original_entries: 0xAAE954,
    entry_size: 0x20,
    entries: 20000,
    vtable: 0x85BBF0,
    refs: &[
        (0x4c63f2, 248),
        (0x4c662d, 249),
        (0x4c6822, 250),
        (0x4c6829, 251),
        (0x4c6877, 252),
        (0x4c6881, 253),
        (0x4c6890, 254),
        (0x4c68a5, 255),
        (0x4c68f3, 256),
        (0x4c6932, 257),
        (0x4c6971, 258),
        (0x4c69b0, 259),
        (0x4c69ef, 260),
        (0x4c6a2e, 261),
    ],
};

/// `CModelInfo::ms_pedModelInfoStore` entries (0x44-byte `CPedModelInfo`,
/// vtable 0x85BDC0). samp moves only the reference in `AddPedModel`.
pub const PED: Store = Store {
    name: "ped model-info store",
    original_entries: 0xB478FC,
    entry_size: 0x44,
    entries: 319,
    vtable: 0x85BDC0,
    refs: &[(0x4c67ad, 132)],
};

pub const STORES: [&Store; 2] = [&ATOMIC, &PED];

/// A new, zeroed entry array with the class vtable in every entry.
pub fn new_entries(store: &Store) -> Vec<u8> {
    let mut b = vec![0u8; store.bytes() as usize];
    for e in b.chunks_exact_mut(store.entry_size as usize) {
        e[..4].copy_from_slice(&store.vtable.to_le_bytes());
    }
    b
}

/// Points every reference at `buffer` (from [`new_entries`], never freed).
/// Nothing is written unless every reference holds its original value.
pub fn relocate<M: Memory>(patcher: &mut Patcher, mem: &mut M, store: &Store, buffer: u32) -> Result<(), PatchError> {
    for &(site, _) in store.refs {
        let found = mem.read_u32(site)?;
        if found != store.original_entries {
            return Err(PatchError::Mismatch {
                addr: site,
                expected: store.original_entries.to_le_bytes().to_vec(),
                found: found.to_le_bytes().to_vec(),
            });
        }
    }
    for &(site, _) in store.refs {
        patcher.write(mem, store.name, site, &buffer.to_le_bytes(), Some(&store.original_entries.to_le_bytes()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::fake::FakeMemory;
    use crate::patches::testutil;

    #[test]
    fn tables_match_csv() {
        let csv = testutil::csv_rows();
        for (store, samp_buf) in [(&ATOMIC, 0x101C_EA50u32), (&PED, 0x101C_9590)] {
            for &(site, row) in store.refs {
                let r = &csv[&row];
                assert_eq!(testutil::field_u32(r, "gta_addr"), site, "row {row}");
                assert_eq!(r["size"], "4");
                assert!(r["kind"].contains(&format!("A:{samp_buf:#x}")), "row {row}: {}", r["kind"]);
                assert_eq!(
                    testutil::unhex(&r["original_bytes_hex"]),
                    store.original_entries.to_le_bytes(),
                    "row {row}"
                );
            }
        }
        assert_eq!(ATOMIC.refs.len(), 14);
        // samp's buffers: entries up to the next samp array.
        assert_eq!(0x101C_EA50 + ATOMIC.bytes(), 0x1026_AE50);
        // 319 whole 0x44-byte entries fit before the atomic array (4 bytes spare).
        assert_eq!(0x101C_EA50 - (0x101C_9590 + PED.bytes()), 4);
    }

    #[test]
    fn originals_match_reference_exe() {
        let Some(exe) = testutil::Exe::load() else { return };
        for s in STORES {
            for &(site, _) in s.refs {
                assert_eq!(exe.read_va(site, 4), s.original_entries.to_le_bytes(), "{:#x}", site);
            }
        }
    }

    #[test]
    fn all_or_nothing_and_vtables() {
        let mut m = FakeMemory::new(0x0040_0000, 0x0100_0000);
        for &(site, _) in PED.refs {
            m.set(site, &PED.original_entries.to_le_bytes());
        }
        let buf = 0x0120_0000 - PED.bytes() - 0x10;
        let mut p = Patcher::new();
        for &(site, _) in ATOMIC.refs {
            m.set(site, &ATOMIC.original_entries.to_le_bytes());
        }
        m.set(ATOMIC.refs[5].0, &[1, 2, 3, 4]);
        assert!(relocate(&mut p, &mut m, &ATOMIC, 0x0100_0000).is_err());
        assert_eq!(m.read_u32(ATOMIC.refs[0].0).unwrap(), ATOMIC.original_entries);
        assert!(p.applied().is_empty());
        relocate(&mut p, &mut m, &PED, buf).unwrap();
        assert_eq!(m.read_u32(0x4c67ad).unwrap(), buf);
        let e = new_entries(&PED);
        assert_eq!(e.len(), 319 * 0x44);
        assert_eq!(e[..8], [0xC0, 0xBD, 0x85, 0x00, 0, 0, 0, 0]);
        assert_eq!(e[318 * 0x44..318 * 0x44 + 4], PED.vtable.to_le_bytes());
    }
}
