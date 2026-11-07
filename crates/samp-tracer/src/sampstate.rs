//! Control queries that read the original client's own data, for the cross-client comparison:
//! `bubbles` (the chat bubbles samp holds for each player) and `synced` (the health/armour samp keeps
//! for each remote player, i.e. what its nametag bars show; the GTA ped of a remote player always
//! keeps 1000/0). samp-client answers the same commands from its own state in the same line format.
//!
//! Only data is read, never code, and only for samp.dll build B (the build of the diff lanes); build A's
//! player-pool layout differs, so it answers an error. Layouts and their sources:
//! docs/re/addresses/samp-tracer.md, "Chat bubbles and synced health".

use crate::efx::Build;
use crate::mem::Mem;

/// Player slots of samp's pools (ids < 1004).
pub const MAX_PLAYERS: u32 = 1004;

/// Build B address of samp's pointer to the chat-bubble pool.
pub const B_BUBBLE_POOL: u32 = 0x1026_e9f0;
/// Build B address of samp's netgame pointer (the pools hang off it).
pub const B_NETGAME: u32 = 0x1026_ea0c;
/// Size of one bubble slot; field offsets in `bubble`.
pub const BUBBLE_SLOT: u32 = 0x118;
/// Offsets inside one bubble slot.
pub mod bubble {
    pub const ACTIVE: u32 = 0x0;
    pub const TEXT: u32 = 0x4;
    pub const TEXT_MAX: usize = 0x100;
    pub const DURATION: u32 = 0x108;
    pub const COLOUR: u32 = 0x10c;
    pub const DISTANCE: u32 = 0x110;
    pub const LINES: u32 = 0x114;
}
/// Offsets along the netgame → player pool → remote player chain (build B).
pub mod pool {
    pub const NETGAME_POOLS: u32 = 0x3de;
    pub const POOLS_PLAYER: u32 = 0x8;
    pub const SLOT_USED: u32 = 0xfde;
    pub const ENTRY: u32 = 0x2e;
    pub const ENTRY_REMOTE: u32 = 0x10;
    pub const REMOTE_HEALTH: u32 = 0x1b0;
    pub const REMOTE_ARMOUR: u32 = 0x1ac;
}

/// Quoted text, the same escaping as samp-client's control replies (`ui::core::quote`).
pub fn quote(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() + 2);
    s.push('"');
    for &c in b {
        match c {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            0x20..=0x7E => s.push(c as char),
            _ => s.push_str(&format!("\\x{c:02x}")),
        }
    }
    s.push('"');
    s
}

/// One `bubble …` line (shared format with samp-client).
pub fn bubble_line(id: u32, colour: u32, distance: f32, duration_ms: u32, lines: u32, text: &[u8]) -> String {
    format!(
        "bubble id={id} colour={colour:#010x} distance={distance:.2} duration={duration_ms} lines={lines} text={}",
        quote(text)
    )
}

/// One `synced …` line (shared format with samp-client).
pub fn synced_line(id: u32, health: f32, armour: f32) -> String {
    format!("synced id={id} health={health:.1} armour={armour:.1}")
}

fn rebase(base: u32, va: u32) -> u32 {
    base.wrapping_add(va.wrapping_sub(crate::efx::SAMP_IMAGE_BASE))
}

/// `bubbles` reply: header + one line per active bubble (ascending player id).
pub fn format_bubbles<M: Mem>(m: &M, samp_base: u32, build: Build) -> String {
    if build != Build::B {
        return "error bubbles: only samp.dll build B is mapped".into();
    }
    let Some(pool) = m.u32(rebase(samp_base, B_BUBBLE_POOL)).filter(|&p| p != 0) else {
        return "bubbles count=0 pool=none".into();
    };
    let mut lines = Vec::new();
    for id in 0..MAX_PLAYERS {
        let s = pool + id * BUBBLE_SLOT;
        if m.u32(s + bubble::ACTIVE).unwrap_or(0) == 0 {
            continue;
        }
        let mut text = vec![0u8; bubble::TEXT_MAX];
        if !m.read(s + bubble::TEXT, &mut text) {
            continue;
        }
        let n = text.iter().position(|&b| b == 0).unwrap_or(text.len());
        text.truncate(n);
        lines.push(bubble_line(
            id,
            m.u32(s + bubble::COLOUR).unwrap_or(0),
            m.f32(s + bubble::DISTANCE).unwrap_or(0.0),
            m.u32(s + bubble::DURATION).unwrap_or(0),
            m.u32(s + bubble::LINES).unwrap_or(0),
            &text,
        ));
    }
    let mut out = format!("bubbles count={}", lines.len());
    for l in lines {
        out.push('\n');
        out.push_str(&l);
    }
    out
}

