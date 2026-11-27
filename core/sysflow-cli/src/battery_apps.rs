//! `sysflow battery apps` — per-UID power attribution from BatteryStats.
//!
//! This is deliberately **not** part of `snapshot`: `dumpsys batterystats` takes hundreds of
//! milliseconds to seconds and prints megabytes, so it can only be pulled on demand. Results are
//! cached under `<data_dir>/state/battery_apps.json` for a minute so opening the battery screen
//! twice in a row does not re-run the dump.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Map, Value};

use sysflow_core::batterystats::{
    aid_label, parse_package_uids, parse_power_section, resolve_uid, PowerReport,
};
use sysflow_core::cache;
use sysflow_core::error::{Error, Result};
use sysflow_core::registry::Registry;

/// Safety net: a wedged `dumpsys` must not hang the caller forever.
const DUMP_TIMEOUT: Duration = Duration::from_secs(8);

pub fn run(reg: &Registry, limit: usize, refresh: bool) -> Result<Value> {
    let cache_path = cache_path(reg);
    if !refresh {
        if let Some(cached) = read_cache(&cache_path) {
            return Ok(cached);
        }
    }

    let report = match dump_power_section()? {
        Some(report) => report,
        None => {
            return Ok(json!({
                "available": false,
                "reason": "batterystats did not expose a power estimate section",
                "entries": [],
            }))
        }
    };

    let packages = read_package_uids();
    let mut value = report_to_json(&report, &packages, limit);
    value["available"] = json!(true);
    value["generated_at"] = json!(cache::now_secs());
    value["cached"] = json!(false);

    if let Some(obj) = value.as_object() {
        write_cache(&cache_path, obj);
    }
    Ok(value)
}

fn report_to_json(
    report: &PowerReport,
    packages: &std::collections::BTreeMap<u32, Vec<String>>,
    limit: usize,
) -> Value {
    let mut value = report.to_json(limit);
    if let Some(entries) = value.get_mut("entries").and_then(Value::as_array_mut) {
        for entry in entries {
            let uid_text = entry["uid"].as_str().unwrap_or_default().to_string();
            let numeric = entry["numeric_uid"].as_u64().map(|n| n as u32);
            // `u0a404` is BatteryStats' short form for app id 404 under user 0, i.e. UID 10404.
            // Resolve the uid first: a device may have no package for it (uninstalled app, system
            // daemon), and the numeric uid is still useful to the caller.
            let resolved = numeric.or_else(|| resolve_uid(&uid_text));
            if let Some(uid) = resolved {
                entry["numeric_uid"] = json!(uid);
                if let Some(names) = packages.get(&uid) {
                    entry["packages"] = json!(names);
                }
                if entry["name"].is_null() {
                    if let Some(label) = aid_label(uid) {
                        entry["name"] = json!(label);
                    }
                }
            }
            if entry["name"].is_null() {
                let from_package = entry["packages"]
                    .as_array()
                    .and_then(|names| names.first())
                    .cloned();
                entry["name"] = match from_package {
                    Some(name) => name,
                    // Never a null label: fall back to a readable uid so the row is still usable.
                    None => json!(match resolved {
                        Some(uid) => format!("UID {uid}"),
                        None => format!("UID {uid_text}"),
                    }),
                };
            }
        }
    }
    value
}

fn cache_path(reg: &Registry) -> PathBuf {
    reg.ctx().data_dir.join("state").join("battery_apps.json")
}

/// Cached results are reused for a minute; `--refresh` bypasses them.
fn read_cache(path: &Path) -> Option<Value> {
    let mut value = cache::read_fresh(path, cache::TTL_SECS)?;
    value["cached"] = json!(true);
    Some(value)
}

fn write_cache(path: &Path, value: &Map<String, Value>) {
    cache::write(path, &json!(value));
}

/// Run `dumpsys batterystats --charged` and stop reading as soon as the power section ends.
///
/// Stopping early is what keeps the call cheap: the remainder of the dump is megabytes, and closing
/// the pipe makes `dumpsys` abort with a broken-pipe message on stderr, which we discard.
fn dump_power_section() -> Result<Option<PowerReport>> {
    let mut child = Command::new("dumpsys")
        .args(["batterystats", "--charged"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Error::io(format!("cannot run dumpsys: {e}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::io("dumpsys produced no stdout"))?;

    let guard = Arc::new(Mutex::new(Some(child)));
    // The watchdog waits on a channel instead of sleeping, so a fast dump does not pay the timeout.
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let watchdog = {
        let guard = Arc::clone(&guard);
        thread::spawn(move || match done_rx.recv_timeout(DUMP_TIMEOUT) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(mut slot) = guard.lock() {
                    if let Some(child) = slot.as_mut() {
                        let _ = child.kill();
                    }
                }
            }
        })
    };

    let result = parse_power_section(
        BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok),
    );

    if let Ok(mut slot) = guard.lock() {
        if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    drop(done_tx);
    watchdog.join().ok();
    Ok(result)
}

/// UID -> package names. Best effort: a device without a usable package service simply yields no
/// names and the ranking falls back to raw UIDs.
fn read_package_uids() -> std::collections::BTreeMap<u32, Vec<String>> {
    let output = Command::new("cmd")
        .args(["package", "list", "packages", "-U"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
        .and_then(|child| child.wait_with_output().ok());
    match output {
        Some(output) => parse_package_uids(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        ),
        None => std::collections::BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_batterystats_short_uid_form() {
        // The shared helper also accepts plain numeric uids; the caller prefers the parsed value.
        assert_eq!(resolve_uid("u0a404"), Some(10404));
        assert_eq!(resolve_uid("u0a166"), Some(10166));
        assert_eq!(resolve_uid("u10a7"), Some(1_010_007));
        assert_eq!(resolve_uid("1000"), Some(1000));
        assert_eq!(resolve_uid("garbage"), None);
    }
}
