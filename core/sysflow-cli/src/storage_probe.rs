//! `sysflow storage probe` — hidden Step 1 diagnostic for the storage module.
//!
//! Not part of the protocol: it dumps the raw mount table, `statvfs` numbers, every block device
//! with its queue knobs, and the `/proc/diskstats` lines for the device backing `/data`, so the
//! device matrix in `docs/modules/storage.md` can be filled in from measurements instead of guesses.

use std::time::Duration;

use serde_json::{json, Map, Value};

use sysflow_core::command;
use sysflow_core::error::Result;
use sysflow_core::registry::Registry;
use sysflow_core::sysfs::Sysfs;

use sysflow_modules::storage::{capacity, device, diskstats, trim_binary};

pub fn run(reg: &Registry) -> Result<Value> {
    let fs = &reg.ctx().sysfs;
    let mut out = Map::new();

    let mounts = capacity::mounts(fs).unwrap_or_default();
    out.insert(
        "mounts".into(),
        Value::Array(
            mounts
                .iter()
                .take(64)
                .map(|mount| {
                    json!({
                        "device": mount.device,
                        "mount": mount.mount,
                        "fstype": mount.fstype,
                    })
                })
                .collect(),
        ),
    );

    let data_path = sysflow_modules::storage::default_data_path();
    let storage = capacity::storage_mounts(fs, &data_path);
    match &storage {
        Ok((data, sdcard)) => {
            out.insert(
                "data".into(),
                json!({
                    "mount": data.mount,
                    "device": data.device,
                    "fstype": data.fstype,
                    "capacity": capacity::capacity(&data.mount).ok().map(|cap| json!({
                        "total_kb": cap.total_kb,
                        "free_kb": cap.free_kb,
                        "avail_kb": cap.avail_kb,
                    })),
                }),
            );
            out.insert(
                "sdcard".into(),
                sdcard.as_ref().map_or(Value::Null, |mount| {
                    json!({
                        "mount": mount.mount,
                        "device": mount.device,
                        "capacity": capacity::capacity(&mount.mount).ok().map(|cap| json!({
                            "total_kb": cap.total_kb,
                            "free_kb": cap.free_kb,
                            "avail_kb": cap.avail_kb,
                        })),
                    })
                }),
            );
        }
        Err(error) => {
            out.insert("data_error".into(), json!(error.to_string()));
        }
    }

    out.insert(
        "block_devices".into(),
        Value::Array(
            device::list(fs)
                .into_iter()
                .map(|device| {
                    json!({
                        "name": device.name,
                        "size_kb": device.size_kb,
                        "model": device.model,
                        "rotational": device.rotational,
                        "scheduler": device.scheduler.as_ref().map(|scheduler| json!({
                            "current": scheduler.current,
                            "options": scheduler.options,
                        })),
                        "read_ahead_kb": device.read_ahead_kb,
                        "nr_requests": device.nr_requests,
                    })
                })
                .collect(),
        ),
    );

    if let Ok((data, _)) =
        capacity::storage_mounts(fs, &sysflow_modules::storage::default_data_path())
    {
        if let Some(physical) = device::for_mount(fs, &data) {
            out.insert(
                "data_device".into(),
                json!({
                    "name": physical.name,
                    "physical": device::resolve_physical(fs, &physical.name),
                    "counters": diskstats::read(fs, &physical.name).map(|counters| json!({
                        "read_ios": counters.read_ios,
                        "read_sectors": counters.read_sectors,
                        "write_ios": counters.write_ios,
                        "write_sectors": counters.write_sectors,
                        "io_ms": counters.io_ms,
                    })),
                    "diskstats_line": diskstats_line(fs, &physical.name),
                }),
            );
        }
    }

    out.insert(
        "trim".into(),
        json!({
            "binary": trim_binary(fs),
            "help": trim_binary(fs).map(|binary| run_cmd(binary, &["--help"])),
        }),
    );
    out.insert("df_data".into(), run_cmd("df", &["-k", "/data"]));
    out.insert("block_entries".into(), json!(list_entries(fs)));
    Ok(Value::Object(out))
}

fn diskstats_line(fs: &Sysfs, device: &str) -> Option<String> {
    fs.read_lossy("/proc/diskstats")
        .ok()?
        .lines()
        .find(|line| line.split_whitespace().nth(2) == Some(device))
        .map(str::to_string)
}

fn list_entries(fs: &Sysfs) -> Vec<String> {
    fs.list_dir("/sys/block").unwrap_or_default()
}

fn run_cmd(bin: &str, args: &[&str]) -> Value {
    match command::run(bin, args, Duration::from_secs(3)) {
        Ok(text) => json!({
            "ok": true,
            "first_lines": text.lines().take(5).map(|line| line.trim_end().to_string()).collect::<Vec<_>>(),
        }),
        Err(error) => json!({"ok": false, "error": error.to_string()}),
    }
}
