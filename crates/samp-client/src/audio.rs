//! Audio (milestone M9): the internet audio stream of RPC 41/42 played through the user's
//! own `bass.dll`, its per-frame radio calls and title line, and PlayerPlaySound (RPC 16).
//!
//! Specs: `docs/re/subsystems/ui-chat-audio.md` §41/§42 (static), `docs/phase3/trace-notes.md`
//! "Audio (TRACE-8)" (measured), `docs/phase3/m9-notes.md` (our notes and decisions).
//!
//! The original samp.dll imports bass.dll statically; so does this client (Windows build,
//! `raw-dylib`), with the same argument values, from the same threads, in the same order:
//! * boot (first frame with a D3D device, main thread): `BASS_Free`, `BASS_Init(-1, 44100, 0,
//!   0, 0)` and, only if that succeeded, user agent `SA-MP/0.3`, stream volume radio × 7000,
//!   playlists on, net timeout 10000, the `audioproxyoff` key (created with 0 when missing)
//!   → proxy off, EAX off (B 0x10066440);
//! * RPC 41 (main thread, B 0x10066920): stop the current stream and wait for its worker, stop
//!   a leftover handle, nothing more while GTA's radio volume is not > 0, store URL (256 bytes),
//!   position, distance and usePos, clear title / station, print `Audio stream: <url>` (type 4)
//!   unless `audiomsgoff`, start the worker;
//! * worker (B 0x100666c0): `StreamCreateURL(url, 0, BLOCK|STATUS|AUTOFREE)`, `ChannelPlay`,
//!   station from the ICY (else HTTP) tags, title from META (else OGG) tags, META / OGG_CHANGE
//!   syncs refreshing the title, then every 20 ms `StreamGetFilePosition(h, CONNECTED)` and the
//!   stream volume ([`stream_volume`]) until stopped, then `ChannelStop`;
//! * RPC 42 (B 0x100180a0 → 0x10066520 without waiting): raise the stop flag and free the
//!   stream at once; lost connection / server closed: the same with waiting;
//! * every frame while a stream "plays": `CAudioEngine::StartRadio(-1, 0)`, then
//!   `StopRadio(NULL, 0)` (B 0x10066a40);
//! * Present (B 0x10066a70): `"<title> - <station>"` in the small font at (15, H − 20)–(W,
//!   H + 10), DT_NOCLIP, 0x99FFFFFF, no outline, when both are known.
//!
//! The BASS 3D functions samp imports are only referenced from dead code (B 0x10066B90) and are
//! not imported here.

/// `BASS_CONFIG_*` options samp sets (public BASS 2.4 API).
pub mod config {
    pub const GVOL_STREAM: u32 = 5;
    pub const NET_TIMEOUT: u32 = 11;
    pub const NET_AGENT: u32 = 16;
    pub const NET_PROXY: u32 = 17;
    pub const NET_PLAYLIST: u32 = 21;
}

/// `BASS_TAG_*`.
pub mod tag {
    pub const OGG: u32 = 2;
    pub const HTTP: u32 = 3;
    pub const ICY: u32 = 4;
    pub const META: u32 = 5;
}

/// `BASS_SYNC_META` / `BASS_SYNC_OGG_CHANGE`.
pub const SYNC_META: u32 = 4;
pub const SYNC_OGG_CHANGE: u32 = 12;
/// `BASS_FILEPOS_CONNECTED`.
pub const FILEPOS_CONNECTED: u32 = 4;
/// `BASS_STREAM_BLOCK | BASS_STREAM_STATUS | BASS_STREAM_AUTOFREE` (measured 0x940000).
pub const STREAM_FLAGS: u32 = 0x0094_0000;
/// User agent (B 0x101027c8).
pub const NET_AGENT: &[u8] = b"SA-MP/0.3\0";
/// `BASS_Init` frequency.
pub const INIT_FREQ: u32 = 44100;
/// Net timeout set at boot (ms).
pub const NET_TIMEOUT_MS: u32 = 10000;
/// Worker volume loop period (B 0x100668e7 `Sleep(20)`; measured 479 iterations in 9.59 s).
pub const VOLUME_TICK_MS: u64 = 20;
/// Poll interval of the waiting stop (B 0x1006654b `Sleep(2)`).
pub const STOP_WAIT_MS: u64 = 2;
/// Stream volume scale without a position (B 0x100e9bb4) and with one (B 0x100e9be0).
pub const GLOBAL_SCALE: f32 = 7000.0;
pub const POSITIONAL_SCALE: f32 = 8000.0;
/// samp's URL / title / station buffers: 256 characters (+ NUL).
pub const TEXT_MAX: usize = 256;
/// Title line colour and DrawText flags (B 0x10066b04).
pub const TITLE_COLOUR: u32 = 0x99FF_FFFF;
pub const TITLE_FLAGS: u32 = 0x100;

