//! EFX-1 — effect tracer, platform-independent part.
//!
//! "Capture everything a function does end to end and compare it with our client." Two modes:
//!
//! * **step** (`efx arm TARGET`): a one-shot `int3` at the entry of a client function (an RPC
//!   handler of samp.dll from `docs/re/rpc-ids.md`, an RVA, a named event or a GTA function).
//!   When it is hit, the tracer single-steps that thread (trap flag, vectored handler) through
//!   the client code and every gta_sa.exe function it calls, until the target returns. Every
//!   instruction is decoded with iced-x86 here (`analyse`) to get its memory operands, so the
//!   Windows side can log each write into game memory (old and new bytes), each call into
//!   gta_sa.exe (arguments, return value), reads of game memory that decide a branch, and the
//!   client-internal calls for context.
//! * **window** (`efx watch FRAMES SPEC`): for a client we do not single-step (our own
//!   samp_client.dll has no symbol for its RPC dispatch, and samp-client must not be changed for
//!   the tracer): guard pages on the game pages a step capture of the original wrote, plus
//!   one-shot `int3` call hooks on the game functions it called, for a window of frames. The
//!   records use the same names and symbolic addresses, so `tools/efx/efx_diff.py` can align
//!   the two.
//!
//! Addresses of game objects are written symbolically (`World::sym`): `player_ped+0x540`,
//! `player_ped.matrix+0x30`, `veh#12+0x4c0`, `gta:0xb7cd98`, so that captures of two clients
//! (different heaps) can be compared.

use crate::mem::Mem;
use iced_x86::{
    ConditionCode, Decoder, DecoderOptions, FlowControl, Instruction, InstructionInfoFactory, Mnemonic, OpAccess,
    OpKind, Register,
};

// ------------------------------------------------------------------ builds and targets ------

/// Preferred image base of both samp.dll 0.3.7-R4 builds (handler VAs in rpc-ids.md use it).
pub const SAMP_IMAGE_BASE: u32 = 0x1000_0000;
/// PE `TimeDateStamp` of build A (fv0.3.7.4, 2021-05-07 02:57:47 UTC) and build B
/// (fv0.3.7.3, 2019-11-21 03:38:53 UTC), re/samp-dll/VERSIONS.md.
pub const STAMP_A: u32 = 0x6094_ACAB;
pub const STAMP_B: u32 = 0x5DD6_06CD;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    A,
    B,
}

pub fn build_of_stamp(stamp: u32) -> Option<Build> {
    match stamp {
        STAMP_A => Some(Build::A),
        STAMP_B => Some(Build::B),
        _ => None,
    }
}

/// The RPC table of `docs/re/rpc-ids.md` (id, handler in build A, handler in build B, purpose).
pub const RPC_IDS_MD: &str = include_str!("../../../docs/re/rpc-ids.md");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcRow {
    pub id: u8,
    pub a: u32,
    pub b: u32,
    pub purpose: String,
}

fn hex_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    u32::from_str_radix(h, 16).ok()
}

/// Number in decimal or `0x` hex.
pub fn num(s: &str) -> Option<u64> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// Rows `| id | 0xhex | A handler | B handler | group | purpose |` of the S→C table.
pub fn rpc_table(md: &str) -> Vec<RpcRow> {
    md.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split('|').map(str::trim).collect();
            if c.len() < 8 || !c[0].is_empty() {
                return None;
            }
            let id: u8 = c[1].parse().ok()?;
            let (a, b) = (hex_u32(c[3])?, hex_u32(c[4])?);
            (a >= SAMP_IMAGE_BASE && b >= SAMP_IMAGE_BASE).then(|| RpcRow {
                id,
                a,
                b,
                purpose: c[6].trim_start_matches("open.mp: ").to_string(),
            })
        })
        .collect()
}

pub fn rpc_handler(id: u8, build: Build) -> Option<u32> {
    rpc_table(RPC_IDS_MD).into_iter().find(|r| r.id == id).map(|r| if build == Build::A { r.a } else { r.b })
}

/// Named entry points found from the game side (so they work for any client module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Target of the `Render2dStuff` call in `Idle` (0x53EB12), which both clients retarget
    /// to their per-frame entry (gta-patches CSV row 6).
    Frame,
    /// Target of the `CGame::Process` call in `Idle` (0x53E981, CSV row 10).
    GameProcess,
}

pub const IDLE_RENDER_2D_CALL: u32 = 0x53EB12;
pub const IDLE_GAME_PROCESS_CALL: u32 = 0x53E981;

