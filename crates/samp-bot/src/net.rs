//! UDP transport: the per-port cipher and the unconnected cookie handshake
//! (docs/re/handshake.md §1–§2).

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use samp_proto::cipher::{self, CipherKey};
use samp_proto::unconnected::{ServerReply, open_connection_request, parse_reply};

use crate::Result;

/// A connected UDP socket that encrypts everything it sends.
pub struct Net {
    sock: UdpSocket,
    pub server: SocketAddr,
    key: CipherKey,
    wire: Vec<u8>,
}

impl Net {
    pub fn connect(server: SocketAddr) -> Result<Self> {
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.connect(server)?;
        Ok(Self { sock, server, key: CipherKey::from_port(server.port()), wire: Vec::new() })
    }

    pub fn send_plain(&mut self, plain: &[u8]) -> Result<()> {
        cipher::encrypt(plain, self.key, &mut self.wire);
        self.sock.send(&self.wire)?;
        Ok(())
    }

    /// Receive one datagram, waiting at most `timeout` (`None` on timeout).
    pub fn recv(&self, buf: &mut [u8], timeout: Duration) -> Result<Option<usize>> {
        self.sock.set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
        match self.sock.recv(buf) {
            Ok(n) => Ok(Some(n)),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Unconnected handshake: open-connection requests with the cookie the
    /// server hands out, until it accepts (6 attempts, 1 s apart).
    pub fn open_connection(&mut self, buf: &mut [u8]) -> Result<()> {
        let mut cookie = 0u16;
        for attempt in 1..=6 {
            self.send_plain(&open_connection_request(cookie))?;
            let deadline = Instant::now() + Duration::from_millis(1000);
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                let Some(n) = self.recv(buf, left)? else { break };
                match parse_reply(&buf[..n]) {
                    Some(ServerReply::Cookie(c)) => cookie = c,
                    Some(ServerReply::Accepted) => {
                        println!("[open] accepted after {attempt} request(s)");
                        return Ok(());
                    }
                    Some(other) => return Err(format!("server refused: {other:?}").into()),
                    None => {}
                }
            }
        }
        Err("server did not respond".into())
    }
}
