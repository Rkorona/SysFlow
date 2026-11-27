//! GPU frequency control through devfreq (Adreno kgsl, Mali, G3D, ...).
//!
//! devfreq exposes frequencies in Hz while the protocol uses kHz. The raw Hz string from
//! `available_frequencies` is kept and reused for writes, so a value is never reconstructed by
//! arithmetic (some tables use values that do not round-trip through kHz cleanly).

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::schema::{ItemType, Risk, SchemaItem};

use super::state::CpuState;
use super::CpuModule;

pub(crate) const ITEM_GPU_MIN: &str = "cpu.gpu_min_freq";
pub(crate) const ITEM_GPU_MAX: &str = "cpu.gpu_max_freq";
pub(crate) const ITEM_GPU_GOV: &str = "cpu.gpu_governor";
pub(crate) const ITEM_GPU_CUR: &str = "cpu.gpu_cur_freq";
const ITEM_GPU_DCVS: &str = "cpu.gpu_dcvs_boost";
const ITEM_GPU_BUS: &str = "cpu.gpu_bus_boost";

const KGSL: &str = "/sys/class/kgsl/kgsl-3d0";

#[derive(Clone, Debug)]
pub(crate) struct Gpu {
    pub root: String,
    pub freqs: Vec<i64>,
    raw: BTreeMap<i64, String>,
    pub governors: Vec<String>,
    pub min: i64,
    pub max: i64,
    pub cur: i64,
    pub gov: String,
}

fn hz_to_khz(text: &str) -> Option<i64> {
    let hz: i64 = text.trim().parse().ok()?;
    if hz < 0 {
        return None;
    }
    Some(hz / 1000)
}

impl CpuModule {
    fn gpu_root(&self) -> Option<String> {
        let preferred = format!("{KGSL}/devfreq");
        if self.fs.exists(&format!("{preferred}/cur_freq")) {
            return Some(preferred);
        }
        let entries = self.fs.list_dir("/sys/class/devfreq").unwrap_or_default();
        for name in entries {
            let lower = name.to_ascii_lowercase();
            if ["bw", "bus", "llcc", "ddr", "cache"]
                .iter()
                .any(|bad| lower.contains(bad))
            {
                continue;
            }
            let matches = ["kgsl", "mali", "g3d", "gpu"]
                .iter()
                .any(|good| lower.contains(good));
            if !matches {
                continue;
            }
            let root = format!("/sys/class/devfreq/{name}");
            if self.fs.exists(&format!("{root}/cur_freq")) {
                return Some(root);
            }
        }
        None
    }