impl Event {
    pub fn call_site(self) -> u32 {
        match self {
            Event::Frame => IDLE_RENDER_2D_CALL,
            Event::GameProcess => IDLE_GAME_PROCESS_CALL,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// RPC id → handler of the loaded samp.dll build.
    Rpc(u8),
    /// VA in the client module's preferred-base space (0x10000000-based, as in rpc-ids.md).
    ClientVa(u32),
    /// `module+0xRVA` (any loaded module).
    ModRva(String, u32),
    /// A gta_sa.exe function (fixed address).
    Gta(u32),
    Event(Event),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Rpc(n) => write!(f, "rpc:{n}"),
            Target::ClientVa(v) => write!(f, "{v:#x}"),
            Target::ModRva(m, r) => write!(f, "{m}+{r:#x}"),
            Target::Gta(a) => write!(f, "gta:{a:#x}"),
            Target::Event(Event::Frame) => write!(f, "event:frame"),
            Target::Event(Event::GameProcess) => write!(f, "event:game_process"),
        }
    }
}

pub fn parse_target(s: &str) -> Result<Target, String> {
    let t = s.trim().to_ascii_lowercase();
    if let Some(n) = t.strip_prefix("rpc:") {
        return n.parse().map(Target::Rpc).map_err(|_| format!("bad rpc id {n:?}"));
    }
    if let Some(e) = t.strip_prefix("event:") {
        return match e {
            "frame" => Ok(Target::Event(Event::Frame)),
            "game_process" => Ok(Target::Event(Event::GameProcess)),
            _ => Err(format!("unknown event {e:?} (frame, game_process)")),
        };
    }
    if let Some(a) = t.strip_prefix("gta:") {
        return hex_u32(a).map(Target::Gta).ok_or_else(|| format!("bad address {a:?}"));
    }
    if let Some((m, r)) = t.split_once('+') {
        let rva = hex_u32(r).ok_or_else(|| format!("bad rva {r:?}"))?;
        return Ok(Target::ModRva(m.to_string(), rva));
    }
    match hex_u32(&t) {
        Some(v) if v >= SAMP_IMAGE_BASE => Ok(Target::ClientVa(v)),
        Some(v) => Ok(Target::Gta(v)),
        None => Err(format!("bad target {s:?} (rpc:N, 0xVA, module+0xRVA, gta:0xADDR, event:frame)")),
    }
}