/// MSVC `_ftol2`: truncation toward zero.
fn ftol(v: f64) -> i32 {
    v as i32
}

/// What the worker's volume step reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeInputs {
    /// GTA's pause menu is active (0xBA67A4 ≠ 0).
    pub menu_active: bool,
    /// The game window (0xC97C1C) is the foreground window.
    pub foreground: bool,
    /// GTA's radio volume (float at 0xB5FCC8).
    pub radio_volume: f32,
    pub use_pos: bool,
    pub distance: f32,
    /// Distance from the local ped to the stream position; `None` without a local ped matrix
    /// (samp then reads uninitialised stack memory; decision in m9-notes: volume 0).
    pub ped_distance: Option<f32>,
}

/// The `BASS_CONFIG_GVOL_STREAM` value of one worker step (B 0x10066823..0x100668e2):
/// 0 in the menu or without focus; usePos: `trunc(radio × 8000)` scaled by `1 − d/distance`
/// inside `distance`, else 0 (measured 7945 at 0.33 m, 4799 at 20 m, 0 at 60 m of 50 m);
/// otherwise `trunc(radio × 7000)` (measured 7000).
pub fn stream_volume(i: &VolumeInputs) -> u32 {
    if i.menu_active || !i.foreground {
        return 0;
    }
    if !i.use_pos {
        return ftol(f64::from(i.radio_volume) * f64::from(GLOBAL_SCALE)) as u32;
    }
    let base = ftol(f64::from(i.radio_volume) * f64::from(POSITIONAL_SCALE));
    match i.ped_distance {
        Some(d) if d < i.distance => ftol((1.0 - f64::from(d) / f64::from(i.distance)) * f64::from(base)) as u32,
        _ => 0,
    }
}

/// `_snprintf(buf, 256, "%s", …)` into a zeroed 257-byte buffer: at most 256 bytes.
fn clip(b: &[u8]) -> Vec<u8> {
    b[..b.len().min(TEXT_MAX)].to_vec()
}

/// Case-insensitive prefix test (`_strnicmp(s, prefix, len) == 0`).
fn has_prefix(s: &[u8], prefix: &[u8]) -> bool {
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Station name and URL from an ICY / HTTP header list (B 0x1006674a..0x100667c3): every line
/// starting with `icy-name:` / `icy-url:` (case-insensitive) sets the value to the rest of the
/// line, untrimmed; a later line wins.
pub fn station_from_headers<'a>(lines: impl IntoIterator<Item = &'a [u8]>) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let (mut name, mut url) = (None, None);
    for l in lines {
        if has_prefix(l, b"icy-name:") {
            name = Some(clip(&l[9..]));
        }
        if has_prefix(l, b"icy-url:") {
            url = Some(clip(&l[8..]));
        }
    }
    (name, url)
}

/// Title from a META tag (B 0x10066580): the text after `StreamTitle='` up to (not including)
/// the character before the first `;`. `None` if the tag has no `StreamTitle='`.
/// Without any `;` samp writes before a null pointer (crash); we keep the whole rest
/// (decision in m9-notes).
pub fn title_from_meta(meta: &[u8]) -> Option<Vec<u8>> {
    const KEY: &[u8] = b"StreamTitle='";
    let at = meta.windows(KEY.len()).position(|w| w == KEY)? + KEY.len();
    let rest = &meta[at..];
    let end = rest.iter().position(|&b| b == b';').map(|i| i.saturating_sub(1)).unwrap_or(rest.len());
    Some(clip(&rest[..end]))
}

/// Title from OGG comments (B 0x100665f0..0x1006669f): the last `artist=` and `title=` values
/// (case-insensitive keys); `"artist - title"` (MSVC prints `(null)` for a missing title),
/// else the title alone, else nothing.
pub fn title_from_ogg<'a>(comments: impl IntoIterator<Item = &'a [u8]>) -> Option<Vec<u8>> {
    let (mut artist, mut title): (Option<&[u8]>, Option<&[u8]>) = (None, None);
    for c in comments {
        if has_prefix(c, b"artist=") {
            artist = Some(&c[7..]);
        }
        if has_prefix(c, b"title=") {
            title = Some(&c[6..]);
        }
    }
    match (artist, title) {
        (Some(a), t) => {
            let mut s = a.to_vec();
            s.extend_from_slice(b" - ");
            s.extend_from_slice(t.unwrap_or(b"(null)"));
            Some(clip(&s))
        }
        (None, Some(t)) => Some(clip(t)),
        (None, None) => None,
    }
}

