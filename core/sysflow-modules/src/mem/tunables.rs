//! The writable half of the memory module (M3b): `swappiness` and `vfs_cache_pressure`.
//!
//! Only two knobs, on purpose. `dirty_*` is absent: S25 uses the byte interface
//! (`dirty_bytes`), and its `watermark_scale_factor` / `page-cluster` are already tuned by the ROM,
//! so writing ratios would fight that tuning for no measurable gain
//! (`docs/mem-module-plan.md`, 待定项 10). `min_free_kbytes` stays out too: it is a watermark, and a
//! wrong value causes OOM instead of preventing it.
//!
//! Unlike the telemetry half, these are plain integers the kernel either accepts or rejects, so
//! `apply` writes and reads back and lets `mismatch` tell the truth.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::module::ApplyResult;
use sysflow_core::schema::{ItemType, Risk, SchemaItem};
use sysflow_core::sysfs::Sysfs;

use super::probe;

pub const SWAPPINESS: &str = "mem.swappiness";
pub const VFS_CACHE_PRESSURE: &str = "mem.vfs_cache_pressure";

const SWAPPINESS_PATH: &str = "/proc/sys/vm/swappiness";
const VFS_PATH: &str = "/proc/sys/vm/vfs_cache_pressure";
const OSRELEASE: &str = "/proc/sys/kernel/osrelease";

/// A repair that keeps being undone is not a drift to fix, it is a fight to lose. After this many
/// repairs inside [`WINDOW_S`], the item is left alone for [`SUPPRESS_S`].
pub const WINDOW_S: u64 = 30 * 60;
pub const MAX_REPAIRS: usize = 3;
pub const SUPPRESS_S: u64 = 6 * 3600;

/// One writable sysctl.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tunable {
    pub id: &'static str,
    pub path: &'static str,
    pub min: i64,
    pub max: i64,
}

/// The tunables this device actually offers.
///
/// `swappiness` accepts 0..=100 before 5.8 and 0..=200 from 5.8, so the ceiling is read from the
/// running kernel instead of guessed: writing 130 on a 5.4 kernel would be rejected with `EINVAL`.
pub fn specs(fs: &Sysfs) -> Vec<Tunable> {
    let release = fs.read_str(OSRELEASE).ok();
    let mut out = Vec::new();
    if fs.read_str(SWAPPINESS_PATH).is_ok() {
        out.push(Tunable {
            id: SWAPPINESS,
            path: SWAPPINESS_PATH,
            min: 0,
            max: probe::swappiness_max(release.as_deref()),
        });
    }
    if fs.read_str(VFS_PATH).is_ok() {
        out.push(Tunable {
            id: VFS_CACHE_PRESSURE,
            path: VFS_PATH,
            // 0 means "never reclaim dentries", which is not a plausible thing to ask for.
            min: 1,
            max: 1000,
        });
    }
    out
}

pub fn schema_items(fs: &Sysfs) -> Vec<SchemaItem> {
    specs(fs)
        .into_iter()
        .map(|spec| {
            SchemaItem::new(spec.id, ItemType::IntRange)
                .range(spec.min, spec.max, 1)
                .risk(Risk::Low)
                .persist(true)
        })
        .collect()
}

/// Current values, for `collect` / `snapshot`.
pub fn values(fs: &Sysfs) -> Map<String, Value> {
    let mut out = Map::new();
    for spec in specs(fs) {
        if let Ok(value) = fs.read_i64(spec.path) {
            out.insert(spec.id.to_string(), json!(value));
        }
    }
    out
}

