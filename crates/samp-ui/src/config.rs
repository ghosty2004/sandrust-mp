//! `sa-mp.cfg` in the user files directory: [`SampCfg`] reads it into typed values and rewrites it,
//! and [`UiSettings`] holds the UI settings derived from it. Like samp's writer, every store
//! rewrites the whole file with one line per key, using the format strings `"%s=%d\n"` (int),
//! `"%s=\"%s\"\n"` (string) and `"%s=%f\n"` (float). Specs: docs/re/subsystems/ui-rendering.md
//! ("Local commands — UI effects and persistence") and docs/phase3/trace-notes.md ("sa-mp.cfg").

use crate::chat::PAGE_SIZE_SETTER_RANGE;
use crate::text::atoi;

/// File name in the user files directory.
pub const FILE_NAME: &str = "sa-mp.cfg";
/// Line terminator on disk. The writer opens the file with mode `"w"` (text mode),
/// so the CRT turns `\n` into CR LF (inferred).
pub const LINE_END: &[u8] = b"\r\n";
/// Value `fpslimit` is created with when missing at start-up.
pub const DEFAULT_FPS_LIMIT: i32 = 48;
/// `/fpslimit` and start-up accept this range.
pub const FPS_LIMIT_RANGE: std::ops::RangeInclusive<i32> = 20..=90;

/// Config keys.
pub mod keys {
    pub const PAGESIZE: &str = "pagesize";
    pub const FONTSIZE: &str = "fontsize";
    pub const TIMESTAMP: &str = "timestamp";
    pub const FPSLIMIT: &str = "fpslimit";
    pub const DISABLEHEADMOVE: &str = "disableheadmove";
    pub const NOHUDSCALEFIX: &str = "nohudscalefix";
    pub const NONAMETAGSTATUS: &str = "nonametagstatus";
    pub const AUDIOMSGOFF: &str = "audiomsgoff";
    pub const LOGURLS: &str = "logurls";
    pub const FONTFACE: &str = "fontface";
    pub const FONTWEIGHT: &str = "fontweight";
    pub const DIRECTMODE: &str = "directmode";
    pub const IME: &str = "ime";
    pub const MULTICORE: &str = "multicore";
    pub const AUDIOPROXYOFF: &str = "audioproxyoff";
}

/// A typed config value (the three writer formats).
#[derive(Clone, Debug, PartialEq)]
pub enum CfgValue {
    Int(i32),
    Str(Vec<u8>),
    Float(f32),
}

impl CfgValue {
    /// Integer view: ints as is, floats truncated, strings through `atoi` (inferred).
    pub fn as_int(&self) -> i32 {
        match self {
            Self::Int(v) => *v,
            Self::Float(f) => *f as i32,
            Self::Str(s) => atoi(s),
        }
    }
}

/// The parsed file, keys in file order (new keys are appended).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SampCfg {
    entries: Vec<(String, CfgValue)>,
}

