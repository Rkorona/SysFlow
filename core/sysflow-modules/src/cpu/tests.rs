//! CPU module tests on a fake sysfs/procfs tree.

use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Tree {
    root: PathBuf,
}

impl Tree {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sysflow-cpu-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let t = Tree { root };
        for cpu in 0..8 {
            t.put(&format!("sys/devices/system/cpu/cpu{cpu}/x"), "");
            if cpu != 0 {
                t.put(&format!("sys/devices/system/cpu/cpu{cpu}/online"), "1");
            }
        }
        t.policy(
            0,
            "0 1 2 3",
            "300000 1000000 1800000",
            "300000",
            "1800000",
            "schedutil",
        );
        t.policy(
            4,
            "4 5 6 7",
            "500000 1500000 2400000",
            "500000",
            "2400000",
            "schedutil",
        );
        for (id, mn, mx) in [(0, "300000", "1800000"), (4, "500000", "2400000")] {
            t.put(
                &format!("sys/devices/system/cpu/cpufreq/policy{id}/cpuinfo_min_freq"),
                mn,
            );
            t.put(
                &format!("sys/devices/system/cpu/cpufreq/policy{id}/cpuinfo_max_freq"),
                mx,
            );
            t.put(
                &format!("sys/devices/system/cpu/cpufreq/policy{id}/scaling_cur_freq"),
                mx,
            );
        }
        t.put(
            "sys/devices/system/cpu/cpu1/cpufreq/scaling_cur_freq",
            "1000000",
        );
        t.put(
            "sys/devices/system/cpu/cpu4/cpufreq/scaling_cur_freq",
            "1500000",
        );
        t.put("sys/devices/system/cpu/possible", "0-7");
        t.put("sys/class/thermal/thermal_zone0/type", "cpu-0-0-usr");
        t.put("sys/class/thermal/thermal_zone0/temp", "42500");
        t.put("sys/class/thermal/thermal_zone1/type", "cpuss-1");
        t.put("sys/class/thermal/thermal_zone1/temp", "47000");
        t.put("sys/class/thermal/thermal_zone2/type", "battery");
        t.put("sys/class/thermal/thermal_zone2/temp", "99000");
        t.proc_stat(&[100, 100, 100, 100, 100, 100, 100, 100], 100);
        t.put(
            "proc/pressure/cpu",
            "some avg10=0.00 avg60=0.00 avg300=0.00 total=0",
        );
        t.put("sys/class/power_supply/battery/capacity", "80");
        t.put("sys/class/power_supply/battery/status", "Discharging");
        t
    }

    fn put(&self, rel: &str, content: &str) {
        let p = self.root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn policy(&self, id: u32, cpus: &str, freqs: &str, min: &str, max: &str, gov: &str) {
        let d = format!("sys/devices/system/cpu/cpufreq/policy{id}");
        self.put(&format!("{d}/related_cpus"), cpus);
        self.put(&format!("{d}/scaling_available_frequencies"), freqs);
        self.put(
            &format!("{d}/scaling_available_governors"),
            "schedutil performance powersave",
        );
        self.put(&format!("{d}/scaling_min_freq"), min);
        self.put(&format!("{d}/scaling_max_freq"), max);
        self.put(&format!("{d}/scaling_governor"), gov);
    }

    fn proc_stat(&self, busy: &[u64], idle: u64) {
        let mut text = String::from("cpu 0 0 0 0 0 0 0 0 0 0\n");
        for (cpu, b) in busy.iter().enumerate() {
            text.push_str(&format!("cpu{cpu} {b} 0 0 {idle} 0 0 0 0 0 0\n"));
        }
        self.put("proc/stat", &text);
    }

    fn walt(&self, policy: u32, key: &str, value: &str) {
        self.put(
            &format!("sys/devices/system/cpu/cpufreq/policy{policy}/walt/{key}"),
            value,
        );
    }

    fn gpu(&self) {
        let d = "sys/class/devfreq/kgsl-3d0";
        self.put(&format!("{d}/cur_freq"), "500000000");
        self.put(&format!("{d}/min_freq"), "300000000");
        self.put(&format!("{d}/max_freq"), "800000000");
        self.put(
            &format!("{d}/available_frequencies"),
            "300000000 500000000 800000000",
        );
        self.put(&format!("{d}/governor"), "msm-adreno-tz");
        self.put(
            &format!("{d}/available_governors"),
            "msm-adreno-tz performance powersave",
        );
        self.put("sys/class/kgsl/kgsl-3d0/dcvs_boost", "0");
    }

    fn cpuset(&self) {
        self.put("dev/cpuset/cpus", "0-7");
        self.put("dev/cpuset/top-app/cpus", "0-7");
        self.put("dev/cpuset/background/cpus", "0-3");
        self.put("dev/cpuset/system-background/cpus", "0-3");
    }

    fn mem(&self) {
        self.put("sys/devices/system/cpu/bus_dcvs/DDR/boost_freq", "300");
        self.put(
            "sys/devices/system/cpu/bus_dcvs/DDR/available_frequencies",
            "100 200 300",
        );
        self.put("sys/devices/system/cpu/bus_dcvs/LLCC/boost_freq", "400");
        self.put(
            "sys/devices/system/cpu/bus_dcvs/LLCC/available_frequencies",
            "200 400 600",
        );
    }

    fn module(&self) -> CpuModule {
        CpuModule::new(Sysfs::new(&self.root))
    }

    fn stateful(&self) -> CpuModule {
        CpuModule::with_state_dir(Sysfs::new(&self.root), self.root.join("state"))
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root.join(rel))
            .unwrap()
            .trim()
            .to_string()
    }

    fn mode(&self, rel: &str) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(self.root.join(rel))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn req(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn items<'a>(schema: &'a Value, id: &str) -> &'a Value {
    schema["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == id)
        .unwrap_or_else(|| panic!("missing schema item {id}"))
}

