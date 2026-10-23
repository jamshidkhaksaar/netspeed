use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::SystemTime;

pub struct Logger {
    dir: PathBuf,
    cur_date: String,
    file: Option<BufWriter<File>>,
    retention_days: u32,
}

impl Logger {
    pub fn new(retention_days: u32) -> Self {
        Self::new_in(
            crate::util::data_dir().join("logs"),
            retention_days,
            SystemTime::now(),
        )
    }

    fn new_in(dir: PathBuf, retention_days: u32, now: SystemTime) -> Self {
        let _ = fs::create_dir_all(&dir);
        let me = Self {
            dir,
            cur_date: String::new(),
            file: None,
            retention_days: retention_days.clamp(1, 3650),
        };
        me.prune_at(now);
        me
    }

    fn prune_at(&self, now: SystemTime) {
        let today = crate::util::fmt_utc(now).0;
        let Ok(dir) = self.dir.canonicalize() else {
            return;
        };
        let Ok(rd) = fs::read_dir(&dir) else { return };
        for entry in rd.flatten() {
            // Only regular NetSpeed CSV files in this exact directory are ours.
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(date) = date_in_log_name(&name) else {
                continue;
            };
            if let Some(age) = crate::util::days_between(&today, date)
                && age > self.retention_days as i64
            {
                let path = entry.path();
                if path.is_absolute() && path.parent() == Some(dir.as_path()) {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }

    pub fn log_sample(&mut self, adapter: &str, down_bps: f64, up_bps: f64) {
        self.log_sample_at(adapter, down_bps, up_bps, SystemTime::now());
    }

    fn log_sample_at(&mut self, adapter: &str, down_bps: f64, up_bps: f64, now: SystemTime) {
        let (date, ts) = crate::util::fmt_utc(now);
        if self.cur_date != date {
            // Release the previous file before pruning, including after a long
            // pause where that file itself may have expired on Windows.
            self.file = None;
            self.prune_at(now);
        }
        if self.file.is_none() || self.cur_date != date {
            let path = self.dir.join(format!("netspeed-{date}.csv"));
            let exists = path.exists();
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(f) => {
                    let mut w = BufWriter::new(f);
                    if !exists {
                        let _ =
                            writeln!(w, "timestamp,adapter,down_bytes_per_sec,up_bytes_per_sec");
                    }
                    self.file = Some(w);
                    self.cur_date = date;
                }
                Err(e) => {
                    crate::util::log_error(&format!("cannot open log file: {e}"));
                    self.file = None;
                    return;
                }
            }
        }
        if let Some(f) = &mut self.file {
            let adapter = adapter.replace(',', ";").replace('\n', " ");
            let _ = writeln!(f, "{ts},{adapter},{down_bps:.1},{up_bps:.1}");
            let _ = f.flush();
        }
    }
}

fn date_in_log_name(name: &str) -> Option<&str> {
    let date = name.strip_prefix("netspeed-")?.strip_suffix(".csv")?;
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || !bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return None;
    }
    let year: u32 = date[..4].parse().ok()?;
    let month: u32 = date[5..7].parse().ok()?;
    let day: u32 = date[8..].parse().ok()?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let last_day = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    (day >= 1 && day <= last_day).then_some(date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, UNIX_EPOCH};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
        root: PathBuf,
        name: String,
    }

    impl TestDir {
        fn new() -> Self {
            let root = std::env::temp_dir().canonicalize().unwrap();
            let name = format!(
                "netspeed-log-test-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed),
            );
            let path = root.join(&name);
            fs::create_dir(&path).unwrap();
            let path = path.canonicalize().unwrap();
            assert_eq!(path.parent(), Some(root.as_path()));
            assert_eq!(path.file_name(), Some(OsStr::new(&name)));
            Self { path, root, name }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            if let Ok(path) = self.path.canonicalize() {
                // Never clean up anything except this test's exact owned subtree.
                assert_eq!(path.parent(), Some(self.root.as_path()));
                assert_eq!(path.file_name(), Some(OsStr::new(&self.name)));
                fs::remove_dir_all(path).unwrap();
            }
        }
    }

