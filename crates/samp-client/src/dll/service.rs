//! Control-channel servicing: one answer per command (from the frame tick, or the init thread while the game is
//! not ticking).

use std::ffi::c_void;
use std::sync::atomic::Ordering;

use super::{CLIENT, CONTROL, TICKS, frame_hook_installed, request_quit};
use crate::control::Command;
use crate::game;
use crate::patch::{Memory, ProcessMemory};

pub(super) fn service_control(_from: &str) {
    let mut g = match CONTROL.try_lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let Some(server) = g.as_mut() else { return };
    let handled = server.poll(|cmd| match cmd {
        Command::Ping => "pong".into(),
        Command::State => match game::snapshot(&ProcessMemory) {
            Ok(s) => format!(
                "{} frame_hook={}",
                game::format_state(&s, TICKS.load(Ordering::SeqCst)),
                if frame_hook_installed() { "installed" } else { "absent" }
            ),
            Err(e) => format!("error {e}"),
        },
        Command::DevReset(n) => {
            crate::ui::force_device_lost(*n);
            format!("ok device reports lost for {n} checks")
        }
        Command::Peek { addr, len } => {
            if !readable(*addr, *len) {
                format!("error {addr:#x}+{len} is not readable")
            } else {
                let mut buf = vec![0u8; *len];
                match ProcessMemory.read(*addr, &mut buf) {
                    Ok(()) => format!("peek {addr:#x} {}", crate::patch::hex(&buf)),
                    Err(e) => format!("error {e}"),
                }
            }
        }
        Command::Net => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.status_line()).unwrap_or_else(|| "net phase=none".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Remote => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.remote_lines()).unwrap_or_else(|| "remote count=0 streamed=0".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Files => crate::filehook::report(),
        Command::Vehicles => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.vehicle_lines()).unwrap_or_else(|| "vehicles count=0".into()),
            Err(_) => "error client busy".into(),
        },
        Command::World => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.world_lines()).unwrap_or_else(|| "world count=0".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Behaviour => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.behaviour_line()).unwrap_or_else(|| "behaviour none".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Spawn => match CLIENT.try_lock() {
            Ok(mut g) => g.as_mut().map(|c| c.request_debug_spawn()).unwrap_or_else(|| "error no session".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Combat => crate::combathooks::status_line(),
        Command::Chat(n) => crate::ui::control_chat(*n),
        Command::Input => crate::ui::control_input(),
        Command::Ui => crate::ui::control_ui(),
        Command::Bubbles => crate::ui::control_bubbles(),
        Command::Synced => match CLIENT.try_lock() {
            Ok(g) => g.as_ref().map(|c| c.synced_lines()).unwrap_or_else(|| "synced count=0".into()),
            Err(_) => "error client busy".into(),
        },
        Command::Say(t) => crate::ui::control_say(t),
        Command::Audio => crate::audio::status(),
        Command::Quit => "bye".into(),
        Command::Unknown(c) => format!("error unknown command {c:?}"),
    });
    drop(g);
    if handled.contains(&Command::Quit) {
        request_quit();
    }
}

/// True if every page of `addr..addr+len` is committed and readable.
pub(crate) fn readable(addr: u32, len: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };
    let end = addr as usize + len;
    let mut p = addr as usize;
    while p < end {
        // SAFETY: VirtualQuery only inspects the address space.
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let n = unsafe { VirtualQuery(p as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) };
        if n == 0 || mbi.State != MEM_COMMIT || mbi.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
            return false;
        }
        p = mbi.BaseAddress as usize + mbi.RegionSize;
    }
    true
}
