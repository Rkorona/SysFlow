//! `sysflow wakelock held` — the userspace wake locks held right now.
//!
//! `dumpsys power` is a Java service call: hundreds of milliseconds and, on a wedged system server,
//! potentially unbounded. It therefore stays out of `snapshot` and is fetched on demand, with a
//! one-minute cache — the same contract as `battery apps` and `battery health`.
//!
//! Kernel wakeup sources are *not* here: they live in the `wakelock` module's snapshot items
//! (`wakelock.kernel_sources`, `wakelock.ranked`), because reading that table is a cheap `seq_file`
//! read with no shelling out.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use sysflow_core::batterystats::{aid_label, read_package_uids};
use sysflow_core::cache;
use sysflow_core::dumpsys;
use sysflow_core::error::Result;
use sysflow_core::registry::Registry;
use sysflow_core::wakeup_sources::{parse_power_wakelocks, HeldWakelock};

/// `dumpsys power` prints a lot; give up quickly rather than stall the caller.
const TIMEOUT: Duration = Duration::from_secs(8);

pub fn run(reg: &Registry, refresh: bool) -> Result<Value> {
    let cache_path = reg.ctx().data_dir.join("state").join("wakelock_held.json");
    if let Some(value) = cached_payload(&cache_path, refresh) {
        return Ok(value);
    }

    let dump = dumpsys::run_with_timeout(&["power"], TIMEOUT)?;
    let locks = sorted_locks(&dump);
    let owners = resolve_owners(&locks);
    let value = held_payload(&locks, &owners, cache::now_secs(), false);
    cache::write(&cache_path, &value);
    Ok(value)
}

/// A display label for each lock's owner.
///
/// Samsung-style dumps carry only `uid=…` (no `pkg=`), so without this the held list shows no owner
/// at all — and "block this app" has no package to act on. A UID that maps to exactly one package
/// gives that package; a shared UID is deliberately *not* guessed (blocking one of several apps
/// because it shares a UID would be a wrong action), so it falls back to the AID name or `UID <n>`.
fn resolve_owners(locks: &[HeldWakelock]) -> Vec<Option<String>> {
    let needs_lookup = locks
        .iter()
        .any(|lock| lock.package.is_none() && lock.uid.is_some());
    let packages = if needs_lookup {
        read_package_uids()
    } else {
        std::collections::BTreeMap::new()
    };

    locks
        .iter()
        .map(|lock| {
            if let Some(package) = &lock.package {
                return Some(package.clone());
            }
            let uid = u32::try_from(lock.uid?).ok()?;
            match packages.get(&uid) {
                Some(names) if names.len() == 1 => Some(names[0].clone()),
                _ => Some(
                    aid_label(uid)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("UID {uid}")),
                ),
            }
        })
        .collect()
}

/// Serve a cached payload when it is fresh, marked as such. `--refresh` bypasses the cache.
fn cached_payload(cache_path: &Path, refresh: bool) -> Option<Value> {
    if refresh {
        return None;
    }
    let mut value = cache::read_fresh(cache_path, cache::TTL_SECS)?;
    value["cached"] = json!(true);
    Some(value)
}

/// Parse and order: longest-held first, which is what a user opens this screen to find.
fn sorted_locks(dump: &str) -> Vec<HeldWakelock> {
    let mut locks = parse_power_wakelocks(dump);
    locks.sort_by(|a, b| b.held_ms.cmp(&a.held_ms).then_with(|| a.tag.cmp(&b.tag)));
    locks
}

