//! Direct3D9 / D3DX call-stream logger: the platform-independent part (TRACE-3).
//!
//! The Windows side (`win::d3d`) patches the vtables of the objects the game really draws
//! with: the **real** `IDirect3DDevice9` (reached through the swap chain of whatever device
//! pointer sits in `_RwD3DDevice` 0xC97C28, so a client's proxy in that slot is bypassed),
//! the D3DX font, sprite, render-to-surface and vertex-buffer classes, and inline-hooks the
//! D3DX factory exports (`D3DXCreateFont*`, `D3DXCreateSprite`, texture loaders, …) of every
//! loaded `d3dx9_NN.dll`. This module holds the method tables, argument rendering, vertex
//! decoding and the `d3d` control command, so they can be tested on the host.
//!
//! Records go to `d3d.jsonl` next to the trace. Factory calls, cursor calls, resets and
//! state-block recordings are always written ("always" methods); everything else only
//! while logging is switched on through the control channel (`d3d ui|all|text [FRAMES]`,
//! `d3d off`), for whole frames (Present to Present).

use crate::json::Val;
use crate::mem::Mem;

/// COM interface a vtable slot belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Iface {
    Device,
    Sprite,
    Font,
    VertexBuffer,
    RenderToSurface,
}

impl Iface {
    pub fn prefix(self) -> &'static str {
        match self {
            Iface::Device => "dev",
            Iface::Sprite => "sprite",
            Iface::Font => "font",
            Iface::VertexBuffer => "vb",
            Iface::RenderToSurface => "rts",
        }
    }
}

