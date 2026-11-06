//! Text helpers shared by chat, bubbles and dialogs: C-string length, control
//! character replacement, character-count line splitting and the `~k~` hook.

/// `strlen`: bytes up to the first NUL (wire strings are not NUL-terminated, but
/// the client copies them into C strings before measuring).
pub fn c_strlen(s: &[u8]) -> usize {
    s.iter().position(|&b| b == 0).unwrap_or(s.len())
}

/// The bytes up to the first NUL.
pub fn c_str(s: &[u8]) -> &[u8] {
    &s[..c_strlen(s)]
}

/// Replaces control characters 1..=31 with spaces (UCA "chat window model",
/// A 0x10067420).
pub fn replace_control_chars(s: &mut [u8]) {
    for b in s.iter_mut() {
        if (1..=31).contains(b) {
            *b = b' ';
        }
    }
}

/// `strncpy`-style truncation to at most `max` bytes.
pub fn truncate(s: &[u8], max: usize) -> &[u8] {
    &s[..s.len().min(max)]
}

/// Splits `text` once at `width` characters, preferring a space (UCA "Type-2 wrapping").
///
/// If `text` is longer than `width`, look backwards from index `width` for a space.
/// A space at most `backtrack` characters before `width` splits there (the space is
/// dropped); otherwise the text is hard-split at `width`.
pub fn split_line(text: &[u8], width: usize, backtrack: usize) -> (&[u8], Option<&[u8]>) {
    if text.len() <= width {
        return (text, None);
    }
    let low = width.saturating_sub(backtrack);
    match (low..=width).rev().find(|&i| text[i] == b' ') {
        Some(i) => (&text[..i], Some(&text[i + 1..])),
        None => (&text[..width], Some(&text[width..])),
    }
}

/// Applies [`split_line`] repeatedly until the rest fits.
pub fn wrap_lines(mut text: &[u8], width: usize, backtrack: usize) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    loop {
        let (first, rest) = split_line(text, width, backtrack);
        lines.push(first);
        match rest {
            Some(r) => text = r,
            None => return lines,
        }
    }
}

/// GTA's key-name substitution (`~k~~ACTION~` → key name), supplied by the game layer.
/// The DLL build calls GTA 0x69E160; a headless build may return the text unchanged.
pub trait KeyNameExpander {
    fn expand(&self, text: &[u8]) -> Vec<u8>;
}

/// An expander that leaves text unchanged.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoExpansion;

impl KeyNameExpander for NoExpansion {
    fn expand(&self, text: &[u8]) -> Vec<u8> {
        text.to_vec()
    }
}

/// Texts this long or longer are never expanded (UCA "Key-name expansion").
pub const KEY_EXPANSION_MAX_INPUT: usize = 400;
/// The expanded text is kept only if it fits in this many characters.
/// Kept when its length is `<= 255`: A 0x100b6140 rejects with a signed `>` against the
/// caller's limit (the chat caller A 0x10068190 passes 255).
pub const KEY_EXPANSION_MAX_OUTPUT: usize = 255;
/// 3D text labels (RPC 36 store, A 0x100011d0) pass 399 as the limit.
pub const LABEL_KEY_EXPANSION_MAX_OUTPUT: usize = 399;

/// Runs `~k~` expansion like A 0x100b6140: only if the text contains `~k~` and is
/// shorter than 400 chars; the result is kept only if it fits in 255 chars.
pub fn expand_key_names(text: &[u8], expander: &dyn KeyNameExpander) -> Vec<u8> {
    expand_key_names_limit(text, expander, KEY_EXPANSION_MAX_OUTPUT)
}

/// [`expand_key_names`] with the caller's output limit (`max_out`: chat 255, labels 399).
pub fn expand_key_names_limit(text: &[u8], expander: &dyn KeyNameExpander, max_out: usize) -> Vec<u8> {
    let has_k = text.windows(3).any(|w| w == b"~k~");
    if has_k && text.len() < KEY_EXPANSION_MAX_INPUT {
        let r = expander.expand(text);
        if r.len() <= max_out {
            return r;
        }
    }
    text.to_vec()
}