/// `synced` reply: header + one line per remote player samp holds (ascending id).
pub fn format_synced<M: Mem>(m: &M, samp_base: u32, build: Build) -> String {
    if build != Build::B {
        return "error synced: only samp.dll build B is mapped".into();
    }
    let player_pool = m
        .u32(rebase(samp_base, B_NETGAME))
        .filter(|&n| n != 0)
        .and_then(|n| m.u32(n + pool::NETGAME_POOLS))
        .filter(|&p| p != 0)
        .and_then(|p| m.u32(p + pool::POOLS_PLAYER))
        .filter(|&p| p != 0);
    let Some(pp) = player_pool else { return "synced count=0 pool=none".into() };
    let mut lines = Vec::new();
    for id in 0..MAX_PLAYERS {
        if m.u32(pp + pool::SLOT_USED + 4 * id).unwrap_or(0) == 0 {
            continue;
        }
        let Some(remote) = m
            .u32(pp + pool::ENTRY + 4 * id)
            .filter(|&e| e != 0)
            .and_then(|e| m.u32(e + pool::ENTRY_REMOTE))
            .filter(|&r| r != 0)
        else {
            continue;
        };
        let (Some(h), Some(a)) = (m.f32(remote + pool::REMOTE_HEALTH), m.f32(remote + pool::REMOTE_ARMOUR)) else {
            continue;
        };
        lines.push(synced_line(id, h, a));
    }
    let mut out = format!("synced count={}", lines.len());
    for l in lines {
        out.push('\n');
        out.push_str(&l);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::fake::FakeMem;

    const BASE: u32 = 0x1000_0000;

    #[test]
    fn bubbles_from_pool() {
        let mut m = FakeMem::default();
        let pool = 0x0200_0000;
        m.set_u32(B_BUBBLE_POOL, pool);
        // Every slot readable and inactive, slot 3 active.
        m.set(pool, &vec![0u8; (MAX_PLAYERS * BUBBLE_SLOT) as usize]);
        let s = pool + 3 * BUBBLE_SLOT;
        m.set_u32(s, 1);
        m.set(s + bubble::TEXT, b"bubble of A\0");
        m.set_u32(s + bubble::DURATION, 15000);
        m.set_u32(s + bubble::COLOUR, 0xFFFF_AA00);
        m.set_f32(s + bubble::DISTANCE, 50.0);
        m.set_u32(s + bubble::LINES, 1);
        assert_eq!(
            format_bubbles(&m, BASE, Build::B),
            "bubbles count=1\nbubble id=3 colour=0xffffaa00 distance=50.00 duration=15000 lines=1 text=\"bubble of A\""
        );
        assert!(format_bubbles(&m, BASE, Build::A).starts_with("error"));
    }

    #[test]
    fn synced_from_player_pool() {
        let mut m = FakeMem::default();
        let (netgame, pools, pp, entry, remote) =
            (0x0300_0000u32, 0x0310_0000u32, 0x0320_0000u32, 0x0330_0000u32, 0x0340_0000u32);
        m.set_u32(B_NETGAME, netgame);
        m.set_u32(netgame + pool::NETGAME_POOLS, pools);
        m.set_u32(pools + pool::POOLS_PLAYER, pp);
        m.set(pp, &vec![0u8; 0x2000]);
        m.set_u32(pp + pool::SLOT_USED + 4, 1);
        m.set_u32(pp + pool::ENTRY + 4, entry);
        m.set_u32(entry + pool::ENTRY_REMOTE, remote);
        m.set_f32(remote + pool::REMOTE_HEALTH, 54.0);
        m.set_f32(remote + pool::REMOTE_ARMOUR, 0.0);
        assert_eq!(format_synced(&m, BASE, Build::B), "synced count=1\nsynced id=1 health=54.0 armour=0.0");
    }
}
