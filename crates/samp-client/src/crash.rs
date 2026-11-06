//! Crash handler (FEAT-12): samp's unhandled-exception filter and crash dialog, without the upload.
//!
//! samp.dll build B (behaviour in our own words): at attach it installs a top-level exception filter (B 0x100C50E0 →
//! B 0x10060920). On an unhandled exception the filter keeps the exception context, minimises the game window and
//! shows the modal dialog resource 101 (B 0x10060820): caption "SA:MP An exception occured", a static text, a
//! read-only multi-line edit (id 1000) holding the report, "Close" (id 1001, default, focused) and "Report Crash"
//! (id 1002, uploads the report to a third-party site). The filter then returns EXCEPTION_EXECUTE_HANDLER, so the
//! process ends. The report (B 0x10060560 with the module list off for the dialog) is: a header with the client
//! version, the exception address, the client module base, the registers and EFLAGS; 0x280 bytes of stack from
//! ESP as dwords, four per line (B 0x10060240); the last script opcode, whether a script command was running
//! ("lDbg") and the model of the last entity samp's render hook saw; the game version; and, while the netgame
//! exists, "State Information" with `PlayerInFocus` and a list of remote player entries. The module list
//! (B 0x100603D0, "%s\tB: 0x%08X\tS: 0x%08X\t(%s)" per module from a ToolHelp snapshot) is only part of the
//! uploaded report.
//!
//! Decisions (docs/phase3/featb-notes.md, ROADMAP "Decisions made autonomously"):
//! * no "Report Crash" button and no upload of any kind; the full report (with the module list) is written to
//!   `samp-rs-crash.txt` next to `samp-rs.log` instead, and the static text says so;
//! * the header names this client ("samp-rs <version> (SA-MP 0.3.7-R4 compatible)");
//! * "LastRendObj" is 0 (no render hook records entity models here) and the remote player entries are left out
//!   (samp prints two fields of its remote-player object, +0x10A and +0x1E7, whose meaning is not identified).

/// Register values of the faulting context (x86 `CONTEXT`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Regs {
    pub eip: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
    pub esp: u32,
    pub eflags: u32,
}

/// Stack bytes dumped from ESP (B 0x10060596).
pub const STACK_DUMP: usize = 0x280;

/// What the report needs besides the registers.
#[derive(Debug, Default, Clone)]
pub struct ReportInput {
    pub client: String,
    pub base: u32,
    pub regs: Regs,
    /// Dwords from ESP (fewer when the stack ends earlier).
    pub stack: Vec<u32>,
    pub last_opcode: u32,
    pub in_command: bool,
    pub last_render_model: u16,
    pub game_version: &'static str,
    /// `PlayerInFocus` while the netgame exists.
    pub ped_context: Option<u8>,
    /// (name, base, size, path) — only for the file.
    pub modules: Vec<(String, u32, u32, String)>,
    /// Exception code, access kind (0 read, 1 write, 8 execute) and accessed address (file only).
    pub exception: Option<(u32, u32, u32)>,
    /// The last client events, oldest first (file only; [`note`]).
    pub breadcrumbs: Vec<Crumb>,
}

/// One breadcrumb of the game thread (see [`note`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Crumb {
    /// Milliseconds since the DLL attached.
    pub ms: u32,
    pub kind: u8,
    pub a: u16,
    pub b: u32,
}

/// Breadcrumb kinds.
pub mod crumb {
    /// An RPC handled on the game thread (a = RPC id).
    pub const RPC: u8 = 1;
    /// Object create (a = object id, b = model).
    pub const OBJ_CREATE: u8 = 2;
    /// Object destroy (a = object id, b = model).
    pub const OBJ_DESTROY: u8 = 3;
    /// MoveObject (a = object id).
    pub const OBJ_MOVE: u8 = 4;
    /// Object RW object created by our streaming (a = object id, b = model).
    pub const OBJ_RW: u8 = 5;
    /// Device Reset through the proxy (b = HRESULT).
    pub const DEVICE_RESET: u8 = 6;
    /// The device state changed (b = TestCooperativeLevel HRESULT; 0 = usable again).
    pub const DEVICE_LOST: u8 = 7;
    /// A frame tick (b = tick number), recorded every 64 ticks.
    pub const TICK: u8 = 8;
    /// Material texture / text applied (a = object id, b = slot).
    pub const OBJ_MATERIAL: u8 = 9;
    /// `CreateVertexBuffer` failed (b = length).
    pub const VB_CREATE_FAILED: u8 = 10;
}

