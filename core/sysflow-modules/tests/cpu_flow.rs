//! Registry + real CpuModule on a fake sysfs tree: persistence, reset, boot-time tolerance,
//! and structured event emission across lifecycle / enforcement / adaptive transitions.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use sysflow_core::ctx::Ctx;
use sysflow_core::event::{EventFilter, EventStore, SystemEvent};
use sysflow_core::registry::Registry;
use sysflow_core::sysfs::Sysfs;
use sysflow_core::Module;
use sysflow_modules::cpu::CpuModule;

const CPUFREQ: &str = "sys/devices/system/cpu/cpufreq";

fn put(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel))
        .unwrap()
        .trim()
        .to_string()
}

/// 4 cores: policy0 = cpu0-1, policy2 = cpu2-3. policy2 is currently thermal-capped to 1.8 GHz.
fn setup(name: &str) -> (Registry, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "sysflow-cpu-flow-{name}-{}-{nanos}",
        std::process::id()
    ));
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
    let ctx = Ctx {
        data_dir: dir.join("data"),
        module_dir: dir.join("moddir"),
        sysfs: Sysfs::new(&dir),
    };
    let state_dir = ctx.data_dir.join("state");
    let modules: Vec<Box<dyn Module>> = vec![Box::new(CpuModule::with_state_dir(
        Sysfs::new(&dir),
        state_dir,
    ))];
    (Registry::new(ctx, modules), dir)
}

fn persisted(reg: &Registry, key: &str) -> Value {
    let cfg = reg.store().load().unwrap();
    cfg.modules
        .get("cpu")
        .and_then(|m| m.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn events_in(dir: &Path) -> Vec<SystemEvent> {
    EventStore::new(&dir.join("data"))
        .list(EventFilter {
            limit: 100,
            ascending: true,
            ..Default::default()
        })
        .unwrap()
}

#[test]
fn persist_keeps_only_the_clusters_the_user_touched() {
    let (reg, _dir) = setup("persist");
    reg.set("cpu", json!({"cpu.max_freq": {"2": 1500000}}), true)
        .unwrap();
    // cluster 0 was not requested: its live value must not be frozen into the config
    assert_eq!(persisted(&reg, "cpu.max_freq"), json!({"2": 1500000}));

    reg.set("cpu", json!({"cpu.max_freq": {"0": 1000000}}), true)
        .unwrap();
    // a later change to another cluster merges instead of replacing
    assert_eq!(
        persisted(&reg, "cpu.max_freq"),
        json!({"0": 1000000, "2": 1500000})
    );
}

#[test]
fn reset_restores_the_hardware_range_not_the_transient_value() {
    let (reg, dir) = setup("reset");
    // policy2 is capped to 1.8 GHz right now; the hardware ceiling is 2.4 GHz
    reg.set("cpu", json!({"cpu.max_freq": {"2": 1500000}}), true)
        .unwrap();
    assert_eq!(
        read(&dir, &format!("{CPUFREQ}/policy2/scaling_max_freq")),
        "1500000"
    );
    reg.reset("cpu").unwrap();
    assert_eq!(
        read(&dir, &format!("{CPUFREQ}/policy2/scaling_max_freq")),
        "2400000"
    );
    // the other cluster ends up at its hardware ceiling as well (unchanged here)
    assert_eq!(
        read(&dir, &format!("{CPUFREQ}/policy0/scaling_max_freq")),
        "1800000"
    );
}

#[test]
fn apply_boot_survives_stale_saved_entries() {
    let (reg, dir) = setup("boot");
    reg.store()
        .update(|c| {
            c.modules.insert(
                "cpu".to_string(),
                json!({
                    "cpu.max_freq": {"0": 123, "2": 1500000},
                    "cpu.governor": {"0": "ghost", "2": "performance"}
                }),
            );
            Ok(())
        })
        .unwrap();
    let out = reg.apply_boot(true).unwrap();
    assert!(out.get("skipped").is_none(), "unexpected skip: {out}");
    assert_eq!(out["errors"], json!({}), "{out}");
    // the valid entries were applied, the stale ones were dropped
    assert_eq!(
        read(&dir, &format!("{CPUFREQ}/policy2/scaling_max_freq")),
        "1500000"
    );
    assert_eq!(
        read(&dir, &format!("{CPUFREQ}/policy2/scaling_governor")),
        "performance"
    );
    assert_eq!(
        read(&dir, &format!("{CPUFREQ}/policy0/scaling_max_freq")),
        "1800000"
    );
    assert_eq!(
        read(&dir, &format!("{CPUFREQ}/policy0/scaling_governor")),
        "schedutil"
    );
}

#[test]
fn check_reports_drift_without_writing() {
    let (reg, dir) = setup("check");
    reg.set("cpu", json!({"cpu.governor": {"2": "performance"}}), true)
        .unwrap();
    assert!(reg.check().unwrap()["clean"].as_bool().unwrap());
    // something (thermal HAL, vendor service) rewrote the node
    put(
        &dir,
        &format!("{CPUFREQ}/policy2/scaling_governor"),
        "schedutil",
    );
    let out = reg.check().unwrap();
    assert_eq!(out["clean"], json!(false));
    assert_eq!(out["drift"]["cpu"], json!(["cpu.governor"]));
    // the drift check is read-only
    assert_eq!(
        read(&dir, &format!("{CPUFREQ}/policy2/scaling_governor")),
        "schedutil"
    );
}

#[test]
fn boot_clears_settings_applied_without_persist() {
    let (reg, _dir) = setup("boot-reset");
    reg.set(
        "cpu",
        json!({"cpu.adaptive": true, "cpu.screen_off": true}),
        false,
    )
    .unwrap();
    // not persisted, but live module state reflects the change
    assert!(reg.store().load().unwrap().modules.get("cpu").is_none());
    assert_eq!(
        reg.snapshot().unwrap()["modules"]["cpu"]["cpu.adaptive"],
        json!(true)
    );

    let out = reg.apply_boot(true).unwrap();
    assert!(out.get("skipped").is_none(), "unexpected skip: {out}");
    // a new boot starts from clean module-local state
    let snap = reg.snapshot().unwrap();
    assert_eq!(snap["modules"]["cpu"]["cpu.adaptive"], json!(false));
    assert_eq!(snap["modules"]["cpu"]["cpu.screen_off"], json!(false));
}

#[test]
fn set_emits_events_for_temporary_and_mismatch_but_skips_screen_off_active() {
    let (reg, dir) = setup("events-set");
    reg.set("cpu", json!({"cpu.max_freq": {"2": 1500000}}), false)
        .unwrap();
    // Purely ephemeral runtime toggle must not spam configuration settings_applied events.
    reg.set("cpu", json!({"cpu.screen_off_active": false}), false)
        .unwrap();

    let initial = events_in(&dir);
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].module, "cpu");
    assert_eq!(initial[0].event_type, "settings_applied");
    assert_eq!(initial[0].severity, "info");
    assert_eq!(initial[0].metadata["persist"], json!(false));

    put(&dir, "dev/cpuset/cpus", "0-3");
    put(&dir, "dev/cpuset/top-app/cpus", "0-3");
    put(&dir, "sys/devices/system/cpu/possible", "0-3");
    put(&dir, "sys/devices/system/cpu/cpu3/online", "0");

    reg.set("cpu", json!({"cpu.cpuset.top-app": [1, 1, 1, 1]}), true)
        .unwrap();
    let after = events_in(&dir);
    assert_eq!(after.len(), 2);
    assert_eq!(after[1].event_type, "settings_partially_applied");
    assert_eq!(after[1].severity, "warning");
    assert_eq!(after[1].metadata["persist"], json!(true));
    assert_eq!(after[1].metadata["mismatch"], json!(["cpu.cpuset.top-app"]));
}

