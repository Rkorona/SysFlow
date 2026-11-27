use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::error::Result;
use crate::schema::Schema;

/// One sample for the history ring buffer (phase 2): metric id -> value.
pub type Sample = Map<String, Value>;

/// What `probe()` found out about this device.
#[derive(Clone, Debug)]
pub struct Caps {
    pub supported: bool,
    pub features: Vec<String>,
    pub reason: Option<String>,
}

impl Caps {
    pub fn supported(features: &[&str]) -> Self {
        Caps {
            supported: true,
            features: features.iter().map(|s| s.to_string()).collect(),
            reason: None,
        }
    }

    pub fn unsupported(reason: impl Into<String>) -> Self {
        Caps {
            supported: false,
            features: Vec::new(),
            reason: Some(reason.into()),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut o = json!({ "supported": self.supported, "features": self.features });
        if let Some(r) = &self.reason {
            o["reason"] = json!(r);
        }
        o
    }
}

/// Is `requested` satisfied by `applied`? Objects are compared as subsets, so a
/// cluster-scoped request for one cluster is satisfied by a full read-back.
pub fn satisfied(requested: &Value, applied: &Value) -> bool {
    match (requested, applied) {
        (Value::Object(r), Value::Object(a)) => r
            .iter()
            .all(|(k, rv)| a.get(k).map(|av| satisfied(rv, av)).unwrap_or(false)),
        _ => requested == applied,
    }
}

/// The part of `requested` that `applied` confirms, for persistence.
///
/// Map-valued items are what this exists for: an app-scoped switch is *one* item holding many
/// entries, so a single rejected package must not throw away the rules that did apply. Scalars
/// behave exactly like [`satisfied`]: persisted when equal, dropped otherwise. Returns `None` when
/// nothing read back as requested, or when the module reported no value at all.
pub fn persistable(requested: &Value, applied: Option<&Value>) -> Option<Value> {
    let applied = applied?;
    match (requested, applied) {
        (Value::Object(wanted), Value::Object(seen)) => {
            let mut out = Map::new();
            for (key, value) in wanted {
                let Some(actual) = seen.get(key) else {
                    continue;
                };
                if let Some(kept) = persistable(value, Some(actual)) {
                    out.insert(key.clone(), kept);
                }
            }
            (!out.is_empty()).then_some(Value::Object(out))
        }
        _ => satisfied(requested, applied).then(|| requested.clone()),
    }
}

/// Result of `apply`: always the *read-back* state, never an echo of the request.
#[derive(Clone, Debug, Default)]
pub struct ApplyResult {
    pub applied: Map<String, Value>,
    pub requested: Map<String, Value>,
    pub mismatch: Vec<String>,
}

impl ApplyResult {
    /// Build from the request and what was read back afterwards; computes `mismatch`.
    pub fn from_readback(requested: &Map<String, Value>, applied: Map<String, Value>) -> Self {
        let mut mismatch = Vec::new();
        for (k, v) in requested {
            let ok = applied.get(k).map(|a| satisfied(v, a)).unwrap_or(false);
            if !ok {
                mismatch.push(k.clone());
            }
        }
        ApplyResult {
            applied,
            requested: requested.clone(),
            mismatch,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "applied": self.applied,
            "requested": self.requested,
            "mismatch": self.mismatch,
        })
    }
}

/// A feature area (cpu, battery, ...). Adding one = implement this + register it.
pub trait Module: Send + Sync {
    fn id(&self) -> &'static str;

    /// Detect what this device supports. Must be cheap and side-effect free.
    fn probe(&self) -> Caps;

    /// Describe tunable items. Options (frequency tables, governors, ...) are filled at runtime.
    fn schema(&self) -> Schema;

    /// Read current values, optionally only the given keys.
    fn collect(&self, keys: Option<&[String]>) -> Result<Value>;

    /// Apply already-validated values, then read back and return the real state.
    fn apply(&self, req: &Map<String, Value>) -> Result<ApplyResult>;

    /// Apply `req` knowing the module's persisted settings (a deep merge of the stored config
    /// with `req`). Modules whose live behaviour depends on settings outside the request (the
    /// CPU adaptive cap and screen-off profile read their own options this way) override it.
    /// The default ignores the extra context.
    fn apply_with_settings(
        &self,
        req: &Map<String, Value>,
        _settings: &Map<String, Value>,
    ) -> Result<ApplyResult> {
        self.apply(req)
    }

    /// Whether a requested value should be persisted even when its read-back differs.
    /// This is intended for controls whose desired state can exceed the currently effective
    /// state temporarily (for example cpuset selections containing offline CPU cores).
    fn persist_on_mismatch(&self, _item_id: &str) -> bool {
        false
    }

