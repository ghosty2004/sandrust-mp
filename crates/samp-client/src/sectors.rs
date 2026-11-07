//! Relocation of `CWorld::ms_aSectors`, GTA's 120 × 120 sector grid, into a larger buffer of ours.
//! SA:MP creates remote players with player indices 2…209, so GTA's own `CWorld::Players[slot]` indexing
//! runs past the two-entry array into the grid's memory (docs/re/subsystems/remote-control-onfoot.md,
//! "Why a player index above 1 does not corrupt memory"). [`relocate`] moves every listed reference
//! all-or-nothing (a half-moved grid would corrupt the world), then zeroes the original grid.
//! Spec: docs/re/subsystems/gta-patches.md, "Sector array relocation".

use crate::patch::{Memory, PatchError, Patcher};

/// `CWorld::ms_aSectors` (named via gta-reversed `World.h`).
pub const ORIGINAL_GRID: u32 = 0xB7D0B8;
/// 120 × 120 × 8 bytes.
pub const GRID_BYTES: u32 = 115_200;
/// Size of the replacement buffer (the same as samp's).
pub const BUFFER_BYTES: u32 = 160_000;

/// What the relocated reference points at, relative to our buffer.
pub const BASE: u32 = 0;
/// The second list of sector 0 (`ms_aSectors + 4`).
pub const BASE_PLUS_4: u32 = 4;
/// End of an original-size grid (`ms_aRepeatSectors` in `FindObjectsOfTypeInRange`).
pub const GRID_END: u32 = GRID_BYTES;
/// Loop bounds `cmp reg, 0xB992B8` move to the end of the whole buffer.
pub const BUFFER_END: u32 = BUFFER_BYTES;

/// One 4-byte code/data reference to the grid.
#[derive(Debug, Clone, Copy)]
pub struct SectorRef {
    pub addr: u32,
    /// Value the dword holds in the unmodified 1.0 US executable.
    pub original: u32,
    /// New value = buffer base + `offset`.
    pub offset: u32,
    /// Row in docs/re/data/gta-patches-r4-annotated.csv.
    pub csv_row: u32,
}

const fn r(addr: u32, original: u32, offset: u32, csv_row: u32) -> SectorRef {
    SectorRef { addr, original, offset, csv_row }
}

