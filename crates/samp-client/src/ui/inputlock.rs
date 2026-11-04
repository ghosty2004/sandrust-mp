//! Cursor / input-lock modes (UR §5, gta-patches.md §5 "Input lock").
//!
//! samp's mode setter (A 0x100a0750) and per-frame restore (A 0x100a0630) as a
//! state machine that returns the operations to perform; the Windows glue
//! executes them (byte writes into gta_sa.exe, two `CPad` calls, the D3D cursor).
//! Byte values were read from the setter/restore bodies of the 2021 build:
//!
//! | site | locked | original (1.0 US) | what |
//! |---|---|---|---|
//! | 0x541DF5 | 5 × NOP | `E8 46 F3 FE FF` | keyboard → pad call |
//! | 0x53F417 | 5 × NOP | `E8 B4 7A 20 00` | mouse-state call |
//! | 0x53F41F | `33 C0 0F 84` | `85 C0 0F 8C` | branch after it |
//! | 0x6194A0 | `C3` | `E9` | `RsMouseSetPos` |
//!
//! Every mode of the setter `set(mode, flag)` (A 0x100a0750), read from its body:
//!
//! | mode | guard | does |
//! |---|---|---|
//! | 0 | mode ≠ 0 | countdown := `flag ? 0 : 10`, cursor hidden, mode := 0 (the game code comes back in the per-frame restore) |
//! | 1 | mode ≠ 1 | NOP the keyboard → pad call, mode := 1 (nothing else) |
//! | 2 | none (re-applied every call) | keyboard NOP + mouse lock + deltas 0 + `ClearMouseHistory` + `UpdatePads` + `RsMouseSetPos` := RET + cursor shown |
//! | 3 | mode ≠ 3 | as 2 without the keyboard NOP |
//! | 4 | mode ≠ 4 | mouse **redirect** instead of the mouse lock: the button/axis bytes 0xB7340C/10/14 := 0 and the three store operands at 0x53F47A/0x53F49A/0x53F4B3 point at a dummy (A 0x101501a8), then deltas 0, `ClearMouseHistory`, `UpdatePads`, `RsMouseSetPos` := RET; the cursor is not touched |
//!
//! Other modes are ignored. The restore (A 0x100a0630) also points the three store operands
//! back at 0xB73410/0xB73414/0xB7340C (A 0x100a0590), every time it runs. The call bytes it
//! writes back at 0x53F417 depend on the exe version samp detected (A 0x1026e9a0: 1 = 1.0 US
//! → `E8 B4 7A 20 00`, the 1.0 US original; EU gets `E8 04 7B 20 00`): we only run on 1.0 US.
//!
//! Callers in R4 (objdump of every `call 0x100a0750`): only modes 0, 2 and 3 are passed;
//! modes 1 and 4 have no caller in either R4 build (object editing / selection use 3, or 0
//! with flag 1, A 0x1006dc00 / 0x100737e0; the editor stop A 0x10072c20 passes (0, 1); the
//! editor start A 0x10072480 passes (0, ·), then forces the countdown to 0 and runs the
//! restore at once — [`InputLock::restore_now`]).

/// Keyboard → pad call site.
pub const KEYBOARD_TO_PAD: u32 = 0x541DF5;
pub const KEYBOARD_TO_PAD_ORIGINAL: [u8; 5] = [0xE8, 0x46, 0xF3, 0xFE, 0xFF];
/// Mouse-state call site and the branch after it.
pub const MOUSE_STATE_CALL: u32 = 0x53F417;
pub const MOUSE_STATE_CALL_ORIGINAL: [u8; 5] = [0xE8, 0xB4, 0x7A, 0x20, 0x00];
pub const MOUSE_BRANCH: u32 = 0x53F41F;
pub const MOUSE_BRANCH_ORIGINAL: [u8; 4] = [0x85, 0xC0, 0x0F, 0x8C];
pub const MOUSE_BRANCH_LOCKED: [u8; 4] = [0x33, 0xC0, 0x0F, 0x84];
/// `RsMouseSetPos` first byte.
pub const RS_MOUSE_SET_POS: u32 = 0x6194A0;
/// Mouse deltas zeroed on lock and restore.
pub const MOUSE_DELTA_X: u32 = 0xB73424;
pub const MOUSE_DELTA_Y: u32 = 0xB73428;
/// `CPad::ClearMouseHistory` and `CPad::UpdatePads` (cdecl, no arguments).
pub const PAD_CLEAR_MOUSE_HISTORY: u32 = 0x541BD0;
pub const PAD_UPDATE_PADS: u32 = 0x541DD0;
/// Frames before the restore when mode 0 is requested "with delay".
pub const RESTORE_DELAY_FRAMES: i32 = 10;
/// Mouse button/axis store bytes cleared by mode 4 (CPad mouse state, 1.0 US).
pub const MOUSE_STORE_0: u32 = 0xB7340C;
pub const MOUSE_STORE_1: u32 = 0xB73410;
pub const MOUSE_STORE_2: u32 = 0xB73414;
/// The three store operands mode 4 redirects, with the address each holds originally.
pub const MOUSE_STORE_OPERANDS: [(u32, u32); 3] =
    [(0x53F47A, MOUSE_STORE_1), (0x53F49A, MOUSE_STORE_2), (0x53F4B3, MOUSE_STORE_0)];

