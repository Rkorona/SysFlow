//! Module-local state for the CPU module.
//!
//! The registry owns `config.json`, but some CPU features need state that is neither a sysfs
//! value nor a user setting:
//!
//! * WALT `adaptive_*` originals while the max frequency is artificially capped,
//! * the pre-screen-off frequency range / core list so `screen on` can restore it,
//! * the adaptive cap sampler and its hysteresis,
//! * the effective adaptive / screen-off / GPU options, which are not sysfs backed.
//!
//! This state lives in `<data_dir>/state/cpu.json`. It is deliberately small and is rewritten
//! atomically. A missing or corrupt file falls back to defaults: no setting is lost, only
//! transient state (which is re-created on the next apply).

use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use sysflow_core::error::{Error, Result};

fn get_bool(settings: &Map<String, Value>, id: &str) -> Option<bool> {
    settings.get(id).and_then(Value::as_bool)
}

fn get_i64(settings: &Map<String, Value>, id: &str) -> Option<i64> {
    settings.get(id).and_then(Value::as_i64)
}

fn put_bool(map: &mut Map<String, Value>, id: &str, v: bool) {
    map.insert(id.to_string(), Value::Bool(v));
}

fn put_i64(map: &mut Map<String, Value>, id: &str, v: i64) {
    map.insert(id.to_string(), Value::from(v));
}

/// Adaptive cap options (the same knobs as the shell `adaptive.conf`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdaptiveConfig {
    pub enabled: bool,
    pub smart: bool,
    pub load_t1: i64,
    pub load_t2: i64,
    pub load_t3: i64,
    pub load_cap1: i64,
    pub load_cap2: i64,
    pub load_cap3: i64,
    pub load_cap4: i64,
    pub load_down_rounds: i64,
    pub thermal: bool,
    pub temp_high: i64,
    pub temp_low: i64,
    pub temp_step: i64,
    pub temp_max: i64,
    pub temp_release_rounds: i64,
    pub battery: bool,
    pub batt_low: i64,
    pub batt_critical: i64,
    pub batt_low_cut: i64,
    pub batt_critical_cut: i64,
    pub batt_hyst: i64,
    pub psi: bool,
    pub psi_threshold: i64,
    pub psi_cap: i64,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        AdaptiveConfig {
            enabled: false,
            smart: true,
            load_t1: 12,
            load_t2: 35,
            load_t3: 70,
            load_cap1: 45,
            load_cap2: 65,
            load_cap3: 85,
            load_cap4: 100,
            load_down_rounds: 2,
            thermal: false,
            temp_high: 75,
            temp_low: 68,
            temp_step: 5,
            temp_max: 20,
            temp_release_rounds: 2,
            battery: false,
            batt_low: 20,
            batt_critical: 10,
            batt_low_cut: 5,
            batt_critical_cut: 5,
            batt_hyst: 2,
            psi: false,
            psi_threshold: 8,
            psi_cap: 85,
        }
    }
}

impl AdaptiveConfig {
    pub fn apply_settings(&mut self, settings: &Map<String, Value>) {
        if let Some(v) = get_bool(settings, "cpu.adaptive") {
            self.enabled = v;
        }
        if let Some(v) = get_bool(settings, "cpu.adaptive.smart") {
            self.smart = v;
        }
        let ints = [
            ("cpu.adaptive.load_t1", &mut self.load_t1),
            ("cpu.adaptive.load_t2", &mut self.load_t2),
            ("cpu.adaptive.load_t3", &mut self.load_t3),
            ("cpu.adaptive.load_cap1", &mut self.load_cap1),
            ("cpu.adaptive.load_cap2", &mut self.load_cap2),
            ("cpu.adaptive.load_cap3", &mut self.load_cap3),
            ("cpu.adaptive.load_cap4", &mut self.load_cap4),
            ("cpu.adaptive.load_down_rounds", &mut self.load_down_rounds),
            ("cpu.adaptive.temp_high", &mut self.temp_high),
            ("cpu.adaptive.temp_low", &mut self.temp_low),
            ("cpu.adaptive.temp_step", &mut self.temp_step),
            ("cpu.adaptive.temp_max", &mut self.temp_max),
            (
                "cpu.adaptive.temp_release_rounds",
                &mut self.temp_release_rounds,
            ),
            ("cpu.adaptive.batt_low", &mut self.batt_low),
            ("cpu.adaptive.batt_critical", &mut self.batt_critical),
            ("cpu.adaptive.batt_low_cut", &mut self.batt_low_cut),
            (
                "cpu.adaptive.batt_critical_cut",
                &mut self.batt_critical_cut,
            ),
            ("cpu.adaptive.batt_hyst", &mut self.batt_hyst),
            ("cpu.adaptive.psi_threshold", &mut self.psi_threshold),
            ("cpu.adaptive.psi_cap", &mut self.psi_cap),
        ];
        for (id, slot) in ints {
            if let Some(v) = get_i64(settings, id) {
                *slot = v;
            }
        }
        if let Some(v) = get_bool(settings, "cpu.adaptive.thermal") {
            self.thermal = v;
        }
        if let Some(v) = get_bool(settings, "cpu.adaptive.battery") {
            self.battery = v;
        }
        if let Some(v) = get_bool(settings, "cpu.adaptive.psi") {
            self.psi = v;
        }
        self.normalize();
    }

