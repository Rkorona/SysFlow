//! End-to-end tests of the registry with the mock module (no device needed).

use std::path::PathBuf;

use serde_json::{json, Value};

use sysflow_core::ctx::Ctx;
use sysflow_core::registry::Registry;
use sysflow_core::sysfs::Sysfs;
use sysflow_core::{ErrorCode, Module};
use sysflow_modules::mock::MockModule;

fn setup(name: &str) -> (Registry, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "sysflow-test-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let ctx = Ctx {
        data_dir: dir.clone(),
        module_dir: dir.join("moddir"),
        sysfs: Sysfs::new(dir.join("sys")),
    };
    let modules: Vec<Box<dyn Module>> =
        vec![Box::new(MockModule::new(dir.join("mock_state.json")))];
    (Registry::new(ctx, modules), dir)
}

fn mock_value(reg: &Registry, key: &str) -> Value {
    reg.get("mock", Some(&[key.to_string()])).unwrap()[key].clone()
}

#[test]
fn caps_and_snapshot_list_the_module() {
    let (reg, _dir) = setup("caps");
    let caps = reg.caps().unwrap();
    assert_eq!(caps["protocol"], 1);
    assert_eq!(caps["modules"]["mock"]["supported"], true);
    let snap = reg.snapshot().unwrap();
    assert_eq!(snap["modules"]["mock"]["mock.level"], 50);
    assert_eq!(snap["safe_mode"], false);
}

#[test]
fn set_returns_readback_and_reports_mismatch() {
    let (reg, _dir) = setup("mismatch");
    let ok = reg.set("mock", json!({"mock.level": 80}), false).unwrap();
    assert_eq!(ok["applied"]["mock.level"], 80);
    assert_eq!(ok["mismatch"], json!([]));

    let clamped = reg.set("mock", json!({"mock.level": 100}), false).unwrap();
    assert_eq!(clamped["requested"]["mock.level"], 100);
    assert_eq!(clamped["applied"]["mock.level"], 95);
    assert_eq!(clamped["mismatch"], json!(["mock.level"]));
}

#[test]
fn set_rejects_bad_input() {
    let (reg, _dir) = setup("invalid");
    for bad in [
        json!({"mock.level": 51}),
        json!({"mock.temp": 1}),
        json!({"nope": 1}),
        json!({}),
        json!([1, 2]),
    ] {
        let e = reg.set("mock", bad.clone(), false).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArg, "should reject {bad}");
    }
    // module-enforced lock
    let e = reg
        .set("mock", json!({"mock.cores": [0, 1, 1, 1]}), false)
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArg);
    // unknown module
    let e = reg.set("nope", json!({"x": 1}), false).unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidArg);
}

#[test]
fn reset_restores_the_values_seen_before_the_first_change() {
    let (reg, _dir) = setup("reset");
    reg.set("mock", json!({"mock.level": 80, "mock.mode": "c"}), true)
        .unwrap();
    assert_eq!(mock_value(&reg, "mock.level"), 80);
    reg.reset("mock").unwrap();
    assert_eq!(mock_value(&reg, "mock.level"), 50);
    assert_eq!(mock_value(&reg, "mock.mode"), "a");
    let cfg = reg.store().load().unwrap();
    assert!(cfg.modules.get("mock").is_none());
    assert!(cfg.defaults_snapshot.get("mock").is_none());
}

#[test]
fn persisted_values_are_reapplied_by_apply_boot() {
    let (reg, dir) = setup("boot");
    reg.set("mock", json!({"mock.flag": true, "mock.level": 100}), true)
        .unwrap();
    // pretend the device rebooted: the module's live state is back to defaults
    std::fs::remove_file(dir.join("mock_state.json")).unwrap();
    assert_eq!(mock_value(&reg, "mock.flag"), false);

    let out = reg.apply_boot(true).unwrap();
    assert!(out.get("skipped").is_none(), "unexpected skip: {out}");
    assert_eq!(mock_value(&reg, "mock.flag"), true);
    // the mismatched item (level 100 -> 95) was not persisted
    assert_eq!(mock_value(&reg, "mock.level"), 50);
    assert_eq!(reg.safe_status().unwrap()["boot_count"], 1);

    reg.boot_confirm().unwrap();
    assert_eq!(reg.safe_status().unwrap()["boot_count"], 0);
}

#[test]
fn three_unconfirmed_boots_enter_safe_mode() {
    let (reg, _dir) = setup("loop");
    for _ in 0..3 {
        let out = reg.apply_boot(true).unwrap();
        assert!(out.get("skipped").is_none());
    }
    let out = reg.apply_boot(true).unwrap();
    assert_eq!(out["skipped"], "safe_mode");
    assert_eq!(reg.safe_status().unwrap()["safe_mode"], true);
    assert_eq!(reg.caps().unwrap()["safe_mode"], true);

    // writes are blocked, reset is not
    let e = reg
        .set("mock", json!({"mock.level": 60}), false)
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::SafeMode);
    assert!(reg.reset("mock").is_ok());

    reg.safe_clear().unwrap();
    assert!(reg.set("mock", json!({"mock.level": 60}), false).is_ok());
}

#[test]
fn rescue_marker_skips_boot_tuning_without_counting() {
    let (reg, dir) = setup("rescue");
    std::fs::write(dir.join("disable_tuning"), "").unwrap();
    let out = reg.apply_boot(true).unwrap();
    assert_eq!(out["skipped"], "disable_tuning");
    assert_eq!(reg.safe_status().unwrap()["boot_count"], 0);
    assert_eq!(reg.safe_status().unwrap()["rescue_marker"], true);
}

#[test]
fn history_stub_validates_arguments() {
    let (reg, _dir) = setup("history");
    let h = reg.history("mock", "24h", Some("5m")).unwrap();
    assert_eq!(h["step_s"], 300);
    assert_eq!(h["daemon_running"], false);
    assert!(reg.history("mock", "bogus", None).is_err());
    assert!(reg.history("nope", "1h", None).is_err());
}
