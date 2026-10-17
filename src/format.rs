use crate::config::Units;

/// Format a rate given in bytes per second.
pub fn format_rate(bytes_per_sec: f64, units: Units, decimals: u32) -> String {
    let (base, mult, suffixes): (f64, f64, &[&str]) = match units {
        Units::Bits => (1000.0, 8.0, &["b/s", "Kb/s", "Mb/s", "Gb/s", "Tb/s"]),
        Units::BytesSi => (1000.0, 1.0, &["B/s", "KB/s", "MB/s", "GB/s", "TB/s"]),
        Units::BytesBin => (1024.0, 1.0, &["B/s", "KiB/s", "MiB/s", "GiB/s", "TiB/s"]),
    };
    let mut v = bytes_per_sec.max(0.0) * mult;
    let mut i = 0usize;
    while v >= base && i < suffixes.len() - 1 {
        v /= base;
        i += 1;
    }
    if v == 0.0 {
        return format!("0 {}", suffixes[i]);
    }
    let dec = if v >= 100.0 {
        0
    } else if v >= 10.0 {
        decimals.min(1)
    } else {
        decimals
    };
    format!("{:.*} {}", dec as usize, v, suffixes[i])
}

/// Format a byte count for session totals, always binary units, compact.
pub fn format_bytes(bytes: u64) -> String {
    let base = 1024.0;
    let suffixes = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut i = 0usize;
    while v >= base && i < suffixes.len() - 1 {
        v /= base;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", suffixes[i])
    } else if v >= 10.0 {
        format!("{v:.1} {}", suffixes[i])
    } else {
        format!("{v:.2} {}", suffixes[i])
    }
}
