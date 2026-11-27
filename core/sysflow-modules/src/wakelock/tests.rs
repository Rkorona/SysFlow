use super::window::WINDOW_SECS;
use super::*;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use sysflow_core::event::EventStore;

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sysflow-wakelock-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("sys/kernel/debug")).unwrap();
    fs::create_dir_all(dir.join("state")).unwrap();
    dir
}

/// debugfs layout: name + 9 counters, times in ms.
fn table(rows: &[(&str, i64, i64, i64)]) -> String {
    let mut text = String::from(
        "name\t\tactive_count\tevent_count\twakeup_count\texpire_count\tactive_since\ttotal_time\tmax_time\tlast_change\tprevent_suspend_time\n",
    );
    for (name, active_since, total, wakeups) in rows {
        text.push_str(&format!(
            "{name}\t1\t2\t{wakeups}\t0\t{active_since}\t{total}\t{total}\t{total}\t0\n"
        ));
    }
    text
}

fn write_table(root: &Path, rows: &[(&str, i64, i64, i64)]) {
    fs::write(root.join("sys/kernel/debug/wakeup_sources"), table(rows)).unwrap();
}

fn module(root: &Path) -> WakelockModule {
    WakelockModule::new(Sysfs::new(root), root.join("state"))
}

#[test]
fn blocking_stays_available_without_a_kernel_table() {
    let root = temp_root("missing");
    fs::remove_file(root.join("sys/kernel/debug/wakeup_sources")).ok();
    let caps = module(&root).probe();
    assert!(caps.supported, "AppOps blocking does not need debugfs");
    assert!(caps.features.iter().any(|f| f == "block"));
    assert!(!caps.features.iter().any(|f| f == "kernel_sources"));

    let value = module(&root).collect(None).unwrap();
    assert_eq!(value[ITEM_SUMMARY]["available"], json!(false));
    assert_eq!(value[ITEM_SOURCES], json!([]));
    assert!(module(&root).sample().is_none(), "nothing to sample");
}

#[test]
fn unreadable_table_reports_a_reason() {
    let root = temp_root("reason");
    fs::remove_file(root.join("sys/kernel/debug/wakeup_sources")).ok();
    let value = module(&root).collect(None).unwrap();
    assert_eq!(value[ITEM_SUMMARY]["available"], json!(false));
    let reason = value[ITEM_SUMMARY]["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("wakeup_sources") || reason.contains("wakelocks"),
        "reason should name the missing node, got {reason:?}"
    );
}

#[test]
fn a_table_with_invalid_utf8_still_parses() {
    let root = temp_root("lossy");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"name\tactive_count\tevent_count\twakeup_count\texpire_count\tactive_since\ttotal_time\tmax_time\tlast_change\tprevent_suspend_time\n");
    // One source whose name contains a byte that is not valid UTF-8.
    bytes.extend_from_slice(b"wlan\xff_wow\t0\t0\t1\t0\t0\t12000\t12000\t12000\t0\n");
    fs::write(root.join("sys/kernel/debug/wakeup_sources"), bytes).unwrap();

    let value = module(&root).collect(None).unwrap();
    assert_eq!(value[ITEM_SUMMARY]["available"], json!(true));
    let sources = value[ITEM_SOURCES].as_array().unwrap();
    assert_eq!(
        sources.len(),
        1,
        "a malformed byte must not drop the whole table"
    );
    assert!(sources[0]["name"].as_str().unwrap().starts_with("wlan"));
}

#[test]
fn collects_sources_sorted_by_activity_then_time() {
    let root = temp_root("collect");
    write_table(
        &root,
        &[
            ("idle_source", 0, 10_000, 0),
            ("busy_source", 0, 500, 3),
            ("active_source", 1_234, 100, 0),
        ],
    );
    let m = module(&root);
    let value = m.collect(None).unwrap();
    let list = value[ITEM_SOURCES].as_array().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(
        list[0]["name"],
        json!("active_source"),
        "active comes first"
    );
    assert_eq!(list[0]["active"], json!(true));
    assert_eq!(list[1]["name"], json!("idle_source"), "then by total time");
    assert_eq!(list[1]["total_ms"], json!(10_000));
    assert_eq!(list[2]["name"], json!("busy_source"));
    assert_eq!(list[2]["wakeup_count"], json!(3));
}

