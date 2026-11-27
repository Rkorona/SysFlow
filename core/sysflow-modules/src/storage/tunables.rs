//! The writable half of the storage module: I/O scheduler and read-ahead.
//!
//! Both are queue knobs on the **physical** device backing `/data` (see [`super::device`]); both are
//! low risk, persisted and drift-repaired through the same path the CPU and memory modules use.
//! `io.trim` is deliberately a verb, not an item: it is a one-shot action, not a setting, and must
//! never be replayed at boot.

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::module::ApplyResult;
use sysflow_core::schema::{ItemType, Risk, SchemaItem};
use sysflow_core::sysfs::Sysfs;

use super::device::{self, BlockDevice};

pub const SCHEDULER: &str = "storage.scheduler";
pub const READ_AHEAD_KB: &str = "storage.read_ahead_kb";

/// Read-ahead values offered in the picker. The current value is added when the kernel reports
/// something outside this list, so `get` and `set` never disagree about whether a value is legal.
const READ_AHEAD_CHOICES: &[u64] = &[128, 256, 512, 1024, 2048, 4096];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadAhead {
    pub current: u64,
    pub options: Vec<u64>,
}

pub fn read_ahead_options(current: u64) -> Vec<u64> {
    let mut options = READ_AHEAD_CHOICES.to_vec();
    if !options.contains(&current) {
        options.push(current);
    }
    options.sort_unstable();
    options.dedup();
    options
}

pub fn schema_items(device: &BlockDevice) -> Vec<SchemaItem> {
    let mut items = Vec::new();
    if let Some(scheduler) = &device.scheduler {
        items.push(
            SchemaItem::new(SCHEDULER, ItemType::Enum)
                .options(
                    scheduler
                        .options
                        .iter()
                        .map(|option| json!(option))
                        .collect(),
                )
                .risk(Risk::Low)
                .persist(true),
        );
    }
    if let Some(current) = device.read_ahead_kb {
        items.push(
            SchemaItem::new(READ_AHEAD_KB, ItemType::IntEnum)
                .options(
                    read_ahead_options(current)
                        .into_iter()
                        .map(|value| json!(value))
                        .collect(),
                )
                .unit("kB")
                .risk(Risk::Low)
                .persist(true),
        );
    }
    items
}

/// Current values of the writable items, for `collect` / `drift`.
pub fn collect(device: &BlockDevice) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(scheduler) = &device.scheduler {
        out.insert(SCHEDULER.into(), json!(scheduler.current));
    }
    if let Some(read_ahead) = device.read_ahead_kb {
        out.insert(READ_AHEAD_KB.into(), json!(read_ahead));
    }
    out
}

fn read_back(fs: &Sysfs, device_name: &str) -> Map<String, Value> {
    let mut out = Map::new();
    if let Ok(text) = fs.read_str(&device::scheduler_path(device_name)) {
        out.insert(
            SCHEDULER.into(),
            json!(device::parse_scheduler(&text).current),
        );
    }
    if let Ok(text) = fs.read_str(&device::read_ahead_path(device_name)) {
        if let Ok(value) = text.trim().parse::<u64>() {
            out.insert(READ_AHEAD_KB.into(), json!(value));
        }
    }
    out
}

/// Apply a request to the data device, rolling back the first item if the second write fails.
pub fn apply(fs: &Sysfs, device: &BlockDevice, req: &Map<String, Value>) -> Result<ApplyResult> {
    let mut written: Vec<(String, String)> = Vec::new();
    let result =
        (|| -> Result<()> {
            if let Some(value) = req.get(SCHEDULER) {
                let wanted = value
                    .as_str()
                    .ok_or_else(|| Error::invalid_arg(format!("{SCHEDULER}: expected a string")))?;
                let path = device::scheduler_path(&device.name);
                let before = fs.read_str(&path)?;
                if !device.scheduler.as_ref().is_some_and(|scheduler| {
                    scheduler.options.iter().any(|option| option == wanted)
                }) {
                    return Err(Error::invalid_arg(format!(
                        "{SCHEDULER}: {wanted:?} is not one of the kernel's options"
                    )));
                }
                fs.write_str(&path, wanted)?;
                written.push((path, before));
            }
            if let Some(value) = req.get(READ_AHEAD_KB) {
                let wanted = value.as_i64().ok_or_else(|| {
                    Error::invalid_arg(format!("{READ_AHEAD_KB}: expected an integer"))
                })?;
                let current = device.read_ahead_kb.unwrap_or(0);
                if !read_ahead_options(current)
                    .iter()
                    .any(|option| *option as i64 == wanted)
                {
                    return Err(Error::invalid_arg(format!(
                        "{READ_AHEAD_KB}: {wanted} is not an offered value"
                    )));
                }
                let path = device::read_ahead_path(&device.name);
                let before = fs.read_str(&path)?;
                fs.write_str(&path, &wanted.to_string())?;
                written.push((path, before));
            }
            Ok(())
        })();
    if let Err(error) = result {
        for (path, before) in written.iter().rev() {
            let _ = fs.write_str(path, before);
        }
        return Err(error);
    }
    Ok(ApplyResult::from_readback(req, read_back(fs, &device.name)))
}

