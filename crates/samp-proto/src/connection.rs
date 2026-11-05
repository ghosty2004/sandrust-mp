//! Sans-IO RakNet connection: reliability, acks, resends, ordering,
//! sequencing, split packets, internal ping/pong and keep-alive.
//!
//! The caller feeds received datagrams to [`Connection::handle_datagram`],
//! calls [`Connection::poll`] regularly to obtain plaintext datagrams to send
//! (client→server datagrams must then go through `cipher::encrypt`), and
//! queues application messages with [`Connection::send`].
//!
//! Spec: docs/re/handshake.md §3. The rules below are those of samp's RakNet (A = 2021 build
//! addresses; measured on the original client in docs/phase3/net6-notes.md and m10-notes.md):
//! * one [`Connection::poll`] is one reliability-layer update (A 0x10049ae0). samp's RakNet runs
//!   it on its own thread every 2 ms (`threadSleepTimer` 2, A 0x10008988); the caller decides the
//!   cadence (`samp-client` runs a 2 ms network thread, the bots poll per loop);
//! * datagram generation (A 0x10048070): pending acks first, then the due resends (oldest first;
//!   a resend that does not fit ends the datagram), then the new messages by priority
//!   (0 system, 1 high, 2 medium, 3 low; FIFO inside a priority; a message that does not fit
//!   waits and the next priority is tried); as many datagrams as needed per update. A message fits
//!   when the datagram's bits + its header estimate + its payload bits stay within
//!   (MTU − 28) × 8; MTU 576 (samp never calls SetMTUSize);
//! * resend timeout (A 0x10045c00 / 0x10044070): 3 × ping, ping = the last accepted internal-ping
//!   sample (at least 30 ms; 500 when above the timeout), 1000 before the first sample, so
//!   3000 ms. No backoff: a resent message is due again one timeout later;
//! * split packets (A 0x10046b00 / 0x10045800): a message whose payload exceeds
//!   MTU − 28 − ⌈header estimate / 8⌉ bytes is cut into pieces of that size; the first piece keeps
//!   the message number, the others take the next numbers, all share the ordering index;
//! * internal ping (A 0x10041450): the application pings once on `0x22`; the update cycle pings
//!   again (5 s apart) only while no pong has been accepted yet, so normally once more right
//!   after connecting and never again. Pings are answered with a pong; both go out with the
//!   NEXT update (they are handled after the update in RakNet's cycle);
//! * keep-alive (A 0x10041853): a reliable `0x17` (low priority) when connected, nothing awaits an
//!   ack and more than 5000 ms passed since the last reliable send; the next base is moved to
//!   now + timeout, i.e. 15 s apart while idle (send A 0x1003cbf9);
//! * duplicate filter (A 0x10049250, docs/re/handshake.md §3): a window over the peer's single
//!   16-bit message-number counter (every message, reliable or not), with a base and holes that are
//!   given up after [`TIMEOUT`]; robust across the counter's wrap;
//! * dead connection ([`Connection::is_dead`]): the "ack timer" starts when a reliable message
//!   is first transmitted while none is awaiting an ack, is moved to the receive time of every
//!   ack that leaves messages pending and is cleared when none are; the connection is dead when
//!   messages are pending and the timer is more than 10000 ms old (A 0x10049b8b, 0x10045b70,
//!   0x10049390).
//!
//! Not reproduced: RakNet's bandwidth bucket (A 0x10049ae0; floor 144400 bit/s, a 1-second
//! burst): the client's traffic stays far below it (docs/phase3/net6-notes.md).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::bitstream::BitWriter;
use crate::raknet::{AckRange, Datagram, Message, Ordering, Reliability, Split, ack_ranges, id};

