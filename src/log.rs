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
        let dir = crate::util::data_dir().join("logs");
        let _ = fs::create_dir_all(&dir);
        let me = Self {
            dir,
            cur_date: String::new(),
            file: None,
            retention_days: retention_days.clamp(1, 3650),
        };
        me.prune();
        me
    }

    fn prune(&self) {
        let today = crate::util::fmt_utc(SystemTime::now()).0;
        let Ok(rd) = fs::read_dir(&self.dir) else { return };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(rest) = name.strip_prefix("netspeed-") else { continue };
            let Some(date) = rest.strip_suffix(".csv") else { continue };
            if let Some(age) = crate::util::days_between(&today, date)
                && age > self.retention_days as i64
            {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    pub fn log_sample(&mut self, adapter: &str, down_bps: f64, up_bps: f64) {
        let now = SystemTime::now();
        let (date, ts) = crate::util::fmt_utc(now);
        if self.file.is_none() || self.cur_date != date {
            let path = self.dir.join(format!("netspeed-{date}.csv"));
            let exists = path.exists();
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(f) => {
                    let mut w = BufWriter::new(f);
                    if !exists {
                        let _ = writeln!(w, "timestamp,adapter,down_bytes_per_sec,up_bytes_per_sec");
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