const NOP5: [u8; 5] = [0x90; 5];

/// One operation for the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Write(u32, Vec<u8>),
    /// Write a zero u32.
    Zero(u32),
    /// Call a cdecl `void f(void)` in gta_sa.exe.
    Call(u32),
    /// `IDirect3DDevice9::ShowCursor` on the real device.
    ShowCursor(bool),
    /// Write the address of the host's dummy store (samp: A 0x101501a8) as a u32 here.
    MouseSink(u32),
}

/// Cursor mode (game object +0x61) and restore countdown (+0x65).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputLock {
    mode: u8,
    countdown: i32,
}

impl Default for InputLock {
    fn default() -> Self {
        Self::new()
    }
}

impl InputLock {
    /// TRACE-3 U-LOCK-INIT: a restore runs at start-up (a poked NOP at 0x541DF5 was
    /// restored within the first frames), so the countdown starts at 0.
    pub fn new() -> Self {
        Self { mode: 0, countdown: 0 }
    }

    pub fn mode(&self) -> u8 {
        self.mode
    }

    fn lock_ops(out: &mut Vec<Op>, keyboard: bool) {
        if keyboard {
            out.push(Op::Write(KEYBOARD_TO_PAD, NOP5.to_vec()));
        }
        out.push(Op::Write(MOUSE_STATE_CALL, NOP5.to_vec()));
        out.push(Op::Write(MOUSE_BRANCH, MOUSE_BRANCH_LOCKED.to_vec()));
        out.push(Op::Zero(MOUSE_DELTA_X));
        out.push(Op::Zero(MOUSE_DELTA_Y));
        out.push(Op::Call(PAD_CLEAR_MOUSE_HISTORY));
        out.push(Op::Call(PAD_UPDATE_PADS));
        out.push(Op::Write(RS_MOUSE_SET_POS, vec![0xC3]));
        out.push(Op::ShowCursor(true));
    }

    /// The setter (A 0x100a0750) with every mode; see the module table.
    pub fn set_mode(&mut self, mode: u8, flag: bool) -> Vec<Op> {
        match mode {
            0 => self.request_mode0(flag),
            1 => self.request_mode1(),
            2 => self.request_mode2(),
            3 => self.request_mode3(),
            4 => self.request_mode4(),
            _ => Vec::new(),
        }
    }

    /// Mode 1: only the keyboard → pad call is cut.
    pub fn request_mode1(&mut self) -> Vec<Op> {
        if self.mode == 1 {
            return Vec::new();
        }
        self.mode = 1;
        vec![Op::Write(KEYBOARD_TO_PAD, NOP5.to_vec())]
    }

    /// Mode 4: mouse input redirected into a dummy store (no lock, no cursor change).
    pub fn request_mode4(&mut self) -> Vec<Op> {
        if self.mode == 4 {
            return Vec::new();
        }
        let mut out = vec![Op::Zero(MOUSE_STORE_0), Op::Zero(MOUSE_STORE_1), Op::Zero(MOUSE_STORE_2)];
        out.extend(MOUSE_STORE_OPERANDS.iter().map(|&(a, _)| Op::MouseSink(a)));
        out.push(Op::Zero(MOUSE_DELTA_X));
        out.push(Op::Zero(MOUSE_DELTA_Y));
        out.push(Op::Call(PAD_CLEAR_MOUSE_HISTORY));
        out.push(Op::Call(PAD_UPDATE_PADS));
        out.push(Op::Write(RS_MOUSE_SET_POS, vec![0xC3]));
        self.mode = 4;
        out
    }

    /// Editor start (A 0x10072480): mode 0, countdown forced to 0 and the restore run at once.
    pub fn restore_now(&mut self, flag: bool) -> Vec<Op> {
        let mut out = self.request_mode0(flag);
        self.countdown = 0;
        out.extend(self.frame());
        out
    }

    /// Mode 2 (chat input, dialogs, class selection): keyboard and mouse locked,
    /// cursor shown. Re-applied on every request (the original has no "already in
    /// mode 2" check).
    pub fn request_mode2(&mut self) -> Vec<Op> {
        let mut out = Vec::new();
        Self::lock_ops(&mut out, true);
        self.mode = 2;
        out
    }

    /// Mode 3 (scoreboard): like 2 but the keyboard stays connected.
    pub fn request_mode3(&mut self) -> Vec<Op> {
        if self.mode == 3 {
            return Vec::new();
        }
        let mut out = Vec::new();
        Self::lock_ops(&mut out, false);
        self.mode = 3;
        out
    }

