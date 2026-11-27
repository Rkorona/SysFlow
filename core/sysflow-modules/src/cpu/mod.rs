//! CPU module: core on/off, per-cluster frequency limits and governor, WALT parameters, GPU
//! devfreq, DDR/LLCC boost, cpuset groups, screen-off profile and the adaptive cap controller.
//!
//! All paths are probed, nothing is assumed (ADR 0003 / 0004 / 0005). The module is stateless
//! apart from a small state file (`<data_dir>/state/cpu.json`) used for transient data that is not
//! a sysfs value: WALT `adaptive_*` originals, the pre-screen-off range, and adaptive hysteresis.

mod adaptive;
mod cpuset;
mod gpu;
mod mem;
mod state;
mod walt;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::event::{EventStore, NewSystemEvent};
use sysflow_core::module::{ApplyResult, Caps, Module};
use sysflow_core::schema::{ItemType, Risk, Schema, SchemaItem, Scope};
use sysflow_core::sysfs::Sysfs;

use state::{CpuState, StateStore};

const CPU_ROOT: &str = "/sys/devices/system/cpu";
const CPUFREQ_ROOT: &str = "/sys/devices/system/cpu/cpufreq";
const THERMAL_ROOT: &str = "/sys/class/thermal";

pub(crate) const ITEM_ONLINE: &str = "cpu.online";
pub(crate) const ITEM_MAX: &str = "cpu.max_freq";
pub(crate) const ITEM_MIN: &str = "cpu.min_freq";
pub(crate) const ITEM_GOV: &str = "cpu.governor";
pub(crate) const ITEM_CUR: &str = "cpu.cur_freq";
pub(crate) const ITEM_TEMP: &str = "cpu.temp";
pub(crate) const ITEM_CLUSTERS: &str = "cpu.clusters";
const ITEM_CLUSTER_CUR: &str = "cpu.cluster_cur_freq";
const ITEM_CLUSTER_HW_CUR: &str = "cpu.cluster_hw_cur_freq";
const ITEM_HW_MIN: &str = "cpu.hw_min_freq";
const ITEM_HW_MAX: &str = "cpu.hw_max_freq";
const ITEM_TIME_IN_STATE: &str = "cpu.time_in_state";
const ITEM_FREQ_TABLE: &str = "cpu.freq_table";
const ITEM_SCREEN_FREQ_TABLE: &str = "cpu.screen_off.freq_table";
const ITEM_GOVERNOR_TABLE: &str = "cpu.governor_table";
const ITEM_STAT: &str = "cpu.stat";
const ITEM_USAGE_STAT: &str = "cpu.usage_stat";

const ITEM_SCREEN_ENABLED: &str = "cpu.screen_off";
const ITEM_SCREEN_MAX: &str = "cpu.screen_off.max_freq";
const ITEM_SCREEN_CORES: &str = "cpu.screen_off.cores";
const ITEM_SCREEN_ACTIVE: &str = "cpu.screen_off_active";
const ITEM_SCREEN_SAVED_ONLINE: &str = "cpu.screen_off_saved_online";

const ITEM_ADAPTIVE_STATE: &str = "cpu.adaptive_state";
const ITEM_GPU_BUSY: &str = "cpu.gpu_busy";
const ITEM_BATTERY_TEMP: &str = "cpu.battery_temp";
const ITEM_THERMAL_SENSORS: &str = "cpu.thermal_sensors";
const ITEM_THERMAL_SENSORS_ALL: &str = "cpu.thermal_sensors_all";
const ITEM_COOLING_DEVICES: &str = "cpu.cooling_devices";
const ITEM_GPU_INFO: &str = "cpu.gpu_info";

/// One cpufreq policy (a cluster of cores that share frequency settings).
#[derive(Clone, Debug)]
pub(crate) struct Cluster {
    pub(crate) id: u32,
    pub(crate) cpus: Vec<usize>,
    /// Online CPUs used for utilization sampling (`affected_cpus`), falling back to `related_cpus`.
    pub(crate) sample_cpus: Vec<usize>,
    pub(crate) freqs: Vec<i64>,
    /// Raw scaling_available_frequencies for the legacy-compatible screen-off slider.
    /// Unlike `freqs`, this excludes synthetic time_in_state and hardware boundary entries.
    pub(crate) screen_freqs: Vec<i64>,
    pub(crate) governors: Vec<String>,
    pub(crate) hw_min: Option<i64>,
    pub(crate) hw_max: Option<i64>,
    pub(crate) cur_min: i64,
    pub(crate) cur_max: i64,
    pub(crate) cur_gov: String,
}

/// Validated per-cluster changes for one `apply`.
#[derive(Debug)]
struct ClusterPlan {
    id: u32,
    gov: Option<String>,
    min: Option<i64>,
    max: Option<i64>,
    cur_min: i64,
    cur_max: i64,
    cur_gov: String,
}

pub struct CpuModule {
    fs: Sysfs,
    state: StateStore,
    events: Option<EventStore>,
}

impl CpuModule {
    /// Module without a state directory: only sysfs-backed behaviour. Used by unit tests that do
    /// not need screen-off / adaptive persistence.
    pub fn new(fs: Sysfs) -> Self {
        CpuModule {
            fs,
            state: StateStore::none(),
            events: None,
        }
    }

    /// Production constructor: state is kept under `<data_dir>/state/cpu.json`.
    pub fn with_state_dir(fs: Sysfs, state_dir: impl Into<std::path::PathBuf>) -> Self {
        let state_dir = state_dir.into();
        let events = state_dir.parent().map(EventStore::new);
        CpuModule {
            fs,
            state: StateStore::new(state_dir),
            events,
        }
    }

    /// Runtime policy transitions are emitted by the Root module, never inferred by the APK.
    pub(crate) fn emit_runtime_event(&self, event: NewSystemEvent) {
        let Some(events) = &self.events else {
            return;
        };
        if let Err(error) = events.emit_sanitized(event) {
            // Event logging is best-effort; it must never interrupt thermal or battery protection.
            eprintln!("SysFlow CPU event emission failed: {error}");
        }
    }

    // ---- discovery ----------------------------------------------------------

    /// Number of core slots (highest `cpuN` index + 1); 0 if the tree is missing.
    fn cpu_count(&self) -> usize {
        let names = match self.fs.list_dir(CPU_ROOT) {
            Ok(n) => n,
            Err(_) => return 0,
        };
        names
            .iter()
            .filter_map(|n| n.strip_prefix("cpu").and_then(|d| d.parse::<usize>().ok()))
            .max()
            .map(|m| m + 1)
            .unwrap_or(0)
    }

    fn cpu_dir(cpu: usize) -> String {
        format!("{CPU_ROOT}/cpu{cpu}")
    }

    fn online_path(cpu: usize) -> String {
        format!("{CPU_ROOT}/cpu{cpu}/online")
    }

    fn policy_path(id: u32, file: &str) -> String {
        format!("{CPUFREQ_ROOT}/policy{id}/{file}")
    }

    /// Cores without an `online` node (usually cpu0) cannot be switched.
    fn can_toggle(&self, cpu: usize) -> bool {
        self.fs.exists(&Self::online_path(cpu))
    }

    fn read_online(&self, cpu: usize) -> i64 {
        match self.fs.read_i64(&Self::online_path(cpu)) {
            Ok(v) => {
                if v != 0 {
                    1
                } else {
                    0
                }
            }
            // no node: the core exists and cannot be turned off, or it does not exist
            Err(_) => {
                if self.fs.exists(&Self::cpu_dir(cpu)) {
                    1
                } else {
                    0
                }
            }
        }
    }