#[test]
fn keys_filter_limits_the_payload() {
    let root = temp_root("keys");
    write_table(&root, &[("src", 0, 42, 0)]);
    let m = module(&root);
    let keys = vec![ITEM_SUMMARY.to_string()];
    let value = m.collect(Some(&keys)).unwrap();
    assert!(value.get(ITEM_SOURCES).is_none());
    assert!(value.get(ITEM_SUMMARY).is_some());
    assert_eq!(value[ITEM_SUMMARY]["known"], json!(1));
}

#[test]
fn second_sample_produces_a_window_ranking() {
    let root = temp_root("ranking");
    write_table(&root, &[("alpha", 0, 1_000, 0), ("beta", 0, 500, 0)]);
    let m = module(&root);

    m.sample().expect("first sample").unwrap();
    // The first sample only establishes the baseline, so nothing can be ranked yet.
    assert_eq!(m.collect(None).unwrap()[ITEM_RANKED], json!([]));

    write_table(&root, &[("alpha", 0, 4_000, 0), ("beta", 0, 500, 0)]);
    m.sample().expect("second sample").unwrap();

    let value = m.collect(None).unwrap();
    let ranked = value[ITEM_RANKED].as_array().unwrap();
    assert_eq!(ranked.len(), 1, "beta did not move, so it has no delta");
    assert_eq!(ranked[0]["name"], json!("alpha"));
    assert_eq!(ranked[0]["awake_ms"], json!(3_000));
    assert_eq!(ranked[0]["share"], json!(1.0));
    assert_eq!(value[ITEM_SUMMARY]["ranking_available"], json!(true));
    assert_eq!(value[ITEM_SUMMARY]["active"], json!(0));
}

#[test]
fn state_is_persisted_after_sampling() {
    let root = temp_root("persist");
    write_table(&root, &[("src", 0, 100, 0)]);
    let m = module(&root);
    let path = super::state::state_path(&root.join("state"));
    assert!(!path.exists());
    m.sample().unwrap().unwrap();
    assert!(path.exists(), "sample must persist its baseline");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"baseline_ts\""));
}

#[test]
fn an_expired_window_rolls_forward_instead_of_clamping_to_zero() {
    let root = temp_root("rollover");
    write_table(&root, &[("src", 0, 2_000, 0)]);
    let m = module(&root);
    let now = sysflow_core::history::now_secs();
    // A window that started more than a day ago, with a known previous observation.
    let state = json!({
        "version": 1,
        "baseline_ts": now - WINDOW_SECS - 60,
        "baseline": { "src": 1_000 },
        "last": { "src": 1_500 },
        "updated_ts": now - WINDOW_SECS - 60,
    });
    fs::write(m.state.path(), serde_json::to_string(&state).unwrap()).unwrap();

    m.sample().unwrap().unwrap();
    let value = m.collect(None).unwrap();
    let ranked = value[ITEM_RANKED].as_array().unwrap();
    assert_eq!(ranked.len(), 1);
    // New baseline is the previous observation (1500), so the delta is 500 rather than 2000.
    assert_eq!(ranked[0]["awake_ms"], json!(500));
}

#[test]
fn a_whole_table_rollback_is_treated_as_a_reboot() {
    let root = temp_root("reboot");
    write_table(&root, &[("src", 0, 900, 0)]);
    let m = module(&root);
    let now = sysflow_core::history::now_secs();
    let state = json!({
        "version": 1,
        "baseline_ts": now - 600,
        "baseline": { "src": 5_000 },
        "last": { "src": 5_000 },
        "updated_ts": now - 600,
    });
    fs::write(m.state.path(), serde_json::to_string(&state).unwrap()).unwrap();

    m.sample().unwrap().unwrap();
    let value = m.collect(None).unwrap();
    // The window restarts at the reboot, so this sample has nothing to rank yet...
    assert_eq!(value[ITEM_RANKED], json!([]));
    // ...and the client is told why, instead of being told the device was idle.
    assert_eq!(value[ITEM_SUMMARY]["window_reason"], json!("reboot"));
    assert_eq!(value[ITEM_SUMMARY]["window_s"], json!(0));

    // The next sample ranks normally, so a reboot no longer kills the ranking for 24 h.
    write_table(&root, &[("src", 0, 1_200, 0)]);
    m.sample().unwrap().unwrap();
    let ranked = m.collect(None).unwrap()[ITEM_RANKED].clone();
    assert_eq!(ranked[0]["awake_ms"], json!(300), "counted from the reboot");
}

