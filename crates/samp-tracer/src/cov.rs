//! First-hit coverage: which gta_sa.exe functions execute, per named time window.
//! The Windows side (`win::cov`) writes a one-shot `int3` over the first byte of every function in an
//! arm list (from `tools/cov/inventory.sh`); a vectored exception handler records the first hit
//! (frame, time, thread, return address), restores the byte and resumes. `cov start NAME` closes the
//! current window and re-arms; `cov dump PATH` writes every window seen so far.
//! This module holds the host-testable parts: arm-list parser, arming rules and dump format.
//! Background: docs/phase3/coverage-notes.md.

/// One armable function entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmEntry {
    pub addr: u32,
    /// The entry's first byte in the unmodified exe.
    pub orig: u8,
}

/// Parses `0xADDR BYTE` lines (`#` comments, blank lines ignored); sorted, unique by address.
pub fn parse_list(text: &str) -> Vec<ArmEntry> {
    let mut v: Vec<ArmEntry> = text
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            let mut it = l.split_whitespace();
            let a = it.next()?;
            let a = u32::from_str_radix(a.strip_prefix("0x").unwrap_or(a), 16).ok()?;
            let b = u8::from_str_radix(it.next()?, 16).ok()?;
            Some(ArmEntry { addr: a, orig: b })
        })
        .collect();
    v.sort_by_key(|e| e.addr);
    v.dedup_by_key(|e| e.addr);
    v
}

/// Why an entry was not armed in a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Inside the stolen prologue of one of the tracer's own hooks.
    TracerHook,
    /// The live byte is neither the exe's byte nor our `int3` (a client patched it).
    Patched(u8),
}

/// `int3`.
pub const INT3: u8 = 0xCC;

/// Arming decision for one entry given its live byte and the tracer's hook ranges
/// (`[start, end)`): `Ok(true)` = write `int3`, `Ok(false)` = already armed.
pub fn arm_decision(e: ArmEntry, live: u8, hook_ranges: &[(u32, u32)]) -> Result<bool, Skip> {
    if hook_ranges.iter().any(|&(a, b)| (a..b).contains(&e.addr)) {
        return Err(Skip::TracerHook);
    }
    if live == e.orig {
        Ok(true)
    } else if live == INT3 {
        Ok(false)
    } else {
        Err(Skip::Patched(live))
    }
}

/// First hit of one function in one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub addr: u32,
    pub frame: u32,
    pub t_ms: u32,
    pub tid: u32,
    /// Return address on the stack at the entry (the caller).
    pub ret: u32,
}

/// A closed (or the current) window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Window {
    pub name: String,
    pub start_frame: u32,
    pub start_ms: u64,
    pub armed: u32,
    pub skipped: Vec<(u32, Skip)>,
    pub hits: Vec<Hit>,
}

/// Dump text: a header line per window, `skip` lines and one `hit` line per first hit.
/// `caller` formats a return address (module+offset).
pub fn format_dump(windows: &[Window], caller: impl Fn(u32) -> String) -> String {
    let mut s = String::from("# samp-tracer coverage dump v1\n");
    for w in windows {
        s.push_str(&format!(
            "window\t{}\tstart_frame={}\tstart_ms={}\tarmed={}\tskipped={}\thits={}\n",
            w.name,
            w.start_frame,
            w.start_ms,
            w.armed,
            w.skipped.len(),
            w.hits.len()
        ));
        for (a, why) in &w.skipped {
            let why = match why {
                Skip::TracerHook => "tracer_hook".to_string(),
                Skip::Patched(b) => format!("patched:{b:02x}"),
            };
            s.push_str(&format!("skip\t{}\t{a:#010x}\t{why}\n", w.name));
        }
        let mut hits = w.hits.clone();
        hits.sort_by_key(|h| (h.frame, h.t_ms, h.addr));
        for h in hits {
            s.push_str(&format!(
                "hit\t{}\t{:#010x}\t{}\t{}\t{}\t{}\n",
                w.name,
                h.addr,
                h.frame,
                h.t_ms,
                h.tid,
                caller(h.ret)
            ));
        }
    }
    s
}

/// Window names become a field of a TSV line: keep them to one printable token.
pub fn clean_name(n: &str) -> String {
    let s: String = n.trim().chars().map(|c| if c.is_ascii_graphic() { c } else { '_' }).take(64).collect();
    if s.is_empty() { "unnamed".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_arm_list() {
        let v = parse_list("# hdr\n0x00401025 9c\n\n0x00400010 e9\n0x00401025 9c\nbad\n0x5 zz\n");
        assert_eq!(v, vec![ArmEntry { addr: 0x400010, orig: 0xE9 }, ArmEntry { addr: 0x401025, orig: 0x9C }]);
    }

    #[test]
    fn arming_rules() {
        let e = ArmEntry { addr: 0x500000, orig: 0x55 };
        assert_eq!(arm_decision(e, 0x55, &[]), Ok(true));
        assert_eq!(arm_decision(e, INT3, &[]), Ok(false));
        assert_eq!(arm_decision(e, 0xE9, &[]), Err(Skip::Patched(0xE9)));
        assert_eq!(arm_decision(e, 0x55, &[(0x4FFFFE, 0x500003)]), Err(Skip::TracerHook));
        assert_eq!(arm_decision(e, 0x55, &[(0x500001, 0x500006)]), Ok(true));
    }

    #[test]
    fn dump_format() {
        let w = Window {
            name: clean_name("grove idle"),
            start_frame: 10,
            start_ms: 1234,
            armed: 2,
            skipped: vec![(0x53E230, Skip::Patched(0xE9))],
            hits: vec![
                Hit { addr: 0x500000, frame: 12, t_ms: 1300, tid: 7, ret: 0x53EB17 },
                Hit { addr: 0x400000, frame: 11, t_ms: 1250, tid: 7, ret: 0x1000_0010 },
            ],
        };
        let d = format_dump(&[w], |r| format!("{r:#x}"));
        assert_eq!(
            d,
            "# samp-tracer coverage dump v1\n\
             window\tgrove_idle\tstart_frame=10\tstart_ms=1234\tarmed=2\tskipped=1\thits=2\n\
             skip\tgrove_idle\t0x0053e230\tpatched:e9\n\
             hit\tgrove_idle\t0x00400000\t11\t1250\t7\t0x10000010\n\
             hit\tgrove_idle\t0x00500000\t12\t1300\t7\t0x53eb17\n"
        );
        assert_eq!(clean_name("  "), "unnamed");
    }
}
