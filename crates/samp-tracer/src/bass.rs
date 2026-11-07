//! Decoding of the BASS audio library calls a SA:MP client makes (audio streams,
//! RPC 41/42). The original samp.dll imports 17 functions from `bass.dll` (BASS 2.4); the
//! Windows side (`win/audio.rs`) redirects the client module's import-table slots for them to
//! logging wrappers. This module turns the raw stack arguments into record fields, so the
//! formatting is testable on the host.
//!
//! Signatures and option / tag numbers are the public BASS 2.4 API (`bass.h`).

use crate::json::Val;
use crate::mem::Mem;

/// One imported BASS function: name and number of 32-bit stack slots (stdcall).
pub struct BassFn {
    pub name: &'static str,
    pub slots: usize,
}

/// The functions samp.dll 0.3.7-R4 imports from bass.dll (import directory of build B, in IAT order).
pub const BASS_FNS: &[BassFn] = &[
    BassFn { name: "BASS_SetEAXParameters", slots: 4 },
    BassFn { name: "BASS_SetConfig", slots: 2 },
    BassFn { name: "BASS_SetConfigPtr", slots: 2 },
    BassFn { name: "BASS_Init", slots: 5 },
    BassFn { name: "BASS_Free", slots: 0 },
    BassFn { name: "BASS_StreamFree", slots: 1 },
    BassFn { name: "BASS_ChannelGetTags", slots: 2 },
    BassFn { name: "BASS_StreamGetFilePosition", slots: 2 },
    // QWORD param = 2 slots.
    BassFn { name: "BASS_ChannelSetSync", slots: 6 },
    BassFn { name: "BASS_ChannelPlay", slots: 2 },
    BassFn { name: "BASS_StreamCreateURL", slots: 5 },
    BassFn { name: "BASS_ChannelStop", slots: 1 },
    BassFn { name: "BASS_Apply3D", slots: 0 },
    BassFn { name: "BASS_ChannelSet3DPosition", slots: 4 },
    BassFn { name: "BASS_ChannelSet3DAttributes", slots: 7 },
    BassFn { name: "BASS_Set3DPosition", slots: 4 },
    BassFn { name: "BASS_Set3DFactors", slots: 3 },
];

pub fn index_of(name: &str) -> Option<usize> {
    BASS_FNS.iter().position(|f| f.name.eq_ignore_ascii_case(name))
}

/// `BASS_CONFIG_*` names for the options a client is likely to set.
pub fn config_name(opt: u32) -> &'static str {
    match opt {
        0 => "BUFFER",
        1 => "UPDATEPERIOD",
        4 => "GVOL_SAMPLE",
        5 => "GVOL_STREAM",
        6 => "GVOL_MUSIC",
        9 => "FLOATDSP",
        11 => "NET_TIMEOUT",
        12 => "NET_BUFFER",
        15 => "NET_PREBUF",
        16 => "NET_AGENT",
        17 => "NET_PROXY",
        18 => "NET_PASSIVE",
        21 => "NET_PLAYLIST",
        _ => "?",
    }
}

/// `BASS_SYNC_*` type (low bits; MIXTIME/ONETIME flags kept separately).
pub fn sync_name(t: u32) -> &'static str {
    match t & 0x00FF_FFFF {
        0 => "POS",
        2 => "END",
        4 => "META",
        5 => "SLIDE",
        6 => "STALL",
        7 => "DOWNLOAD",
        8 => "FREE",
        12 => "OGG_CHANGE",
        _ => "?",
    }
}

/// `BASS_TAG_*` name.
pub fn tag_name(t: u32) -> &'static str {
    match t {
        0 => "ID3",
        1 => "ID3V2",
        2 => "OGG",
        3 => "HTTP",
        4 => "ICY",
        5 => "META",
        _ => "?",
    }
}

/// Reads a tag block returned by `BASS_ChannelGetTags`: META is one string, OGG/HTTP/ICY are a
/// series of NUL-terminated strings ending with an empty one. At most 16 strings of 256 bytes.
pub fn read_tags(m: &dyn Mem, tags: u32, ptr: u32) -> Val {
    if ptr == 0 {
        return Val::Null;
    }
    match tags {
        2..=4 => {
            let mut out = Vec::new();
            let mut p = ptr;
            for _ in 0..16 {
                let Some(s) = m.cstr(p, 256) else { break };
                if s.is_empty() {
                    break;
                }
                p = p.wrapping_add(s.len() as u32 + 1);
                out.push(Val::Str(s));
            }
            Val::List(out)
        }
        _ => m.cstr(ptr, 512).map(Val::Str).unwrap_or(Val::Null),
    }
}

fn f(v: u32) -> Val {
    Val::Float(f32::from_bits(v))
}

fn vec3(m: &dyn Mem, p: u32) -> Val {
    if p == 0 {
        return Val::Null;
    }
    Val::List((0..3).map(|i| m.f32(p + 4 * i).map(Val::Float).unwrap_or(Val::Null)).collect())
}

