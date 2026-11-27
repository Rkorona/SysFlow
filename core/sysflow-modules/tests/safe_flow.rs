//! Integration tests for the safe-mode / rescue-marker contract at the registry boundary.
//!
//! The pure state machine is unit-tested in `sysflow-core/src/safe.rs`; these tests pin the wire
//! behaviour that `module/service.sh`, the APK and the rescue WebUI actually consume. Every
//! assertion is on the real JSON reply, the persisted `config.json`, the event journal or the
//! module's own state — not on a return code.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use sysflow_core::ctx::Ctx;
use sysflow_core::error::ErrorCode;
use sysflow_core::event::{EventFilter, EventStore, SystemEvent};
use sysflow_core::registry::Registry;
use sysflow_core::sysfs::Sysfs;
use sysflow_core::Module;
use sysflow_modules::cpu::CpuModule;
use sysflow_modules::mock::MockModule;

const CPUFREQ: &str = "sys/devices/system/cpu/cpufreq";

fn temp_root(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "sysflow-safe-flow-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Mock-backed registry. The data dir and the module dir are separate so both rescue-marker
/// locations can be exercised independently (the marker must work from either one).
fn setup_mock(name: &str) -> (Registry, PathBuf) {
    let dir = temp_root(name);
    let data_dir = dir.join("data");
    let module_dir = dir.join("moddir");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&module_dir).unwrap();
    let ctx = Ctx {
        data_dir: data_dir.clone(),
        module_dir,
        sysfs: Sysfs::new(&dir),
    };
    let modules: Vec<Box<dyn Module>> =
        vec![Box::new(MockModule::new(data_dir.join("mock_state.json")))];
    (Registry::new(ctx, modules), dir)
}

/// CPU-backed registry on a fake 4-core sysfs tree, with one WALT guard node present so the
/// optional `cpu.walt_guard` item exists. The layout mirrors `cpu_flow.rs`.
fn setup_cpu(name: &str) -> (Registry, PathBuf) {
    let dir = temp_root(name);
    for cpu in 0..4 {
        put(&dir, &format!("sys/devices/system/cpu/cpu{cpu}/x"), "");
        if cpu != 0 {
            put(
                &dir,
                &format!("sys/devices/system/cpu/cpu{cpu}/online"),
                "1",
            );
        }
    }
    for (id, cpus, table, hw_max, cur_max) in [
        (0, "0 1", "300000 1000000 1800000", "1800000", "1800000"),
        (
            2,
            "2 3",
            "500000 1500000 1800000 2400000",
            "2400000",
            "1800000",
        ),
    ] {
        let d = format!("{CPUFREQ}/policy{id}");
        put(&dir, &format!("{d}/related_cpus"), cpus);
        put(&dir, &format!("{d}/scaling_available_frequencies"), table);
        put(
            &dir,
            &format!("{d}/scaling_available_governors"),
            "schedutil performance",
        );
        put(
            &dir,
            &format!("{d}/cpuinfo_min_freq"),
            table.split(' ').next().unwrap(),
        );
        put(&dir, &format!("{d}/cpuinfo_max_freq"), hw_max);
        put(
            &dir,
            &format!("{d}/scaling_min_freq"),
            table.split(' ').next().unwrap(),
        );
        put(&dir, &format!("{d}/scaling_max_freq"), cur_max);
        put(&dir, &format!("{d}/scaling_governor"), "schedutil");
    }
    // `walt_guard_available` needs at least one guarded node; the guard itself is a mode change.
    put(
        &dir,
        &format!("{CPUFREQ}/policy0/walt/adaptive_level_1"),
        "0\n",
    );

    let data_dir = dir.join("data");
    let module_dir = dir.join("moddir");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&module_dir).unwrap();
    let ctx = Ctx {
        data_dir: data_dir.clone(),
        module_dir,
        sysfs: Sysfs::new(&dir),
    };
    let modules: Vec<Box<dyn Module>> = vec![Box::new(CpuModule::with_state_dir(
        Sysfs::new(&dir),
        ctx.data_dir.join("state"),
    ))];
    (Registry::new(ctx, modules), dir)
}

fn events_in(data_dir: &Path) -> Vec<SystemEvent> {
    EventStore::new(data_dir)
        .list(EventFilter {
            limit: 200,
            ascending: true,
            ..Default::default()
        })
        .unwrap()
}

fn mock_live(reg: &Registry, key: &str) -> Value {
    reg.get("mock", None).unwrap()[key].clone()
}

