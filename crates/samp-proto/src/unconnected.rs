//! Messages exchanged before a RakNet connection exists (single plaintext
//! message per datagram, no reliability header).
//!
//! Spec: docs/re/handshake.md §1.

/// Message IDs as used by 0.3.7 (SA:MP shifted RakNet's numbering).
pub mod id {
    pub const OPEN_CONNECTION_REQUEST: u8 = 0x18;
    pub const OPEN_CONNECTION_REPLY: u8 = 0x19;
    pub const OPEN_CONNECTION_COOKIE: u8 = 0x1a;
    pub const CONNECTION_ATTEMPT_FAILED: u8 = 0x1d;
    pub const NO_FREE_INCOMING_CONNECTIONS: u8 = 0x1f;
    pub const CONNECTION_BANNED: u8 = 0x24;
}

/// XOR mask applied to the cookie in the open-connection request.
pub const COOKIE_MASK: u16 = 0x6969;

/// Build an open-connection request carrying `cookie` (0 before the server issued one).
pub fn open_connection_request(cookie: u16) -> [u8; 3] {
    let v = (COOKIE_MASK ^ cookie).to_le_bytes();
    [id::OPEN_CONNECTION_REQUEST, v[0], v[1]]
}

/// A server reply to the open-connection request, parsed with the same
/// length rules the 0.3.7 client applies.
#[derive(Debug, PartialEq, Eq)]
pub enum ServerReply {
    /// Server issued a cookie; resend the request with it.
    Cookie(u16),
    /// Server accepted; next step is the reliable connection request.
    Accepted,
    Failed,
    ServerFull,
    Banned,
}

pub fn parse_reply(data: &[u8]) -> Option<ServerReply> {
    match *data {
        [id::OPEN_CONNECTION_COOKIE, lo, hi] => Some(ServerReply::Cookie(u16::from_le_bytes([lo, hi]))),
        [first, ..] if data.len() <= 2 => match first {
            id::OPEN_CONNECTION_REPLY => Some(ServerReply::Accepted),
            id::CONNECTION_ATTEMPT_FAILED => Some(ServerReply::Failed),
            id::NO_FREE_INCOMING_CONNECTIONS => Some(ServerReply::ServerFull),
            id::CONNECTION_BANNED => Some(ServerReply::Banned),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_layout() {
        assert_eq!(open_connection_request(0), [0x18, 0x69, 0x69]);
        assert_eq!(open_connection_request(0x1234), [0x18, 0x5d, 0x7b]);
    }

    #[test]
    fn reply_parsing() {
        assert_eq!(parse_reply(&[0x1a, 0x34, 0x12]), Some(ServerReply::Cookie(0x1234)));
        assert_eq!(parse_reply(&[0x19, 0x00]), Some(ServerReply::Accepted));
        assert_eq!(parse_reply(&[0x1f]), Some(ServerReply::ServerFull));
        assert_eq!(parse_reply(&[0x1a, 0x00]), None); // cookie must be exactly 3 bytes
        assert_eq!(parse_reply(&[0x19, 0, 0]), None); // reply must be <= 2 bytes
        assert_eq!(parse_reply(&[]), None);
    }
}