/// RakNet's default MTU (A 0x1003de50); samp never changes it.
pub const MTU: usize = 576;
/// IP + UDP header bytes RakNet subtracts from the MTU.
const UDP_HEADER: usize = 28;
/// Message bits a datagram may carry: (MTU − 28) × 8 (A 0x10048070).
const DATAGRAM_BITS: usize = (MTU - UDP_HEADER) * 8;
/// Ack bits a datagram may carry (MTU × 8 − 0xE1, A 0x10048070).
const ACK_BITS: usize = MTU * 8 - 0xE1;
/// Keep-alive after more than this many ms since the last reliable send (A 0x10041853 `5000 <`),
/// compared on RakNet's whole-millisecond clock (µs time / 1000, A 0x1003cc13 / 0x10041660).
const KEEP_ALIVE_AFTER_MS: u64 = 5000;
/// RakNet's timeout (default of the reliability layer, A 0x10048e10; samp never changes it): the
/// dead-connection limit, and the extra delay of the keep-alive after a keep-alive.
pub const TIMEOUT: Duration = Duration::from_millis(10000);
/// The update cycle's ping interval while no pong was accepted (A 0x10041a2c `+ 5000`).
const PING_INTERVAL: Duration = Duration::from_millis(5000);
/// Ping before the first sample (reliability-layer reset A 0x10047780: SetPing(1000)).
const INITIAL_PING_MS: u32 = 1000;
/// Lowest ping the resend timeout uses (A 0x10045c00).
const MIN_PING_MS: u32 = 30;
/// Ping used when a sample exceeds the timeout (A 0x10045c00).
const OVER_TIMEOUT_PING_MS: u32 = 500;
/// Pong samples at or above this are ignored unless their ring slot is empty (A 0x10041ee0).
const MAX_PING_SAMPLE_MS: u32 = 1200;
/// Samples in the ping ring (A 0x10041f40).
const PING_RING: usize = 5;
/// Largest forward distance (16-bit wrapping) of a received message number from the window base
/// that still counts as new; anything further is behind the base, i.e. a duplicate (A 0x10049250).
const MAX_AHEAD: u16 = 0x8000;
const CHANNELS: usize = 32;

/// RakNet send priorities (PacketPriority).
pub mod priority {
    pub const SYSTEM: u8 = 0;
    pub const HIGH: u8 = 1;
    pub const MEDIUM: u8 = 2;
    pub const LOW: u8 = 3;
}
const PRIORITIES: usize = 4;

/// A message delivered to the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivered {
    pub data: Vec<u8>,
    pub bit_len: usize,
}

struct Pending {
    msg: Message,
    /// Transmitted at least once (reliable messages stay until acked).
    sent: bool,
    /// When a reliable message is due for a resend.
    next_resend: Option<Instant>,
}

pub struct Connection {
    start: Instant,
    next_number: u16,
    next_split_id: u16,
    ordered_out: [u16; CHANNELS],
    sequenced_out: [u16; CHANNELS],
    /// Not yet sent, or sent and awaiting ack (reliable), keyed by message number.
    outgoing: BTreeMap<u16, Pending>,
    /// New messages per priority, in send order.
    queues: [VecDeque<u16>; PRIORITIES],
    /// Transmitted reliable messages in resend order (acked ones are skipped lazily).
    resend: VecDeque<u16>,
    /// Internal replies queued while handling a datagram: they enter the queues after the next
    /// update, like RakNet's handling after its update.
    deferred: Vec<(Vec<u8>, Reliability, u8)>,
    /// Message numbers to acknowledge in the next datagram.
    acks_due: Vec<u16>,
    /// Received-number window (A 0x10049250): the lowest number not yet accounted for ...
    recv_base: u16,
    /// ... and one slot per number from `recv_base` on: `None` = received, `Some(t)` = a hole that
    /// is given up at `t` (receive time + [`TIMEOUT`]).
    recv_slots: VecDeque<Option<Instant>>,
    ordered_expected: [u16; CHANNELS],
    ordered_buffer: [HashMap<u16, Delivered>; CHANNELS],
    sequenced_last: [Option<u16>; CHANNELS],
    splits: HashMap<u16, Vec<Option<Delivered>>>,
    /// RakNet's "connected" state (8): keep-alives and the update cycle's ping run only then.
    connected: bool,
    /// RakNet ms ([`Self::clock_ms`]) of the update that took the last reliable message into the
    /// layer (keep-alive base; lies in the future after a keep-alive).
    last_reliable_ms: u64,
    /// The ack timer of [`Self::is_dead`] (`None` = cleared).
    ack_timer: Option<Instant>,
    /// When the update cycle's next ping is due (set once connected).
    next_ping: Option<Instant>,
    ping_ring: [u16; PING_RING],
    ping_slot: usize,
    /// Lowest accepted ping sample (`None` = no pong accepted yet).
    lowest_ping: Option<u16>,
    /// The reliability layer's ping (ms), the base of the resend timeout.
    ping_ms: u32,
    /// Last accepted round-trip sample.
    pub rtt: Duration,
}

/// Is `a` newer than `b` in 16-bit wrapping arithmetic?
fn newer(a: u16, b: u16) -> bool {
    (a.wrapping_sub(b) as i16) > 0
}

/// RakNet's per-message header estimate in bits (A 0x10043ec0): 4 reliability bits + 32,
/// + 21 for ordered/sequenced messages, + 97 when split, else + 17.
fn header_estimate(reliability: Reliability, split: bool) -> usize {
    let base = 4 + if reliability.is_ordered_or_sequenced() { 0x35 } else { 0x20 };
    base + if split { 0x61 } else { 0x11 }
}

