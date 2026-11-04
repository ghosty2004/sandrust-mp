//! Minimal pcap / pcapng reader yielding IPv4 UDP datagrams.
//!
//! Supported link types: NULL/loopback (0), Ethernet (1), raw IPv4 (101 and
//! 228), Linux cooked v1 (113) and v2 (276). Enough for loopback captures on
//! Linux (dumpcap/tcpdump) and Windows (Npcap loopback adapter).

use std::net::{Ipv4Addr, SocketAddrV4};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UdpPacket {
    /// Capture timestamp in microseconds.
    pub ts_us: u64,
    pub src: SocketAddrV4,
    pub dst: SocketAddrV4,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}

fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    big_endian: bool,
}

impl<'a> Cursor<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let Some(end) = end else { return err("truncated capture file") };
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, Error> {
        let b: [u8; 2] = self.bytes(2)?.try_into().unwrap();
        Ok(if self.big_endian { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
    }
    fn u32(&mut self) -> Result<u32, Error> {
        let b: [u8; 4] = self.bytes(4)?.try_into().unwrap();
        Ok(if self.big_endian { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    }
}

/// Parse a whole capture file.
pub fn read(data: &[u8]) -> Result<Vec<UdpPacket>, Error> {
    match data.get(..4) {
        Some([0xd4, 0xc3, 0xb2, 0xa1]) | Some([0xa1, 0xb2, 0xc3, 0xd4]) => read_pcap(data, false),
        Some([0x4d, 0x3c, 0xb2, 0xa1]) | Some([0xa1, 0xb2, 0x3c, 0x4d]) => read_pcap(data, true),
        Some([0x0a, 0x0d, 0x0d, 0x0a]) => read_pcapng(data),
        _ => err("not a pcap or pcapng file"),
    }
}

fn read_pcap(data: &[u8], nanos: bool) -> Result<Vec<UdpPacket>, Error> {
    let big_endian = data[0] == 0xa1;
    let mut c = Cursor { data, pos: 4, big_endian };
    c.bytes(16)?; // version, thiszone, sigfigs, snaplen
    let linktype = c.u32()? & 0x0fff_ffff;
    let mut out = Vec::new();
    while c.pos < data.len() {
        let (sec, frac, incl) = (c.u32()?, c.u32()?, c.u32()?);
        c.u32()?; // original length
        let frame = c.bytes(incl as usize)?;
        let ts_us = u64::from(sec) * 1_000_000 + if nanos { u64::from(frac) / 1000 } else { u64::from(frac) };
        if let Some(p) = parse_frame(linktype, frame, ts_us) {
            out.push(p);
        }
    }
    Ok(out)
}

fn read_pcapng(data: &[u8]) -> Result<Vec<UdpPacket>, Error> {
    let mut out = Vec::new();
    let mut c = Cursor { data, pos: 0, big_endian: false };
    // Interface table: (link type, timestamp units per second).
    let mut ifaces: Vec<(u32, u64)> = Vec::new();
    while c.pos + 12 <= data.len() {
        let start = c.pos;
        if data[start..start + 4] == [0x0a, 0x0d, 0x0d, 0x0a] {
            // Section header: byte order magic decides endianness.
            c.big_endian = data.get(start + 8..start + 12) == Some(&[0x1a, 0x2b, 0x3c, 0x4d]);
            ifaces.clear();
        }
        let block_type = c.u32()?;
        let len = c.u32()? as usize;
        if len < 12 || start + len > data.len() {
            return err("corrupt pcapng block");
        }
        let body = &data[start + 8..start + len - 4];
        let mut b = Cursor { data: body, pos: 0, big_endian: c.big_endian };
        match block_type {
            1 => {
                let linktype = u32::from(b.u16()?);
                b.u16()?; // reserved
                b.u32()?; // snaplen
                let mut units = 1_000_000u64;
                // Options: look for if_tsresol (code 9).
                while b.pos + 4 <= body.len() {
                    let (code, olen) = (b.u16()?, b.u16()? as usize);
                    if code == 0 {
                        break;
                    }
                    let v = b.bytes(olen)?;
                    b.bytes((4 - olen % 4) % 4)?;
                    if code == 9 && olen >= 1 {
                        let r = v[0];
                        units = if r & 0x80 == 0 { 10u64.pow(u32::from(r)) } else { 1u64 << (r & 0x7f) };
                    }
                }
                ifaces.push((linktype, units));
            }
            6 => {
                let iface = b.u32()? as usize;
                let ts = (u64::from(b.u32()?) << 32) | u64::from(b.u32()?);
                let incl = b.u32()? as usize;
                b.u32()?; // original length
                let frame = b.bytes(incl)?;
                let Some(&(linktype, units)) = ifaces.get(iface) else { return err("packet for unknown interface") };
                let ts_us = (u128::from(ts) * 1_000_000 / u128::from(units)) as u64;
                if let Some(p) = parse_frame(linktype, frame, ts_us) {
                    out.push(p);
                }
            }
            _ => {}
        }
        c.pos = start + len;
    }
    Ok(out)
}

fn parse_frame(linktype: u32, frame: &[u8], ts_us: u64) -> Option<UdpPacket> {
    let ip = match linktype {
        0 => frame.get(4..)?, // BSD/Npcap loopback: 4-byte address family
        1 => {
            let ethertype = u16::from_be_bytes([*frame.get(12)?, *frame.get(13)?]);
            if ethertype != 0x0800 {
                return None;
            }
            frame.get(14..)?
        }
        101 | 228 => frame,
        113 => frame.get(16..)?,
        276 => frame.get(20..)?,
        _ => return None,
    };
    if ip.first()? >> 4 != 4 || *ip.get(9)? != 17 {
        return None;
    }
    let ihl = usize::from(ip[0] & 0x0f) * 4;
    let total = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
    let src_ip = Ipv4Addr::new(ip[12], ip[13], ip[14], ip[15]);
    let dst_ip = Ipv4Addr::new(ip[16], ip[17], ip[18], ip[19]);
    let udp = ip.get(ihl..total.min(ip.len()))?;
    let sport = u16::from_be_bytes([*udp.first()?, *udp.get(1)?]);
    let dport = u16::from_be_bytes([*udp.get(2)?, *udp.get(3)?]);
    Some(UdpPacket {
        ts_us,
        src: SocketAddrV4::new(src_ip, sport),
        dst: SocketAddrV4::new(dst_ip, dport),
        payload: udp.get(8..)?.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ipv4_udp(sport: u16, dport: u16, payload: &[u8]) -> Vec<u8> {
        let total = (20 + 8 + payload.len()) as u16;
        let mut p = vec![0x45, 0, 0, 0, 0, 0, 0, 0, 64, 17, 0, 0, 127, 0, 0, 1, 127, 0, 0, 1];
        p[2..4].copy_from_slice(&total.to_be_bytes());
        p.extend_from_slice(&sport.to_be_bytes());
        p.extend_from_slice(&dport.to_be_bytes());
        p.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
        p.extend_from_slice(&[0, 0]);
        p.extend_from_slice(payload);
        p
    }

    #[test]
    fn classic_pcap_ethernet() {
        let mut frame = vec![0u8; 12];
        frame.extend_from_slice(&[0x08, 0x00]);
        frame.extend(ipv4_udp(5000, 7777, b"\x08\x1e\x77\xda"));
        let mut f = vec![0xd4, 0xc3, 0xb2, 0xa1, 2, 0, 4, 0];
        f.extend_from_slice(&[0; 8]);
        f.extend_from_slice(&65535u32.to_le_bytes());
        f.extend_from_slice(&1u32.to_le_bytes());
        f.extend_from_slice(&3u32.to_le_bytes());
        f.extend_from_slice(&7u32.to_le_bytes());
        f.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        f.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        f.extend_from_slice(&frame);
        let pkts = read(&f).unwrap();
        assert_eq!(pkts.len(), 1);
        assert_eq!(pkts[0].ts_us, 3_000_007);
        assert_eq!(pkts[0].dst.port(), 7777);
        assert_eq!(pkts[0].payload, b"\x08\x1e\x77\xda");
    }

    #[test]
    fn pcapng_null_linktype() {
        let block = |t: u32, body: &[u8]| {
            let len = (12 + body.len()) as u32;
            let mut b = t.to_le_bytes().to_vec();
            b.extend_from_slice(&len.to_le_bytes());
            b.extend_from_slice(body);
            b.extend_from_slice(&len.to_le_bytes());
            b
        };
        let mut shb = 0x1a2b3c4du32.to_le_bytes().to_vec();
        shb.extend_from_slice(&[1, 0, 0, 0]);
        shb.extend_from_slice(&u64::MAX.to_le_bytes());
        let mut f = block(0x0a0d0d0a, &shb);
        let mut idb = 0u16.to_le_bytes().to_vec(); // linktype NULL
        idb.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        f.extend(block(1, &idb));
        let mut frame = 2u32.to_le_bytes().to_vec();
        frame.extend(ipv4_udp(7777, 5000, b"\x19\x00"));
        let mut epb = 0u32.to_le_bytes().to_vec();
        epb.extend_from_slice(&0u32.to_le_bytes());
        epb.extend_from_slice(&5u32.to_le_bytes());
        epb.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        epb.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        epb.extend_from_slice(&frame);
        while !epb.len().is_multiple_of(4) {
            epb.push(0);
        }
        f.extend(block(6, &epb));
        let pkts = read(&f).unwrap();
        assert_eq!(pkts.len(), 1);
        assert_eq!(pkts[0].src.port(), 7777);
        assert_eq!(pkts[0].payload, b"\x19\x00");
    }
}