/// Remove the mock module's live state file, so the next read looks like a fresh boot: only
/// `apply-boot` can bring the persisted values back.
fn simulate_reboot(reg: &Registry) {
    std::fs::remove_file(reg.ctx().data_dir.join("mock_state.json")).unwrap();
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

// ---- 1. rescue marker in the module dir ------------------------------------

#[test]
fn module_dir_rescue_marker_blocks_apply_boot_enforce_and_daemon() {
    let (reg, _dir) = setup_mock("moddir-marker");
    reg.set("mock", json!({"mock.flag": true}), true).unwrap();
    simulate_reboot(&reg);
    assert_eq!(
        mock_live(&reg, "mock.flag"),
        json!(false),
        "precondition: the live module state looks like a fresh boot"
    );

    // Only the module-dir copy of the marker exists (it survives a data wipe).
    std::fs::write(reg.ctx().module_dir.join("disable_tuning"), "").unwrap();
    assert!(!reg.ctx().data_dir.join("disable_tuning").exists());

    let status = reg.safe_status().unwrap();
    assert_eq!(status["rescue_marker"], json!(true));
    assert_eq!(status["safe_mode"], json!(false));
    assert_eq!(status["boot_count"], json!(0));

    let out = reg.apply_boot(true).unwrap();
    assert_eq!(out, json!({"skipped": "disable_tuning"}));
    assert_eq!(
        reg.safe_status().unwrap()["boot_count"],
        json!(0),
        "the rescue marker must not advance the boot counter"
    );
    assert_eq!(
        mock_live(&reg, "mock.flag"),
        json!(false),
        "apply-boot must not replay persisted settings while the marker is present"
    );

    let out = reg.enforce().unwrap();
    assert_eq!(out, json!({"skipped": "disable_tuning"}));
    assert_eq!(mock_live(&reg, "mock.flag"), json!(false));

    let out = reg.daemon_start().unwrap();
    assert_eq!(
        out,
        json!({"running": false, "enabled": false, "reason": "disable_tuning"})
    );
    assert!(
        !reg.ctx().data_dir.join("daemon.pid").exists(),
        "a blocked daemon start must not leave a pid file behind"
    );
    let status = reg.daemon_status().unwrap();
    assert_eq!(status["enabled"], json!(false));
    assert_eq!(status["running"], json!(false));

    // The configure error names the real cause: the marker is not safe mode.
    let e = reg.daemon_configure(Some(true), None).unwrap_err();
    assert_eq!(e.code, ErrorCode::SafeMode);
    assert!(
        e.msg.contains("rescue marker"),
        "error message should name the rescue marker, got: {}",
        e.msg
    );
}

#[test]
fn rescue_marker_wins_over_safe_mode_for_every_entry_point() {
    let (reg, _dir) = setup_mock("marker-priority");
    std::fs::write(reg.ctx().data_dir.join("disable_tuning"), "").unwrap();
    reg.store()
        .update(|c| {
            c.safe.safe_mode = true;
            c.safe.boot_count = 2;
            Ok(())
        })
        .unwrap();

    let status = reg.safe_status().unwrap();
    assert_eq!(status["safe_mode"], json!(true));
    assert_eq!(status["rescue_marker"], json!(true));

    assert_eq!(
        reg.apply_boot(true).unwrap(),
        json!({"skipped": "disable_tuning"})
    );
    assert_eq!(reg.enforce().unwrap(), json!({"skipped": "disable_tuning"}));
    assert_eq!(
        reg.daemon_start().unwrap()["reason"],
        json!("disable_tuning"),
        "the marker is the reported reason, not safe mode"
    );

    // Reporting the marker must not mutate the persisted safe-mode state.
    let status = reg.safe_status().unwrap();
    assert_eq!(status["safe_mode"], json!(true));
    assert_eq!(status["boot_count"], json!(2));
}

// ---- 2/3. safe mode: isolation, blocking, clear ---------------------------

#[test]
fn three_unconfirmed_boots_isolate_the_profile_and_emit_the_event() {
    let (reg, dir) = setup_mock("isolation");
    reg.set("mock", json!({"mock.level": 80, "mock.mode": "c"}), true)
        .unwrap();
    simulate_reboot(&reg);
    for boot in 0..3 {
        let out = reg.apply_boot(true).unwrap();
        assert!(
            out.get("skipped").is_none(),
            "unconfirmed boot {boot} should still apply, got {out}"
        );
    }
    let before = reg.store().load().unwrap();
    assert_eq!(before.safe.boot_count, 3);
    assert!(before.recovery_backup.is_none());
    assert!(before.modules.contains_key("mock"));

    let out = reg.apply_boot(true).unwrap();
    assert_eq!(out, json!({"skipped": "safe_mode"}));

    let after = reg.store().load().unwrap();
    assert!(
        after.safe.safe_mode,
        "reaching the threshold must enter safe mode"
    );
    assert_eq!(
        after.safe.boot_count, 3,
        "entering safe mode is not itself a counted boot"
    );
    assert!(
        after.modules.is_empty(),
        "the persisted profile must be isolated, not left active: {:?}",
        after.modules
    );
    let backup = after.recovery_backup.as_ref().expect("recovery backup");
    assert_eq!(backup["mock"]["mock.level"], json!(80));
    assert_eq!(backup["mock"]["mock.mode"], json!("c"));

    let status = reg.safe_status().unwrap();
    assert_eq!(status["safe_mode"], json!(true));
    assert_eq!(status["rescue_marker"], json!(false));
    assert_eq!(status["threshold"], json!(3));

    let events = events_in(&dir.join("data"));
    let entered = events
        .iter()
        .find(|e| e.event_type == "safe_mode_entered")
        .expect("expected a safe_mode_entered event");
    assert_eq!(entered.module, "system");
    assert_eq!(entered.category, "safety");
    assert_eq!(entered.severity, "warning");
    assert_eq!(entered.metadata["isolated_modules"], json!(["mock"]));
    assert_eq!(entered.metadata["boot_count"], json!(3));

    // The recovery surface the APK/WebUI reads agrees with the config.
    let recovery = reg.recovery_status().unwrap();
    assert_eq!(recovery["has_backup"], json!(true));
    assert_eq!(recovery["backup_modules"], json!(["mock"]));
    assert_eq!(recovery["safe_mode"], json!(true));
}

#[test]
fn safe_mode_blocks_enforce_and_daemon_until_cleared() {
    let (reg, _dir) = setup_mock("safe-mode-blocks");
    reg.set("mock", json!({"mock.flag": true}), true).unwrap();
    simulate_reboot(&reg);
    reg.store()
        .update(|c| {
            c.safe.safe_mode = true;
            Ok(())
        })
        .unwrap();

    assert_eq!(reg.enforce().unwrap(), json!({"skipped": "safe_mode"}));
    assert_eq!(
        reg.apply_boot(true).unwrap(),
        json!({"skipped": "safe_mode"})
    );
    assert_eq!(
        mock_live(&reg, "mock.flag"),
        json!(false),
        "safe mode must not write"
    );
    assert_eq!(
        reg.daemon_start().unwrap(),
        json!({"running": false, "enabled": false, "reason": "safe_mode"})
    );
    assert_eq!(reg.daemon_status().unwrap()["enabled"], json!(false));

    let cleared = reg.safe_clear().unwrap();
    assert_eq!(cleared["safe_mode"], json!(false));
    assert_eq!(cleared["boot_count"], json!(0));
    assert_eq!(cleared["threshold"], json!(3));
    assert_eq!(cleared["rescue_marker"], json!(false));

    let out = reg.apply_boot(true).unwrap();
    assert!(
        out.get("skipped").is_none(),
        "apply-boot must proceed after the user cleared safe mode: {out}"
    );
    assert_eq!(out["errors"], json!({}));
    assert_eq!(
        mock_live(&reg, "mock.flag"),
        json!(true),
        "the persisted setting is replayed once tuning is allowed"
    );
    assert_eq!(reg.safe_status().unwrap()["boot_count"], json!(1));
    assert_eq!(
        reg.daemon_status().unwrap()["enabled"],
        json!(true),
        "the guard is allowed again once safe mode is cleared"
    );
}

// ---- 4. hand-edited threshold ---------------------------------------------

#[test]
fn hand_edited_zero_threshold_is_normalised_on_load() {
    let (reg, _dir) = setup_mock("threshold-zero");
    std::fs::write(
        reg.ctx().config_path(),
        r#"{"version":1,"safe":{"threshold":0}}"#,
    )
    .unwrap();

    let status = reg.safe_status().unwrap();
    assert_eq!(
        status["threshold"],
        json!(1),
        "a hand-edited 0 must be clamped so clearing safe mode is not pointless"
    );
    assert_eq!(status["boot_count"], json!(0));
    assert_eq!(status["safe_mode"], json!(false));

    let cleared = reg.safe_clear().unwrap();
    assert_eq!(cleared["threshold"], json!(1));

    let out = reg.apply_boot(true).unwrap();
    assert!(
        out.get("skipped").is_none(),
        "the first boot after clear must proceed, got {out}"
    );
    assert_eq!(reg.safe_status().unwrap()["boot_count"], json!(1));

    // Only the second unconfirmed boot reaches the normalised threshold.
    let out = reg.apply_boot(true).unwrap();
    assert_eq!(out, json!({"skipped": "safe_mode"}));
    assert_eq!(reg.safe_status().unwrap()["safe_mode"], json!(true));
}

// ---- 5. boot apply disabled: the WALT-guard-only path ---------------------

#[test]
fn boot_apply_disabled_without_a_saved_guard_reports_not_configured() {
    let (reg, _dir) = setup_mock("walt-not-configured");
    let boot = reg.boot_configure(Some(false), Some(0)).unwrap();
    assert_eq!(boot["enabled"], json!(false));

    let out = reg.apply_boot(true).unwrap();
    assert_eq!(
        out,
        json!({"skipped": "boot_disabled", "walt_guard": "not_configured"})
    );
    assert_eq!(
        reg.safe_status().unwrap()["boot_count"],
        json!(0),
        "the WALT-only path is not an ordinary boot"
    );

    // `daemon start` uses the opposite reason order: "boot apply is off" wins over the marker.
    std::fs::write(reg.ctx().data_dir.join("disable_tuning"), "").unwrap();
    let out = reg.daemon_start().unwrap();
    assert_eq!(
        out,
        json!({"running": false, "enabled": false, "reason": "boot_apply_disabled"})
    );
    assert_eq!(reg.daemon_status().unwrap()["enabled"], json!(false));
}

#[cfg(unix)]
#[test]
fn boot_apply_disabled_restores_the_walt_guard_and_the_marker_stops_it() {
    let (reg, dir) = setup_cpu("walt-guard");
    let walt_node = dir.join(format!("{CPUFREQ}/policy0/walt/adaptive_level_1"));
    assert_ne!(
        mode(&walt_node),
        0o444,
        "precondition: the node is writable"
    );

    // Seed the persisted guard directly: this test is about the boot path, not `set`.
    reg.store()
        .update(|c| {
            c.modules
                .insert("cpu".to_string(), json!({"cpu.walt_guard": true}));
            Ok(())
        })
        .unwrap();
    let boot = reg.boot_configure(Some(false), Some(0)).unwrap();
    assert_eq!(boot["enabled"], json!(false));
    assert_eq!(boot["delay_s"], json!(0));

    // Rescue marker: no automatic WALT write, and the reply does not claim a skipped guard.
    std::fs::write(reg.ctx().data_dir.join("disable_tuning"), "").unwrap();
    let out = reg.apply_boot(true).unwrap();
    assert_eq!(out, json!({"skipped": "disable_tuning"}));
    assert_ne!(
        mode(&walt_node),
        0o444,
        "the rescue marker must stop the WALT guard write too"
    );
    assert_eq!(reg.safe_status().unwrap()["boot_count"], json!(0));

    std::fs::remove_file(reg.ctx().data_dir.join("disable_tuning")).unwrap();
    let out = reg.apply_boot(true).unwrap();
    assert_eq!(out["boot_disabled"], json!(true));
    assert_eq!(out["walt_guard"]["applied"]["cpu.walt_guard"], json!(true));
    assert_eq!(out["walt_guard"]["mismatch"], json!([]));
    assert_eq!(
        mode(&walt_node),
        0o444,
        "the guard must actually lock the guarded node"
    );
    assert_eq!(
        reg.safe_status().unwrap()["boot_count"],
        json!(0),
        "the WALT-only path must not advance the boot counter"
    );
}

// ---- 6. recovery surface --------------------------------------------------

#[test]
fn recovery_surface_restores_and_clears_without_leaving_safe_mode() {
    let (reg, _dir) = setup_mock("recovery");
    reg.set("mock", json!({"mock.mode": "c"}), true).unwrap();
    simulate_reboot(&reg);
    for _ in 0..3 {
        reg.apply_boot(true).unwrap();
    }
    assert_eq!(
        reg.apply_boot(true).unwrap(),
        json!({"skipped": "safe_mode"})
    );

    let status = reg.recovery_status().unwrap();
    assert_eq!(status["has_backup"], json!(true));
    assert_eq!(status["backup_modules"], json!(["mock"]));
    assert_eq!(status["safe_mode"], json!(true));
    assert_eq!(status["boot_count"], json!(3));
    assert_eq!(status["threshold"], json!(3));
    assert_eq!(status["has_corrupt_config"], json!(false));

    // Restore puts the profile back but deliberately stays in safe mode.
    let restored = reg.recovery_restore().unwrap();
    assert_eq!(restored["has_backup"], json!(false));
    assert_eq!(restored["backup_modules"], json!([]));
    assert_eq!(restored["safe_mode"], json!(true));
    let cfg = reg.store().load().unwrap();
    assert!(cfg.recovery_backup.is_none());
    assert_eq!(cfg.modules["mock"]["mock.mode"], json!("c"));
    assert!(cfg.safe.safe_mode);

    // A valid backup is discarded by `recovery clear`; safe mode is untouched.
    reg.store()
        .update(|c| {
            c.recovery_backup = json!({"mock": {"mock.mode": "c"}}).as_object().cloned();
            Ok(())
        })
        .unwrap();
    assert_eq!(reg.recovery_status().unwrap()["has_backup"], json!(true));
    let cleared = reg.recovery_clear().unwrap();
    assert_eq!(cleared["has_backup"], json!(false));
    assert!(cleared["backup_modules"].as_array().unwrap().is_empty());
    assert_eq!(cleared["safe_mode"], json!(true));
    assert!(reg.store().load().unwrap().recovery_backup.is_none());

    // A damaged config leaves a `.corrupt` file; removing it keeps the user in safe mode.
    let corrupt = reg.ctx().config_path().with_extension("json.corrupt");
    std::fs::write(&corrupt, "{ not json").unwrap();
    let status = reg.recovery_status().unwrap();
    assert_eq!(status["has_corrupt_config"], json!(true));
    assert_eq!(status["safe_mode"], json!(true));
    let cleaned = reg.recovery_clear_corrupt().unwrap();
    assert_eq!(cleaned["has_corrupt_config"], json!(false));
    assert!(!corrupt.exists());
    assert_eq!(cleaned["safe_mode"], json!(true));
}

// ---- 7. documented side contracts ----------------------------------------

#[test]
fn rescue_marker_pauses_automatic_writes_but_not_manual_tuning() {
    let (reg, _dir) = setup_mock("marker-manual");
    std::fs::write(reg.ctx().data_dir.join("disable_tuning"), "").unwrap();

    // The marker pauses *automatic* writes; a manual `set` is still allowed (§3.7).
    let out = reg.set("mock", json!({"mock.level": 60}), true).unwrap();
    assert_eq!(out["applied"]["mock.level"], json!(60));
    assert_eq!(mock_live(&reg, "mock.level"), json!(60));

    // Safe mode is the stricter state: `set` is refused, but manual recovery via `reset` works.
    reg.store()
        .update(|c| {
            c.safe.safe_mode = true;
            Ok(())
        })
        .unwrap();
    let e = reg
        .set("mock", json!({"mock.level": 70}), false)
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::SafeMode);
    assert!(e.msg.contains("safe mode"), "got: {}", e.msg);
    assert_eq!(
        mock_live(&reg, "mock.level"),
        json!(60),
        "a refused set must not write"
    );

    let reset = reg.reset("mock").unwrap();
    assert_eq!(reset["mismatch"], json!([]));
    assert_eq!(
        mock_live(&reg, "mock.level"),
        json!(50),
        "reset is manual recovery and must stay available in safe mode"
    );
}

