//! `sysflow battery health` — cycle count and health with a vendor fallback.
//!
//! The battery module reads sysfs only, which is the right default for a 2 s snapshot. Samsung keeps
//! its cycle counter out of sysfs entirely: the value lives in `dumpsys batteryproperties` as
//! `mSavedBatteryUsage`, in hundredths of a cycle. That needs a command that costs ~100 ms, so it is
//! an on-demand verb with a cache rather than part of `snapshot`.

use std::path::Path;

use serde_json::{json, Map, Value};

use std::time::Duration;
use sysflow_core::error::Result;
use sysflow_core::registry::Registry;
use sysflow_core::{cache, dumpsys};
use sysflow_modules::battery::{battery_node, read_cycle_count, read_soh};
const DUMP_TIMEOUT: Duration = Duration::from_secs(4);

pub fn run(reg: &Registry, refresh: bool) -> Result<Value> {
    let cache_path = reg.ctx().data_dir.join("state").join("battery_health.json");
    if !refresh {
        if let Some(cached) = read_cache(&cache_path) {
            return Ok(cached);
        }
    }

    let fs = &reg.ctx().sysfs;
    let mut value = Map::new();

    let node = battery_node(fs);
    let mut cycles = node.and_then(|node| read_cycle_count(fs, node));
    let mut soh = node.and_then(|node| read_soh(fs, node));

    // `dumpsys batteryproperties` is the only place Samsung publishes both its cycle counter and
    // its calibrated health. The call is cheap enough for an on-demand verb and is cached below.
    let properties = dump_battery_properties()?;
    if cycles.is_none() {
        cycles = samsung_usage(&properties);
    }
    if soh.is_none() {
        soh = samsung_bsoh(&properties);
    }
    // Last resort: Android 14's Health HAL, which some devices populate when the driver does not.
    if cycles.is_none() {
        let health = dump_health_hal()?;
        cycles = health_cycles(&health);
    }

    // Health priority, best source first. A derived ratio must never occupy the slot before a
    // measured source has had its chance — that bug made a calibrated 99 % show as 100 %.
    let ratio = node.and_then(|node| capacity_health_percent(fs, node));
    let learned = if soh.is_none() {
        learned_health_percent(fs, node)
    } else {
        None
    };
    // Publish the two capacities behind the percentage: a flat 100 % is only credible if the
    // learned capacity really equals the design capacity.
    if let Some(node) = node {
        if let Ok(design) = fs.read_i64(&format!("{node}/charge_full_design")) {
            value.insert("design_mah".into(), json!(load_capacity_mah(design)));
        }
    }
    if let Some(learned_mah) = learned_capacity_mah() {
        value.insert("learned_mah".into(), json!(learned_mah));
    }
    let (health_percent, health_source) = if let Some(percent) = soh {
        (Some(percent), Some("厂商 SOH"))
    } else if let Some((percent, source)) = learned {
        (Some(percent), Some(source))
    } else if let Some(percent) = ratio {
        (Some(percent), Some("sysfs 容量比"))
    } else {
        (None, None)
    };
    if let (Some(percent), Some(source)) = (health_percent, health_source) {
        value.insert("health_percent".into(), json!(percent));
        value.insert("health_source".into(), json!(source));
    }

    match cycles {
        Some(cycles) => {
            value.insert("cycle_count".into(), json!(cycles.value));
            value.insert("cycle_source".into(), json!(cycles.source));
        }
        None => {
            value.insert("cycle_count".into(), Value::Null);
            value.insert("cycle_source".into(), Value::Null);
        }
    }
    value.insert("generated_at".into(), json!(cache::now_secs()));
    value.insert("cached".into(), json!(false));

    write_cache(&cache_path, &value);
    Ok(Value::Object(value))
}

/// `charge_full / charge_full_design`, the same ratio the module reports.
fn capacity_health_percent(fs: &sysflow_core::sysfs::Sysfs, node: &str) -> Option<u32> {
    let full = fs.read_i64(&format!("{node}/charge_full")).ok()?;
    let design = fs.read_i64(&format!("{node}/charge_full_design")).ok()?;
    if full <= 0 || design <= 0 {
        return None;
    }
    Some(((full * 100 / design).clamp(0, 100)) as u32)
}

/// `mSavedBatteryUsage` counts hundredths of a full cycle: `[68142]` is 681 cycles.
fn samsung_usage(text: &str) -> Option<sysflow_modules::battery::CycleCount> {
    let raw = find_number(text, "mSavedBatteryUsage")
        .or_else(|| find_number(text, "mSecBatteryUsage"))?;
    if raw <= 0 {
        return None;
    }
    Some(sysflow_modules::battery::CycleCount {
        value: (raw / 100) as u32,
        source: "Samsung 电池 EFS",
    })
}