/// Drop persisted values this kernel no longer accepts (a scheduler removed by an update, a
/// read-ahead outside the offered set) instead of failing the whole boot replay.
pub fn sanitize_boot(device: &BlockDevice, req: &Map<String, Value>) -> Map<String, Value> {
    let mut out = req.clone();
    if let Some(value) = out.get(SCHEDULER).and_then(Value::as_str) {
        let keep = device
            .scheduler
            .as_ref()
            .is_some_and(|scheduler| scheduler.options.iter().any(|option| option == value));
        if !keep {
            out.remove(SCHEDULER);
        }
    }
    if let Some(value) = out.get(READ_AHEAD_KB).and_then(Value::as_i64) {
        let current = device.read_ahead_kb.unwrap_or(0);
        if !read_ahead_options(current)
            .iter()
            .any(|option| *option as i64 == value)
        {
            out.remove(READ_AHEAD_KB);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sysflow-storage-tunables-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn fixture(name: &str) -> (PathBuf, BlockDevice) {
        let root = temp(name);
        write(
            &root,
            "sys/block/sda/queue/scheduler",
            "[mq-deadline] kyber none\n",
        );
        write(&root, "sys/block/sda/queue/read_ahead_kb", "128\n");
        let fs = Sysfs::new(&root);
        let device = device::get(&fs, "sda").unwrap();
        (root, device)
    }

    #[test]
    fn read_ahead_options_include_the_current_value_once() {
        let options = read_ahead_options(128);
        assert!(options.contains(&128));
        assert_eq!(options.iter().filter(|value| **value == 128).count(), 1);
        let options = read_ahead_options(777);
        assert!(
            options.contains(&777),
            "a ROM value outside the list stays selectable"
        );
    }

    #[test]
    fn apply_writes_both_items_and_reads_them_back() {
        let (root, device) = fixture("apply");
        let fs = Sysfs::new(&root);
        let mut req = Map::new();
        req.insert(SCHEDULER.into(), json!("kyber"));
        req.insert(READ_AHEAD_KB.into(), json!(1024));
        let result = apply(&fs, &device, &req).unwrap();
        assert_eq!(result.applied["storage.scheduler"], "kyber");
        assert_eq!(result.applied["storage.read_ahead_kb"], 1024);
        assert!(result.mismatch.is_empty());
        assert_eq!(
            std::fs::read_to_string(root.join("sys/block/sda/queue/scheduler"))
                .unwrap()
                .trim(),
            "kyber"
        );
    }

    #[test]
    fn apply_rejects_options_the_kernel_does_not_offer() {
        let (root, device) = fixture("reject");
        let fs = Sysfs::new(&root);
        let mut req = Map::new();
        req.insert(SCHEDULER.into(), json!("bfq"));
        let error = apply(&fs, &device, &req).unwrap_err();
        assert_eq!(error.code.as_str(), "invalid_arg");
        assert_eq!(
            std::fs::read_to_string(root.join("sys/block/sda/queue/scheduler"))
                .unwrap()
                .trim(),
            "[mq-deadline] kyber none",
            "a rejected request must not write"
        );
    }

    #[test]
    fn sanitize_drops_values_this_kernel_cannot_accept() {
        let (_root, device) = fixture("sanitize");
        let mut req = Map::new();
        req.insert(SCHEDULER.into(), json!("bfq"));
        req.insert(READ_AHEAD_KB.into(), json!(256));
        let out = sanitize_boot(&device, &req);
        assert!(out.get(SCHEDULER).is_none());
        assert_eq!(out[READ_AHEAD_KB], 256);
    }
}
