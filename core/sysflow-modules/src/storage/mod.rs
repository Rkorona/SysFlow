//! Storage module — capacity, I/O throughput, schedulers, read-ahead, TRIM and history.
//!
//! Scope (`docs/storage-module-plan.md`): the read-only half is always available when `/data` is
//! mounted; the two tunables (`storage.scheduler`, `storage.read_ahead_kb`) apply to the **physical**
//! device behind `/data`; `trim` and `usage` are verbs because they are one-shot actions, not
//! settings. Nothing here touches `config.json` except through the standard `set --persist` path.

pub mod capacity;
pub mod device;
pub mod diskstats;
pub mod history;
pub mod partitions;
pub mod tunables;
pub mod usage;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use sysflow_core::command;
use sysflow_core::error::{Error, Result};
use sysflow_core::event::EventStore;
use sysflow_core::module::{ApplyResult, Caps, Module, Sample};
use sysflow_core::schema::{ItemType, Schema, SchemaItem};
use sysflow_core::sysfs::Sysfs;

use capacity::Mount;
use device::BlockDevice;

// Read-only capacity items (whole-device view).
pub const DATA_TOTAL_KB: &str = "storage.data.total_kb";
pub const DATA_USED_KB: &str = "storage.data.used_kb";
pub const DATA_FREE_KB: &str = "storage.data.free_kb";
pub const DATA_AVAIL_KB: &str = "storage.data.avail_kb";
pub const DATA_USED_PCT: &str = "storage.data.used_pct";
// Emulated storage, only when it is a separate filesystem.
pub const SDCARD_TOTAL_KB: &str = "storage.sdcard.total_kb";
pub const SDCARD_USED_KB: &str = "storage.sdcard.used_kb";
pub const SDCARD_FREE_KB: &str = "storage.sdcard.free_kb";
pub const SDCARD_AVAIL_KB: &str = "storage.sdcard.avail_kb";
pub const SDCARD_USED_PCT: &str = "storage.sdcard.used_pct";
// Device identity and queue geometry.
pub const DEVICE_NAME: &str = "storage.device.name";
pub const DEVICE_SIZE_KB: &str = "storage.device.size_kb";
pub const DEVICE_MODEL: &str = "storage.device.model";
pub const DEVICE_ROTATIONAL: &str = "storage.device.rotational";
pub const DEVICE_HEALTH_PCT: &str = "storage.device.health_pct";
// I/O counters since boot (the throughput is a verb/history series: it needs an interval).
pub const IO_READ_KB: &str = "storage.io.read_kb";
pub const IO_WRITE_KB: &str = "storage.io.write_kb";
pub const IO_READ_IOS: &str = "storage.io.read_ios";
pub const IO_WRITE_IOS: &str = "storage.io.write_ios";

pub struct StorageModule {
    fs: Sysfs,
    /// `None` in tests that only exercise detection.
    state_dir: Option<PathBuf>,
    /// Derived from `state_dir.parent()`, like the memory and wake-lock modules.
    events: Option<EventStore>,
    /// `/data` in production; a temporary directory in host tests. Kept as a field instead of a
    /// hard-coded constant so `collect` / `schema` can be exercised without a device.
    data_path: String,
}

/// Everything a snapshot needs, read once.
pub struct Layout {
    pub data_mount: Mount,
    pub data_capacity: capacity::Capacity,
    pub sdcard: Option<(Mount, capacity::Capacity)>,
    pub device: Option<BlockDevice>,
}

impl StorageModule {
    pub fn new(fs: Sysfs) -> Self {
        StorageModule {
            fs,
            state_dir: None,
            events: None,
            data_path: "/data".to_string(),
        }
    }

    /// Production constructor: history baseline under `<data_dir>/state/storage.json`.
    pub fn with_state_dir(fs: Sysfs, state_dir: impl Into<PathBuf>) -> Self {
        let state_dir = state_dir.into();
        let events = state_dir.parent().map(EventStore::new);
        StorageModule {
            fs,
            state_dir: Some(state_dir),
            events,
            data_path: "/data".to_string(),
        }
    }

    /// Point the module at another mount path (host tests, or a future alternate data partition).
    pub fn with_data_path(mut self, path: impl Into<String>) -> Self {
        self.data_path = path.into();
        self
    }

    fn layout(&self) -> Option<Layout> {
        let (data_mount, sdcard_mount) =
            capacity::storage_mounts(&self.fs, &self.data_path).ok()?;
        let data_capacity = capacity::capacity(&data_mount.mount).ok()?;
        let sdcard = sdcard_mount.and_then(|mount| {
            capacity::capacity(&mount.mount)
                .ok()
                .map(|cap| (mount, cap))
        });
        let device = device::for_mount(&self.fs, &data_mount).filter(|device| device.can_tune());
        Some(Layout {
            data_mount,
            data_capacity,
            sdcard,
            device,
        })
    }