    /// Cross-field constraints from the shell implementation. Invalid individual values are
    /// rejected by the schema; this only fixes combinations that are individually valid but
    /// contradictory (t2 above t3, recovery above start, ...). Used after loading persisted
    /// settings so a bad config can never spin the controller.
    pub fn normalize(&mut self) {
        if self.load_t2 <= self.load_t1 {
            self.load_t2 = self.load_t1 + 1;
        }
        if self.load_t3 <= self.load_t2 {
            self.load_t3 = self.load_t2 + 1;
        }
        self.load_t3 = self.load_t3.clamp(3, 100);
        self.load_t2 = self
            .load_t2
            .clamp(2, self.load_t3 - 1)
            .max(self.load_t1 + 1);
        if self.load_cap2 < self.load_cap1 {
            self.load_cap2 = self.load_cap1;
        }
        if self.load_cap3 < self.load_cap2 {
            self.load_cap3 = self.load_cap2;
        }
        if self.load_cap4 < self.load_cap3 {
            self.load_cap4 = self.load_cap3;
        }
        if self.temp_low >= self.temp_high {
            self.temp_low = self.temp_high - 1;
        }
        if self.batt_critical >= self.batt_low {
            self.batt_low = (self.batt_critical + 1).min(50);
        }
    }

    /// Cap percentage for a tier (0..=3).
    pub fn load_cap(&self, tier: i64) -> i64 {
        match tier {
            0 => self.load_cap1,
            1 => self.load_cap2,
            2 => self.load_cap3,
            _ => self.load_cap4,
        }
    }

    pub fn to_json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        put_bool(&mut m, "cpu.adaptive", self.enabled);
        put_bool(&mut m, "cpu.adaptive.smart", self.smart);
        put_i64(&mut m, "cpu.adaptive.load_t1", self.load_t1);
        put_i64(&mut m, "cpu.adaptive.load_t2", self.load_t2);
        put_i64(&mut m, "cpu.adaptive.load_t3", self.load_t3);
        put_i64(&mut m, "cpu.adaptive.load_cap1", self.load_cap1);
        put_i64(&mut m, "cpu.adaptive.load_cap2", self.load_cap2);
        put_i64(&mut m, "cpu.adaptive.load_cap3", self.load_cap3);
        put_i64(&mut m, "cpu.adaptive.load_cap4", self.load_cap4);
        put_i64(
            &mut m,
            "cpu.adaptive.load_down_rounds",
            self.load_down_rounds,
        );
        put_bool(&mut m, "cpu.adaptive.thermal", self.thermal);
        put_i64(&mut m, "cpu.adaptive.temp_high", self.temp_high);
        put_i64(&mut m, "cpu.adaptive.temp_low", self.temp_low);
        put_i64(&mut m, "cpu.adaptive.temp_step", self.temp_step);
        put_i64(&mut m, "cpu.adaptive.temp_max", self.temp_max);
        put_i64(
            &mut m,
            "cpu.adaptive.temp_release_rounds",
            self.temp_release_rounds,
        );
        put_bool(&mut m, "cpu.adaptive.battery", self.battery);
        put_i64(&mut m, "cpu.adaptive.batt_low", self.batt_low);
        put_i64(&mut m, "cpu.adaptive.batt_critical", self.batt_critical);
        put_i64(&mut m, "cpu.adaptive.batt_low_cut", self.batt_low_cut);
        put_i64(
            &mut m,
            "cpu.adaptive.batt_critical_cut",
            self.batt_critical_cut,
        );
        put_i64(&mut m, "cpu.adaptive.batt_hyst", self.batt_hyst);
        put_bool(&mut m, "cpu.adaptive.psi", self.psi);
        put_i64(&mut m, "cpu.adaptive.psi_threshold", self.psi_threshold);
        put_i64(&mut m, "cpu.adaptive.psi_cap", self.psi_cap);
        m
    }
}

