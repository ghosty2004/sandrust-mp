//! Sans-IO RakNet connection: reliability, acks, resends, ordering,
//! sequencing, split reassembly, internal ping/pong and keep-alive.
//!
//! The caller feeds received datagrams to [`Connection::handle_datagram`],
//! calls [`Connection::poll`] regularly to obtain plaintext datagrams to send
//! (client→server datagrams must then go through `cipher::encrypt`), and
//! queues application messages with [`Connection::send`].
//!
//! Spec: docs/re/handshake.md §3. Resend/ack timing was not recovered from the
//! original client; the policy here is our own (see constants). Two rules are samp's
//! RakNet's (A addresses, docs/phase3/m10-notes.md "M10-P"):
//! * keep-alive: a reliable `0x17` when more than 5000 ms passed since the last reliable send
//!   the application queued; after a keep-alive the next one is due 10000 ms (the timeout)
//!   later still, i.e. 15 s apart while idle (update cycle A 0x10041853, send A 0x1003cbf9);
//! * dead connection ([`Connection::is_dead`]): the "ack timer" starts when a reliable message
//!   is first transmitted while none is awaiting an ack, is moved to the receive time of every
//!   ack that leaves messages pending and is cleared when none are; the connection is dead when
//!   messages are pending and the timer is more than 10000 ms old (A 0x10049b8b, 0x10045b70,
//!   0x10049390).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use crate::bitstream::BitWriter;
use crate::raknet::{AckRange, Datagram, Message, Ordering, Reliability, Split, ack_ranges, id};

/// Usable bytes per datagram: MTU 576 − 28 (IP+UDP) − 1 (cipher checksum) − small slack.
const MAX_DATAGRAM_BYTES: usize = 540;
/// Keep-alive after more than this since the last reliable send (A 0x10041853 `5000 <`).
const KEEP_ALIVE_AFTER: Duration = Duration::from_millis(5000);
/// RakNet's timeout (default of the reliability layer, A 0x10048e10; samp never changes it): the
/// dead-connection limit, and the extra delay of the keep-alive after a keep-alive.
pub const TIMEOUT: Duration = Duration::from_millis(10000);
const PING_INTERVAL: Duration = Duration::from_millis(5000);
const MIN_RESEND: Duration = Duration::from_millis(100);
const MAX_RESEND: Duration = Duration::from_millis(1000);
/// How many recent reliable message numbers are remembered for duplicate detection.
const DEDUP_WINDOW: usize = 8192;
const CHANNELS: usize = 32;

/// A message delivered to the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivered {
    pub data: Vec<u8>,
    pub bit_len: usize,
}

struct Pending {
    msg: Message,
    last_sent: Option<Instant>,
}

pub struct Connection {
    start: Instant,
    next_number: u16,
    ordered_out: [u16; CHANNELS],
    sequenced_out: [u16; CHANNELS],
    /// Not yet sent, or sent and awaiting ack (reliable), keyed by message number.
    outgoing: BTreeMap<u16, Pending>,
    /// Message numbers to acknowledge in the next datagram.
    acks_due: Vec<u16>,
    seen: HashSet<u16>,
    seen_order: VecDeque<u16>,
    ordered_expected: [u16; CHANNELS],
    ordered_buffer: [HashMap<u16, Delivered>; CHANNELS],
    sequenced_last: [Option<u16>; CHANNELS],
    splits: HashMap<u16, Vec<Option<Delivered>>>,
    /// Last reliable message queued by the application (keep-alive base; may lie in the
    /// future after a keep-alive).
    last_reliable_send: Instant,
    /// The ack timer of [`Self::is_dead`] (`None` = cleared).
    ack_timer: Option<Instant>,
    last_ping: Option<Instant>,
    /// Smoothed round-trip time from internal pings.
    pub rtt: Duration,
}

/// Is `a` newer than `b` in 16-bit wrapping arithmetic?
fn newer(a: u16, b: u16) -> bool {
    (a.wrapping_sub(b) as i16) > 0
}