#[test]
fn enforce_emits_deduped_drift_repaired_event() {
    let (reg, dir) = setup("events-enforce");
    reg.set("cpu", json!({"cpu.governor": {"2": "performance"}}), true)
        .unwrap();

    put(
        &dir,
        &format!("{CPUFREQ}/policy2/scaling_governor"),
        "schedutil",
    );
    reg.enforce().unwrap();

    put(
        &dir,
        &format!("{CPUFREQ}/policy2/scaling_governor"),
        "schedutil",
    );
    reg.enforce().unwrap();

    let repaired: Vec<SystemEvent> = events_in(&dir)
        .into_iter()
        .filter(|e| e.event_type == "drift_repaired")
        .collect();
    assert_eq!(repaired.len(), 1);
    assert_eq!(repaired[0].module, "cpu");
    assert_eq!(repaired[0].category, "enforcement");
    assert_eq!(repaired[0].severity, "notice");
    assert_eq!(repaired[0].metadata["drift"], json!(["cpu.governor"]));
}

#[test]
fn apply_boot_emits_stale_dropped_and_applied_events() {
    let (reg, dir) = setup("events-boot");
    reg.store()
        .update(|c| {
            c.modules.insert(
                "cpu".to_string(),
                json!({
                    "cpu.max_freq": {"0": 123, "2": 1500000},
                    "cpu.governor": {"0": "ghost", "2": "performance"}
                }),
            );
            Ok(())
        })
        .unwrap();

    reg.apply_boot(true).unwrap();
    let events = events_in(&dir);
    let dropped = events
        .iter()
        .find(|e| e.event_type == "stale_settings_dropped")
        .expect("expected stale_settings_dropped event");
    assert_eq!(dropped.module, "cpu");
    assert_eq!(dropped.severity, "warning");

    let applied = events
        .iter()
        .find(|e| e.event_type == "boot_settings_applied")
        .expect("expected boot_settings_applied event");
    assert_eq!(applied.module, "cpu");
    assert_eq!(applied.severity, "info");
}