/// GTA functions that are logged as one call and stepped over by default: model streaming
/// and CD-stream I/O (huge, and their effect is "a model got loaded"). Named via gta-reversed
/// Streaming.cpp / CdStreamInfo.cpp.
pub const DEFAULT_STEP_OVER: &[u32] = &[
    0x40EA10, // CStreaming::LoadAllRequestedModels
    0x40E3A0, // CStreaming::LoadRequestedModels
    0x40E4E0, // CStreaming::FlushRequestList
    0x40C6B0, // CStreaming::ConvertBufferToObject
    0x40EB70, // CStreaming::LoadScene
    0x406460, // CdStreamSync
    0x406A20, // CdStreamRead
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmOpts {
    /// Total single steps before the capture stops (the target still returns normally).
    pub budget: u64,
    /// Steps allowed inside one top-level game call before the rest of it is stepped over.
    pub sub: u64,
    /// Max nesting of game calls that is stepped into (deeper ones are stepped over).
    pub depth: u32,
    /// Also log client-code writes to heap memory that is not a known game object.
    pub heap: bool,
    /// Max `efx_branch` records (game-memory reads that decide a client branch).
    pub branches: u32,
    /// Max `efx_icall` records (client-internal calls).
    pub icalls: u32,
    /// Number of hits to capture (re-armed after each).
    pub count: u32,
    /// Extra functions (absolute VAs) to step over.
    pub over: Vec<u32>,
    /// Step over every game call (log calls only, no game-side writes).
    pub shallow: bool,
}

impl Default for ArmOpts {
    fn default() -> Self {
        ArmOpts {
            budget: 3_000_000,
            sub: 400_000,
            depth: 24,
            heap: false,
            branches: 400,
            icalls: 4000,
            count: 1,
            over: Vec::new(),
            shallow: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchOpts {
    /// Minimum frames and milliseconds the window stays open (both must pass).
    pub frames: u32,
    pub ms: u32,
    /// Max guard-page faults per window (then the guards are dropped).
    pub faults: u32,
    /// Max logged hits per hooked function.
    pub calls: u32,
}

impl Default for WatchOpts {
    fn default() -> Self {
        WatchOpts { frames: 10, ms: 1500, faults: 400_000, calls: 300 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EfxCmd {
    Arm(Target, ArmOpts),
    /// `efx watch SPECFILE [frames=N] [ms=N] [faults=N] [calls=N]`
    Watch(String, WatchOpts),
    Disarm,
    Status,
}

fn opt_u64(k: &str, v: &str) -> Result<u64, String> {
    num(v).ok_or_else(|| format!("bad value for {k}: {v:?}"))
}

/// Arguments after `efx `.
pub fn parse_cmd(args: &str) -> Result<EfxCmd, String> {
    let w: Vec<&str> = args.split_whitespace().collect();
    match w.as_slice() {
        ["status"] => Ok(EfxCmd::Status),
        ["disarm"] | ["off"] => Ok(EfxCmd::Disarm),
        ["arm", target, rest @ ..] => {
            let t = parse_target(target)?;
            let mut o = ArmOpts::default();
            for kv in rest {
                let (k, v) = kv.split_once('=').ok_or_else(|| format!("expected key=value, got {kv:?}"))?;
                match k {
                    "budget" => o.budget = opt_u64(k, v)?,
                    "sub" => o.sub = opt_u64(k, v)?,
                    "depth" => o.depth = opt_u64(k, v)? as u32,
                    "heap" => o.heap = v == "1",
                    "branches" => o.branches = opt_u64(k, v)? as u32,
                    "icalls" => o.icalls = opt_u64(k, v)? as u32,
                    "count" => o.count = opt_u64(k, v)?.max(1) as u32,
                    "shallow" => o.shallow = v == "1",
                    "over" => {
                        for a in v.split(',').filter(|a| !a.is_empty()) {
                            o.over.push(hex_u32(a).ok_or_else(|| format!("bad address {a:?}"))?);
                        }
                    }
                    _ => return Err(format!("unknown option {k:?}")),
                }
            }
            Ok(EfxCmd::Arm(t, o))
        }
        ["watch", spec, rest @ ..] => {
            let mut o = WatchOpts::default();
            for kv in rest {
                let (k, v) = kv.split_once('=').ok_or_else(|| format!("expected key=value, got {kv:?}"))?;
                let n = opt_u64(k, v)? as u32;
                match k {
                    "frames" => o.frames = n,
                    "ms" => o.ms = n,
                    "faults" => o.faults = n,
                    "calls" => o.calls = n,
                    _ => return Err(format!("unknown option {k:?}")),
                }
            }
            Ok(EfxCmd::Watch(spec.to_string(), o))
        }
        _ => Err("usage: efx arm TARGET [opts] | efx watch SPECFILE [opts] | efx status | efx disarm".into()),
    }
}

// ------------------------------------------------------------------ instruction analysis ----

/// Register state of the stepped thread (from the exception CONTEXT).
#[derive(Debug, Default, Clone, Copy)]
pub struct Cpu {
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esp: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,
    pub eip: u32,
    pub eflags: u32,
    /// Linear base of `fs:` (the thread's TEB).
    pub fs_base: u32,
}

impl Cpu {
    fn reg(&self, r: Register) -> Option<u64> {
        let full = r.full_register32();
        let v = match full {
            Register::EAX => self.eax,
            Register::ECX => self.ecx,
            Register::EDX => self.edx,
            Register::EBX => self.ebx,
            Register::ESP => self.esp,
            Register::EBP => self.ebp,
            Register::ESI => self.esi,
            Register::EDI => self.edi,
            Register::EIP => self.eip,
            _ => {
                return match r {
                    Register::ES | Register::CS | Register::SS | Register::DS | Register::GS => Some(0),
                    Register::FS => Some(u64::from(self.fs_base)),
                    _ => None,
                };
            }
        };
        let v = u64::from(v);
        Some(match r.size() {
            1 if r.is_gpr8() && matches!(r, Register::AH | Register::CH | Register::DH | Register::BH) => {
                (v >> 8) & 0xFF
            }
            1 => v & 0xFF,
            2 => v & 0xFFFF,
            _ => v,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub addr: u32,
    pub size: u32,
    pub read: bool,
    pub write: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Next,
    /// Near call; target if it could be computed.
    Call(Option<u32>),
    /// Unconditional jump (incl. indirect).
    Jump(Option<u32>),
    /// Conditional branch with its taken target.
    Cond(u32),
    Ret,
    /// int/iret/syscall/… (the capture is aborted on these).
    Other,
}

/// `rep movs*` / `rep stos*` with DF = 0 (emulated as one record).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepString {
    pub stos: bool,
    pub elem: u32,
    pub count: u32,
    pub dst: u32,
    pub src: u32,
}

#[derive(Debug, Clone)]
pub struct Step {
    pub len: u32,
    pub next_ip: u32,
    pub flow: Flow,
    pub acc: Vec<Access>,
    pub rep: Option<RepString>,
    /// Jcc / SETcc / CMOVcc: consumes the flags set before.
    pub cond_use: bool,
    /// Modifies arithmetic flags.
    pub sets_flags: bool,
    /// popf / iret: may clear the trap flag.
    pub writes_tf: bool,
    pub mnemonic: Mnemonic,
}

/// Decodes the instruction at `cpu.eip` (`code` = up to 15 bytes from there) and computes
/// its memory operands and control flow. `mem` is used for indirect call/jump targets.
pub fn analyse<M: Mem>(code: &[u8], cpu: &Cpu, mem: &M, info: &mut InstructionInfoFactory) -> Option<Step> {
    let mut d = Decoder::with_ip(32, code, u64::from(cpu.eip), DecoderOptions::NONE);
    let mut ins = Instruction::default();
    d.decode_out(&mut ins);
    if ins.is_invalid() {
        return None;
    }
    let getter = |r: Register, _: usize, _: usize| cpu.reg(r);
    let next_ip = ins.next_ip32();
    let indirect = |ins: &Instruction| -> Option<u32> {
        match ins.op0_kind() {
            OpKind::Register => cpu.reg(ins.op0_register()).map(|v| v as u32),
            OpKind::Memory => {
                let a = ins.virtual_address(0, 0, getter)? as u32;
                mem.u32(a)
            }
            _ => None,
        }
    };
    let flow = match ins.flow_control() {
        FlowControl::Next => Flow::Next,
        FlowControl::Call => Flow::Call(Some(ins.near_branch32())),
        FlowControl::IndirectCall => Flow::Call(indirect(&ins)),
        FlowControl::UnconditionalBranch => Flow::Jump(Some(ins.near_branch32())),
        FlowControl::IndirectBranch => Flow::Jump(indirect(&ins)),
        FlowControl::ConditionalBranch => Flow::Cond(ins.near_branch32()),
        FlowControl::Return => Flow::Ret,
        _ => Flow::Other,
    };
    let rep = if (ins.has_rep_prefix() || ins.has_repe_prefix())
        && ins.op0_kind() == OpKind::MemoryESEDI
        && cpu.eflags & 0x400 == 0
        && matches!(
            ins.mnemonic(),
            Mnemonic::Movsb | Mnemonic::Movsw | Mnemonic::Movsd | Mnemonic::Stosb | Mnemonic::Stosw | Mnemonic::Stosd
        ) {
        let stos = matches!(ins.mnemonic(), Mnemonic::Stosb | Mnemonic::Stosw | Mnemonic::Stosd);
        let elem = match ins.mnemonic() {
            Mnemonic::Movsb | Mnemonic::Stosb => 1,
            Mnemonic::Movsw | Mnemonic::Stosw => 2,
            _ => 4,
        };
        Some(RepString { stos, elem, count: cpu.ecx, dst: cpu.edi, src: cpu.esi })
    } else {
        None
    };
    let i = info.info(&ins);
    let acc = i
        .used_memory()
        .iter()
        .filter_map(|u| {
            let (read, write) = match u.access() {
                OpAccess::Read | OpAccess::CondRead => (true, false),
                OpAccess::Write | OpAccess::CondWrite => (false, true),
                OpAccess::ReadWrite | OpAccess::ReadCondWrite => (true, true),
                _ => return None,
            };
            let addr = u.virtual_address(0, getter)? as u32;
            let size = (u.memory_size().size() as u32).clamp(1, 512);
            Some(Access { addr, size, read, write })
        })
        .collect();
    Some(Step {
        len: ins.len() as u32,
        next_ip,
        flow,
        acc,
        rep,
        cond_use: ins.condition_code() != ConditionCode::None,
        sets_flags: ins.rflags_modified() != 0,
        writes_tf: matches!(ins.mnemonic(), Mnemonic::Popf | Mnemonic::Popfd | Mnemonic::Iretd | Mnemonic::Iret),
        mnemonic: ins.mnemonic(),
    })
}

// ------------------------------------------------------------------ symbolic addresses ------

/// `CPools::ms_p*Pool` pointers (gta-reversed Pools.cpp) and element sizes (largest class in
/// each pool: CCopPed 0x7C4, CHeli 0xA18, CPedIntelligence 0x294; gta-reversed VALIDATE_SIZE).
pub const POOLS: &[(&str, u32, u32)] = &[
    ("ped", 0xB74490, 0x7C4),
    ("veh", 0xB74494, 0xA18),
    ("obj", 0xB7449C, 0x19C),
    ("task", 0xB744A8, 0x80),
    ("event", 0xB744AC, 0x44),
    ("intel", 0xB744C0, 0x294),
    ("taskalloc", 0xB744BC, 0x20),
    ("pointroute", 0xB744B0, 0x64),
    ("noderoute", 0xB744B8, 0x24),
];

/// `gMatrixList.m_pObjects` (0xB74288 + 6 × 0x54; gta-reversed MatrixLinkList.h),
/// `CPlaceable::NUM_MATRICES_TO_CREATE` and `sizeof(CMatrixLink)`.
pub const MATRIX_LIST_OBJECTS: u32 = 0xB74288 + 6 * 0x54;
pub const MATRIX_COUNT: u32 = 900;
pub const MATRIX_SIZE: u32 = 0x54;
/// `CPed::m_pIntelligence`, `m_pPlayerData`, `m_pVehicle` (plugin-sdk VALIDATE_OFFSET).
pub const PED_INTELLIGENCE: u32 = 0x47C;
pub const PED_PLAYER_DATA: u32 = 0x480;
pub const PED_VEHICLE: u32 = 0x58C;
pub const PLAYER_DATA_SIZE: u32 = 0xAC;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pool {
    pub name: &'static str,
    pub base: u32,
    pub count: u32,
    pub elem: u32,
}

/// The game objects a capture refers to symbolically, read at capture start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct World {
    pub gta: (u32, u32),
    /// Named objects: (name, base, size), checked in order (matrices before their owners).
    pub named: Vec<(String, u32, u32)>,
    pub pools: Vec<Pool>,
    pub matrices: u32,
    /// How `near_veh` was chosen (live vehicles seen, nearest distance), for the begin record.
    pub note: String,
}

impl World {
    pub fn read<M: Mem>(m: &M, gta: (u32, u32)) -> World {
        let mut w = World { gta, ..World::default() };
        let ped = m.u32(crate::gta::CWORLD_PLAYERS).unwrap_or(0);
        let add = |w: &mut World, name: &str, base: u32, size: u32| {
            if base >= 0x10000 {
                w.named.push((name.to_string(), base, size));
            }
        };
        if ped != 0 {
            let veh = m.u32(ped + PED_VEHICLE).unwrap_or(0);
            add(&mut w, "player_ped.matrix", m.u32(ped + crate::gta::off::PLACEABLE_MATRIX).unwrap_or(0), MATRIX_SIZE);
            if veh != 0 {
                add(
                    &mut w,
                    "player_veh.matrix",
                    m.u32(veh + crate::gta::off::PLACEABLE_MATRIX).unwrap_or(0),
                    MATRIX_SIZE,
                );
            }
            add(&mut w, "player_ped", ped, 0x7C4);
            if veh != 0 {
                add(&mut w, "player_veh", veh, 0xA18);
            }
            add(&mut w, "player_intel", m.u32(ped + PED_INTELLIGENCE).unwrap_or(0), 0x294);
            add(&mut w, "player_data", m.u32(ped + PED_PLAYER_DATA).unwrap_or(0), PLAYER_DATA_SIZE);
        }
        for &(name, ptr, elem) in POOLS {
            let Some(pool) = m.u32(ptr).filter(|&p| p != 0) else { continue };
            let (Some(base), Some(count)) = (m.u32(pool), m.i32(pool + 8)) else { continue };
            if base != 0 && (1..=20000).contains(&count) {
                w.pools.push(Pool { name, base, count: count as u32, elem });
            }
        }
        // The vehicle nearest to the player (≤ 30 m) that the player is not in: the target of
        // an enter/put-in RPC, named the same way in both clients' captures.
        if ped != 0
            && let Some(pp) = crate::gta::entity_pos(m, ped)
            && let Some(vp) = POOLS.iter().position(|p| p.0 == "veh")
            && let Some(pool) = m.u32(POOLS[vp].1).filter(|&p| p != 0)
            && let Some(flags) = m.u32(pool + 4)
            && let Some(vehs) = w.pools.iter().find(|p| p.name == "veh").cloned()
        {
            let cur = m.u32(ped + PED_VEHICLE).unwrap_or(0);
            let mut best: Option<(f32, u32)> = None;
            let (mut live, mut nearest) = (0, f32::MAX);
            for i in 0..vehs.count.min(1000) {
                if m.u8(flags + i).is_none_or(|f| f & 0x80 != 0) {
                    continue;
                }
                live += 1;
                let v = vehs.base + i * vehs.elem;
                if v == cur {
                    continue;
                }
                let Some(q) = crate::gta::entity_pos(m, v) else { continue };
                let d = (0..3).map(|k| (q[k] - pp[k]) * (q[k] - pp[k])).sum::<f32>().sqrt();
                nearest = nearest.min(d);
                if d <= 30.0 && best.is_none_or(|b| d < b.0) {
                    best = Some((d, v));
                }
            }
            w.note =
                format!("player at {:.1},{:.1},{:.1}; {live} vehicles, nearest {nearest:.1} m", pp[0], pp[1], pp[2]);
            if let Some((_, v)) = best {
                let mat = m.u32(v + crate::gta::off::PLACEABLE_MATRIX).unwrap_or(0);
                let at = w.named.iter().position(|n| n.0 == "player_ped").unwrap_or(0);
                if mat >= 0x10000 {
                    w.named.insert(at, ("near_veh.matrix".into(), mat, MATRIX_SIZE));
                }
                w.named.push(("near_veh".into(), v, 0xA18));
            }
        }
        w.matrices = m.u32(MATRIX_LIST_OBJECTS).unwrap_or(0);
        w
    }

    /// Whether `a` lies inside a known game object (named, pool slot, matrix) or the game image.
    pub fn is_game_object(&self, a: u32) -> bool {
        self.sym_obj(a).is_some() || (a >= self.gta.0 && a < self.gta.1)
    }

    fn sym_obj(&self, a: u32) -> Option<String> {
        for (n, b, s) in &self.named {
            if a >= *b && a - b < *s {
                return Some(format!("{n}+{:#x}", a - b));
            }
        }
        for p in &self.pools {
            if a >= p.base && a - p.base < p.count * p.elem {
                let r = a - p.base;
                return Some(format!("{}#{}+{:#x}", p.name, r / p.elem, r % p.elem));
            }
        }
        if self.matrices != 0 && a >= self.matrices && a - self.matrices < MATRIX_COUNT * MATRIX_SIZE {
            let r = a - self.matrices;
            return Some(format!("matrix#{}+{:#x}", r / MATRIX_SIZE, r % MATRIX_SIZE));
        }
        None
    }

    /// Symbolic name of a data address; `None` for memory that is not a known game object
    /// (client heap, stack, other modules).
    pub fn sym(&self, a: u32) -> Option<String> {
        if let Some(s) = self.sym_obj(a) {
            return Some(s);
        }
        (a >= self.gta.0 && a < self.gta.1).then(|| format!("gta:{a:#x}"))
    }

    /// Inverse of `sym` (window specs): `player_ped+0x540`, `gta:0xb7cd98`, `veh#3+0x10`.
    pub fn resolve(&self, s: &str) -> Option<u32> {
        let s = s.trim();
        if let Some(a) = s.strip_prefix("gta:") {
            return hex_u32(a);
        }
        let (base, off) = match s.rsplit_once('+') {
            Some((b, o)) => (b, hex_u32(o)?),
            None => (s, 0),
        };
        if let Some((_, b, _)) = self.named.iter().find(|(n, _, _)| n == base) {
            return Some(b + off);
        }
        if let Some((pool, idx)) = base.split_once('#') {
            let idx: u32 = idx.parse().ok()?;
            if pool == "matrix" {
                return (self.matrices != 0 && idx < MATRIX_COUNT).then(|| self.matrices + idx * MATRIX_SIZE + off);
            }
            let p = self.pools.iter().find(|p| p.name == pool)?;
            return (idx < p.count).then(|| p.base + idx * p.elem + off);
        }
        None
    }
}

/// Lower-case hex of bytes in memory order.
pub fn hex_bytes(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// One entry of a window spec: a memory address to watch, or a function to hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecLine {
    Mem { sym: String, size: u32 },
    Fn { addr: u32, name: String },
}

/// Window spec file: `mem SYM [SIZE]` and `fn 0xADDR [NAME]` lines, `#` comments.
pub fn parse_spec(text: &str) -> Vec<SpecLine> {
    text.lines()
        .filter_map(|l| {
            let l = l.split('#').next()?.trim();
            let mut w = l.split_whitespace();
            match (w.next()?, w.next()?) {
                ("mem", s) => Some(SpecLine::Mem {
                    sym: s.to_string(),
                    size: w.next().and_then(num).unwrap_or(4).clamp(1, 64) as u32,
                }),
                ("fn", a) => Some(SpecLine::Fn { addr: hex_u32(a)?, name: w.collect::<Vec<_>>().join(" ") }),
                _ => None,
            }
        })
        .collect()
}

/// Converts the x87 80-bit value at `b` (FSAVE register layout) to f64.
pub fn f80_to_f64(b: &[u8; 10]) -> f64 {
    let mant = u64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8]));
    let se = u16::from_le_bytes([b[8], b[9]]);
    let sign = if se & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from(se & 0x7FFF);
    if exp == 0 && mant == 0 {
        return 0.0 * sign;
    }
    if exp == 0x7FFF {
        return if mant << 1 == 0 { sign * f64::INFINITY } else { f64::NAN };
    }
    sign * (mant as f64) * 2f64.powi(exp - 16383 - 63)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::fake::FakeMem;

    #[test]
    fn rpc_table_matches_doc() {
        let t = rpc_table(RPC_IDS_MD);
        assert_eq!(t.len(), 141, "141 S→C registrations in rpc-ids.md");
        assert_eq!(rpc_handler(12, Build::B), Some(0x100192d0));
        assert_eq!(rpc_handler(14, Build::B), Some(0x10019500));
        assert_eq!(rpc_handler(153, Build::B), Some(0x10019140));
        assert_eq!(rpc_handler(70, Build::B), Some(0x100195b0));
        assert_eq!(rpc_handler(12, Build::A), Some(0x10019320));
        assert_eq!(t.iter().find(|r| r.id == 12).map(|r| r.purpose.as_str()), Some("SetPlayerPosition"));
        assert_eq!(build_of_stamp(0x5DD606CD), Some(Build::B));
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse_cmd("arm rpc:12"), Ok(EfxCmd::Arm(Target::Rpc(12), ArmOpts::default())));
        let Ok(EfxCmd::Arm(t, o)) = parse_cmd("arm samp.dll+0x192D0 budget=1000 over=0x40ea10,0x4087e0 heap=1") else {
            panic!()
        };
        assert_eq!(t, Target::ModRva("samp.dll".into(), 0x192d0));
        assert_eq!((o.budget, o.over.len(), o.heap), (1000, 2, true));
        assert_eq!(parse_target("0x100192d0"), Ok(Target::ClientVa(0x100192d0)));
        assert_eq!(parse_target("gta:0x5E4110"), Ok(Target::Gta(0x5E4110)));
        assert_eq!(parse_target("event:frame"), Ok(Target::Event(Event::Frame)));
        assert!(parse_target("event:nope").is_err());
        assert!(matches!(parse_cmd("watch C:\\x.txt frames=5"), Ok(EfxCmd::Watch(_, WatchOpts { frames: 5, .. }))));
        assert_eq!(parse_cmd("status"), Ok(EfxCmd::Status));
        assert!(parse_cmd("arm rpc:12 bogus=1").is_err());
        assert_eq!(Target::Rpc(14).to_string(), "rpc:14");
    }

    fn step(code: &[u8], cpu: &Cpu, m: &FakeMem) -> Step {
        let mut f = InstructionInfoFactory::new();
        analyse(code, cpu, m, &mut f).expect("decodes")
    }

    #[test]
    fn memory_operands() {
        let m = FakeMem::default();
        let cpu = Cpu {
            eax: 0x1000,
            ecx: 0x8000_0000,
            esi: 0x2000,
            edi: 0x3000,
            esp: 0x0019_F000,
            eip: 0x1000_1000,
            ..Cpu::default()
        };
        // mov dword [ecx+0x540], eax
        let s = step(&[0x89, 0x81, 0x40, 0x05, 0x00, 0x00], &cpu, &m);
        assert_eq!(s.acc, vec![Access { addr: 0x8000_0540, size: 4, read: false, write: true }]);
        assert_eq!((s.len, s.flow), (6, Flow::Next));
        // fstp dword [esi+eax*4+8]
        let s = step(&[0xD9, 0x5C, 0x86, 0x08], &cpu, &m);
        assert_eq!(s.acc[0], Access { addr: 0x2000 + 0x4000 + 8, size: 4, read: false, write: true });
        // add byte [0xB7CD98], 1 → read+write
        let s = step(&[0x80, 0x05, 0x98, 0xCD, 0xB7, 0x00, 0x01], &cpu, &m);
        assert_eq!(s.acc[0], Access { addr: 0xB7CD98, size: 1, read: true, write: true });
        assert!(s.sets_flags);
        // push eax → stack write at esp-4
        let s = step(&[0x50], &cpu, &m);
        assert_eq!(s.acc[0].addr, 0x0019_F000 - 4);
        // mov fs:[0], esp (SEH frame) uses the fs base
        let c2 = Cpu { fs_base: 0x7FFD_E000, ..cpu };
        let s = step(&[0x64, 0x89, 0x25, 0, 0, 0, 0], &c2, &m);
        assert_eq!(s.acc[0].addr, 0x7FFD_E000);
        // lea does not access memory
        assert!(step(&[0x8D, 0x41, 0x10], &cpu, &m).acc.is_empty());
    }

    #[test]
    fn control_flow_and_rep() {
        let mut m = FakeMem::default();
        m.set_u32(0x1004_0000, 0x0053_BEE0);
        let cpu = Cpu { eax: 0x5E4110, ecx: 3, esi: 0x2000, edi: 0x3000, eip: 0x1000_1000, ..Cpu::default() };
        // call rel32 → 0x10001000 + 5 + 0x10
        assert_eq!(step(&[0xE8, 0x10, 0, 0, 0], &cpu, &m).flow, Flow::Call(Some(0x1000_1015)));
        // call eax / call [0x10040000]
        assert_eq!(step(&[0xFF, 0xD0], &cpu, &m).flow, Flow::Call(Some(0x5E4110)));
        assert_eq!(step(&[0xFF, 0x15, 0, 0, 0x04, 0x10], &cpu, &m).flow, Flow::Call(Some(0x53BEE0)));
        // jne +2, ret 8, sete al
        let s = step(&[0x75, 0x02], &cpu, &m);
        assert_eq!((s.flow, s.cond_use), (Flow::Cond(0x1000_1004), true));
        assert_eq!(step(&[0xC2, 0x08, 0x00], &cpu, &m).flow, Flow::Ret);
        assert!(step(&[0x0F, 0x94, 0xC0], &cpu, &m).cond_use);
        // rep movsd / rep stosb; DF=1 is not emulated
        let s = step(&[0xF3, 0xA5], &cpu, &m);
        assert_eq!(s.rep, Some(RepString { stos: false, elem: 4, count: 3, dst: 0x3000, src: 0x2000 }));
        assert_eq!(step(&[0xF3, 0xAA], &cpu, &m).rep.map(|r| (r.stos, r.elem)), Some((true, 1)));
        assert_eq!(step(&[0xF3, 0xA5], &Cpu { eflags: 0x400, ..cpu }, &m).rep, None);
        // SSE movsd is not a string op
        assert_eq!(step(&[0xF2, 0x0F, 0x11, 0x01], &cpu, &m).rep, None);
        assert!(step(&[0x9D], &cpu, &m).writes_tf);
    }

    #[test]
    fn symbolic_addresses() {
        let mut m = FakeMem::default();
        let ped = 0x0900_0000;
        m.set_u32(crate::gta::CWORLD_PLAYERS, ped);
        m.set_u32(ped + 0x14, 0x0800_0054);
        m.set_u32(ped + PED_VEHICLE, 0);
        m.set_u32(ped + PED_INTELLIGENCE, 0x0A00_0294);
        m.set_u32(ped + PED_PLAYER_DATA, 0x0B00_0000);
        m.set_u32(0xB74490, 0x0700_0000); // ped pool object
        m.set_u32(0x0700_0000, 0x08F0_0000);
        m.set_u32(0x0700_0008, 140);
        m.set_u32(MATRIX_LIST_OBJECTS, 0x0800_0000);
        let w = World::read(&m, (0x400000, 0x1577000));
        assert_eq!(w.sym(ped + 0x540).as_deref(), Some("player_ped+0x540"));
        assert_eq!(w.sym(0x0800_0054 + 0x30).as_deref(), Some("player_ped.matrix+0x30"));
        assert_eq!(w.sym(0x0800_0000 + 2 * 0x54 + 4).as_deref(), Some("matrix#2+0x4"));
        assert_eq!(w.sym(0x08F0_0000 + 0x7C4 + 8).as_deref(), Some("ped#1+0x8"));
        assert_eq!(w.sym(0xB7CD98).as_deref(), Some("gta:0xb7cd98"));
        assert_eq!(w.sym(0x0C00_0000), None);
        for s in
            ["player_ped+0x540", "player_ped.matrix+0x30", "ped#1+0x8", "gta:0xb7cd98", "matrix#2+0x4", "player_data"]
        {
            let a = w.resolve(s).unwrap_or_else(|| panic!("{s}"));
            let back = w.sym(a).unwrap_or_default();
            assert!(back == s || back == format!("{s}+0x0"), "{s} -> {a:#x} -> {back}");
        }
    }

    #[test]
    fn nearest_vehicle() {
        let mut m = FakeMem::default();
        let ped = 0x0900_0000;
        m.set_u32(crate::gta::CWORLD_PLAYERS, ped);
        m.set_u32(ped + 0x14, 0x0800_0054);
        m.set_f32(0x0800_0054 + 0x30, 1510.0);
        m.set_f32(0x0800_0054 + 0x34, -1660.0);
        m.set_f32(0x0800_0054 + 0x38, 13.5);
        m.set_u32(ped + PED_VEHICLE, 0);
        m.set_u32(ped + PED_INTELLIGENCE, 0);
        m.set_u32(ped + PED_PLAYER_DATA, 0);
        m.set_u32(0xB74494, 0x0700_0000); // vehicle pool object
        m.set_u32(0x0700_0000, 0x0A00_0000);
        m.set_u32(0x0700_0004, 0x0700_1000);
        m.set_u32(0x0700_0008, 3);
        m.set(0x0700_1000, &[0x80, 0x00, 0x00]);
        for (i, x) in [(0u32, 0.0f32), (1, 1515.0), (2, 1600.0)] {
            let v = 0x0A00_0000 + i * 0xA18;
            let mat = 0x0800_0100 + i * 0x54;
            m.set_u32(v + 0x14, mat);
            m.set_f32(mat + 0x30, x);
            m.set_f32(mat + 0x34, -1665.0);
            m.set_f32(mat + 0x38, 13.6);
        }
        m.set_u32(MATRIX_LIST_OBJECTS, 0);
        let w = World::read(&m, (0x400000, 0x1577000));
        assert_eq!(w.sym(0x0A00_0000 + 0xA18 + 0x10).as_deref(), Some("near_veh+0x10"));
        assert_eq!(w.sym(0x0800_0100 + 0x54 + 0x30).as_deref(), Some("near_veh.matrix+0x30"));
        assert_eq!(w.sym(0x0A00_0000 + 2 * 0xA18).as_deref(), Some("veh#2+0x0"));
    }

    #[test]
    fn spec_and_floats() {
        let s = parse_spec("# x\nmem player_ped+0x540 4\nfn 0x5E4110 CPed::Teleport\nmem gta:0xb7cd98\nbogus");
        assert_eq!(s.len(), 3);
        assert_eq!(s[1], SpecLine::Fn { addr: 0x5E4110, name: "CPed::Teleport".into() });
        assert_eq!(s[2], SpecLine::Mem { sym: "gta:0xb7cd98".into(), size: 4 });
        // 1.0 and -2.5 in 80-bit form
        let one = [0, 0, 0, 0, 0, 0, 0, 0x80, 0xFF, 0x3F];
        assert_eq!(f80_to_f64(&one), 1.0);
        let m25 = [0, 0, 0, 0, 0, 0, 0, 0xA0, 0x00, 0xC0];
        assert_eq!(f80_to_f64(&m25), -2.5);
        assert_eq!(hex_bytes(&[0, 0xAB]), "00ab");
    }
}