    fn utc(date: &str) -> SystemTime {
        let days = crate::util::days_between(date, "1970-01-01").unwrap();
        assert!(days >= 0);
        UNIX_EPOCH + Duration::from_secs(days as u64 * 86_400)
    }

    #[test]
    fn daily_rotation_prunes_newly_expired_files_without_restarting_logger() {
        let dir = TestDir::new();
        let expires = dir.path.join("netspeed-2026-10-22.csv");
        fs::write(&expires, "old sample\n").unwrap();
        let mut logger = Logger::new_in(dir.path.clone(), 1, utc("2026-10-23"));
        assert!(expires.exists());
        logger.log_sample_at("Wi-Fi", 100.0, 10.0, utc("2026-10-23"));
        assert!(expires.exists());
        logger.log_sample_at("Wi-Fi", 200.0, 20.0, utc("2026-10-24"));
        assert!(!expires.exists());
        assert!(dir.path.join("netspeed-2026-10-23.csv").exists());
        assert!(dir.path.join("netspeed-2026-10-24.csv").exists());
    }

    #[test]
    fn rotation_after_a_long_gap_can_remove_the_previous_open_file() {
        let dir = TestDir::new();
        let mut logger = Logger::new_in(dir.path.clone(), 1, utc("2026-10-23"));
        logger.log_sample_at("Wi-Fi", 100.0, 10.0, utc("2026-10-23"));
        let previous = dir.path.join("netspeed-2026-10-23.csv");
        assert!(previous.exists());
        logger.log_sample_at("Wi-Fi", 100.0, 10.0, utc("2026-10-26"));
        assert!(!previous.exists());
        assert!(dir.path.join("netspeed-2026-10-26.csv").exists());
    }

    #[test]
    fn pruning_keeps_unrelated_files_invalid_dates_directories_and_future_logs() {
        let dir = TestDir::new();
        for name in [
            "netspeed-2026-10-20.csv",
            "netspeed-2026-10-25.csv",
            "netspeed-2026-02-30.csv",
            "netspeed-not-a-date.csv",
            "netspeed-2026-10-20.csv.bak",
            "notes.txt",
        ] {
            fs::write(dir.path.join(name), "keep unless expired\n").unwrap();
        }
        let folder = dir.path.join("netspeed-2026-10-01.csv");
        fs::create_dir(&folder).unwrap();
        let _logger = Logger::new_in(dir.path.clone(), 1, utc("2026-10-23"));
        assert!(!dir.path.join("netspeed-2026-10-20.csv").exists());
        for name in [
            "netspeed-2026-10-25.csv",
            "netspeed-2026-02-30.csv",
            "netspeed-not-a-date.csv",
            "netspeed-2026-10-20.csv.bak",
            "notes.txt",
        ] {
            assert!(dir.path.join(name).exists(), "must preserve {name}");
        }
        assert!(folder.is_dir());
    }

    #[test]
    fn same_day_samples_append_with_one_header() {
        let dir = TestDir::new();
        let mut logger = Logger::new_in(dir.path.clone(), 30, utc("2026-10-23"));
        logger.log_sample_at(
            "Wi-Fi",
            12.5,
            1.5,
            utc("2026-10-23") + Duration::from_secs(1),
        );
        logger.log_sample_at(
            "Wi-Fi",
            25.0,
            2.0,
            utc("2026-10-23") + Duration::from_secs(2),
        );
        let text = fs::read_to_string(dir.path.join("netspeed-2026-10-23.csv")).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "timestamp,adapter,down_bytes_per_sec,up_bytes_per_sec"
        );
        assert_eq!(lines[1], "2026-10-23T00:00:01Z,Wi-Fi,12.5,1.5");
        assert_eq!(lines[2], "2026-10-23T00:00:02Z,Wi-Fi,25.0,2.0");
    }
}