#[test]
fn disabling_adaptive_emits_released_events() {
    let (reg, dir) = setup("events-adaptive-disable");
    put(&dir, "sys/class/thermal/thermal_zone0/type", "cpu-0-0-usr");
    put(&dir, "sys/class/thermal/thermal_zone0/temp", "85000");

    reg.set(
        "cpu",
        json!({"cpu.adaptive": true, "cpu.adaptive.thermal": true}),
        true,
    )
    .unwrap();
    reg.set("cpu", json!({"cpu.adaptive": false}), true)
        .unwrap();

    let thermal: Vec<SystemEvent> = events_in(&dir)
        .into_iter()
        .filter(|e| e.event_type == "thermal_protection_transition")
        .collect();
    assert_eq!(thermal.len(), 2, "{thermal:?}");
    assert_eq!(thermal[0].title, "CPU thermal protection engaged");
    assert_eq!(thermal[1].title, "CPU thermal protection released");
    assert_eq!(thermal[1].metadata["reason"], json!("adaptive_disabled"));
}

#[test]
fn reset_emits_adaptive_release_and_module_reset_events() {
    let (reg, dir) = setup("events-adaptive-reset");
    put(&dir, "sys/class/thermal/thermal_zone0/type", "cpu-0-0-usr");
    put(&dir, "sys/class/thermal/thermal_zone0/temp", "85000");

    reg.set(
        "cpu",
        json!({"cpu.adaptive": true, "cpu.adaptive.thermal": true}),
        true,
    )
    .unwrap();
    reg.reset("cpu").unwrap();

    let events = events_in(&dir);
    let released = events
        .iter()
        .find(|e| {
            e.event_type == "thermal_protection_transition"
                && e.title == "CPU thermal protection released"
        })
        .expect("expected thermal release event on module reset");
    assert_eq!(released.metadata["reason"], json!("module_reset"));
    assert!(events.iter().any(|e| e.event_type == "module_reset"));
}

fn defaults(reg: &Registry) -> Value {
    let cfg = reg.store().load().unwrap();
    cfg.defaults_snapshot
        .get("cpu")
        .cloned()
        .unwrap_or(Value::Null)
}

/// The defaults are what `reset` restores, and they are otherwise written once and never refreshed.
/// Re-seeding them is the only way a client can correct a baseline captured before the hardware
/// changed; this proves the new value is stored *and* is what reset actually writes back.
#[test]
fn rebaselined_defaults_are_what_reset_restores() {
    let (reg, dir) = setup("rebaseline");
    // Touch both clusters so a first-run default exists for each.
    reg.set(
        "cpu",
        json!({"cpu.min_freq": {"0": 300000, "2": 1800000}}),
        true,
    )
    .unwrap();
    let before = defaults(&reg);
    let cluster0_default = before["cpu.min_freq"]["0"].clone();
    assert_eq!(cluster0_default, json!(300000));

    // Re-seed one cluster only: the other cluster's default must survive the merge.
    // 1500000 is in policy2's table, and differs from both the live value (1800000) and the
    // originally recorded default (500000 kHz, the hardware floor), so the reset below can only
    // pass by having used the new baseline.
    let values = json!({"cpu.min_freq": {"2": 1500000}});
    reg.set_defaults("cpu", values.as_object().unwrap().clone())
        .unwrap();

    let after = defaults(&reg);
    assert_eq!(after["cpu.min_freq"]["2"], json!(1500000));
    assert_eq!(after["cpu.min_freq"]["0"], cluster0_default);

    // Reset must write the new baseline through to the kernel node, not just into the config.
    reg.reset("cpu").unwrap();
    assert_eq!(
        read(&dir, &format!("{CPUFREQ}/policy2/scaling_min_freq")),
        "1500000"
    );
}

/// A read-only item can never be a default: the snapshot is applied through the module's write path,
/// so storing one would look accepted and then silently do nothing on restore.
#[test]
fn read_only_items_cannot_be_stored_as_defaults() {
    let (reg, _dir) = setup("rebaseline-readonly");
    let values = json!({"cpu.cur_freq": {"0": 1000000}});
    let error = reg
        .set_defaults("cpu", values.as_object().unwrap().clone())
        .unwrap_err();
    assert!(
        error.msg.contains("non-writable"),
        "unexpected error: {}",
        error.msg
    );
}
