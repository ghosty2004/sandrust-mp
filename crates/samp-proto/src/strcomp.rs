//! RakNet-style Huffman string compression ("compressed string", cstr).
//!
//! Spec: docs/re/subsystems/ui-chat-audio.md "Shared: compressed string" and
//! docs/re/subsystems/world-entities.md ("cstr"). Used by ShowDialog (61, body,
//! client buffer 4096), PlayerShowTextLabel (36, text, 2048) and object
//! material text (44/84, 2048).
//!
//! Wire format: a compressed u16 bit count, then that many Huffman-coded bits
//! (MSB-first), one code per output byte. The encoder pads its output to a
//! whole number of bytes (counted from the start of the coded bits, not from
//! the start of the enclosing stream) with a *prefix* of a longer code, so the
//! padding never decodes to a byte; the bit count includes that padding.
//!
//! Tree: built from a fixed 256-entry frequency table (A 0x10101de8; entries
//! 128..255 are zero), zero weights replaced by 1. Construction keeps a list
//! of nodes sorted by ascending weight. A node is inserted *before* the first
//! node whose weight is greater than or equal to its own (i.e. after all
//! strictly lighter nodes), so among equal weights the most recently inserted
//! node comes first. Leaves are inserted in byte order 0..=255. Then the two
//! front nodes are repeatedly removed: the first becomes the 0-branch, the
//! second the 1-branch, and their parent (sum of weights) is inserted back by
//! the same rule. The tie-break was confirmed against the client's insertion
//! routine (A 0x10026ea0: advances while the listed weight is strictly less)
//! and by decoding live open.mp traffic (see the golden tests below).

use std::sync::OnceLock;

use crate::bitstream::{BitReader, BitWriter};

/// Byte frequencies for values 0..=127 (spec table; 128..=255 are all 0).
const FREQUENCIES_LOW: [u32; 128] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 722, 0, 0, 2, 0, 0, //
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
    11084, 58, 63, 1, 0, 31, 0, 317, 64, 64, 44, 0, 695, 62, 980, 266, //
    69, 67, 56, 7, 73, 3, 14, 2, 69, 1, 167, 9, 1, 2, 25, 94, //
    0, 195, 139, 34, 96, 48, 103, 56, 125, 653, 21, 5, 23, 64, 85, 44, //
    34, 7, 92, 76, 147, 12, 14, 57, 15, 39, 15, 1, 1, 1, 2, 3, //
    0, 3611, 845, 1077, 1884, 5870, 841, 1057, 2501, 3212, 164, 531, 2019, 1330, 3056, 4037, //
    848, 47, 2586, 2919, 4771, 1707, 535, 1106, 152, 1243, 100, 0, 2, 0, 10, 0, //
];

/// Largest bit count the u16 length field can carry, rounded down to whole
/// bytes (the encoder always emits a byte-aligned bit count).
const MAX_ENCODED_BITS: usize = (u16::MAX as usize) & !7;

/// One Huffman code: `len` bits, stored right-aligned in `bits` (first bit is
/// the most significant of the `len`).
#[derive(Clone, Copy, Debug, Default)]
struct Code {
    bits: u64,
    len: u8,
}

struct Tree {
    /// Children of each node, `[0-branch, 1-branch]`. Indices 0..=255 are the
    /// leaves for the byte of the same value (no children); internal nodes
    /// follow; the root is the last node.
    children: Vec<Option<[u16; 2]>>,
    codes: [Code; 256],
}

impl Tree {
    fn root(&self) -> usize {
        self.children.len() - 1
    }

    fn build() -> Self {
        let mut weights: Vec<u32> = (0..256).map(|b| FREQUENCIES_LOW.get(b).copied().unwrap_or(0).max(1)).collect();
        let mut children: Vec<Option<[u16; 2]>> = vec![None; 256];

        // Sorted (ascending weight) list of node indices; see module docs for
        // the insertion rule.
        let mut list: Vec<usize> = Vec::with_capacity(256);
        let insert = |list: &mut Vec<usize>, weights: &[u32], node: usize| {
            let at = list.partition_point(|&n| weights[n] < weights[node]);
            list.insert(at, node);
        };
        for leaf in 0..256 {
            insert(&mut list, &weights, leaf);
        }
        while list.len() > 1 {
            let zero = list.remove(0);
            let one = list.remove(0);
            let parent = weights.len();
            weights.push(weights[zero] + weights[one]);
            children.push(Some([zero as u16, one as u16]));
            insert(&mut list, &weights, parent);
        }

        let mut tree = Tree { children, codes: [Code::default(); 256] };
        // Depth-first walk assigning root-to-leaf paths.
        let mut stack = vec![(tree.root(), Code::default())];
        while let Some((node, code)) = stack.pop() {
            match tree.children[node] {
                None => tree.codes[node] = code,
                Some(kids) => {
                    assert!(code.len < 64, "Huffman code longer than 64 bits");
                    for (bit, &kid) in kids.iter().enumerate() {
                        let next = Code { bits: (code.bits << 1) | bit as u64, len: code.len + 1 };
                        stack.push((kid.into(), next));
                    }
                }
            }
        }
        tree
    }
}

