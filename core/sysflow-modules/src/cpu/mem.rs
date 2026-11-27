//! DDR / LLCC bus boost frequencies under `/sys/devices/system/cpu/bus_dcvs`.
//!
//! These nodes are optional and chipset specific. They are exposed as one `int_enum` per
//! existing kind; the original value is remembered before the first change so `reset` can put
//! it back even when the default snapshot was never recorded.

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::schema::{ItemType, Risk, SchemaItem};

use super::state::CpuState;
use super::CpuModule;

const MEM_ROOT: &str = "/sys/devices/system/cpu/bus_dcvs";

pub(crate) fn mem_item_id(kind: &str) -> String {
    format!("cpu.{}_boost_freq", kind.to_ascii_lowercase())
}

fn kind_from_item(id: &str) -> Option<&'static str> {
    match id {
        "cpu.ddr_boost_freq" => Some("DDR"),
        "cpu.llcc_boost_freq" => Some("LLCC"),
        _ => None,
    }
}

#[derive(Clone, Debug)]
struct MemNode {
    kind: &'static str,
    root: String,
    freqs: Vec<i64>,
    current: i64,
}

impl CpuModule {
    fn mem_node(&self, kind: &str) -> Option<MemNode> {
        let root = format!("{MEM_ROOT}/{kind}");
        let path = format!("{root}/boost_freq");
        if !self.fs.exists(&path) {
            return None;
        }
        let current = self.fs.read_i64(&path).ok()?;
        let mut freqs: Vec<i64> = self
            .fs
            .read_str(&format!("{root}/available_frequencies"))
            .map(|t| {
                t.split_whitespace()
                    .filter_map(|f| f.parse::<i64>().ok())
                    .collect()
            })
            .unwrap_or_default();
        freqs.sort_unstable();
        freqs.dedup();
        Some(MemNode {
            kind: kind_name(kind),
            root,
            freqs,
            current,
        })
    }

    fn mem_nodes(&self) -> Vec<MemNode> {
        ["DDR", "LLCC"]
            .iter()
            .filter_map(|k| self.mem_node(k))
            .collect()
    }

    pub(crate) fn mem_schema_items(&self) -> Vec<SchemaItem> {
        self.mem_nodes()
            .into_iter()
            .map(|n| {
                let options = if n.freqs.is_empty() {
                    vec![json!(n.current)]
                } else {
                    n.freqs.iter().map(|f| json!(f)).collect()
                };
                SchemaItem::new(&mem_item_id(n.kind), ItemType::IntEnum)
                    .options(options)
                    .risk(Risk::Medium)
                    .persist(true)
            })
            .collect()
    }

    pub(crate) fn read_mem_item(&self, id: &str) -> Option<Value> {
        let kind = kind_from_item(id)?;
        let n = self.mem_node(kind)?;
        Some(json!(n.current))
    }

    pub(crate) fn apply_mem(
        &self,
        desired: &Map<String, Value>,
        state: &mut CpuState,
    ) -> Result<()> {
        for kind in ["DDR", "LLCC"] {
            let id = mem_item_id(kind);
            let Some(want) = desired.get(&id).and_then(Value::as_i64) else {
                continue;
            };
            let Some(node) = self.mem_node(kind) else {
                return Err(Error::unsupported(format!(
                    "{id}: {MEM_ROOT}/{kind}/boost_freq is not available"
                )));
            };
            if !node.freqs.is_empty() && !node.freqs.contains(&want) {
                return Err(Error::invalid_arg(format!(
                    "{id}: {want} is not in available_frequencies"
                )));
            }
            if want == node.current {
                continue;
            }
            state
                .mem_originals
                .entry(kind.to_string())
                .or_insert(node.current);
            self.fs
                .write_str(&format!("{}/boost_freq", node.root), &want.to_string())?;
        }
        Ok(())
    }

    pub(crate) fn reset_mem(&self, state: &mut CpuState) {
        for (kind, value) in std::mem::take(&mut state.mem_originals) {
            if let Some(node) = self.mem_node(&kind) {
                let _ = self
                    .fs
                    .write_str(&format!("{}/boost_freq", node.root), &value.to_string());
            }
        }
    }

    pub(crate) fn sanitize_mem(&self, out: &mut Map<String, Value>) {
        for kind in ["DDR", "LLCC"] {
            let id = mem_item_id(kind);
            let Some(v) = out.get(&id).and_then(Value::as_i64) else {
                continue;
            };
            let keep = self
                .mem_node(kind)
                .map(|n| n.freqs.is_empty() || n.freqs.contains(&v))
                .unwrap_or(false);
            if !keep {
                out.remove(&id);
            }
        }
    }
}

fn kind_name(kind: &str) -> &'static str {
    match kind {
        "DDR" => "DDR",
        _ => "LLCC",
    }
}