    /// Mode 0: hide the cursor and arm the restore (`immediate` → next frame
    /// entry, else after [`RESTORE_DELAY_FRAMES`]).
    pub fn request_mode0(&mut self, immediate: bool) -> Vec<Op> {
        if self.mode == 0 {
            return Vec::new();
        }
        self.countdown = if immediate { 0 } else { RESTORE_DELAY_FRAMES };
        self.mode = 0;
        vec![Op::ShowCursor(false)]
    }

    /// Per-frame game entry (A 0x100a0630): in mode 0 with the countdown at
    /// exactly 0 restore the game code; a positive countdown is decremented.
    pub fn frame(&mut self) -> Vec<Op> {
        if self.mode != 0 {
            return Vec::new();
        }
        if self.countdown > 0 {
            self.countdown -= 1;
            return Vec::new();
        }
        if self.countdown < 0 {
            return Vec::new();
        }
        self.countdown -= 1;
        vec![
            Op::Write(KEYBOARD_TO_PAD, KEYBOARD_TO_PAD_ORIGINAL.to_vec()),
            Op::Write(MOUSE_STATE_CALL, MOUSE_STATE_CALL_ORIGINAL.to_vec()),
            Op::Write(MOUSE_STORE_OPERANDS[0].0, MOUSE_STORE_OPERANDS[0].1.to_le_bytes().to_vec()),
            Op::Write(MOUSE_STORE_OPERANDS[1].0, MOUSE_STORE_OPERANDS[1].1.to_le_bytes().to_vec()),
            Op::Write(MOUSE_STORE_OPERANDS[2].0, MOUSE_STORE_OPERANDS[2].1.to_le_bytes().to_vec()),
            Op::Write(MOUSE_BRANCH, MOUSE_BRANCH_ORIGINAL.to_vec()),
            Op::Zero(MOUSE_DELTA_X),
            Op::Zero(MOUSE_DELTA_Y),
            Op::Call(PAD_CLEAR_MOUSE_HISTORY),
            Op::Call(PAD_UPDATE_PADS),
            Op::Zero(MOUSE_DELTA_X),
            Op::Zero(MOUSE_DELTA_Y),
            Op::Call(PAD_CLEAR_MOUSE_HISTORY),
            Op::Write(RS_MOUSE_SET_POS, vec![0xE9]),
            Op::ShowCursor(false),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode2_then_immediate_restore_on_next_frame() {
        let mut l = InputLock::new();
        // TRACE-3 U-LOCK-INIT: a restore runs at start-up.
        assert!(!l.frame().is_empty());
        assert!(l.frame().is_empty());
        let ops = l.request_mode2();
        assert!(ops.contains(&Op::Write(KEYBOARD_TO_PAD, NOP5.to_vec())));
        assert!(ops.contains(&Op::ShowCursor(true)));
        assert!(l.frame().is_empty());
        assert_eq!(l.request_mode0(true), vec![Op::ShowCursor(false)]);
        let r = l.frame();
        assert_eq!(r[0], Op::Write(KEYBOARD_TO_PAD, KEYBOARD_TO_PAD_ORIGINAL.to_vec()));
        assert!(l.frame().is_empty());
    }

    #[test]
    fn modes_1_and_4_and_guards() {
        let mut l = InputLock::new();
        l.frame();
        assert_eq!(l.set_mode(1, false), vec![Op::Write(KEYBOARD_TO_PAD, NOP5.to_vec())]);
        assert!(l.set_mode(1, false).is_empty());
        let ops = l.set_mode(4, false);
        assert_eq!(l.mode(), 4);
        assert!(ops.contains(&Op::MouseSink(0x53F47A)));
        assert!(!ops.iter().any(|o| matches!(o, Op::ShowCursor(_))));
        assert!(!ops.contains(&Op::Write(KEYBOARD_TO_PAD, NOP5.to_vec())));
        assert!(l.set_mode(4, false).is_empty());
        assert!(l.set_mode(9, false).is_empty());
        // (0, 1) → restore on the next frame, which also puts the store operands back.
        assert_eq!(l.set_mode(0, true), vec![Op::ShowCursor(false)]);
        let r = l.frame();
        assert!(r.contains(&Op::Write(0x53F4B3, 0xB7340Cu32.to_le_bytes().to_vec())));
        // Mode 2 has no guard.
        assert!(!l.set_mode(2, false).is_empty());
        assert!(!l.set_mode(2, false).is_empty());
        // Editor start: restore at once even with the delay flag clear.
        let r = l.restore_now(false);
        assert_eq!(r[0], Op::ShowCursor(false));
        assert!(r.contains(&Op::Write(KEYBOARD_TO_PAD, KEYBOARD_TO_PAD_ORIGINAL.to_vec())));
        assert!(l.frame().is_empty());
    }

    #[test]
    fn delayed_restore_after_ten_frames() {
        let mut l = InputLock::new();
        l.request_mode2();
        l.request_mode0(false);
        for _ in 0..RESTORE_DELAY_FRAMES {
            assert!(l.frame().is_empty());
        }
        assert!(!l.frame().is_empty());
        assert!(l.frame().is_empty());
    }
}