fn tree() -> &'static Tree {
    static TREE: OnceLock<Tree> = OnceLock::new();
    TREE.get_or_init(Tree::build)
}

/// Decode a compressed string; `max_len` is the receiver's buffer limit in bytes.
///
/// Mirrors the client decoder (A 0x100542d0): reads the compressed u16 bit
/// count; if fewer bits remain the decode fails (`None`). Otherwise *all*
/// `count` bits are consumed, and the decoded bytes are truncated to
/// `max_len - 1` (the client NUL-terminates inside a `max_len` buffer; empty
/// for `max_len == 0`). Trailing bits that do not complete a code (the
/// encoder's padding) produce nothing. Bytes are returned verbatim, including
/// any NUL; the client treats the buffer as a C string, so it ends at the
/// first NUL.
pub fn read_compressed_string(r: &mut BitReader, max_len: usize) -> Option<Vec<u8>> {
    let bit_count = usize::from(r.read_compressed_u16()?);
    if r.remaining() < bit_count {
        return None;
    }
    let t = tree();
    let limit = max_len.saturating_sub(1);
    let mut out = Vec::new();
    let mut node = t.root();
    for _ in 0..bit_count {
        let bit = r.read_bit()?;
        let kids = t.children[node].expect("walk always sits on an internal node");
        node = kids[usize::from(bit)].into();
        if t.children[node].is_none() {
            if out.len() < limit {
                out.push(node as u8);
            }
            node = t.root();
        }
    }
    Some(out)
}

/// Encode `s` as a compressed string.
///
/// Every byte is encoded verbatim (NUL included; the reference encoders stop
/// at the first NUL because they take C strings). The coded bits are padded
/// to a multiple of 8 with the leading bits of the code of the lowest byte
/// value whose code is longer than the padding, as the reference encoder
/// does, so output is bit-identical to what servers send. If the coded string
/// would not fit the u16 bit count, trailing bytes are dropped until it does.
pub fn write_compressed_string(w: &mut BitWriter, s: &[u8]) {
    let t = tree();
    let mut bits = 0usize;
    let mut take = 0usize;
    for &b in s {
        let next = bits + usize::from(t.codes[usize::from(b)].len);
        if next.div_ceil(8) * 8 > MAX_ENCODED_BITS {
            break;
        }
        bits = next;
        take += 1;
    }
    let total = bits.div_ceil(8) * 8;
    w.write_compressed_u16(total as u16);
    for &b in &s[..take] {
        write_code(w, t.codes[usize::from(b)]);
    }
    let pad = total - bits;
    if pad > 0 {
        let filler = t.codes.iter().find(|c| usize::from(c.len) > pad).expect("some code is longer than 7 bits");
        write_code(w, Code { bits: filler.bits >> (filler.len as usize - pad), len: pad as u8 });
    }
}