/// Per-policy adaptive sampler state (load counters and tier hysteresis).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyRuntime {
    pub busy: u64,
    pub total: u64,
    pub tier: i64,
    pub down: i64,
    pub load: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdaptiveRuntime {
    pub policies: BTreeMap<String, PolicyRuntime>,
    pub temp_level: i64,
    pub temp_cool: i64,
    pub batt_level: i64,
    pub cpu_adj: i64,
    pub gpu_adj: i64,
    pub psi_floor: i64,
    pub max_temp: i64,
    pub temp_sensor: String,
    pub battery_cap: i64,
    pub temp_cut: i64,
    pub batt_cut: i64,
}

impl Default for AdaptiveRuntime {
    fn default() -> Self {
        AdaptiveRuntime {
            policies: BTreeMap::new(),
            temp_level: 0,
            temp_cool: 0,
            batt_level: 0,
            cpu_adj: 100,
            gpu_adj: 100,
            psi_floor: 0,
            max_temp: -1,
            temp_sensor: String::new(),
            battery_cap: -1,
            temp_cut: 0,
            batt_cut: 0,
        }
    }
}

impl AdaptiveRuntime {
    pub fn to_json(&self, enabled: bool) -> Value {
        let mut policies = Map::new();
        for (id, p) in &self.policies {
            policies.insert(
                id.clone(),
                serde_json::json!({
                    "tier": p.tier,
                    "load": p.load,
                    "down_rounds": p.down,
                }),
            );
        }
        serde_json::json!({
            "enabled": enabled,
            "cpu_adj": self.cpu_adj,
            "gpu_adj": self.gpu_adj,
            "psi_floor": self.psi_floor,
            "temp_level": self.temp_level,
            "max_temp": self.max_temp,
            "temp_sensor": if self.temp_sensor.is_empty() { Value::Null } else { Value::String(self.temp_sensor.clone()) },
            "battery_cap": self.battery_cap,
            "temp_cut": self.temp_cut,
            "battery_cut": self.batt_cut,
            "policies": Value::Object(policies),
        })
    }

    pub fn reset_if_disabled(&mut self, enabled: bool) {
        if !enabled {
            *self = AdaptiveRuntime::default();
        }
    }
}

/// Screen-off profile options.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenConfig {
    pub enabled: bool,
    /// policy id -> max kHz. `0` means "no cap for this cluster".
    pub max_freq: BTreeMap<String, i64>,
    /// Extra cores to keep offline while the screen is off.
    pub cores: Vec<usize>,
}

impl ScreenConfig {
    pub fn apply_settings(&mut self, settings: &Map<String, Value>) {
        if let Some(v) = get_bool(settings, "cpu.screen_off") {
            self.enabled = v;
        }
        if let Some(Value::Object(m)) = settings.get("cpu.screen_off.max_freq") {
            self.max_freq.clear();
            for (k, v) in m {
                if let Some(f) = v.as_i64() {
                    self.max_freq.insert(k.clone(), f);
                }
            }
        }
        if let Some(Value::Array(a)) = settings.get("cpu.screen_off.cores") {
            self.cores = a
                .iter()
                .enumerate()
                .filter(|(_, v)| v.as_i64() == Some(1))
                .map(|(i, _)| i)
                .collect();
        }
    }

    pub fn to_json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        put_bool(&mut m, "cpu.screen_off", self.enabled);
        let mut caps = Map::new();
        for (k, v) in &self.max_freq {
            caps.insert(k.clone(), Value::from(*v));
        }
        m.insert("cpu.screen_off.max_freq".to_string(), Value::Object(caps));
        let mut cores = vec![Value::from(0); self.cores.iter().copied().max().map_or(0, |m| m + 1)];
        for c in &self.cores {
            if let Some(slot) = cores.get_mut(*c) {
                *slot = Value::from(1);
            }
        }
        m.insert("cpu.screen_off.cores".to_string(), Value::Array(cores));
        m
    }
}

