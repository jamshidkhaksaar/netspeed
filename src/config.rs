use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    /// bits/s with SI prefixes: b/s, Kb/s, Mb/s, Gb/s
    Bits,
    /// bytes/s with SI (1000) prefixes: B/s, KB/s, MB/s
    BytesSi,
    /// bytes/s with binary (1024) prefixes: B/s, KiB/s, MiB/s
    BytesBin,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub interval_ms: u32,
    pub units: Units,
    pub decimals: u32,
    pub adapter_luid: Option<u64>,
    pub adapter_name: String,
    pub logging: bool,
    pub log_retention_days: u32,
    pub click_through: bool,
    pub autostart: bool,
    pub show: bool,
    pub font_size: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_ms: 1000,
            units: Units::BytesBin,
            decimals: 1,
            adapter_luid: None,
            adapter_name: String::new(),
            logging: false,
            log_retention_days: 30,
            click_through: false,
            autostart: false,
            show: true,
            font_size: 12.0,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        crate::util::data_dir().join("config.json")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), s);
        }
    }

    pub fn interval_clamped(&self) -> u32 {
        self.interval_ms.clamp(250, 60_000)
    }
}