#[test]
fn proc_stat_aggregate_row_is_not_a_per_core_entry() {
    let t = Tree::new();
    // A non-zero aggregate row must not be mistaken for cpu0's counters.
    t.put(
        "proc/stat",
        "cpu 900 0 100 200 0 0 0 0 0 0\ncpu0 10 0 5 20 0 0 0 0 0 0\ncpu1 20 0 5 20 0 0 0 0 0 0\n",
    );
    let stat = adaptive::read_proc_stat(&Sysfs::new(&t.root)).unwrap();
    assert_eq!(stat.len(), 2);
    assert_eq!(stat.get(&0), Some(&(15, 35)));
    assert_eq!(stat.get(&1), Some(&(25, 45)));
    assert!(!stat.contains_key(&900));
}

#[test]
fn adaptive_telemetry_items_are_exposed() {
    let t = Tree::new();
    t.put("sys/class/kgsl/kgsl-3d0/gpu_busy_percentage", "62\n");
    t.put("sys/class/power_supply/battery/temp", "347\n");
    let module = t.module();
    let schema = serde_json::to_value(module.schema()).unwrap();
    for key in ["cpu.gpu_busy", "cpu.battery_temp", "cpu.thermal_sensors"] {
        assert!(schema["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == key));
    }
    let values = module.collect(None).unwrap();
    assert_eq!(values["cpu.gpu_busy"], json!(62));
    assert_eq!(values["cpu.battery_temp"], json!(347));
    let sensors = values["cpu.thermal_sensors"].as_array().unwrap();
    assert_eq!(sensors.len(), 2);
    assert_eq!(sensors[0]["participates"], json!(true));
}

#[test]
fn probe_reports_features() {
    let t = Tree::new();
    t.gpu();
    t.cpuset();
    t.walt(4, "hispeed_load", "80");
    t.walt(4, "adaptive_high_freq", "2000000");
    t.walt(4, "adaptive_level_1", "50");
    t.mem();
    let caps = t.module().probe();
    assert!(caps.supported);
    for f in [
        "online",
        "max_freq",
        "min_freq",
        "governor",
        "gpu",
        "cpuset",
        "walt",
        "walt_guard",
        "screen_off",
        "adaptive",
        "mem_boost",
    ] {
        assert!(
            caps.features.iter().any(|x| x == f),
            "missing feature {f}: {:?}",
            caps.features
        );
    }
}

#[test]
fn probe_unsupported_without_sysfs() {
    let t = Tree {
        root: std::env::temp_dir().join("sysflow-cpu-test-missing"),
    };
    assert!(!t.module().probe().supported);
}

#[test]
fn schema_has_runtime_options_and_locks() {
    let t = Tree::new();
    let v = serde_json::to_value(t.module().schema()).unwrap();
    let online = items(&v, "cpu.online");
    assert_eq!(online["count"], 8);
    assert_eq!(online["locked"], json!([0]));
    let max = items(&v, "cpu.max_freq");
    assert_eq!(max["scope"], "cluster");
    assert_eq!(
        max["options"],
        json!([300000, 500000, 1000000, 1500000, 1800000, 2400000])
    );
    assert_eq!(items(&v, "cpu.screen_off.max_freq")["type"], "int_range");
    assert_eq!(items(&v, "cpu.screen_off.max_freq")["min"], json!(0));
    assert_eq!(items(&v, "cpu.adaptive")["type"], "bool");
}

#[test]
fn collect_reads_everything() {
    let t = Tree::new();
    t.walt(4, "hispeed_load", "80");
    t.put(
        "sys/devices/system/cpu/cpufreq/policy0/stats/time_in_state",
        "300000 10\n1800000 5",
    );
    let v = t.module().collect(None).unwrap();
    assert_eq!(v["cpu.online"], json!([1, 1, 1, 1, 1, 1, 1, 1]));
    assert_eq!(v["cpu.max_freq"], json!({"0": 1800000, "4": 2400000}));
    assert_eq!(v["cpu.min_freq"], json!({"0": 300000, "4": 500000}));
    assert_eq!(
        v["cpu.governor"],
        json!({"0": "schedutil", "4": "schedutil"})
    );
    assert_eq!(v["cpu.cur_freq"], json!({"1": 1000000, "4": 1500000}));
    assert_eq!(
        v["cpu.cluster_cur_freq"],
        json!({"0": 1800000, "4": 2400000})
    );
    assert_eq!(v["cpu.cluster_hw_cur_freq"], json!({}));
    assert_eq!(
        v["cpu.clusters"],
        json!({"0": [0, 1, 2, 3], "4": [4, 5, 6, 7]})
    );
    assert_eq!(v["cpu.time_in_state"]["0"]["300000"], json!(10));
    assert_eq!(v["cpu.stat"]["0"], json!({"busy": 400, "total": 800}));
    assert_eq!(
        v["cpu.usage_stat"]["cpu0"],
        json!({"busy": 100, "total": 200})
    );
    assert_eq!(
        v["cpu.usage_stat"]["cpu"],
        json!({"busy": 800, "total": 1600})
    );
    assert_eq!(v["cpu.walt.hispeed_load"], json!({"4": 80}));
    // hottest cpu zone (47.0 C); the battery zone is ignored
    assert_eq!(v["cpu.temp"], json!(470));
    // non-sysfs items come from the (default) module state
    assert_eq!(v["cpu.adaptive"], json!(false));
    assert_eq!(v["cpu.screen_off"], json!(false));
    assert_eq!(v["cpu.screen_off_active"], json!(false));
}

#[test]
fn collect_reports_hardware_current_frequency_separately() {
    let t = Tree::new();
    t.put(
        "sys/devices/system/cpu/cpufreq/policy0/cpuinfo_cur_freq",
        "1123000",
    );
    let v = t.module().collect(None).unwrap();
    assert_eq!(v["cpu.cluster_hw_cur_freq"], json!({"0": 1123000}));
}

#[test]
fn collect_filters_keys() {
    let t = Tree::new();
    let keys = vec!["cpu.governor".to_string()];
    let v = t.module().collect(Some(&keys)).unwrap();
    assert_eq!(v.as_object().unwrap().len(), 1);
}

#[test]
fn temp_whole_degrees_and_missing() {
    let t = Tree::new();
    t.put("sys/class/thermal/thermal_zone1/temp", "50");
    t.put("sys/class/thermal/thermal_zone0/temp", "30");
    assert_eq!(t.module().collect(None).unwrap()["cpu.temp"], json!(500));
    fs::remove_dir_all(t.root.join("sys/class/thermal")).unwrap();
    assert_eq!(t.module().collect(None).unwrap()["cpu.temp"], Value::Null);
}

#[test]
fn online_switches_cores_and_reads_back() {
    let t = Tree::new();
    let r = t
        .module()
        .apply(&req(json!({"cpu.online": [1,1,1,1,1,1,0,0]})))
        .unwrap();
    assert!(r.mismatch.is_empty());
    assert_eq!(r.applied["cpu.online"], json!([1, 1, 1, 1, 1, 1, 0, 0]));
    assert_eq!(t.read("sys/devices/system/cpu/cpu7/online"), "0");
    assert_eq!(t.read("sys/devices/system/cpu/cpu5/online"), "1");
}

#[test]
fn cpu0_and_wrong_length_are_rejected_without_writing() {
    let t = Tree::new();
    let m = t.module();
    assert!(m
        .apply(&req(json!({"cpu.online": [0,1,1,1,1,1,1,1]})))
        .is_err());
    assert!(m.apply(&req(json!({"cpu.online": [1,1]}))).is_err());
    assert!(m
        .apply(&req(json!({"cpu.online": [0,0,0,0,0,0,0,0]})))
        .is_err());
    assert_eq!(t.read("sys/devices/system/cpu/cpu1/online"), "1");
}

#[test]
fn freq_and_governor_apply() {
    let t = Tree::new();
    let r = t
        .module()
        .apply(&req(json!({
            "cpu.max_freq": {"4": 1500000},
            "cpu.min_freq": {"4": 1500000},
            "cpu.governor": {"0": "performance"}
        })))
        .unwrap();
    assert!(r.mismatch.is_empty(), "{:?}", r.mismatch);
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/scaling_max_freq"),
        "1500000"
    );
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/scaling_min_freq"),
        "1500000"
    );
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy0/scaling_governor"),
        "performance"
    );
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy0/scaling_max_freq"),
        "1800000"
    );
}