const CRUMBS: usize = 64;
static CRUMB_TIME: [std::sync::atomic::AtomicU32; CRUMBS] = [const { std::sync::atomic::AtomicU32::new(0) }; CRUMBS];
static CRUMB_DATA: [std::sync::atomic::AtomicU64; CRUMBS] = [const { std::sync::atomic::AtomicU64::new(0) }; CRUMBS];
static CRUMB_NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Records a breadcrumb (lock-free, so the exception filter can read the ring whatever lock the crashing
/// thread held).
pub fn note(kind: u8, a: u16, b: u32) {
    use std::sync::atomic::Ordering::Relaxed;
    let ms = EPOCH.get_or_init(std::time::Instant::now).elapsed().as_millis() as u32;
    let i = CRUMB_NEXT.fetch_add(1, Relaxed) as usize % CRUMBS;
    CRUMB_TIME[i].store(ms, Relaxed);
    CRUMB_DATA[i].store((u64::from(kind) << 56) | (u64::from(a) << 32) | u64::from(b), Relaxed);
}

/// The breadcrumb ring, oldest first.
pub fn breadcrumbs() -> Vec<Crumb> {
    use std::sync::atomic::Ordering::Relaxed;
    let next = CRUMB_NEXT.load(Relaxed) as usize;
    let n = next.min(CRUMBS);
    (next - n..next)
        .map(|k| {
            let i = k % CRUMBS;
            let d = CRUMB_DATA[i].load(Relaxed);
            Crumb { ms: CRUMB_TIME[i].load(Relaxed), kind: (d >> 56) as u8, a: (d >> 32) as u16, b: d as u32 }
        })
        .collect()
}

fn crumb_text(c: &Crumb) -> String {
    let what = match c.kind {
        crumb::RPC => format!("rpc {}", c.a),
        crumb::OBJ_CREATE => format!("object create id={} model={}", c.a, c.b),
        crumb::OBJ_DESTROY => format!("object destroy id={} model={}", c.a, c.b),
        crumb::OBJ_MOVE => format!("object move id={}", c.a),
        crumb::OBJ_RW => format!("object rw id={} model={}", c.a, c.b as i32),
        crumb::DEVICE_RESET => format!("device reset hr={:#010x}", c.b),
        crumb::DEVICE_LOST => format!("device state hr={:#010x}", c.b),
        crumb::TICK => format!("frame {}", c.b),
        crumb::OBJ_MATERIAL => format!("object material id={} slot={}", c.a, c.b),
        crumb::VB_CREATE_FAILED => format!("CreateVertexBuffer failed, length {}", c.b),
        k => format!("kind {k} a={} b={:#x}", c.a, c.b),
    };
    format!("{:>10}.{:03} {what}", c.ms / 1000, c.ms % 1000)
}

/// "module+0xOFFSET" for an address inside one of `modules`.
pub fn locate(addr: u32, modules: &[(String, u32, u32, String)]) -> Option<String> {
    modules
        .iter()
        .find(|(_, base, size, _)| addr >= *base && addr - *base < *size)
        .map(|(name, base, _, _)| format!("{name}+{:#x}", addr - base))
}

/// The dialog text (`with_modules` = false) or the file text (true).
pub fn report(r: &ReportInput, with_modules: bool) -> String {
    let g = &r.regs;
    let mut s = format!(
        "{}\r\nException At Address: 0x{:08X}\r\nBase: 0x{:08X}\r\n\r\nRegisters:\r\n\
         EAX: 0x{:08X}\tEBX: 0x{:08X}\tECX: 0x{:08X}\tEDX: 0x{:08X}\r\n\
         ESI: 0x{:08X}\tEDI: 0x{:08X}\tEBP: 0x{:08X}\tESP: 0x{:08X}\r\nEFLAGS: 0x{:08X}\r\n\r\nStack:\r\n",
        r.client, g.eip, r.base, g.eax, g.ebx, g.ecx, g.edx, g.esi, g.edi, g.ebp, g.esp, g.eflags
    );
    for (i, row) in r.stack.chunks(4).enumerate() {
        let w = |k: usize| row.get(k).copied().unwrap_or(0);
        s += &format!("+{:04X}: 0x{:08X}   0x{:08X}   0x{:08X}   0x{:08X}\r\n", i * 16, w(0), w(1), w(2), w(3));
    }
    s += &format!(
        "\r\nSCM Op: 0x{:X}, lDbg: {} LastRendObj: {}\r\n",
        r.last_opcode,
        i32::from(r.in_command),
        r.last_render_model
    );
    s += &format!("\r\nGame Version: {}\r\n", r.game_version);
    if let Some(c) = r.ped_context {
        s += &format!("\r\nState Information: Ped Context: {c}\r\n");
    }
    if with_modules {
        s += "\r\nDiagnostics (samp-rs):\r\n";
        if let Some((code, kind, addr)) = r.exception {
            let access = match kind {
                0 => "read",
                1 => "write",
                8 => "execute",
                _ => "access",
            };
            s += &format!("Exception code: 0x{code:08X} ({access} of 0x{addr:08X})\r\n");
        }
        s += &format!("Fault at: {}\r\n", locate(g.eip, &r.modules).unwrap_or_else(|| "unknown module".into()));
        s += "Code addresses on the stack (return address candidates):\r\n";
        let mut shown = 0;
        for (i, w) in r.stack.iter().enumerate() {
            if let Some(at) = locate(*w, &r.modules).filter(|_| shown < 40) {
                s += &format!("  esp+{:03X}: 0x{w:08X} {at}\r\n", i * 4);
                shown += 1;
            }
        }
        s += "Last client events (oldest first, seconds since attach):\r\n";
        for c in &r.breadcrumbs {
            s += &format!("  {}\r\n", crumb_text(c));
        }
        s += "\r\nLoaded Modules:\r\n";
        for (name, base, size, path) in &r.modules {
            s += &format!("{name}\tB: 0x{base:08X}\tS: 0x{size:08X}\t({path})\r\n");
        }
    }
    s
}

/// The dialog's static text: samp's first sentence, then where the report was saved (no upload button).
pub fn static_text(file: Option<&str>) -> String {
    match file {
        Some(f) => format!(
            "SA:MP has crashed. The error report has been generated below.\r\n\r\nIt was also saved to {f}."
        ),
        None => "SA:MP has crashed. The error report has been generated below.".into(),
    }
}

/// Dialog caption, control ids and layout (dialog units) of samp's resource 101.
pub const CAPTION: &str = "SA:MP An exception occured";
pub const ID_TEXT: u16 = 1000;
pub const ID_CLOSE: u16 = 1001;
pub const REPORT_FILE: &str = "samp-rs-crash.txt";

/// A `DLGTEMPLATEEX` with samp's layout minus the report button: static text, report edit, Close.
pub fn dialog_template(static_text: &str) -> Vec<u8> {
    fn w16(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn w32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn wstr(v: &mut Vec<u8>, s: &str) {
        for c in s.encode_utf16() {
            w16(v, c);
        }
        w16(v, 0);
    }
    fn align4(v: &mut Vec<u8>) {
        while !v.len().is_multiple_of(4) {
            v.push(0);
        }
    }
    let mut v = Vec::new();
    w16(&mut v, 1);
    w16(&mut v, 0xFFFF);
    w32(&mut v, 0); // help id
    w32(&mut v, 0); // ex style
    w32(&mut v, 0x10CA_0AC0); // samp's dialog style
    w16(&mut v, 3);
    for x in [0i16, 0, 282, 180] {
        w16(&mut v, x as u16);
    }
    w16(&mut v, 0); // no menu
    w16(&mut v, 0); // default class
    wstr(&mut v, CAPTION);
    w16(&mut v, 8); // point size
    w16(&mut v, 0); // weight
    v.push(0); // italic
    v.push(0); // charset
    wstr(&mut v, "MS Sans Serif");
    // (style, rect, id, class ordinal, text)
    let items: [(u32, [i16; 4], u32, u16, &str); 3] = [
        (0x50B1_0804, [7, 44, 268, 110], u32::from(ID_TEXT), 0x81, ""),
        (0x5002_0000, [7, 7, 239, 36], u32::MAX, 0x82, static_text),
        (0x5001_0001, [212, 159, 63, 15], u32::from(ID_CLOSE), 0x80, "Close"),
    ];
    for (style, r, id, class, text) in items {
        align4(&mut v);
        w32(&mut v, 0);
        w32(&mut v, 0);
        w32(&mut v, style);
        for x in r {
            w16(&mut v, x as u16);
        }
        w32(&mut v, id);
        w16(&mut v, 0xFFFF);
        w16(&mut v, class);
        wstr(&mut v, text);
        w16(&mut v, 0); // no creation data
    }
    v
}

#[cfg(windows)]
pub use imp::install;

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows_sys::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, DialogBoxIndirectParamA, EndDialog, GetDlgItem, IDC_ARROW, LoadCursorW, SW_HIDE, SW_MINIMIZE,
        SetCursor, SetDlgItemTextA, ShowWindow, WM_COMMAND, WM_DESTROY, WM_INITDIALOG,
    };

    /// GTA main window handle (`RsGlobal.ps->window`).
    const GTA_HWND: u32 = 0xC97C1C;
    /// x86 CONTEXT offsets (winnt.h).
    const C_EDI: usize = 0x9C;
    const C_ESI: usize = 0xA0;
    const C_EBX: usize = 0xA4;
    const C_EDX: usize = 0xA8;
    const C_ECX: usize = 0xAC;
    const C_EAX: usize = 0xB0;
    const C_EBP: usize = 0xB4;
    const C_EIP: usize = 0xB8;
    const C_EFLAGS: usize = 0xC0;
    const C_ESP: usize = 0xC4;
    const EXECUTE_HANDLER: i32 = 1;

    /// The dialog text, read by the dialog procedure.
    static TEXT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

    /// At attach, like samp (B 0x100C5121).
    pub fn install() {
        // SAFETY: plain Win32 call; the filter is a valid `extern "system"` function for the process lifetime.
        unsafe { SetUnhandledExceptionFilter(Some(filter)) };
        crate::log!("crash: unhandled-exception filter installed");
    }

    fn gta_hwnd() -> HWND {
        // SAFETY: the game's HWND global (.bss of gta_sa.exe).
        unsafe { std::ptr::read_volatile(GTA_HWND as usize as *const usize) as HWND }
    }

    fn modules() -> Vec<(String, u32, u32, String)> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, MODULEENTRY32, Module32First, Module32Next, TH32CS_SNAPMODULE,
        };
        let cstr = |b: &[u8]| String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).into_owned();
        let mut out = Vec::new();
        // SAFETY: ToolHelp snapshot of our own process; the entry struct is sized before each call.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, 0);
            if snap == INVALID_HANDLE_VALUE {
                out.push(("-FailedCreate-".into(), 0, 0, String::new()));
                return out;
            }
            let mut e: MODULEENTRY32 = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<MODULEENTRY32>() as u32;
            let mut ok = Module32First(snap, &mut e) != 0;
            if !ok {
                out.push(("-FailedFirst-".into(), 0, 0, String::new()));
            }
            while ok {
                let name = cstr(&e.szModule.map(|c| c as u8));
                out.push((name, e.modBaseAddr as usize as u32, e.modBaseSize, cstr(&e.szExePath.map(|c| c as u8))));
                ok = Module32Next(snap, &mut e) != 0;
            }
            CloseHandle(snap);
        }
        out
    }

    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the pointers come from the system's exception dispatcher.
        let ctx = unsafe { (*info).ContextRecord as *const u8 };
        let reg = |o: usize| -> u32 {
            // SAFETY: inside the CONTEXT record.
            unsafe { std::ptr::read_unaligned(ctx.add(o).cast::<u32>()) }
        };
        let regs = Regs {
            eip: reg(C_EIP),
            eax: reg(C_EAX),
            ebx: reg(C_EBX),
            ecx: reg(C_ECX),
            edx: reg(C_EDX),
            esi: reg(C_ESI),
            edi: reg(C_EDI),
            ebp: reg(C_EBP),
            esp: reg(C_ESP),
            eflags: reg(C_EFLAGS),
        };
        let mut stack = Vec::new();
        for i in 0..STACK_DUMP / 4 {
            let a = regs.esp.wrapping_add(4 * i as u32);
            if !crate::dll::readable(a, 4) {
                break;
            }
            // SAFETY: committed, readable page (checked above).
            stack.push(unsafe { std::ptr::read_unaligned(a as usize as *const u32) });
        }
        let input = ReportInput {
            client: format!("samp-rs {} (SA-MP 0.3.7-R4 compatible)", env!("CARGO_PKG_VERSION")),
            base: crate::dll::module_base(),
            regs,
            stack,
            last_opcode: u32::from(crate::script::last_opcode()),
            in_command: crate::script::in_command(),
            last_render_model: 0,
            game_version: "US 1.0",
            // SAFETY: byte global of gta_sa.exe.
            ped_context: Some(unsafe { std::ptr::read_volatile(crate::game::PLAYER_IN_FOCUS as usize as *const u8) }),
            modules: modules(),
            exception: {
                // SAFETY: the record comes from the system's exception dispatcher.
                let rec = unsafe { (*info).ExceptionRecord };
                (!rec.is_null()).then(|| {
                    // SAFETY: as above.
                    let r = unsafe { &*rec };
                    let (kind, addr) = if r.NumberParameters >= 2 {
                        (r.ExceptionInformation[0] as u32, r.ExceptionInformation[1] as u32)
                    } else {
                        (u32::MAX, 0)
                    };
                    (r.ExceptionCode as u32, kind, addr)
                })
            },
            breadcrumbs: breadcrumbs(),
        };
        let full = report(&input, true);
        let path = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(REPORT_FILE)));
        let saved = path.as_ref().is_some_and(|p| std::fs::write(p, full.as_bytes()).is_ok());
        crate::log::write_unblocked(format_args!(
            "crash: exception at {:#x}; report {}:\n{full}",
            regs.eip,
            if saved { "saved" } else { "NOT saved" }
        ));
        let mut text = report(&input, false).into_bytes();
        text.push(0);
        *TEXT.lock().unwrap_or_else(|e| e.into_inner()) = text;
        let template = dialog_template(&static_text(saved.then_some(REPORT_FILE)));
        let hwnd = gta_hwnd();
        // SAFETY: plain Win32 calls; the template outlives the modal dialog.
        unsafe {
            ShowWindow(hwnd, SW_MINIMIZE);
            DialogBoxIndirectParamA(
                crate::dll::module_base() as usize as *mut c_void,
                template.as_ptr().cast(),
                hwnd,
                Some(dialog_proc),
                0,
            );
        }
        EXECUTE_HANDLER
    }

    unsafe extern "system" fn dialog_proc(dlg: HWND, msg: u32, w: WPARAM, _l: LPARAM) -> isize {
        // SAFETY (whole body): Win32 calls on the dialog and the game window of this process.
        unsafe {
            match msg {
                WM_INITDIALOG => {
                    let text = TEXT.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    SetDlgItemTextA(dlg, i32::from(ID_TEXT), text.as_ptr());
                    SetFocus(GetDlgItem(dlg, i32::from(ID_CLOSE)));
                    SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_ARROW));
                    // samp's own import resolves to its ShowCursor stub (cursorhook.rs): no counter change.
                    crate::cursorhook::show_cursor_stub(1);
                    0
                }
                WM_COMMAND if (w & 0xFFFF) as u16 == ID_CLOSE => {
                    EndDialog(dlg, 1);
                    0
                }
                WM_DESTROY => {
                    EndDialog(dlg, 1);
                    let hwnd = gta_hwnd();
                    ShowWindow(hwnd, SW_HIDE);
                    DestroyWindow(hwnd);
                    0
                }
                _ => 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_layout_matches_samp_fields() {
        let r = ReportInput {
            client: "samp-rs 0.1.0 (SA-MP 0.3.7-R4 compatible)".into(),
            base: 0x2F8_0000,
            regs: Regs { eip: 0x72B850, esp: 0x18_F000, eflags: 0x246, ..Regs::default() },
            stack: (0..6).collect(),
            last_opcode: 0x9BA,
            in_command: false,
            last_render_model: 0,
            game_version: "US 1.0",
            ped_context: Some(0),
            modules: vec![("gta_sa.exe".into(), 0x40_0000, 0x10_0000, "C:\\gta_sa.exe".into())],
            ..ReportInput::default()
        };
        let d = report(&r, false);
        assert!(d.starts_with("samp-rs 0.1.0 (SA-MP 0.3.7-R4 compatible)\r\nException At Address: 0x0072B850\r\nBase: 0x02F80000\r\n"));
        assert!(d.contains("EFLAGS: 0x00000246\r\n\r\nStack:\r\n+0000: 0x00000000   0x00000001   0x00000002   0x00000003\r\n+0010: 0x00000004   0x00000005   0x00000000   0x00000000\r\n"));
        assert!(d.contains("\r\nSCM Op: 0x9BA, lDbg: 0 LastRendObj: 0\r\n\r\nGame Version: US 1.0\r\n\r\nState Information: Ped Context: 0\r\n"));
        assert!(!d.contains("Loaded Modules"));
        assert!(report(&r, true).ends_with("\r\nLoaded Modules:\r\ngta_sa.exe\tB: 0x00400000\tS: 0x00100000\t(C:\\gta_sa.exe)\r\n"));
    }

    #[test]
    fn file_report_names_the_fault_module_and_the_last_events() {
        note(crumb::RPC, 44, 0);
        note(crumb::OBJ_CREATE, 736, 904);
        let crumbs = breadcrumbs();
        assert!(crumbs.len() >= 2);
        let last = &crumbs[crumbs.len() - 2..];
        assert_eq!((last[0].kind, last[0].a), (crumb::RPC, 44));
        assert_eq!((last[1].kind, last[1].a, last[1].b), (crumb::OBJ_CREATE, 736, 904));
        let modules = vec![
            ("gta_sa.exe".to_string(), 0x40_0000, 0x80_0000, "C:\\gta_sa.exe".to_string()),
            ("d3d9.dll".to_string(), 0x7600_0000, 0x10_0000, "C:\\d3d9.dll".to_string()),
        ];
        assert_eq!(locate(0x7F_5A3A, &modules).as_deref(), Some("gta_sa.exe+0x3f5a3a"));
        assert_eq!(locate(0x10, &modules), None);
        let r = ReportInput {
            regs: Regs { eip: 0x7F_5A3A, ..Regs::default() },
            stack: vec![0, 0x75_7C76, 0x7600_1234],
            modules,
            exception: Some((0xC000_0005, 0, 0)),
            breadcrumbs: last.to_vec(),
            ..ReportInput::default()
        };
        let f = report(&r, true);
        assert!(f.contains("Exception code: 0xC0000005 (read of 0x00000000)\r\n"));
        assert!(f.contains("Fault at: gta_sa.exe+0x3f5a3a\r\n"));
        assert!(f.contains("  esp+004: 0x00757C76 gta_sa.exe+0x357c76\r\n"));
        assert!(f.contains("  esp+008: 0x76001234 d3d9.dll+0x1234\r\n"));
        assert!(f.contains("rpc 44\r\n") && f.contains("object create id=736 model=904\r\n"));
        assert!(!report(&r, false).contains("Diagnostics"));
    }

    #[test]
    fn template_has_three_controls_and_no_report_button() {
        let t = dialog_template("x");
        assert_eq!(&t[..4], &[1, 0, 0xFF, 0xFF]);
        assert_eq!(u16::from_le_bytes([t[16], t[17]]), 3);
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        let has = |s: &str| t.windows(utf16(s).len()).any(|w| w == utf16(s).as_slice());
        assert!(has("Close") && has(CAPTION) && !has("Report"));
    }
}