impl Connection {
    pub fn new(now: Instant) -> Self {
        Self {
            start: now,
            next_number: 0,
            ordered_out: [0; CHANNELS],
            sequenced_out: [0; CHANNELS],
            outgoing: BTreeMap::new(),
            acks_due: Vec::new(),
            seen: HashSet::new(),
            seen_order: VecDeque::new(),
            ordered_expected: [0; CHANNELS],
            ordered_buffer: std::array::from_fn(|_| HashMap::new()),
            sequenced_last: [None; CHANNELS],
            splits: HashMap::new(),
            last_reliable_send: now,
            ack_timer: None,
            last_ping: None,
            rtt: Duration::from_millis(300),
        }
    }

    /// RakNet time: milliseconds since the connection object was created.
    pub fn time_ms(&self, now: Instant) -> u32 {
        now.duration_since(self.start).as_millis() as u32
    }

    /// Queue a message. `bit_len` is the exact payload length in bits.
    pub fn send(&mut self, data: &[u8], bit_len: usize, reliability: Reliability, channel: u8) {
        assert!(bit_len.div_ceil(8) <= MAX_DATAGRAM_BYTES - 16, "outgoing split packets are not implemented");
        let c = usize::from(channel) % CHANNELS;
        let ordering = match reliability {
            Reliability::ReliableOrdered => {
                let index = self.ordered_out[c];
                self.ordered_out[c] = index.wrapping_add(1);
                Some(Ordering { channel, index })
            }
            Reliability::UnreliableSequenced | Reliability::ReliableSequenced => {
                let index = self.sequenced_out[c];
                self.sequenced_out[c] = index.wrapping_add(1);
                Some(Ordering { channel, index })
            }
            _ => None,
        };
        let number = self.next_number;
        self.next_number = number.wrapping_add(1);
        let msg = Message { number, reliability, ordering, split: None, bit_len: bit_len as u16, data: data.to_vec() };
        self.outgoing.insert(number, Pending { msg, last_sent: None });
    }

    pub fn send_bytes(&mut self, data: &[u8], reliability: Reliability, channel: u8) {
        self.send(data, data.len() * 8, reliability, channel);
    }

    pub fn send_bits(&mut self, w: &BitWriter, reliability: Reliability, channel: u8) {
        self.send(w.as_bytes(), w.bit_len(), reliability, channel);
    }

    /// Process one received (plaintext) datagram; returns delivered messages
    /// in delivery order. Internal ping/pong is handled here and not delivered.
    pub fn handle_datagram(
        &mut self,
        bytes: &[u8],
        now: Instant,
    ) -> Result<Vec<Delivered>, crate::raknet::DecodeError> {
        let dg = Datagram::decode(bytes)?;
        for AckRange { min, max } in dg.acks {
            let mut n = min;
            loop {
                if self.outgoing.get(&n).is_some_and(|p| p.last_sent.is_some()) {
                    self.outgoing.remove(&n);
                }
                // Every acked number moves the ack timer (A 0x10049390).
                self.ack_timer = if self.awaiting_ack() > 0 { Some(now) } else { None };
                if n == max {
                    break;
                }
                n = n.wrapping_add(1);
            }
        }
        let mut out = Vec::new();
        for m in dg.messages {
            if m.reliability.is_reliable() {
                self.acks_due.push(m.number);
                if !self.remember(m.number) {
                    continue; // duplicate (our ack was lost and the peer resent)
                }
            }
            let Some(d) = self.reassemble(&m) else { continue };
            self.order(&m, d, &mut out);
        }
        out.retain(|d| !self.handle_internal(d, now));
        Ok(out)
    }

    fn remember(&mut self, number: u16) -> bool {
        if !self.seen.insert(number) {
            return false;
        }
        self.seen_order.push_back(number);
        if self.seen_order.len() > DEDUP_WINDOW {
            let old = self.seen_order.pop_front().unwrap();
            self.seen.remove(&old);
        }
        true
    }

    fn reassemble(&mut self, m: &Message) -> Option<Delivered> {
        let piece = Delivered { data: m.data.clone(), bit_len: m.bit_len.into() };
        let Some(Split { id, index, count }) = m.split else { return Some(piece) };
        if count == 0 || index >= count || count > 4096 {
            return None;
        }
        let parts = self.splits.entry(id).or_insert_with(|| vec![None; count as usize]);
        if parts.len() != count as usize {
            return None;
        }
        parts[index as usize] = Some(piece);
        if parts.iter().any(Option::is_none) {
            return None;
        }
        let parts = self.splits.remove(&id).unwrap();
        let mut w = BitWriter::new();
        for p in parts.into_iter().flatten() {
            w.write_bits_from(&p.data, p.bit_len);
        }
        Some(Delivered { bit_len: w.bit_len(), data: w.into_bytes() })
    }