/// C `atof` on the leading numeric part (`strtod` subset: sign, digits, one `.`, exponent).
fn atof(v: &[u8]) -> f32 {
    let mut i = 0;
    while i < v.len() && matches!(v[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        i += 1;
    }
    let start = i;
    if i < v.len() && matches!(v[i], b'+' | b'-') {
        i += 1;
    }
    while i < v.len() && v[i].is_ascii_digit() {
        i += 1;
    }
    if i < v.len() && v[i] == b'.' {
        i += 1;
        while i < v.len() && v[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < v.len() && matches!(v[i], b'e' | b'E') {
        let mut j = i + 1;
        if j < v.len() && matches!(v[j], b'+' | b'-') {
            j += 1;
        }
        if j < v.len() && v[j].is_ascii_digit() {
            while j < v.len() && v[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    std::str::from_utf8(&v[start..i]).ok().and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0) as f32
}

/// Value type: first and last character `"` → string (without the quotes), else a `.` anywhere →
/// float (`atof`), else integer (`atol`). So an unquoted `fontface=Courier New` is the integer 0,
/// and the original writes it back as `fontface=0`. Mirrors samp.dll B 0x10065f20 (2019 build).
fn parse_value(v: &[u8]) -> CfgValue {
    if v.len() >= 2 && v[0] == b'"' && v[v.len() - 1] == b'"' {
        return CfgValue::Str(v[1..v.len() - 1].to_vec());
    }
    if v == b"\"" {
        // samp: the one-character string `"` (first = last quote) is stored as empty.
        return CfgValue::Str(Vec::new());
    }
    if v.contains(&b'.') {
        return CfgValue::Float(atof(v));
    }
    CfgValue::Int(atoi(v))
}

/// samp's line buffer (`fgets(buf, 256, f)`): longer lines are read as several lines.
const LINE_BUF: usize = 255;

impl SampCfg {
    /// Parses the file like samp's loader: per line (at most 255 bytes), leading spaces/tabs are
    /// skipped; lines starting with `;`, `[`, or empty are ignored; the key runs up to a space, tab,
    /// `=`, `;` or the line end and is lower-cased; then optional blanks, a required `=`, optional
    /// blanks, the value up to the line end with trailing spaces, tabs and CRs removed; empty keys
    /// or values are ignored. A later duplicate key wins. Mirrors samp.dll B 0x10066230 (2019 build).
    pub fn parse(bytes: &[u8]) -> Self {
        let mut cfg = Self::default();
        let mut lines = Vec::new();
        for raw in bytes.split_inclusive(|&b| b == b'\n') {
            for chunk in raw.chunks(LINE_BUF) {
                lines.push(chunk);
            }
        }
        for line in lines {
            let line = &line[..line.iter().position(|&b| b == 0).unwrap_or(line.len())];
            let mut i = 0;
            while i < line.len() && matches!(line[i], b' ' | b'\t') {
                i += 1;
            }
            if i >= line.len() || matches!(line[i], b';' | b'\n' | b'[') {
                continue;
            }
            let ks = i;
            while i < line.len() && !matches!(line[i], b' ' | b'=' | b'\n' | b'\t' | b';') {
                i += 1;
            }
            if i == ks {
                continue;
            }
            let key = String::from_utf8_lossy(&line[ks..i]).to_ascii_lowercase();
            while i < line.len() && matches!(line[i], b' ' | b'\t') {
                i += 1;
            }
            if i >= line.len() || line[i] != b'=' {
                continue;
            }
            i += 1;
            while i < line.len() && matches!(line[i], b' ' | b'\t') {
                i += 1;
            }
            let vs = i;
            while i < line.len() && line[i] != b'\n' {
                i += 1;
            }
            if i == vs {
                continue;
            }
            let mut v = &line[vs..i];
            while let Some((&last, rest)) = v.split_last() {
                if matches!(last, b' ' | b'\t' | b'\r') {
                    v = rest;
                } else {
                    break;
                }
            }
            if v.is_empty() {
                continue;
            }
            cfg.set(&key, parse_value(v));
        }
        cfg
    }

    /// Serialises every key with the writer's formats and [`LINE_END`].
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in &self.entries {
            out.extend_from_slice(k.as_bytes());
            out.push(b'=');
            match v {
                CfgValue::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
                CfgValue::Float(f) => out.extend_from_slice(format!("{:.6}", f64::from(*f)).as_bytes()),
                CfgValue::Str(s) => {
                    out.push(b'"');
                    out.extend_from_slice(s);
                    out.push(b'"');
                }
            }
            out.extend_from_slice(LINE_END);
        }
        out
    }

    /// Key lookup; keys are compared lower-cased, as samp's lookup lower-cases the name.
    pub fn get(&self, key: &str) -> Option<&CfgValue> {
        let key = key.to_ascii_lowercase();
        self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }
    pub fn get_int(&self, key: &str) -> Option<i32> {
        self.get(key).map(CfgValue::as_int)
    }
    pub fn get_str(&self, key: &str) -> Option<&[u8]> {
        match self.get(key) {
            Some(CfgValue::Str(s)) => Some(s),
            _ => None,
        }
    }
    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Sets a key in place, or appends it.
    pub fn set(&mut self, key: &str, value: CfgValue) {
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.entries.push((key.to_owned(), value)),
        }
    }
    pub fn set_int(&mut self, key: &str, value: i32) {
        self.set(key, CfgValue::Int(value));
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    /// Keys the client creates on its first frame (a file the original wrote has `fpslimit=48`,
    /// `multicore=1`, then `audioproxyoff=0` from the audio set-up):
    /// - `fpslimit` = 48 when its integer value is 0 or the key is missing;
    /// - `multicore` = 1 when the key is missing.
    ///
    /// `audioproxyoff` is not created here: samp adds it only after a successful `BASS_Init`
    /// ([`Self::ensure_audio_proxy_key`]).
    ///
    /// Returns true if the file must be rewritten.
    pub fn apply_startup_defaults(&mut self) -> bool {
        let mut changed = false;
        if self.get_int(keys::FPSLIMIT).unwrap_or(0) == 0 {
            self.set_int(keys::FPSLIMIT, DEFAULT_FPS_LIMIT);
            changed = true;
        }
        if !self.contains(keys::MULTICORE) {
            self.set_int(keys::MULTICORE, 1);
            changed = true;
        }
        changed
    }

    /// The audio set-up after a successful `BASS_Init`: `audioproxyoff` = 0 when the key is missing.
    /// Returns true if the file must be rewritten. Mirrors samp.dll 0x10066480.
    pub fn ensure_audio_proxy_key(&mut self) -> bool {
        if self.contains(keys::AUDIOPROXYOFF) {
            return false;
        }
        self.set_int(keys::AUDIOPROXYOFF, 0);
        true
    }
}

/// UI-relevant settings: persisted ones read from `sa-mp.cfg` plus session-only toggles.
#[derive(Clone, Debug, PartialEq)]
pub struct UiSettings {
    /// Chat page size (10..100 accepted from the file through the setter).
    pub page_size: u32,
    /// `fontsize` (assumed not range-checked at start-up).
    pub font_size: i32,
    pub timestamps: bool,
    /// Frame limiter; `None` when the stored value is outside 20..90 (ignored).
    pub fps_limit: Option<u32>,
    pub head_move_disabled: bool,
    /// HUD scale fix, default on (`nohudscalefix` = 0).
    pub hud_scale_fix: bool,
    /// AFK icon on nametags, default on (`nonametagstatus` = 0).
    pub nametag_status: bool,
    pub audio_messages_off: bool,
    pub log_urls: bool,
    pub font_face: Option<String>,
    pub font_weight: Option<i32>,
    pub direct_mode: bool,
    pub ime: bool,
    pub multicore: bool,
    pub audio_proxy_off: bool,
    /// `/togobjlight` flag; starts false (assumed), visible effect unknown. Mirrors the samp.dll
    /// global at 0x101166c0.
    pub object_light_flag: bool,
    /// `/dl` vehicle debug labels.
    pub vehicle_labels: bool,
    /// `/ctd` camera-target debug labels.
    pub camera_target_labels: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            page_size: crate::chat::DEFAULT_PAGE_SIZE,
            font_size: 0,
            timestamps: false,
            fps_limit: None,
            head_move_disabled: false,
            hud_scale_fix: true,
            nametag_status: true,
            audio_messages_off: false,
            log_urls: false,
            font_face: None,
            font_weight: None,
            direct_mode: false,
            ime: false,
            multicore: false,
            audio_proxy_off: false,
            object_light_flag: false,
            vehicle_labels: false,
            camera_target_labels: false,
        }
    }
}

impl UiSettings {
    /// Start-up read of every UI key.
    pub fn from_cfg(cfg: &SampCfg) -> Self {
        let mut s = Self::default();
        let flag = |k: &str| cfg.get_int(k).is_some_and(|v| v != 0);
        if let Some(p) = cfg.get_int(keys::PAGESIZE)
            && p >= 0
            && PAGE_SIZE_SETTER_RANGE.contains(&(p as u32))
        {
            s.page_size = p as u32;
        }
        s.font_size = cfg.get_int(keys::FONTSIZE).unwrap_or(0);
        s.timestamps = flag(keys::TIMESTAMP);
        s.fps_limit = cfg.get_int(keys::FPSLIMIT).filter(|v| FPS_LIMIT_RANGE.contains(v)).map(|v| v as u32);
        s.head_move_disabled = flag(keys::DISABLEHEADMOVE);
        s.hud_scale_fix = !flag(keys::NOHUDSCALEFIX);
        s.nametag_status = !flag(keys::NONAMETAGSTATUS);
        s.audio_messages_off = flag(keys::AUDIOMSGOFF);
        s.log_urls = flag(keys::LOGURLS);
        s.font_face = cfg.get_str(keys::FONTFACE).map(|b| String::from_utf8_lossy(b).into_owned());
        s.font_weight = cfg.get_int(keys::FONTWEIGHT);
        s.direct_mode = flag(keys::DIRECTMODE);
        s.ime = flag(keys::IME);
        s.multicore = flag(keys::MULTICORE);
        s.audio_proxy_off = flag(keys::AUDIOPROXYOFF);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_formats_int_string_float_crlf() {
        let mut c = SampCfg::default();
        c.set_int("pagesize", 15);
        c.set("fontface", CfgValue::Str(b"Tahoma".to_vec()));
        c.set("x", CfgValue::Float(1.5));
        assert_eq!(c.to_bytes(), b"pagesize=15\r\nfontface=\"Tahoma\"\r\nx=1.500000\r\n");
    }

    #[test]
    fn parse_round_trip_and_types() {
        let c = SampCfg::parse(b"pagesize=15\r\nfontface=\"Comic Sans\"\nbad line\n\nfontsize=-2\nf=0.25\n");
        assert_eq!(c.get_int("pagesize"), Some(15));
        assert_eq!(c.get_str("fontface"), Some(&b"Comic Sans"[..]));
        assert_eq!(c.get_int("fontsize"), Some(-2));
        assert_eq!(c.get("f"), Some(&CfgValue::Float(0.25)));
        assert_eq!(SampCfg::parse(&c.to_bytes()), c);
        let mut c2 = c.clone();
        c2.set_int("pagesize", 20);
        assert_eq!(c2.keys().next(), Some("pagesize"));
    }

    /// Expected values from docs/phase3/trace-notes.md ("sa-mp.cfg"): an unquoted string is the
    /// integer 0 and is rewritten as such; quoted strings, floats, case-insensitive keys, comments
    /// and blanks.
    #[test]
    fn trace8_cfg_value_types_and_rewrite() {
        let mut c = SampCfg::parse(b"fontface=Courier New\r\n");
        assert_eq!(c.get("fontface"), Some(&CfgValue::Int(0)));
        c.set_int("pagesize", 15);
        assert_eq!(c.to_bytes(), b"fontface=0\r\npagesize=15\r\n");
        let c = SampCfg::parse(
            b"  FontFace = \"Courier New\"  \r\n; comment=1\n[section]\nx=\n=5\nratio=1.5\nFPSLIMIT\t=\t30 \nbad 7\n",
        );
        assert_eq!(c.get_str("fontface"), Some(&b"Courier New"[..]));
        assert_eq!(c.get("ratio"), Some(&CfgValue::Float(1.5)));
        assert_eq!(c.get_int("fpslimit"), Some(30));
        assert_eq!(c.get_int("FpsLimit"), Some(30));
        assert!(!c.contains("comment") && !c.contains("x") && !c.contains("bad"));
        assert_eq!(c.keys().collect::<Vec<_>>(), vec!["fontface", "ratio", "fpslimit"]);
    }

    #[test]
    fn ur_9_startup_creates_fpslimit_48_and_audioproxyoff() {
        let mut c = SampCfg::parse(b"pagesize=12\n");
        assert!(c.apply_startup_defaults());
        assert_eq!(c.get_int("fpslimit"), Some(48));
        // Not before BASS started.
        assert!(!c.contains("audioproxyoff"));
        assert!(c.ensure_audio_proxy_key());
        assert_eq!(c.get_int("audioproxyoff"), Some(0));
        assert_eq!(c.to_bytes(), b"pagesize=12\r\nfpslimit=48\r\nmulticore=1\r\naudioproxyoff=0\r\n");
        assert!(!c.apply_startup_defaults());
        assert!(!c.ensure_audio_proxy_key());
        let mut c = SampCfg::parse(b"fpslimit=60\n");
        c.apply_startup_defaults();
        assert_eq!(c.get_int("fpslimit"), Some(60));
    }

    #[test]
    fn ur_9_settings_from_cfg_defaults_and_ranges() {
        let s = UiSettings::from_cfg(&SampCfg::default());
        assert_eq!(s, UiSettings::default());
        assert!(s.hud_scale_fix && s.nametag_status);
        let s = UiSettings::from_cfg(&SampCfg::parse(
            b"pagesize=50\nfpslimit=95\ntimestamp=1\nnohudscalefix=1\nnonametagstatus=1\nfontweight=1\n",
        ));
        assert_eq!(s.page_size, 50); // the start-up path uses the 10..100 setter
        assert_eq!(s.fps_limit, None); // outside 20..90: ignored
        assert!(s.timestamps && !s.hud_scale_fix && !s.nametag_status);
        assert_eq!(s.font_weight, Some(1));
        let s = UiSettings::from_cfg(&SampCfg::parse(b"pagesize=5\nfpslimit=90\n"));
        assert_eq!((s.page_size, s.fps_limit), (10, Some(90)));
    }
}
