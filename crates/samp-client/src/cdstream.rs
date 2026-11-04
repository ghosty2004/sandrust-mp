//! Fix for a lost-wakeup race in the game's streaming code (not a SA:MP patch).
//!
//! Observed under Wine (about 1 boot in 6 froze in LOADING_STARTED): the main
//! thread blocks forever in `CdStreamSync` (0x406460, body relocated to
//! 0x156CD80) inside `WaitForSingleObject(channel semaphore, INFINITE)`, while
//! `CdStreamThread` (0x406560) idles on its request semaphore. The sync side
//! tests "sectors left to read", then sets the channel's "locked" flag and
//! waits; the reader thread, on completion, clears "sectors left", releases
//! the channel semaphore only if "locked" is already set, then clears "in
//! use". If the reader's completion falls between the main thread's test and
//! its flag store, nobody ever releases the semaphore.
//!
//! The fix replaces the `call [WaitForSingleObject]` at 0x156CDAE with a call
//! to our own wait (same stdcall signature) that waits in short slices and
//! returns once the channel is idle ("sectors left" == 0 and "in use" == 0).
//! By then the reader has made its release decision, so a pending release is
//! consumed with a zero-timeout wait and the semaphore count stays balanced.
//!
//! Layout (named via gta-reversed `CdStreamInfo.h`: `CdStream`, 0x30 bytes):
//! +0x04 nSectorsToRead, +0x0D bLocked, +0x0E bInUse, +0x14 sync.hSemaphore.
//! `gCdStreams` pointer at 0x8E3FFC; `gStreamQueue.m_size` (= channel count
//! + 1, `CdStreamInit(5)` in CGame) at 0x8E3FF8.

use crate::patch::Memory;

/// `call dword ptr [0x858088]` (WaitForSingleObject via the IAT) in CdStreamSync.
pub const SYNC_WAIT_CALL: u32 = 0x156CDAE;
pub const SYNC_WAIT_CALL_ORIGINAL: [u8; 6] = [0xFF, 0x15, 0x88, 0x80, 0x85, 0x00];
/// `gCdStreams` (CdStream*), named via gta-reversed.
pub const G_CD_STREAMS: u32 = 0x8E3FFC;
/// `gStreamQueue.m_size`, named via gta-reversed (`Queue` at 0x8E3FEC).
pub const G_STREAM_QUEUE_SIZE: u32 = 0x8E3FF8;
pub const CHANNEL_SIZE: u32 = 0x30;

pub mod off {
    pub const SECTORS_TO_READ: u32 = 0x04;
    pub const LOCKED: u32 = 0x0D;
    pub const IN_USE: u32 = 0x0E;
    pub const SEMAPHORE: u32 = 0x14;
}

/// Address of the channel whose semaphore is `handle`, if any.
pub fn find_channel<M: Memory>(mem: &M, handle: u32) -> Option<u32> {
    let base = mem.read_u32(G_CD_STREAMS).ok()?;
    if base == 0 {
        return None;
    }
    let count = mem.read_i32(G_STREAM_QUEUE_SIZE).ok()?.saturating_sub(1).clamp(1, 32) as u32;
    (0..count).map(|i| base + i * CHANNEL_SIZE).find(|&ch| mem.read_u32(ch + off::SEMAPHORE).ok() == Some(handle))
}

/// True once the reader thread has finished the channel's request.
pub fn channel_idle<M: Memory>(mem: &M, ch: u32) -> bool {
    mem.read_u32(ch + off::SECTORS_TO_READ).ok() == Some(0) && mem.read_u8(ch + off::IN_USE).ok() == Some(0)
}

#[cfg(windows)]
pub mod hook {
    use super::*;
    use crate::patch::ProcessMemory;
    use std::sync::atomic::{AtomicU32, Ordering};
    use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    /// How many times the race was detected and resolved.
    pub static RESCUES: AtomicU32 = AtomicU32::new(0);

    /// Replacement for the `WaitForSingleObject(sem, INFINITE)` in CdStreamSync.
    /// The handle arrives as a plain 32-bit value (HANDLE is 32-bit on i686).
    pub extern "system" fn sync_wait(h: u32, timeout: u32) -> u32 {
        let handle = h as usize as HANDLE;
        let mut ch = None;
        let mut waited = 0u32;
        loop {
            // SAFETY: waiting on the game's own semaphore handle.
            let r = unsafe { WaitForSingleObject(handle, 5) };
            if r == WAIT_OBJECT_0 {
                return r;
            }
            waited = waited.saturating_add(5);
            if timeout != u32::MAX && waited >= timeout {
                return r;
            }
            if ch.is_none() {
                ch = find_channel(&ProcessMemory, h);
            }
            if let Some(c) = ch
                && channel_idle(&ProcessMemory, c)
            {
                // The reader is done; consume a release it may have issued
                // between our last wait and now.
                // SAFETY: as above.
                unsafe { WaitForSingleObject(handle, 0) };
                let n = RESCUES.fetch_add(1, Ordering::Relaxed) + 1;
                crate::log!("cdstream: lost wake-up on channel {c:#x} resolved (#{n})");
                return WAIT_OBJECT_0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::fake::FakeMemory;

    #[test]
    fn finds_channel_and_idle_state() {
        let mut m = FakeMemory::new(0x0080_0000, 0x0100_0000);
        let base = 0x0100_0000u32;
        m.set(G_CD_STREAMS, &base.to_le_bytes());
        m.set(G_STREAM_QUEUE_SIZE, &6i32.to_le_bytes());
        for i in 0..5u32 {
            m.set(base + i * CHANNEL_SIZE + off::SEMAPHORE, &(0x100 + i * 4).to_le_bytes());
        }
        assert_eq!(find_channel(&m, 0x108), Some(base + 2 * CHANNEL_SIZE));
        assert_eq!(find_channel(&m, 0x999), None);
        let ch = base + CHANNEL_SIZE;
        m.set(ch + off::SECTORS_TO_READ, &3u32.to_le_bytes());
        m.set(ch + off::IN_USE, &[1]);
        assert!(!channel_idle(&m, ch));
        m.set(ch + off::SECTORS_TO_READ, &0u32.to_le_bytes());
        assert!(!channel_idle(&m, ch));
        m.set(ch + off::IN_USE, &[0]);
        assert!(channel_idle(&m, ch));
    }
}