    fn capacity_values(out: &mut Map<String, Value>, prefix: &str, state: &capacity::Capacity) {
        let used = state.total_kb.saturating_sub(state.free_kb);
        let pct = used
            .saturating_mul(100)
            .checked_div(state.total_kb)
            .unwrap_or(0);
        out.insert(format!("{prefix}.total_kb"), json!(state.total_kb));
        out.insert(format!("{prefix}.used_kb"), json!(used));
        out.insert(format!("{prefix}.free_kb"), json!(state.free_kb));
        out.insert(format!("{prefix}.avail_kb"), json!(state.avail_kb));
        out.insert(format!("{prefix}.used_pct"), json!(pct));
    }
}

impl Module for StorageModule {
    fn id(&self) -> &'static str {
        "storage"
    }

    fn probe(&self) -> Caps {
        let Ok((data, sdcard)) = capacity::storage_mounts(&self.fs, &self.data_path) else {
            return Caps::unsupported(format!("/proc/mounts has no {} entry", self.data_path));
        };
        if capacity::capacity(&data.mount).is_err() {
            return Caps::unsupported(format!("statvfs {} failed", data.mount));
        }
        let mut features = vec!["data"];
        let Some(layout_device) = device::for_mount(&self.fs, &data) else {
            return Caps::supported(&features);
        };
        if sdcard.is_some() {
            features.push("sdcard");
        }
        if self.fs.exists("/proc/diskstats") {
            features.push("io");
        }
        if layout_device.scheduler.is_some() {
            features.push("scheduler");
        }
        if layout_device.read_ahead_kb.is_some() {
            features.push("read_ahead");
        }
        if trim_available(&self.fs) {
            features.push("trim");
        }
        if self.fs.exists(&self.data_path) {
            features.push("usage");
        }
        Caps::supported(&features)
    }

    fn schema(&self) -> Schema {
        let caps = self.probe();
        let mut items = Vec::new();
        if caps.supported {
            for (id, unit) in [
                (DATA_TOTAL_KB, "kB"),
                (DATA_USED_KB, "kB"),
                (DATA_FREE_KB, "kB"),
                (DATA_AVAIL_KB, "kB"),
            ] {
                items.push(SchemaItem::new(id, ItemType::Readonly).unit(unit));
            }
            items.push(SchemaItem::new(DATA_USED_PCT, ItemType::Readonly).unit("%"));
            for (id, unit) in [
                (SDCARD_TOTAL_KB, "kB"),
                (SDCARD_USED_KB, "kB"),
                (SDCARD_FREE_KB, "kB"),
                (SDCARD_AVAIL_KB, "kB"),
            ] {
                items.push(SchemaItem::new(id, ItemType::Readonly).unit(unit));
            }
            items.push(SchemaItem::new(SDCARD_USED_PCT, ItemType::Readonly).unit("%"));
            items.push(SchemaItem::new(DEVICE_NAME, ItemType::Readonly));
            items.push(SchemaItem::new(DEVICE_SIZE_KB, ItemType::Readonly).unit("kB"));
            items.push(SchemaItem::new(DEVICE_MODEL, ItemType::Readonly));
            items.push(SchemaItem::new(DEVICE_ROTATIONAL, ItemType::Readonly));
            items.push(SchemaItem::new(DEVICE_HEALTH_PCT, ItemType::Readonly).unit("%"));
            for id in [IO_READ_KB, IO_WRITE_KB] {
                items.push(SchemaItem::new(id, ItemType::Readonly).unit("kB"));
            }
            for id in [IO_READ_IOS, IO_WRITE_IOS] {
                items.push(SchemaItem::new(id, ItemType::Readonly));
            }
            if let Some(layout) = self.layout() {
                if let Some(device) = &layout.device {
                    items.extend(tunables::schema_items(device));
                }
            }
        }
        Schema {
            module: "storage".to_string(),
            supported: caps.supported,
            items,
        }
    }

    fn collect(&self, keys: Option<&[String]>) -> Result<Value> {
        let layout = self.layout().ok_or_else(|| {
            Error::unsupported(format!(
                "/proc/mounts has no usable {} entry",
                self.data_path
            ))
        })?;
        let mut out = Map::new();
        StorageModule::capacity_values(&mut out, "storage.data", &layout.data_capacity);
        if let Some((_, state)) = &layout.sdcard {
            StorageModule::capacity_values(&mut out, "storage.sdcard", state);
        }
        if let Some(device) = &layout.device {
            out.insert(DEVICE_NAME.into(), json!(device.name));
            if let Some(size) = device.size_kb {
                out.insert(DEVICE_SIZE_KB.into(), json!(size));
            }
            if let Some(model) = &device.model {
                out.insert(DEVICE_MODEL.into(), json!(model));
            }
            out.insert(DEVICE_ROTATIONAL.into(), json!(device.rotational));
            if let Some(health) = device.health_pct {
                out.insert(DEVICE_HEALTH_PCT.into(), json!(health));
            }
            for (key, value) in tunables::collect(device) {
                out.insert(key, value);
            }
            if let Some(counters) = diskstats::read(&self.fs, &device.name) {
                out.insert(IO_READ_KB.into(), json!(counters.read_sectors / 2));
                out.insert(IO_WRITE_KB.into(), json!(counters.write_sectors / 2));
                out.insert(IO_READ_IOS.into(), json!(counters.read_ios));
                out.insert(IO_WRITE_IOS.into(), json!(counters.write_ios));
            }
        }
        if let Some(keys) = keys {
            out.retain(|key, _| keys.iter().any(|wanted| wanted == key));
        }
        Ok(Value::Object(out))
    }

    fn apply(&self, req: &Map<String, Value>) -> Result<ApplyResult> {
        let layout = self
            .layout()
            .ok_or_else(|| Error::unsupported("no block device backing /data"))?;
        let device = layout
            .device
            .ok_or_else(|| Error::unsupported("the data device exposes no queue tunables"))?;
        // Validate against the schema first: an unknown key must be rejected before any write.
        self.schema().validate_request(req)?;
        tunables::apply(&self.fs, &device, req)
    }

    fn sanitize_boot(&self, req: &Map<String, Value>) -> Map<String, Value> {
        match self.layout().and_then(|layout| layout.device) {
            Some(device) => tunables::sanitize_boot(&device, req),
            None => {
                let mut out = req.clone();
                out.remove(tunables::SCHEDULER);
                out.remove(tunables::READ_AHEAD_KB);
                out
            }
        }
    }

    fn sample(&self) -> Option<Result<Sample>> {
        let state_dir = self.state_dir.as_deref()?;
        history::sample_at(
            &self.fs,
            state_dir,
            sysflow_core::history::now_secs(),
            self.events.as_ref(),
            &self.data_path,
        )
        .map(Ok)
    }

    /// Five minutes: write-volume history stays meaningful and fits the 30-day line cap.
    fn sample_interval(&self) -> Option<Duration> {
        self.state_dir
            .as_ref()
            .map(|_| Duration::from_secs(history::SAMPLE_INTERVAL_S))
    }
}

