//! Fix for a lost-wakeup race in GTA's own streaming code (an addition; samp.dll has no such patch).
//!
//! `CdStreamSync` tests "sectors left to read" before it sets the channel's "locked" flag, while the
//! reader thread releases the channel semaphore only if "locked" is already set. A completion in between
//! leaves the main thread in `WaitForSingleObject(INFINITE)` forever (seen under Wine during loading).
//! [`hook::sync_wait`] replaces that wait: it waits in short slices and returns once the channel is idle.
//! Background: docs/phase3/m1-notes.md ("Extra fix, not in samp"); layout named via gta-reversed `CdStreamInfo.h`.

use crate::patch::Memory;

/// The `call [WaitForSingleObject]` (through the import table) inside `CdStreamSync`.
pub const SYNC_WAIT_CALL: u32 = 0x156CDAE;
/// Bytes of that call in the unmodified executable.
pub const SYNC_WAIT_CALL_ORIGINAL: [u8; 6] = [0xFF, 0x15, 0x88, 0x80, 0x85, 0x00];
/// `gCdStreams` (CdStream*), named via gta-reversed.
pub const G_CD_STREAMS: u32 = 0x8E3FFC;
/// `gStreamQueue.m_size` (channel count + 1), named via gta-reversed.
pub const G_STREAM_QUEUE_SIZE: u32 = 0x8E3FF8;
/// Size of one `CdStream` channel record.
pub const CHANNEL_SIZE: u32 = 0x30;

/// Field offsets inside a `CdStream` channel record.
pub mod off {
    /// `nSectorsToRead`.
    pub const SECTORS_TO_READ: u32 = 0x04;
    /// `bLocked`: the sync side is waiting.
    pub const LOCKED: u32 = 0x0D;
    /// `bInUse`: a request is in flight.
    pub const IN_USE: u32 = 0x0E;
    /// `sync.hSemaphore`.
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
