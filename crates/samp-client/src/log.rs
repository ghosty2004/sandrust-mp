//! Tiny file logger: one line per record, `[YYYY-MM-DD HH:MM:SS.mmm UTC +secs.mmm] [thread] message`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

struct Logger {
    file: File,
    start: Instant,
}

static LOGGER: Mutex<Option<Logger>> = Mutex::new(None);

/// Opens (truncates) the log file. Later calls replace the target.
pub fn init(path: &Path) -> std::io::Result<()> {
    let file = OpenOptions::new().create(true).write(true).truncate(true).open(path)?;
    let mut g = LOGGER.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(Logger { file, start: Instant::now() });
    Ok(())
}

/// Writes one record; silently does nothing before [`init`].
pub fn write(msg: std::fmt::Arguments<'_>) {
    let mut g = LOGGER.lock().unwrap_or_else(|e| e.into_inner());
    write_to(&mut g, msg);
}

/// Like [`write`], but gives up instead of waiting when the logger is busy (the crash handler: the faulting
/// thread may hold the lock).
pub fn write_unblocked(msg: std::fmt::Arguments<'_>) {
    match LOGGER.try_lock() {
        Ok(mut g) => write_to(&mut g, msg),
        Err(std::sync::TryLockError::Poisoned(e)) => write_to(&mut e.into_inner(), msg),
        Err(std::sync::TryLockError::WouldBlock) => {}
    }
}

fn write_to(g: &mut Option<Logger>, msg: std::fmt::Arguments<'_>) {
    if let Some(l) = g.as_mut() {
        let since = l.start.elapsed();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let thread = std::thread::current();
        let name = thread.name().map(str::to_owned).unwrap_or_else(|| format!("{:?}", thread.id()));
        let _ = writeln!(
            l.file,
            "[{} +{}.{:03}] [{}] {}",
            format_utc(now.as_secs(), now.subsec_millis()),
            since.as_secs(),
            since.subsec_millis(),
            name,
            msg
        );
        let _ = l.file.flush();
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(format_args!($($arg)*)) };
}

/// Formats Unix time as `YYYY-MM-DD HH:MM:SS.mmm UTC`.
pub fn format_utc(secs: u64, millis: u32) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}.{millis:03} UTC", rem / 3600, (rem / 60) % 60, rem % 60)
}

/// Days since 1970-01-01 → (year, month, day) in the proleptic Gregorian calendar.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_format() {
        assert_eq!(format_utc(0, 0), "1970-01-01 00:00:00.000 UTC");
        assert_eq!(format_utc(951_782_400, 5), "2000-02-29 00:00:00.005 UTC");
        assert_eq!(format_utc(1_793_806_245, 123), "2026-11-04 15:30:45.123 UTC");
    }
}
