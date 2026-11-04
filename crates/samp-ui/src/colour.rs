//! Embedded colour codes `{RRGGBB}` and colour conversions (UR §3.3).
//!
//! Colours are D3DCOLOR `0xAARRGGBB` unless a name says otherwise. Wire colours
//! are RGBA (`0xRRGGBBAA`) and become ARGB with alpha forced to FF.

/// Length of one colour code: `{` + six hex digits + `}`.
pub const CODE_LEN: usize = 8;

/// Player chat text colour (chat object +0x122, UR §7.1).
pub const CHAT_PLAYER_TEXT: u32 = 0xFFFF_FFFF;
/// Type-4 info lines: local feedback and command output (UR §7.1).
pub const CHAT_INFO: u32 = 0xFF88_AA62;
/// Type-8 debug lines (UR §7.1).
pub const CHAT_DEBUG: u32 = 0xFFA9_C4E4;
/// Default dialog body / tablist header colour (UR §13.3).
pub const DIALOG_BODY: u32 = 0xFFA9_C4E4;
/// Outline colour of every outlined text (UR §3.4).
pub const OUTLINE: u32 = 0xFF00_0000;

/// Wire RGBA → ARGB with alpha forced to FF: `(c >> 8) | 0xFF000000` (UCA conventions).
pub const fn rgba_to_argb(rgba: u32) -> u32 {
    (rgba >> 8) | 0xFF00_0000
}

/// Parses a colour code at the start of `s` and returns its `0xRRGGBB` value.
///
/// A code is exactly `{` + six hex digits + `}` (UR §3.3, A 0x100b5fd0); anything
/// else is literal text. Hex digits are accepted in both cases (notes U-HEXCASE).
pub fn parse_code(s: &[u8]) -> Option<u32> {
    if s.len() < CODE_LEN || s[0] != b'{' || s[7] != b'}' {
        return None;
    }
    let mut v = 0u32;
    for &c in &s[1..7] {
        v = (v << 4) | (c as char).to_digit(16)?;
    }
    Some(v)
}

/// Removes every valid colour code (UR §3.3, strip helper A 0x100b6100).
///
/// Works like an in-place removal: after a code is removed the scan continues at
/// the same index, so a code that only forms after a removal at that index is
/// removed too (notes U-STRIP).
pub fn strip_codes(s: &[u8]) -> Vec<u8> {
    let mut out = s.to_vec();
    let mut i = 0;
    while i < out.len() {
        if parse_code(&out[i..]).is_some() {
            out.drain(i..i + CODE_LEN);
        } else {
            i += 1;
        }
    }
    out
}

/// True for the characters D3DX does not emit a glyph for (UR §3.3 step 2).
pub const fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

/// One drawn glyph and the colour it gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// Index of the glyph in the stripped text.
    pub index: usize,
    pub byte: u8,
    pub colour: u32,
}

/// Per-glyph colouring exactly as the colour-code wrapper font does it (UR §3.3).
///
/// D3DX draws the *stripped* text and reports every visible glyph (not spaces,
/// tabs, CR, LF). For each, a cursor in the *original* text skips blanks, consumes
/// any run of codes (each sets `0xFF000000 | RRGGBB`) and then steps over one
/// character. So `base`'s alpha applies only until the first code, and odd inputs
/// (a code directly followed by a blank) shift the colouring like the original.
pub fn glyph_colours(text: &[u8], base: u32) -> Vec<Glyph> {
    let stripped = strip_codes(text);
    let mut colour = base;
    let mut cur = 0usize;
    let mut out = Vec::new();
    for (index, &byte) in stripped.iter().enumerate() {
        if is_blank(byte) {
            continue;
        }
        while cur < text.len() && is_blank(text[cur]) {
            cur += 1;
        }
        while let Some(rgb) = text.get(cur..).and_then(parse_code) {
            colour = 0xFF00_0000 | rgb;
            cur += CODE_LEN;
        }
        cur += 1;
        out.push(Glyph { index, byte, colour });
    }
    out
}

