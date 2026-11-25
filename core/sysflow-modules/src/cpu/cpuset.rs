//! cpuset binding: which cores each process group may use (`/dev/cpuset/<group>/cpus`).
//!
//! The requested set is intersected with the cores that are online at apply time, because a
//! cpuset write that names an offline core is rejected by the kernel. The persisted value keeps
//! the user's intent, so cores that come back online later are included by the next apply.

use std::collections::BTreeSet;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::schema::{ItemType, Risk, SchemaItem};

use super::CpuModule;

const CPUSET_ROOT: &str = "/dev/cpuset";
const PREFIX: &str = "cpu.cpuset.";

fn valid_group(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

pub(crate) fn cpuset_item_id(group: &str) -> String {
    format!("{PREFIX}{group}")
}

fn group_from_item(id: &str) -> Option<&str> {
    let g = id.strip_prefix(PREFIX)?;
    valid_group(g).then_some(g)
}

pub(crate) fn parse_cpu_list(text: &str) -> Vec<usize> {
    let mut out = BTreeSet::new();
    for part in text.split(',') {
        let part = part.trim();
        if let Some((a, b)) = part.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                if a <= b && b <= 4096 {
                    out.extend(a..=b);
                }
            }
        } else if let Ok(v) = part.parse::<usize>() {
            out.insert(v);
        }
    }
    out.into_iter().collect()
}

fn cpu_csv(cpus: &[usize]) -> String {
    cpus.iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

impl CpuModule {
    pub(crate) fn cpuset_groups(&self) -> Vec<String> {
        if !self.fs.exists(&format!("{CPUSET_ROOT}/cpus")) {
            return Vec::new();
        }
        let mut out: Vec<String> = self
            .fs
            .list_dir(CPUSET_ROOT)
            .unwrap_or_default()
            .into_iter()
            .filter(|g| valid_group(g) && self.fs.exists(&format!("{CPUSET_ROOT}/{g}/cpus")))
            .collect();
        out.sort();
        out
    }

    fn cpuset_root(&self) -> Vec<usize> {
        self.fs
            .read_str(&format!("{CPUSET_ROOT}/cpus"))
            .map(|t| parse_cpu_list(&t))
            .unwrap_or_default()
    }

    fn cpuset_possible(&self) -> Vec<usize> {
        let mut out = self
            .fs
            .read_str("/sys/devices/system/cpu/possible")
            .map(|t| parse_cpu_list(&t))
            .unwrap_or_default();
        if out.is_empty() {
            out = (0..self.cpu_count()).collect();
        }
        out
    }

    fn cpuset_counts(&self) -> usize {
        self.cpuset_possible()
            .into_iter()
            .max()
            .map(|m| m + 1)
            .unwrap_or_else(|| self.cpu_count())
    }

    pub(crate) fn cpuset_schema_items(&self) -> Vec<SchemaItem> {
        let count = self.cpuset_counts();
        if count == 0 {
            return Vec::new();
        }
        self.cpuset_groups()
            .into_iter()
            .map(|g| {
                SchemaItem::new(&cpuset_item_id(&g), ItemType::BoolList)
                    .count(count)
                    .risk(Risk::Medium)
                    .persist(true)
            })
            .collect()
    }

    pub(crate) fn read_cpuset_item(&self, id: &str) -> Option<Value> {
        let group = group_from_item(id)?;
        if !self.cpuset_groups().iter().any(|g| g == group) {
            return None;
        }
        let cpus: BTreeSet<usize> = self
            .fs
            .read_str(&format!("{CPUSET_ROOT}/{group}/cpus"))
            .map(|t| parse_cpu_list(&t).into_iter().collect())
            .unwrap_or_default();
        let n = self.cpuset_counts();
        Some(Value::Array(
            (0..n)
                .map(|c| json!(i64::from(cpus.contains(&c))))
                .collect(),
        ))
    }

    /// Apply cpuset changes after onlining cores. Returns an error only when a write itself
    /// fails or the request is unusable; the read-back mismatch is reported by the registry.
    pub(crate) fn apply_cpuset(&self, desired: &Map<String, Value>) -> Result<()> {
        let root = self.cpuset_root();
        if root.is_empty() {
            return Ok(());
        }
        let root_set: BTreeSet<usize> = root.iter().copied().collect();
        let possible: BTreeSet<usize> = self.cpuset_possible().into_iter().collect();
        let mut done: Vec<(String, String)> = Vec::new();
        for group in self.cpuset_groups() {
            let id = cpuset_item_id(&group);
            let Some(Value::Array(arr)) = desired.get(&id) else {
                continue;
            };
            let want: BTreeSet<usize> = arr
                .iter()
                .enumerate()
                .filter(|(_, v)| v.as_i64() == Some(1))
                .map(|(i, _)| i)
                .collect();
            if want.is_empty() {
                return Err(Error::invalid_arg(format!(
                    "{id}: at least one core must stay enabled"
                )));
            }
            let target: Vec<usize> = want
                .iter()
                .copied()
                .filter(|c| {
                    possible.contains(c) && root_set.contains(c) && self.read_online(*c) == 1
                })
                .collect();
            if target.is_empty() {
                return Err(Error::invalid_arg(format!(
                    "{id}: none of the selected cores is online and usable right now"
                )));
            }
            let path = format!("{CPUSET_ROOT}/{group}/cpus");
            let before = self.fs.read_str(&path).unwrap_or_default();
            if parse_cpu_list(&before) != target {
                if let Err(e) = self.fs.write_str(&path, &cpu_csv(&target)) {
                    for (p, old) in done.iter().rev() {
                        let _ = self.fs.write_str(p, old);
                    }
                    return Err(e);
                }
                done.push((path, before));
            }
        }
        Ok(())
    }

    /// Offline cores are saved as user intent but cannot appear in the live cpuset mask.
    /// Treat that difference as satisfied for drift checks; once a core comes online, the
    /// normal apply path adds it to the group automatically.
    pub(crate) fn cpuset_effectively_satisfied(
        &self,
        requested: &Value,
        actual: Option<&Value>,
    ) -> bool {
        let (Some(wanted), Some(current)) =
            (requested.as_array(), actual.and_then(Value::as_array))
        else {
            return false;
        };
        let root: BTreeSet<usize> = self.cpuset_root().into_iter().collect();
        for index in 0..wanted.len().max(current.len()) {
            let want = wanted.get(index).and_then(Value::as_i64) == Some(1);
            let have = current.get(index).and_then(Value::as_i64) == Some(1);
            let expected_now = want && root.contains(&index) && self.read_online(index) == 1;
            if expected_now != have {
                return false;
            }
        }
        true
    }

    pub(crate) fn sanitize_cpuset(&self, out: &mut Map<String, Value>) {
        let groups = self.cpuset_groups();
        let n = self.cpuset_counts();
        let ids: Vec<String> = out
            .keys()
            .filter(|k| k.starts_with(PREFIX))
            .cloned()
            .collect();
        for id in ids {
            let Some(group) = group_from_item(&id) else {
                out.remove(&id);
                continue;
            };
            let keep = groups.iter().any(|g| g == group)
                && match out.get(&id) {
                    Some(Value::Array(a)) => {
                        a.len() == n && a.iter().any(|v| v.as_i64() == Some(1))
                    }
                    _ => false,
                };
            if !keep {
                out.remove(&id);
            }
        }
    }
}
