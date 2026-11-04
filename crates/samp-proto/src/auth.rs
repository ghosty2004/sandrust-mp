//! Connection auth key (`0x0c`) exchange.
//!
//! Spec: docs/re/handshake.md §4. The server sends `0x0c, u8 len, len bytes`
//! (open.mp includes the trailing NUL in `len`); the client answers
//! `0x0c, u8 len, len bytes` with a 40-char uppercase hex string.
//!
//! Servers validate the answer against a fixed table of 256 challenge →
//! response pairs. That table is NOT part of this repository (its source is
//! under a third-party license); it is loaded at runtime from a text file with
//! one `CHALLENGE RESPONSE` pair per line (see tools/extract_auth_table.py).

use std::collections::HashMap;

use crate::raknet::id;

#[derive(Debug, Default)]
pub struct AuthTable {
    pairs: HashMap<String, String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
}

impl AuthTable {
    /// Parse `CHALLENGE RESPONSE` lines; blank lines and `#` comments are ignored.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut pairs = HashMap::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split_whitespace();
            match (it.next(), it.next(), it.next()) {
                (Some(c), Some(r), None) => {
                    pairs.insert(c.to_owned(), r.to_owned());
                }
                _ => return Err(ParseError { line: i + 1 }),
            }
        }
        Ok(Self { pairs })
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    pub fn response(&self, challenge: &str) -> Option<&str> {
        self.pairs.get(challenge).map(String::as_str)
    }
}

/// Extract the challenge string from a server `0x0c` message.
pub fn parse_challenge(msg: &[u8]) -> Option<String> {
    let [id::AUTH_KEY, len, rest @ ..] = msg else { return None };
    let raw = rest.get(..usize::from(*len))?;
    let raw = raw.split(|&b| b == 0).next().unwrap_or(raw); // drop trailing NUL if present
    String::from_utf8(raw.to_vec()).ok()
}

/// Build the client `0x0c` reply carrying `response`.
pub fn build_response(response: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 + response.len());
    v.push(id::AUTH_KEY);
    v.push(response.len() as u8);
    v.extend_from_slice(response.as_bytes());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_parse_and_lookup() {
        let t = AuthTable::parse("# c r\nABC 0123\n\nDEF 4567\n").unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t.response("DEF"), Some("4567"));
        assert!(AuthTable::parse("only-one-field").is_err());
    }

    #[test]
    fn challenge_with_and_without_nul() {
        assert_eq!(parse_challenge(b"\x0c\x04ABC\0").as_deref(), Some("ABC"));
        assert_eq!(parse_challenge(b"\x0c\x03ABC").as_deref(), Some("ABC"));
        assert_eq!(parse_challenge(b"\x0c\x09ABC"), None);
    }

    #[test]
    fn response_layout() {
        assert_eq!(build_response("AB"), b"\x0c\x02AB");
    }
}
