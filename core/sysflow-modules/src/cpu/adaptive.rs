//! Adaptive performance cap (the shell `adaptive.conf` engine).
//!
//! The engine is a pure function of the current sample plus its small hysteresis state; it never
//! writes sysfs itself. `apply` calls it once per request while enabled and then uses the
//! resulting percentages as an extra ceiling on top of the user's own max frequency. The
//! module-owned daemon calls the same apply path on the configured enforcement interval.
//!
//! Policy: load tiers rise immediately and fall slowly; thermal cuts step up immediately and
//! step down only after `temp_release_rounds`; battery cuts use a hysteresis; PSI acts only as a
//! floor on the load cap and only when no thermal/battery cut is active.

use std::collections::BTreeMap;

use serde_json::json;
use sysflow_core::event::NewSystemEvent;
use sysflow_core::sysfs::Sysfs;

use super::state::{AdaptiveConfig, AdaptiveRuntime, PolicyRuntime};
use super::{Cluster, CpuModule};

const PROC_ROOT: &str = "/proc";
const THERMAL_ROOT: &str = "/sys/class/thermal";
const BATTERY_ROOT: &str = "/sys/class/power_supply/battery";

pub(crate) type Stat = BTreeMap<usize, (u64, u64)>;

impl CpuModule {
    /// Emit release transitions whenever active adaptive protections are cleared externally
    /// (by turning off `cpu.adaptive` or resetting the CPU module).
    pub(crate) fn emit_adaptive_releases(&self, rt: &AdaptiveRuntime, reason: &str) {
        if rt.temp_level > 0 {
            self.emit_runtime_event(
                NewSystemEvent::new(
                    "cpu",
                    "thermal",
                    "thermal_protection_transition",
                    "notice",
                    "CPU thermal protection released",
                )
                .message(format!("Thermal protection released ({reason})."))
                .metadata(json!({
                    "reason": reason,
                    "previous_level": rt.temp_level,
                    "previous_cap_percent": rt.temp_cut,
                }))
                .dedupe("cpu.thermal.released", 2),
            );
        }
        if rt.batt_level > 0 {
            self.emit_runtime_event(
                NewSystemEvent::new(
                    "cpu",
                    "battery",
                    "battery_saving_transition",
                    "info",
                    "Battery saving released",
                )
                .message(format!("Battery saving released ({reason})."))
                .metadata(json!({
                    "reason": reason,
                    "previous_level": rt.batt_level,
                    "previous_cap_percent": rt.batt_cut,
                }))
                .dedupe("cpu.battery_saving.released", 2),
            );
        }
        if rt.psi_floor > 0 {
            self.emit_runtime_event(
                NewSystemEvent::new(
                    "cpu",
                    "pressure",
                    "psi_guard_transition",
                    "info",
                    "CPU PSI guard released",
                )
                .message(format!("PSI guard released ({reason})."))
                .metadata(json!({
                    "reason": reason,
                    "previous_floor_percent": rt.psi_floor,
                }))
                .dedupe("cpu.psi.released", 2),
            );
        }
    }