/// C `atoi`: optional leading blanks, optional sign, then digits; 0 if none.
pub fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = (v * 10 + i64::from(s[i] - b'0')).min(i64::from(u32::MAX) + 1);
        i += 1;
    }
    let v = if neg { -v } else { v };
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// `sscanf("%d")`-style reading of consecutive integers: returns the values read
/// before the first failure.
pub fn scan_ints(s: &[u8], max: usize) -> Vec<i32> {
    let mut out = Vec::new();
    let mut i = 0;
    while out.len() < max {
        while i < s.len() && s[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
            i += 1;
        }
        let digits = i;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        if i == digits {
            break;
        }
        out.push(atoi(&s[start..i]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uca_chat_model_control_chars_become_spaces() {
        let mut s = *b"a\x01b\tc\nd\x1f\x20e\x7f";
        replace_control_chars(&mut s);
        assert_eq!(&s, b"a b c d  e\x7f");
    }

    #[test]
    fn uca_type2_wrap_short_text_not_split() {
        let t = [b'x'; 72];
        assert_eq!(split_line(&t, 72, 12), (&t[..], None));
    }

    #[test]
    fn uca_type2_wrap_space_within_12_before_72() {
        let mut t = vec![b'a'; 100];
        t[61] = b' ';
        let (a, b) = split_line(&t, 72, 12);
        assert_eq!(a.len(), 61);
        assert_eq!(b.unwrap().len(), 100 - 62);
        // Space exactly at index 72 counts too.
        let mut t = vec![b'a'; 80];
        t[72] = b' ';
        let (a, b) = split_line(&t, 72, 12);
        assert_eq!((a.len(), b.unwrap().len()), (72, 7));
    }

    #[test]
    fn uca_type2_wrap_space_too_far_back_hard_split() {
        let mut t = vec![b'a'; 100];
        t[59] = b' ';
        let (a, b) = split_line(&t, 72, 12);
        assert_eq!(a.len(), 72);
        assert_eq!(b.unwrap().len(), 28);
        // Index 60 is exactly 12 before 72: still a word split.
        t[60] = b' ';
        assert_eq!(split_line(&t, 72, 12).0.len(), 60);
    }

    #[test]
    fn wrap_lines_repeats() {
        let t = b"aaaa bbbb cccc";
        assert_eq!(wrap_lines(t, 6, 3), vec![&b"aaaa"[..], b"bbbb", b"cccc"]);
    }

    struct Upper;
    impl KeyNameExpander for Upper {
        fn expand(&self, t: &[u8]) -> Vec<u8> {
            let mut v = t.to_ascii_uppercase();
            v.extend_from_slice(b"!!");
            v
        }
    }

    #[test]
    fn uca_key_expansion_only_with_k_code_and_limits() {
        assert_eq!(expand_key_names(b"press ~k~~VEHICLE_HORN~", &Upper), b"PRESS ~K~~VEHICLE_HORN~!!");
        assert_eq!(expand_key_names(b"no code", &Upper), b"no code");
        // Result longer than 255 is discarded.
        let mut t = vec![b'a'; 251];
        t.extend_from_slice(b"~k~");
        assert_eq!(expand_key_names(&t, &Upper), t);
        // Input of 400+ chars is not expanded.
        let mut t = vec![b'a'; 397];
        t.extend_from_slice(b"~k~");
        assert_eq!(expand_key_names(&t, &NoExpansion), t);
    }

    #[test]
    fn label_key_expansion_limit_399() {
        // 397 + "!!" = 399 fits the label limit, 398 + "!!" does not.
        let mut t = vec![b'a'; 394];
        t.extend_from_slice(b"~k~");
        let r = expand_key_names_limit(&t, &Upper, LABEL_KEY_EXPANSION_MAX_OUTPUT);
        assert_eq!(r.len(), 399);
        let mut t = vec![b'a'; 395];
        t.extend_from_slice(b"~k~");
        assert_eq!(expand_key_names_limit(&t, &Upper, LABEL_KEY_EXPANSION_MAX_OUTPUT), t);
        // Colour codes pass through the filter untouched.
        assert_eq!(expand_key_names_limit(b"{FF0000}red", &Upper, 399), b"{FF0000}red");
    }

    #[test]
    fn atoi_and_scan() {
        assert_eq!(atoi(b" 15abc"), 15);
        assert_eq!(atoi(b"-3"), -3);
        assert_eq!(atoi(b"abc"), 0);
        assert_eq!(scan_ints(b"12 30", 2), vec![12, 30]);
        assert_eq!(scan_ints(b"12 x", 2), vec![12]);
        assert_eq!(c_str(b"ab\0cd"), b"ab");
    }
}