/// Every reference samp moves: absolute pointers, imm32 operands at instruction+3, references to the
/// `+4` field, the repeat-sector base and the loop bounds. One site from samp's absolute-pointer list,
/// 0x40914E, is left out on purpose: the dword there is the opcode/ModRM/SIB of an instruction in
/// `CStreaming::DeleteAllRwObjects`, not its displacement (that function is made to return at once in
/// `patches.rs` instead). Rows of the CSV that repeat a site listed here are not repeated.
pub const SECTOR_REFS: &[SectorRef] = &[
    r(0x5dc7aa, 0xB7D0B8, BASE, 262),
    r(0x41a85d, 0xB7D0B8, BASE, 263),
    r(0x41a864, 0xB7D0B8, BASE, 264),
    r(0x408259, 0xB7D0B8, BASE, 265),
    r(0x711b32, 0xB7D0B8, BASE, 266),
    r(0x699cf8, 0xB7D0B8, BASE, 267),
    r(0x4092ec, 0xB7D0B8, BASE, 268),
    r(0x408702, 0xB7D0B8, BASE, 270),
    r(0x564220, 0xB7D0B8, BASE, 271),
    r(0x564172, 0xB7D0B8, BASE, 272),
    r(0x563845, 0xB7D0B8, BASE, 273),
    r(0x84e9c2, 0xB7D0B8, BASE, 274),
    r(0x85652d, 0xB7D0B8, BASE, 275),
    r(0x40d68f, 0xB7D0B8, BASE, 276),
    r(0x5664da, 0xB7D0B8, BASE, 277),
    r(0x566589, 0xB7D0B8, BASE, 278),
    r(0x408709, 0xB7D0B8, BASE, 279),
    r(0x56b3b4, 0xB7D0B8, BASE, 280),
    r(0x56ad94, 0xB7D0B8, BASE, 281),
    r(0x56a862, 0xB7D0B8, BASE, 282),
    r(0x5675fd, 0xB7D0B8, BASE, 283),
    r(0x56cd87, 0xB7D0B8, BASE, 284),
    r(0x56cc7c, 0xB7D0B8, BASE, 285),
    r(0x56cb54, 0xB7D0B8, BASE, 286),
    r(0x56ca4d, 0xB7D0B8, BASE, 287),
    r(0x56c667, 0xB7D0B8, BASE, 288),
    r(0x56c56c, 0xB7D0B8, BASE, 289),
    r(0x56c448, 0xB7D0B8, BASE, 290),
    r(0x56c344, 0xB7D0B8, BASE, 291),
    r(0x56bd49, 0xB7D0B8, BASE, 292),
    r(0x56bc56, 0xB7D0B8, BASE, 293),
    r(0x56be59, 0xB7D0B8, BASE, 294),
    r(0x56a943, 0xB7D0B8, BASE, 295),
    r(0x567738, 0xB7D0B8, BASE, 296),
    r(0x54673b, 0xB7D0B8, BASE, 297),
    r(0x54bb26, 0xB7D0B8, BASE, 298),
    r(0x6e31ad, 0xB7D0B8, BASE, 299),
    r(0x40dc2c, 0xB7D0B8, BASE, 300),
    r(0x534a0c, 0xB7D0B8, BASE, 301),
    r(0x534d6e, 0xB7D0B8, BASE, 302),
    r(0x564b5c, 0xB7D0B8, BASE, 303),
    r(0x564dac, 0xB7D0B8, BASE, 304),
    r(0x67ff60, 0xB7D0B8, BASE, 305),
    r(0x568cbc, 0xB7D0B8, BASE, 306),
    r(0x568efe, 0xB7D0B8, BASE, 307),
    r(0x569f5a, 0xB7D0B8, BASE, 308),
    r(0x56953a, 0xB7D0B8, BASE, 309),
    r(0x56912a, 0xB7D0B8, BASE, 310),
    r(0x56b4b8, 0xB7D0B8, BASE, 311),
    r(0x56b597, 0xB7D0B8, BASE, 312),
    r(0x56b2c6, 0xB7D0B8, BASE, 313),
    r(0x56af77, 0xB7D0B8, BASE, 314),
    r(0x56ae98, 0xB7D0B8, BASE, 315),
    r(0x56bf52, 0xB7D0B8, BASE, 316),
    r(0x56aca6, 0xB7D0B8, BASE, 317),
    r(0x56a769, 0xB7D0B8, BASE, 318),
    r(0x56a688, 0xB7D0B8, BASE, 319),
    r(0x70b9bd, 0xB7D0B8, BASE, 320),
    r(0x5647a0, 0xB7D0B8, BASE, 321),
    r(0x70acb5, 0xB7D0B8, BASE, 322),
    r(0x6063ca, 0xB7D0B8, BASE, 323),
    r(0x699d01, 0xB7D0B8, BASE, 324),
    r(0x40e064, 0xB7D0B8, BASE, 326),
    r(0x40df61, 0xB7D0B8, BASE, 327),
    r(0x40ddd1, 0xB7D0B8, BASE, 328),
    r(0x40db11, 0xB7D0B8, BASE, 329),
    r(0x40d98f, 0xB7D0B8, BASE, 330),
    r(0x1566858, 0xB7D0B8, BASE, 331),
    r(0x4091c8, 0xB7D0BC, BASE_PLUS_4, 332),
    r(0x40936a, 0xB7D0BC, BASE_PLUS_4, 333),
    r(0x40d9c8, 0xB7D0BC, BASE_PLUS_4, 334),
    r(0x40db4a, 0xB7D0BC, BASE_PLUS_4, 335),
    r(0x40dc64, 0xB7D0BC, BASE_PLUS_4, 336),
    r(0x40de0a, 0xB7D0BC, BASE_PLUS_4, 337),
    r(0x40df9a, 0xB7D0BC, BASE_PLUS_4, 338),
    r(0x40e09d, 0xB7D0BC, BASE_PLUS_4, 339),
    r(0x534a9b, 0xB7D0BC, BASE_PLUS_4, 340),
    r(0x534dfd, 0xB7D0BC, BASE_PLUS_4, 341),
    r(0x71cdb3, 0xB7D0BC, BASE_PLUS_4, 342),
    r(0x564dc7, 0xB992B8, GRID_END, 138),
    r(0x5634a6, 0xB992B8, BUFFER_END, 343),
    r(0x5638df, 0xB992B8, BUFFER_END, 344),
    r(0x56420f, 0xB992B8, BUFFER_END, 345),
    r(0x564283, 0xB992B8, BUFFER_END, 346),
];

/// Row of the annotated CSV that records zeroing the original grid after the move.
pub const ZERO_FILL_CSV_ROW: u32 = 347;