/// Encoded size in bits of an ack list (hasAcks bit excluded).
fn ack_bits(ranges: &[AckRange]) -> usize {
    let mut w = BitWriter::new();
    w.write_compressed_u16(ranges.len() as u16);
    w.bit_len() + ranges.iter().map(|r| if r.min == r.max { 17 } else { 33 }).sum::<usize>()
}

impl Connection {
    /// A connection in RakNet's connected state (keep-alives run from the start). Used by the
    /// bots; the in-game client uses [`Self::connecting`] and [`Self::mark_connected`].
    pub fn new(now: Instant) -> Self {
        let mut c = Self::connecting(now);
        c.connected = true;
        c
    }

    /// A connection that is still being set up (the RakNet request is out, `0x22` not yet
    /// received): no keep-alives, no update-cycle pings until [`Self::mark_connected`].
    pub fn connecting(now: Instant) -> Self {
        Self {
            start: now,
            next_number: 0,
            next_split_id: 0,
            ordered_out: [0; CHANNELS],
            sequenced_out: [0; CHANNELS],
            outgoing: BTreeMap::new(),
            queues: std::array::from_fn(|_| VecDeque::new()),
            resend: VecDeque::new(),
            deferred: Vec::new(),
            acks_due: Vec::new(),
            recv_base: 0,
            recv_slots: VecDeque::new(),
            ordered_expected: [0; CHANNELS],
            ordered_buffer: std::array::from_fn(|_| HashMap::new()),
            sequenced_last: [None; CHANNELS],
            splits: HashMap::new(),
            connected: false,
            last_reliable_ms: 0,
            ack_timer: None,
            next_ping: None,
            ping_ring: [0; PING_RING],
            ping_slot: 0,
            lowest_ping: None,
            ping_ms: INITIAL_PING_MS,
            rtt: Duration::from_millis(INITIAL_PING_MS.into()),
        }
    }

    /// RakNet's state 8 (connected, after `0x22`): keep-alives start and the update cycle pings
    /// from the next update on while no pong has been accepted.
    pub fn mark_connected(&mut self, now: Instant) {
        if !self.connected || self.next_ping.is_none() {
            self.connected = true;
            self.next_ping = Some(now);
        }
    }

    /// RakNet time: milliseconds since the connection object was created.
    pub fn time_ms(&self, now: Instant) -> u32 {
        now.duration_since(self.start).as_millis() as u32
    }

    /// RakNet's millisecond clock: whole ms since the connection object was created (truncated,
    /// like RakNet's µs time / 1000).
    fn clock_ms(&self, now: Instant) -> u64 {
        now.duration_since(self.start).as_millis() as u64
    }

    /// The reliability layer's ping (ms) the resend timeout is based on.
    pub fn ping_ms(&self) -> u32 {
        self.ping_ms
    }

    /// Resend timeout: 3 × ping (A 0x10044070).
    pub fn resend_timeout(&self) -> Duration {
        Duration::from_millis(u64::from(self.ping_ms) * 3)
    }

    /// Queue a message with high priority (every samp RPC and sync packet uses it).
    /// `bit_len` is the exact payload length in bits.
    pub fn send(&mut self, data: &[u8], bit_len: usize, reliability: Reliability, channel: u8) {
        self.send_with(data, bit_len, reliability, channel, priority::HIGH);
    }

    /// Queue a message with an explicit RakNet priority (0..=3, see [`priority`]).
    pub fn send_with(&mut self, data: &[u8], bit_len: usize, reliability: Reliability, channel: u8, prio: u8) {
        if bit_len == 0 {
            return; // RakNet drops empty sends (A 0x10046b2e).
        }
        let prio = usize::from(prio).min(PRIORITIES - 1);
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
        let bytes = bit_len.div_ceil(8);
        let limit = MTU - UDP_HEADER - header_estimate(reliability, false).div_ceil(8);
        if bytes <= limit {
            let number = self.take_number();
            let msg =
                Message { number, reliability, ordering, split: None, bit_len: bit_len as u16, data: data.to_vec() };
            self.enqueue(msg, prio);
            return;
        }
        // Split (A 0x10045800): pieces of MTU − 28 − ⌈split header estimate / 8⌉ bytes.
        let piece = MTU - UDP_HEADER - header_estimate(reliability, true).div_ceil(8);
        let count = (bytes - 1) / piece + 1;
        let split_id = self.next_split_id;
        self.next_split_id = split_id.wrapping_add(1);
        for i in 0..count {
            let from = i * piece;
            let len = piece.min(bytes - from);
            let bits = if i + 1 == count { bit_len - piece * 8 * i } else { len * 8 };
            let number = self.take_number();
            let msg = Message {
                number,
                reliability,
                ordering,
                split: Some(Split { id: split_id, index: i as u32, count: count as u32 }),
                bit_len: bits as u16,
                data: data[from..from + len].to_vec(),
            };
            self.enqueue(msg, prio);
        }
    }