    pub(crate) fn gpu(&self) -> Option<Gpu> {
        let root = self.gpu_root()?;
        let mut raw = BTreeMap::new();
        if let Ok(text) = self.fs.read_str(&format!("{root}/available_frequencies")) {
            for token in text.split_whitespace() {
                if let Some(khz) = hz_to_khz(token) {
                    raw.entry(khz).or_insert_with(|| token.to_string());
                }
            }
        }
        let min = self
            .fs
            .read_str(&format!("{root}/min_freq"))
            .ok()
            .and_then(|s| hz_to_khz(&s))
            .unwrap_or(0);
        let max = self
            .fs
            .read_str(&format!("{root}/max_freq"))
            .ok()
            .and_then(|s| hz_to_khz(&s))
            .unwrap_or(0);
        let cur = self
            .fs
            .read_str(&format!("{root}/cur_freq"))
            .ok()
            .and_then(|s| hz_to_khz(&s))
            .unwrap_or(0);
        for f in [min, max, cur] {
            if f > 0 {
                raw.entry(f).or_insert_with(|| format!("{}000", f));
            }
        }
        let freqs: Vec<i64> = raw.keys().copied().collect();
        if freqs.is_empty() {
            return None;
        }
        let governors = self
            .fs
            .read_str(&format!("{root}/available_governors"))
            .map(|t| t.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default();
        let gov = self
            .fs
            .read_str(&format!("{root}/governor"))
            .unwrap_or_default();
        Some(Gpu {
            root,
            freqs,
            raw,
            governors,
            min,
            max,
            cur,
            gov,
        })
    }

    fn gpu_raw(&self, gpu: &Gpu, khz: i64) -> String {
        gpu.raw
            .get(&khz)
            .cloned()
            .unwrap_or_else(|| format!("{}000", khz))
    }

    fn gpu_hw(&self, gpu: &Gpu) -> (i64, i64) {
        (
            gpu.freqs.first().copied().unwrap_or(gpu.min),
            gpu.freqs.last().copied().unwrap_or(gpu.max),
        )
    }

    pub(crate) fn gpu_schema_items(&self) -> Vec<SchemaItem> {
        let Some(gpu) = self.gpu() else {
            return Vec::new();
        };
        let options: Vec<Value> = gpu.freqs.iter().map(|f| json!(f)).collect();
        let govs: Vec<Value> = gpu.governors.iter().map(|g| json!(g)).collect();
        let mut items = vec![
            SchemaItem::new(ITEM_GPU_MAX, ItemType::IntEnum)
                .options(options.clone())
                .unit("kHz")
                .risk(Risk::Low)
                .persist(true),
            SchemaItem::new(ITEM_GPU_MIN, ItemType::IntEnum)
                .options(options)
                .unit("kHz")
                .risk(Risk::Low)
                .persist(true),
            SchemaItem::new(ITEM_GPU_CUR, ItemType::Readonly).unit("kHz"),
        ];
        if !govs.is_empty() {
            items.push(
                SchemaItem::new(ITEM_GPU_GOV, ItemType::Enum)
                    .options(govs)
                    .risk(Risk::Low)
                    .persist(true),
            );
        }
        if self.fs.exists(&format!("{KGSL}/dcvs_boost")) {
            items.push(
                SchemaItem::new(ITEM_GPU_DCVS, ItemType::Bool)
                    .risk(Risk::Medium)
                    .persist(true),
            );
        }
        if self.gpu_bus_node().is_some() {
            items.push(
                SchemaItem::new(ITEM_GPU_BUS, ItemType::Bool)
                    .risk(Risk::Medium)
                    .persist(true),
            );
        }
        items
    }

    /// Detailed read-only GPU / bus / DCVS telemetry for the diagnostics report.
    pub(crate) fn read_gpu_info(&self) -> Value {
        let Some(gpu) = self.gpu() else {
            return Value::Null;
        };
        let bus = self.gpu_bus_node();
        let dcvs_path = format!("{KGSL}/dcvs_boost");
        let dcvs = self.fs.read_i64(&dcvs_path).ok();
        let bus_max_hz = bus
            .as_ref()
            .and_then(|root| self.fs.read_i64(&format!("{root}/max_freq")).ok());
        let bus_target_hz = bus
            .as_ref()
            .and_then(|_| gpu.raw.get(&gpu.max).and_then(|s| s.parse::<i64>().ok()));
        json!({
            "devfreq_path": gpu.root,
            "current_min_khz": gpu.min,
            "current_max_khz": gpu.max,
            "current_freq_khz": gpu.cur,
            "governor": gpu.gov,
            "available_governors": gpu.governors,
            "available_frequencies_khz": gpu.freqs,
            "dcvs_boost": dcvs,
            "bus_devfreq_path": bus,
            "bus_max_hz": bus_max_hz,
            "bus_target_hz": bus_target_hz,
        })
    }

    pub(crate) fn read_gpu_item(&self, id: &str) -> Option<Value> {
        let gpu = self.gpu()?;
        Some(match id {
            ITEM_GPU_MIN => json!(gpu.min),
            ITEM_GPU_MAX => json!(gpu.max),
            ITEM_GPU_CUR => json!(gpu.cur),
            ITEM_GPU_GOV => json!(gpu.gov),
            ITEM_GPU_DCVS => {
                let raw = self.fs.read_i64(&format!("{KGSL}/dcvs_boost")).ok()?;
                json!(raw != 0)
            }
            ITEM_GPU_BUS => {
                let root = self.gpu_bus_node()?;
                let val = self.fs.read_i64(&format!("{root}/max_freq")).ok()?;
                // linkage is on when the bus ceiling follows the GPU ceiling
                let gpu_max = gpu
                    .raw
                    .get(&gpu.max)
                    .cloned()
                    .unwrap_or_else(|| format!("{}000", gpu.max));
                json!(val.to_string() == gpu_max)
            }
            _ => return None,
        })
    }

    fn gpu_bus_node(&self) -> Option<String> {
        let candidates = [format!("{KGSL}/kgsl-busmon/devfreq"), String::new()];
        if self.fs.exists(&format!("{}/max_freq", candidates[0])) {
            return Some(candidates[0].clone());
        }
        if let Ok(platforms) = self.fs.list_dir("/sys/devices/platform") {
            for p in platforms {
                if !p.contains("kgsl-3d0") {
                    continue;
                }
                let root = format!("/sys/devices/platform/{p}/kgsl-busmon/devfreq");
                if self.fs.exists(&format!("{root}/max_freq")) {
                    return Some(root);
                }
            }
        }
        if let Ok(entries) = self.fs.list_dir("/sys/class/devfreq") {
            for name in entries {
                if !name.to_ascii_lowercase().contains("busmon") {
                    continue;
                }
                let root = format!("/sys/class/devfreq/{name}");
                if self.fs.exists(&format!("{root}/max_freq")) {
                    return Some(root);
                }
            }
        }
        None
    }

    /// Apply GPU settings. `cap_pct` is the adaptive ceiling percentage (100 = no cap) and only
    /// affects a max that is present in `desired`.
    pub(crate) fn apply_gpu(
        &self,
        desired: &Map<String, Value>,
        state: &mut CpuState,
        cap_pct: i64,
    ) -> Result<()> {
        let any = [
            ITEM_GPU_MIN,
            ITEM_GPU_MAX,
            ITEM_GPU_GOV,
            ITEM_GPU_DCVS,
            ITEM_GPU_BUS,
        ]
        .iter()
        .any(|k| desired.contains_key(*k));
        if !any {
            return Ok(());
        }
        let Some(gpu) = self.gpu() else {
            // A GPU that disappeared (driver unload) is not an error for a CPU-only request.
            return Err(Error::unsupported("GPU devfreq node is not available"));
        };
        let has_range = desired.contains_key(ITEM_GPU_MIN)
            || desired.contains_key(ITEM_GPU_MAX)
            || desired.contains_key(ITEM_GPU_GOV);
        if !has_range {
            return self.apply_gpu_extras(desired, state, &gpu, gpu.max);
        }
        let (hw_min, hw_max) = self.gpu_hw(&gpu);
        let want_min = desired.get(ITEM_GPU_MIN).and_then(Value::as_i64);
        let mut want_max = desired.get(ITEM_GPU_MAX).and_then(Value::as_i64);
        let want_gov = desired.get(ITEM_GPU_GOV).and_then(Value::as_str);
        if let Some(max) = want_max {
            if !gpu.freqs.contains(&max) || max < hw_min || max > hw_max {
                return Err(Error::invalid_arg(format!(
                    "{ITEM_GPU_MAX}: {max} kHz is not available"
                )));
            }
        }
        if let Some(min) = want_min {
            if !gpu.freqs.contains(&min) || min < hw_min || min > hw_max {
                return Err(Error::invalid_arg(format!(
                    "{ITEM_GPU_MIN}: {min} kHz is not available"
                )));
            }
        }
        if let Some(gov) = want_gov {
            if !gpu.governors.is_empty() && !gpu.governors.iter().any(|g| g == gov) {
                return Err(Error::invalid_arg(format!(
                    "{ITEM_GPU_GOV}: {gov:?} is not available"
                )));
            }
        }
        if let (Some(min), Some(max)) = (want_min, want_max) {
            if min > max {
                return Err(Error::invalid_arg(
                    "GPU min frequency is above max frequency",
                ));
            }
        }
        let floor = want_min.unwrap_or(gpu.min);
        if let Some(max) = want_max {
            if cap_pct < 100 {
                want_max = Some(self.gpu_cap_khz(&gpu, max, cap_pct).max(floor));
            }
        }

        if let Some(gov) = want_gov {
            if gov != gpu.gov {
                self.fs.write_str(&format!("{}/governor", gpu.root), gov)?;
            }
        }
        let min = want_min.unwrap_or(gpu.min);
        let max = want_max.unwrap_or(gpu.max);
        if min > max {
            return Err(Error::invalid_arg(
                "GPU min frequency is above max frequency",
            ));
        }
        let old_min = gpu.min;
        let old_max = gpu.max;
        let old_gov = gpu.gov.clone();
        let min_path = format!("{}/min_freq", gpu.root);
        let max_path = format!("{}/max_freq", gpu.root);
        let apply = || -> Result<()> {
            // same order as CPU: widen first, narrow last
            if max < old_min {
                self.fs.write_str(&min_path, &self.gpu_raw(&gpu, min))?;
                self.fs.write_str(&max_path, &self.gpu_raw(&gpu, max))?;
            } else {
                self.fs.write_str(&max_path, &self.gpu_raw(&gpu, max))?;
                self.fs.write_str(&min_path, &self.gpu_raw(&gpu, min))?;
            }
            Ok(())
        };
        if let Err(e) = apply() {
            let _ = self.fs.write_str(&max_path, &self.gpu_raw(&gpu, old_max));
            let _ = self.fs.write_str(&min_path, &self.gpu_raw(&gpu, old_min));
            if want_gov.is_some() {
                let _ = self
                    .fs
                    .write_str(&format!("{}/governor", gpu.root), &old_gov);
            }
            return Err(e);
        }

        self.apply_gpu_extras(desired, state, &gpu, max)?;
        Ok(())
    }

    fn gpu_cap_khz(&self, gpu: &Gpu, static_max: i64, cap_pct: i64) -> i64 {
        let target = static_max * cap_pct.clamp(20, 100) / 100;
        let mut snap = gpu
            .freqs
            .iter()
            .copied()
            .filter(|f| *f >= target)
            .min()
            .unwrap_or(static_max);
        snap = snap.min(static_max);
        if cap_pct < 100 && snap >= static_max {
            if let Some(below) = gpu.freqs.iter().copied().filter(|f| *f < static_max).max() {
                snap = below;
            }
        }
        snap
    }

    fn apply_gpu_extras(
        &self,
        desired: &Map<String, Value>,
        state: &mut CpuState,
        gpu: &Gpu,
        gpu_max: i64,
    ) -> Result<()> {
        if let Some(want) = desired.get(ITEM_GPU_DCVS).and_then(Value::as_bool) {
            let path = format!("{KGSL}/dcvs_boost");
            if self.fs.exists(&path) {
                if state.gpu_dcvs_original.is_none() {
                    state.gpu_dcvs_original = self.fs.read_i64(&path).ok();
                }
                let cur = self.fs.read_i64(&path).ok();
                let target = i64::from(want);
                if cur != Some(target) {
                    self.fs.write_str(&path, &target.to_string())?;
                }
            }
        }
        if let Some(want) = desired.get(ITEM_GPU_BUS).and_then(Value::as_bool) {
            if let Some(root) = self.gpu_bus_node() {
                let path = format!("{root}/max_freq");
                if state.gpu_bus_original.is_none() {
                    state.gpu_bus_original = self.fs.read_i64(&path).ok();
                }
                let target = if want {
                    self.gpu_raw(gpu, gpu_max)
                } else {
                    match state.gpu_bus_original {
                        Some(v) => v.to_string(),
                        None => return Ok(()),
                    }
                };
                let cur = self.fs.read_str(&path).unwrap_or_default();
                if cur != target {
                    self.fs.write_str(&path, &target)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn reset_gpu_extras(&self, state: &mut CpuState) {
        if let (Some(orig), true) = (
            state.gpu_dcvs_original,
            self.fs.exists(&format!("{KGSL}/dcvs_boost")),
        ) {
            let _ = self
                .fs
                .write_str(&format!("{KGSL}/dcvs_boost"), &orig.to_string());
        }
        if let (Some(orig), Some(root)) = (state.gpu_bus_original, self.gpu_bus_node()) {
            let _ = self
                .fs
                .write_str(&format!("{root}/max_freq"), &orig.to_string());
        }
        state.gpu_dcvs_original = None;
        state.gpu_bus_original = None;
        state.gpu = Default::default();
    }

    /// Default snapshot for GPU frequencies: the advertised hardware range, not a transient
    /// thermal / boost value.
    pub(crate) fn normalize_gpu_default(&self, current: &mut Map<String, Value>) {
        let Some(gpu) = self.gpu() else {
            return;
        };
        let (hw_min, hw_max) = self.gpu_hw(&gpu);
        if current.contains_key(ITEM_GPU_MIN) {
            current.insert(ITEM_GPU_MIN.to_string(), json!(hw_min));
        }
        if current.contains_key(ITEM_GPU_MAX) {
            current.insert(ITEM_GPU_MAX.to_string(), json!(hw_max));
        }
    }

    pub(crate) fn sanitize_gpu(&self, out: &mut Map<String, Value>) {
        let gpu = self.gpu();
        let valid_freq = |v: &Value| -> bool {
            match (gpu.as_ref(), v.as_i64()) {
                (Some(g), Some(f)) => g.freqs.contains(&f),
                (Some(_), None) => false,
                (None, _) => false,
            }
        };
        for item in [ITEM_GPU_MIN, ITEM_GPU_MAX] {
            if out.get(item).map(valid_freq) == Some(false) {
                out.remove(item);
            }
        }
        if let Some(gov) = out.get(ITEM_GPU_GOV).and_then(Value::as_str) {
            let ok = gpu
                .as_ref()
                .map(|g| g.governors.is_empty() || g.governors.iter().any(|x| x == gov))
                .unwrap_or(false);
            if !ok {
                out.remove(ITEM_GPU_GOV);
            }
        }
        if out.contains_key(ITEM_GPU_DCVS) && !self.fs.exists(&format!("{KGSL}/dcvs_boost")) {
            out.remove(ITEM_GPU_DCVS);
        }
        if out.contains_key(ITEM_GPU_BUS) && self.gpu_bus_node().is_none() {
            out.remove(ITEM_GPU_BUS);
        }
        if let (Some(min), Some(max)) = (
            out.get(ITEM_GPU_MIN).and_then(Value::as_i64),
            out.get(ITEM_GPU_MAX).and_then(Value::as_i64),
        ) {
            if min > max {
                out.remove(ITEM_GPU_MIN);
            }
        }
    }
}