/// How a stack argument is rendered (slots count from the first argument after `this`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum A {
    /// Unsigned 32-bit value.
    U(&'static str),
    /// Signed 32-bit value.
    I(&'static str),
    /// Value written as hex (flags, colours, formats).
    X(&'static str),
    F(&'static str),
    /// COM object pointer: labelled by the runtime (texture size/name, font face, …).
    Obj(&'static str),
    /// Pointer to a RECT (4 × i32) or NULL.
    Rect(&'static str),
    /// Pointer to 3 floats or NULL.
    V3(&'static str),
    /// Pointer to a 4×4 float matrix or NULL.
    Mat(&'static str),
    /// Pointer to a D3DVIEWPORT9.
    Viewport(&'static str),
    /// Pointer to D3DPRESENT_PARAMETERS (width, height, format, windowed, …).
    PresentParams(&'static str),
    /// Pointer to D3DRECT array, count in the previous argument.
    Skip,
}

/// When a method is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Every call (low volume: creation, cursor, reset, state-block recording).
    Always,
    /// Only while logging is on.
    Frame,
}

/// Special handling by the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sp {
    None,
    Present,
    BeginScene,
    EndScene,
    BeginStateBlock,
    EndStateBlock,
    SetFvf,
    SetVertexDeclaration,
    SetStreamSource,
    SetTexture,
    DrawPrimitive,
    DrawIndexedPrimitive,
    DrawPrimitiveUp,
    DrawIndexedPrimitiveUp,
    ShowCursor,
    /// Typed wrappers (post-call data needed): the vtable entry points at a Rust function.
    DrawTextA,
    DrawTextW,
    VbLock,
    VbUnlock,
    RtsBeginScene,
    RtsEndScene,
}

#[derive(Debug, Clone, Copy)]
pub struct Method {
    pub iface: Iface,
    pub slot: u32,
    pub name: &'static str,
    pub args: &'static [A],
    pub when: When,
    pub sp: Sp,
}

const fn m(iface: Iface, slot: u32, name: &'static str, args: &'static [A], when: When, sp: Sp) -> Method {
    Method { iface, slot, name, args, when, sp }
}

use A::*;
use Iface::*;
use When::*;

/// Hooked methods. Slot numbers follow the declaration order in `d3d9.h` / `d3dx9core.h`
/// (IUnknown = 0..2).
pub static METHODS: &[Method] = &[
    // IDirect3DDevice9
    m(Device, 10, "SetCursorProperties", &[U("x"), U("y"), Obj("surface")], Always, Sp::None),
    m(Device, 11, "SetCursorPosition", &[I("x"), I("y"), X("flags")], Frame, Sp::None),
    m(Device, 12, "ShowCursor", &[U("show")], Always, Sp::ShowCursor),
    m(Device, 16, "Reset", &[PresentParams("pp")], Always, Sp::None),
    m(Device, 17, "Present", &[Rect("src"), Rect("dst")], Frame, Sp::Present),
    m(Device, 23, "CreateTexture", &[U("w"), U("h"), U("levels"), X("usage"), U("format"), U("pool")], Frame, Sp::None),
    m(Device, 33, "GetFrontBufferData", &[U("swapchain"), Obj("surface")], Always, Sp::None),
    m(
        Device,
        34,
        "StretchRect",
        &[Obj("src"), Rect("src_rect"), Obj("dst"), Rect("dst_rect"), U("filter")],
        Frame,
        Sp::None,
    ),
    m(Device, 35, "ColorFill", &[Obj("surface"), Rect("rect"), X("color")], Frame, Sp::None),
    m(Device, 36, "CreateOffscreenPlainSurface", &[U("w"), U("h"), U("format"), U("pool")], Always, Sp::None),
    m(Device, 37, "SetRenderTarget", &[U("index"), Obj("surface")], Frame, Sp::None),
    m(Device, 39, "SetDepthStencilSurface", &[Obj("surface")], Frame, Sp::None),
    m(Device, 41, "BeginScene", &[], Frame, Sp::BeginScene),
    m(Device, 42, "EndScene", &[], Frame, Sp::EndScene),
    m(Device, 43, "Clear", &[U("count"), Skip, X("flags"), X("color"), F("z"), U("stencil")], Frame, Sp::None),
    m(Device, 44, "SetTransform", &[U("state"), Mat("m")], Frame, Sp::None),
    m(Device, 47, "SetViewport", &[Viewport("vp")], Frame, Sp::None),
    m(Device, 57, "SetRenderState", &[U("rs"), X("value")], Frame, Sp::None),
    m(Device, 59, "CreateStateBlock", &[U("type")], Always, Sp::None),
    m(Device, 60, "BeginStateBlock", &[], Always, Sp::BeginStateBlock),
    m(Device, 61, "EndStateBlock", &[], Always, Sp::EndStateBlock),
    m(Device, 65, "SetTexture", &[U("stage"), Obj("tex")], Frame, Sp::SetTexture),
    m(Device, 67, "SetTextureStageState", &[U("stage"), U("tss"), X("value")], Frame, Sp::None),
    m(Device, 69, "SetSamplerState", &[U("sampler"), U("ss"), X("value")], Frame, Sp::None),
    m(Device, 75, "SetScissorRect", &[Rect("rect")], Frame, Sp::None),
    m(Device, 81, "DrawPrimitive", &[U("type"), U("start"), U("prims")], Frame, Sp::DrawPrimitive),
    m(
        Device,
        82,
        "DrawIndexedPrimitive",
        &[U("type"), I("base"), U("min_index"), U("num_vertices"), U("start_index"), U("prims")],
        Frame,
        Sp::DrawIndexedPrimitive,
    ),
    m(Device, 83, "DrawPrimitiveUP", &[U("type"), U("prims"), Skip, U("stride")], Frame, Sp::DrawPrimitiveUp),
    m(
        Device,
        84,
        "DrawIndexedPrimitiveUP",
        &[U("type"), U("min_index"), U("num_vertices"), U("prims"), Skip, U("index_format"), Skip, U("stride")],
        Frame,
        Sp::DrawIndexedPrimitiveUp,
    ),
    m(Device, 87, "SetVertexDeclaration", &[Obj("decl")], Frame, Sp::SetVertexDeclaration),
    m(Device, 89, "SetFVF", &[X("fvf")], Frame, Sp::SetFvf),
    m(Device, 92, "SetVertexShader", &[Obj("vs")], Frame, Sp::None),
    m(Device, 100, "SetStreamSource", &[U("stream"), Obj("vb"), U("offset"), U("stride")], Frame, Sp::SetStreamSource),
    m(Device, 104, "SetIndices", &[Obj("ib")], Frame, Sp::None),
    m(Device, 107, "SetPixelShader", &[Obj("ps")], Frame, Sp::None),
    // ID3DXSprite
    m(Sprite, 5, "SetTransform", &[Mat("m")], Frame, Sp::None),
    m(Sprite, 8, "Begin", &[X("flags")], Frame, Sp::None),
    m(Sprite, 9, "Draw", &[Obj("tex"), Rect("src"), V3("center"), V3("pos"), X("color")], Frame, Sp::None),
    m(Sprite, 10, "Flush", &[], Frame, Sp::None),
    m(Sprite, 11, "End", &[], Frame, Sp::None),
    m(Sprite, 12, "OnLostDevice", &[], Always, Sp::None),
    m(Sprite, 13, "OnResetDevice", &[], Always, Sp::None),
    // ID3DXFont
    m(Font, 14, "DrawTextA", &[], Frame, Sp::DrawTextA),
    m(Font, 15, "DrawTextW", &[], Frame, Sp::DrawTextW),
    m(Font, 16, "OnLostDevice", &[], Always, Sp::None),
    m(Font, 17, "OnResetDevice", &[], Always, Sp::None),
    // IDirect3DVertexBuffer9 (only to shadow the bytes D3DX writes into dynamic buffers)
    m(VertexBuffer, 11, "Lock", &[], Frame, Sp::VbLock),
    m(VertexBuffer, 12, "Unlock", &[], Frame, Sp::VbUnlock),
    // ID3DXRenderToSurface
    m(RenderToSurface, 5, "BeginScene", &[Obj("surface"), Viewport("vp")], Frame, Sp::RtsBeginScene),
    m(RenderToSurface, 6, "EndScene", &[U("mip_filter")], Frame, Sp::RtsEndScene),
];

/// Methods of one interface, with their index in `METHODS`.
pub fn methods_of(iface: Iface) -> impl Iterator<Item = (usize, &'static Method)> {
    METHODS.iter().enumerate().filter(move |(_, m)| m.iface == iface)
}

/// Interface vtable sizes (to sanity-check a vtable pointer before patching).
pub fn vtable_len(iface: Iface) -> u32 {
    match iface {
        Device => 119,
        Sprite => 14,
        Font => 18,
        VertexBuffer => 14,
        RenderToSurface => 9,
    }
}

// ------------------------------------------------------------------ argument rendering ----

fn rect<M: Mem>(m: &M, p: u32) -> Val {
    if p == 0 {
        return Val::Null;
    }
    match (m.i32(p), m.i32(p + 4), m.i32(p + 8), m.i32(p + 12)) {
        (Some(l), Some(t), Some(r), Some(b)) => Val::List(vec![l.into(), t.into(), r.into(), b.into()]),
        _ => Val::Str("unreadable".into()),
    }
}

fn floats<M: Mem>(m: &M, p: u32, n: u32) -> Val {
    if p == 0 {
        return Val::Null;
    }
    let mut out = Vec::new();
    for i in 0..n {
        match m.f32(p + 4 * i) {
            Some(f) => out.push(Val::Float(f)),
            None => return Val::Str("unreadable".into()),
        }
    }
    Val::List(out)
}

/// Renders the arguments of `method` for a call whose stack is at `esp` (return address at
/// `esp`, `this` at `esp + 4`, the first argument at `esp + 8`). `label` turns object
/// pointers into descriptive strings (texture size, font face, …).
pub fn render_args<M: Mem>(m: &M, method: &Method, esp: u32, label: &mut dyn FnMut(u32) -> Option<String>) -> Val {
    let mut v = Val::obj();
    for (i, a) in method.args.iter().enumerate() {
        let slot = esp + 8 + 4 * i as u32;
        let raw = m.u32(slot);
        match *a {
            U(k) => v.put(k, raw),
            I(k) => v.put(k, raw.map(|x| x as i32)),
            X(k) => v.put(k, raw.map(|x| Val::Str(format!("{x:#x}"))).unwrap_or(Val::Null)),
            F(k) => v.put(k, raw.map(f32::from_bits)),
            Obj(k) => {
                let p = raw.unwrap_or(0);
                if p == 0 {
                    v.put(k, Val::Null);
                } else {
                    v.put(&format!("{k}_ptr"), Val::Hex(p));
                    if let Some(l) = label(p) {
                        v.put(k, l);
                    }
                }
            }
            Rect(k) => v.put(k, rect(m, raw.unwrap_or(0))),
            V3(k) => v.put(k, floats(m, raw.unwrap_or(0), 3)),
            Mat(k) => v.put(k, floats(m, raw.unwrap_or(0), 16)),
            Viewport(k) => {
                let p = raw.unwrap_or(0);
                let vp = match (m.u32(p), m.u32(p + 4), m.u32(p + 8), m.u32(p + 12), m.f32(p + 16), m.f32(p + 20)) {
                    (Some(x), Some(y), Some(w), Some(h), Some(z0), Some(z1)) if p != 0 => {
                        Val::List(vec![x.into(), y.into(), w.into(), h.into(), z0.into(), z1.into()])
                    }
                    _ => Val::Null,
                };
                v.put(k, vp);
            }
            PresentParams(k) => {
                let p = raw.unwrap_or(0);
                // BackBufferWidth, BackBufferHeight, BackBufferFormat, BackBufferCount,
                // MultiSampleType, MultiSampleQuality, SwapEffect, hDeviceWindow, Windowed, …,
                // FullScreen_RefreshRateInHz (+0x30), PresentationInterval (+0x34).
                let g = |o: u32| m.u32(p + o).map(Val::from).unwrap_or(Val::Null);
                v.put(
                    k,
                    Val::obj()
                        .with("w", g(0))
                        .with("h", g(4))
                        .with("format", g(8))
                        .with("windowed", g(0x20))
                        .with("refresh", g(0x30))
                        .with("interval", g(0x34)),
                );
            }
            Skip => {}
        }
    }
    v
}

// ------------------------------------------------------------------ vertices ---------------

/// Position kind of a fixed-function vertex format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pos {
    None,
    Xyz,
    XyzRhw,
    /// XYZ + n blend weights (skipped).
    XyzB(u32),
}

/// Byte layout of an FVF code: what the decoder prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub pos: Pos,
    pub pos_off: u32,
    pub diffuse: Option<u32>,
    pub specular: Option<u32>,
    pub tex0: Option<u32>,
    pub size: u32,
}

/// Layout of an FVF code (D3DFVF_* bits; texture coordinate sets assumed 2D unless the
/// D3DFVF_TEXCOORDSIZEn bits say otherwise).
pub fn fvf_layout(fvf: u32) -> Layout {
    let mut off = 0;
    let pos = match fvf & 0x400E {
        0x002 => {
            off = 12;
            Pos::Xyz
        }
        0x004 => {
            off = 16;
            Pos::XyzRhw
        }
        p @ (0x006 | 0x008 | 0x00A | 0x00C | 0x00E) => {
            let n = (p - 0x004) / 2;
            off = 12 + 4 * n;
            Pos::XyzB(n)
        }
        0x4002 => {
            off = 16;
            Pos::XyzB(0)
        }
        _ => Pos::None,
    };
    if fvf & 0x010 != 0 {
        off += 12; // normal
    }
    if fvf & 0x020 != 0 {
        off += 4; // point size
    }
    let diffuse = (fvf & 0x040 != 0).then(|| {
        off += 4;
        off - 4
    });
    let specular = (fvf & 0x080 != 0).then(|| {
        off += 4;
        off - 4
    });
    let ntex = (fvf >> 8) & 0xF;
    let mut tex0 = None;
    for i in 0..ntex {
        let sz = match (fvf >> (16 + 2 * i)) & 3 {
            0 => 8,
            1 => 12,
            2 => 16,
            _ => 4,
        };
        if i == 0 {
            tex0 = Some(off);
        }
        off += sz;
    }
    Layout { pos, pos_off: 0, diffuse, specular, tex0, size: off }
}

fn num(f: f32) -> String {
    if !f.is_finite() {
        return format!("{f}");
    }
    let s = format!("{f:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// Layout of a vertex declaration (D3DVERTEXELEMENT9 records of 8 bytes: stream, offset, type,
/// method, usage, usage index; ends at stream 0xFF). Stream 0 only.
pub fn decl_layout(elems: &[u8]) -> Layout {
    let mut l = Layout { pos: Pos::None, pos_off: 0, diffuse: None, specular: None, tex0: None, size: 0 };
    for e in elems.as_chunks::<8>().0 {
        let stream = u16::from_le_bytes([e[0], e[1]]);
        if stream == 0xFF {
            break;
        }
        if stream != 0 {
            continue;
        }
        let off = u32::from(u16::from_le_bytes([e[2], e[3]]));
        let (ty, usage, idx) = (e[4], e[6], e[7]);
        let size = match ty {
            0 => 4,
            1 => 8,
            2 => 12,
            3 => 16,
            4 | 5 => 4,
            _ => 4,
        };
        l.size = l.size.max(off + size);
        match (usage, idx) {
            (0, 0) => {
                l.pos = if ty == 3 { Pos::XyzRhw } else { Pos::Xyz };
                l.pos_off = off;
            }
            (9, 0) => {
                l.pos = Pos::XyzRhw;
                l.pos_off = off;
            }
            (10, 0) if ty == 4 => l.diffuse = Some(off),
            (10, 1) if ty == 4 => l.specular = Some(off),
            (5, 0) if ty == 1 => l.tex0 = Some(off),
            _ => {}
        }
    }
    l
}

/// One vertex as `x,y,z[,rhw] #AARRGGBB u,v` (fields the layout has).
pub fn format_vertex(b: &[u8], l: &Layout) -> String {
    let f = |o: u32| -> f32 {
        let o = o as usize;
        b.get(o..o + 4).map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]])).unwrap_or(f32::NAN)
    };
    let u = |o: u32| -> u32 {
        let o = o as usize;
        b.get(o..o + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]])).unwrap_or(0)
    };
    let p = l.pos_off;
    let mut s = match l.pos {
        Pos::None => String::new(),
        Pos::Xyz | Pos::XyzB(_) => format!("{},{},{}", num(f(p)), num(f(p + 4)), num(f(p + 8))),
        Pos::XyzRhw => format!("{},{},{},{}", num(f(p)), num(f(p + 4)), num(f(p + 8)), num(f(p + 12))),
    };
    if let Some(o) = l.diffuse {
        s.push_str(&format!(" #{:08X}", u(o)));
    }
    if let Some(o) = l.tex0 {
        s.push_str(&format!(" {},{}", num(f(o)), num(f(o + 4))));
    }
    s
}

