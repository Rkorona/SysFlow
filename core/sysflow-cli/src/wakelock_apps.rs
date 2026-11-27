//! `sysflow wakelock apps` — per-app partial wake-lock time from BatteryStats.
//!
//! Complements `wakelock held` (who holds a lock *right now*) and `wakelock.ranked` (which kernel
//! wakeup source woke the device): this verb answers "which app accumulated the most wake-lock time
//! since the last charge".
//!
//! It is a verb rather than a snapshot item because it needs `dumpsys batterystats --charged`: a
//! 4 000+ line dump that takes hundreds of milliseconds. The section sits hundreds of lines in, so
//! the reader stops as soon as it ends instead of draining the whole dump.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use sysflow_core::batterystats::{
    aid_label, read_package_uids, resolve_uid, WakeLockScanner, WakeLockStat,
};
use sysflow_core::cache;
use sysflow_core::error::{Error, Result};
use sysflow_core::registry::Registry;

/// Generous: the dump is large, but a wedged system server must not hang the caller.
const DUMP_TIMEOUT: Duration = Duration::from_secs(15);

pub fn run(reg: &Registry, limit: usize, refresh: bool) -> Result<Value> {
    let cache_path = reg.ctx().data_dir.join("state").join("wakelock_apps.json");
    if !refresh {
        if let Some(mut cached) = cache::read_fresh(&cache_path, cache::TTL_SECS) {
            cached["cached"] = json!(true);
            return Ok(cached);
        }
    }

    let stats = match dump_wake_locks()? {
        Some(stats) => stats,
        None => {
            return Ok(json!({
                "available": false,
                "reason": "batterystats did not expose an 'All partial wake locks' section",
                "entries": [],
            }))
        }
    };

    let packages = read_package_uids();
    let mut value = to_json(&stats, &packages, limit);
    value["available"] = json!(true);
    value["generated_at"] = json!(cache::now_secs());
    value["cached"] = json!(false);
    cache::write(&cache_path, &value);
    Ok(value)
}

/// Aggregate the (uid, tag) rows by UID, biggest first.
fn to_json(stats: &[WakeLockStat], packages: &BTreeMap<u32, Vec<String>>, limit: usize) -> Value {
    struct Agg {
        uid: String,
        numeric: Option<u32>,
        total_ms: i64,
        count: u32,
        tags: Vec<(String, i64, u32)>,
    }

    let mut by_uid: BTreeMap<String, Agg> = BTreeMap::new();
    for stat in stats {
        let entry = by_uid.entry(stat.uid.clone()).or_insert_with(|| Agg {
            uid: stat.uid.clone(),
            numeric: stat.numeric_uid.or_else(|| resolve_uid(&stat.uid)),
            total_ms: 0,
            count: 0,
            tags: Vec::new(),
        });
        entry.total_ms = entry.total_ms.saturating_add(stat.total_ms);
        entry.count = entry.count.saturating_add(stat.count);
        entry
            .tags
            .push((stat.tag.clone(), stat.total_ms, stat.count));
    }

    let mut rows: Vec<Agg> = by_uid.into_values().collect();
    for row in &mut rows {
        row.tags
            .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        row.tags.truncate(3);
    }
    rows.sort_by(|a, b| b.total_ms.cmp(&a.total_ms).then_with(|| a.uid.cmp(&b.uid)));

    let total_ms: i64 = rows.iter().map(|row| row.total_ms).sum();
    let mut entries = Vec::new();
    for row in rows.into_iter().take(limit.max(1)) {
        let packages_for_uid = row
            .numeric
            .and_then(|uid| packages.get(&uid))
            .cloned()
            .unwrap_or_default();
        let label = packages_for_uid
            .first()
            .cloned()
            .or_else(|| row.numeric.and_then(aid_label).map(str::to_string))
            .unwrap_or_else(|| match row.numeric {
                Some(uid) => format!("UID {uid}"),
                None => format!("UID {}", row.uid),
            });
        let tags: Vec<Value> = row
            .tags
            .iter()
            .map(|(tag, ms, count)| json!({ "tag": tag, "total_ms": ms, "count": count }))
            .collect();
        entries.push(json!({
            "uid": row.uid,
            "numeric_uid": row.numeric,
            "name": label,
            "packages": packages_for_uid,
            "total_ms": row.total_ms,
            "count": row.count,
            "share": if total_ms > 0 { row.total_ms as f64 / total_ms as f64 } else { 0.0 },
            "tags": tags,
        }));
    }

    json!({ "total_ms": total_ms, "entries": entries })
}