#[test]
fn a_single_source_regression_clamps_without_losing_the_window() {
    let root = temp_root("one-backwards");
    // `b` re-registered (its counter restarted) while `a` kept growing: not a reboot.
    write_table(&root, &[("a", 0, 6_000, 0), ("b", 0, 10, 0)]);
    let m = module(&root);
    let now = sysflow_core::history::now_secs();
    let state = json!({
        "version": 1,
        "baseline_ts": now - 600,
        "baseline": { "a": 5_000, "b": 3_000 },
        "last": { "a": 5_000, "b": 3_000 },
        "updated_ts": now - 600,
    });
    fs::write(m.state.path(), serde_json::to_string(&state).unwrap()).unwrap();

    m.sample().unwrap().unwrap();
    let value = m.collect(None).unwrap();
    assert_ne!(value[ITEM_SUMMARY]["window_reason"], json!("reboot"));
    let ranked = value[ITEM_RANKED].as_array().unwrap();
    assert_eq!(ranked.len(), 1, "b clamps to zero, a still ranks");
    assert_eq!(ranked[0]["name"], json!("a"));
    assert_eq!(ranked[0]["awake_ms"], json!(1_000));
}

#[test]
fn share_is_relative_to_every_measured_source() {
    let root = temp_root("share");
    const NAMES: [&str; 16] = [
        "s00", "s01", "s02", "s03", "s04", "s05", "s06", "s07", "s08", "s09", "s10", "s11", "s12",
        "s13", "s14", "s15",
    ];
    let rows = |total: i64| -> Vec<(&'static str, i64, i64, i64)> {
        NAMES.iter().map(|name| (*name, 0, total, 0)).collect()
    };

    write_table(&root, &rows(1_000));
    let m = module(&root);
    m.sample().unwrap().unwrap();
    write_table(&root, &rows(2_000));
    m.sample().unwrap().unwrap();

    let value = m.collect(None).unwrap();
    let ranked = value[ITEM_RANKED].as_array().unwrap();
    assert_eq!(
        ranked.len(),
        super::read::RANK_LIMIT,
        "only the top rows are listed"
    );
    assert_eq!(
        value[ITEM_SUMMARY]["measured_ms"],
        json!(16_000),
        "the denominator covers all 16 sources"
    );
    let sum: f64 = ranked
        .iter()
        .map(|row| row["share"].as_f64().unwrap())
        .sum();
    assert!(
        sum < 0.999,
        "shares must not be renormalized over the truncated list, got {sum}"
    );
}

#[test]
fn the_ranking_reports_wakeup_counts_alongside_time() {
    let root = temp_root("counts");
    write_table(&root, &[("alpha", 0, 1_000, 10), ("beta", 0, 1_000, 3)]);
    let m = module(&root);
    m.sample().unwrap().unwrap();
    write_table(&root, &[("alpha", 0, 1_500, 15), ("beta", 0, 4_000, 40)]);
    m.sample().unwrap().unwrap();

    let value = m.collect(None).unwrap();
    let ranked = value[ITEM_RANKED].as_array().unwrap();
    // Sorted by time: beta grew by 3 000 ms, alpha by 500 ms.
    assert_eq!(ranked[0]["name"], json!("beta"));
    assert_eq!(ranked[0]["wakeups"], json!(37));
    assert_eq!(ranked[1]["wakeups"], json!(5));
    assert_eq!(value[ITEM_SUMMARY]["total_wakeups"], json!(42));
}