/// Vertices drawn by `prims` primitives of D3DPRIMITIVETYPE `ty`.
pub fn vertex_count(ty: u32, prims: u32) -> u32 {
    match ty {
        1 => prims,         // point list
        2 => prims * 2,     // line list
        3 => prims + 1,     // line strip
        4 => prims * 3,     // triangle list
        5 | 6 => prims + 2, // strip / fan
        _ => 0,
    }
}

/// Up to this many vertices are written decoded; larger draws only get a hash.
pub const MAX_DECODED: usize = 1024;

/// FNV-1a 32-bit (content hash of vertex data, for large draws).
pub fn fnv1a(b: &[u8]) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for &x in b {
        h ^= u32::from(x);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Renders `count` vertices of `stride` bytes from `raw` (decoded when small, else hashed).
pub fn vertices_val(raw: &[u8], stride: u32, count: u32, l: Option<&Layout>) -> Val {
    let mut o = Val::obj().with("n", count).with("hash", Val::Str(format!("{:08x}", fnv1a(raw))));
    if let Some(l) = l
        && stride > 0
        && (count as usize) <= MAX_DECODED
    {
        let list: Vec<Val> =
            raw.chunks(stride as usize).take(count as usize).map(|c| Val::Str(format_vertex(c, l))).collect();
        o.put("v", Val::List(list));
    }
    o
}

// ------------------------------------------------------------------ text --------------------

/// UTF-16 string of `count` units (−1: up to the NUL, at most `max`).
pub fn wide_str<M: Mem>(m: &M, p: u32, count: i32, max: usize) -> Option<String> {
    if p == 0 {
        return None;
    }
    let n = if count < 0 { max } else { (count as usize).min(max) };
    let mut units = Vec::new();
    for i in 0..n as u32 {
        let c = m.u16(p + 2 * i)?;
        if count < 0 && c == 0 {
            break;
        }
        units.push(c);
    }
    Some(String::from_utf16_lossy(&units))
}

/// ANSI string of `count` bytes (−1: up to the NUL), as Latin-1 (samp's text is CP-1252).
pub fn ansi_str<M: Mem>(m: &M, p: u32, count: i32, max: usize) -> Option<String> {
    if p == 0 {
        return None;
    }
    let n = if count < 0 { max } else { (count as usize).min(max) };
    let mut out = String::new();
    for i in 0..n as u32 {
        let c = m.u8(p + i)?;
        if count < 0 && c == 0 {
            break;
        }
        out.push(char::from(c));
    }
    Some(out)
}

/// `D3DXFONT_DESCA` (56 bytes) as a short label: `Arial h16 w0 wt700 mip1 it0 cs1 op0 q4 pf0`.
pub fn font_desc_label(b: &[u8; 56]) -> String {
    let u = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let face: String = b[24..56].iter().take_while(|&&c| c != 0).map(|&c| char::from(c)).collect();
    format!(
        "{face} h{} w{} wt{} mip{} it{} cs{} op{} q{} pf{}",
        u(0) as i32,
        u(4),
        u(8),
        u(12),
        u(16),
        b[20],
        b[21],
        b[22],
        b[23]
    )
}

// ------------------------------------------------------------------ control -----------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    /// Calls outside GTA's own scenes (the client's Present pass) plus D3DX render-to-surface
    /// scenes and every D3DX font/sprite call.
    Ui,
    /// Every call of the frame.
    All,
    /// Only font DrawText calls (cheap; for long windows).
    Text,
}