/// Write every requested tunable and report what the kernel actually kept.
pub fn apply(fs: &Sysfs, req: &Map<String, Value>) -> Result<ApplyResult> {
    let specs = specs(fs);
    let mut requested = Map::new();
    let mut applied = Map::new();

    // Validate everything before writing anything: a half-applied request is worse than a rejected
    // one, because the persisted settings would then describe a state the device never had.
    for (id, value) in req {
        let Some(spec) = specs.iter().find(|spec| spec.id == id) else {
            return Err(Error::invalid_arg(format!(
                "unknown or read-only key: {id}"
            )));
        };
        let Some(want) = value.as_i64() else {
            return Err(Error::invalid_arg(format!("{id}: expected an integer")));
        };
        if want < spec.min || want > spec.max {
            return Err(Error::invalid_arg(format!(
                "{id}: {want} is outside {}..={} (this kernel)",
                spec.min, spec.max
            )));
        }
        requested.insert(id.clone(), json!(want));
    }

    for (id, value) in &requested {
        let spec = specs
            .iter()
            .find(|spec| spec.id == id)
            .expect("validated above");
        let want = value.as_i64().expect("validated above");
        fs.write_str(spec.path, &want.to_string())?;
        // Read back like every other module: `mismatch` must describe the kernel, not the request.
        let readback = fs.read_i64(spec.path)?;
        applied.insert(id.clone(), json!(readback));
    }

    Ok(ApplyResult::from_readback(&requested, applied))
}

/// Drop persisted values this kernel cannot accept (a swapped kernel, a hand-edited config).
pub fn sanitize(fs: &Sysfs, out: &mut Map<String, Value>) {
    let specs = specs(fs);
    out.retain(
        |key, value| match specs.iter().find(|spec| spec.id == key) {
            Some(spec) => value
                .as_i64()
                .is_some_and(|want| want >= spec.min && want <= spec.max),
            // Anything that is not a tunable is none of this module's business here; the registry's
            // generic pass rejects ids the schema does not know.
            None => true,
        },
    );
}

/// What the repair guard decided about one mismatch.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Decision {
    /// Repair it now.
    Repair,
    /// The item is being undone by something else: do not touch it.
    Suppressed,
    /// This call is the one that gave up; the caller should say so once.
    JustSuppressed,
}

/// Calls closer together than this are one observation, not two.
///
/// The enforcement loop asks about every item **twice per tick**: once to find what drifted, and
/// again right after the repair to see what is still wrong. Counting both would burn the repair
/// budget twice as fast, and treating the second as "already being repaired" (the previous design)
/// made a value that the repair could not fix stay wrong forever without ever being reported. The
/// shortest guard interval is 2 s, so two seconds separates ticks from the two calls inside one.
pub const TICK_GAP_S: u64 = 2;

#[derive(Debug, Default, Clone)]
struct Watch {
    /// When repairs were attempted, oldest first, trimmed to [`WINDOW_S`].
    attempts: Vec<u64>,
    /// The second at which the current mismatch was first answered with [`Decision::Repair`].
    /// `None` once the value matched again, so the next mismatch is always a fresh observation.
    answered_at: Option<u64>,
    suppressed_until: u64,
}

/// Rate limiter for repeated repairs of the same item.
///
/// Kept in memory: it exists to stop a tight repair loop, and a daemon restart is exactly the moment
/// where one fresh attempt is reasonable.
///
/// Each *tick* that finds the item mismatched counts as one attempt, whether the value went back in
/// between (something resets it every few minutes) or never did (the write is refused, or reverted
/// within the tick). Either way, [`MAX_REPAIRS`] attempts inside [`WINDOW_S`] means this is a fight
/// that repairing will not win.
#[derive(Debug, Default)]
pub struct Guard {
    inner: Mutex<BTreeMap<&'static str, Watch>>,
}

impl Guard {
    pub fn check(&self, id: &'static str, now: u64) -> Decision {
        let mut map = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let watch = map.entry(id).or_default();
        if now < watch.suppressed_until {
            return Decision::Suppressed;
        }
        // Same tick as an answer already given: repeat it without spending another attempt.
        if let Some(at) = watch.answered_at {
            if now.saturating_sub(at) < TICK_GAP_S {
                return Decision::Repair;
            }
        }
        watch
            .attempts
            .retain(|at| now.saturating_sub(*at) <= WINDOW_S);
        watch.attempts.push(now);
        if watch.attempts.len() >= MAX_REPAIRS {
            watch.suppressed_until = now.saturating_add(SUPPRESS_S);
            watch.attempts.clear();
            watch.answered_at = None;
            Decision::JustSuppressed
        } else {
            watch.answered_at = Some(now);
            Decision::Repair
        }
    }

    /// The value matches the persisted intent again: the next mismatch is a new observation.
    pub fn cleared(&self, id: &'static str) {
        let mut map = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(watch) = map.get_mut(id) {
            watch.answered_at = None;
        }
    }
}