#[test]
fn sampling_records_wakeup_counts_for_the_trend() {
    let root = temp_root("sample-wakeups");
    write_table(&root, &[("alpha", 0, 1_000, 10), ("beta", 0, 1_000, 3)]);
    let m = module(&root);
    m.sample().unwrap().unwrap();
    write_table(&root, &[("alpha", 0, 1_500, 15), ("beta", 0, 4_000, 40)]);
    let sample = m.sample().unwrap().unwrap();
    assert_eq!(sample["top_ms"], json!(3_000));
    assert_eq!(
        sample["wakeups"],
        json!(42),
        "counts land in the history series"
    );
}

#[test]
fn apply_without_the_rule_item_is_rejected() {
    let root = temp_root("readonly");
    write_table(&root, &[("src", 0, 1, 0)]);
    let m = module(&root);
    let err = m
        .apply(&Map::new())
        .expect_err("only wakelock.blocked is writable");
    assert_eq!(err.code.as_str(), "invalid_arg");
    assert!(err.msg.contains("wakelock.blocked"), "msg: {}", err.msg);
}

#[test]
fn legacy_proc_wakelocks_table_is_supported() {
    let root = temp_root("legacy");
    fs::remove_file(root.join("sys/kernel/debug/wakeup_sources")).ok();
    fs::create_dir_all(root.join("proc")).unwrap();
    fs::write(
        root.join("proc/wakelocks"),
        "name\t\tactive_count\texpire_count\twakeup_count\tactive_since\ttotal_time\tmax_time\tlast_change\tprevent_suspend_time\n\
sensor_wl\t1\t0\t2\t1000000000\t2000000000\t2000000000\t1000000000\t0\n",
    )
    .unwrap();
    let m = module(&root);
    assert!(m.probe().supported);
    let value = m.collect(None).unwrap();
    assert_eq!(value[ITEM_SOURCES][0]["name"], json!("sensor_wl"));
    assert_eq!(
        value[ITEM_SOURCES][0]["total_ms"],
        json!(2_000),
        "nanoseconds converted to milliseconds"
    );
}

// ---- AppOps blocking -------------------------------------------------------

/// Fake `cmd` with an in-memory AppOps store. `fail_on` simulates a package whose write fails.
fn fake_cmd(
    state: Arc<Mutex<HashSet<String>>>,
    installed: &'static [&'static str],
    fail_on: Option<&'static str>,
) -> Arc<Runner> {
    Arc::new(move |args: &[&str]| match args {
        ["appops", "query-op", "WAKE_LOCK", "ignore"] => {
            let set = state.lock().unwrap();
            let mut names: Vec<String> = set.iter().cloned().collect();
            names.sort();
            Ok(names.join("\n"))
        }
        ["appops", "set", package, "WAKE_LOCK", mode] => {
            if fail_on == Some(*package) {
                return Err(Error::io(format!("cannot set {package}: EPERM")));
            }
            let mut set = state.lock().unwrap();
            if *mode == "ignore" {
                set.insert((*package).to_string());
            } else {
                set.remove(*package);
            }
            Ok(String::new())
        }
        ["package", "list", "packages", filter] => {
            let names: Vec<String> = installed
                .iter()
                .filter(|p| p.contains(filter))
                .map(|p| format!("package:{p}"))
                .collect();
            Ok(names.join("\n"))
        }
        other => Err(Error::io(format!("unexpected command: {other:?}"))),
    })
}

fn blocking_module(
    root: &Path,
    state: Arc<Mutex<HashSet<String>>>,
    installed: &'static [&'static str],
    fail_on: Option<&'static str>,
) -> WakelockModule {
    WakelockModule::with_runner(
        Sysfs::new(root),
        root.join("state"),
        fake_cmd(state, installed, fail_on),
    )
}

fn rule(package: &str, blocked: bool) -> Map<String, Value> {
    let mut req = Map::new();
    let mut rules = Map::new();
    rules.insert(package.to_string(), json!(blocked));
    req.insert(ITEM_BLOCKED.to_string(), Value::Object(rules));
    req
}