#[test]
fn new_range_entirely_below_current_floor_is_applied() {
    let t = Tree::new();
    t.put(
        "sys/devices/system/cpu/cpufreq/policy0/scaling_min_freq",
        "1000000",
    );
    let r = t
        .module()
        .apply(&req(
            json!({"cpu.min_freq": {"0": 300000}, "cpu.max_freq": {"0": 300000}}),
        ))
        .unwrap();
    assert!(r.mismatch.is_empty());
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy0/scaling_min_freq"),
        "300000"
    );
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy0/scaling_max_freq"),
        "300000"
    );
}

#[test]
fn invalid_requests_are_rejected_before_any_write() {
    let t = Tree::new();
    let m = t.module();
    assert!(m
        .apply(&req(json!({"cpu.max_freq": {"0": 2400000}})))
        .is_err());
    assert!(m
        .apply(&req(json!({"cpu.governor": {"0": "ondemand"}})))
        .is_err());
    assert!(m
        .apply(&req(
            json!({"cpu.min_freq": {"0": 1800000}, "cpu.max_freq": {"0": 1000000}})
        ))
        .is_err());
    assert!(m
        .apply(&req(json!({"cpu.max_freq": {"big": 1500000}})))
        .is_err());
    assert!(m
        .apply(&req(
            json!({"cpu.online": [1,1,1,1,1,1,1,0], "cpu.governor": {"0": "nope"}})
        ))
        .is_err());
    assert_eq!(t.read("sys/devices/system/cpu/cpu7/online"), "1");
}

