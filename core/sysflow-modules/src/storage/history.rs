//! History sampling for the storage module (`state/storage.json` baseline + interval deltas).
//!
//! The series follow `docs/protocol.md` §3.26: instantaneous capacity on every sample, and I/O
//! throughput as a **difference of cumulative counters**. A counter that went backwards means the
//! machine rebooted (or the device was reset), so the delta series start one point after the boot
//! instead of writing a fabricated `0`. Rates divide by the measured interval, not the nominal 300 s:
//! a device that slept for hours would otherwise report a burst that never happened.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map};

use sysflow_core::event::{EventStore, NewSystemEvent};
use sysflow_core::module::Sample;
use sysflow_core::sysfs::Sysfs;

use super::{capacity, device, diskstats};

pub const FILE_NAME: &str = "storage.json";
/// Five minutes: write-volume history stays meaningful and fits the 30 day line cap.
pub const SAMPLE_INTERVAL_S: u64 = 300;
const STATE_VERSION: u32 = 1;

/// Space warning thresholds. `used` counts blocks the filesystem cannot hand out, so 90 % is really
/// "the next big install may fail"; recovery has a lower bar to avoid flapping in the middle.
pub const LOW_PCT: u64 = 90;
pub const LOW_RECOVER_PCT: u64 = 85;

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Counters {
    #[serde(default)]
    pub read_ios: u64,
    #[serde(default)]
    pub read_sectors: u64,
    #[serde(default)]
    pub write_ios: u64,
    #[serde(default)]
    pub write_sectors: u64,
    #[serde(default)]
    pub io_ms: u64,
}

impl Counters {
    fn from_io(counters: diskstats::IoCounters) -> Self {
        Counters {
            read_ios: counters.read_ios,
            read_sectors: counters.read_sectors,
            write_ios: counters.write_ios,
            write_sectors: counters.write_sectors,
            io_ms: counters.io_ms,
        }
    }

    fn to_io(self) -> diskstats::IoCounters {
        diskstats::IoCounters {
            read_ios: self.read_ios,
            read_sectors: self.read_sectors,
            write_ios: self.write_ios,
            write_sectors: self.write_sectors,
            io_ms: self.io_ms,
        }
    }