/// The title line (B 0x10066a70): `"<title> - <station>"` when both are non-empty.
pub fn title_line(title: &[u8], station: &[u8]) -> Option<Vec<u8>> {
    if title.is_empty() || station.is_empty() {
        return None;
    }
    let mut s = title.to_vec();
    s.extend_from_slice(b" - ");
    s.extend_from_slice(station);
    Some(clip(&s))
}

/// Splits a BASS tag list (consecutive NUL-terminated strings ending with an empty one).
pub fn split_tag_list(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    for s in bytes.split(|&b| b == 0) {
        if s.is_empty() {
            break;
        }
        out.push(s);
    }
    out
}

/// PlayerPlaySound (RPC 16) on samp's game audio object (B 0x100a22d0) and its per-frame poll
/// (B 0x100a21e0). Script commands for the host to run, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SoundCmd {
    /// `CAEAmbienceTrackManager::StopSpecialMissionAmbienceTrack()` (0x4D6D60).
    StopMissionTrack,
    /// `CAEAmbienceTrackManager::PlaySpecialMissionAmbienceTrack(id)` (0x4D6D50).
    PlayMissionTrack(i32),
    /// `0x018C ADD_ONE_OFF_SOUND(x, y, z, id)`.
    OneOff([f32; 3], i32),
    /// `0x040D CLEAR_MISSION_AUDIO(1)`.
    ClearMissionAudio,
    /// `0x03CF LOAD_MISSION_AUDIO(1, id)`.
    LoadMissionAudio(i32),
    /// `0x03D7 SET_MISSION_AUDIO_POSITION(1, x, y, z)`.
    MissionAudioPosition([f32; 3]),
    /// `0x03D0 HAS_MISSION_AUDIO_LOADED(1)`: the host reports the result with
    /// [`MissionSound::loaded`].
    PollMissionAudio,
    /// `0x03D1 PLAY_MISSION_AUDIO(1)`.
    PlayMissionAudio,
}

/// samp's game audio object: mission-audio state (u32 +0: 0 idle, 1 loading, 2 played) and the
/// flag byte +4 set by sound id 1 (its reader B 0x100a23b0 has no caller in the paths traced).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MissionSound {
    pub state: u32,
    pub flag: bool,
}

impl MissionSound {
    /// RPC 16 / samp's own sounds (class selection 1052/1053).
    pub fn play(&mut self, id: i32, pos: [f32; 3]) -> Vec<SoundCmd> {
        match id {
            0 => {
                let mut v = Vec::new();
                if self.state != 0 {
                    v.push(SoundCmd::ClearMissionAudio);
                    self.state = 0;
                }
                v.push(SoundCmd::StopMissionTrack);
                self.flag = false;
                v
            }
            1 => {
                self.flag = true;
                Vec::new()
            }
            i if i < 1000 => vec![SoundCmd::PlayMissionTrack(i)],
            i if i < 2000 => vec![SoundCmd::OneOff(pos, i)],
            i => {
                self.state = 1;
                vec![SoundCmd::ClearMissionAudio, SoundCmd::LoadMissionAudio(i), SoundCmd::MissionAudioPosition(pos)]
            }
        }
    }

    /// Every frame: poll while loading.
    pub fn frame(&self) -> Option<SoundCmd> {
        (self.state == 1).then_some(SoundCmd::PollMissionAudio)
    }