#[test]
fn check_stays_read_only_and_available_in_safe_states() {
    let (reg, _dir) = setup_mock("check-safe");
    reg.set("mock", json!({"mock.level": 80}), true).unwrap();
    simulate_reboot(&reg);

    let check = reg.check().unwrap();
    assert_eq!(check["clean"], json!(false));
    assert_eq!(check["drift"]["mock"], json!(["mock.level"]));
    assert_eq!(check["errors"], json!({}));

    // A pure diagnostic: still runs with the rescue marker present and in safe mode (§3.12).
    std::fs::write(reg.ctx().data_dir.join("disable_tuning"), "").unwrap();
    let check = reg.check().unwrap();
    assert_eq!(check["drift"]["mock"], json!(["mock.level"]));
    assert_eq!(
        mock_live(&reg, "mock.level"),
        json!(50),
        "check must not write anything"
    );

    std::fs::remove_file(reg.ctx().data_dir.join("disable_tuning")).unwrap();
    reg.store()
        .update(|c| {
            c.safe.safe_mode = true;
            Ok(())
        })
        .unwrap();
    let check = reg.check().unwrap();
    assert_eq!(check["drift"]["mock"], json!(["mock.level"]));
    assert_eq!(mock_live(&reg, "mock.level"), json!(50));
}