impl Mode {
    pub fn code(self) -> u8 {
        match self {
            Mode::Off => 0,
            Mode::Ui => 1,
            Mode::All => 2,
            Mode::Text => 3,
        }
    }
    pub fn from_code(c: u8) -> Mode {
        match c {
            1 => Mode::Ui,
            2 => Mode::All,
            3 => Mode::Text,
            _ => Mode::Off,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Ui => "ui",
            Mode::All => "all",
            Mode::Text => "text",
        }
    }
}

/// Max frames one `d3d` command may log.
pub const MAX_FRAMES: u32 = 100_000;

/// `d3d off` | `d3d on|ui|all|text [FRAMES]` (default 1 frame; `on` = `ui`).
pub fn parse_cmd(args: &str) -> Option<(Mode, u32)> {
    let w: Vec<&str> = args.split_whitespace().collect();
    let mode = match w.first().copied()? {
        "off" => return (w.len() == 1).then_some((Mode::Off, 0)),
        "on" | "ui" => Mode::Ui,
        "all" => Mode::All,
        "text" => Mode::Text,
        _ => return None,
    };
    let frames = match w.get(1) {
        None => 1,
        Some(n) => n.parse::<u32>().ok().filter(|&n| (1..=MAX_FRAMES).contains(&n))?,
    };
    (w.len() <= 2).then_some((mode, frames))
}

