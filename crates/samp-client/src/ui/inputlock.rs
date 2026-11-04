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
//! Mode 1 (keyboard only) and mode 4 (object editing, with the mouse-pointer
//! redirect A 0x100a0540/0x100a0590) are not used before M6/M8 and are not
//! modelled.

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
    /// TODO-unknown (m4-notes U-LOCK-INIT): the countdown's initial value; −1 means
    /// no restore runs before the first mode change.
    pub fn new() -> Self {
        Self { mode: 0, countdown: -1 }
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