#[test]
fn unknown_cluster_is_rejected_instead_of_becoming_a_silent_mismatch() {
    let t = Tree::new();
    let err = t
        .module()
        .apply(&req(json!({"cpu.max_freq": {"7": 1500000}})))
        .unwrap_err();
    assert!(err.to_string().contains("policy7 does not exist"), "{err}");
}

#[test]
fn persist_on_mismatch_includes_frequency_limits() {
    let t = Tree::new();
    let m = t.module();
    assert!(m.persist_on_mismatch("cpu.max_freq"));
    assert!(m.persist_on_mismatch("cpu.min_freq"));
    assert!(m.persist_on_mismatch("cpu.cpuset.top-app"));
    assert!(!m.persist_on_mismatch("cpu.governor"));
}

#[test]
fn hardware_range_is_always_selectable() {
    let t = Tree::new();
    t.put(
        "sys/devices/system/cpu/cpufreq/policy0/cpuinfo_min_freq",
        "200000",
    );
    t.put(
        "sys/devices/system/cpu/cpufreq/policy0/cpuinfo_max_freq",
        "1900000",
    );
    let v = serde_json::to_value(t.module().schema()).unwrap();
    let opts = items(&v, "cpu.max_freq")["options"].as_array().unwrap();
    assert!(opts.contains(&json!(200000)) && opts.contains(&json!(1900000)));
}

#[test]
fn temp_matches_vendor_zone_names() {
    let t = Tree::new();
    t.put("sys/class/thermal/thermal_zone3/type", "mtktscpu");
    t.put("sys/class/thermal/thermal_zone3/temp", "52000");
    t.put("sys/class/thermal/thermal_zone4/type", "cpu-trip-0");
    t.put("sys/class/thermal/thermal_zone4/temp", "95000");
    assert_eq!(t.module().collect(None).unwrap()["cpu.temp"], json!(520));
}

#[test]
fn default_snapshot_uses_the_hardware_range() {
    let t = Tree::new();
    t.put(
        "sys/devices/system/cpu/cpufreq/policy0/scaling_min_freq",
        "1000000",
    );
    t.put(
        "sys/devices/system/cpu/cpufreq/policy0/scaling_max_freq",
        "1000000",
    );
    let m = t.module();
    let keys = vec![
        "cpu.min_freq".to_string(),
        "cpu.max_freq".to_string(),
        "cpu.governor".to_string(),
    ];
    let cur = m.collect(Some(&keys)).unwrap().as_object().unwrap().clone();
    let d = m.normalize_default(cur);
    assert_eq!(d["cpu.min_freq"], json!({"0": 300000, "4": 500000}));
    assert_eq!(d["cpu.max_freq"], json!({"0": 1800000, "4": 2400000}));
    assert_eq!(
        d["cpu.governor"],
        json!({"0": "schedutil", "4": "schedutil"})
    );
}

#[test]
fn sanitize_boot_drops_only_the_stale_entries() {
    let t = Tree::new();
    let m = t.module();
    let saved = req(json!({
        "cpu.online": [1, 1],
        "cpu.max_freq": {"0": 123, "4": 1500000},
        "cpu.min_freq": {"4": 2400000},
        "cpu.governor": {"0": "ghost", "4": "performance"}
    }));
    let out = m.sanitize_boot(&saved);
    assert!(out.get("cpu.online").is_none());
    assert_eq!(out["cpu.max_freq"], json!({"4": 1500000}));
    assert!(out.get("cpu.min_freq").is_none());
    assert_eq!(out["cpu.governor"], json!({"4": "performance"}));
    let r = m.apply(&out).unwrap();
    assert!(r.mismatch.is_empty(), "{:?}", r.mismatch);
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/scaling_max_freq"),
        "1500000"
    );
}

#[test]
fn sanitize_boot_keeps_valid_and_offline_cluster_entries() {
    let t = Tree::new();
    let saved = req(json!({
        "cpu.online": [1, 1, 1, 1, 1, 1, 0, 0],
        "cpu.max_freq": {"4": 1500000, "7": 1500000}
    }));
    assert_eq!(t.module().sanitize_boot(&saved), saved);
}

