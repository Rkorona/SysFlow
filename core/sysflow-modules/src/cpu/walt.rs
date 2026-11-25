//! WALT scheduler parameters and the "official tuning" write guard.
//!
//! Two behaviours that matter on real devices and are easy to miss:
//!
//! * `scaling_max_freq` is not the only frequency ceiling. WALT's
//!   `adaptive_high_freq` / `adaptive_low_freq` can raise the effective frequency above it, so
//!   while a cluster is capped both nodes are forced to 0 and their previous values are restored
//!   when the cap is removed (ADR 0005 #5).
//! * Some vendor tuning services keep rewriting WALT nodes. The optional guard makes the selected
//!   nodes read-only (`0444`) and re-applies that mode after every reboot / apply.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::schema::{ItemType, Risk, SchemaItem, Scope};

use super::state::CpuState;
use super::{Cluster, CpuModule};

pub(crate) const WALT_KEYS: [&str; 12] = [
    "up_rate_limit_us",
    "down_rate_limit_us",
    "hispeed_freq",
    "hispeed_load",
    "hispeed_cond_freq",
    "target_loads",
    "boost",
    "pl",
    "rtg_boost_freq",
    "adaptive_low_freq",
    "adaptive_high_freq",
    "adaptive_level_1",
];

pub(crate) const WALT_GUARD_KEYS: [&str; 7] = [
    "adaptive_level_1",
    "boost",
    "hispeed_cond_freq",
    "hispeed_load",
    "pl",
    "rtg_boost_freq",
    "target_loads",
];

const ITEM_WALT_GUARD: &str = "cpu.walt_guard";
const ITEM_WALT_PREFIX: &str = "cpu.walt.";

pub(crate) fn walt_item_id(key: &str) -> String {
    format!("{ITEM_WALT_PREFIX}{key}")
}

pub(crate) fn item_walt_guard() -> &'static str {
    ITEM_WALT_GUARD
}

pub(crate) fn is_walt_item(id: &str) -> bool {
    id.starts_with(ITEM_WALT_PREFIX)
}

pub(crate) fn walt_key_from_item(id: &str) -> Option<&str> {
    id.strip_prefix(ITEM_WALT_PREFIX)
}

fn is_numeric_key(key: &str) -> bool {
    key != "target_loads"
}

fn numeric_bounds(key: &str) -> (i64, i64) {
    match key {
        "hispeed_load" | "adaptive_level_1" => (0, 100),
        "boost" | "pl" => (0, 1),
        "up_rate_limit_us" | "down_rate_limit_us" => (0, 1_000_000),
        _ => (0, 5_000_000),
    }
}

pub(crate) fn walt_value_valid(key: &str, value: &str) -> bool {
    if value.is_empty() || value.len() > 256 {
        return false;
    }
    if key == "target_loads" {
        return value
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ':' | ',' | '.' | ' ' | '-'));
    }
    value.len() <= 10 && value.chars().all(|c| c.is_ascii_digit())
}

impl CpuModule {
    fn walt_path(&self, policy: u32, key: &str) -> String {
        format!("/sys/devices/system/cpu/cpufreq/policy{policy}/walt/{key}")
    }

    fn walt_present(&self, clusters: &[Cluster], key: &str) -> bool {
        clusters
            .iter()
            .any(|c| self.fs.exists(&self.walt_path(c.id, key)))
    }

    pub(crate) fn walt_guard_available(&self, clusters: &[Cluster]) -> bool {
        WALT_GUARD_KEYS.iter().any(|k| {
            clusters
                .iter()
                .any(|c| self.fs.exists(&self.walt_path(c.id, k)))
        })
    }

