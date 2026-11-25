//! `sysflow wakelock wakeup` — what woke the device last.
//!
//! `dumpsys power` is the only portable source, and it is a Java service call: the same cost as
//! `wakelock held`, hence a verb with a one-minute cache rather than a snapshot item. Builds differ
//! (AOSP prints `Last wakeup reason: …`, vendor builds print `mLastWakeupReason=…`, several print
//! neither), so an unreported reason is `available: false` with a note — never an invented answer.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use sysflow_core::cache;
use sysflow_core::dumpsys;
use sysflow_core::error::Result;
use sysflow_core::registry::Registry;
use sysflow_core::wakeup_sources::{parse_last_wakeup, parse_uptime, LastWakeup};

/// Same budget as `wakelock held`: never stall the caller on a wedged system server.
const TIMEOUT: Duration = Duration::from_secs(8);

pub fn run(reg: &Registry, refresh: bool) -> Result<Value> {
    let cache_path = reg
        .ctx()
        .data_dir
        .join("state")
        .join("wakelock_wakeup.json");
    if let Some(value) = cached_payload(&cache_path, refresh) {
        return Ok(value);
    }

    let dump = dumpsys::run_with_timeout(&["power"], TIMEOUT)?;
    let value = payload(
        parse_last_wakeup(&dump),
        uptime_ms(),
        cache::now_secs(),
        false,
    );
    cache::write(&cache_path, &value);
    Ok(value)
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

/// Milliseconds since boot, so a build that prints the wakeup uptime can also be aged.
fn uptime_ms() -> Option<i64> {
    let text = std::fs::read_to_string("/proc/uptime").ok()?;
    let seconds = parse_uptime(&text)?;
    Some((seconds as i64).saturating_mul(1_000))
}

/// Build the verb payload.
///
/// `age_ms` is only reported when the build printed the wakeup uptime *and* that value is
/// consistent with the current uptime; a stale or future value is dropped rather than shown as a
/// negative age.
fn payload(
    wakeup: Option<LastWakeup>,
    now_uptime_ms: Option<i64>,
    generated_at: u64,
    cached: bool,
) -> Value {
    match wakeup {
        Some(wakeup) => {
            // A printed `(N ms ago)` is exact; the uptime subtraction is the fallback for builds
            // that print an uptime instead.
            let age_ms = wakeup
                .age_ms
                .or_else(|| match (wakeup.uptime_ms, now_uptime_ms) {
                    (Some(at), Some(now)) if at >= 0 && at <= now => Some(now - at),
                    _ => None,
                });
            json!({
                "available": true,
                "reason": wakeup.reason,
                "age_ms": age_ms,
                "generated_at": generated_at,
                "cached": cached,
            })
        }
        None => json!({
            "available": false,
            "reason": Value::Null,
            "age_ms": null,
            // Told apart from "no wakeup yet": this ROM simply does not print the field.
            "note": "此 ROM 的 dumpsys power 没有输出上次唤醒原因",
            "generated_at": generated_at,
            "cached": cached,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_becomes_an_available_payload() {
        let wakeup = LastWakeup {
            reason: "1234".to_string(),
            uptime_ms: Some(10_000),
            age_ms: None,
        };
        let value = payload(Some(wakeup), Some(70_000), 1_700_000_000, false);
        assert_eq!(value["available"], json!(true));
        assert_eq!(value["reason"], json!("1234"));
        assert_eq!(value["age_ms"], json!(60_000));
        assert_eq!(value["cached"], json!(false));
    }

    #[test]
    fn an_unknown_uptime_omits_the_age() {
        let wakeup = LastWakeup {
            reason: "abort: no reason".to_string(),
            uptime_ms: None,
            age_ms: None,
        };
        let value = payload(Some(wakeup), Some(70_000), 1, false);
        assert_eq!(value["available"], json!(true));
        assert_eq!(value["age_ms"], json!(null));
    }

    #[test]
    fn an_implausible_uptime_does_not_produce_a_negative_age() {
        // The wakeup claims to be newer than the current uptime: a clock/RAM mismatch, not a bug we
        // should render as "-5 s ago".
        let wakeup = LastWakeup {
            reason: "x".to_string(),
            uptime_ms: Some(90_000),
            age_ms: None,
        };
        let value = payload(Some(wakeup), Some(70_000), 1, false);
        assert_eq!(value["age_ms"], json!(null));
    }

    #[test]
    fn a_fresh_cache_answers_and_refresh_bypasses_it() {
        let dir = std::env::temp_dir().join(format!("sysflow-wakeup-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wakelock_wakeup.json");
        cache::write(&path, &payload(None, None, cache::now_secs(), false));

        let cached = cached_payload(&path, false).expect("a fresh entry is served");
        assert_eq!(cached["cached"], json!(true));
        assert!(cached_payload(&path, true).is_none());
    }

    #[test]
    fn a_printed_age_wins_over_the_uptime_math() {
        let wakeup = LastWakeup {
            reason: "19".to_string(),
            uptime_ms: Some(87_574_287),
            age_ms: Some(802_127),
        };
        let value = payload(Some(wakeup), Some(88_376_414), 1, false);
        assert_eq!(value["age_ms"], json!(802_127));
    }

    #[test]
    fn an_unreported_reason_is_explicitly_unavailable() {
        let value = payload(None, None, 1, true);
        assert_eq!(value["available"], json!(false));
        assert_eq!(value["reason"], json!(null));
        assert!(value["note"].as_str().unwrap().contains("dumpsys"));
        assert_eq!(value["cached"], json!(true));
    }
}