#[test]
fn failed_cluster_write_restores_clusters_already_changed() {
    let t = Tree::new();
    let gov4 = t
        .root
        .join("sys/devices/system/cpu/cpufreq/policy4/scaling_governor");
    fs::remove_file(&gov4).unwrap();
    fs::create_dir(&gov4).unwrap();
    let r = t.module().apply(&req(json!({
        "cpu.governor": {"0": "performance", "4": "performance"},
        "cpu.max_freq": {"0": 1000000}
    })));
    assert!(r.is_err());
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy0/scaling_governor"),
        "schedutil"
    );
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy0/scaling_max_freq"),
        "1800000"
    );
}

#[test]
fn cur_freq_falls_back_to_cpuinfo_cur_freq() {
    let t = Tree::new();
    fs::remove_file(
        t.root
            .join("sys/devices/system/cpu/cpu4/cpufreq/scaling_cur_freq"),
    )
    .unwrap();
    t.put(
        "sys/devices/system/cpu/cpu4/cpufreq/cpuinfo_cur_freq",
        "1234000",
    );
    let v = t.module().collect(None).unwrap();
    assert_eq!(v["cpu.cur_freq"]["4"], json!(1234000));
}

#[test]
fn walt_parameter_roundtrip_and_guard() {
    let t = Tree::new();
    t.walt(0, "hispeed_load", "80");
    t.walt(4, "hispeed_load", "70");
    t.walt(4, "adaptive_level_1", "50");
    t.walt(4, "boost", "0");
    let m = t.stateful();
    let keys = vec!["cpu.walt.hispeed_load".to_string()];
    let v = m.collect(Some(&keys)).unwrap();
    assert_eq!(v["cpu.walt.hispeed_load"], json!({"0": 80, "4": 70}));

    let r = m
        .apply(&req(json!({"cpu.walt.hispeed_load": {"4": 90}})))
        .unwrap();
    assert!(r.mismatch.is_empty());
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/walt/hispeed_load"),
        "90"
    );

    let r = m.apply(&req(json!({"cpu.walt_guard": true}))).unwrap();
    assert!(r.mismatch.is_empty(), "{:?}", r.mismatch);
    assert_eq!(
        t.mode("sys/devices/system/cpu/cpufreq/policy4/walt/adaptive_level_1"),
        0o444
    );
    let v = m.collect(None).unwrap();
    assert_eq!(v["cpu.walt_guard"], json!(true));

    m.apply(&req(json!({"cpu.walt_guard": false}))).unwrap();
    assert_eq!(
        t.mode("sys/devices/system/cpu/cpufreq/policy4/walt/adaptive_level_1"),
        0o644
    );
}

#[test]
fn walt_adaptive_is_forced_to_zero_while_capped_and_restored() {
    let t = Tree::new();
    for policy in [0, 4] {
        t.walt(policy, "adaptive_high_freq", "2400000");
        t.walt(policy, "adaptive_low_freq", "500000");
    }
    let m = t.stateful();
    m.apply(&req(json!({"cpu.max_freq": {"4": 1500000}})))
        .unwrap();
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/walt/adaptive_high_freq"),
        "0"
    );
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/walt/adaptive_low_freq"),
        "0"
    );
    // policy0 is not capped
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy0/walt/adaptive_high_freq"),
        "2400000"
    );

    m.apply(&req(json!({"cpu.max_freq": {"4": 2400000}})))
        .unwrap();
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/walt/adaptive_high_freq"),
        "2400000"
    );
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/walt/adaptive_low_freq"),
        "500000"
    );
}

#[test]
fn gpu_frequency_and_extras() {
    let t = Tree::new();
    t.gpu();
    t.put("sys/class/devfreq/kgsl-busmon/max_freq", "800000000");
    let m = t.stateful();
    let v = m.collect(None).unwrap();
    assert_eq!(v["cpu.gpu_min_freq"], json!(300000));
    assert_eq!(v["cpu.gpu_max_freq"], json!(800000));
    assert_eq!(v["cpu.gpu_governor"], json!("msm-adreno-tz"));

    let r = m
        .apply(&req(json!({
            "cpu.gpu_min_freq": 300000,
            "cpu.gpu_max_freq": 500000,
            "cpu.gpu_governor": "performance",
            "cpu.gpu_dcvs_boost": true,
            "cpu.gpu_bus_boost": true
        })))
        .unwrap();
    assert!(r.mismatch.is_empty(), "{:?}", r.mismatch);
    assert_eq!(t.read("sys/class/devfreq/kgsl-3d0/max_freq"), "500000000");
    assert_eq!(t.read("sys/class/devfreq/kgsl-3d0/governor"), "performance");
    assert_eq!(t.read("sys/class/kgsl/kgsl-3d0/dcvs_boost"), "1");
    assert_eq!(
        t.read("sys/class/devfreq/kgsl-busmon/max_freq"),
        "500000000"
    );
}