    pub(crate) fn adaptive_tick(
        &self,
        cfg: &AdaptiveConfig,
        rt: &mut AdaptiveRuntime,
        clusters: &[Cluster],
        screen_off_active: bool,
    ) {
        if !cfg.enabled {
            self.emit_adaptive_releases(rt, "adaptive_disabled");
            rt.reset_if_disabled(false);
            return;
        }
        // SoCFlow intentionally pauses smart load sampling while the screen-off profile is
        // active. Keep the last tier until the display wakes so the two profiles do not race.
        if cfg.smart && !screen_off_active {
            let stat = read_proc_stat(&self.fs);
            for c in clusters {
                let key = c.id.to_string();
                let p = rt.policies.entry(key).or_default();
                match stat.as_ref().and_then(|s| aggregate(s, &c.sample_cpus)) {
                    Some((busy, total)) => update_load(p, busy, total, cfg),
                    None => {
                        // /proc/stat is not readable in this context; hold the last tier.
                        if p.load == 0 {
                            p.load = -1;
                        }
                    }
                }
            }
            rt.policies
                .retain(|k, _| clusters.iter().any(|c| c.id.to_string() == *k));
        }

        let previous_temp_level = rt.temp_level;
        let previous_battery_level = rt.batt_level;
        let previous_psi_floor = rt.psi_floor;
        let (max_temp, sensor) = read_max_thermal_c(&self.fs);
        rt.max_temp = max_temp;
        rt.temp_sensor = sensor;
        if cfg.thermal {
            let max_level = if cfg.temp_step > 0 {
                (cfg.temp_max + cfg.temp_step - 1) / cfg.temp_step
            } else {
                0
            };
            if rt.temp_level > max_level {
                rt.temp_level = max_level;
            }
            if max_temp >= 0 {
                if max_temp > cfg.temp_high && rt.temp_level < max_level {
                    rt.temp_level += 1;
                    rt.temp_cool = 0;
                } else if max_temp < cfg.temp_low && rt.temp_level > 0 {
                    rt.temp_cool += 1;
                    if rt.temp_cool >= cfg.temp_release_rounds {
                        rt.temp_level -= 1;
                        rt.temp_cool = 0;
                    }
                } else {
                    rt.temp_cool = 0;
                }
            }
        } else {
            rt.temp_level = 0;
            rt.temp_cool = 0;
        }
        rt.temp_cut = (rt.temp_level * cfg.temp_step).min(cfg.temp_max);
        if rt.temp_level != previous_temp_level {
            let (severity, title, dedupe_key) = if rt.temp_level == 0 {
                (
                    "notice",
                    "CPU thermal protection released",
                    "cpu.thermal.released".to_string(),
                )
            } else if previous_temp_level == 0 {
                (
                    "warning",
                    "CPU thermal protection engaged",
                    format!("cpu.thermal.level_{}", rt.temp_level),
                )
            } else if rt.temp_level > previous_temp_level {
                (
                    "warning",
                    "CPU thermal protection tightened",
                    format!("cpu.thermal.level_{}", rt.temp_level),
                )
            } else {
                (
                    "notice",
                    "CPU thermal protection eased",
                    format!("cpu.thermal.level_{}", rt.temp_level),
                )
            };
            self.emit_runtime_event(
                NewSystemEvent::new(
                    "cpu",
                    "thermal",
                    "thermal_protection_transition",
                    severity,
                    title,
                )
                .message(format!(
                    "Temperature {}°C; thermal cap {}% (sensor: {}).",
                    max_temp, rt.temp_cut, rt.temp_sensor
                ))
                .metadata(json!({
                    "temperature_c": max_temp,
                    "sensor": rt.temp_sensor,
                    "level": rt.temp_level,
                    "previous_level": previous_temp_level,
                    "cap_percent": rt.temp_cut,
                }))
                .dedupe(dedupe_key, 2),
            );
        }

        rt.battery_cap = -1;
        let mut batt_level = 0;
        if cfg.battery {
            if let Some((cap, discharging)) = read_battery(&self.fs) {
                if cap <= 100 && discharging {
                    rt.battery_cap = cap;
                    let was = rt.batt_level;
                    if cap <= cfg.batt_critical
                        || (was == 2 && cap <= cfg.batt_critical + cfg.batt_hyst)
                    {
                        batt_level = 2;
                    } else if cap <= cfg.batt_low
                        || (was >= 1 && cap <= cfg.batt_low + cfg.batt_hyst)
                    {
                        batt_level = 1;
                    }
                }
            }
        }
        rt.batt_level = batt_level;
        rt.batt_cut = match batt_level {
            2 => cfg.batt_low_cut + cfg.batt_critical_cut,
            1 => cfg.batt_low_cut,
            _ => 0,
        };
        if batt_level != previous_battery_level {
            let (severity, title, dedupe_key) = if batt_level >= 2 {
                (
                    "warning",
                    "Critical battery saving engaged",
                    format!("cpu.battery_saving.level_{batt_level}"),
                )
            } else if batt_level == 1 && previous_battery_level >= 2 {
                (
                    "notice",
                    "Battery saving eased",
                    format!("cpu.battery_saving.level_{batt_level}"),
                )
            } else if batt_level > 0 {
                (
                    "notice",
                    "Battery saving engaged",
                    format!("cpu.battery_saving.level_{batt_level}"),
                )
            } else {
                (
                    "info",
                    "Battery saving released",
                    "cpu.battery_saving.released".to_string(),
                )
            };
            self.emit_runtime_event(
                NewSystemEvent::new(
                    "cpu",
                    "battery",
                    "battery_saving_transition",
                    severity,
                    title,
                )
                .message(format!(
                    "Battery level {}%; applied adaptive cut {}%.",
                    rt.battery_cap, rt.batt_cut
                ))
                .metadata(json!({
                    "battery_percent": rt.battery_cap,
                    "level": batt_level,
                    "previous_level": previous_battery_level,
                    "cap_percent": rt.batt_cut,
                }))
                .dedupe(dedupe_key, 2),
            );
        }

        rt.psi_floor = 0;
        let mut observed_psi_avg10 = None;
        if cfg.psi && cfg.smart && rt.temp_cut == 0 && rt.batt_cut == 0 {
            if let Some(avg10) = read_psi_avg10(&self.fs) {
                observed_psi_avg10 = Some(avg10);
                if avg10 >= cfg.psi_threshold as f64 {
                    rt.psi_floor = cfg.psi_cap;
                }
            }
        }

        let adj = (100 - rt.temp_cut - rt.batt_cut).clamp(20, 100);
        rt.cpu_adj = adj;
        rt.gpu_adj = adj;
        if rt.psi_floor != previous_psi_floor {
            let active = rt.psi_floor > 0;
            let message = if active {
                "CPU pressure crossed the configured guard threshold."
            } else if rt.temp_cut > 0 || rt.batt_cut > 0 {
                "CPU PSI guard yielded to active thermal or battery cap."
            } else {
                "CPU pressure guard is no longer active."
            };
            self.emit_runtime_event(
                NewSystemEvent::new(
                    "cpu",
                    "pressure",
                    "psi_guard_transition",
                    if active { "notice" } else { "info" },
                    if active {
                        "CPU PSI guard engaged"
                    } else {
                        "CPU PSI guard released"
                    },
                )
                .message(message)
                .metadata(json!({
                    "psi_avg10": observed_psi_avg10,
                    "threshold": cfg.psi_threshold,
                    "psi_floor_percent": rt.psi_floor,
                    "previous_floor_percent": previous_psi_floor,
                }))
                .dedupe(
                    if active {
                        "cpu.psi.engaged"
                    } else {
                        "cpu.psi.released"
                    },
                    60,
                ),
            );
        }
    }