/// A run of stripped text drawn in one colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColourRun {
    pub text: Vec<u8>,
    pub colour: u32,
}

/// Groups [`glyph_colours`] into runs for renderers that draw strings, not glyphs.
/// Blanks join the run before them (their colour is irrelevant: they are not drawn).
pub fn colour_runs(text: &[u8], base: u32) -> Vec<ColourRun> {
    let stripped = strip_codes(text);
    let glyphs = glyph_colours(text, base);
    let mut colours = vec![None; stripped.len()];
    for g in &glyphs {
        colours[g.index] = Some(g.colour);
    }
    let mut runs: Vec<ColourRun> = Vec::new();
    for (i, &b) in stripped.iter().enumerate() {
        match (colours[i], runs.last_mut()) {
            (None, Some(last)) => last.text.push(b),
            (Some(c), Some(last)) if last.colour == c => last.text.push(b),
            (c, _) => runs.push(ColourRun { text: vec![b], colour: c.unwrap_or(base) }),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uca_conventions_rgba_to_argb_forces_alpha() {
        assert_eq!(rgba_to_argb(0x11223300), 0xFF112233);
        assert_eq!(rgba_to_argb(0xAABBCCFF), 0xFFAABBCC);
    }

    #[test]
    fn ur_3_3_code_is_brace_six_hex_brace() {
        assert_eq!(parse_code(b"{FF00aa}x"), Some(0xFF00AA));
        assert_eq!(parse_code(b"{FF00A}"), None);
        assert_eq!(parse_code(b"{FF00AG}"), None);
        assert_eq!(parse_code(b"{FF00AA0}"), None);
        assert_eq!(parse_code(b"FF00AA}"), None);
    }

    #[test]
    fn ur_3_3_strip_removes_every_valid_code_only() {
        assert_eq!(strip_codes(b"{FF0000}red {00ff00}green {xyz} {12345}"), b"red green {xyz} {12345}");
        assert_eq!(strip_codes(b"{FF0000}{00FF00}"), b"");
        // In-place semantics: the scan continues at the same index after a removal.
        assert_eq!(strip_codes(b"{{FF0000}FF0000}"), b"{FF0000}");
    }

    #[test]
    fn ur_3_3_alpha_applies_until_first_code_then_forced_ff() {
        let g = glyph_colours(b"ab{112233}cd", 0x80FFFFFF);
        let c: Vec<u32> = g.iter().map(|g| g.colour).collect();
        assert_eq!(c, vec![0x80FFFFFF, 0x80FFFFFF, 0xFF112233, 0xFF112233]);
    }

    #[test]
    fn ur_3_3_blanks_are_skipped_by_the_cursor() {
        let g = glyph_colours(b"a {FF0000}b", 0xFFFFFFFF);
        assert_eq!(g.len(), 2);
        assert_eq!((g[1].byte, g[1].colour), (b'b', 0xFFFF0000));
        assert_eq!(g[1].index, 2);
    }

    #[test]
    fn ur_3_3_code_followed_by_blank_shifts_like_the_original() {
        // The cursor steps over the blank as if it were the glyph; the second code is
        // only consumed for the glyph after 'x'.
        let g = glyph_colours(b"{FF0000} {00FF00}xy", 0xFFFFFFFF);
        let c: Vec<(u8, u32)> = g.iter().map(|g| (g.byte, g.colour)).collect();
        assert_eq!(c, vec![(b'x', 0xFFFF0000), (b'y', 0xFF00FF00)]);
    }

    #[test]
    fn colour_runs_group_glyphs() {
        let r = colour_runs(b"hi {FF0000}there you", 0xFFFFFFFF);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], ColourRun { text: b"hi ".to_vec(), colour: 0xFFFFFFFF });
        assert_eq!(r[1], ColourRun { text: b"there you".to_vec(), colour: 0xFFFF0000 });
    }
}
