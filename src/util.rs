use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::core::PCWSTR;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn from_wide_buf(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Returns (date "YYYY-MM-DD", timestamp "YYYY-MM-DDTHH:MM:SSZ") in UTC.
pub fn fmt_utc(t: SystemTime) -> (String, String) {
    let secs = unix_secs(t) as i64;
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400) as u32;
    let (y, m, d) = civil_from_days(days);
    let hh = sod / 3600;
    let mm = (sod % 3600) / 60;
    let ss = sod % 60;
    (
        format!("{y:04}-{m:02}-{d:02}"),
        format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z"),
    )
}

/// Number of whole days between two "YYYY-MM-DD" dates (a - b).
pub fn days_between(a: &str, b: &str) -> Option<i64> {
    fn parse(s: &str) -> Option<i64> {
        let mut it = s.split('-');
        let y: i64 = it.next()?.parse().ok()?;
        let m: i64 = it.next()?.parse().ok()?;
        let d: i64 = it.next()?.parse().ok()?;
        // days_from_civil
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Some(era * 146097 + doe - 719468)
    }
    Some(parse(a)? - parse(b)?)
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("NetSpeed");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn log_error(msg: &str) {
    let path = data_dir().join("netspeed-errors.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let (_, ts) = fmt_utc(SystemTime::now());
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

/// Open a folder in Explorer.
pub fn open_path(path: &std::path::Path) {
    let file = wide(&path.to_string_lossy());
    unsafe {
        ShellExecuteW(
            None,
            windows::core::w!("open"),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}