    fn parse_ints(s: &str) -> Vec<i64> {
        s.split_whitespace()
            .filter_map(|t| t.parse::<i64>().ok())
            .collect()
    }

    fn clusters(&self) -> Vec<Cluster> {
        let names = match self.fs.list_dir(CPUFREQ_ROOT) {
            Ok(n) => n,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        for name in names {
            let id = match name
                .strip_prefix("policy")
                .and_then(|d| d.parse::<u32>().ok())
            {
                Some(id) => id,
                None => continue,
            };
            // `related_cpus` describes the policy's full topology and is used by controls/UI.
            // `/proc/stat` sampling should prefer `affected_cpus`, which tracks the online CPUs
            // currently participating in that policy; this matches SoCFlow's adaptive sampler.
            let related = self.fs.read_str(&Self::policy_path(id, "related_cpus"));
            let affected = self.fs.read_str(&Self::policy_path(id, "affected_cpus"));
            let mut cpus: Vec<usize> = match related.as_ref().or(affected.as_ref()) {
                Ok(t) => Self::parse_ints(t)
                    .into_iter()
                    .filter(|c| *c >= 0)
                    .map(|c| c as usize)
                    .collect(),
                Err(_) => continue,
            };
            cpus.sort_unstable();
            cpus.dedup();
            if cpus.is_empty() {
                continue;
            }
            let mut sample_cpus: Vec<usize> = affected
                .as_ref()
                .map(|t| Self::parse_ints(t))
                .unwrap_or_default()
                .into_iter()
                .filter(|c| *c >= 0)
                .map(|c| c as usize)
                .collect();
            if sample_cpus.is_empty() {
                sample_cpus = cpus.clone();
            }
            sample_cpus.sort_unstable();
            sample_cpus.dedup();
            let mut screen_freqs = self
                .fs
                .read_str(&Self::policy_path(id, "scaling_available_frequencies"))
                .map(|t| Self::parse_ints(&t))
                .unwrap_or_default();
            screen_freqs.retain(|f| *f > 0);
            screen_freqs.sort_unstable();
            screen_freqs.dedup();
            let mut freqs = screen_freqs.clone();
            if freqs.is_empty() {
                // time_in_state: "<freq> <ticks>" per line
                if let Ok(t) = self
                    .fs
                    .read_str(&Self::policy_path(id, "stats/time_in_state"))
                {
                    freqs = t
                        .lines()
                        .filter_map(|l| l.split_whitespace().next())
                        .filter_map(|f| f.parse::<i64>().ok())
                        .collect();
                }
            }
            let hw_min = self
                .fs
                .read_i64(&Self::policy_path(id, "cpuinfo_min_freq"))
                .ok();
            let hw_max = self
                .fs
                .read_i64(&Self::policy_path(id, "cpuinfo_max_freq"))
                .ok();
            // The hardware min/max are always selectable, even when the advertised table omits
            // them: "reset" and "no limit" must be expressible on every kernel.
            for f in [hw_min, hw_max].into_iter().flatten() {
                freqs.push(f);
            }
            freqs.sort_unstable();
            freqs.dedup();
            let governors = self
                .fs
                .read_str(&Self::policy_path(id, "scaling_available_governors"))
                .map(|t| t.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default();
            let cur_min = self
                .fs
                .read_i64(&Self::policy_path(id, "scaling_min_freq"))
                .unwrap_or(0);
            let cur_max = self
                .fs
                .read_i64(&Self::policy_path(id, "scaling_max_freq"))
                .unwrap_or(0);
            let cur_gov = self
                .fs
                .read_str(&Self::policy_path(id, "scaling_governor"))
                .unwrap_or_default();
            out.push(Cluster {
                id,
                cpus,
                sample_cpus,
                freqs,
                screen_freqs,
                governors,
                hw_min,
                hw_max,
                cur_min,
                cur_max,
                cur_gov,
            });
        }
        out.sort_by_key(|c| c.id);
        out
    }

    // ---- reading ------------------------------------------------------------

    fn read_online_all(&self) -> Value {
        let n = self.cpu_count();
        Value::Array((0..n).map(|c| json!(self.read_online(c))).collect())
    }

    fn read_per_cluster(&self, file: &str, clusters: &[Cluster], as_int: bool) -> Value {
        let mut m = Map::new();
        for c in clusters {
            let p = Self::policy_path(c.id, file);
            let v = if as_int {
                self.fs.read_i64(&p).ok().map(|v| json!(v))
            } else {
                self.fs.read_str(&p).ok().map(|v| json!(v))
            };
            if let Some(v) = v {
                m.insert(c.id.to_string(), v);
            }
        }
        Value::Object(m)
    }

    fn read_cur_freq(&self) -> Value {
        let mut m = Map::new();
        for cpu in 0..self.cpu_count() {
            if self.read_online(cpu) == 0 {
                continue;
            }
            // some kernels only expose the hardware-reported value
            let dir = format!("{CPU_ROOT}/cpu{cpu}/cpufreq");
            let v = self
                .fs
                .read_i64(&format!("{dir}/scaling_cur_freq"))
                .or_else(|_| self.fs.read_i64(&format!("{dir}/cpuinfo_cur_freq")));
            if let Ok(v) = v {
                m.insert(cpu.to_string(), json!(v));
            }
        }
        Value::Object(m)
    }

    fn read_cluster_cur_freq(&self, clusters: &[Cluster]) -> Value {
        let mut m = Map::new();
        for c in clusters {
            let v = self
                .fs
                .read_i64(&Self::policy_path(c.id, "scaling_cur_freq"))
                .or_else(|_| {
                    self.fs
                        .read_i64(&Self::policy_path(c.id, "cpuinfo_cur_freq"))
                });
            if let Ok(v) = v {
                m.insert(c.id.to_string(), json!(v));
            }
        }
        Value::Object(m)
    }

    /// Hardware-reported current frequency per policy. Kept separate from scaling_cur_freq so
    /// clients can use the legacy fallback order: time-in-state average, cpuinfo_cur_freq, then
    /// scaling_cur_freq.
    fn read_cluster_hw_cur_freq(&self, clusters: &[Cluster]) -> Value {
        let mut m = Map::new();
        for c in clusters {
            if let Ok(v) = self
                .fs
                .read_i64(&Self::policy_path(c.id, "cpuinfo_cur_freq"))
            {
                m.insert(c.id.to_string(), json!(v));
            }
        }
        Value::Object(m)
    }

    /// Hottest CPU-related thermal zone in 0.1 °C, or null.
    fn read_temp(&self) -> Value {
        let zones = match self.fs.list_dir(THERMAL_ROOT) {
            Ok(z) => z,
            Err(_) => return Value::Null,
        };
        let mut best: Option<i64> = None;
        for z in zones.iter().filter(|z| z.starts_with("thermal_zone")) {
            let ty = match self.fs.read_str(&format!("{THERMAL_ROOT}/{z}/type")) {
                Ok(t) => t.to_ascii_lowercase(),
                Err(_) => continue,
            };
            // vendor names vary: cpu-0-0-usr, cpuss-1, mtktscpu, soc-cpu_thermal ...
            if !ty.contains("cpu") || ty.contains("trip") {
                continue;
            }
            if let Ok(raw) = self.fs.read_i64(&format!("{THERMAL_ROOT}/{z}/temp")) {
                // kernels report millidegrees; a few report whole degrees
                let deci = if raw.abs() >= 1000 {
                    raw / 100
                } else {
                    raw * 10
                };
                best = Some(best.map_or(deci, |b| b.max(deci)));
            }
        }
        best.map(|v| json!(v)).unwrap_or(Value::Null)
    }

    /// GPU busy percentage (KGSL on Qualcomm; absent on devices without a readable load node).
    fn read_gpu_busy(&self) -> Value {
        const CANDIDATES: &[&str] = &[
            "/sys/class/kgsl/kgsl-3d0/gpu_busy_percentage",
            "/sys/class/kgsl/kgsl-3d0/gpu_busy",
            "/sys/kernel/gpu/gpu_busy_percentage",
        ];
        for path in CANDIDATES {
            let Ok(raw) = self.fs.read_str(path) else {
                continue;
            };
            let text = raw.trim().trim_end_matches('%').trim();
            if let Ok(value) = text.parse::<i64>() {
                if (0..=100).contains(&value) {
                    return json!(value);
                }
            }
        }
        Value::Null
    }

    /// Battery temperature in 0.1 °C. Android power_supply battery/temp conventionally reports
    /// tenths of a degree (e.g. 350 means 35.0 °C); implausible readings are omitted.
    fn read_battery_temp(&self) -> Value {
        let Ok(raw) = self.fs.read_i64("/sys/class/power_supply/battery/temp") else {
            return Value::Null;
        };
        if !(0..=1000).contains(&raw) {
            return Value::Null;
        }
        json!(raw)
    }

    /// CPU/GPU/SoC zones used by adaptive thermal control, each expressed in 0.1 °C.
    fn read_thermal_sensors(&self, all: bool) -> Value {
        let zones = self.fs.list_dir(THERMAL_ROOT).unwrap_or_default();
        let mut sensors = Vec::new();
        for zone in zones.iter().filter(|z| z.starts_with("thermal_zone")) {
            let Ok(kind) = self.fs.read_str(&format!("{THERMAL_ROOT}/{zone}/type")) else {
                continue;
            };
            let lower = kind.to_ascii_lowercase();
            let participates = ["cpu", "gpu", "soc"]
                .iter()
                .any(|part| lower.contains(part));
            if (!all && !participates) || lower.contains("trip") {
                continue;
            }
            let Ok(raw) = self.fs.read_i64(&format!("{THERMAL_ROOT}/{zone}/temp")) else {
                continue;
            };
            let deci = if raw.abs() >= 1000 {
                raw / 100
            } else {
                raw.saturating_mul(10)
            };
            if !(0..=1500).contains(&deci) {
                continue;
            }
            sensors.push(json!({
                "zone": zone,
                "sensor": kind,
                "temp_deci_c": deci,
                "participates": participates,
            }));
        }
        Value::Array(sensors)
    }

    /// Enumerate cooling devices for thermal diagnostics without modifying their state.
    fn read_cooling_devices(&self) -> Value {
        let root = THERMAL_ROOT.to_string();
        let entries = self.fs.list_dir(&root).unwrap_or_default();
        let mut devices = Vec::new();
        for entry in entries
            .iter()
            .filter(|name| name.starts_with("cooling_device"))
        {
            let path = format!("{root}/{entry}");
            let kind = self
                .fs
                .read_str(&format!("{path}/type"))
                .unwrap_or_default();
            let current = self.fs.read_i64(&format!("{path}/cur_state")).ok();
            let maximum = self.fs.read_i64(&format!("{path}/max_state")).ok();
            devices.push(json!({
                "device": entry,
                "type": kind,
                "current_state": current,
                "max_state": maximum,
            }));
        }
        Value::Array(devices)
    }

    fn read_time_in_state(&self, clusters: &[Cluster]) -> Value {
        let mut out = Map::new();
        for c in clusters {
            let Ok(text) = self
                .fs
                .read_str(&Self::policy_path(c.id, "stats/time_in_state"))
            else {
                continue;
            };
            let mut per_freq = Map::new();
            for line in text.lines() {
                let mut it = line.split_whitespace();
                let (Some(f), Some(t)) = (it.next(), it.next()) else {
                    continue;
                };
                if let (Ok(f), Ok(t)) = (f.parse::<i64>(), t.parse::<i64>()) {
                    per_freq.insert(f.to_string(), json!(t));
                }
            }
            out.insert(c.id.to_string(), Value::Object(per_freq));
        }
        Value::Object(out)
    }

    fn read_stat(&self, clusters: &[Cluster]) -> Value {
        let Some(stat) = adaptive::read_proc_stat(&self.fs) else {
            return Value::Object(Map::new());
        };
        let mut out = Map::new();
        for c in clusters {
            let mut busy = 0u64;
            let mut total = 0u64;
            for cpu in &c.cpus {
                if let Some((b, t)) = stat.get(cpu) {
                    busy = busy.saturating_add(*b);
                    total = total.saturating_add(*t);
                }
            }
            out.insert(c.id.to_string(), json!({"busy": busy, "total": total}));
        }
        Value::Object(out)
    }

    /// Raw per-core counters for the UI's two-sample load calculation. The aggregate `cpu`
    /// entry is derived by summing the individual CPU counters.
    fn read_usage_stat(&self) -> Value {
        let Some(stat) = adaptive::read_proc_stat(&self.fs) else {
            return Value::Object(Map::new());
        };
        let mut out = Map::new();
        let mut total_busy = 0u64;
        let mut total_ticks = 0u64;
        for (cpu, (busy, total)) in stat {
            total_busy = total_busy.saturating_add(busy);
            total_ticks = total_ticks.saturating_add(total);
            out.insert(format!("cpu{cpu}"), json!({ "busy": busy, "total": total }));
        }
        out.insert(
            "cpu".to_string(),
            json!({ "busy": total_busy, "total": total_ticks }),
        );
        Value::Object(out)
    }

    fn read_item(&self, key: &str, clusters: &[Cluster], st: &CpuState) -> Option<Value> {
        if key == walt::item_walt_guard() {
            return self.read_walt_item(key, clusters);
        }
        if walt::is_walt_item(key) {
            return self.read_walt_item(key, clusters);
        }
        if key.starts_with("cpu.cpuset.") {
            return self.read_cpuset_item(key);
        }
        if key.starts_with("cpu.gpu_") {
            if let Some(v) = self.read_gpu_item(key) {
                return Some(v);
            }
        }
        if key == "cpu.ddr_boost_freq" || key == "cpu.llcc_boost_freq" {
            return self.read_mem_item(key);
        }
        if key.starts_with("cpu.adaptive") {
            if key == ITEM_ADAPTIVE_STATE {
                return Some(st.adaptive_runtime.to_json(st.adaptive.enabled));
            }
            return st.adaptive.to_json().get(key).cloned();
        }
        if key == ITEM_SCREEN_ENABLED || key == ITEM_SCREEN_MAX || key == ITEM_SCREEN_CORES {
            if key == ITEM_SCREEN_CORES {
                let n = self.cpu_count();
                let mut arr = vec![json!(0); n];
                for c in &st.screen.cores {
                    if let Some(slot) = arr.get_mut(*c) {
                        *slot = json!(1);
                    }
                }
                return Some(Value::Array(arr));
            }
            return st.screen.to_json().get(key).cloned();
        }
        if key == ITEM_SCREEN_ACTIVE {
            return Some(json!(st.screen_active));
        }
        if key == ITEM_SCREEN_SAVED_ONLINE {
            let saved = st.screen_saved.as_ref()?;
            let mut online = self.read_online_all().as_array()?.clone();
            // Older state versions only stored the list of cores that the profile actually
            // switched off. Reconstruct those as previously-online before applying the newer,
            // more complete baseline map (when present).
            for cpu in &saved.cores_off {
                if let Some(slot) = online.get_mut(*cpu) {
                    *slot = json!(1);
                }
            }
            for (cpu, was_online) in &saved.core_online_before {
                if let Some(slot) = online.get_mut(*cpu) {
                    *slot = json!(if *was_online { 1 } else { 0 });
                }
            }
            return Some(Value::Array(online));
        }
        Some(match key {
            ITEM_ONLINE => self.read_online_all(),
            ITEM_MAX => self.read_per_cluster("scaling_max_freq", clusters, true),
            ITEM_MIN => self.read_per_cluster("scaling_min_freq", clusters, true),
            ITEM_GOV => self.read_per_cluster("scaling_governor", clusters, false),
            ITEM_CUR => self.read_cur_freq(),
            ITEM_CLUSTER_CUR => self.read_cluster_cur_freq(clusters),
            ITEM_CLUSTER_HW_CUR => self.read_cluster_hw_cur_freq(clusters),
            ITEM_HW_MIN => self.read_per_cluster("cpuinfo_min_freq", clusters, true),
            ITEM_HW_MAX => self.read_per_cluster("cpuinfo_max_freq", clusters, true),
            ITEM_TEMP => self.read_temp(),
            ITEM_GPU_BUSY => self.read_gpu_busy(),
            ITEM_BATTERY_TEMP => self.read_battery_temp(),
            ITEM_THERMAL_SENSORS => self.read_thermal_sensors(false),
            ITEM_THERMAL_SENSORS_ALL => self.read_thermal_sensors(true),
            ITEM_COOLING_DEVICES => self.read_cooling_devices(),
            ITEM_GPU_INFO => self.read_gpu_info(),
            ITEM_TIME_IN_STATE => self.read_time_in_state(clusters),
            ITEM_FREQ_TABLE => {
                let mut m = Map::new();
                for c in clusters {
                    m.insert(c.id.to_string(), json!(c.freqs));
                }
                Value::Object(m)
            }
            ITEM_SCREEN_FREQ_TABLE => {
                let mut m = Map::new();
                for c in clusters {
                    m.insert(c.id.to_string(), json!(c.screen_freqs));
                }
                Value::Object(m)
            }
            ITEM_GOVERNOR_TABLE => {
                let mut m = Map::new();
                for c in clusters {
                    m.insert(c.id.to_string(), json!(c.governors));
                }
                Value::Object(m)
            }
            ITEM_STAT => self.read_stat(clusters),
            ITEM_USAGE_STAT => self.read_usage_stat(),
            ITEM_CLUSTERS => {
                let mut m = Map::new();
                for c in clusters {
                    m.insert(c.id.to_string(), json!(c.cpus));
                }
                Value::Object(m)
            }
            _ => return None,
        })
    }

    // ---- writing ------------------------------------------------------------

    fn parse_online_request(&self, v: &Value) -> Result<Vec<i64>> {
        let arr = v
            .as_array()
            .ok_or_else(|| Error::invalid_arg("cpu.online: expected an array"))?;
        let want: Vec<i64> = arr.iter().filter_map(|x| x.as_i64()).collect();
        let n = self.cpu_count();
        if want.len() != n || arr.len() != n {
            return Err(Error::invalid_arg(format!(
                "cpu.online: expected {n} entries"
            )));
        }
        if want.iter().all(|w| *w == 0) {
            return Err(Error::invalid_arg(
                "cpu.online: at least one core must stay online",
            ));
        }
        for (cpu, w) in want.iter().enumerate() {
            if *w == 0 && (cpu == 0 || !self.can_toggle(cpu)) {
                return Err(Error::invalid_arg(format!("cpu{cpu} cannot be turned off")));
            }
        }
        Ok(want)
    }

    /// Bring cores up first, then take cores down. Records old states for rollback.
    fn apply_online(&self, want: &[i64], changed: &mut Vec<(usize, i64)>) -> Result<()> {
        for pass_up in [true, false] {
            for (cpu, w) in want.iter().enumerate() {
                if (*w == 1) != pass_up || !self.can_toggle(cpu) {
                    continue;
                }
                let cur = self.read_online(cpu);
                if cur == *w {
                    continue;
                }
                self.fs
                    .write_str(&Self::online_path(cpu), if *w == 1 { "1" } else { "0" })?;
                changed.push((cpu, cur));
            }
        }
        Ok(())
    }

    fn rollback_online(&self, changed: &[(usize, i64)]) {
        for (cpu, old) in changed.iter().rev() {
            let _ = self
                .fs
                .write_str(&Self::online_path(*cpu), if *old == 1 { "1" } else { "0" });
        }
    }

    /// Clusters as they are now. After cores were switched, a policy directory can take a moment
    /// to appear, so wait briefly (at most 0.5 s) for the clusters the request names.
    fn clusters_settled(&self, wanted: &[u32], settle: bool) -> Vec<Cluster> {
        let mut clusters = self.clusters();
        if !settle {
            return clusters;
        }
        for _ in 0..5 {
            if wanted.iter().all(|id| clusters.iter().any(|c| c.id == *id)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            clusters = self.clusters();
        }
        clusters
    }

    fn requested_clusters(desired: &Map<String, Value>) -> Vec<u32> {
        let mut ids = BTreeSet::new();
        for item in [ITEM_GOV, ITEM_MIN, ITEM_MAX, ITEM_SCREEN_MAX] {
            if let Some(o) = desired.get(item).and_then(|v| v.as_object()) {
                ids.extend(o.keys().filter_map(|k| k.parse::<u32>().ok()));
            }
        }
        ids.into_iter().collect()
    }

    fn snap_cap(freqs: &[i64], target: i64, static_max: i64, force: bool) -> i64 {
        let mut snap = freqs
            .iter()
            .copied()
            .filter(|f| *f >= target)
            .min()
            .unwrap_or(static_max);
        snap = snap.min(static_max);
        if force && snap >= static_max {
            if let Some(below) = freqs.iter().copied().filter(|f| *f < static_max).max() {
                snap = below;
            }
        }
        snap
    }

    /// Turn the effective settings into per-cluster writes, applying the adaptive and screen-off
    /// ceilings on top of the user's static values.
    fn build_plan(
        &self,
        req: &Map<String, Value>,
        desired: &Map<String, Value>,
        clusters: &[Cluster],
        st: &CpuState,
        screen_active: bool,
    ) -> Result<(Vec<ClusterPlan>, BTreeMap<u32, i64>)> {
        for item in [ITEM_GOV, ITEM_MIN, ITEM_MAX, ITEM_SCREEN_MAX] {
            if let Some(obj) = req.get(item).and_then(Value::as_object) {
                for key in obj.keys() {
                    if let Ok(policy) = key.parse::<u32>() {
                        if !clusters.iter().any(|c| c.id == policy) {
                            return Err(Error::invalid_arg(format!(
                                "policy{policy} does not exist"
                            )));
                        }
                    }
                }
            }
        }
        let mut plans = Vec::new();
        let mut effective_max = BTreeMap::new();
        for c in clusters {
            let get = |item: &str| -> Option<Value> {
                desired
                    .get(item)
                    .and_then(|v| v.as_object())
                    .and_then(|o| o.get(&c.id.to_string()))
                    .cloned()
            };
            let want_gov = match get(ITEM_GOV) {
                Some(v) => {
                    let g = v.as_str().unwrap_or_default().to_string();
                    if !c.governors.is_empty() && !c.governors.contains(&g) {
                        return Err(Error::invalid_arg(format!(
                            "{ITEM_GOV}[{}]: {g:?} is not available",
                            c.id
                        )));
                    }
                    Some(g)
                }
                None => None,
            };
            let want_min = get(ITEM_MIN);
            let want_max = get(ITEM_MAX);
            let mut min = match want_min.as_ref().and_then(Value::as_i64) {
                Some(v) => {
                    if !c.freqs.contains(&v) {
                        return Err(Error::invalid_arg(format!(
                            "{ITEM_MIN}[{}]: {v} kHz is not in the frequency table",
                            c.id
                        )));
                    }
                    v
                }
                None => c.cur_min,
            };
            let mut max = match want_max.as_ref().and_then(Value::as_i64) {
                Some(v) => {
                    if !c.freqs.contains(&v) {
                        return Err(Error::invalid_arg(format!(
                            "{ITEM_MAX}[{}]: {v} kHz is not in the frequency table",
                            c.id
                        )));
                    }
                    v
                }
                None => c.cur_max,
            };

            if want_min.is_some() && want_max.is_some() && min > max {
                return Err(Error::invalid_arg(format!(
                    "policy{}: min {min} kHz is above max {max} kHz",
                    c.id
                )));
            }
            if st.adaptive.enabled && want_max.is_some() {
                let pct = self.adaptive_cpu_pct(&st.adaptive, &st.adaptive_runtime, c.id);
                if pct < 100 {
                    // The adaptive cap never goes below the user's floor; it only lowers the
                    // ceiling. Lowering the floor is the screen-off profile's job.
                    max = Self::snap_cap(&c.freqs, max * pct / 100, max, true).max(min);
                }
            }
            if screen_active {
                if let Some(cap) = st.screen.max_freq.get(&c.id.to_string()).copied() {
                    if cap > 0 && cap < max {
                        let lo = c.hw_min.unwrap_or(0);
                        let hi = c.hw_max.unwrap_or(cap);
                        let cap = c
                            .freqs
                            .iter()
                            .copied()
                            .filter(|f| *f <= cap)
                            .max()
                            .unwrap_or(cap)
                            .clamp(lo, hi);
                        max = cap;
                        if min > max {
                            min = max;
                        }
                    }
                }
            }
            if min > max {
                return Err(Error::invalid_arg(format!(
                    "policy{}: min {min} kHz is above max {max} kHz",
                    c.id
                )));
            }
            effective_max.insert(c.id, max);
            let gov = want_gov.filter(|g| *g != c.cur_gov);
            let min = (min != c.cur_min).then_some(min);
            let max = (max != c.cur_max).then_some(max);
            if gov.is_none() && min.is_none() && max.is_none() {
                continue;
            }
            plans.push(ClusterPlan {
                id: c.id,
                gov,
                min,
                max,
                cur_min: c.cur_min,
                cur_max: c.cur_max,
                cur_gov: c.cur_gov.clone(),
            });
        }
        Ok((plans, effective_max))
    }

    fn write_cluster(&self, p: &ClusterPlan) -> Result<()> {
        if let Some(g) = &p.gov {
            self.fs
                .write_str(&Self::policy_path(p.id, "scaling_governor"), g)?;
        }
        let new_min = p.min.unwrap_or(p.cur_min);
        let new_max = p.max.unwrap_or(p.cur_max);
        let min_path = Self::policy_path(p.id, "scaling_min_freq");
        let max_path = Self::policy_path(p.id, "scaling_max_freq");
        let write_min = |v: i64| -> Result<()> {
            if Some(v) != p.min {
                return Ok(());
            }
            self.fs.write_str(&min_path, &v.to_string())
        };
        let write_max = |v: i64| -> Result<()> {
            if Some(v) != p.max {
                return Ok(());
            }
            self.fs.write_str(&max_path, &v.to_string())
        };
        // never let the kernel see min > max: lower the floor first when the new
        // ceiling is below the current floor, otherwise raise/lower the ceiling first
        if new_max < p.cur_min {
            write_min(new_min)?;
            write_max(new_max)?;
        } else {
            write_max(new_max)?;
            write_min(new_min)?;
        }
        Ok(())
    }

    fn restore_cluster(&self, p: &ClusterPlan) {
        if p.gov.is_some() {
            let _ = self
                .fs
                .write_str(&Self::policy_path(p.id, "scaling_governor"), &p.cur_gov);
        }
        let min_path = Self::policy_path(p.id, "scaling_min_freq");
        let max_path = Self::policy_path(p.id, "scaling_max_freq");
        let _ = self.fs.write_str(&max_path, &p.cur_max.to_string());
        let _ = self.fs.write_str(&min_path, &p.cur_min.to_string());
        let _ = self.fs.write_str(&max_path, &p.cur_max.to_string());
    }

    fn rollback_plans(&self, plans: &[ClusterPlan]) {
        for p in plans.iter().rev() {
            self.restore_cluster(p);
        }
    }

    fn apply_plans(&self, plans: &[ClusterPlan]) -> Result<()> {
        let mut done: Vec<&ClusterPlan> = Vec::new();
        for p in plans {
            if let Err(e) = self.write_cluster(p) {
                self.restore_cluster(p);
                for old in done.iter().rev() {
                    self.restore_cluster(old);
                }
                return Err(e);
            }
            done.push(p);
        }
        Ok(())
    }

    // ---- screen-off ---------------------------------------------------------

    fn absorb_config(&self, desired: &Map<String, Value>, st: &mut CpuState) {
        st.adaptive.apply_settings(desired);
        st.screen.apply_settings(desired);
        st.gpu.apply_settings(desired);
        if let Some(v) = desired
            .get(walt::item_walt_guard())
            .and_then(Value::as_bool)
        {
            st.walt_guard = v;
        }
    }

    fn collect_with_clusters(
        &self,
        keys: Option<&[String]>,
        clusters: &[Cluster],
        st: &CpuState,
    ) -> Result<Value> {
        let schema = self.schema();
        let mut out = Map::new();
        let wanted: BTreeSet<&str> = match keys {
            Some(k) => k.iter().map(String::as_str).collect(),
            None => schema.items.iter().map(|i| i.id.as_str()).collect(),
        };
        for item in &schema.items {
            if !wanted.contains(item.id.as_str()) {
                continue;
            }
            if let Some(v) = self.read_item(&item.id, clusters, st) {
                out.insert(item.id.clone(), v);
            }
        }
        Ok(Value::Object(out))
    }

    fn collect_with_state(&self, keys: Option<&[String]>, st: &CpuState) -> Result<Value> {
        let clusters = self.clusters();
        self.collect_with_clusters(keys, &clusters, st)
    }

    fn apply_inner(
        &self,
        req: &Map<String, Value>,
        settings: &Map<String, Value>,
    ) -> Result<ApplyResult> {
        let schema = self.schema();
        schema.validate_request(req)?;
        self.validate_cluster_keys(req)?;
        self.validate_screen_request(req)?;
        let mut desired = overlay(settings, req);
        let ids: BTreeSet<String> = schema
            .items
            .iter()
            .filter(|i| i.ty != ItemType::Readonly)
            .map(|i| i.id.clone())
            .collect();
        desired.retain(|k, _| ids.contains(k));

        // A request for a policy that is not present must fail loudly instead of being
        // silently skipped and later surfacing as a value-less `cpu.max_freq` mismatch.
        // `clusters_settled` gives a just-hotplugged policy a short chance to appear.
        let requested = Self::requested_clusters(req);
        let clusters0 = self.clusters_settled(&requested, true);
        self.validate_requested_clusters(req, &clusters0)?;

        let mut st = self.state.load();
        self.absorb_config(&desired, &mut st);

        // Screen-off transition uses the current state before any frequency write.
        let active_req = desired.get(ITEM_SCREEN_ACTIVE).and_then(Value::as_bool);
        let mut target_active = active_req.unwrap_or(st.screen_active);
        if !st.screen.enabled {
            target_active = false;
        }
        self.adaptive_tick(
            &st.adaptive,
            &mut st.adaptive_runtime,
            &clusters0,
            target_active,
        );
        let mut online = match desired.get(ITEM_ONLINE) {
            Some(v) => Some(self.parse_online_request(v)?),
            None => None,
        };
        if target_active && st.screen_active {
            self.reconcile_screen_cores(&mut st, &mut online, desired.get(ITEM_ONLINE));
        }
        if target_active && !st.screen_active {
            self.begin_screen_off(&clusters0, &mut st);
            let _ = self.state.save(&st);
        } else if !target_active && st.screen_active {
            self.end_screen_off(&mut st, &mut online, desired.get(ITEM_ONLINE))?;
            let _ = self.state.save(&st);
            if !st.screen.enabled && active_req.is_none() {
                self.emit_runtime_event(
                    NewSystemEvent::new(
                        "cpu",
                        "power",
                        "screen_off_profile_released",
                        "notice",
                        "CPU screen-off profile released",
                    )
                    .message(
                        "Restored normal CPU limits and cores after screen-off profile was disabled.",
                    )
                    .metadata(json!({ "reason": "screen_off_disabled" }))
                    .dedupe("cpu.screen_off.released", 5),
                );
            }
        }
        st.screen_active = target_active;
        let online = self.screen_online(&st, online);
        if let Some(want) = &online {
            if want.iter().all(|w| *w == 0) {
                return Err(Error::invalid_arg("at least one core must stay online"));
            }
        }

        let mut changed = Vec::new();
        if let Some(want) = &online {
            if let Err(e) = self.apply_online(want, &mut changed) {
                self.rollback_online(&changed);
                return Err(e);
            }
        }

        // Re-read after online changes; affected_cpus can change while the policy list settles.
        let clusters = self.clusters_settled(&requested, true);
        self.validate_requested_clusters(req, &clusters)?;
        self.validate_walt(&desired, &clusters)?;
        let (plans, effective_max) =
            self.build_plan(req, &desired, &clusters, &st, target_active)?;
        if let Err(e) = self.apply_plans(&plans) {
            self.rollback_online(&changed);
            return Err(e);
        }

        if let Err(e) = self.apply_walt(&desired, &mut st, &clusters, &effective_max) {
            self.rollback_plans(&plans);
            self.rollback_online(&changed);
            return Err(e);
        }
        let gpu_pct = self.adaptive_gpu_pct(&st.adaptive, &st.adaptive_runtime);
        if let Err(e) = self.apply_gpu(&desired, &mut st, gpu_pct) {
            self.rollback_plans(&plans);
            self.rollback_online(&changed);
            return Err(e);
        }
        if let Err(e) = self.apply_mem(&desired, &mut st) {
            self.rollback_plans(&plans);
            self.rollback_online(&changed);
            return Err(e);
        }
        if let Err(e) = self.apply_cpuset(&desired) {
            self.rollback_plans(&plans);
            self.rollback_online(&changed);
            return Err(e);
        }

        self.state.save(&st)?;
        let keys: Vec<String> = req.keys().cloned().collect();
        let applied = match self.collect_with_clusters(Some(&keys), &clusters, &st)? {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        let mut result = ApplyResult::from_readback(req, applied);
        Self::accept_dynamic_max_cap(&mut result, req, &effective_max, &st, target_active);
        Ok(result)
    }

    /// The effective max written by `build_plan` can be lower than the user's static request
    /// because adaptive control or the screen-off profile caps it. The read-back is then not
    /// equal to the request, but the static value *was* accepted and must be persisted. Keep a
    /// real kernel/HAL rewrite as a mismatch: only excuse the key when every requested cluster
    /// reads back the effective value computed for this apply or is dynamically capped within limit.
    fn accept_dynamic_max_cap(
        result: &mut ApplyResult,
        requested: &Map<String, Value>,
        effective_max: &BTreeMap<u32, i64>,
        st: &CpuState,
        target_active: bool,
    ) {
        if !result.mismatch.iter().any(|key| key == ITEM_MAX) {
            return;
        }
        let Some(want) = requested.get(ITEM_MAX).and_then(Value::as_object) else {
            return;
        };
        let Some(have) = result.applied.get(ITEM_MAX).and_then(Value::as_object) else {
            return;
        };
        let all_match_effective = want.iter().all(|(id, want_val)| {
            let Ok(policy) = id.parse::<u32>() else {
                return false;
            };
            let Some(expected) = effective_max.get(&policy) else {
                return false;
            };
            let Some(actual) = have.get(id).and_then(Value::as_i64) else {
                return false;
            };
            if actual == *expected {
                return true;
            }
            let want_i64 = want_val.as_i64().unwrap_or(*expected);
            let dynamic_capping_active = (st.adaptive.enabled && st.adaptive_runtime.cpu_adj < 100)
                || target_active
                || st.screen_active;
            dynamic_capping_active && actual <= want_i64
        });
        if all_match_effective {
            result.mismatch.retain(|key| key != ITEM_MAX);
        }
    }

    fn validate_requested_clusters(
        &self,
        req: &Map<String, Value>,
        clusters: &[Cluster],
    ) -> Result<()> {
        for item in [ITEM_GOV, ITEM_MIN, ITEM_MAX, ITEM_SCREEN_MAX] {
            let Some(Value::Object(obj)) = req.get(item) else {
                continue;
            };
            for key in obj.keys() {
                let Ok(policy) = key.parse::<u32>() else {
                    return Err(Error::invalid_arg(format!(
                        "{item}: bad cluster id {key:?}"
                    )));
                };
                if !clusters.iter().any(|cluster| cluster.id == policy) {
                    return Err(Error::invalid_arg(format!("policy{policy} does not exist")));
                }
            }
        }
        Ok(())
    }

    fn validate_cluster_keys(&self, req: &Map<String, Value>) -> Result<()> {
        for item in [ITEM_GOV, ITEM_MIN, ITEM_MAX, ITEM_SCREEN_MAX] {
            let Some(Value::Object(obj)) = req.get(item) else {
                continue;
            };
            for key in obj.keys() {
                if key.parse::<u32>().is_err() {
                    return Err(Error::invalid_arg(format!(
                        "{item}: bad cluster id {key:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    // ---- schema helpers -----------------------------------------------------
}

/// Dynamic caps may make the live value lower than the persisted static value; only a value above
/// the request is drift (the controller is supposed to keep it at or below).
/// Is the live value at least the requested one? Used for `min_freq`, where the kernel may enforce a
/// floor above what was persisted and retrying cannot lower it.
fn lower_bound_ok(requested: &Value, applied: Option<&Value>) -> bool {
    let (Some(req), Some(applied)) = (requested.as_object(), applied.and_then(Value::as_object))
    else {
        return false;
    };
    req.iter().all(|(key, want)| {
        let (Some(want), Some(have)) = (want.as_i64(), applied.get(key).and_then(Value::as_i64))
        else {
            return false;
        };
        have >= want
    })
}

fn upper_bound_ok(requested: &Value, applied: Option<&Value>) -> bool {
    let (Some(req), Some(applied)) = (requested.as_object(), applied.and_then(Value::as_object))
    else {
        return false;
    };
    req.iter().all(|(key, want)| {
        let (Some(want), Some(have)) = (want.as_i64(), applied.get(key).and_then(Value::as_i64))
        else {
            return false;
        };
        have <= want
    })
}

fn walt_drift_ok(
    m: &CpuModule,
    key: &str,
    requested: &Value,
    current: &Map<String, Value>,
    clusters: &[Cluster],
) -> bool {
    let Some(want) = requested.as_object() else {
        return false;
    };
    let Some(have) = current.get(key).and_then(Value::as_object) else {
        return false;
    };
    for (id, want_value) in want {
        let Ok(policy) = id.parse::<u32>() else {
            return false;
        };
        let Some(c) = clusters.iter().find(|c| c.id == policy) else {
            continue;
        };
        let limited = matches!((c.cur_max, c.hw_max), (max, Some(hw)) if max < hw)
            || m.fs
                .read_i64(&CpuModule::policy_path(c.id, "scaling_max_freq"))
                .ok()
                .zip(c.hw_max)
                .is_some_and(|(max, hw)| max < hw);
        if limited
            && matches!(
                key,
                "cpu.walt.adaptive_high_freq" | "cpu.walt.adaptive_low_freq"
            )
        {
            if have.get(id).and_then(Value::as_i64) != Some(0) {
                return false;
            }
        } else if have.get(id) != Some(want_value) {
            return false;
        }
    }
    true
}

fn overlay(base: &Map<String, Value>, over: &Map<String, Value>) -> Map<String, Value> {
    let mut out = base.clone();
    for (k, v) in over {
        let merged = match (out.get_mut(k), v) {
            (Some(Value::Object(old)), Value::Object(new)) => {
                for (sub, v) in new {
                    old.insert(sub.clone(), v.clone());
                }
                true
            }
            _ => false,
        };
        if !merged {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

impl Module for CpuModule {
    fn id(&self) -> &'static str {
        "cpu"
    }

    fn probe(&self) -> Caps {
        let n = self.cpu_count();
        if n == 0 {
            return Caps::unsupported("cpu sysfs tree not readable");
        }
        let mut features = Vec::new();
        if (1..n).any(|c| self.can_toggle(c)) {
            features.push("online");
        }
        let clusters = self.clusters();
        if !clusters.is_empty() {
            features.extend([
                "max_freq",
                "min_freq",
                "cur_freq",
                "clusters",
                "time_in_state",
                "load",
            ]);
            if clusters.iter().any(|c| !c.governors.is_empty()) {
                features.push("governor");
            }
            if clusters
                .iter()
                .any(|c| c.hw_min.is_some() && c.hw_max.is_some())
            {
                features.push("hw_range");
            }
        }
        if self.gpu().is_some() {
            features.push("gpu");
        }
        if !self.walt_available_keys(&clusters).is_empty() {
            features.push("walt");
        }
        if self.walt_guard_available(&clusters) {
            features.push("walt_guard");
        }
        if !self.cpuset_groups().is_empty() {
            features.push("cpuset");
        }
        if self.mem_nodes_exist() {
            features.push("mem_boost");
        }
        if !clusters.is_empty() {
            features.push("screen_off");
            features.push("adaptive");
        }
        Caps::supported(&features)
    }

    fn schema(&self) -> Schema {
        self.build_schema()
    }

    fn collect(&self, keys: Option<&[String]>) -> Result<Value> {
        let st = self.state.load();
        self.collect_with_state(keys, &st)
    }

    /// The live min/max can be a transient boost or thermal cap (the APK itself triggers an input
    /// boost when the user taps it). Remember the hardware range as the default instead, so
    /// `reset` means "no limit" rather than "whatever the system had at the first change".
    fn normalize_default(&self, mut current: Map<String, Value>) -> Map<String, Value> {
        let clusters = self.clusters();
        for (item, file) in [
            (ITEM_MIN, "cpuinfo_min_freq"),
            (ITEM_MAX, "cpuinfo_max_freq"),
        ] {
            if let Some(Value::Object(m)) = current.get_mut(item) {
                for c in &clusters {
                    let key = c.id.to_string();
                    if !m.contains_key(&key) {
                        continue;
                    }
                    if let Ok(v) = self.fs.read_i64(&Self::policy_path(c.id, file)) {
                        m.insert(key, json!(v));
                    }
                }
            }
        }
        self.normalize_gpu_default(&mut current);
        current
    }

    /// A kernel / ROM update can change frequency tables and governor lists. Entries that are no
    /// longer valid are dropped one by one instead of failing the whole module at boot.
    /// Entries for clusters that are not present right now are kept (they end up in `mismatch`).
    fn sanitize_boot(&self, req: &Map<String, Value>) -> Map<String, Value> {
        let clusters = self.clusters();
        let schema = self.schema();
        let known: BTreeSet<&str> = schema
            .items
            .iter()
            .filter(|i| i.ty != ItemType::Readonly)
            .map(|i| i.id.as_str())
            .collect();
        let mut out: Map<String, Value> = req
            .iter()
            .filter(|(k, _)| known.contains(k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if let Some(v) = req.get(ITEM_ONLINE) {
            if self.parse_online_request(v).is_err() {
                out.remove(ITEM_ONLINE);
            }
        }
        for item in [ITEM_GOV, ITEM_MIN, ITEM_MAX] {
            let Some(Value::Object(obj)) = req.get(item) else {
                continue;
            };
            let mut kept = Map::new();
            for (key, val) in obj {
                let Ok(id) = key.parse::<u32>() else {
                    continue;
                };
                let keep = match clusters.iter().find(|c| c.id == id) {
                    None => true,
                    Some(c) if item == ITEM_GOV => val
                        .as_str()
                        .is_some_and(|g| c.governors.iter().any(|x| x == g)),
                    Some(c) => val.as_i64().is_some_and(|f| c.freqs.contains(&f)),
                };
                if keep {
                    kept.insert(key.clone(), val.clone());
                }
            }
            if kept.is_empty() {
                out.remove(item);
            } else {
                out.insert(item.to_string(), Value::Object(kept));
            }
        }
        // a floor above the ceiling cannot be applied: keep the ceiling, drop the floor
        let mins = out.get(ITEM_MIN).and_then(|v| v.as_object()).cloned();
        if let Some(mins) = mins {
            let maxs = out
                .get(ITEM_MAX)
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let mut ok = Map::new();
            for (key, val) in &mins {
                let cap = match maxs.get(key).and_then(|m| m.as_i64()) {
                    Some(m) => Some(m),
                    None => key.parse::<u32>().ok().and_then(|id| {
                        self.fs
                            .read_i64(&Self::policy_path(id, "scaling_max_freq"))
                            .ok()
                    }),
                };
                let fine = match (val.as_i64(), cap) {
                    (Some(mn), Some(mx)) => mn <= mx,
                    _ => true,
                };
                if fine {
                    ok.insert(key.clone(), val.clone());
                }
            }
            if ok.is_empty() {
                out.remove(ITEM_MIN);
            } else {
                out.insert(ITEM_MIN.to_string(), Value::Object(ok));
            }
        }
        self.sanitize_screen(&mut out);
        self.sanitize_adaptive(&mut out);
        self.sanitize_mem(&mut out);
        self.sanitize_cpuset(&mut out);
        self.sanitize_gpu(&mut out);
        self.sanitize_walt(&mut out, &clusters);
        // Final generic pass: a single out-of-range persisted value (kernel/ROM update,
        // hand-edited config) must not make the whole module fail validation at boot.
        out.retain(|k, v| {
            schema
                .items
                .iter()
                .find(|i| i.id == *k)
                .is_some_and(|item| item.validate_value(v).is_ok())
        });
        out
    }

    fn apply(&self, req: &Map<String, Value>) -> Result<ApplyResult> {
        self.apply_inner(req, &Map::new())
    }

    fn apply_with_settings(
        &self,
        req: &Map<String, Value>,
        settings: &Map<String, Value>,
    ) -> Result<ApplyResult> {
        self.apply_inner(req, settings)
    }

    /// Reset module-local state first (the registry calls this before replaying the default
    /// snapshot). Forced WALT values, the write guard, the screen-off range and the GPU extras are
    /// restored here; ordinary sysfs values are restored by the snapshot.
    fn reset(&self) -> Result<()> {
        let _lock = self.state.lock()?;
        let mut st = self.state.load();
        let clusters = self.clusters();
        let was_screen_active = st.screen_active;
        if st.screen_active {
            let mut online = None;
            self.end_screen_off(&mut st, &mut online, None)?;
            if let Some(want) = online {
                let mut changed = Vec::new();
                self.apply_online(&want, &mut changed)?;
            }
        }
        self.emit_adaptive_releases(&st.adaptive_runtime, "module_reset");
        if was_screen_active {
            self.emit_runtime_event(
                NewSystemEvent::new(
                    "cpu",
                    "power",
                    "screen_off_profile_released",
                    "notice",
                    "CPU screen-off profile released",
                )
                .message("Restored normal CPU limits and cores during CPU reset.")
                .metadata(json!({ "reason": "module_reset" }))
                .dedupe("cpu.screen_off.released", 5),
            );
        }
        self.reset_walt(&mut st, &clusters)?;
        self.reset_gpu_extras(&mut st);
        self.reset_mem(&mut st);
        self.state.clear();
        Ok(())
    }

    fn persist_on_mismatch(&self, item_id: &str) -> bool {
        item_id.starts_with("cpu.cpuset.") || item_id == ITEM_MAX || item_id == ITEM_MIN
    }

    /// Read-only enforcement check. Dynamic caps intentionally make the live max lower than the
    /// persisted static max, so those keys are not reported as drift while adaptive/screen-off is
    /// active; WALT `adaptive_*` is likewise expected to be 0 while a cluster is capped.
    fn drift(&self, settings: &Map<String, Value>) -> Result<Vec<String>> {
        if settings.is_empty() {
            return Ok(Vec::new());
        }
        let st = self.state.load();
        let clusters = self.clusters();
        let keys: Vec<String> = settings.keys().cloned().collect();
        let current = self.collect_with_state(Some(&keys), &st)?;
        let current = current.as_object().cloned().unwrap_or_default();
        let dynamic = st.adaptive.enabled || st.screen_active;
        let mut out = Vec::new();
        for (k, want) in settings {
            if k == ITEM_SCREEN_ACTIVE
                || (k.starts_with("cpu.adaptive") && k != ITEM_ADAPTIVE_STATE)
            {
                continue;
            }
            // `max_freq` and `min_freq` need opposite comparisons. A live *upper* cap below the
            // requested one is fine (thermal, adaptive); a live *lower* bound above the requested
            // one is equally fine, because the kernel's own floor cannot be written past. Using the
            // upper-bound test for both made a kernel-enforced min_freq floor retry forever.
            let ok = if dynamic && k == ITEM_MAX {
                upper_bound_ok(want, current.get(k))
            } else if dynamic && k == ITEM_MIN {
                lower_bound_ok(want, current.get(k))
            } else if dynamic && k == gpu::ITEM_GPU_MAX {
                match (want.as_i64(), current.get(k).and_then(Value::as_i64)) {
                    (Some(w), Some(h)) => h <= w,
                    _ => false,
                }
            } else if dynamic && walt::is_walt_item(k) {
                walt_drift_ok(self, k, want, &current, &clusters)
            } else if k.starts_with("cpu.cpuset.") {
                self.cpuset_effectively_satisfied(want, current.get(k))
            } else {
                current
                    .get(k)
                    .map(|have| sysflow_core::module::satisfied(want, have))
                    .unwrap_or(false)
            };
            if !ok {
                out.push(k.clone());
            }
        }
        Ok(out)
    }
}

// ---- helpers used by probe / reset that do not need the full module -------------------------

impl CpuModule {
    fn mem_nodes_exist(&self) -> bool {
        ["DDR", "LLCC"].iter().any(|k| {
            self.fs
                .exists(&format!("/sys/devices/system/cpu/bus_dcvs/{k}/boost_freq"))
        })
    }

    fn sanitize_walt(&self, out: &mut Map<String, Value>, clusters: &[Cluster]) {
        let keys = self.walt_available_keys(clusters);
        let ids: Vec<String> = out
            .keys()
            .filter(|k| walt::is_walt_item(k))
            .cloned()
            .collect();
        for id in ids {
            let Some(key) = walt::walt_key_from_item(&id) else {
                out.remove(&id);
                continue;
            };
            if !keys.contains(&key) {
                out.remove(&id);
                continue;
            }
            let Some(Value::Object(entries)) = out.get(&id) else {
                out.remove(&id);
                continue;
            };
            let mut kept = Map::new();
            for (policy, value) in entries {
                let Ok(policy) = policy.parse::<u32>() else {
                    continue;
                };
                if !clusters.iter().any(|c| c.id == policy) {
                    kept.insert(policy.to_string(), value.clone());
                    continue;
                }
                let text = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => continue,
                };
                if walt::walt_value_valid(key, &text) {
                    kept.insert(policy.to_string(), value.clone());
                }
            }
            if kept.is_empty() {
                out.remove(&id);
            } else {
                out.insert(id, Value::Object(kept));
            }
        }
    }

    fn sanitize_screen(&self, out: &mut Map<String, Value>) {
        let clusters = self.clusters();
        if let Some(Value::Object(m)) = out.get(ITEM_SCREEN_MAX) {
            let kept: Map<String, Value> = m
                .iter()
                .filter(|(k, v)| {
                    let id = k.parse::<u32>().ok();
                    let cap = v.as_i64();
                    let Some(cluster) = id.and_then(|id| clusters.iter().find(|c| c.id == id))
                    else {
                        return false;
                    };
                    let Some(cap) = cap else { return false };
                    if cap == 0 {
                        return true;
                    }
                    if cap < 0 {
                        return false;
                    }
                    let low = cluster
                        .hw_min
                        .or_else(|| cluster.freqs.first().copied())
                        .unwrap_or(0);
                    let high = cluster
                        .hw_max
                        .or_else(|| cluster.freqs.last().copied())
                        .unwrap_or(i64::MAX);
                    (low <= 0 || cap >= low) && (high <= 0 || cap <= high)
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if kept.is_empty() {
                out.remove(ITEM_SCREEN_MAX);
            } else {
                out.insert(ITEM_SCREEN_MAX.to_string(), Value::Object(kept));
            }
        }
        if let Some(Value::Array(a)) = out.get(ITEM_SCREEN_CORES) {
            let n = self.cpu_count();
            let mut ok = a.len() == n;
            for (i, v) in a.iter().enumerate() {
                let one = v.as_i64() == Some(1);
                if one && (i == 0 || !self.can_toggle(i)) {
                    ok = false;
                }
            }
            if ok {
                let on = a.iter().filter(|v| v.as_i64() == Some(1)).count();
                let toggles = (1..n).filter(|c| self.can_toggle(*c)).count();
                if on >= toggles.max(1) {
                    ok = false;
                }
            }
            if !ok {
                out.remove(ITEM_SCREEN_CORES);
            }
        }
        out.remove(ITEM_SCREEN_ACTIVE); // runtime only, never replayed at boot
    }

    fn sanitize_adaptive(&self, out: &mut Map<String, Value>) {
        let mut cfg = state::AdaptiveConfig::default();
        cfg.apply_settings(out);
        let invalid = cfg.load_t2 <= cfg.load_t1
            || cfg.load_t3 <= cfg.load_t2
            || cfg.temp_low >= cfg.temp_high
            || cfg.batt_critical >= cfg.batt_low
            || cfg.load_cap1 > cfg.load_cap2
            || cfg.load_cap2 > cfg.load_cap3
            || cfg.load_cap3 > cfg.load_cap4;
        if invalid {
            let ids: Vec<String> = out
                .keys()
                .filter(|k| k.starts_with("cpu.adaptive"))
                .cloned()
                .collect();
            for id in ids {
                out.remove(&id);
            }
        }
    }
}

mod schema;
mod screen;

#[cfg(test)]
mod tests;