#[test]
fn apply_blocks_an_app_and_reports_the_readback() {
    let root = temp_root("block");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    let m = blocking_module(&root, Arc::clone(&state), &["com.example.app"], None);

    let result = m
        .apply(&rule("com.example.app", true))
        .expect("apply should succeed");
    assert_eq!(result.applied[ITEM_BLOCKED]["com.example.app"], json!(true));
    // The registry persists `requested` verbatim, so both maps must be keyed by item id; keying
    // them by package flattens `config.modules.wakelock` and breaks boot replay.
    assert!(result.requested.contains_key(ITEM_BLOCKED));
    assert!(result.applied.contains_key(ITEM_BLOCKED));
    assert!(result.applied.get("com.example.app").is_none());
    assert!(result.mismatch.is_empty());
    assert!(state.lock().unwrap().contains("com.example.app"));
}

#[test]
fn apply_rejects_a_protected_app_without_changing_anything() {
    let root = temp_root("protected");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    let m = blocking_module(&root, Arc::clone(&state), &["com.android.systemui"], None);

    let err = m
        .apply(&rule("com.android.systemui", true))
        .expect_err("protected apps must be refused");
    assert_eq!(err.code.as_str(), "invalid_arg");
    assert!(err.msg.contains("protected"));
    assert!(state.lock().unwrap().is_empty());
}

#[test]
fn a_failing_write_rolls_back_the_earlier_ones() {
    let root = temp_root("rollback");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    let m = blocking_module(
        &root,
        Arc::clone(&state),
        &["com.example.first", "com.example.second"],
        Some("com.example.second"),
    );

    let mut req = Map::new();
    let mut rules = Map::new();
    rules.insert("com.example.first".to_string(), json!(true));
    rules.insert("com.example.second".to_string(), json!(true));
    req.insert(ITEM_BLOCKED.to_string(), Value::Object(rules));

    let err = m.apply(&req).expect_err("second write fails");
    assert_eq!(err.code.as_str(), "io");
    assert!(
        !state.lock().unwrap().contains("com.example.first"),
        "the first block must be undone"
    );
}

#[test]
fn a_mixed_request_may_release_a_protected_app() {
    let root = temp_root("mixed");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    state
        .lock()
        .unwrap()
        .insert("com.android.phone".to_string());
    let m = blocking_module(&root, Arc::clone(&state), &["com.example.app"], None);

    let mut req = Map::new();
    let mut rules = Map::new();
    rules.insert("com.android.phone".to_string(), json!(false));
    rules.insert("com.example.app".to_string(), json!(true));
    req.insert(ITEM_BLOCKED.to_string(), Value::Object(rules));

    m.apply(&req)
        .expect("releasing a protected app alongside a block is allowed");
    let blocked = state.lock().unwrap();
    assert!(!blocked.contains("com.android.phone"));
    assert!(blocked.contains("com.example.app"));
}

#[test]
fn a_failed_rollback_is_reported_instead_of_hidden() {
    let root = temp_root("rollback-fail");
    write_table(&root, &[("src", 0, 1, 0)]);
    let runner: Arc<Runner> = Arc::new(|args: &[&str]| match args {
        ["appops", "query-op", "WAKE_LOCK", "ignore"] => Ok(String::new()),
        ["appops", "set", "com.example.first", "WAKE_LOCK", "ignore"] => Ok(String::new()),
        ["appops", "set", "com.example.first", "WAKE_LOCK", "default"] => {
            Err(Error::io("rollback refused"))
        }
        ["appops", "set", "com.example.second", "WAKE_LOCK", "ignore"] => {
            Err(Error::io("block refused"))
        }
        _ => Ok(String::new()),
    });
    let m = WakelockModule::with_runner(Sysfs::new(&root), root.join("state"), runner);

    let mut req = Map::new();
    let mut rules = Map::new();
    rules.insert("com.example.first".to_string(), json!(true));
    rules.insert("com.example.second".to_string(), json!(true));
    req.insert(ITEM_BLOCKED.to_string(), Value::Object(rules));

    let err = m.apply(&req).expect_err("the second write fails");
    assert_eq!(err.code.as_str(), "io");
    assert!(err.msg.contains("rollback failed"), "msg: {}", err.msg);
    assert!(
        err.msg.contains("com.example.first"),
        "the stuck package must be named: {}",
        err.msg
    );
}