/// Moves every reference to `buffer` (which must be `BUFFER_BYTES` long and
/// zeroed) and zeroes the original grid. Nothing is written unless every
/// reference still holds its original value.
pub fn relocate<M: Memory>(patcher: &mut Patcher, mem: &mut M, buffer: u32) -> Result<usize, PatchError> {
    for s in SECTOR_REFS {
        let found = mem.read_u32(s.addr)?;
        if found != s.original {
            return Err(PatchError::Mismatch {
                addr: s.addr,
                expected: s.original.to_le_bytes().to_vec(),
                found: found.to_le_bytes().to_vec(),
            });
        }
    }
    for s in SECTOR_REFS {
        let v = buffer.wrapping_add(s.offset);
        patcher.write(mem, "sector grid reference", s.addr, &v.to_le_bytes(), Some(&s.original.to_le_bytes()))?;
    }
    patcher.write(mem, "zero CWorld::ms_aSectors", ORIGINAL_GRID, &vec![0u8; GRID_BYTES as usize], None)?;
    Ok(SECTOR_REFS.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::fake::FakeMemory;
    use crate::patches::testutil;

    #[test]
    fn table_matches_csv() {
        let csv = testutil::csv_rows();
        for s in SECTOR_REFS {
            let row = &csv[&s.csv_row];
            assert_eq!(testutil::field_u32(row, "gta_addr"), s.addr, "row {}", s.csv_row);
            assert_eq!(row["size"], "4");
            assert!(row["kind"].starts_with("pointer-redirect"), "row {}", s.csv_row);
            assert_eq!(testutil::unhex(&row["original_bytes_hex"]), s.original.to_le_bytes(), "row {}", s.csv_row);
            // samp's target, relative to its sector buffer (docs/re/addresses/sectors.md).
            let a = row["kind"].split("A:").nth(1).and_then(|t| t.split_whitespace().next()).unwrap();
            let a = u32::from_str_radix(a.trim_start_matches("0x"), 16).unwrap();
            assert_eq!(a - 0x101A_2490, s.offset, "row {}", s.csv_row);
        }
        let fill = &csv[&ZERO_FILL_CSV_ROW];
        assert_eq!(testutil::field_u32(fill, "gta_addr"), ORIGINAL_GRID);
        assert_eq!(fill["size"], GRID_BYTES.to_string());
    }

    #[test]
    fn covers_every_relocation_row_once() {
        let mut addrs: Vec<u32> = SECTOR_REFS.iter().map(|s| s.addr).collect();
        addrs.sort_unstable();
        addrs.dedup();
        assert_eq!(addrs.len(), SECTOR_REFS.len(), "duplicate site");
        // 14 absolute pointers (minus 0x40914E) + 56 imm32 (minus the duplicate
        // 0x41A864) + 11 (+4) + 1 + 4 loop bounds.
        assert_eq!(SECTOR_REFS.len(), 13 + 55 + 11 + 1 + 4);
        assert!(!addrs.contains(&0x40914E));
    }

    #[test]
    fn originals_match_reference_exe() {
        let Some(exe) = testutil::Exe::load() else { return };
        for s in SECTOR_REFS {
            assert_eq!(exe.read_va(s.addr, 4), s.original.to_le_bytes(), "{:#x}", s.addr);
        }
    }

    #[test]
    fn all_or_nothing() {
        let mut m = FakeMemory::new(0x0040_0000, 0x0130_0000);
        for s in SECTOR_REFS {
            m.set(s.addr, &s.original.to_le_bytes());
        }
        m.set(ORIGINAL_GRID + 8, &[0xAA]);
        // One site changed: nothing may be written.
        let bad = SECTOR_REFS[40];
        m.set(bad.addr, &[0, 0, 0, 0]);
        let mut p = Patcher::new();
        assert!(relocate(&mut p, &mut m, 0x0120_0000).is_err());
        assert_eq!(m.read_u32(SECTOR_REFS[0].addr).unwrap(), SECTOR_REFS[0].original);
        assert!(p.applied().is_empty());
        // Restored site: everything moves, the grid is zeroed.
        m.set(bad.addr, &bad.original.to_le_bytes());
        assert_eq!(relocate(&mut p, &mut m, 0x0120_0000).unwrap(), SECTOR_REFS.len());
        for s in SECTOR_REFS {
            assert_eq!(m.read_u32(s.addr).unwrap(), 0x0120_0000 + s.offset);
        }
        assert_eq!(m.get(ORIGINAL_GRID + 8, 1), &[0]);
    }
}