    /// The poll's result: loaded → play once, state 2.
    pub fn loaded(&mut self, loaded: bool) -> Option<SoundCmd> {
        if self.state == 1 && loaded {
            self.state = 2;
            return Some(SoundCmd::PlayMissionAudio);
        }
        None
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::patch::{Memory, ProcessMemory};
    use crate::script::{Arg, Runner};

    type SyncProc = extern "system" fn(u32, u32, u32, *mut c_void);

    // The user's bass.dll (BASS 2.4, shipped with SA:MP in the game directory), imported
    // statically like samp.dll does; nothing is redistributed.
    #[link(name = "bass", kind = "raw-dylib", import_name_type = "undecorated")]
    unsafe extern "system" {
        fn BASS_SetEAXParameters(env: i32, vol: f32, decay: f32, damp: f32) -> i32;
        fn BASS_SetConfig(option: u32, value: u32) -> i32;
        fn BASS_SetConfigPtr(option: u32, value: *const c_void) -> i32;
        fn BASS_Init(device: i32, freq: u32, flags: u32, win: *mut c_void, clsid: *const c_void) -> i32;
        fn BASS_Free() -> i32;
        fn BASS_StreamFree(handle: u32) -> i32;
        fn BASS_ChannelGetTags(handle: u32, tags: u32) -> *const u8;
        fn BASS_StreamGetFilePosition(handle: u32, mode: u32) -> u64;
        fn BASS_ChannelSetSync(handle: u32, kind: u32, param: u64, proc_: SyncProc, user: *mut c_void) -> u32;
        fn BASS_ChannelPlay(handle: u32, restart: i32) -> i32;
        fn BASS_StreamCreateURL(
            url: *const u8,
            offset: u32,
            flags: u32,
            proc_: *const c_void,
            user: *mut c_void,
        ) -> u32;
        fn BASS_ChannelStop(handle: u32) -> i32;
    }

    /// GTA addresses (1.0 US).
    mod gta {
        /// `CAudioEngine` instance `AudioEngine`.
        pub const AUDIO_ENGINE: u32 = 0xB6BC90;
        /// `CAudioEngine::StartRadio(int8 station, int8 bass)` (thiscall).
        pub const START_RADIO: u32 = 0x507DC0;
        /// `CAudioEngine::StopRadio(tVehicleAudioSettings*, bool)` (thiscall).
        pub const STOP_RADIO: u32 = 0x506F70;
        /// `CAEAmbienceTrackManager` instance and its two special-mission-track methods.
        pub const AMBIENCE_TRACK_MANAGER: u32 = 0x8AC15C;
        pub const PLAY_MISSION_TRACK: u32 = 0x4D6D50;
        pub const STOP_MISSION_TRACK: u32 = 0x4D6D60;
        /// Radio volume float read by samp's game wrapper (B 0x100a2290).
        pub const RADIO_VOLUME: u32 = 0xB5FCC8;
        /// `FrontEndMenuManager.m_bMenuActive`.
        pub const MENU_ACTIVE: u32 = 0xBA67A4;
        /// The game window handle (`RsGlobal.ps->window`).
        pub const WINDOW: u32 = 0xC97C1C;
        /// `CWorld::Players[0].m_pPed`; entity `+0x14` matrix, position at matrix `+0x30`.
        pub const PLAYER_PED: u32 = 0xB7CD98;
    }

    const OP_ADD_ONE_OFF_SOUND: u16 = 0x018C;
    const OP_LOAD_MISSION_AUDIO: u16 = 0x03CF;
    const OP_HAS_MISSION_AUDIO_LOADED: u16 = 0x03D0;
    const OP_PLAY_MISSION_AUDIO: u16 = 0x03D1;
    const OP_SET_MISSION_AUDIO_POSITION: u16 = 0x03D7;
    const OP_CLEAR_MISSION_AUDIO: u16 = 0x040D;

    /// The audio object exists and `BASS_Init` succeeded (B: object byte +0).
    static READY: AtomicBool = AtomicBool::new(false);
    /// A worker runs (B 0x1012e7b8).
    static PLAYING: AtomicBool = AtomicBool::new(false);
    /// Stop request for the worker (B 0x101027d2).
    static STOP: AtomicBool = AtomicBool::new(false);
    /// The stream handle (B 0x1012e7b4).
    static HANDLE: AtomicU32 = AtomicU32::new(0);

    /// URL, position and the texts (B 0x1012e398.. buffers).
    #[derive(Default)]
    struct Stream {
        url: Vec<u8>,
        pos: [f32; 3],
        distance: f32,
        use_pos: bool,
        title: Vec<u8>,
        station: Vec<u8>,
        station_url: Vec<u8>,
    }

    static STREAM: Mutex<Option<Stream>> = Mutex::new(None);
    static SOUND: Mutex<MissionSound> = Mutex::new(MissionSound { state: 0, flag: false });

    fn with_stream<R>(f: impl FnOnce(&mut Stream) -> R) -> R {
        let mut g = STREAM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(Stream::default))
    }