#[test]
fn ddr_boost_set_and_reset() {
    let t = Tree::new();
    t.mem();
    let m = t.stateful();
    let v = m.collect(None).unwrap();
    assert_eq!(v["cpu.ddr_boost_freq"], json!(300));
    let r = m.apply(&req(json!({"cpu.ddr_boost_freq": 100}))).unwrap();
    assert!(r.mismatch.is_empty(), "{:?}", r.mismatch);
    assert_eq!(
        t.read("sys/devices/system/cpu/bus_dcvs/DDR/boost_freq"),
        "100"
    );
    m.reset().unwrap();
    assert_eq!(
        t.read("sys/devices/system/cpu/bus_dcvs/DDR/boost_freq"),
        "300"
    );
}

#[test]
fn cpuset_intersects_online_cores() {
    let t = Tree::new();
    t.cpuset();
    t.put("sys/devices/system/cpu/cpu5/online", "0");
    let m = t.module();
    // request 0,1,4,5; cpu5 is offline, so only 0,1,4 are written
    let r = m
        .apply(&req(json!({"cpu.cpuset.top-app": [1,1,0,0,1,1,0,0]})))
        .unwrap();
    assert_eq!(t.read("dev/cpuset/top-app/cpus"), "0,1,4");
    assert_eq!(r.mismatch, vec!["cpu.cpuset.top-app".to_string()]);
    // Keep the user's requested CPU5 selection persisted even though CPU5 is offline now.
    assert!(m.persist_on_mismatch("cpu.cpuset.top-app"));
    let requested = json!([1, 1, 0, 0, 1, 1, 0, 0]);
    let actual = m.read_cpuset_item("cpu.cpuset.top-app").unwrap();
    assert!(m.cpuset_effectively_satisfied(&requested, Some(&actual)));
}

#[test]
fn screen_off_saved_online_readback_supports_legacy_state() {
    let t = Tree::new();
    t.put("sys/devices/system/cpu/cpu6/online", "0");
    let m = t.stateful();
    let mut st = m.state.load();
    st.screen_active = true;
    st.screen_saved = Some(state::ScreenSaved {
        // Older SysFlow versions only kept the cores that the screen profile switched off.
        cores_off: vec![6],
        ..Default::default()
    });
    m.state.save(&st).unwrap();

    let values = m
        .collect(Some(&[ITEM_SCREEN_SAVED_ONLINE.to_string()]))
        .unwrap();
    assert_eq!(values[ITEM_SCREEN_SAVED_ONLINE][6], json!(1));
    assert_eq!(values[ITEM_SCREEN_SAVED_ONLINE][7], json!(1));
}

#[test]
fn screen_off_frequency_table_exposes_only_native_available_steps() {
    let t = Tree::new();
    let path = t
        .root
        .join("sys/devices/system/cpu/cpufreq/policy4/scaling_available_frequencies");
    fs::remove_file(path).unwrap();
    let m = t.module();
    let values = m
        .collect(Some(&[
            ITEM_SCREEN_FREQ_TABLE.to_string(),
            ITEM_FREQ_TABLE.to_string(),
        ]))
        .unwrap();
    assert_eq!(values[ITEM_SCREEN_FREQ_TABLE]["4"], json!([]));
    // The normal tuning table still contains hardware-boundary fallback values.
    assert_eq!(values[ITEM_FREQ_TABLE]["4"], json!([500000, 2400000]));
}

#[test]
fn screen_off_accepts_generated_cap_within_policy_hardware_range() {
    let t = Tree::new();
    let m = t.stateful();
    // 1.7 GHz is within policy4's 500 MHz–2.4 GHz hardware range, even though it is not
    // one of the discrete frequency-table entries. The legacy UI generates 1 MHz detents
    // when a frequency table is unavailable or incomplete.
    m.apply(&req(json!({
        "cpu.screen_off": true,
        "cpu.screen_off.max_freq": {"4": 1700000}
    })))
    .unwrap();
    assert_eq!(m.state.load().screen.max_freq.get("4"), Some(&1700000));
    m.apply(&req(json!({"cpu.screen_off_active": true})))
        .unwrap();
    // The module snaps the generated value down to the highest supported kernel frequency.
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/scaling_max_freq"),
        "1500000"
    );

    // Turning the master switch off while the profile is active must restore the captured range.
    m.apply(&req(json!({"cpu.screen_off": false}))).unwrap();
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/scaling_max_freq"),
        "2400000"
    );
    let status = m.collect(Some(&[ITEM_SCREEN_ACTIVE.to_string()])).unwrap();
    assert_eq!(status[ITEM_SCREEN_ACTIVE], json!(false));

    let rejected = m.apply(&req(json!({"cpu.screen_off.max_freq": {"4": 2500000}})));
    assert!(
        rejected.is_err(),
        "a cap above the policy hardware maximum must be rejected"
    );
}

