//! Memory module — telemetry first.
//!
//! **Scope** (`docs/mem-module-plan.md`): the read-only telemetry half — detection, `schema`,
//! `collect` (which is what `snapshot` returns) and the hidden `mem probe` report. Nothing here
//! writes a node. Tunables (`swappiness`, …) arrive in M3b and go through the CPU-style
//! `apply`/`drift`/`persist` path; until then `apply` rejects everything.

pub mod history;
pub mod probe;
pub mod tunables;

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::event::{EventStore, NewSystemEvent};
use sysflow_core::module::{ApplyResult, Caps, Module, Sample};
use sysflow_core::schema::Schema;
use sysflow_core::sysfs::Sysfs;

pub struct MemModule {
    fs: Sysfs,
    /// `None` in unit tests that only exercise detection: no baseline means no sampling.
    state_dir: Option<PathBuf>,
    /// Derived from `state_dir.parent()`, like the CPU and wake-lock modules.
    events: Option<EventStore>,
    /// Stops the daemon from repairing the same setting in a loop when something else keeps
    /// undoing it (`tunables::Guard`).
    guard: tunables::Guard,
}

impl MemModule {
    /// Module without a state directory: read/schema/probe only.
    pub fn new(fs: Sysfs) -> Self {
        MemModule {
            fs,
            state_dir: None,
            events: None,
            guard: tunables::Guard::default(),
        }
    }

    /// Production constructor: history baseline under `<data_dir>/state/mem.json`, events under
    /// `<data_dir>/events/`.
    pub fn with_state_dir(fs: Sysfs, state_dir: impl Into<PathBuf>) -> Self {
        let state_dir = state_dir.into();
        let events = state_dir.parent().map(EventStore::new);
        MemModule {
            fs,
            state_dir: Some(state_dir),
            events,
            guard: tunables::Guard::default(),
        }
    }

    /// Tell the user once that SysFlow gave up on a value, instead of silently losing the fight.
    fn emit_contested(&self, id: &'static str, live: i64) {
        let Some(events) = &self.events else {
            return;
        };
        let _ = events.emit(
            NewSystemEvent::new(
                "mem",
                "settings",
                "mem_setting_contested",
                "warning",
                "A memory setting keeps being reset",
            )
            .message(format!(
                "{id} was reset to {live} again; SysFlow will stop re-applying it for {} hours",
                tunables::SUPPRESS_S / 3600
            ))
            .metadata(json!({
                "item": id,
                "live": live,
                "window_s": tunables::WINDOW_S,
                "suppress_s": tunables::SUPPRESS_S,
            }))
            .dedupe(format!("mem.contested.{id}"), tunables::SUPPRESS_S),
        );
    }

    /// [`Module::drift`] with the clock passed in, so the repair guard can be tested tick by tick.
    fn drift_at(&self, settings: &Map<String, Value>, now: u64) -> Result<Vec<String>> {
        if settings.is_empty() {
            return Ok(Vec::new());
        }
        let specs = tunables::specs(&self.fs);
        let mut drifted = Vec::new();
        for (id, want) in settings {
            let Some(spec) = specs.iter().find(|spec| spec.id == id) else {
                continue;
            };
            let Ok(live) = self.fs.read_i64(spec.path) else {
                continue;
            };
            if Some(live) == want.as_i64() {
                self.guard.cleared(spec.id);
                continue;
            }
            match self.guard.check(spec.id, now) {
                tunables::Decision::Repair => drifted.push(id.clone()),
                tunables::Decision::Suppressed => {}
                tunables::Decision::JustSuppressed => self.emit_contested(spec.id, live),
            }
        }
        Ok(drifted)
    }

    /// Test hook: attach an event store so the pressure events can be asserted.
    #[cfg(test)]
    pub fn with_events(mut self, events: EventStore) -> Self {
        self.events = Some(events);
        self
    }
}

impl Module for MemModule {
    fn id(&self) -> &'static str {
        "mem"
    }

    fn probe(&self) -> Caps {
        let detected = probe::detect(&self.fs);
        if !detected.supported() {
            return Caps::unsupported(detected.unsupported_reason());
        }
        Caps::supported(&detected.features())
    }

    fn schema(&self) -> Schema {
        let detected = probe::detect(&self.fs);
        // An unsupported module advertises nothing at all: half a schema would make clients render
        // a page they cannot fill. Tunables only exist alongside the telemetry they modulate.
        let mut items = detected.items();
        if detected.supported() {
            items.extend(tunables::schema_items(&self.fs));
        }
        Schema {
            module: "mem".to_string(),
            supported: detected.supported(),
            items,
        }
    }

    /// One pass over the sources. `schema` advertises exactly the keys this can produce, because
    /// both halves read the same [`probe::Detected`] judgement.
    fn collect(&self, keys: Option<&[String]>) -> Result<Value> {
        let detected = probe::detect(&self.fs);
        if !detected.supported() {
            return Err(Error::unsupported(detected.unsupported_reason()));
        }
        let mut all = probe::readings(&detected);
        // The writable half is part of `snapshot` too: clients must be able to show the current
        // value of every item the schema advertises.
        all.extend(tunables::values(&self.fs));
        let selected = match keys {
            None => all,
            Some(keys) => {
                let mut out = Map::new();
                for key in keys {
                    if let Some(value) = all.get(key) {
                        out.insert(key.clone(), value.clone());
                    }
                }
                out
            }
        };
        Ok(Value::Object(selected))
    }

    fn apply(&self, req: &Map<String, Value>) -> Result<ApplyResult> {
        tunables::apply(&self.fs, req)
    }

    /// Compare the persisted intent with the live sysctl, but stop fighting when something else
    /// keeps winning: after `MAX_REPAIRS` repairs inside `WINDOW_S`, the item is left alone for
    /// `SUPPRESS_S` and the user is told once, instead of the daemon rewriting it every tick.
    fn drift(&self, settings: &Map<String, Value>) -> Result<Vec<String>> {
        self.drift_at(settings, sysflow_core::history::now_secs())
    }

    fn sanitize_boot(&self, req: &Map<String, Value>) -> Map<String, Value> {
        let mut out = req.clone();
        tunables::sanitize(&self.fs, &mut out);
        out
    }

    /// Telemetry is cheap (six small `/proc` reads plus one `list_dir`), so `snapshot` includes it.
    fn in_snapshot(&self) -> bool {
        true
    }

    /// Interval sample: instantaneous values every time, deltas only against a baseline from the
    /// same boot (see `history.rs`).
    fn sample(&self) -> Option<Result<Sample>> {
        let state_dir = self.state_dir.as_deref()?;
        let now = sysflow_core::history::now_secs();
        history::sample(&self.fs, state_dir, now, self.events.as_ref()).map(Ok)
    }

    /// Five minutes: delta series stay meaningful and 30 days fit inside the history line cap.
    fn sample_interval(&self) -> Option<Duration> {
        self.state_dir
            .as_ref()
            .map(|_| Duration::from_secs(history::SAMPLE_INTERVAL_S))
    }
}

#[cfg(test)]
mod tests;
