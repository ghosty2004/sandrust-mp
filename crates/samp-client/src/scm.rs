//! Our own minimal replacement for the story script `main.scm`, used when no samp.saa is available
//! (otherwise `filehook` serves samp's own script). samp keeps GTA's script engine running but replaces
//! the story script; we may not ship samp's file, so [`build_main_scm`] generates an equivalent: the chunk
//! header `CTheScripts::Init` expects, a global-variable area, no objects, missions or streamed scripts,
//! and a short main thread that sets up the world, creates the local player and then only waits.
//! `dll` writes it once per run and serves it in place of `data\script\main.scm`; `script.img` stays
//! stock (with no streamed scripts declared the game never reads it). Background: docs/phase3/m2-notes.md,
//! "How the story script is kept out".

use crate::script::{Arg, encode, op};

/// Size of the global-variable area (offset of the first chunk after it).
/// The same size as in samp's script, so global offsets used by game code and servers stay valid.
pub const GLOBAL_SPACE: u32 = 46268;
/// Global variable that receives the player handle.
pub const VAR_PLAYER: u16 = 8;
/// Global variable that receives the player's char handle.
pub const VAR_PLAYER_CHAR: u16 = 12;

const GOTO: u16 = 0x0002;
const WAIT: u16 = 0x0001;

fn chunk_header(out: &mut Vec<u8>, next: u32, index: u8) {
    out.extend_from_slice(&GOTO.to_le_bytes());
    out.push(0x01); // int32 parameter
    out.extend_from_slice(&next.to_le_bytes());
    out.push(index);
}

fn instr(out: &mut Vec<u8>, opcode: u16, args: &[Arg]) {
    let mut b = Vec::new();
    encode(opcode, args, &mut b);
    out.extend_from_slice(&b);
}

/// Builds the whole SCM file.
pub fn build_main_scm(player_pos: [f32; 3]) -> Vec<u8> {
    let mut s = Vec::with_capacity(GLOBAL_SPACE as usize + 512);
    // Chunk 's': global variables (zero-filled) up to GLOBAL_SPACE.
    chunk_header(&mut s, GLOBAL_SPACE, b's');
    s.resize(GLOBAL_SPACE as usize, 0);
    // Chunk 0: used objects. One empty entry (the game skips entry 0).
    let next = s.len() as u32 + 8 + 4 + 24;
    chunk_header(&mut s, next, 0);
    s.extend_from_slice(&1u32.to_le_bytes());
    s.extend_from_slice(&[0u8; 24]);
    // Chunk 1: script file info (main size is patched below), no missions.
    let info_at = s.len();
    let next = s.len() as u32 + 0x18;
    chunk_header(&mut s, next, 1);
    s.extend_from_slice(&0u32.to_le_bytes()); // main script size (patched)
    s.extend_from_slice(&0u32.to_le_bytes()); // largest mission
    s.extend_from_slice(&0u16.to_le_bytes()); // missions
    s.extend_from_slice(&0u16.to_le_bytes()); // exclusive missions
    s.extend_from_slice(&0u32.to_le_bytes()); // largest mission local vars
    // Chunk 2: streamed scripts: none.
    let next = s.len() as u32 + 0x10;
    chunk_header(&mut s, next, 2);
    s.extend_from_slice(&0u32.to_le_bytes());
    s.extend_from_slice(&0u32.to_le_bytes());
    // Chunk 3.
    let next = s.len() as u32 + 0xC;
    chunk_header(&mut s, next, 3);
    s.extend_from_slice(&0u32.to_le_bytes());
    // Chunk 4: extra info; its "next" is the start of the main thread code.
    let next = s.len() as u32 + 0x10;
    chunk_header(&mut s, next, 4);
    s.extend_from_slice(&GLOBAL_SPACE.to_le_bytes());
    s.extend_from_slice(&0u32.to_le_bytes());

    // MAIN: world setup and the local player, then idle forever.
    let [x, y, z] = player_pos;
    instr(&mut s, op::SET_TIME_OF_DAY, &[Arg::Int(12), Arg::Int(0)]);
    instr(&mut s, op::SET_PED_DENSITY_MULTIPLIER, &[Arg::Float(0.0)]);
    instr(&mut s, op::SET_CAR_DENSITY_MULTIPLIER, &[Arg::Float(0.0)]);
    // samp's script caps the wanted level at 0: no police, no forbidden-territory wanted level.
    instr(&mut s, op::SET_MAX_WANTED_LEVEL, &[Arg::Int(0)]);
    instr(&mut s, op::REQUEST_COLLISION, &[Arg::Float(x), Arg::Float(y)]);
    instr(
        &mut s,
        op::CREATE_PLAYER,
        &[Arg::Int(0), Arg::Float(x), Arg::Float(y), Arg::Float(z), Arg::Global(VAR_PLAYER)],
    );
    instr(&mut s, op::GET_PLAYER_CHAR, &[Arg::Global(VAR_PLAYER), Arg::Global(VAR_PLAYER_CHAR)]);
    instr(&mut s, WAIT, &[Arg::Int(0)]);
    instr(&mut s, op::BUILD_PLAYER_MODEL, &[Arg::Global(VAR_PLAYER)]);
    instr(&mut s, op::SET_PLAYER_CONTROL, &[Arg::Global(VAR_PLAYER), Arg::Int(0)]);
    instr(&mut s, op::SWITCH_ROADS_OFF, &[-4000.0, -4000.0, -500.0, 4000.0, 4000.0, 1500.0].map(Arg::Float));
    let lp = s.len() as i32;
    instr(&mut s, WAIT, &[Arg::Int(1000)]);
    instr(&mut s, GOTO, &[Arg::Int(lp)]);

    let size = s.len() as u32;
    s[info_at + 8..info_at + 12].copy_from_slice(&size.to_le_bytes());
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(s: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(s[o..o + 4].try_into().unwrap())
    }

    /// Walks the chunk chain the way `CTheScripts::GetSCMChunk` does.
    #[test]
    fn chunk_chain_is_valid() {
        let s = build_main_scm([1.0, 2.0, 3.0]);
        let mut at = 0usize;
        let mut seen = Vec::new();
        for _ in 0..6 {
            assert_eq!(&s[at..at + 3], &[0x02, 0x00, 0x01], "GOTO at {at:#x}");
            seen.push(s[at + 7]);
            let next = u32_at(&s, at + 3) as usize;
            assert!(next > at && next < s.len());
            if s[at + 7] == 4 {
                at = next;
                break;
            }
            at = next;
        }
        assert_eq!(seen, [b's', 0, 1, 2, 3, 4]);
        assert_eq!(u32_at(&s, 3), GLOBAL_SPACE);
        // First instruction of MAIN: SET_TIME_OF_DAY.
        assert_eq!(&s[at..at + 2], &op::SET_TIME_OF_DAY.to_le_bytes());
        // Main size covers the whole file.
        let info = GLOBAL_SPACE as usize + 8 + 4 + 24;
        assert_eq!(s[info + 7], 1);
        assert_eq!(u32_at(&s, info + 8) as usize, s.len());
        assert!(s.len() < 200_000);
    }

    #[test]
    fn ends_in_a_wait_loop() {
        let s = build_main_scm([0.0; 3]);
        let n = s.len();
        // ... WAIT 1000 (0001 01 e8030000) ; GOTO loop (0002 01 <loop>)
        assert_eq!(&s[n - 7..n - 5], &[0x02, 0x00]);
        let target = u32_at(&s, n - 4) as usize;
        assert_eq!(&s[target..target + 7], &[0x01, 0x00, 0x01, 0xE8, 0x03, 0x00, 0x00]);
    }
}