    /// Percentage of the user's static CPU ceiling for one cluster (100 = no extra cap).
    pub(crate) fn adaptive_cpu_pct(
        &self,
        cfg: &AdaptiveConfig,
        rt: &AdaptiveRuntime,
        policy: u32,
    ) -> i64 {
        if !cfg.enabled {
            return 100;
        }
        if !cfg.smart {
            return rt.cpu_adj.clamp(20, 100);
        }
        let tier = rt
            .policies
            .get(&policy.to_string())
            .map(|p| p.tier)
            .unwrap_or(3);
        let mut base = cfg.load_cap(tier);
        if rt.psi_floor > 0 && rt.cpu_adj == 100 && base < rt.psi_floor {
            base = rt.psi_floor;
        }
        (base * rt.cpu_adj / 100).clamp(20, 100)
    }

    pub(crate) fn adaptive_gpu_pct(&self, cfg: &AdaptiveConfig, rt: &AdaptiveRuntime) -> i64 {
        if cfg.enabled {
            rt.gpu_adj.clamp(20, 100)
        } else {
            100
        }
    }
}

fn aggregate(stat: &Stat, cpus: &[usize]) -> Option<(u64, u64)> {
    let mut busy = 0u64;
    let mut total = 0u64;
    let mut any = false;
    for cpu in cpus {
        if let Some((b, t)) = stat.get(cpu) {
            busy = busy.saturating_add(*b);
            total = total.saturating_add(*t);
            any = true;
        }
    }
    (any && total > 0).then_some((busy, total))
}

