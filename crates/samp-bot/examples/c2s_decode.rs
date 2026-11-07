//! Helper for comparing captures: decodes the client→server lines of a `samp-dump --raw` log.
//!
//! samp-dump prints C→S RPC payloads and sync packets as hex; this reads such a log on stdin
//! and appends the decoded payload (RPCs through `rpcs::decode_outgoing`, sync packets
//! through their `SyncPacket` readers). Server→client lines pass through unchanged unless
//! `--c2s` is given (then only C→S lines are printed). `--only 206,115,...` keeps C→S lines of
//! those ids.
//!
//! Usage: samp-dump CAPTURE --port 7790 --raw | c2s_decode [--c2s] [--only IDS]

use std::io::{BufRead, Write};

use samp_proto::bitstream::BitReader;
use samp_proto::rpcs;
use samp_proto::sync::{
    AimSync, BulletSync, FootSync, PassengerSync, SpectatorSync, StatsSync, SyncPacket, TrailerSync, UnoccupiedSync,
    VehicleSync, WeaponsUpdate,
};

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn sync_text(id: u8, body: &[u8]) -> Option<String> {
    let mut r = BitReader::new(body);
    Some(match id {
        200 => format!("{:?}", VehicleSync::read(&mut r)?),
        203 => format!("{:?}", AimSync::read(&mut r)?),
        204 => format!("{:?}", WeaponsUpdate::read(&mut r)?),
        205 => format!("{:?}", StatsSync::read(&mut r)?),
        206 => format!("{:?}", BulletSync::read(&mut r)?),
        207 => format!("{:?}", FootSync::read(&mut r)?),
        209 => format!("{:?}", UnoccupiedSync::read(&mut r)?),
        210 => format!("{:?}", TrailerSync::read(&mut r)?),
        211 => format!("{:?}", PassengerSync::read(&mut r)?),
        212 => format!("{:?}", SpectatorSync::read(&mut r)?),
        _ => return None,
    })
}

/// Returns (id, decoded) for a C→S rpc / sync line.
fn decode(line: &str) -> Option<(u8, String)> {
    let toks: Vec<&str> = line.split_whitespace().collect();
    if let Some(i) = toks.iter().position(|&t| t == "rpc") {
        let id: u8 = toks.get(i + 1)?.parse().ok()?;
        // "rpc ID N bits HEX"
        let hex = if toks.get(i + 3) == Some(&"bits") { toks.get(i + 4).copied().unwrap_or("") } else { "" };
        let bytes = unhex(hex)?;
        let mut r = BitReader::new(&bytes);
        let d = rpcs::decode_outgoing(id, &mut r).map(|o| format!("{o:?}")).unwrap_or_else(|| "??".into());
        return Some((id, d));
    }
    if let Some(i) = toks.iter().position(|&t| t == "sync") {
        let id: u8 = toks.get(i + 1)?.parse().ok()?;
        // "sync ID N bytes HEX"
        let bytes = unhex(toks.get(i + 4).copied().unwrap_or(""))?;
        return Some((id, sync_text(id, &bytes).unwrap_or_else(|| "??".into())));
    }
    None
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let c2s_only = args.iter().any(|a| a == "--c2s");
    let only: Option<Vec<u8>> = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect());
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let c2s = line.contains("C>S");
        if !c2s {
            if !c2s_only && only.is_none() {
                let _ = writeln!(out, "{line}");
            }
            continue;
        }
        match decode(&line) {
            Some((id, d)) => {
                if only.as_ref().is_some_and(|o| !o.contains(&id)) {
                    continue;
                }
                // Keep the time / transport prefix, drop the hex.
                let head: String = line.split(" bits ").next().unwrap_or(&line).to_string();
                let head = head.split(" bytes ").next().unwrap_or(&head).to_string();
                let _ = writeln!(out, "{head} {d}");
            }
            None => {
                if only.is_none() {
                    let _ = writeln!(out, "{line}");
                }
            }
        }
    }
}