/// The verb payload (protocol §3.19).
///
/// `available` stays `true` for an empty list: the dump succeeded, there is simply nothing held.
/// A *failed* dump is an `io` error, so a SELinux denial is never reported as "no wake locks".
fn held_payload(
    locks: &[HeldWakelock],
    owners: &[Option<String>],
    generated_at: u64,
    cached: bool,
) -> Value {
    let entries: Vec<Value> = locks
        .iter()
        .enumerate()
        .map(|(index, lock)| {
            json!({
                "kind": lock.kind,
                "tag": lock.tag,
                "held_ms": lock.held_ms,
                "uid": lock.uid,
                "pid": lock.pid,
                "package": lock.package,
                // Display label resolved from the UID; `package` stays the actionable field.
                "owner": owners.get(index).cloned().flatten(),
            })
        })
        .collect();

    json!({
        "available": true,
        "count": entries.len(),
        "held": entries,
        "generated_at": generated_at,
        "cached": cached,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "Wake Locks: size=2\n\
        \x20 PARTIAL_WAKE_LOCK   'Short' ACQ=-512ms (uid=1000, pid=1, pkg=android)\n\
        \x20 PARTIAL_WAKE_LOCK   'Long' ACQ=-1m2s (uid=1041, pid=2, pkg=com.android.systemui)\n";

    #[test]
    fn locks_are_ordered_by_held_time() {
        let locks = sorted_locks(DUMP);
        assert_eq!(locks.len(), 2);
        assert_eq!(locks[0].tag, "Long", "the lock held longest comes first");
        assert_eq!(locks[1].tag, "Short");
    }

    #[test]
    fn equal_durations_are_ordered_by_tag() {
        let dump = "Wake Locks: size=2\n\
            \x20 PARTIAL_WAKE_LOCK   'Bravo' ACQ=-1s (uid=1000, pid=1, pkg=android)\n\
            \x20 PARTIAL_WAKE_LOCK   'Alpha' ACQ=-1s (uid=1000, pid=2, pkg=android)\n";
        let locks = sorted_locks(dump);
        assert_eq!(locks[0].tag, "Alpha");
        assert_eq!(locks[1].tag, "Bravo");
    }

    #[test]
    fn the_payload_carries_attribution_and_the_cache_flag() {
        let locks = sorted_locks(DUMP);
        let owners = resolve_owners(&locks);
        let value = held_payload(&locks, &owners, 1_700_000_000, true);
        assert_eq!(value["available"], json!(true));
        assert_eq!(value["count"], json!(2));
        assert_eq!(value["generated_at"], json!(1_700_000_000u64));
        assert_eq!(value["cached"], json!(true));
        assert_eq!(value["held"][0]["tag"], json!("Long"));
        assert_eq!(value["held"][0]["held_ms"], json!(62_000));
        assert_eq!(value["held"][0]["package"], json!("com.android.systemui"));
        assert_eq!(value["held"][0]["uid"], json!(1041));
    }

    #[test]
    fn a_fresh_cache_answers_and_refresh_bypasses_it() {
        let dir = std::env::temp_dir().join(format!("sysflow-held-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wakelock_held.json");
        cache::write(&path, &held_payload(&[], &[], cache::now_secs(), false));

        let cached = cached_payload(&path, false).expect("a fresh entry is served");
        assert_eq!(cached["cached"], json!(true));
        assert!(
            cached_payload(&path, true).is_none(),
            "--refresh must bypass the cache"
        );
        assert!(
            cached_payload(&dir.join("missing.json"), false).is_none(),
            "no file means no cache hit"
        );
    }

    #[test]
    fn an_explicit_package_is_used_verbatim() {
        let locks = sorted_locks(DUMP);
        let owners = resolve_owners(&locks);
        assert_eq!(owners[0].as_deref(), Some("com.android.systemui"));
    }

    #[test]
    fn a_lock_without_a_package_gets_a_display_owner() {
        // The Samsung layout carries only `uid=`; the AID table still gives a readable owner.
        let lock = HeldWakelock {
            kind: "PARTIAL_WAKE_LOCK".to_string(),
            tag: "*vibrator*".to_string(),
            held_ms: 39,
            uid: Some(1000),
            pid: Some(14_922),
            package: None,
        };
        let owners = resolve_owners(std::slice::from_ref(&lock));
        assert_eq!(owners[0].as_deref(), Some("Android 系统"));
    }

    #[test]
    fn an_empty_dump_is_still_available() {
        let value = held_payload(&[], &[], 1, false);
        assert_eq!(value["available"], json!(true));
        assert_eq!(value["count"], json!(0));
        assert_eq!(value["held"], json!([]));
    }
}