    pub(crate) fn walt_available_keys(&self, clusters: &[Cluster]) -> Vec<&'static str> {
        WALT_KEYS
            .iter()
            .copied()
            .filter(|k| self.walt_present(clusters, k))
            .collect()
    }

    pub(crate) fn walt_schema_items(&self, clusters: &[Cluster]) -> Vec<SchemaItem> {
        let mut out = Vec::new();
        for key in self.walt_available_keys(clusters) {
            let mut item = SchemaItem::new(&walt_item_id(key), ItemType::Enum)
                .scope(Scope::Cluster)
                .risk(Risk::Medium)
                .persist(true);
            if key == "target_loads" {
                item = item.with_type(ItemType::Text).max_len(256);
            } else {
                let (min, max) = numeric_bounds(key);
                item = item.with_type(ItemType::IntRange).range(min, max, 1);
            }
            out.push(item);
        }
        if self.walt_guard_available(clusters) {
            out.push(
                SchemaItem::new(ITEM_WALT_GUARD, ItemType::Bool)
                    .risk(Risk::High)
                    .persist(true),
            );
        }
        out
    }

    pub(crate) fn read_walt_item(&self, id: &str, clusters: &[Cluster]) -> Option<Value> {
        if id == ITEM_WALT_GUARD {
            return Some(json!(
                self.walt_guard_available(clusters)
                    && self
                        .walt_guarded_nodes(clusters)
                        .iter()
                        .all(|p| { self.fs.mode(p).map(|m| m == 0o444).unwrap_or(false) })
            ));
        }
        let key = walt_key_from_item(id)?;
        if !WALT_KEYS.contains(&key) {
            return None;
        }
        let mut map = Map::new();
        for c in clusters {
            let path = self.walt_path(c.id, key);
            let Ok(text) = self.fs.read_str(&path) else {
                continue;
            };
            let value = if is_numeric_key(key) {
                match text.parse::<i64>() {
                    Ok(v) => json!(v),
                    Err(_) => continue,
                }
            } else {
                json!(text)
            };
            map.insert(c.id.to_string(), value);
        }
        if map.is_empty() {
            None
        } else {
            Some(Value::Object(map))
        }
    }

    fn walt_guarded_nodes(&self, clusters: &[Cluster]) -> Vec<String> {
        let mut out = Vec::new();
        for c in clusters {
            for key in WALT_GUARD_KEYS {
                let p = self.walt_path(c.id, key);
                if self.fs.exists(&p) {
                    out.push(p);
                }
            }
        }
        out
    }

    /// Lock (`0444`) or unlock (`0644`) the guarded nodes. Re-run on every apply so a ROM that
    /// resets modes at boot gets the guard back.
    pub(crate) fn apply_walt_guard(&self, enabled: bool, clusters: &[Cluster]) -> Result<()> {
        let mut failed = 0usize;
        for c in clusters {
            for key in WALT_GUARD_KEYS {
                let p = self.walt_path(c.id, key);
                if !self.fs.exists(&p) {
                    continue;
                }
                let want = if enabled { 0o444 } else { 0o644 };
                if self.fs.mode(&p).map(|m| m == want).unwrap_or(false) {
                    continue;
                }
                if self.fs.set_mode(&p, want).is_err() {
                    failed += 1;
                }
            }
        }
        if enabled && failed > 0 && self.walt_guard_available(clusters) {
            return Err(Error::permission(format!(
                "failed to lock {failed} WALT node(s); the write guard is only partially active"
            )));
        }
        Ok(())
    }

    fn write_walt_value(&self, policy: u32, key: &str, value: &str, guard: bool) -> Result<()> {
        let path = self.walt_path(policy, key);
        self.fs.write_str(&path, value)?;
        if guard && WALT_GUARD_KEYS.contains(&key) {
            let _ = self.fs.set_mode(&path, 0o444);
        }
        Ok(())
    }

    fn keep_walt_adaptive_zero(
        &self,
        desired: &Map<String, Value>,
        clusters: &[Cluster],
        final_max: &BTreeMap<u32, i64>,
        state: &mut CpuState,
        guard: bool,
    ) -> Result<()> {
        for c in clusters {
            let limited = match (final_max.get(&c.id), c.hw_max) {
                (Some(max), Some(hw)) => *max < hw,
                _ => false,
            };
            for key in ["adaptive_high_freq", "adaptive_low_freq"] {
                let path = self.walt_path(c.id, key);
                if !self.fs.exists(&path) {
                    continue;
                }
                let saved_key = format!("policy{}.{key}", c.id);
                let explicit = desired
                    .get(&walt_item_id(key))
                    .and_then(Value::as_object)
                    .and_then(|o| o.get(&c.id.to_string()))
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        _ => String::new(),
                    });
                if limited {
                    // A capped cluster always wins: WALT must not be able to bypass the ceiling.
                    if !state.walt_saved.contains_key(&saved_key) {
                        if let Ok(old) = self.fs.read_str(&path) {
                            state.walt_saved.insert(saved_key.clone(), old);
                        }
                    }
                    let cur = self.fs.read_str(&path).unwrap_or_default();
                    if cur != "0" {
                        self.write_walt_value(c.id, key, "0", guard)?;
                    }
                } else if let Some(value) = explicit.filter(|v| !v.is_empty()) {
                    // The user explicitly set this key: keep it instead of the pre-cap original.
                    let cur = self.fs.read_str(&path).unwrap_or_default();
                    if cur != value {
                        self.write_walt_value(c.id, key, &value, guard)?;
                    }
                    state.walt_saved.remove(&saved_key);
                } else if let Some(saved) = state.walt_saved.get(&saved_key).cloned() {
                    self.write_walt_value(c.id, key, &saved, guard)?;
                    state.walt_saved.remove(&saved_key);
                }
            }
        }
        Ok(())
    }

    /// Validate all requested WALT values before any sysfs write.
    pub(crate) fn validate_walt(
        &self,
        desired: &Map<String, Value>,
        clusters: &[Cluster],
    ) -> Result<()> {
        for key in self.walt_available_keys(clusters) {
            let id = walt_item_id(key);
            let Some(Value::Object(entries)) = desired.get(&id) else {
                continue;
            };
            for (policy, value) in entries {
                let Ok(policy_id) = policy.parse::<u32>() else {
                    return Err(Error::invalid_arg(format!(
                        "{id}: bad cluster id {policy:?}"
                    )));
                };
                if !clusters.iter().any(|c| c.id == policy_id) {
                    continue;
                }
                let path = self.walt_path(policy_id, key);
                if !self.fs.exists(&path) {
                    continue;
                }
                let text = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                };
                if !walt_value_valid(key, &text) {
                    return Err(Error::invalid_arg(format!(
                        "{id}[{policy}]: invalid value {value}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Apply WALT items from the effective settings, then coordinate `adaptive_*` with the final
    /// CPU ceiling and (re-)apply the write guard.
    pub(crate) fn apply_walt(
        &self,
        desired: &Map<String, Value>,
        state: &mut CpuState,
        clusters: &[Cluster],
        final_max: &BTreeMap<u32, i64>,
    ) -> Result<()> {
        let guard = state.walt_guard;

        // Explicit WALT parameters first. adaptive_* is handled below so a capped cluster cannot
        // be re-opened by a saved user value.
        for key in self.walt_available_keys(clusters) {
            let id = walt_item_id(key);
            let Some(Value::Object(entries)) = desired.get(&id) else {
                continue;
            };
            for (id_text, value) in entries {
                let Ok(policy) = id_text.parse::<u32>() else {
                    continue;
                };
                let Some(c) = clusters.iter().find(|c| c.id == policy) else {
                    continue;
                };
                if !self.fs.exists(&self.walt_path(c.id, key)) {
                    continue;
                }
                let text = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => continue,
                };
                if !walt_value_valid(key, &text) {
                    return Err(Error::invalid_arg(format!(
                        "{id}[{id_text}]: invalid value {text:?}"
                    )));
                }
                if matches!(key, "adaptive_high_freq" | "adaptive_low_freq") {
                    // handled by keep_walt_adaptive_zero below
                    continue;
                }
                let cur = self
                    .fs
                    .read_str(&self.walt_path(c.id, key))
                    .unwrap_or_default();
                if cur != text {
                    self.write_walt_value(c.id, key, &text, guard)?;
                }
            }
        }

        self.keep_walt_adaptive_zero(desired, clusters, final_max, state, guard)?;
        self.apply_walt_guard(guard, clusters)?;
        Ok(())
    }

    /// Restore everything WALT related: forced adaptive values and node permissions.
    pub(crate) fn reset_walt(&self, state: &mut CpuState, clusters: &[Cluster]) -> Result<()> {
        for (key, value) in std::mem::take(&mut state.walt_saved) {
            let Some(rest) = key.strip_prefix("policy") else {
                continue;
            };
            let Some((policy, node)) = rest.split_once('.') else {
                continue;
            };
            let Ok(policy) = policy.parse::<u32>() else {
                continue;
            };
            if clusters.iter().any(|c| c.id == policy)
                && self.fs.exists(&self.walt_path(policy, node))
            {
                let _ = self.fs.write_str(&self.walt_path(policy, node), &value);
            }
        }
        let _ = self.apply_walt_guard(false, clusters);
        state.walt_guard = false;
        Ok(())
    }
}