    fn regressed(&self, previous: &Counters) -> bool {
        self.read_ios < previous.read_ios
            || self.read_sectors < previous.read_sectors
            || self.write_ios < previous.write_ios
            || self.write_sectors < previous.write_sectors
            || self.io_ms < previous.io_ms
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Baseline {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    ts: u64,
    #[serde(default)]
    boot_id: String,
    #[serde(default)]
    uptime_s: f64,
    #[serde(default)]
    counters: Option<Counters>,
    #[serde(default)]
    warned_low: bool,
}

impl Default for Baseline {
    fn default() -> Self {
        Baseline {
            version: 0,
            ts: 0,
            boot_id: String::new(),
            uptime_s: 0.0,
            counters: None,
            warned_low: false,
        }
    }
}

struct StateFile {
    path: PathBuf,
}

impl StateFile {
    fn new(state_dir: &Path) -> Self {
        StateFile {
            path: state_dir.join(FILE_NAME),
        }
    }

    fn load(&self) -> Baseline {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn store(&self, baseline: &Baseline) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(body) = serde_json::to_string(baseline) else {
            return;
        };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

fn boot_id(fs: &Sysfs) -> String {
    fs.read_str("/proc/sys/kernel/random/boot_id")
        .unwrap_or_default()
}

fn uptime_seconds(fs: &Sysfs) -> f64 {
    fs.read_str("/proc/uptime")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse().ok())
        .unwrap_or(0.0)
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Produce one history sample for `/data`.
pub fn sample(
    fs: &Sysfs,
    state_dir: &Path,
    now_ts: u64,
    events: Option<&EventStore>,
) -> Option<Sample> {
    sample_at(fs, state_dir, now_ts, events, "/data")
}

/// Same sample for another mount path (host tests use a temporary directory).
pub fn sample_at(
    fs: &Sysfs,
    state_dir: &Path,
    now_ts: u64,
    events: Option<&EventStore>,
    data_path: &str,
) -> Option<Sample> {
    let (data_mount, _sdcard) = capacity::storage_mounts(fs, data_path).ok()?;
    let state = capacity::capacity(&data_mount.mount).ok()?;
    let device = device::for_mount(fs, &data_mount);
    let counters = device
        .as_ref()
        .and_then(|device| diskstats::read(fs, &device.name))
        .map(Counters::from_io);
    Some(sample_with(
        state_dir,
        now_ts,
        state,
        counters,
        boot_id(fs),
        uptime_seconds(fs),
        events,
    ))
}

/// The testable core: takes the already-read capacity/counters instead of calling `statvfs` and
/// `/proc/diskstats`, so the interval and reboot rules can be exercised on the host.
fn sample_with(
    state_dir: &Path,
    now_ts: u64,
    state: capacity::Capacity,
    counters: Option<Counters>,
    boot: String,
    uptime: f64,
    events: Option<&EventStore>,
) -> Sample {
    let used_kb = state.total_kb.saturating_sub(state.free_kb);
    let used_pct = used_kb
        .saturating_mul(100)
        .checked_div(state.total_kb)
        .unwrap_or(0);

    let file = StateFile::new(state_dir);
    let previous = file.load();

    let mut out = Map::new();
    out.insert("used_kb".into(), json!(used_kb));
    out.insert("free_kb".into(), json!(state.avail_kb));
    out.insert("used_pct".into(), json!(used_pct));

    let same_boot =
        previous.version == STATE_VERSION && !boot.is_empty() && boot == previous.boot_id;
    let interval_ok = previous.ts > 0
        && now_ts > previous.ts
        && uptime >= previous.uptime_s
        && !previous
            .counters
            .is_some_and(|old| counters.is_some_and(|new| new.regressed(&old)));
    if same_boot && interval_ok {
        if let (Some(old), Some(new)) = (previous.counters, counters) {
            let rates = diskstats::rates(
                old.to_io(),
                new.to_io(),
                (now_ts - previous.ts).saturating_mul(1000),
            );
            out.insert("read_kb".into(), json!(rates.read_kb));
            out.insert("write_kb".into(), json!(rates.write_kb));
            out.insert("io_util_pct".into(), json!(round1(rates.util_pct)));
        }
    }

    // Low-space events cross a threshold once; the hysteresis prevents a bar hovering at the line
    // from emitting every five minutes.
    let mut warned_low = previous.warned_low;
    if let Some(events) = events {
        if used_pct >= LOW_PCT && !previous.warned_low {
            warned_low = true;
            let _ = events.emit(
                NewSystemEvent::new(
                    "storage",
                    "space",
                    "storage_low_space",
                    "warning",
                    "存储空间不足",
                )
                .message(format!(
                    "/data 已使用 {used_pct}%（剩余 {}），请清理空间",
                    human_kb(state.avail_kb)
                ))
                .metadata(json!({
                    "used_pct": used_pct,
                    "used_kb": used_kb,
                    "free_kb": state.avail_kb,
                    "total_kb": state.total_kb,
                }))
                .dedupe("storage.low_space", 6 * 3600),
            );
        } else if used_pct <= LOW_RECOVER_PCT && previous.warned_low {
            warned_low = false;
            let _ = events.emit(
                NewSystemEvent::new(
                    "storage",
                    "space",
                    "storage_space_recovered",
                    "info",
                    "存储空间已恢复",
                )
                .message(format!("/data 使用率回落到 {used_pct}%"))
                .metadata(json!({"used_pct": used_pct, "free_kb": state.avail_kb}))
                .dedupe("storage.space_recovered", 6 * 3600),
            );
        }
    } else if used_pct >= LOW_PCT {
        warned_low = true;
    } else if used_pct <= LOW_RECOVER_PCT {
        warned_low = false;
    }

    file.store(&Baseline {
        version: STATE_VERSION,
        ts: now_ts,
        boot_id: boot,
        uptime_s: uptime,
        counters,
        warned_low,
    });
    out
}

fn human_kb(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.1} GB", kb as f64 / (1024.0 * 1024.0))
    } else if kb >= 1024 {
        format!("{:.1} MB", kb as f64 / 1024.0)
    } else {
        format!("{kb} KB")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sysflow-storage-history-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("state")).unwrap();
        dir
    }

    fn capacity(total_kb: u64, free_kb: u64, avail_kb: u64) -> capacity::Capacity {
        capacity::Capacity {
            total_kb,
            free_kb,
            avail_kb,
        }
    }

    fn counters(read_sectors: u64, write_sectors: u64, io_ms: u64) -> Counters {
        Counters {
            read_ios: read_sectors / 8,
            read_sectors,
            write_ios: write_sectors / 8,
            write_sectors,
            io_ms,
        }
    }

    #[test]
    fn first_sample_reports_capacity_but_no_delta_series() {
        let root = temp("first");
        let sample = sample_with(
            &root.join("state"),
            1_000,
            capacity(100_000, 60_000, 60_000),
            Some(counters(2_000, 1_000, 20)),
            "boot-a".to_string(),
            1_000.0,
            None,
        );
        assert_eq!(sample["used_kb"], 40_000);
        assert_eq!(sample["free_kb"], 60_000);
        assert_eq!(sample["used_pct"], 40);
        assert!(
            !sample.contains_key("read_kb"),
            "no baseline means no fabricated delta"
        );
        assert!(root.join("state/storage.json").exists());
    }

    #[test]
    fn second_sample_computes_rates_from_the_measured_interval() {
        let root = temp("second");
        let state_dir = root.join("state");
        let _ = sample_with(
            &state_dir,
            1_000,
            capacity(100_000, 60_000, 60_000),
            Some(counters(2_000, 1_000, 20)),
            "boot-a".to_string(),
            1_000.0,
            None,
        );
        let sample = sample_with(
            &state_dir,
            1_300,
            capacity(100_000, 60_000, 60_000),
            Some(counters(4_000, 2_200, 20_100)),
            "boot-a".to_string(),
            1_300.0,
            None,
        );
        assert_eq!(sample["read_kb"], 1_000);
        assert_eq!(sample["write_kb"], 600);
        // 20 100 ms busy over the 300 s interval = 6.7 %.
        assert_eq!(sample["io_util_pct"], 6.7);
    }

    #[test]
    fn a_reboot_skips_the_delta_series() {
        let root = temp("reboot");
        let state_dir = root.join("state");
        let _ = sample_with(
            &state_dir,
            1_000,
            capacity(100_000, 60_000, 60_000),
            Some(counters(2_000, 1_000, 20)),
            "boot-a".to_string(),
            1_000.0,
            None,
        );
        let sample = sample_with(
            &state_dir,
            1_300,
            capacity(100_000, 60_000, 60_000),
            Some(counters(10, 5, 1)),
            "boot-b".to_string(),
            1_300.0,
            None,
        );
        assert!(!sample.contains_key("read_kb"));
        assert!(sample.contains_key("used_kb"));
    }

    #[test]
    fn low_space_hysteresis_is_persisted_between_samples() {
        let root = temp("low-space");
        let state_dir = root.join("state");
        let high = capacity(100_000, 5_000, 5_000);
        let _ = sample_with(
            &state_dir,
            1_000,
            high,
            None,
            "boot-a".into(),
            1_000.0,
            None,
        );
        let baseline: Baseline =
            serde_json::from_str(&std::fs::read_to_string(state_dir.join("storage.json")).unwrap())
                .unwrap();
        assert!(baseline.warned_low, "95 % used must arm the warning");

        let recovered = sample_with(
            &state_dir,
            1_600,
            capacity(100_000, 20_000, 20_000),
            None,
            "boot-a".into(),
            1_600.0,
            None,
        );
        assert_eq!(recovered["used_pct"], 80);
        let baseline: Baseline =
            serde_json::from_str(&std::fs::read_to_string(state_dir.join("storage.json")).unwrap())
                .unwrap();
        assert!(!baseline.warned_low, "80 % used must clear the warning");
    }
}