/// Stream `dumpsys batterystats --charged` until the wake-lock section ends, then kill the child.
fn dump_wake_locks() -> Result<Option<Vec<WakeLockStat>>> {
    let mut child = Command::new("dumpsys")
        .args(["batterystats", "--charged"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Error::io(format!("cannot run dumpsys: {e}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::io("dumpsys produced no stdout"))?;

    let guard = Arc::new(Mutex::new(Some(child)));
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

    let mut scanner = WakeLockScanner::new();
    for line in BufReader::new(stdout)
        .lines()
        .map_while(std::result::Result::ok)
    {
        if scanner.push(&line) {
            break;
        }
    }
    let result = scanner.finish();

    // Closing the pipe early makes `dumpsys` abort with EPIPE; discard its status either way.
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

#[cfg(test)]
mod tests {
    use super::*;
    use sysflow_core::batterystats::parse_wake_lock_section;

    const DUMP: &str = "All partial wake locks:\n\
  Wake lock 1000 AnyMotionDetector: 1m 16s 158ms (11 times) max=9942 actual=88161 realtime\n\
  Wake lock u0a346 *job*e//SessionWorker/@eM_ADDR/x: 28s 608ms (3 times) max=20422 realtime\n\
  Wake lock u0a346 AudioMix: 1s 500ms (2 times) max=1000 realtime\n\
  Wake lock u0a229 Collection: 9s 937ms (3 times) max=3473 realtime\n";

    fn packages() -> BTreeMap<u32, Vec<String>> {
        let mut map = BTreeMap::new();
        map.insert(10_346, vec!["com.example.worker".to_string()]);
        map
    }

    fn rows() -> Vec<WakeLockStat> {
        parse_wake_lock_section(DUMP.lines().map(str::to_string)).expect("section")
    }

    #[test]
    fn aggregates_per_uid_and_sorts_by_time() {
        let value = to_json(&rows(), &packages(), 10);
        let entries = value["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        // UID 1000: 76 158 ms from one tag.
        assert_eq!(entries[0]["numeric_uid"], json!(1000));
        assert_eq!(entries[0]["name"], json!("Android 系统"));
        assert_eq!(entries[0]["total_ms"], json!(76_158));
        // u0a346 has two tags summing to 28 608 + 1 500.
        assert_eq!(entries[1]["numeric_uid"], json!(10_346));
        assert_eq!(entries[1]["name"], json!("com.example.worker"));
        assert_eq!(entries[1]["total_ms"], json!(30_108));
        assert_eq!(entries[1]["count"], json!(5));
        assert_eq!(entries[1]["tags"].as_array().unwrap().len(), 2);
        assert_eq!(
            entries[1]["tags"][0]["tag"],
            json!("*job*e//SessionWorker/@eM_ADDR/x")
        );
        // Shares are relative to the measured total (76 158 + 30 108 + 9 937).
        let total = 76_158.0 + 30_108.0 + 9_937.0;
        assert!((entries[0]["share"].as_f64().unwrap() - 76_158.0 / total).abs() < 1e-9);
    }

    #[test]
    fn limit_trims_the_tail_but_keeps_the_total() {
        let value = to_json(&rows(), &packages(), 1);
        assert_eq!(value["entries"].as_array().unwrap().len(), 1);
        assert_eq!(value["total_ms"], json!(76_158 + 30_108 + 9_937));
    }
}