    /// Read-only check used by the enforcement loop (phase 2 daemon): return the keys whose
    /// current value does not satisfy the persisted settings. Must not write anything.
    fn drift(&self, settings: &Map<String, Value>) -> Result<Vec<String>> {
        if settings.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<String> = settings.keys().cloned().collect();
        let current = self.collect(Some(&keys))?;
        let current = current.as_object().cloned().unwrap_or_default();
        let mut out = Vec::new();
        for (k, v) in settings {
            let ok = current.get(k).map(|a| satisfied(v, a)).unwrap_or(false);
            if !ok {
                out.push(k.clone());
            }
        }
        Ok(out)
    }

    /// Turn "what the system has right now" into the value `reset` should restore, before it is
    /// stored as the default snapshot. Modules whose live value can be transient (boost / thermal
    /// caps) override this; the default keeps the value as read.
    fn normalize_default(&self, current: Map<String, Value>) -> Map<String, Value> {
        current
    }

    /// Drop persisted entries that can no longer be applied on this boot (kernel or ROM update
    /// changed a frequency table, a governor disappeared, ...), so one stale value does not
    /// stop the rest of the module's settings from being applied. The default keeps everything.
    fn sanitize_boot(&self, req: &Map<String, Value>) -> Map<String, Value> {
        req.clone()
    }

    /// Module-specific cleanup on reset. The registry already restores the default
    /// snapshot through `apply`, so most modules keep the default no-op.
    fn reset(&self) -> Result<()> {
        Ok(())
    }

    /// Module-specific cleanup on reset, with the settings that are about to be dropped.
    ///
    /// [`Module::reset`] cannot see the persisted config, so a module that restores *system* state
    /// must not use it to undo "everything it can see": that also removes rules the user set
    /// outside this app. The default ignores the extra context and delegates, which keeps modules
    /// whose reset only touches their own files unchanged.
    fn reset_with_settings(&self, _settings: &Map<String, Value>) -> Result<()> {
        self.reset()
    }

    /// Whether `snapshot` should include this module (the process module opts out: it is slow).
    fn in_snapshot(&self) -> bool {
        true
    }

    /// History sampling (phase 2). Only modules that need curves implement it.
    fn sample(&self) -> Option<Result<Sample>> {
        None
    }

    fn sample_interval(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subset_match_for_objects() {
        assert!(satisfied(&json!({"0": 1}), &json!({"0": 1, "4": 2})));
        assert!(!satisfied(&json!({"0": 1}), &json!({"0": 2, "4": 2})));
        assert!(!satisfied(&json!({"9": 1}), &json!({"0": 1})));
        assert!(satisfied(&json!([1, 0]), &json!([1, 0])));
        assert!(!satisfied(&json!([1, 0]), &json!([1, 1])));
    }

    #[test]
    fn readback_reports_mismatch() {
        let req = json!({"a": 1, "b": 2}).as_object().unwrap().clone();
        let applied = json!({"a": 1, "b": 3}).as_object().unwrap().clone();
        let r = ApplyResult::from_readback(&req, applied);
        assert_eq!(r.mismatch, vec!["b".to_string()]);
    }

    #[test]
    fn partial_map_readback_persists_only_what_applied() {
        let requested = json!({"wakelock.blocked": {"a": true, "b": true}});
        let applied = json!({"wakelock.blocked": {"a": true, "b": false}});
        assert_eq!(
            persistable(&requested, Some(&applied)),
            Some(json!({"wakelock.blocked": {"a": true}})),
            "the rejected package must not discard the accepted one"
        );
    }

    #[test]
    fn a_missing_readback_persists_nothing() {
        let requested = json!({"wakelock.blocked": {"a": true}});
        assert_eq!(persistable(&requested, None), None);
        assert_eq!(persistable(&json!(true), None), None);
    }

    #[test]
    fn scalar_items_persist_only_when_equal() {
        assert_eq!(
            persistable(&json!(true), Some(&json!(true))),
            Some(json!(true))
        );
        assert_eq!(persistable(&json!(true), Some(&json!(false))), None);
        assert_eq!(persistable(&json!(42), Some(&json!(42))), Some(json!(42)));
    }

    #[test]
    fn cluster_items_keep_the_requested_subset() {
        // The read-back covers every cluster; only what was asked for is persisted.
        let requested = json!({"cpu.freq": {"0": 100}});
        let applied = json!({"cpu.freq": {"0": 100, "4": 200}});
        assert_eq!(
            persistable(&requested, Some(&applied)),
            Some(json!({"cpu.freq": {"0": 100}}))
        );
    }
}