/// Record arguments for one call (`a` = the stack slots, `ret` = EDX:EAX).
pub fn render(m: &dyn Mem, idx: usize, a: &[u32], ret: u64) -> Val {
    let name = BASS_FNS.get(idx).map(|f| f.name).unwrap_or("?");
    let g = |i: usize| a.get(i).copied().unwrap_or(0);
    let r32 = ret as u32;
    let v = Val::obj();
    match name {
        "BASS_SetEAXParameters" => v.with("env", g(0) as i32).with("vol", f(g(1))).with("decay", f(g(2))).with("damp", f(g(3))),
        "BASS_SetConfig" => v.with("option", g(0)).with("name", config_name(g(0))).with("value", g(1)).with("ret", r32),
        "BASS_SetConfigPtr" => {
            let s = if g(1) == 0 { Val::Null } else { m.cstr(g(1), 256).map(Val::Str).unwrap_or(Val::Null) };
            v.with("option", g(0)).with("name", config_name(g(0))).with("value", s).with("ret", r32)
        }
        "BASS_Init" => v
            .with("device", g(0) as i32)
            .with("freq", g(1))
            .with("flags", Val::Hex(g(2)))
            .with("win_ptr", Val::Hex(g(3)))
            .with("guid_ptr", Val::Hex(g(4)))
            .with("ret", r32),
        "BASS_Free" | "BASS_Apply3D" => v,
        "BASS_StreamFree" | "BASS_ChannelStop" => v.with("handle_ptr", Val::Hex(g(0))).with("ret", r32),
        "BASS_ChannelGetTags" => v
            .with("handle_ptr", Val::Hex(g(0)))
            .with("tags", g(1))
            .with("tag_name", tag_name(g(1)))
            .with("value", read_tags(m, g(1), r32)),
        "BASS_StreamGetFilePosition" => v.with("handle_ptr", Val::Hex(g(0))).with("mode", g(1)).with("ret", ret),
        "BASS_ChannelSetSync" => v
            .with("handle_ptr", Val::Hex(g(0)))
            .with("type", Val::Hex(g(1)))
            .with("type_name", sync_name(g(1)))
            .with("param", u64::from(g(2)) | (u64::from(g(3)) << 32))
            .with("proc_ptr", Val::Hex(g(4)))
            .with("user_ptr", Val::Hex(g(5)))
            .with("ret_ptr", Val::Hex(r32)),
        "BASS_ChannelPlay" => v.with("handle_ptr", Val::Hex(g(0))).with("restart", g(1)).with("ret", r32),
        "BASS_StreamCreateURL" => v
            .with("url", m.cstr(g(0), 512).map(Val::Str).unwrap_or(Val::Null))
            .with("offset", g(1))
            .with("flags", Val::Hex(g(2)))
            .with("proc_ptr", Val::Hex(g(3)))
            .with("user_ptr", Val::Hex(g(4)))
            .with("ret_ptr", Val::Hex(r32)),
        "BASS_ChannelSet3DPosition" => v
            .with("handle_ptr", Val::Hex(g(0)))
            .with("pos", vec3(m, g(1)))
            .with("orient", vec3(m, g(2)))
            .with("vel", vec3(m, g(3)))
            .with("ret", r32),
        "BASS_ChannelSet3DAttributes" => v
            .with("handle_ptr", Val::Hex(g(0)))
            .with("mode", g(1) as i32)
            .with("min", f(g(2)))
            .with("max", f(g(3)))
            .with("iangle", g(4) as i32)
            .with("oangle", g(5) as i32)
            .with("outvol", f(g(6)))
            .with("ret", r32),
        "BASS_Set3DPosition" => v
            .with("pos", vec3(m, g(0)))
            .with("vel", vec3(m, g(1)))
            .with("front", vec3(m, g(2)))
            .with("top", vec3(m, g(3)))
            .with("ret", r32),
        "BASS_Set3DFactors" => v.with("distf", f(g(0))).with("rollf", f(g(1))).with("doppf", f(g(2))).with("ret", r32),
        _ => v.with("raw", Val::List(a.iter().map(|&x| Val::Hex(x)).collect())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::fake::FakeMem;

    #[test]
    fn table_matches_bass_import_count() {
        assert_eq!(BASS_FNS.len(), 17);
        assert_eq!(index_of("bass_streamcreateurl"), Some(10));
        assert!(BASS_FNS.iter().all(|f| f.slots <= 7));
    }

    #[test]
    fn renders_url_and_config() {
        let mut m = FakeMem::default();
        m.set(0x1000, b"http://127.0.0.1:8000/a.mp3\0");
        let r = render(&m, index_of("BASS_StreamCreateURL").unwrap(), &[0x1000, 0, 0x940000, 0, 0], 0x8000_0001);
        let s = r.to_json();
        assert!(s.contains("\"url\":\"http://127.0.0.1:8000/a.mp3\""), "{s}");
        assert!(s.contains("\"flags\":\"0x940000\""), "{s}");
        let r = render(&m, index_of("BASS_SetConfig").unwrap(), &[5, 7000], 1);
        assert!(r.to_json().contains("\"name\":\"GVOL_STREAM\""));
    }

    #[test]
    fn reads_tag_lists() {
        let mut m = FakeMem::default();
        m.set(0x2000, b"icy-name:Test\0icy-url:x\0\0");
        let v = read_tags(&m, 4, 0x2000);
        assert_eq!(v, Val::List(vec![Val::Str("icy-name:Test".into()), Val::Str("icy-url:x".into())]));
        m.set(0x3000, b"StreamTitle='A - B';\0");
        assert_eq!(read_tags(&m, 5, 0x3000), Val::Str("StreamTitle='A - B';".into()));
        assert_eq!(read_tags(&m, 5, 0), Val::Null);
    }
}