#[test]
fn screen_off_caps_and_restores() {
    let t = Tree::new();
    let m = t.stateful();
    let enabled = req(json!({
        "cpu.screen_off": true,
        "cpu.screen_off.max_freq": {"4": 1500000},
        "cpu.screen_off.cores": [0,0,0,0,0,0,1,1]
    }));
    m.apply(&enabled).unwrap();
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/scaling_max_freq"),
        "2400000"
    );

    m.apply(&req(json!({"cpu.screen_off_active": true})))
        .unwrap();
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/scaling_max_freq"),
        "1500000"
    );
    assert_eq!(t.read("sys/devices/system/cpu/cpu6/online"), "0");
    assert_eq!(t.read("sys/devices/system/cpu/cpu7/online"), "0");
    let saved_online = m
        .collect(Some(&[ITEM_SCREEN_SAVED_ONLINE.to_string()]))
        .unwrap();
    assert_eq!(saved_online[ITEM_SCREEN_SAVED_ONLINE][6], json!(1));
    assert_eq!(saved_online[ITEM_SCREEN_SAVED_ONLINE][7], json!(1));

    let v = m
        .collect(Some(&["cpu.screen_off_active".to_string()]))
        .unwrap();
    assert_eq!(v["cpu.screen_off_active"], json!(true));

    m.apply(&req(json!({"cpu.screen_off_active": false})))
        .unwrap();
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/scaling_max_freq"),
        "2400000"
    );
    assert_eq!(t.read("sys/devices/system/cpu/cpu6/online"), "1");
    assert_eq!(t.read("sys/devices/system/cpu/cpu7/online"), "1");
}

#[test]
fn screen_off_core_selection_can_change_while_active() {
    let t = Tree::new();
    let m = t.stateful();
    m.apply(&req(json!({
        "cpu.screen_off": true,
        "cpu.screen_off.cores": [0,0,0,0,0,0,1,0]
    })))
    .unwrap();
    m.apply(&req(json!({"cpu.screen_off_active": true})))
        .unwrap();
    assert_eq!(t.read("sys/devices/system/cpu/cpu6/online"), "0");

    // Move the screen-off selection from CPU6 to CPU7 without waking the device.
    m.apply(&req(json!({"cpu.screen_off.cores": [0,0,0,0,0,0,0,1]})))
        .unwrap();
    assert_eq!(t.read("sys/devices/system/cpu/cpu6/online"), "1");
    assert_eq!(t.read("sys/devices/system/cpu/cpu7/online"), "0");

    m.apply(&req(json!({"cpu.screen_off_active": false})))
        .unwrap();
    assert_eq!(t.read("sys/devices/system/cpu/cpu6/online"), "1");
    assert_eq!(t.read("sys/devices/system/cpu/cpu7/online"), "1");
}

#[test]
fn screen_off_added_core_keeps_its_original_offline_state() {
    let t = Tree::new();
    t.put("sys/devices/system/cpu/cpu7/online", "0");
    let m = t.stateful();
    m.apply(&req(json!({
        "cpu.screen_off": true,
        "cpu.screen_off.cores": [0,0,0,0,0,0,1,0]
    })))
    .unwrap();
    m.apply(&req(json!({"cpu.screen_off_active": true})))
        .unwrap();

    // CPU7 is selected after the screen-off profile has already started. Its original offline
    // state was captured at entry and must not be replaced with the transient selection state.
    m.apply(&req(json!({"cpu.screen_off.cores": [0,0,0,0,0,0,1,1]})))
        .unwrap();
    m.apply(&req(json!({"cpu.screen_off_active": false})))
        .unwrap();

    assert_eq!(t.read("sys/devices/system/cpu/cpu6/online"), "1");
    assert_eq!(t.read("sys/devices/system/cpu/cpu7/online"), "0");
}

#[test]
fn screen_off_restores_core_that_was_already_offline() {
    let t = Tree::new();
    t.put("sys/devices/system/cpu/cpu7/online", "0");
    let m = t.stateful();
    m.apply(&req(json!({
        "cpu.screen_off": true,
        "cpu.screen_off.cores": [0,0,0,0,0,0,0,1]
    })))
    .unwrap();
    m.apply(&req(json!({"cpu.screen_off_active": true})))
        .unwrap();
    m.apply(&req(json!({"cpu.screen_off_active": false})))
        .unwrap();
    assert_eq!(t.read("sys/devices/system/cpu/cpu7/online"), "0");
}

#[test]
fn adaptive_cap_uses_load_tier() {
    let t = Tree::new();
    let m = t.stateful();
    let settings = req(json!({
        "cpu.adaptive": true,
        "cpu.adaptive.smart": true,
        "cpu.adaptive.load_t1": 12,
        "cpu.adaptive.load_t2": 35,
        "cpu.adaptive.load_t3": 70,
        "cpu.adaptive.load_cap1": 45,
        "cpu.adaptive.load_cap2": 65,
        "cpu.adaptive.load_cap3": 85,
        "cpu.adaptive.load_cap4": 100,
        "cpu.adaptive.load_down_rounds": 1,
        "cpu.max_freq": {"4": 2400000}
    }));
    m.apply_with_settings(&settings, &settings).unwrap();
    // establish a baseline at 50% busy
    let mut busy = [100u64; 8];
    for b in busy.iter_mut().skip(4) {
        *b = 150;
    }
    t.proc_stat(&busy, 150);
    let r = m.apply_with_settings(&settings, &settings).unwrap();
    // The static ceiling was accepted even though the dynamic cap wrote a lower value.
    assert!(r.mismatch.is_empty(), "{:?}", r.mismatch);
    assert_eq!(r.applied["cpu.max_freq"]["4"], json!(1500000));
    let v = m
        .collect(Some(&["cpu.adaptive_state".to_string()]))
        .unwrap();
    assert_eq!(v["cpu.adaptive_state"]["cpu_adj"], json!(100));
    assert_eq!(v["cpu.adaptive_state"]["policies"]["4"]["tier"], json!(2));
    // tier 2 = 85% of 2.4 GHz, snapped below the static ceiling: 1.5 GHz
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/scaling_max_freq"),
        "1500000"
    );
}