    fn order(&mut self, m: &Message, d: Delivered, out: &mut Vec<Delivered>) {
        let Some(Ordering { channel, index }) = m.ordering else {
            out.push(d);
            return;
        };
        let c = usize::from(channel) % CHANNELS;
        if m.reliability.is_sequenced() {
            if self.sequenced_last[c].is_none_or(|last| newer(index, last)) {
                self.sequenced_last[c] = Some(index);
                out.push(d);
            }
            return;
        }
        // Reliable ordered.
        let expected = self.ordered_expected[c];
        if index == expected {
            out.push(d);
            let mut next = expected.wrapping_add(1);
            while let Some(buffered) = self.ordered_buffer[c].remove(&next) {
                out.push(buffered);
                next = next.wrapping_add(1);
            }
            self.ordered_expected[c] = next;
        } else if newer(index, expected) {
            self.ordered_buffer[c].insert(index, d);
        }
    }

    /// Returns true if the message was internal and consumed.
    fn handle_internal(&mut self, d: &Delivered, now: Instant) -> bool {
        match d.data.as_slice() {
            [id::INTERNAL_PING, a, b, c, e] if d.bit_len == 40 => {
                let mut pong = vec![id::CONNECTED_PONG, *a, *b, *c, *e];
                pong.extend_from_slice(&self.time_ms(now).to_le_bytes());
                self.send_bytes(&pong, Reliability::Unreliable, 0);
                true
            }
            [id::CONNECTED_PONG, a, b, c, e, ..] if d.bit_len == 72 => {
                let sent = u32::from_le_bytes([*a, *b, *c, *e]);
                let sample = Duration::from_millis(self.time_ms(now).wrapping_sub(sent).into());
                if sample < Duration::from_millis(1200) {
                    self.rtt = (self.rtt * 3 + sample) / 4;
                }
                true
            }
            _ => false,
        }
    }

    /// Queue an internal ping now (also done automatically every 5 s).
    pub fn ping(&mut self, now: Instant) {
        let mut p = vec![id::INTERNAL_PING];
        p.extend_from_slice(&self.time_ms(now).to_le_bytes());
        self.send_bytes(&p, Reliability::Unreliable, 0);
        self.last_ping = Some(now);
    }

    fn resend_after(&self) -> Duration {
        (self.rtt * 3).clamp(MIN_RESEND, MAX_RESEND)
    }

    /// Plaintext datagrams to send now.
    pub fn poll(&mut self, now: Instant) -> Vec<Vec<u8>> {
        if self.last_ping.is_some_and(|t| now.duration_since(t) >= PING_INTERVAL) {
            self.ping(now);
        }
        // Reliable messages queued since the last poll (this frame) count as sent now.
        if self.outgoing.values().any(|p| p.last_sent.is_none() && p.msg.reliability.is_reliable()) {
            self.last_reliable_send = now;
        }
        if now > self.last_reliable_send && now - self.last_reliable_send > KEEP_ALIVE_AFTER {
            self.send_bytes(&[id::KEEP_ALIVE], Reliability::Reliable, 0);
            self.last_reliable_send = now + TIMEOUT;
        }

        let resend_after = self.resend_after();
        let mut due: Vec<u16> = Vec::new();
        for (&n, p) in &self.outgoing {
            if p.last_sent.is_none_or(|t| now.duration_since(t) >= resend_after) {
                due.push(n);
            }
        }

        let mut acks = ack_ranges(&mut std::mem::take(&mut self.acks_due));
        let mut datagrams = Vec::new();
        let mut current = Datagram { acks: std::mem::take(&mut acks), messages: Vec::new() };
        let mut size = current.encode().len();
        for n in due {
            let awaiting_ack = self.awaiting_ack();
            let p = self.outgoing.get_mut(&n).unwrap();
            if p.last_sent.is_none()
                && p.msg.reliability.is_reliable()
                && (awaiting_ack == 0 || self.ack_timer.is_none())
            {
                self.ack_timer = Some(now);
            }
            let msg_size = p.msg.data.len() + 12;
            if size + msg_size > MAX_DATAGRAM_BYTES && !current.messages.is_empty() {
                datagrams.push(std::mem::take(&mut current).encode());
                size = 1;
            }
            size += msg_size;
            current.messages.push(p.msg.clone());
            p.last_sent = Some(now);
        }
        // Unreliable messages are sent once.
        self.outgoing.retain(|_, p| p.msg.reliability.is_reliable() || p.last_sent.is_none());
        if !current.acks.is_empty() || !current.messages.is_empty() {
            datagrams.push(current.encode());
        }
        datagrams
    }