/// The mount path the storage module should describe: `/data` on device, the host override
/// (`SYSFLOW_STORAGE_DATA_PATH`) in tests and host runs.
pub fn default_data_path() -> String {
    std::env::var("SYSFLOW_STORAGE_DATA_PATH")
        .ok()
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "/data".to_string())
}

/// The trim binary, if this ROM ships one.
pub fn trim_binary(fs: &Sysfs) -> Option<&'static str> {
    ["/system/bin/fstrim", "/system/xbin/fstrim", "/sbin/fstrim"]
        .into_iter()
        .find(|candidate| fs.exists(candidate))
}

pub fn trim_available(fs: &Sysfs) -> bool {
    trim_binary(fs).is_some()
}

/// One-shot `fstrim -v <path>`. Not persisted, not part of drift, never replayed at boot.
pub fn trim(fs: &Sysfs, path: &str) -> Result<Value> {
    let binary = trim_binary(fs).unwrap_or("fstrim");
    // Resolve through Sysfs so a fake root runs its own fixture; on device the root is `/` and this
    // is the same absolute path.
    let executable = if binary.starts_with('/') {
        fs.resolve(binary).to_string_lossy().into_owned()
    } else {
        binary.to_string()
    };
    let started = Instant::now();
    let output = command::run(&executable, &["-v", path], Duration::from_secs(120))?;
    Ok(json!({
        "path": path,
        "binary": binary,
        "trimmed_bytes": parse_trimmed_bytes(&output),
        "output": output.trim(),
        "ms": started.elapsed().as_millis() as u64,
    }))
}