fn update_load(p: &mut PolicyRuntime, busy: u64, total: u64, cfg: &AdaptiveConfig) {
    if p.total == 0 || busy < p.busy || total <= p.total {
        // First sample, counter wrap, or a counter reset: establish a baseline without
        // throttling. When the counters have not moved yet, keep the previous tier.
        if p.total == 0 || busy < p.busy {
            p.tier = 3;
            p.down = 0;
            p.load = -1;
        }
        p.busy = busy;
        p.total = total;
        return;
    }
    let dt = total - p.total;
    let db = (busy - p.busy).min(dt);
    let load = ((db as u128 * 100 + dt as u128 / 2) / dt as u128) as i64;
    let desired = if load < cfg.load_t1 {
        0
    } else if load < cfg.load_t2 {
        1
    } else if load < cfg.load_t3 {
        2
    } else {
        3
    };
    if desired > p.tier {
        p.tier = desired;
        p.down = 0;
    } else if desired < p.tier {
        p.down += 1;
        if p.down >= cfg.load_down_rounds.max(1) {
            p.tier -= 1;
            p.down = 0;
        }
    } else {
        p.down = 0;
    }
    p.tier = p.tier.clamp(0, 3);
    p.busy = busy;
    p.total = total;
    p.load = load;
}

pub(crate) fn read_proc_stat(fs: &Sysfs) -> Option<Stat> {
    let text = fs.read_str(&format!("{PROC_ROOT}/stat")).ok()?;
    let mut out = Stat::new();
    for line in text.lines() {
        // `/proc/stat` starts with an aggregate `cpu` row followed by `cpu0`, `cpu1`, ... .
        // Do not parse the first counter on the aggregate row as a CPU id: it is usually a
        // large number and would contaminate both per-core totals and the aggregate calculation.
        let rest = match line.strip_prefix("cpu") {
            Some(r) => r,
            None => continue,
        };
        let id_len = rest.bytes().take_while(u8::is_ascii_digit).count();
        if id_len == 0 {
            continue;
        }
        let id = match rest[..id_len].parse::<usize>() {
            Ok(id) => id,
            Err(_) => continue,
        };
        let it = rest[id_len..].split_whitespace();
        let fields: Vec<u64> = it.take(8).filter_map(|f| f.parse::<u64>().ok()).collect();
        if fields.len() < 8 {
            continue;
        }
        let (user, nice, system, idle, iowait, irq, softirq, steal) = (
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5], fields[6], fields[7],
        );
        let busy = user
            .saturating_add(nice)
            .saturating_add(system)
            .saturating_add(irq)
            .saturating_add(softirq)
            .saturating_add(steal);
        let total = busy.saturating_add(idle).saturating_add(iowait);
        out.insert(id, (busy, total));
    }
    (!out.is_empty()).then_some(out)
}

/// Hottest CPU/GPU/SoC thermal zone in whole °C, with the zone type for diagnostics.
fn read_max_thermal_c(fs: &Sysfs) -> (i64, String) {
    let zones = fs.list_dir(THERMAL_ROOT).unwrap_or_default();
    let mut best = -1i64;
    let mut sensor = String::new();
    for z in zones.iter().filter(|z| z.starts_with("thermal_zone")) {
        let ty = match fs.read_str(&format!("{THERMAL_ROOT}/{z}/type")) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let lower = ty.to_ascii_lowercase();
        let relevant = ["cpu", "gpu", "soc"].iter().any(|k| lower.contains(k));
        if !relevant || lower.contains("trip") {
            continue;
        }
        let Ok(raw) = fs.read_i64(&format!("{THERMAL_ROOT}/{z}/temp")) else {
            continue;
        };
        if raw < 0 {
            continue;
        }
        let c = if raw >= 1000 { raw / 1000 } else { raw };
        if c > 150 {
            continue;
        }
        if c > best {
            best = c;
            sensor = ty;
        }
    }
    (best, sensor)
}

fn read_battery(fs: &Sysfs) -> Option<(i64, bool)> {
    let cap = fs.read_i64(&format!("{BATTERY_ROOT}/capacity")).ok()?;
    let status = fs
        .read_str(&format!("{BATTERY_ROOT}/status"))
        .unwrap_or_default();
    Some((cap, status.eq_ignore_ascii_case("discharging")))
}

fn read_psi_avg10(fs: &Sysfs) -> Option<f64> {
    let text = fs.read_str(&format!("{PROC_ROOT}/pressure/cpu")).ok()?;
    for line in text.lines() {
        if !line.starts_with("some ") {
            continue;
        }
        for token in line.split_whitespace() {
            if let Some(v) = token.strip_prefix("avg10=") {
                if let Ok(f) = v.parse::<f64>() {
                    return Some(f);
                }
            }
        }
    }
    None
}