    fn sleep_ms(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    fn radio_volume() -> f32 {
        ProcessMemory.read_f32(gta::RADIO_VOLUME).unwrap_or(0.0)
    }

    /// Reads a C string BASS returned (NULL → `None`); tag lists keep their inner NULs.
    fn tag_bytes(p: *const u8, list: bool) -> Option<Vec<u8>> {
        if p.is_null() {
            return None;
        }
        let mut out = Vec::new();
        let mut i = 0usize;
        loop {
            // SAFETY: BASS returns a NUL-terminated string (or a list ending with an empty
            // string) that stays valid while the stream exists; reads stop at the terminator.
            let b = unsafe { *p.add(i) };
            if b == 0 && (!list || out.last().is_none_or(|&l| l == 0)) {
                break;
            }
            out.push(b);
            i += 1;
            if i > 1 << 16 {
                break;
            }
        }
        Some(out)
    }

    /// Boot (A 0x10066480), main thread; returns whether BASS is ready. `proxy_key` runs only after
    /// a successful `BASS_Init`: it creates `audioproxyoff` (0) in sa-mp.cfg when missing and returns
    /// its value, like samp's set-up.
    pub fn init(proxy_key: impl FnOnce() -> bool) -> bool {
        HANDLE.store(0, Ordering::SeqCst);
        // SAFETY: plain BASS calls with the documented signatures and constant arguments.
        unsafe {
            BASS_Free();
            if BASS_Init(-1, INIT_FREQ, 0, std::ptr::null_mut(), std::ptr::null()) == 0 {
                crate::log!("audio: BASS_Init failed; audio streams disabled");
                return false;
            }
            BASS_SetConfigPtr(config::NET_AGENT, NET_AGENT.as_ptr().cast());
            BASS_SetConfig(config::GVOL_STREAM, ftol(f64::from(radio_volume()) * f64::from(GLOBAL_SCALE)) as u32);
            BASS_SetConfig(config::NET_PLAYLIST, 1);
            BASS_SetConfig(config::NET_TIMEOUT, NET_TIMEOUT_MS);
            if proxy_key() {
                BASS_SetConfigPtr(config::NET_PROXY, std::ptr::null());
            }
            BASS_SetEAXParameters(-1, 0.0, -1.0, -1.0);
        }
        READY.store(true, Ordering::SeqCst);
        crate::log!("audio: BASS ready");
        true
    }

    pub fn ready() -> bool {
        READY.load(Ordering::SeqCst)
    }

    /// Stop (B 0x10066520): stop flag, optionally wait for the worker, free the stream.
    pub fn stop(wait: bool) -> bool {
        if !READY.load(Ordering::SeqCst) || !PLAYING.load(Ordering::SeqCst) {
            return false;
        }
        STOP.store(true, Ordering::SeqCst);
        if wait {
            while PLAYING.load(Ordering::SeqCst) {
                sleep_ms(STOP_WAIT_MS);
            }
        }
        // SAFETY: BASS call on a stream handle (0 is rejected by BASS).
        unsafe { BASS_StreamFree(HANDLE.load(Ordering::SeqCst)) };
        HANDLE.store(0, Ordering::SeqCst);
        true
    }

    /// RPC 41 (B 0x10066920). Returns the chat line to print (type 4), if any.
    pub fn play(url: &[u8], pos: [f32; 3], distance: f32, use_pos: bool, messages_off: bool) -> Option<Vec<u8>> {
        if !READY.load(Ordering::SeqCst) {
            return None;
        }
        stop(true);
        let h = HANDLE.load(Ordering::SeqCst);
        if h != 0 {
            // SAFETY: BASS call on our stream handle.
            unsafe { BASS_ChannelStop(h) };
            HANDLE.store(0, Ordering::SeqCst);
        }
        if radio_volume().partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return None;
        }
        let url = url[..url.iter().position(|&b| b == 0).unwrap_or(url.len())].to_vec();
        let url = clip(&url);
        with_stream(|s| {
            *s = Stream { url: url.clone(), pos, distance, use_pos, ..Stream::default() };
        });
        let line = (!messages_off).then(|| {
            let mut l = b"Audio stream: ".to_vec();
            l.extend_from_slice(&url);
            l
        });
        STOP.store(false, Ordering::SeqCst);
        if let Err(e) = std::thread::Builder::new().name("samp-audio".into()).spawn(worker) {
            crate::log!("audio: worker thread not started: {e}");
        }
        line
    }

    /// RPC 41 handler (B 0x1001d370): ignored without the audio object; the line goes to chat.
    pub fn on_play_rpc(a: &samp_proto::rpcs::ui::PlayAudioStreamForPlayer) {
        let p = [a.position.x, a.position.y, a.position.z];
        crate::log!(
            "audio: RPC 41 {:?} pos {p:?} distance {} use_pos {}",
            String::from_utf8_lossy(&a.url),
            a.distance,
            a.use_pos
        );
        if let Some(line) = play(&a.url, p, a.distance, a.use_pos != 0, crate::ui::audio_messages_off()) {
            crate::ui::print_info(&line);
        }
    }