#[test]
fn drift_reports_a_rule_that_no_longer_matches_the_system() {
    let root = temp_root("drift");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    let m = blocking_module(&root, Arc::clone(&state), &["com.example.app"], None);

    let mut settings = Map::new();
    let mut rules = Map::new();
    rules.insert("com.example.app".to_string(), json!(true));
    settings.insert(ITEM_BLOCKED.to_string(), Value::Object(rules));

    assert_eq!(m.drift(&settings).unwrap(), vec![ITEM_BLOCKED.to_string()]);

    m.apply(&rule("com.example.app", true)).unwrap();
    assert!(m.drift(&settings).unwrap().is_empty());
}

#[test]
fn sanitize_boot_drops_protected_and_uninstalled_rules() {
    let root = temp_root("sanitize");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    let m = blocking_module(&root, state, &["com.example.installed"], None);

    let mut req = Map::new();
    let mut rules = Map::new();
    rules.insert("com.example.installed".to_string(), json!(true));
    rules.insert("com.example.gone".to_string(), json!(true));
    rules.insert("com.android.phone".to_string(), json!(true));
    req.insert(ITEM_BLOCKED.to_string(), Value::Object(rules));

    let sanitized = m.sanitize_boot(&req);
    let kept = sanitized[ITEM_BLOCKED].as_object().unwrap();
    assert_eq!(kept.len(), 1);
    assert!(kept.contains_key("com.example.installed"));
}

#[test]
fn reset_releases_only_the_rules_it_owns() {
    let root = temp_root("reset");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    {
        let mut blocked = state.lock().unwrap();
        blocked.insert("com.example.app".to_string()); // ours
        blocked.insert("com.third.party".to_string()); // set outside SysFlow
        blocked.insert("com.android.phone".to_string()); // protected
    }
    let m = blocking_module(&root, Arc::clone(&state), &["com.example.app"], None);

    let mut settings = Map::new();
    let mut rules = Map::new();
    rules.insert("com.example.app".to_string(), json!(true));
    settings.insert(ITEM_BLOCKED.to_string(), Value::Object(rules));

    m.reset_with_settings(&settings)
        .expect("reset should succeed");
    let blocked = state.lock().unwrap();
    assert!(!blocked.contains("com.example.app"), "our rule is released");
    assert!(
        blocked.contains("com.third.party"),
        "a rule we never set must survive a reset"
    );
    assert!(
        blocked.contains("com.android.phone"),
        "protected apps are never touched"
    );
}

#[test]
fn reset_without_settings_touches_nothing() {
    let root = temp_root("reset-bare");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    state.lock().unwrap().insert("com.example.app".to_string());
    let m = blocking_module(&root, Arc::clone(&state), &["com.example.app"], None);

    m.reset().expect("the settings-less reset is a no-op");
    assert!(
        state.lock().unwrap().contains("com.example.app"),
        "without the persisted intent there is no safe set to undo"
    );
}

#[test]
fn blocked_list_is_only_returned_when_explicitly_requested() {
    let root = temp_root("optin");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    state.lock().unwrap().insert("com.example.app".to_string());
    let m = blocking_module(&root, Arc::clone(&state), &["com.example.app"], None);

    // `snapshot` (keys = None) must not pay for a binder call.
    assert!(m.collect(None).unwrap().get(ITEM_BLOCKED).is_none());

    let keys = vec![ITEM_BLOCKED.to_string()];
    let value = m.collect(Some(&keys)).unwrap();
    assert_eq!(value[ITEM_BLOCKED]["com.example.app"], json!(true));
}

/// A module whose `dumpsys power` is the given text and whose events land under `root/events`.
fn watchdog_module(root: &Path, dump: &'static str) -> WakelockModule {
    let power = Power::with_runner(Arc::new(move || Ok(dump.to_string())));
    WakelockModule::new(Sysfs::new(root), root.join("state"))
        .with_power(power)
        .with_events(EventStore::new(root))
}

fn long_held_events(root: &Path) -> Vec<sysflow_core::event::SystemEvent> {
    EventStore::new(root)
        .list(Default::default())
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "wakelock_long_held")
        .collect()
}