    fn take_number(&mut self) -> u16 {
        let n = self.next_number;
        self.next_number = n.wrapping_add(1);
        n
    }

    fn enqueue(&mut self, msg: Message, prio: usize) {
        let number = msg.number;
        self.outgoing.insert(number, Pending { msg, sent: false, next_resend: None });
        self.queues[prio].push_back(number);
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
                if self.outgoing.get(&n).is_some_and(|p| p.sent && p.msg.reliability.is_reliable()) {
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
                self.acks_due.push(m.number); // acked even when it turns out to be a duplicate
            }
            if !self.accept_number(m.number, now) {
                continue; // duplicate (our ack was lost and the peer resent) or too old
            }
            let Some(d) = self.reassemble(&m) else { continue };
            self.order(&m, d, &mut out);
        }
        out.retain(|d| !self.handle_internal(d, now));
        Ok(out)
    }

    /// Duplicate filter over every received message number, reliable or not (the sender numbers all
    /// its messages from one 16-bit counter). Rule of A 0x10049250: with d = number − base (wrapping),
    /// d == 0 takes the base slot; 1 ≤ d ≤ 0x8000 is new unless its slot was already received (a
    /// number beyond the window opens holes up to it, each given up [`TIMEOUT`] after it opened);
    /// d > 0x8000 lies behind the base and is dropped. The base then advances over received slots
    /// and expired holes. Unlike a set of recently seen numbers this stays correct across the
    /// counter's wrap (every 65536 messages), so a new message is never taken for an old one.
    fn accept_number(&mut self, number: u16, now: Instant) -> bool {
        let d = number.wrapping_sub(self.recv_base);
        if d == 0 {
            self.recv_slots.pop_front();
            self.recv_base = self.recv_base.wrapping_add(1);
        } else if d <= MAX_AHEAD {
            let i = usize::from(d);
            if i < self.recv_slots.len() {
                if self.recv_slots[i].is_none() {
                    return false;
                }
                self.recv_slots[i] = None;
            } else {
                let hole = Some(now + TIMEOUT);
                self.recv_slots.resize(i, hole);
                self.recv_slots.push_back(None);
            }
        } else {
            return false;
        }
        while let Some(&front) = self.recv_slots.front() {
            if front.is_some_and(|t| now <= t) {
                break;
            }
            self.recv_slots.pop_front();
            self.recv_base = self.recv_base.wrapping_add(1);
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
                self.deferred.push((pong, Reliability::Unreliable, priority::SYSTEM));
                true
            }
            [id::CONNECTED_PONG, a, b, c, e, ..] if d.bit_len == 72 => {
                let sent = u32::from_le_bytes([*a, *b, *c, *e]);
                let now_ms = self.time_ms(now);
                let sample = now_ms.saturating_sub(sent);
                self.ping_sample(sample);
                true
            }
            _ => false,
        }
    }

    /// A pong's round-trip sample (A 0x10041ea0): accepted when its ring slot is empty, or when
    /// it is below 3 × the slot's sample and below 1200 ms; then it becomes the layer's ping.
    fn ping_sample(&mut self, sample: u32) {
        let slot = u32::from(self.ping_ring[self.ping_slot]);
        if slot != 0 && !(sample < slot * 3 && sample < MAX_PING_SAMPLE_MS) {
            return;
        }
        let s = sample.min(u32::from(u16::MAX)) as u16;
        self.ping_ring[self.ping_slot] = s;
        if self.lowest_ping.is_none_or(|l| s < l) {
            self.lowest_ping = Some(s);
        }
        self.ping_ms = if sample > TIMEOUT.as_millis() as u32 { OVER_TIMEOUT_PING_MS } else { sample };
        self.ping_ms = self.ping_ms.max(MIN_PING_MS);
        self.rtt = Duration::from_millis(sample.into());
        self.ping_slot = (self.ping_slot + 1) % PING_RING;
    }

    /// Queue an internal ping now (unreliable, system priority).
    pub fn ping(&mut self, now: Instant) {
        let mut p = vec![id::INTERNAL_PING];
        p.extend_from_slice(&self.time_ms(now).to_le_bytes());
        self.send_with(&p, p.len() * 8, Reliability::Unreliable, 0, priority::SYSTEM);
    }

    /// One reliability-layer update: the plaintext datagrams to send now.
    pub fn poll(&mut self, now: Instant) -> Vec<Vec<u8>> {
        // Reliable messages queued since the last update count as sent at this update's time
        // (RakNet stores the cycle's time, in whole ms, when it takes a buffered send).
        let now_ms = self.clock_ms(now);
        if self.queues.iter().flatten().any(|n| self.outgoing.get(n).is_some_and(|p| p.msg.reliability.is_reliable())) {
            self.last_reliable_ms = now_ms;
        }
        if self.connected
            && self.awaiting_ack() == 0
            && now_ms > self.last_reliable_ms
            && now_ms - self.last_reliable_ms > KEEP_ALIVE_AFTER_MS
        {
            self.send_with(&[id::KEEP_ALIVE], 8, Reliability::Reliable, 0, priority::LOW);
            self.last_reliable_ms = now_ms + TIMEOUT.as_millis() as u64;
        }

        let mut datagrams = Vec::new();
        loop {
            let dg = self.generate(now);
            if dg.acks.is_empty() && dg.messages.is_empty() {
                break;
            }
            datagrams.push(dg.encode());
        }

        // RakNet's cycle handles received internal messages and pings after the update.
        for (data, rel, prio) in std::mem::take(&mut self.deferred) {
            self.send_with(&data, data.len() * 8, rel, 0, prio);
        }
        if self.connected && self.lowest_ping.is_none() && self.next_ping.is_some_and(|t| now >= t) {
            self.next_ping = Some(now + PING_INTERVAL);
            self.ping(now);
        }
        datagrams
    }

    /// One datagram (A 0x10048070).
    fn generate(&mut self, now: Instant) -> Datagram {
        let mut dg = Datagram::default();
        let mut bits = 1usize;
        if !self.acks_due.is_empty() {
            let mut ranges = ack_ranges(&mut std::mem::take(&mut self.acks_due));
            let mut rest = Vec::new();
            while ack_bits(&ranges) > ACK_BITS {
                rest.push(ranges.pop().unwrap());
            }
            for r in rest.into_iter().rev() {
                let mut n = r.min;
                loop {
                    self.acks_due.push(n);
                    if n == r.max {
                        break;
                    }
                    n = n.wrapping_add(1);
                }
            }
            bits += ack_bits(&ranges);
            dg.acks = ranges;
        }
        let timeout = self.resend_timeout();
        // Resends, oldest first; one that does not fit ends the datagram.
        let mut full = false;
        while let Some(&n) = self.resend.front() {
            let Some(p) = self.outgoing.get(&n) else {
                self.resend.pop_front(); // acked
                continue;
            };
            if p.next_resend.is_none_or(|t| now <= t) {
                break;
            }
            let need = header_estimate(p.msg.reliability, p.msg.split.is_some()) + usize::from(p.msg.bit_len);
            if bits + need > DATAGRAM_BITS {
                full = true;
                break;
            }
            bits += need;
            self.resend.pop_front();
            self.resend.push_back(n);
            let p = self.outgoing.get_mut(&n).unwrap();
            p.next_resend = Some(now + timeout);
            dg.messages.push(p.msg.clone());
        }
        if full {
            return dg;
        }
        // New messages by priority.
        for q in 0..PRIORITIES {
            while let Some(&n) = self.queues[q].front() {
                let Some(p) = self.outgoing.get(&n) else {
                    self.queues[q].pop_front();
                    continue;
                };
                let need = header_estimate(p.msg.reliability, p.msg.split.is_some()) + usize::from(p.msg.bit_len);
                if bits + need > DATAGRAM_BITS {
                    break;
                }
                bits += need;
                self.queues[q].pop_front();
                let reliable = p.msg.reliability.is_reliable();
                if reliable {
                    if self.awaiting_ack() == 0 || self.ack_timer.is_none() {
                        self.ack_timer = Some(now);
                    }
                    let p = self.outgoing.get_mut(&n).unwrap();
                    p.sent = true;
                    p.next_resend = Some(now + timeout);
                    dg.messages.push(p.msg.clone());
                    self.resend.push_back(n);
                } else {
                    // Unreliable messages are sent once.
                    let p = self.outgoing.remove(&n).unwrap();
                    dg.messages.push(p.msg);
                }
            }
        }
        dg
    }

    /// Number of reliable messages still awaiting acknowledgement (or not sent yet).
    pub fn unacked(&self) -> usize {
        self.outgoing.values().filter(|p| p.msg.reliability.is_reliable()).count()
    }

    /// Reliable messages transmitted and not yet acknowledged (RakNet's resend list).
    fn awaiting_ack(&self) -> usize {
        self.outgoing.values().filter(|p| p.sent && p.msg.reliability.is_reliable()).count()
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

    fn decode_all(dgs: &[Vec<u8>]) -> Vec<Datagram> {
        dgs.iter().map(|d| Datagram::decode(d).unwrap()).collect()
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

    /// Before any pong the resend timeout is 3 × 1000 ms; no backoff afterwards.
    #[test]
    fn resend_after_loss_and_dedup() {
        let (mut a, mut b, t) = pair();
        let ms = |n| t + Duration::from_millis(n);
        a.send_bytes(&[1], Reliability::Reliable, 0);
        let _lost = a.poll(t);
        assert_eq!(a.resend_timeout(), Duration::from_millis(3000));
        assert!(a.poll(ms(3000)).is_empty(), "exactly the timeout is not enough");
        let _lost = a.poll(ms(3001));
        assert!(a.poll(ms(6001)).is_empty());
        assert_eq!(pump(&mut a, &mut b, ms(6002)).len(), 1, "resent one timeout after the resend");
        // A duplicate resend (ack lost) is acked again but not delivered twice.
        assert!(pump(&mut a, &mut b, ms(9003)).is_empty());
    }

    /// The resend timeout follows the last accepted pong: 3 × max(sample, 30 ms).
    #[test]
    fn resend_timeout_follows_ping() {
        let (mut a, mut b, t) = pair();
        let ms = |n| t + Duration::from_millis(n);
        a.ping(t);
        pump(&mut a, &mut b, t);
        b.poll(ms(1)); // b's pong is queued after its update
        pump(&mut b, &mut a, ms(2)); // and sent with the next one
        assert_eq!(a.ping_ms(), 30, "2 ms sample, raised to 30");
        assert_eq!(a.resend_timeout(), Duration::from_millis(90));
        // A larger sample below 3 × the previous one (ring slot empty → accepted).
        a.ping_sample(80);
        assert_eq!(a.resend_timeout(), Duration::from_millis(240));
        // Samples over the timeout use 500 ms (only when the ring slot is empty).
        a.ping_sample(20_000);
        assert_eq!(a.ping_ms(), 500);
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

    /// The sender numbers every message (syncs included) from one 16-bit counter, so it wraps every
    /// 65536 messages. A reliable ordered RPC whose number equals that of an RPC received one wrap
    /// earlier must still be delivered: one dropped as a "duplicate" leaves a hole in its ordering
    /// channel and holds back every later RPC for good (the 2-hour derby soak lost all RPCs after
    /// ~173000 messages that way). Here: 3 wraps of unreliable syncs with an RPC at irregular numbers
    /// (about 1 in 40, from a fixed xorshift), so the numbers of later RPCs meet those of earlier ones.
    #[test]
    fn message_numbers_wrap_without_false_duplicates() {
        let (_, mut b, t) = pair();
        let mut index = 0u16;
        let mut delivered = 0u32;
        let mut sent = 0u32;
        let mut x = 0x9e37_79b9u32;
        for i in 0..(3 * 65536u32 + 500) {
            let number = i as u16;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let rpc = x.is_multiple_of(40);
            let msg = if rpc {
                index = index.wrapping_add(1);
                sent += 1;
                Message {
                    number,
                    reliability: Reliability::ReliableOrdered,
                    ordering: Some(Ordering { channel: 0, index: index - 1 }),
                    split: None,
                    bit_len: 8,
                    data: vec![0x14],
                }
            } else {
                Message {
                    number,
                    reliability: Reliability::Unreliable,
                    ordering: None,
                    split: None,
                    bit_len: 8,
                    data: vec![0xcf],
                }
            };
            let d = Datagram { acks: vec![], messages: vec![msg] };
            for got in b.handle_datagram(&d.encode(), t).unwrap() {
                if got.data[0] == 0x14 {
                    delivered += 1;
                }
            }
        }
        assert_eq!(delivered, sent);
        assert!(b.recv_slots.is_empty(), "in-order numbers leave no slots");
    }

    /// Window rule: a gap opens holes; a late message fills its hole once, its duplicate is dropped;
    /// a hole older than the timeout is given up, and a message for it is then dropped too (behind
    /// the base); a number more than 0x8000 behind the base is a duplicate.
    #[test]
    fn receive_window_holes() {
        let (_, mut b, t) = pair();
        let ms = |n| t + Duration::from_millis(n);
        let m = |n: u16| {
            Datagram {
                acks: vec![],
                messages: vec![Message {
                    number: n,
                    reliability: Reliability::Reliable,
                    ordering: None,
                    split: None,
                    bit_len: 8,
                    data: vec![n as u8],
                }],
            }
            .encode()
        };
        assert_eq!(b.handle_datagram(&m(0), t).unwrap().len(), 1);
        assert_eq!(b.handle_datagram(&m(3), t).unwrap().len(), 1, "1 and 2 become holes");
        assert_eq!(b.handle_datagram(&m(2), ms(10)).unwrap().len(), 1, "late, fills its hole");
        assert!(b.handle_datagram(&m(2), ms(20)).unwrap().is_empty(), "duplicate");
        assert!(b.handle_datagram(&m(3), ms(20)).unwrap().is_empty(), "duplicate");
        // Hole 1 expires after the timeout; the next message drains the window past it.
        assert_eq!(b.handle_datagram(&m(4), ms(10001)).unwrap().len(), 1);
        assert_eq!(b.recv_base, 5);
        assert!(b.handle_datagram(&m(1), ms(10002)).unwrap().is_empty(), "behind the base");
        assert!(
            b.handle_datagram(&m(5u16.wrapping_sub(0x7fff)), ms(10003)).unwrap().is_empty(),
            "0x8001 ahead = behind"
        );
        assert_eq!(b.handle_datagram(&m(5 + 0x8000), ms(10004)).unwrap().len(), 1, "0x8000 ahead is new");
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

    /// Outgoing splits (ordered): pieces of MTU − 28 − ⌈(4+53+97)/8⌉ = 528 bytes, one per datagram,
    /// consecutive numbers, one ordering index; the peer reassembles the message.
    #[test]
    fn split_send_roundtrip() {
        let (mut a, mut b, t) = pair();
        let data: Vec<u8> = (0..1200u32).map(|i| (i * 7) as u8).collect();
        a.send(&data, data.len() * 8 - 3, Reliability::ReliableOrdered, 2);
        let dgs = a.poll(t);
        let decoded = decode_all(&dgs);
        assert_eq!(decoded.len(), 3);
        let pieces: Vec<_> = decoded.iter().flat_map(|d| d.messages.iter()).collect();
        assert_eq!(pieces.iter().map(|m| m.data.len()).collect::<Vec<_>>(), vec![528, 528, 144]);
        assert_eq!(pieces.iter().map(|m| m.number).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert!(pieces.iter().all(|m| m.ordering == Some(Ordering { channel: 2, index: 0 })));
        assert!(dgs.iter().all(|d| d.len() <= MTU - UDP_HEADER));
        let mut got = Vec::new();
        for d in &dgs {
            got.extend(b.handle_datagram(d, t).unwrap());
        }
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].bit_len, data.len() * 8 - 3);
        assert_eq!(got[0].data[..1199], data[..1199]);
    }

    /// Packing: as many messages as fit (estimate within (576 − 28) × 8 bits), then a new datagram.
    #[test]
    fn packing_by_mtu() {
        let (mut a, _, t) = pair();
        for _ in 0..12 {
            a.send_bytes(&[0xd0; 70], Reliability::UnreliableSequenced, 1);
        }
        let dgs = decode_all(&a.poll(t));
        // Each message: 74 + 560 = 634 bits → 6 per datagram (3804 ≤ 4384 < 4438).
        assert_eq!(dgs.iter().map(|d| d.messages.len()).collect::<Vec<_>>(), vec![6, 6]);
    }

    /// Priority order inside a datagram: system before high before low; FIFO inside one.
    #[test]
    fn priority_order() {
        let (mut a, _, t) = pair();
        a.send_with(&[0x17], 8, Reliability::Reliable, 0, priority::LOW);
        a.send_bytes(&[0xcf], Reliability::UnreliableSequenced, 1);
        a.send_with(&[0x09, 0, 0, 0, 0, 0, 0, 0, 0], 72, Reliability::Unreliable, 0, priority::SYSTEM);
        a.send_bytes(&[0xcd], Reliability::Unreliable, 0);
        let dg = &decode_all(&a.poll(t))[0];
        let firsts: Vec<u8> = dg.messages.iter().map(|m| m.data[0]).collect();
        assert_eq!(firsts, vec![0x09, 0xcf, 0xcd, 0x17]);
    }

    /// Acks go out in the update after the datagram arrived, alone when nothing else is queued.
    #[test]
    fn standalone_ack() {
        let (mut a, mut b, t) = pair();
        a.send_bytes(&[0x14, 1], Reliability::ReliableOrdered, 2);
        pump(&mut a, &mut b, t);
        let dg = &decode_all(&b.poll(t))[0];
        assert_eq!(dg.acks, vec![AckRange { min: 0, max: 0 }]);
        assert!(dg.messages.is_empty());
    }

    /// Pings are answered with the update after the one that follows the receipt.
    #[test]
    fn pong_goes_out_with_the_next_update() {
        let (mut a, mut b, t) = pair();
        a.ping(t);
        pump(&mut a, &mut b, t);
        assert!(b.poll(t).is_empty(), "nothing to send yet (the ping was unreliable)");
        let dg = &decode_all(&b.poll(t))[0];
        assert_eq!(dg.messages[0].data[0], id::CONNECTED_PONG);
    }

    /// The update cycle pings once more after connecting and stops once a pong was accepted.
    #[test]
    fn update_cycle_ping_only_until_a_pong() {
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut a = Connection::connecting(t);
        let mut b = Connection::new(t);
        let count_pings = |dgs: &[Vec<u8>]| {
            decode_all(dgs).iter().flat_map(|d| d.messages.clone()).filter(|m| m.data[0] == id::INTERNAL_PING).count()
        };
        a.mark_connected(t);
        a.ping(t);
        assert_eq!(count_pings(&a.poll(t)), 1, "the application's ping");
        assert_eq!(count_pings(&a.poll(ms(2))), 1, "the cycle's ping, queued after the first update");
        assert_eq!(count_pings(&a.poll(ms(4))), 0);
        assert_eq!(count_pings(&a.poll(ms(5000))), 0);
        assert_eq!(count_pings(&a.poll(ms(5003))), 1, "5 s later while no pong arrived");
        // A pong is accepted: no more pings.
        let mut pong = vec![id::CONNECTED_PONG];
        pong.extend_from_slice(&a.time_ms(ms(5003)).to_le_bytes());
        pong.extend_from_slice(&0u32.to_le_bytes());
        b.send_with(&pong, 72, Reliability::Unreliable, 0, priority::SYSTEM);
        for d in b.poll(ms(5010)) {
            a.handle_datagram(&d, ms(5010)).unwrap();
        }
        for n in [10_010, 15_010, 30_000] {
            a.poll(ms(n));
            assert_eq!(count_pings(&a.poll(ms(n + 2))), 0);
        }
    }

    /// Keep-alive after > 5 s without a reliable send, then 15 s apart while idle (A 0x10041853),
    /// only while nothing awaits an ack.
    #[test]
    fn keep_alive_cadence() {
        let (mut a, mut b, t) = pair();
        let ms = |n| t + Duration::from_millis(n);
        let keep_alives = |dgs: Vec<Vec<u8>>| {
            decode_all(&dgs).iter().flat_map(|d| d.messages.clone()).filter(|m| m.data == [id::KEEP_ALIVE]).count()
        };
        let step = |a: &mut Connection, b: &mut Connection, n| {
            let out = a.poll(ms(n));
            for d in &out {
                b.handle_datagram(d, ms(n)).unwrap();
            }
            for d in b.poll(ms(n)) {
                a.handle_datagram(&d, ms(n)).unwrap();
            }
            keep_alives(out)
        };
        a.send_bytes(&[1], Reliability::Reliable, 0);
        assert_eq!(step(&mut a, &mut b, 0), 0);
        assert_eq!(step(&mut a, &mut b, 5000), 0, "exactly 5000 ms is not enough");
        assert_eq!(step(&mut a, &mut b, 5001), 1);
        assert_eq!(step(&mut a, &mut b, 20_001), 0);
        assert_eq!(step(&mut a, &mut b, 20_002), 1, "the next keep-alive 15 s after the previous");
        // An application send resets the base to now.
        a.send_bytes(&[2], Reliability::ReliableOrdered, 0);
        assert_eq!(step(&mut a, &mut b, 21_000), 0);
        assert_eq!(step(&mut a, &mut b, 26_000), 0);
        assert_eq!(step(&mut a, &mut b, 26_001), 1);
        // Nothing while a reliable message awaits its ack.
        a.send_bytes(&[3], Reliability::Reliable, 0);
        let _lost = a.poll(ms(27_000));
        assert_eq!(keep_alives(a.poll(ms(40_000))), 0);
        // A connection that is not connected yet sends none.
        let mut c = Connection::connecting(t);
        assert_eq!(keep_alives(c.poll(ms(60_000))), 0);
    }

    /// The 5 s rule runs on whole RakNet milliseconds: a send at 0.9 ms is "ms 0"; the
    /// keep-alive is due at the first update in ms 5001 (5000.1 ms later), not at 5000.95 ms.
    #[test]
    fn keep_alive_on_whole_milliseconds() {
        let (mut a, _, t) = pair();
        let us = |n| t + Duration::from_micros(n);
        let ka = |dgs: Vec<Vec<u8>>| {
            decode_all(&dgs).iter().flat_map(|d| d.messages.clone()).any(|m| m.data == [id::KEEP_ALIVE])
        };
        a.send_bytes(&[1], Reliability::Unreliable, 0);
        a.send_bytes(&[2], Reliability::ReliableOrdered, 0);
        a.poll(us(900));
        a.outgoing.clear(); // acked
        assert!(!ka(a.poll(us(5_000_950))));
        assert!(ka(a.poll(us(5_001_000))));
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
        b.poll(t2);
        assert!(pump(&mut b, &mut a, t2).is_empty(), "pong is internal");
        assert_eq!(a.rtt, Duration::from_millis(40), "rtt sample applied");
    }
}