/// What was in effect before the screen went off, restored on `screen on`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenSaved {
    /// policy id -> (min, max) kHz captured before the profile starts.
    pub ranges: BTreeMap<String, (i64, i64)>,
    /// Legacy list written by earlier SysFlow builds. Kept for backward compatibility.
    pub cores_off: Vec<usize>,
    /// Normal online intent captured before the screen-off profile starts. New versions capture
    /// every switchable core so a core added to the screen-off list later can still be restored.
    #[serde(default)]
    pub core_online_before: BTreeMap<usize, bool>,
    /// Current screen-off selection. `None` distinguishes old state files from a valid empty list.
    #[serde(default)]
    pub selected_cores: Option<Vec<usize>>,
}

/// GPU extras (KGSL DCVS boost / busmon linkage).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GpuConfig {
    pub dcvs_boost: Option<bool>,
    pub bus_boost: Option<bool>,
}

impl GpuConfig {
    pub fn apply_settings(&mut self, settings: &Map<String, Value>) {
        if let Some(v) = get_bool(settings, "cpu.gpu_dcvs_boost") {
            self.dcvs_boost = Some(v);
        }
        if let Some(v) = get_bool(settings, "cpu.gpu_bus_boost") {
            self.bus_boost = Some(v);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CpuState {
    pub adaptive: AdaptiveConfig,
    pub adaptive_runtime: AdaptiveRuntime,
    pub screen: ScreenConfig,
    pub screen_active: bool,
    pub screen_saved: Option<ScreenSaved>,
    pub walt_guard: bool,
    /// `policyN.key` -> value before the CPU module forced it to 0.
    pub walt_saved: BTreeMap<String, String>,
    /// `DDR` / `LLCC` -> boost_freq before the first change.
    pub mem_originals: BTreeMap<String, i64>,
    /// busmon max_freq before the first linkage change.
    pub gpu_bus_original: Option<i64>,
    /// KGSL dcvs_boost before the first change.
    pub gpu_dcvs_original: Option<i64>,
    pub gpu: GpuConfig,
}

/// Atomic JSON store; absent when the module is constructed without a data dir (unit tests
/// that only exercise sysfs behaviour do not need one).
#[derive(Clone, Debug)]
pub struct StateStore {
    dir: Option<PathBuf>,
}

impl StateStore {
    pub fn none() -> Self {
        StateStore { dir: None }
    }

    pub fn new(dir: PathBuf) -> Self {
        StateStore { dir: Some(dir) }
    }

    fn path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("cpu.json"))
    }

    pub fn load(&self) -> CpuState {
        match self.path().and_then(|p| fs::read_to_string(p).ok()) {
            Some(text) => serde_json::from_str(&text).unwrap_or_default(),
            None => CpuState::default(),
        }
    }

    pub fn save(&self, state: &CpuState) -> Result<()> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let Some(path) = self.path() else {
            return Ok(());
        };
        fs::create_dir_all(dir).map_err(|e| Error::io(format!("{}: {e}", dir.display())))?;
        let text = serde_json::to_string(state).map_err(|e| Error::io(e.to_string()))?;
        write_atomic(&path, &text)
    }

    pub fn clear(&self) {
        if let Some(path) = self.path() {
            let _ = fs::remove_file(path);
        }
    }

    /// Serialise sysfs applies between concurrent CLI invocations (APK, WebUI, boot). Returns
    /// `None` when there is no state directory (sysfs-only unit tests).
    pub fn lock(&self) -> Result<Option<StateLock>> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        fs::create_dir_all(dir).map_err(|e| Error::io(format!("{}: {e}", dir.display())))?;
        let path = dir.join("cpu.lock");
        let start = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = write!(f, "{}", std::process::id());
                    return Ok(Some(StateLock { path }));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&path) {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed() > Duration::from_secs(5) {
                        return Err(Error::busy("CPU tuning is locked by another process"));
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(Error::io(format!("{}: {e}", path.display()))),
            }
        }
    }
}

/// Held for the duration of one module apply / reset.
pub struct StateLock {
    path: PathBuf,
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn lock_is_stale(path: &Path) -> bool {
    let old = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|age| age > Duration::from_secs(60))
        .unwrap_or(false);
    if old {
        return true;
    }
    // The owner pid lives in the lock file; a killed CLI must not wedge the next call for a
    // minute. `/proc` exists on both Android and the host.
    match fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    {
        Some(pid) => !Path::new(&format!("/proc/{pid}")).exists(),
        None => false,
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp, text).map_err(|e| Error::io(format!("{}: {e}", tmp.display())))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::io(format!("{}: {e}", path.display()))
    })
}
