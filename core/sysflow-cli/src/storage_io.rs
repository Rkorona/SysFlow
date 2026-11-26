//! `sysflow storage io` — live throughput/IOPS/utilisation for the device behind `/data`.
//!
//! Rates need two readings, so this command sleeps for `--interval-ms` (default 1 s). The APK calls
//! it every 2 s while the page is visible; nothing is sampled in the background here.

use std::time::Instant;

use serde_json::{json, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::registry::Registry;

use sysflow_modules::storage::{capacity, device, diskstats};

pub fn run(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let interval_ms = flag(rest, "--interval-ms")
        .map(|value| {
            value.parse::<u64>().map_err(|_| {
                Error::invalid_arg(format!("--interval-ms expects a number, got {value:?}"))
            })
        })
        .transpose()?
        .unwrap_or(1_000)
        .clamp(300, 5_000);

    let fs = &reg.ctx().sysfs;
    let data_path = sysflow_modules::storage::default_data_path();
    let (mount, _sdcard) = capacity::storage_mounts(fs, &data_path)?;
    let device = device::for_mount(fs, &mount)
        .ok_or_else(|| Error::unsupported("no block device backs /data"))?;
    let before = diskstats::read(fs, &device.name).ok_or_else(|| {
        Error::unsupported(format!("{} is missing from /proc/diskstats", device.name))
    })?;

    let started = Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(interval_ms));
    let after = diskstats::read(fs, &device.name)
        .ok_or_else(|| Error::io(format!("{} disappeared from /proc/diskstats", device.name)))?;
    let elapsed_ms = started.elapsed().as_millis().max(1) as u64;
    let rates = diskstats::rates(before, after, elapsed_ms);

    Ok(json!({
        "device": device.name,
        "model": device.model,
        "scheduler": device.scheduler.as_ref().map(|scheduler| scheduler.current.clone()),
        "interval_ms": elapsed_ms,
        "read_kb": rates.read_kb,
        "write_kb": rates.write_kb,
        "read_iops": round1(rates.read_iops),
        "write_iops": round1(rates.write_iops),
        "util_pct": round1(rates.util_pct),
        "total_read_kb": after.read_sectors / 2,
        "total_write_kb": after.write_sectors / 2,
    }))
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn flag<'a>(args: &[&'a str], name: &str) -> Option<&'a str> {
    let position = args.iter().position(|arg| *arg == name)?;
    args.get(position + 1).copied()
}