/// `fstrim -v` prints `path: 123456 bytes trimmed`; other ROMs append the unit differently, so the
/// number is matched by position rather than by a fixed suffix.
fn parse_trimmed_bytes(output: &str) -> Option<u64> {
    let tokens: Vec<&str> = output.split_whitespace().collect();
    tokens
        .iter()
        .position(|token| *token == "bytes")
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| tokens[index].parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sysflow-storage-module-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data")).unwrap();
        dir
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn fixture(name: &str) -> (PathBuf, StorageModule) {
        let root = temp(name);
        let data = root.join("data");
        write(
            &root,
            "proc/mounts",
            &format!("/dev/block/dm-5 {} ext4 rw 0 0\n", data.display()),
        );
        write(
            &root,
            "proc/diskstats",
            "8 0 sda 100 0 2000 10 50 0 1000 10 0 20 30\n",
        );
        write(&root, "sys/block/dm-5/slaves/sda", "");
        write(
            &root,
            "sys/block/sda/queue/scheduler",
            "[mq-deadline] kyber none\n",
        );
        write(&root, "sys/block/sda/queue/read_ahead_kb", "128\n");
        write(&root, "sys/block/sda/queue/nr_requests", "64\n");
        write(&root, "sys/block/sda/size", "2000000\n");
        write(&root, "sys/block/sda/device/model", "Samsung UFS\n");
        write(&root, "system/bin/fstrim", "");
        let fs = Sysfs::new(&root);
        let module = StorageModule::with_state_dir(fs, root.join("state"))
            .with_data_path(data.to_string_lossy().into_owned());
        (root, module)
    }

    #[test]
    fn probe_and_schema_expose_capacity_and_the_two_tunables() {
        let (_root, module) = fixture("schema");
        let caps = module.probe();
        assert!(caps.supported, "reason: {:?}", caps.reason);
        for feature in ["data", "io", "scheduler", "read_ahead", "trim"] {
            assert!(
                caps.features.iter().any(|f| f == feature),
                "missing {feature}"
            );
        }
        let schema = module.schema();
        assert!(schema.supported);
        let ids: Vec<&str> = schema.items.iter().map(|item| item.id.as_str()).collect();
        assert!(ids.contains(&tunables::SCHEDULER));
        assert!(ids.contains(&tunables::READ_AHEAD_KB));
        assert!(ids.contains(&DATA_USED_PCT));
    }

    #[test]
    fn collect_reports_capacity_device_and_io() {
        let (_root, module) = fixture("collect");
        let value = module.collect(None).unwrap();
        assert!(value[DATA_TOTAL_KB].as_u64().unwrap() > 0);
        assert!(value[DATA_USED_PCT].as_u64().unwrap() <= 100);
        assert_eq!(value[DEVICE_NAME], "sda");
        assert_eq!(value[tunables::SCHEDULER], "mq-deadline");
        assert_eq!(value[tunables::READ_AHEAD_KB], 128);
        assert_eq!(value[IO_READ_KB], 1000);
        assert_eq!(value[IO_WRITE_KB], 500);
    }

    #[test]
    fn apply_changes_the_scheduler_and_read_ahead_and_reads_back() {
        let (_root, module) = fixture("apply");
        let mut req = Map::new();
        req.insert(tunables::SCHEDULER.into(), json!("kyber"));
        req.insert(tunables::READ_AHEAD_KB.into(), json!(1024));
        let result = module.apply(&req).unwrap();
        assert_eq!(result.applied[tunables::SCHEDULER], "kyber");
        assert_eq!(result.applied[tunables::READ_AHEAD_KB], 1024);
        assert!(result.mismatch.is_empty());
    }

    #[test]
    fn sanitize_drops_values_this_device_rejects() {
        let (_root, module) = fixture("sanitize");
        let mut req = Map::new();
        req.insert(tunables::SCHEDULER.into(), json!("bfq"));
        req.insert(tunables::READ_AHEAD_KB.into(), json!(256));
        let out = module.sanitize_boot(&req);
        assert!(out.get(tunables::SCHEDULER).is_none());
        assert_eq!(out[tunables::READ_AHEAD_KB], 256);
    }

    #[test]
    fn a_missing_data_mount_is_unsupported_rather_than_an_error() {
        let root = temp("no-data");
        write(&root, "proc/mounts", "tmpfs /mnt tmpfs rw 0 0\n");
        let module = StorageModule::new(Sysfs::new(&root));
        assert!(!module.probe().supported);
        assert!(!module.schema().supported);
        assert!(module.collect(None).is_err());
    }

    #[test]
    fn trim_output_parses_the_byte_count() {
        assert_eq!(
            parse_trimmed_bytes("/data: 123456 bytes trimmed\n"),
            Some(123_456)
        );
        assert_eq!(parse_trimmed_bytes("nothing here"), None);
    }
}