    /// Title refresh (B 0x10066580), worker thread and BASS sync callbacks.
    fn update_title() {
        let h = HANDLE.load(Ordering::SeqCst);
        with_stream(|s| s.title.clear());
        // SAFETY: BASS tag queries on our handle; the returned memory is read right away.
        let meta = tag_bytes(unsafe { BASS_ChannelGetTags(h, tag::META) }, false);
        let title = match meta {
            Some(m) => title_from_meta(&m),
            None => {
                // SAFETY: as above.
                let ogg = tag_bytes(unsafe { BASS_ChannelGetTags(h, tag::OGG) }, true);
                ogg.and_then(|o| title_from_ogg(split_tag_list(&o)))
            }
        };
        if let Some(t) = title {
            with_stream(|s| s.title = t);
        }
    }

    extern "system" fn sync_proc(_sync: u32, _channel: u32, _data: u32, _user: *mut c_void) {
        let _ = std::panic::catch_unwind(update_title);
    }

    fn ped_distance(to: [f32; 3]) -> Option<f32> {
        let m = ProcessMemory;
        let ped = m.read_u32(gta::PLAYER_PED).ok().filter(|&p| p != 0)?;
        let mat = m.read_u32(ped + 0x14).ok().filter(|&p| p != 0)?;
        let p = [m.read_f32(mat + 0x30).ok()?, m.read_f32(mat + 0x34).ok()?, m.read_f32(mat + 0x38).ok()?];
        let d = [p[0] - to[0], p[1] - to[1], p[2] - to[2]];
        Some((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
    }

    fn foreground() -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        // SAFETY: plain Win32 query.
        let fg = unsafe { GetForegroundWindow() } as usize as u32;
        ProcessMemory.read_u32(gta::WINDOW).ok() == Some(fg)
    }

    /// The stream thread (B 0x100666c0).
    fn worker() {
        PLAYING.store(true, Ordering::SeqCst);
        let old = HANDLE.load(Ordering::SeqCst);
        // SAFETY (whole function): BASS calls with our own handle and NUL-terminated strings.
        unsafe {
            if old != 0 {
                BASS_ChannelStop(old);
                HANDLE.store(0, Ordering::SeqCst);
            }
            let mut url = with_stream(|s| s.url.clone());
            url.push(0);
            let h = BASS_StreamCreateURL(url.as_ptr(), 0, STREAM_FLAGS, std::ptr::null(), std::ptr::null_mut());
            HANDLE.store(h, Ordering::SeqCst);
            BASS_ChannelPlay(h, 0);
            with_stream(|s| {
                s.station.clear();
                s.station_url.clear();
            });
            let h = HANDLE.load(Ordering::SeqCst);
            let mut headers = tag_bytes(BASS_ChannelGetTags(h, tag::ICY), true);
            if headers.is_none() {
                headers = tag_bytes(BASS_ChannelGetTags(HANDLE.load(Ordering::SeqCst), tag::HTTP), true);
            }
            if let Some(hd) = headers {
                let (name, surl) = station_from_headers(split_tag_list(&hd));
                with_stream(|s| {
                    if let Some(n) = name {
                        s.station = n;
                    }
                    if let Some(u) = surl {
                        s.station_url = u;
                    }
                });
            }
            update_title();
            BASS_ChannelSetSync(HANDLE.load(Ordering::SeqCst), SYNC_META, 0, sync_proc, std::ptr::null_mut());
            BASS_ChannelSetSync(HANDLE.load(Ordering::SeqCst), SYNC_OGG_CHANGE, 0, sync_proc, std::ptr::null_mut());
            while !STOP.load(Ordering::SeqCst) {
                BASS_StreamGetFilePosition(HANDLE.load(Ordering::SeqCst), FILEPOS_CONNECTED);
                let (pos, distance, use_pos) = with_stream(|s| (s.pos, s.distance, s.use_pos));
                let menu_active = ProcessMemory.read_u32(gta::MENU_ACTIVE).unwrap_or(0) != 0;
                let fg = !menu_active && foreground();
                let v = stream_volume(&VolumeInputs {
                    menu_active,
                    foreground: fg,
                    radio_volume: radio_volume(),
                    use_pos,
                    distance,
                    ped_distance: if use_pos && fg { ped_distance(pos) } else { None },
                });
                BASS_SetConfig(config::GVOL_STREAM, v);
                sleep_ms(VOLUME_TICK_MS);
            }
            BASS_ChannelStop(HANDLE.load(Ordering::SeqCst));
        }
        HANDLE.store(0, Ordering::SeqCst);
        PLAYING.store(false, Ordering::SeqCst);
    }

    /// Present step 4b (B 0x10066a70): the title line, if any.
    pub fn title_text() -> Option<Vec<u8>> {
        if !READY.load(Ordering::SeqCst) || !PLAYING.load(Ordering::SeqCst) {
            return None;
        }
        with_stream(|s| title_line(&s.title, &s.station))
    }

    /// Control channel `audio`.
    pub fn status() -> String {
        let playing = PLAYING.load(Ordering::SeqCst);
        let (url, title, station, station_url, use_pos, distance) = with_stream(|s| {
            (s.url.clone(), s.title.clone(), s.station.clone(), s.station_url.clone(), s.use_pos, s.distance)
        });
        let sound = *SOUND.lock().unwrap_or_else(|e| e.into_inner());
        let q = |b: &[u8]| crate::ui::core::quote(b);
        format!(
            "audio ready={} playing={} handle={:#x} url={} use_pos={} distance={distance} title={} station={} station_url={} mission_state={} sound_flag={}",
            u8::from(READY.load(Ordering::SeqCst)),
            u8::from(playing),
            HANDLE.load(Ordering::SeqCst),
            q(&url),
            u8::from(use_pos),
            q(&title),
            q(&station),
            q(&station_url),
            sound.state,
            u8::from(sound.flag)
        )
    }

    fn run_sound(runner: &mut Runner, cmd: SoundCmd) -> bool {
        match cmd {
            SoundCmd::StopMissionTrack => {
                // SAFETY: CAEAmbienceTrackManager::StopSpecialMissionAmbienceTrack (thiscall, no args).
                let f: extern "thiscall" fn(u32) = unsafe { std::mem::transmute(gta::STOP_MISSION_TRACK as usize) };
                f(gta::AMBIENCE_TRACK_MANAGER);
                true
            }
            SoundCmd::PlayMissionTrack(id) => {
                // SAFETY: CAEAmbienceTrackManager::PlaySpecialMissionAmbienceTrack(int) (thiscall).
                let f: extern "thiscall" fn(u32, i32) =
                    unsafe { std::mem::transmute(gta::PLAY_MISSION_TRACK as usize) };
                f(gta::AMBIENCE_TRACK_MANAGER, id);
                true
            }
            SoundCmd::OneOff(p, id) => {
                runner.run(OP_ADD_ONE_OFF_SOUND, &[Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2]), Arg::Int(id)])
            }
            SoundCmd::ClearMissionAudio => runner.run(OP_CLEAR_MISSION_AUDIO, &[Arg::Int(1)]),
            SoundCmd::LoadMissionAudio(id) => runner.run(OP_LOAD_MISSION_AUDIO, &[Arg::Int(1), Arg::Int(id)]),
            SoundCmd::MissionAudioPosition(p) => runner.run(
                OP_SET_MISSION_AUDIO_POSITION,
                &[Arg::Int(1), Arg::Float(p[0]), Arg::Float(p[1]), Arg::Float(p[2])],
            ),
            SoundCmd::PollMissionAudio => runner.run(OP_HAS_MISSION_AUDIO_LOADED, &[Arg::Int(1)]),
            SoundCmd::PlayMissionAudio => runner.run(OP_PLAY_MISSION_AUDIO, &[Arg::Int(1)]),
        }
    }

    /// RPC 16 PlayerPlaySound.
    pub fn play_sound(runner: &mut Runner, id: i32, pos: [f32; 3]) {
        let cmds = SOUND.lock().unwrap_or_else(|e| e.into_inner()).play(id, pos);
        for c in cmds {
            run_sound(runner, c);
        }
    }

    thread_local! {
        static RUNNER: std::cell::RefCell<Option<Runner>> = const { std::cell::RefCell::new(None) };
    }

    /// [`frame`] with the audio module's own script runner (main thread).
    pub fn frame_tick() {
        RUNNER.with(|r| {
            let mut r = r.borrow_mut();
            frame(r.get_or_insert_with(Runner::new));
        });
    }

    /// Per frame after the netgame tick (B 0x100c4ff0..0x100c500a): the radio calls while a
    /// stream plays, then the mission-audio poll.
    pub fn frame(runner: &mut Runner) {
        if READY.load(Ordering::SeqCst) && PLAYING.load(Ordering::SeqCst) {
            // SAFETY: CAudioEngine methods (thiscall) on the game's AudioEngine, main thread.
            unsafe {
                let start: extern "thiscall" fn(u32, i32, i32) = std::mem::transmute(gta::START_RADIO as usize);
                start(gta::AUDIO_ENGINE, -1, 0);
                let stop: extern "thiscall" fn(u32, u32, i32) = std::mem::transmute(gta::STOP_RADIO as usize);
                stop(gta::AUDIO_ENGINE, 0, 0);
            }
        }
        let poll = SOUND.lock().unwrap_or_else(|e| e.into_inner()).frame();
        if let Some(cmd) = poll {
            let loaded = run_sound(runner, cmd);
            let next = SOUND.lock().unwrap_or_else(|e| e.into_inner()).loaded(loaded);
            if let Some(c) = next {
                run_sound(runner, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(use_pos: bool, d: Option<f32>) -> u32 {
        stream_volume(&VolumeInputs {
            menu_active: false,
            foreground: true,
            radio_volume: 1.0,
            use_pos,
            distance: 50.0,
            ped_distance: d,
        })
    }

    /// Measured values (trace-notes "Audio (TRACE-8)").
    #[test]
    fn volume_matches_the_measured_values() {
        assert_eq!(vol(false, None), 7000);
        assert_eq!(vol(true, Some(20.0)), 4800);
        assert_eq!(vol(true, Some(60.0)), 0);
        assert_eq!(vol(true, Some(50.0)), 0);
        assert!((7940..=7950).contains(&vol(true, Some(0.33))));
        assert!((7991..=7992).contains(&vol(true, Some(0.05)))); // measured 7991/7992 at "≈ 0.1 m"
        assert_eq!(vol(true, None), 0);
        let menu = VolumeInputs {
            menu_active: true,
            foreground: true,
            radio_volume: 1.0,
            use_pos: false,
            distance: 0.0,
            ped_distance: None,
        };
        assert_eq!(stream_volume(&menu), 0);
        assert_eq!(stream_volume(&VolumeInputs { menu_active: false, foreground: false, ..menu }), 0);
        assert_eq!(stream_volume(&VolumeInputs { menu_active: false, radio_volume: 0.5, ..menu }), 3500);
    }

    #[test]
    fn station_and_title_parsing() {
        let hdr = b"HTTP/1.0 200 OK\0icy-name:TraceStation\0ICY-URL:http://trace.invalid/\0\0";
        let (n, u) = station_from_headers(split_tag_list(hdr));
        assert_eq!(n.as_deref(), Some(&b"TraceStation"[..]));
        assert_eq!(u.as_deref(), Some(&b"http://trace.invalid/"[..]));
        assert_eq!(
            title_from_meta(b"StreamTitle='Trace Artist - Title 0';").as_deref(),
            Some(&b"Trace Artist - Title 0"[..])
        );
        assert_eq!(title_from_meta(b"StreamTitle='a';StreamUrl='x';").as_deref(), Some(&b"a"[..]));
        assert_eq!(title_from_meta(b"nothing"), None);
        assert_eq!(title_from_ogg([&b"ARTIST=A"[..], b"title=T"]).as_deref(), Some(&b"A - T"[..]));
        assert_eq!(title_from_ogg([&b"title=T"[..]]).as_deref(), Some(&b"T"[..]));
        assert_eq!(title_from_ogg([&b"artist=A"[..]]).as_deref(), Some(&b"A - (null)"[..]));
        assert_eq!(title_from_ogg([&b"x=y"[..]]), None);
        assert_eq!(
            title_line(b"Trace Artist - Title 0", b"TraceStation").as_deref(),
            Some(&b"Trace Artist - Title 0 - TraceStation"[..])
        );
        assert_eq!(title_line(b"", b"S"), None);
        assert_eq!(title_line(b"T", b""), None);
        assert_eq!(clip(&[b'a'; 300]).len(), 256);
    }

    /// Measured PlayerPlaySound command sequences (trace-notes "PlayerPlaySound (16)").
    #[test]
    fn play_sound_sequences() {
        let p = [1.0, 2.0, 3.0];
        let mut s = MissionSound::default();
        assert_eq!(s.play(1052, p), vec![SoundCmd::OneOff(p, 1052)]);
        assert_eq!(s.play(5, p), vec![SoundCmd::PlayMissionTrack(5)]);
        assert_eq!(s.play(0, p), vec![SoundCmd::StopMissionTrack]);
        assert!(s.play(1, p).is_empty() && s.flag);
        assert_eq!(
            s.play(2000, p),
            vec![SoundCmd::ClearMissionAudio, SoundCmd::LoadMissionAudio(2000), SoundCmd::MissionAudioPosition(p)]
        );
        assert_eq!(s.frame(), Some(SoundCmd::PollMissionAudio));
        assert_eq!(s.loaded(false), None);
        assert_eq!(s.frame(), Some(SoundCmd::PollMissionAudio));
        assert_eq!(s.loaded(true), Some(SoundCmd::PlayMissionAudio));
        assert_eq!(s.frame(), None);
        assert_eq!(s.play(0, p), vec![SoundCmd::ClearMissionAudio, SoundCmd::StopMissionTrack]);
        assert!(!s.flag && s.state == 0);
    }
}
