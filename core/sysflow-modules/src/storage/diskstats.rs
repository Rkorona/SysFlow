//! `/proc/diskstats` parsing and interval rates.
//!
//! The kernel counts sectors (always 512-byte units in this file, even on 4K-native devices) and
//! milliseconds of I/O. Rates are derived from two readings; a counter that went backwards means the
//! device was reset, and is treated as "unknown" rather than a huge negative number.

use sysflow_core::sysfs::Sysfs;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IoCounters {
    pub read_ios: u64,
    pub read_sectors: u64,
    pub write_ios: u64,
    pub write_sectors: u64,
    pub io_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IoRates {
    pub read_kb: u64,
    pub write_kb: u64,
    pub read_iops: f64,
    pub write_iops: f64,
    /// Busy percentage, clamped to 0..=100 (the kernel can over-count with parallel queues).
    pub util_pct: f64,
}

/// One `/proc/diskstats` line: device name plus the counters SysFlow uses.
///
/// Field numbers from the kernel documentation, with `major`/`minor`/`name` as fields 1–3:
/// reads completed = 4, sectors read = 6, writes completed = 8, sectors written = 10,
/// milliseconds spent doing I/O = 13.
pub fn parse_line(line: &str) -> Option<(String, IoCounters)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let name = *fields.get(2)?;
    let number = |index: usize| -> Option<u64> { fields.get(index)?.parse().ok() };
    Some((
        name.to_string(),
        IoCounters {
            read_ios: number(3)?,
            read_sectors: number(5)?,
            write_ios: number(7)?,
            write_sectors: number(9)?,
            io_ms: number(12)?,
        },
    ))
}

/// Read the counters for one device; `None` when the device is not in the table.
pub fn read(fs: &Sysfs, device: &str) -> Option<IoCounters> {
    let text = fs.read_lossy("/proc/diskstats").ok()?;
    text.lines()
        .filter_map(parse_line)
        .find(|(name, _)| name == device)
        .map(|(_, counters)| counters)
}

/// Convert two readings into the rates shown by `storage io` and stored by the history sampler.
pub fn rates(before: IoCounters, after: IoCounters, elapsed_ms: u64) -> IoRates {
    let elapsed = elapsed_ms.max(1) as f64;
    let read_sectors = after.read_sectors.saturating_sub(before.read_sectors);
    let write_sectors = after.write_sectors.saturating_sub(before.write_sectors);
    let read_ios = after.read_ios.saturating_sub(before.read_ios);
    let write_ios = after.write_ios.saturating_sub(before.write_ios);
    let io_ms = after.io_ms.saturating_sub(before.io_ms);
    IoRates {
        read_kb: read_sectors / 2,
        write_kb: write_sectors / 2,
        read_iops: read_ios as f64 * 1000.0 / elapsed,
        write_iops: write_ios as f64 * 1000.0 / elapsed,
        util_pct: (io_ms as f64 * 100.0 / elapsed).min(100.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
   8       0 sda 1000 0 20000 300 500 0 8000 200 0 400 600\n\
   8       1 sda1 900 0 19000 280 400 0 7000 190 0 380 560\n\
 259       0 nvme0n1 50 0 400 5 60 0 480 6 0 9 11\n";

    #[test]
    fn parses_the_counters_by_name() {
        let (name, counters) =
            parse_line("   8       0 sda 1000 0 20000 300 500 0 8000 200 0 400 600").unwrap();
        assert_eq!(name, "sda");
        assert_eq!(counters.read_ios, 1000);
        assert_eq!(counters.read_sectors, 20000);
        assert_eq!(counters.write_ios, 500);
        assert_eq!(counters.write_sectors, 8000);
        assert_eq!(counters.io_ms, 400);
        assert!(parse_line("too short").is_none());
    }

    #[test]
    fn partitions_do_not_shadow_the_whole_disk() {
        let root =
            std::env::temp_dir().join(format!("sysflow-storage-diskstats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proc")).unwrap();
        std::fs::write(root.join("proc/diskstats"), TABLE).unwrap();
        let fs = Sysfs::new(&root);
        assert_eq!(read(&fs, "sda").unwrap().read_sectors, 20000);
        assert_eq!(read(&fs, "sda1").unwrap().read_sectors, 19000);
        assert!(read(&fs, "missing").is_none());
    }

    #[test]
    fn rates_convert_sectors_to_kb_and_compute_utilisation() {
        let before = IoCounters {
            read_ios: 100,
            read_sectors: 2000,
            write_ios: 50,
            write_sectors: 1000,
            io_ms: 100,
        };
        let after = IoCounters {
            read_ios: 150,
            read_sectors: 4000,
            write_ios: 70,
            write_sectors: 3000,
            io_ms: 300,
        };
        let rates = rates(before, after, 1000);
        assert_eq!(rates.read_kb, 1000);
        assert_eq!(rates.write_kb, 1000);
        assert!((rates.read_iops - 50.0).abs() < f64::EPSILON);
        assert!((rates.write_iops - 20.0).abs() < f64::EPSILON);
        assert!((rates.util_pct - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn counter_regression_is_saturated_instead_of_underflowing() {
        let before = IoCounters {
            read_ios: 100,
            read_sectors: 500,
            ..IoCounters::default()
        };
        let rates = rates(before, IoCounters::default(), 1000);
        assert_eq!(rates.read_kb, 0);
        assert_eq!(rates.read_iops, 0.0);
    }
}