fn write_code(w: &mut BitWriter, code: Code) {
    for i in (0..code.len).rev() {
        w.write_bit((code.bits >> i) & 1 == 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(s: &[u8]) -> BitWriter {
        let mut w = BitWriter::new();
        write_compressed_string(&mut w, s);
        w
    }

    fn roundtrip(s: &[u8]) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write_bit(true); // misalign on purpose
        write_compressed_string(&mut w, s);
        w.write_bits(0b101, 3); // trailing data must stay readable
        let mut r = BitReader::with_bit_len(w.as_bytes(), w.bit_len());
        assert_eq!(r.read_bit(), Some(true));
        let out = read_compressed_string(&mut r, usize::MAX).expect("decode");
        assert_eq!(r.read_bits(3), Some(0b101), "bit count must cover exactly the coded bits");
        out
    }

    #[test]
    fn tree_is_complete_prefix_code() {
        let t = tree();
        assert_eq!(t.children.len(), 511);
        // Kraft equality: a full binary tree.
        let kraft: f64 = t.codes.iter().map(|c| 0.5f64.powi(c.len.into())).sum();
        assert!((kraft - 1.0).abs() < 1e-12);
        // Space (most frequent) is short, high bytes (weight 1) are long.
        assert!(t.codes[b' ' as usize].len <= 3);
        assert!(t.codes[0xFF].len > 8);
    }

    #[test]
    fn roundtrip_all_byte_values() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(roundtrip(&all), all);
        for b in 0..=255u8 {
            assert_eq!(roundtrip(&[b]), [b]);
        }
    }

    #[test]
    fn roundtrip_empty() {
        let w = encode(b"");
        // compressed u16 0: 1 (hi byte zero) 1 (low nibble form) 0000
        assert_eq!((w.bit_len(), w.as_bytes()), (6, &[0b1100_0000][..]));
        assert_eq!(roundtrip(b""), b"");
    }

    #[test]
    fn roundtrip_long_and_aligned() {
        let text: Vec<u8> =
            b"The quick brown fox jumps over the lazy dog. 0123456789\n\t{FF0000}~k~~VEHICLE_ENTER_EXIT~ "
                .iter()
                .copied()
                .cycle()
                .take(4000)
                .collect();
        assert_eq!(roundtrip(&text), text);
        let mut w = BitWriter::new();
        write_compressed_string(&mut w, &text);
        let mut r = BitReader::new(w.as_bytes());
        let n = r.read_compressed_u16().unwrap();
        assert_eq!(n % 8, 0, "coded bits are padded to whole bytes");
    }

    #[test]
    fn writer_truncates_to_u16_bit_count() {
        // 0xFF has a long code; 20000 of them cannot fit in 65535 bits.
        let s = vec![0xFFu8; 20000];
        let out = roundtrip(&s);
        assert!(!out.is_empty() && out.len() < s.len());
        assert!(out.iter().all(|&b| b == 0xFF));
        let enc = encode(&s);
        let mut r = BitReader::new(enc.as_bytes());
        assert!(usize::from(r.read_compressed_u16().unwrap()) <= MAX_ENCODED_BITS);
    }

    #[test]
    fn max_len_truncates_but_consumes_all_bits() {
        let mut w = encode(b"hello world");
        w.write_u8(0xAB);
        let mut r = BitReader::new(w.as_bytes());
        assert_eq!(read_compressed_string(&mut r, 6).as_deref(), Some(&b"hello"[..]));
        assert_eq!(r.read_u8(), Some(0xAB));
        let mut r = BitReader::new(w.as_bytes());
        assert_eq!(read_compressed_string(&mut r, 12).as_deref(), Some(&b"hello world"[..]));
        let mut r = BitReader::new(w.as_bytes());
        assert_eq!(read_compressed_string(&mut r, 0).as_deref(), Some(&b""[..]));
        assert_eq!(r.read_u8(), Some(0xAB));
    }

    #[test]
    fn short_data_fails() {
        let w = encode(b"hello world");
        let bytes = w.as_bytes();
        let mut r = BitReader::with_bit_len(bytes, w.bit_len() - 1);
        assert_eq!(read_compressed_string(&mut r, 4096), None);
    }

    #[test]
    fn padding_never_decodes() {
        // "e" alone is not byte-aligned; the decoded output must be just "e".
        let w = encode(b"e");
        let mut r = BitReader::new(w.as_bytes());
        let n = r.read_compressed_u16().unwrap();
        assert_eq!(n, 8);
        let mut r = BitReader::new(w.as_bytes());
        assert_eq!(read_compressed_string(&mut r, 16).as_deref(), Some(&b"e"[..]));
    }
    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Check a captured cstr: it decodes to `text`, consumes exactly `bits`,
    /// and our encoder reproduces it bit for bit.
    fn check_golden(wire_hex: &str, bits: usize, text: &[u8]) {
        let wire = unhex(wire_hex);
        let mut r = BitReader::with_bit_len(&wire, bits);
        assert_eq!(read_compressed_string(&mut r, 4096).as_deref(), Some(text));
        assert_eq!(r.remaining(), 0);
        let w = encode(text);
        assert_eq!(w.bit_len(), bits);
        assert_eq!(w.as_bytes(), wire.as_slice());
    }

    // Captured from open.mp 1.6.7 (local test server) with
    // crates/samp-bot/examples/cstr_probe.rs: the cstr part of RPC 61
    // (ShowDialog body) and RPC 36 (PlayerShowTextLabel text) sent by a
    // filterscript, starting at the compressed u16 bit count.
    #[test]
    fn golden_open_mp_dialog_body() {
        let text: &[u8] = b"Hello, World! 0123456789\nLine 2:\tTabbed {FF00AA}colour ~r~ (x) [y] <z> 50% #$&*+-=/?;'`|\\ quote\" end\n\x80uro caf\xE9 na\xEFve \xA9 \xDF \xFF.";
        let wire = "7c01be04a51afab9199395df514eaadabc2a355d157d575ceef97e699a421d5b309d24a931e78777ca4562b2a2a895151f29e88942f3eadb9ab6f7d6a6fa77cb8adf2ebbe5f2a8cdf55d2a27c9df2cdf2c9f2f4c77cbc57d5754e19674f54f94b3a61f2ef998b8b0f9f04e35f285798f431d3a11c995eda78efb11e741bce9b3ca80";
        // Header by hand: bit 0 (high byte non-zero), then u16 LE 1016 = f8 03:
        // 0111_1100 0000_0001 1... = 7c 01 + top bit of the next byte.
        let raw = unhex(wire);
        assert_eq!((raw[0], raw[1], raw[2] & 0x80), (0x7c, 0x01, 0x80));
        check_golden(wire, 1033, text);
    }

    #[test]
    fn golden_open_mp_text_label() {
        let text: &[u8] = b"Label: \xC4rger 3D-text #7 (ok)!\nsecond line";
        let wire = "040099a63c48c3bed6e6619eaf033abf62a77df2cd575f7d61fbe8ae9bb14413bc908280";
        check_golden(wire, 281, text);
    }
}