#[test]
fn drift_is_read_only_and_detects_rewritten_governor() {
    let t = Tree::new();
    let m = t.module();
    let settings = req(json!({"cpu.governor": {"0": "performance"}}));
    assert_eq!(
        m.drift(&settings).unwrap(),
        vec!["cpu.governor".to_string()]
    );
    m.apply(&settings).unwrap();
    assert!(m.drift(&settings).unwrap().is_empty());
    t.put(
        "sys/devices/system/cpu/cpufreq/policy0/scaling_governor",
        "schedutil",
    );
    assert_eq!(
        m.drift(&settings).unwrap(),
        vec!["cpu.governor".to_string()]
    );
}

#[test]
fn text_walt_value_is_validated() {
    let t = Tree::new();
    t.walt(4, "target_loads", "85 1500000:90");
    let m = t.module();
    let v = m
        .collect(Some(&["cpu.walt.target_loads".to_string()]))
        .unwrap();
    assert_eq!(v["cpu.walt.target_loads"], json!({"4": "85 1500000:90"}));
    assert!(m
        .apply(&req(json!({"cpu.walt.target_loads": {"4": "85; reboot"}})))
        .is_err());
}

#[test]
fn walt_adaptive_explicit_value_applies_when_uncapped() {
    let t = Tree::new();
    t.walt(4, "adaptive_high_freq", "2400000");
    let m = t.stateful();
    m.apply(&req(json!({"cpu.walt.adaptive_high_freq": {"4": 1800000}})))
        .unwrap();
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/walt/adaptive_high_freq"),
        "1800000"
    );
}

#[test]
fn drift_allows_dynamic_cap_lower_than_static_max() {
    let t = Tree::new();
    let m = t.stateful();
    let settings = req(json!({
        "cpu.adaptive": true,
        "cpu.adaptive.load_t1": 12,
        "cpu.adaptive.load_t2": 35,
        "cpu.adaptive.load_t3": 70,
        "cpu.adaptive.load_cap1": 45,
        "cpu.adaptive.load_cap2": 65,
        "cpu.adaptive.load_cap3": 85,
        "cpu.adaptive.load_cap4": 100,
        "cpu.adaptive.load_down_rounds": 1,
        "cpu.max_freq": {"4": 2400000}
    }));
    m.apply_with_settings(&settings, &settings).unwrap();
    let mut busy = [100u64; 8];
    for b in busy.iter_mut().skip(4) {
        *b = 150;
    }
    t.proc_stat(&busy, 150);
    m.apply_with_settings(&settings, &settings).unwrap();
    assert_eq!(
        t.read("sys/devices/system/cpu/cpufreq/policy4/scaling_max_freq"),
        "1500000"
    );
    // the persisted static ceiling is still 2.4 GHz; the adaptive cap is not drift
    assert!(m.drift(&settings).unwrap().is_empty());
}

#[test]
fn a_kernel_min_freq_floor_is_not_reported_as_drift() {
    let t = Tree::new();
    let state = t.stateful();

    // Adaptive mode is what switches `min_freq`/`max_freq` to the direction-aware comparison, so it
    // has to be live in the module state, not merely present in the settings map.
    state
        .apply(&req(json!({"cpu.adaptive": true})))
        .expect("enable adaptive");

    // The kernel enforces a floor above what was persisted; writing 500 kHz cannot lower it.
    t.put(
        "sys/devices/system/cpu/cpufreq/policy4/scaling_min_freq",
        "1000000",
    );
    let floored = req(json!({"cpu.adaptive": true, "cpu.min_freq": {"4": 500000}}));
    let drift = state.drift(&floored).unwrap();
    assert!(
        !drift.contains(&"cpu.min_freq".to_string()),
        "min_freq reported drift despite a higher kernel floor: {drift:?}"
    );

    // The ceiling keeps the opposite direction: a live cap above the request is still drift.
    t.put(
        "sys/devices/system/cpu/cpufreq/policy4/scaling_max_freq",
        "2400000",
    );
    let capped = req(json!({"cpu.adaptive": true, "cpu.max_freq": {"4": 1200000}}));
    let drift = state.drift(&capped).unwrap();
    assert!(
        drift.contains(&"cpu.max_freq".to_string()),
        "an exceeded ceiling must stay drift: {drift:?}"
    );
}
