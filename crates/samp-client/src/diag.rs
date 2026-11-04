//! Hang diagnostics (Windows only): log every thread's EIP and the
//! gta_sa.exe-range return-address candidates on its stack. Used when the
//! game stops ticking, because attaching winedbg to the WoW64 process is not
//! reliable.

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Debug::{CONTEXT, CONTEXT_FULL_X86, GetThreadContext};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThreadId, OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT,
    THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME,
};

/// Logs EIP/ESP and up to 24 code-looking stack words of every other thread.
pub fn dump_threads(why: &str) {
    crate::log!("diag: thread dump ({why})");
    // SAFETY: Toolhelp/thread APIs on our own process; every opened handle is
    // closed and every suspended thread resumed.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return;
        }
        let me = GetCurrentThreadId();
        let pid = GetCurrentProcessId();
        let mut te: THREADENTRY32 = std::mem::zeroed();
        te.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut ok = Thread32First(snap, &mut te);
        while ok != 0 {
            if te.th32OwnerProcessID == pid && te.th32ThreadID != me {
                dump_one(te.th32ThreadID);
            }
            ok = Thread32Next(snap, &mut te);
        }
        CloseHandle(snap);
    }
}

unsafe fn dump_one(tid: u32) {
    // SAFETY: see dump_threads.
    unsafe {
        let h = OpenThread(THREAD_GET_CONTEXT | THREAD_SUSPEND_RESUME | THREAD_QUERY_INFORMATION, 0, tid);
        if h.is_null() {
            return;
        }
        if SuspendThread(h) == u32::MAX {
            CloseHandle(h);
            return;
        }
        let mut ctx: CONTEXT = std::mem::zeroed();
        ctx.ContextFlags = CONTEXT_FULL_X86;
        let got = GetThreadContext(h, &mut ctx) != 0;
        let mut words = Vec::new();
        if got {
            let mut p = ctx.Esp;
            for _ in 0..2048 {
                if words.len() >= 24 || !crate::dll::readable(p, 4) {
                    break;
                }
                let v = std::ptr::read_unaligned(p as usize as *const u32);
                // gta_sa.exe code, or our own DLL (tagged "c+offset" from dump_threads).
                let ours = dump_threads as fn(&str) as usize as u32;
                if (0x401000..0x0160_0000).contains(&v) {
                    words.push(format!("{v:#x}"));
                } else if v.abs_diff(ours) < 0x0040_0000 {
                    words.push(format!("c{:+#x}", i64::from(v) - i64::from(ours)));
                }
                p += 4;
            }
        }
        ResumeThread(h);
        CloseHandle(h);
        if got {
            crate::log!(
                "diag: thread {tid:#x} eip={:#x} esp={:#x} ebp={:#x} stack code refs: {}",
                ctx.Eip,
                ctx.Esp,
                ctx.Ebp,
                words.join(" ")
            );
        }
    }
}