#[test]
fn a_long_held_partial_lock_is_reported_once() {
    let root = temp_root("watchdog-long");
    write_table(&root, &[("src", 0, 1, 0)]);
    let dump = "Wake Locks: size=1\n\
        \x20 PARTIAL_WAKE_LOCK   'AudioMix' ACQ=-45m (uid=10123, pid=1, pkg=com.example.app)\n";
    let m = watchdog_module(&root, dump);

    m.sample().unwrap().unwrap();
    m.sample().unwrap().unwrap();

    let events = long_held_events(&root);
    assert_eq!(events.len(), 1, "the second sample must be deduped");
    assert_eq!(events[0].module, "wakelock");
    assert_eq!(events[0].severity, "warning");
    assert_eq!(events[0].metadata["package"], json!("com.example.app"));
    assert_eq!(events[0].metadata["held_ms"], json!(2_700_000));
}

#[test]
fn a_short_held_lock_is_ignored() {
    let root = temp_root("watchdog-short");
    write_table(&root, &[("src", 0, 1, 0)]);
    let dump = "Wake Locks: size=1\n\
        \x20 PARTIAL_WAKE_LOCK   'AudioMix' ACQ=-5m (uid=10123, pid=1, pkg=com.example.app)\n";
    watchdog_module(&root, dump).sample().unwrap().unwrap();
    assert!(long_held_events(&root).is_empty());
}

#[test]
fn a_protected_app_is_not_reported() {
    let root = temp_root("watchdog-protected");
    write_table(&root, &[("src", 0, 1, 0)]);
    let dump = "Wake Locks: size=1\n\
        \x20 PARTIAL_WAKE_LOCK   'PhoneService' ACQ=-45m (uid=1001, pid=1, pkg=com.android.phone)\n";
    watchdog_module(&root, dump).sample().unwrap().unwrap();
    assert!(
        long_held_events(&root).is_empty(),
        "protected apps are system noise"
    );
}

#[test]
fn a_kernel_source_active_too_long_is_reported() {
    let root = temp_root("watchdog-kernel");
    // active_since = 2 000 000 ms (33 min), well over the threshold.
    write_table(&root, &[("wlan_wow_wl", 2_000_000, 5_000, 0)]);
    watchdog_module(&root, "Power Manager State:\n")
        .sample()
        .unwrap()
        .unwrap();

    let events = long_held_events(&root);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].metadata["kind"], json!("kernel_source"));
    assert_eq!(events[0].metadata["name"], json!("wlan_wow_wl"));
}

#[test]
fn publishes_the_protection_policy_as_data() {
    let root = temp_root("protected-item");
    write_table(&root, &[("src", 0, 1, 0)]);
    let value = module(&root).collect(None).unwrap();
    let list: Vec<&str> = value[ITEM_PROTECTED]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.as_str().unwrap())
        .collect();
    assert!(list.contains(&"com.android.phone"));
    assert!(list.contains(&"io.sysflow"));
    assert!(!list.contains(&"com.example.app"));

    // Read-only: a client must not be able to write the policy.
    let schema = module(&root).schema();
    let item = schema
        .items
        .iter()
        .find(|item| item.id == ITEM_PROTECTED)
        .expect("protected item is part of the schema");
    assert_eq!(item.ty, ItemType::Readonly);
}

#[test]
fn an_exemption_releases_a_block_and_persists_both_items() {
    let root = temp_root("exempt-release");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    state.lock().unwrap().insert("com.example.app".to_string());
    let m = blocking_module(&root, Arc::clone(&state), &["com.example.app"], None);

    let mut req = Map::new();
    let mut exempt = Map::new();
    exempt.insert("com.example.app".to_string(), json!(true));
    req.insert(ITEM_EXEMPT.to_string(), Value::Object(exempt));

    let result = m.apply(&req).expect("exempting should succeed");
    // The implied release is reported (and therefore persisted) as an explicit block=false.
    assert_eq!(
        result.requested[ITEM_BLOCKED]["com.example.app"],
        json!(false),
        "an exemption must not leave a block behind"
    );
    assert_eq!(
        result.requested[ITEM_EXEMPT]["com.example.app"],
        json!(true)
    );
    assert!(result.mismatch.is_empty());
    assert!(!state.lock().unwrap().contains("com.example.app"));
}