/// Whether a call of `method` is written: `scene_game` = inside a scene that GTA (not D3DX)
/// began.
pub fn wanted(mode: Mode, method: &Method, scene_game: bool) -> bool {
    if method.when == When::Always {
        return true;
    }
    match mode {
        Mode::Off => false,
        Mode::All => true,
        Mode::Text => matches!(method.sp, Sp::DrawTextA | Sp::DrawTextW | Sp::Present),
        Mode::Ui => {
            matches!(method.iface, Iface::Font | Iface::Sprite | Iface::RenderToSurface)
                || matches!(method.sp, Sp::Present | Sp::BeginScene | Sp::EndScene)
                || !scene_game
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::fake::FakeMem;

    #[test]
    fn table_is_consistent() {
        for (i, a) in METHODS.iter().enumerate() {
            assert!(a.slot < vtable_len(a.iface), "{}", a.name);
            for b in &METHODS[i + 1..] {
                assert!(!(a.iface == b.iface && a.slot == b.slot), "duplicate slot {}", a.name);
            }
        }
        let names: Vec<_> = methods_of(Iface::Device).map(|(_, m)| (m.slot, m.name)).collect();
        assert!(
            names.contains(&(17, "Present")) && names.contains(&(16, "Reset")) && names.contains(&(42, "EndScene"))
        );
        assert!(names.contains(&(57, "SetRenderState")) && names.contains(&(83, "DrawPrimitiveUP")));
        assert!(methods_of(Iface::Font).any(|(_, m)| m.slot == 14 && m.name == "DrawTextA"));
        assert!(methods_of(Iface::Sprite).any(|(_, m)| m.slot == 9 && m.name == "Draw"));
    }

    #[test]
    fn fvf_layouts() {
        // XYZRHW | DIFFUSE (nametag bars)
        let l = fvf_layout(0x044);
        assert_eq!((l.pos, l.diffuse, l.tex0, l.size), (Pos::XyzRhw, Some(16), None, 20));
        // XYZ | DIFFUSE | TEX1 (D3DX sprite)
        let l = fvf_layout(0x142);
        assert_eq!((l.pos, l.diffuse, l.tex0, l.size), (Pos::Xyz, Some(12), Some(16), 24));
        // XYZRHW | DIFFUSE | SPECULAR | TEX1
        let l = fvf_layout(0x1C4);
        assert_eq!((l.diffuse, l.specular, l.tex0, l.size), (Some(16), Some(20), Some(24), 32));
        // XYZ | NORMAL | TEX1
        assert_eq!(fvf_layout(0x112).size, 32);
    }

    #[test]
    fn declarations() {
        // position float3 @0, colour @12, texcoord float2 @16, end
        let mut e = Vec::new();
        for (off, ty, usage) in [(0u16, 2u8, 0u8), (12, 4, 10), (16, 1, 5)] {
            e.extend_from_slice(&0u16.to_le_bytes());
            e.extend_from_slice(&off.to_le_bytes());
            e.extend_from_slice(&[ty, 0, usage, 0]);
        }
        e.extend_from_slice(&[0xFF, 0, 0, 0, 17, 0, 0, 0]);
        let l = decl_layout(&e);
        assert_eq!((l.pos, l.pos_off, l.diffuse, l.tex0, l.size), (Pos::Xyz, 0, Some(12), Some(16), 24));
    }

    #[test]
    fn vertex_text() {
        let l = fvf_layout(0x044);
        let mut b = Vec::new();
        for f in [10.5f32, -0.0, 0.0, 1.0] {
            b.extend_from_slice(&f.to_le_bytes());
        }
        b.extend_from_slice(&0xFFB9_2228u32.to_le_bytes());
        assert_eq!(format_vertex(&b, &l), "10.5,0,0,1 #FFB92228");
        let v = vertices_val(&b, 20, 1, Some(&l)).to_json();
        assert!(v.contains("\"v\":[\"10.5,0,0,1 #FFB92228\"]"), "{v}");
        assert_eq!(vertex_count(6, 2), 4);
        assert_eq!(vertex_count(4, 2), 6);
    }

    #[test]
    fn args_render() {
        let mut mem = FakeMem::default();
        // esp: ret, this, rect ptr, ...
        let esp = 0x1000;
        mem.set_u32(esp, 0x1234);
        mem.set_u32(esp + 4, 0xAAAA);
        let draw = METHODS.iter().find(|m| m.iface == Iface::Sprite && m.name == "Draw").unwrap();
        mem.set_u32(esp + 8, 0x5000); // tex
        mem.set_u32(esp + 12, 0x2000); // src rect
        mem.set_u32(esp + 16, 0); // center
        mem.set_u32(esp + 20, 0x3000); // pos
        mem.set_u32(esp + 24, 0xFFFF_0000);
        for (i, v) in [1i32, 2, 3, 4].iter().enumerate() {
            mem.set_u32(0x2000 + 4 * i as u32, *v as u32);
        }
        for (i, v) in [5.0f32, 6.0, 0.0].iter().enumerate() {
            mem.set_f32(0x3000 + 4 * i as u32, *v);
        }
        let v = render_args(&mem, draw, esp, &mut |p| (p == 0x5000).then(|| "tex 512x512".to_string())).to_json();
        assert_eq!(
            v,
            r#"{"tex_ptr":"0x5000","tex":"tex 512x512","src":[1,2,3,4],"center":null,"pos":[5.000,6.000,0.000],"color":"0xffff0000"}"#
        );
    }

    #[test]
    fn strings_and_fonts() {
        let mut mem = FakeMem::default();
        let s: Vec<u8> = "hi{FF0000}x\0".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        mem.set(0x100, &s);
        assert_eq!(wide_str(&mem, 0x100, -1, 64).as_deref(), Some("hi{FF0000}x"));
        assert_eq!(wide_str(&mem, 0x100, 2, 64).as_deref(), Some("hi"));
        mem.set(0x200, b"abc\0");
        assert_eq!(ansi_str(&mem, 0x200, -1, 64).as_deref(), Some("abc"));
        let mut d = [0u8; 56];
        d[0..4].copy_from_slice(&16i32.to_le_bytes());
        d[8..12].copy_from_slice(&700u32.to_le_bytes());
        d[12..16].copy_from_slice(&1u32.to_le_bytes());
        d[20] = 1;
        d[22] = 4;
        d[24..29].copy_from_slice(b"Arial");
        assert_eq!(font_desc_label(&d), "Arial h16 w0 wt700 mip1 it0 cs1 op0 q4 pf0");
    }

    #[test]
    fn commands() {
        assert_eq!(parse_cmd("on"), Some((Mode::Ui, 1)));
        assert_eq!(parse_cmd("all 3"), Some((Mode::All, 3)));
        assert_eq!(parse_cmd("text 600"), Some((Mode::Text, 600)));
        assert_eq!(parse_cmd("off"), Some((Mode::Off, 0)));
        assert_eq!(parse_cmd("on 0"), None);
        assert_eq!(parse_cmd("bogus"), None);
        assert_eq!(parse_cmd("on 2 3"), None);
        let dev_rs = METHODS.iter().find(|m| m.name == "SetRenderState").unwrap();
        assert!(!wanted(Mode::Ui, dev_rs, true));
        assert!(wanted(Mode::Ui, dev_rs, false));
        assert!(wanted(Mode::All, dev_rs, true));
        assert!(!wanted(Mode::Off, dev_rs, false));
        let cur = METHODS.iter().find(|m| m.name == "ShowCursor").unwrap();
        assert!(wanted(Mode::Off, cur, true));
    }
}
