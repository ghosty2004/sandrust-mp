//! Connection-level session: RakNet connection over [`Net`], the auth key
//! exchange and the join (RPC 25). Everything above that is handed to the
//! caller as [`Event`]s; outgoing game messages go through [`Outbox`].

use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};

use samp_proto::auth::{self, AuthTable};
use samp_proto::bitstream::BitWriter;
use samp_proto::connection::{Connection, Delivered};
use samp_proto::raknet::{Reliability, id, strip_timestamp};
use samp_proto::rpc;
use samp_proto::rpcs::{self, Incoming, SendParams};
use samp_proto::session;
use samp_proto::sync::{self, IncomingSync};

use crate::{Outbox, Result};
use crate::net::Net;

/// Something the server sent that the game layer cares about.
#[derive(Debug)]
pub enum Event {
    /// PlayerConnect (RPC 25) was sent; the server will answer with InitGame.
    Joining,
    Rpc(Incoming),
    /// RPC the codec does not decode (id, payload bits).
    UndecodedRpc(u8, u32),
    Sync(IncomingSync),
    /// The server closed the connection.
    Closed,
}

/// A connected session.
pub struct Client {
    net: Net,
    conn: Connection,
    table: AuthTable,
    name: String,
    buf: [u8; 1500],
}

impl Client {
    /// Resolve `target`, run the cookie handshake and send the RakNet
    /// connection request. The join itself happens in [`Client::pump`].
    pub fn connect(target: &str, name: &str, table: AuthTable) -> Result<Self> {
        let server = target.to_socket_addrs()?.next().ok_or("could not resolve address")?;
        let mut net = Net::connect(server)?;
        let mut buf = [0u8; 1500];
        net.open_connection(&mut buf)?;
        let mut conn = Connection::new(Instant::now());
        conn.send_bytes(&[id::CONNECTION_REQUEST], Reliability::Reliable, 0);
        Ok(Self { net, conn, table, name: name.into(), buf })
    }

    /// Flush queued datagrams, wait up to `wait` for one datagram and turn it
    /// into events.
    pub fn pump(&mut self, wait: Duration) -> Result<Vec<Event>> {
        self.flush()?;
        let mut events = Vec::new();
        let Some(n) = self.net.recv(&mut self.buf, wait)? else { return Ok(events) };
        let Ok(delivered) = self.conn.handle_datagram(&self.buf[..n], Instant::now()) else { return Ok(events) };
        for msg in delivered {
            self.handle_message(&msg, &mut events)?;
        }
        Ok(events)
    }

    /// Send everything the connection has queued.
    pub fn flush(&mut self) -> Result<()> {
        for dg in self.conn.poll(Instant::now()) {
            self.net.send_plain(&dg)?;
        }
        Ok(())
    }

    /// Notify the server and flush.
    pub fn disconnect(&mut self) -> Result<()> {
        self.conn.send_bytes(&[id::DISCONNECTION_NOTIFICATION], Reliability::ReliableOrdered, 0);
        self.flush()
    }

    fn handle_message(&mut self, msg: &Delivered, events: &mut Vec<Event>) -> Result<()> {
        let (_, body) = strip_timestamp(&msg.data);
        let Some(&first) = body.first() else { return Ok(()) };
        match first {
            id::AUTH_KEY => {
                let challenge = auth::parse_challenge(body).ok_or("malformed auth challenge")?;
                let response =
                    self.table.response(&challenge).ok_or_else(|| format!("challenge {challenge} not in table"))?;
                println!("[auth] challenge {challenge} -> answered");
                self.conn.send_bytes(&auth::build_response(response), Reliability::Reliable, 0);
            }
            id::CONNECTION_REQUEST_ACCEPTED => {
                let acc = session::parse_accepted(body).ok_or("malformed connection-accepted")?;
                println!("[conn] accepted: player id {}, challenge {:#010x}", acc.player_id, acc.challenge);
                let mut w = BitWriter::new();
                w.write_u8(id::NEW_INCOMING_CONNECTION);
                match self.net.server.ip() {
                    std::net::IpAddr::V4(ip) => w.write_bytes(&ip.octets()),
                    std::net::IpAddr::V6(_) => return Err("IPv6 servers are not supported".into()),
                }
                w.write_u16(self.net.server.port());
                self.conn.send_bits(&w, Reliability::Reliable, 0);
                // Same order as the original client (docs/re/handshake.md §1,
                // differential capture): ping, empty static data, keep-alive.
                self.conn.ping(Instant::now());
                self.conn.send_bytes(&[id::RECEIVED_STATIC_DATA], Reliability::Reliable, 0);
                self.conn.send_bytes(&[id::KEEP_ALIVE], Reliability::Reliable, 0);

                let serial = session::make_serial(&pseudo_random_hex40());
                let connect = rpcs::session::PlayerConnect::new(&session::R4, &self.name, acc.challenge, &serial);
                crate::send_rpc(self, &connect);
                println!("[join] sent PlayerConnect as {:?}", self.name);
                events.push(Event::Joining);
            }
            id::RPC => {
                let Some(rpc) = rpc::decode(&msg.data, msg.bit_len) else { return Ok(()) };
                match rpcs::decode(rpc.id, &mut rpc.reader()) {
                    Some(incoming) => events.push(Event::Rpc(incoming)),
                    None => events.push(Event::UndecodedRpc(rpc.id, rpc.bit_len)),
                }
            }
            id::DISCONNECTION_NOTIFICATION => events.push(Event::Closed),
            id::CONNECTION_BANNED => return Err("banned from this server".into()),
            id::INVALID_PASSWORD => return Err("wrong server password".into()),
            _ => {
                if let Some(s) = sync::decode(&msg.data, msg.bit_len) {
                    events.push(Event::Sync(s));
                }
            }
        }
        Ok(())
    }
}

impl Outbox for Client {
    fn send(&mut self, bits: &BitWriter, params: SendParams) {
        self.conn.send_bits(bits, params.reliability, params.channel);
    }
}

/// 40 hex digits of non-cryptographic randomness for the client serial.
fn pseudo_random_hex40() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    (0..3)
        .map(|_| {
            let mut h = RandomState::new().build_hasher();
            h.write_u128(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),
            );
            format!("{:016X}", h.finish())
        })
        .collect::<String>()[..40]
        .to_owned()
}