/// `dumpsys health` on Android 14+ prints the Health HAL state, which sometimes carries the cycle
/// counter even when the driver exposes none. The format is not contractual, so this scans for a
/// labelled integer rather than parsing structure — and it is only the last fallback.
fn health_cycles(text: &str) -> Option<sysflow_modules::battery::CycleCount> {
    for line in text.lines() {
        let lowered = line.to_ascii_lowercase();
        if !lowered.contains("cycle") {
            continue;
        }
        if let Some(value) =
            find_number(&lowered, "cycle count").or_else(|| find_number(&lowered, "cycle_count"))
        {
            if (1..=9999).contains(&value) {
                return Some(sysflow_modules::battery::CycleCount {
                    value: value as u32,
                    source: "Health HAL",
                });
            }
        }
    }
    None
}

/// Health from BatteryStats' learned capacity against the design capacity.
fn learned_health_percent(
    fs: &sysflow_core::sysfs::Sysfs,
    node: Option<&str>,
) -> Option<(u32, &'static str)> {
    let node = node?;
    let design = fs.read_i64(&format!("{node}/charge_full_design")).ok()?;
    let design = load_capacity_mah(design);
    if design <= 0 {
        return None;
    }
    let learned = learned_capacity_mah()?;
    if learned <= 0 || learned > design {
        return None;
    }
    Some((
        (learned * 100 / design).clamp(0, 100) as u32,
        "BatteryStats 学习容量",
    ))
}

/// The kernel/`BatteryStats` capacity value normalised to mAh, shared by every caller here.
fn load_capacity_mah(raw: i64) -> i64 {
    if raw.abs() > 100_000 {
        raw / 1000
    } else {
        raw
    }
}

/// BatteryStats' learned capacity, in mAh. Printed as `Estimated battery capacity: 4800 mAh` (or
/// `learned battery capacity`), which is the platform's own estimate of what the cell holds today.
fn learned_capacity_mah() -> Option<i64> {
    let text = dumpsys::run_with_timeout(&["batterystats", "--charged"], DUMP_TIMEOUT).ok()?;
    for line in text.lines() {
        let lowered = line.to_ascii_lowercase();
        if !(lowered.contains("estimated battery capacity")
            || lowered.contains("learned battery capacity"))
        {
            continue;
        }
        for token in line.split_whitespace() {
            let digits: String = token
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(value) = digits.parse::<f64>() {
                // Values arrive as e.g. `4800` mAh or `4800000` µAh.
                let mah = if value > 100_000.0 {
                    value / 1000.0
                } else {
                    value
                };
                if mah >= 100.0 {
                    return Some(mah.round() as i64);
                }
            }
        }
    }
    None
}

/// Bounded read of `dumpsys health`; it is small but still a system-service call.
fn dump_health_hal() -> Result<String> {
    dumpsys::run_with_timeout(&["health"], DUMP_TIMEOUT)
}

fn samsung_bsoh(text: &str) -> Option<u32> {
    let raw = find_number(text, "mSavedBatteryBsoh").or_else(|| find_number(text, "bsoh"))?;
    (20..=105).contains(&raw).then_some(raw as u32)
}

/// Value after `key:` or `key=`, tolerating the `[123]` bracket style dumpsys uses.
fn find_number(text: &str, key: &str) -> Option<i64> {
    for line in text.lines() {
        let Some(index) = line.find(key) else {
            continue;
        };
        let rest = &line[index + key.len()..];
        let rest = rest.trim_start_matches([':', '=', ' ', '\t', '[']);
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        if let Ok(value) = digits.parse::<i64>() {
            return Some(value);
        }
    }
    None
}

/// Cached results are reused for a minute; `--refresh` bypasses them.
fn read_cache(path: &Path) -> Option<Value> {
    let mut value = cache::read_fresh(path, cache::TTL_SECS)?;
    value["cached"] = json!(true);
    Some(value)
}

fn write_cache(path: &Path, value: &Map<String, Value>) {
    cache::write(path, &json!(value));
}

/// `dumpsys batteryproperties` — the only place Samsung exposes its cycle counter.
fn dump_battery_properties() -> Result<String> {
    dumpsys::run_with_timeout(&["batteryproperties"], DUMP_TIMEOUT)
}