    /// Number of reliable messages still awaiting acknowledgement.
    pub fn unacked(&self) -> usize {
        self.outgoing.values().filter(|p| p.msg.reliability.is_reliable()).count()
    }

    /// Reliable messages transmitted and not yet acknowledged (RakNet's resend list).
    fn awaiting_ack(&self) -> usize {
        self.outgoing.values().filter(|p| p.last_sent.is_some() && p.msg.reliability.is_reliable()).count()
    }

    /// RakNet's dead-connection test (A 0x10049b8b): messages await an ack and the ack timer is
    /// more than [`TIMEOUT`] old. The caller reports the connection lost (`0x21`).
    pub fn is_dead(&self, now: Instant) -> bool {
        self.awaiting_ack() > 0 && self.ack_timer.is_some_and(|t| now > t && now - t > TIMEOUT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Connection, Connection, Instant) {
        let t = Instant::now();
        (Connection::new(t), Connection::new(t), t)
    }

    fn pump(from: &mut Connection, to: &mut Connection, now: Instant) -> Vec<Delivered> {
        let mut got = Vec::new();
        for dg in from.poll(now) {
            got.extend(to.handle_datagram(&dg, now).unwrap());
        }
        got
    }

    #[test]
    fn reliable_delivery_and_ack() {
        let (mut a, mut b, t) = pair();
        a.send_bytes(&[0x0b], Reliability::Reliable, 0);
        let got = pump(&mut a, &mut b, t);
        assert_eq!(got, vec![Delivered { data: vec![0x0b], bit_len: 8 }]);
        assert_eq!(a.unacked(), 1);
        pump(&mut b, &mut a, t); // b sends the ack
        assert_eq!(a.unacked(), 0);
    }

    #[test]
    fn resend_after_loss_and_dedup() {
        let (mut a, mut b, t) = pair();
        a.send_bytes(&[1], Reliability::Reliable, 0);
        let _lost = a.poll(t);
        assert!(a.poll(t + Duration::from_millis(10)).is_empty(), "too early to resend");
        let later = t + Duration::from_secs(2);
        assert_eq!(pump(&mut a, &mut b, later).len(), 1);
        // A duplicate resend (ack lost) is acked again but not delivered twice.
        let again = later + Duration::from_secs(2);
        assert!(pump(&mut a, &mut b, again).is_empty());
    }

    #[test]
    fn ordered_channel_reorders() {
        let (mut a, mut b, t) = pair();
        for i in 0..3u8 {
            a.send_bytes(&[0x30 + i], Reliability::ReliableOrdered, 2);
        }
        let mut dgs = a.poll(t);
        assert_eq!(dgs.len(), 1);
        // Re-encode the messages in reverse order to simulate reordering.
        let mut d = Datagram::decode(&dgs.pop().unwrap()).unwrap();
        d.messages.reverse();
        let got = b.handle_datagram(&d.encode(), t).unwrap();
        let bytes: Vec<u8> = got.iter().map(|d| d.data[0]).collect();
        assert_eq!(bytes, vec![0x30, 0x31, 0x32]);
    }

    #[test]
    fn sequenced_drops_older() {
        let (_, mut b, t) = pair();
        let m = |idx: u16, n: u16| Message {
            number: n,
            reliability: Reliability::UnreliableSequenced,
            ordering: Some(Ordering { channel: 1, index: idx }),
            split: None,
            bit_len: 8,
            data: vec![idx as u8],
        };
        let d = Datagram { acks: vec![], messages: vec![m(5, 0), m(3, 1), m(6, 2)] };
        let got: Vec<u8> = b.handle_datagram(&d.encode(), t).unwrap().iter().map(|d| d.data[0]).collect();
        assert_eq!(got, vec![5, 6]);
    }

    #[test]
    fn split_reassembly() {
        let (_, mut b, t) = pair();
        let piece = |i: u32, n: u16, data: &[u8]| Message {
            number: n,
            reliability: Reliability::Reliable,
            ordering: None,
            split: Some(Split { id: 9, index: i, count: 3 }),
            bit_len: (data.len() * 8) as u16,
            data: data.to_vec(),
        };
        let d =
            Datagram { acks: vec![], messages: vec![piece(2, 2, &[5, 6]), piece(0, 0, &[1, 2]), piece(1, 1, &[3, 4])] };
        let got = b.handle_datagram(&d.encode(), t).unwrap();
        assert_eq!(got, vec![Delivered { data: vec![1, 2, 3, 4, 5, 6], bit_len: 48 }]);
    }

    /// Keep-alive after > 5 s without a reliable send, then 15 s apart while idle (A 0x10041853).
    /// (Unacked messages are resent, so new messages are counted by their numbers.)
    #[test]
    fn keep_alive_cadence() {
        let (mut a, _, t) = pair();
        let ms = |n| t + Duration::from_millis(n);
        let at = |a: &mut Connection, n| {
            let before = a.next_number;
            a.poll(ms(n));
            a.next_number.wrapping_sub(before)
        };
        a.send_bytes(&[1], Reliability::Reliable, 0);
        assert_eq!(at(&mut a, 0), 0);
        assert_eq!(at(&mut a, 5000), 0, "exactly 5000 ms is not enough");
        assert_eq!(at(&mut a, 5001), 1);
        assert_eq!(at(&mut a, 20_001), 0);
        assert_eq!(at(&mut a, 20_002), 1, "the next keep-alive 15 s after the previous");
        // An application send resets the base to now.
        a.send_bytes(&[2], Reliability::ReliableOrdered, 0);
        assert_eq!(at(&mut a, 21_000), 0);
        assert_eq!(at(&mut a, 26_000), 0);
        assert_eq!(at(&mut a, 26_001), 1);
    }

    /// Dead connection: an unacked reliable message and an ack timer older than 10 s.
    #[test]
    fn dead_connection_rule() {
        let (mut a, mut b, t) = pair();
        let ms = |n| t + Duration::from_millis(n);
        // Nothing pending: never dead.
        assert!(!a.is_dead(ms(60_000)));
        a.send_bytes(&[1], Reliability::Reliable, 0);
        let dgs = a.poll(ms(1000));
        assert!(!a.is_dead(ms(11_000)), "exactly 10 s is not more than the timeout");
        assert!(a.is_dead(ms(11_001)));
        // An ack that leaves another message pending moves the timer to its receive time.
        a.send_bytes(&[2], Reliability::Reliable, 0);
        a.poll(ms(1500));
        for d in dgs {
            b.handle_datagram(&d, ms(1600)).unwrap();
        }
        for d in b.poll(ms(1600)).into_iter().filter(|d| !Datagram::decode(d).unwrap().acks.is_empty()) {
            a.handle_datagram(&d, ms(9000)).unwrap();
        }
        assert!(!a.is_dead(ms(19_000)));
        assert!(a.is_dead(ms(19_001)));
        // Unreliable traffic does not start the timer.
        let (mut c, _, t) = pair();
        c.send_bytes(&[3], Reliability::UnreliableSequenced, 1);
        c.poll(t);
        assert!(!c.is_dead(t + Duration::from_secs(60)));
    }

    #[test]
    fn answers_internal_ping() {
        let (mut a, mut b, t) = pair();
        a.ping(t);
        assert!(pump(&mut a, &mut b, t).is_empty(), "ping is internal");
        let t2 = t + Duration::from_millis(40);
        assert!(pump(&mut b, &mut a, t2).is_empty(), "pong is internal");
        assert!(a.rtt < Duration::from_millis(300), "rtt sample applied: {:?}", a.rtt);
    }
}