#[test]
fn blocking_a_whitelisted_app_is_refused() {
    let root = temp_root("exempt-refuse");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    let m = blocking_module(&root, Arc::clone(&state), &["com.example.app"], None);

    // The exemption is already persisted, so the request itself does not mention it.
    let settings = json!({ "wakelock.exempt": { "com.example.app": true } })
        .as_object()
        .unwrap()
        .clone();
    let err = m
        .apply_with_settings(&rule("com.example.app", true), &settings)
        .expect_err("a whitelisted app must not be blocked");
    assert_eq!(err.code.as_str(), "invalid_arg");
    assert!(err.msg.contains("whitelist"), "msg: {}", err.msg);
    assert!(state.lock().unwrap().is_empty());
}

#[test]
fn an_invalid_exemption_entry_is_refused() {
    let root = temp_root("exempt-invalid");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    let m = blocking_module(&root, Arc::clone(&state), &["com.example.app"], None);

    let mut req = Map::new();
    let mut exempt = Map::new();
    exempt.insert("com.example; rm -rf /".to_string(), json!(true));
    req.insert(ITEM_EXEMPT.to_string(), Value::Object(exempt));
    let err = m
        .apply(&req)
        .expect_err("an unsafe package name is refused");
    assert_eq!(err.code.as_str(), "invalid_arg");
}

#[test]
fn boot_sanitising_drops_blocks_for_whitelisted_apps() {
    let root = temp_root("exempt-boot");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    let m = blocking_module(&root, state, &["com.example.app", "com.other.app"], None);

    let req = json!({
        "wakelock.blocked": { "com.example.app": true, "com.other.app": true },
        "wakelock.exempt": { "com.example.app": true },
    });
    let sanitized = m.sanitize_boot(req.as_object().unwrap());
    let kept = sanitized[ITEM_BLOCKED].as_object().unwrap();
    assert!(
        !kept.contains_key("com.example.app"),
        "exemption wins at boot"
    );
    assert!(kept.contains_key("com.other.app"));
    assert_eq!(sanitized[ITEM_EXEMPT]["com.example.app"], json!(true));
}

#[test]
fn drift_releases_a_block_that_contradicts_a_whitelist_entry() {
    let root = temp_root("exempt-drift");
    write_table(&root, &[("src", 0, 1, 0)]);
    let state = Arc::new(Mutex::new(HashSet::new()));
    state.lock().unwrap().insert("com.example.app".to_string());
    let m = blocking_module(&root, Arc::clone(&state), &["com.example.app"], None);

    let settings = json!({
        "wakelock.blocked": { "com.example.app": true },
        "wakelock.exempt": { "com.example.app": true },
    })
    .as_object()
    .unwrap()
    .clone();
    assert_eq!(m.drift(&settings).unwrap(), vec![ITEM_BLOCKED.to_string()]);
}

#[test]
fn schema_marks_the_whitelist_as_a_low_risk_persisted_switch() {
    let root = temp_root("exempt-schema");
    write_table(&root, &[("src", 0, 1, 0)]);
    let schema = module(&root).schema();
    let item = schema
        .items
        .iter()
        .find(|item| item.id == ITEM_EXEMPT)
        .expect("exempt item is part of the schema");
    assert_eq!(item.ty, ItemType::Bool);
    assert_eq!(item.scope, Scope::App);
    assert_eq!(item.risk, Risk::Low);
    assert!(item.persist);
}

#[test]
fn schema_marks_the_rule_item_as_a_persisted_app_scoped_switch() {
    let root = temp_root("schema");
    write_table(&root, &[("src", 0, 1, 0)]);
    let m = module(&root);
    let schema = m.schema();
    let item = schema
        .items
        .iter()
        .find(|item| item.id == ITEM_BLOCKED)
        .expect("blocked item is part of the schema");
    assert_eq!(item.ty, ItemType::Bool);
    assert_eq!(item.scope, Scope::App);
    assert!(item.persist);
    assert_eq!(item.risk, Risk::Medium);
}
