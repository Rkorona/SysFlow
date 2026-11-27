use std::process::Command;
use std::time::Instant;

use serde_json::{json, Map, Value};

use crate::config::ConfigStore;
use crate::ctx::Ctx;
use crate::envelope::PROTOCOL_VERSION;
use crate::error::{Error, ErrorCode, Result};
use crate::event::{EventStore, NewSystemEvent};
use crate::log::Logger;
use crate::module::{persistable, ApplyResult, Module};
use crate::safe;
use crate::schema::{ItemType, Schema};
use crate::util::{now_secs, parse_duration};

mod boot;
mod daemon;

fn err_json(e: &Error) -> Value {
    json!({ "code": e.code.as_str(), "msg": e.msg })
}

/// Store `v` under `key`. Objects (cluster/cpu/app scoped items) are merged entry by entry so
/// persisting cluster 4 keeps what was persisted earlier for cluster 0; everything else replaces.
fn merge_persisted(o: &mut Map<String, Value>, key: &str, v: &Value) {
    if let (Some(Value::Object(old)), Value::Object(new)) = (o.get_mut(key), v) {
        for (k, nv) in new {
            old.insert(k.clone(), nv.clone());
        }
        return;
    }
    o.insert(key.to_string(), v.clone());
}

/// `base` overlaid with `over`: objects merge key by key, everything else is replaced.
fn overlay(base: &Map<String, Value>, over: &Map<String, Value>) -> Map<String, Value> {
    let mut out = base.clone();
    for (k, v) in over {
        merge_persisted(&mut out, k, v);
    }
    out
}

/// Owns the modules and the config; every CLI command goes through here.
pub struct Registry {
    ctx: Ctx,
    modules: Vec<Box<dyn Module>>,
    store: ConfigStore,
    log: Logger,
    events: EventStore,
}

impl Registry {
    pub fn ctx(&self) -> &Ctx {
        &self.ctx
    }

    pub fn new(ctx: Ctx, modules: Vec<Box<dyn Module>>) -> Self {
        let store = ConfigStore::new(ctx.config_path());
        let log = Logger::new(ctx.log_path());
        let events = EventStore::new(&ctx.data_dir);
        Registry {
            ctx,
            modules,
            store,
            log,
            events,
        }
    }

    pub fn store(&self) -> &ConfigStore {
        &self.store
    }

    pub fn log_error(&self, e: &Error) {
        self.log.error(&format!("{}: {}", e.code.as_str(), e.msg));
    }

    fn find(&self, id: &str) -> Result<&dyn Module> {
        self.modules
            .iter()
            .find(|m| m.id() == id)
            .map(|m| &**m)
            .ok_or_else(|| Error::invalid_arg(format!("unknown module: {id}")))
    }

    fn find_supported(&self, id: &str) -> Result<&dyn Module> {
        let m = self.find(id)?;
        let caps = m.probe();
        if !caps.supported {
            return Err(Error::unsupported(caps.reason.unwrap_or_else(|| {
                format!("module {id} is not supported on this device")
            })));
        }
        Ok(m)
    }

    fn emit_event(&self, event: NewSystemEvent) {
        if let Err(e) = self.events.emit_sanitized(event) {
            self.log.warn(&format!("event emission failed: {e}"));
        }
    }

    // ---- read-only commands -------------------------------------------------

    pub fn caps(&self) -> Result<Value> {
        let cfg = self.store.load()?;
        let mut mods = Map::new();
        for m in &self.modules {
            mods.insert(m.id().to_string(), m.probe().to_json());
        }
        Ok(json!({
            "sysflow_version": crate::VERSION,
            "protocol": PROTOCOL_VERSION,
            "safe_mode": cfg.safe.safe_mode,
            "modules": mods,
        }))
    }

    /// Static platform information retained from SoCFlow's `info` command.
    /// Missing Android properties are returned as empty strings so this remains harmless in host tests.
    pub fn info(&self) -> Value {
        fn property(name: &str) -> String {
            Command::new("getprop")
                .arg(name)
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                .unwrap_or_default()
        }
        let kernel_release = Command::new("uname")
            .arg("-r")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .unwrap_or_default();
        json!({
            "soc_manufacturer": property("ro.soc.manufacturer"),
            "soc_model": property("ro.soc.model"),
            "board_platform": property("ro.board.platform"),
            "hardware": property("ro.hardware"),
            "kernel_release": kernel_release,
            "android_release": property("ro.build.version.release"),
            "device_model": property("ro.product.model"),
            "device_codename": property("ro.product.device"),
            "product_device": property("ro.product.name"),
            "primary_abi": property("ro.product.cpu.abi"),
        })
    }

    pub fn snapshot(&self) -> Result<Value> {
        let cfg = self.store.load()?;
        let mut mods = Map::new();
        let mut errors = Map::new();
        for m in &self.modules {
            if !m.in_snapshot() || !m.probe().supported {
                continue;
            }
            match m.collect(None) {
                Ok(v) => {
                    mods.insert(m.id().to_string(), v);
                }
                Err(e) => {
                    errors.insert(m.id().to_string(), err_json(&e));
                }
            }
        }
        Ok(json!({
            "ts": now_secs(),
            "safe_mode": cfg.safe.safe_mode,
            "modules": mods,
            "settings": cfg.modules,
            "defaults": cfg.defaults_snapshot,
            "errors": errors,
        }))
    }

    pub fn schema(&self, id: &str) -> Result<Value> {
        let m = self.find(id)?;
        let caps = m.probe();
        let schema = if caps.supported {
            m.schema()
        } else {
            Schema::unsupported(id)
        };
        let mut v = serde_json::to_value(&schema).map_err(|e| Error::io(e.to_string()))?;
        if let (Some(reason), Some(o)) = (caps.reason.as_ref(), v.as_object_mut()) {
            o.insert("reason".to_string(), json!(reason));
        }
        Ok(v)
    }

    pub fn get(&self, id: &str, keys: Option<&[String]>) -> Result<Value> {
        let m = self.find_supported(id)?;
        if let Some(keys) = keys {
            let schema = m.schema();
            for k in keys {
                if !schema.items.iter().any(|i| &i.id == k) {
                    return Err(Error::invalid_arg(format!("unknown key: {k}")));
                }
            }
        }
        m.collect(keys)
    }

    /// Read-only drift check for the enforcement loop (phase 2 daemon): for every module with
    /// persisted settings, return the keys whose live value no longer satisfies them. Nothing is
    /// written and no module state is advanced, so this is safe to run on a short interval.
    pub fn check(&self) -> Result<Value> {
        let cfg = self.store.load()?;
        let mut drift = Map::new();
        let mut errors = Map::new();
        for m in &self.modules {
            if !m.probe().supported {
                continue;
            }
            let settings = match cfg.modules.get(m.id()).and_then(|v| v.as_object()) {
                Some(s) if !s.is_empty() => s.clone(),
                _ => continue,
            };
            match m.drift(&settings) {
                Ok(keys) if !keys.is_empty() => {
                    drift.insert(m.id().to_string(), json!(keys));
                }
                Ok(_) => {}
                Err(e) => {
                    errors.insert(m.id().to_string(), err_json(&e));
                }
            }
        }
        let clean = drift.is_empty() && errors.is_empty();
        Ok(json!({ "clean": clean, "drift": drift, "errors": errors }))
    }

    pub fn history(&self, id: &str, since: &str, step: Option<&str>) -> Result<Value> {
        self.find(id)?;
        let since_d = parse_duration(since)?;
        let step_s = match step {
            Some(s) => parse_duration(s)?.as_secs(),
            None => std::cmp::max(60, since_d.as_secs() / 288),
        };
        if step_s == 0 {
            return Err(Error::invalid_arg("step must be greater than zero"));
        }
        // The daemon records samples; a stopped daemon simply means the series stops growing.
        let store = crate::history::HistoryStore::new(self.ctx.data_dir.join("history"));
        let cutoff = crate::history::now_secs().saturating_sub(since_d.as_secs());
        let mut value = store.series(id, cutoff, step_s)?;
        let running = self
            .daemon_status()
            .ok()
            .and_then(|status| status.get("running").and_then(Value::as_bool))
            .unwrap_or(false);
        value["daemon_running"] = json!(running);
        Ok(value)
    }

    /// Reapply only persisted settings that have drifted. The CPU module is also ticked while
    /// adaptive control is enabled, even when there is no drift, so load/thermal/battery/PSI
    /// hysteresis advances at the configured guard interval.
    pub fn enforce(&self) -> Result<Value> {
        // The rescue marker is checked before the config is loaded on purpose: it exists for the
        // case where the config cannot be trusted, so a load error must not pre-empt the rescue.
        if safe::rescue_marker_present(&self.ctx) {
            return Ok(json!({ "skipped": safe::RESCUE_REASON }));
        }
        let cfg = self.store.load()?;
        if let Some(reason) = safe::block_reason(&cfg, &self.ctx) {
            return Ok(json!({ "skipped": reason }));
        }
        let mut applied = Map::new();
        let mut skipped = Map::new();
        for module in &self.modules {
            if !module.probe().supported {
                continue;
            }
            let id = module.id();
            let settings = match cfg.modules.get(id).and_then(Value::as_object) {
                Some(values) if !values.is_empty() => values.clone(),
                _ => continue,
            };
            let drift = match module.drift(&settings) {
                Ok(keys) => keys,
                Err(e) => {
                    skipped.insert(id.to_string(), json!(e.msg));
                    self.emit_event(
                        NewSystemEvent::new(
                            id,
                            "enforcement",
                            "drift_check_failed",
                            "warning",
                            format!("{id} drift check failed"),
                        )
                        .message(&e.msg)
                        .metadata(json!({ "error_code": e.code.as_str() }))
                        .dedupe(format!("{id}.drift_check_failed"), 600),
                    );
                    continue;
                }
            };
            let adaptive_tick =
                id == "cpu" && settings.get("cpu.adaptive").and_then(Value::as_bool) == Some(true);
            if drift.is_empty() && !adaptive_tick {
                continue;
            }
            let clean = module.sanitize_boot(&settings);
            if clean.is_empty() {
                if !drift.is_empty() {
                    self.emit_event(
                        NewSystemEvent::new(
                            id,
                            "configuration",
                            "stale_settings_dropped",
                            "warning",
                            format!("{id} persisted settings unusable"),
                        )
                        .message(format!(
                            "All drifted settings failed validation on current hardware: {}",
                            drift.join(", ")
                        ))
                        .metadata(json!({ "dropped_keys": drift }))
                        .dedupe(format!("{id}.stale_settings_dropped"), 600),
                    );
                }
                continue;
            }
            if let Err(e) = module.schema().validate_request(&clean) {
                skipped.insert(id.to_string(), json!(e.msg));
                self.emit_event(
                    NewSystemEvent::new(
                        id,
                        "enforcement",
                        "enforcement_validation_failed",
                        "warning",
                        format!("{id} enforcement skipped"),
                    )
                    .message(&e.msg)
                    .metadata(json!({ "drift": drift, "error_code": e.code.as_str() }))
                    .dedupe(format!("{id}.enforcement_validation_failed"), 600),
                );
                continue;
            }
            let started = Instant::now();
            match module.apply_with_settings(&clean, &clean) {
                Ok(result) => {
                    // The immediate read-back can differ for keys whose value is allowed to lag: an
                    // offline CPU core cannot join a cpuset, a kernel floor can outrank `min_freq`.
                    // Re-run the module's own tolerance check and only report what it still calls
                    // drift, otherwise every tick raised a warning for a known-good state.
                    let residual = module.drift(&clean).unwrap_or_default();
                    let mismatch: Vec<String> = result
                        .mismatch
                        .iter()
                        .filter(|key| residual.contains(key))
                        .cloned()
                        .collect();
                    // Only log when something was actually repaired: the adaptive tick applies every
                    // interval by design and used to fill the log with no-op lines.
                    if !drift.is_empty() {
                        self.log.info(&format!(
                            "daemon applied {} drift={:?} mismatch={:?} took={}ms",
                            id,
                            drift,
                            mismatch,
                            started.elapsed().as_millis()
                        ));
                    }
                    if !drift.is_empty() {
                        if mismatch.is_empty() {
                            self.emit_event(
                                NewSystemEvent::new(
                                    id,
                                    "enforcement",
                                    "drift_repaired",
                                    "notice",
                                    format!("{id} configuration drift repaired"),
                                )
                                .message(format!("Reapplied drifted items: {}", drift.join(", ")))
                                .metadata(json!({ "drift": drift, "mismatch": [] }))
                                .dedupe(format!("{id}.drift_repaired"), 600),
                            );
                        } else {
                            self.emit_event(
                                NewSystemEvent::new(
                                    id,
                                    "enforcement",
                                    "drift_repair_incomplete",
                                    "warning",
                                    format!("{id} drift repair incomplete"),
                                )
                                .message(format!(
                                    "Drifted: {}; remaining mismatch: {}",
                                    drift.join(", "),
                                    mismatch.join(", ")
                                ))
                                .metadata(json!({
                                    "drift": drift,
                                    "mismatch": mismatch
                                }))
                                .dedupe(format!("{id}.drift_repair_incomplete"), 600),
                            );
                        }
                    }
                    applied.insert(
                        id.to_string(),
                        json!({
                            "drift": drift,
                            "mismatch": mismatch,
                        }),
                    );
                }
                Err(e) => {
                    self.log
                        .error(&format!("daemon apply {} failed: {}", id, e));
                    let event_type = if drift.is_empty() {
                        "adaptive_tick_failed"
                    } else {
                        "drift_repair_failed"
                    };
                    self.emit_event(
                        NewSystemEvent::new(
                            id,
                            "enforcement",
                            event_type,
                            "error",
                            format!("{id} daemon enforcement failed"),
                        )
                        .message(&e.msg)
                        .metadata(json!({ "drift": drift, "error_code": e.code.as_str() }))
                        .dedupe(format!("{id}.{event_type}"), 600),
                    );
                    skipped.insert(id.to_string(), json!(e.msg));
                }
            }
        }
        Ok(json!({ "applied": applied, "skipped": skipped }))
    }

    /// Append a short caller-supplied note to the module log (compatibility with SoCFlow `note`).
    pub fn note(&self, message: &str) -> Result<Value> {
        let message = message.trim();
        if message.is_empty() {
            return Err(Error::invalid_arg("note message must not be empty"));
        }
        if message.chars().count() > 1024 {
            return Err(Error::invalid_arg(
                "note message must be at most 1024 characters",
            ));
        }
        self.log.info(&format!("note: {message}"));
        Ok(json!({ "logged": true }))
    }

    pub fn log_tail(&self, n: usize) -> Result<Value> {
        Ok(json!({ "lines": self.log.tail(n) }))
    }

    // ---- state-changing commands -------------------------------------------

    pub fn set(&self, id: &str, req: Value, persist: bool) -> Result<Value> {
        let m = self.find_supported(id)?;
        let req = req
            .as_object()
            .ok_or_else(|| Error::invalid_arg("set payload must be a JSON object"))?
            .clone();
        if req.is_empty() {
            return Err(Error::invalid_arg("set payload is empty"));
        }
        let cfg = self.store.load()?;
        if cfg.safe.safe_mode {
            return Err(Error::safe_mode("tuning is disabled while in safe mode"));
        }
        let schema = m.schema();
        schema.validate_request(&req)?;
        let changed_keys = req.keys().cloned().collect::<Vec<_>>();
        let all_ephemeral = req.keys().all(|k| {
            schema
                .items
                .iter()
                .find(|i| &i.id == k)
                .is_some_and(|i| !i.persist)
        });

        let res = match self.apply_inner(m, &req, persist) {
            Ok(res) => res,
            Err(e) => {
                if e.code != ErrorCode::InvalidArg {
                    self.emit_event(
                        NewSystemEvent::new(
                            id,
                            "configuration",
                            "settings_apply_failed",
                            "error",
                            format!("{id} settings apply failed"),
                        )
                        .message(&e.msg)
                        .metadata(json!({
                            "persist": persist,
                            "keys": changed_keys,
                            "error_code": e.code.as_str(),
                        })),
                    );
                }
                return Err(e);
            }
        };

        if all_ephemeral {
            if !res.mismatch.is_empty() {
                self.emit_event(
                    NewSystemEvent::new(
                        id,
                        "power",
                        "runtime_state_mismatch",
                        "warning",
                        format!("{id} runtime transition mismatch"),
                    )
                    .message(format!("Mismatch for: {}", res.mismatch.join(", ")))
                    .metadata(json!({
                        "keys": changed_keys,
                        "mismatch": res.mismatch,
                        "requested": res.requested,
                        "applied": res.applied,
                    }))
                    .dedupe(format!("{id}.runtime_mismatch"), 30),
                );
            }
        } else {
            let mode_label = if persist { "persisted" } else { "temporary" };
            let metadata = json!({
                "persist": persist,
                "keys": changed_keys,
                "requested": res.requested,
                "applied": res.applied,
                "mismatch": res.mismatch,
            });
            if !res.mismatch.is_empty() {
                self.emit_event(
                    NewSystemEvent::new(
                        id,
                        "configuration",
                        "settings_partially_applied",
                        "warning",
                        format!("{id} settings partially applied"),
                    )
                    .message(format!(
                        "Changed ({mode_label}): {}; mismatch: {}",
                        changed_keys.join(", "),
                        res.mismatch.join(", ")
                    ))
                    .metadata(metadata),
                );
            } else {
                let title = if persist {
                    format!("{id} settings updated")
                } else {
                    format!("{id} temporary settings applied")
                };
                self.emit_event(
                    NewSystemEvent::new(id, "configuration", "settings_applied", "info", title)
                        .message(format!(
                            "Updated ({mode_label}): {}",
                            changed_keys.join(", ")
                        ))
                        .metadata(metadata),
                );
            }
        }
        Ok(res.to_json())
    }

    /// Record defaults, apply, optionally persist. Persists only the values that
    /// were read back as requested; mismatched items are not written to config.
    fn apply_inner(
        &self,
        m: &dyn Module,
        req: &Map<String, Value>,
        persist: bool,
    ) -> Result<ApplyResult> {
        let cfg = self.store.load()?;
        let stored = cfg
            .modules
            .get(m.id())
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let settings = overlay(&stored, req);
        self.apply_inner_with(m, req, &settings, persist)
    }

    fn apply_inner_with(
        &self,
        m: &dyn Module,
        req: &Map<String, Value>,
        settings: &Map<String, Value>,
        persist: bool,
    ) -> Result<ApplyResult> {
        self.record_defaults(m, req)?;
        let started = Instant::now();
        let res = m.apply_with_settings(req, settings).map_err(|e| {
            self.log.error(&format!("apply {} failed: {}", m.id(), e));
            e
        })?;
        self.log.info(&format!(
            "apply {} requested={} applied={} mismatch={:?} took={}ms",
            m.id(),
            Value::Object(res.requested.clone()),
            Value::Object(res.applied.clone()),
            res.mismatch,
            started.elapsed().as_millis()
        ));
        if persist {
            let id = m.id().to_string();
            self.store.update(|c| {
                let entry = c
                    .modules
                    .entry(id)
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Some(o) = entry.as_object_mut() {
                    // Persist what was *requested* (it was read back as requested, otherwise it
                    // is in `mismatch`), not the whole read-back: a cluster-scoped item reads
                    // back every cluster, and freezing the ones the user never touched would
                    // pin whatever transient value the system had at that moment.
                    for (k, v) in &res.requested {
                        // Persist only the part that read back as requested. For a map-valued
                        // item (`wakelock.blocked`, a cpuset selection) that is decided per
                        // entry, so one rejected package no longer discards the whole request.
                        let applied = res.applied.get(k);
                        if let Some(kept) = persistable(v, applied) {
                            merge_persisted(o, k, &kept);
                        } else if m.persist_on_mismatch(k) {
                            merge_persisted(o, k, v);
                        }
                    }
                }
                Ok(())
            })?;
        }
        Ok(res)
    }

    /// Before the first change to an item, remember the value the system had.
    fn record_defaults(&self, m: &dyn Module, req: &Map<String, Value>) -> Result<()> {
        let id = m.id();
        let cfg = self.store.load()?;
        let known = cfg
            .defaults_snapshot
            .get(id)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let missing: Vec<String> = req
            .keys()
            .filter(|k| !known.contains_key(*k))
            .cloned()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let current = m.collect(Some(missing.as_slice()))?;
        let current = current.as_object().cloned().unwrap_or_default();
        let current = m.normalize_default(current);
        let id = id.to_string();
        self.store.update(|c| {
            let entry = c
                .defaults_snapshot
                .entry(id)
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(o) = entry.as_object_mut() {
                for k in &missing {
                    if !o.contains_key(k) {
                        if let Some(v) = current.get(k) {
                            o.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            Ok(())
        })
    }

    /// Replace the recorded first-run defaults for one module's items.
    ///
    /// `reset` restores exactly these values, but they are otherwise captured once — the first time
    /// an item is written — and never refreshed afterwards. A baseline recorded before a kernel or
    /// ROM update therefore keeps a floor the hardware no longer honours, and no client could correct
    /// it short of deleting `config.json`. This is that correction: the caller supplies the values
    /// the device reports now.
    ///
    /// Object values (cluster-scoped items) merge entry by entry, so recording cluster 4's floor
    /// keeps cluster 0's. Read-only items are rejected: the snapshot is applied through the module's
    /// write path, so a read-only key would be silently dropped on restore.
    pub fn set_defaults(&self, id: &str, values: Map<String, Value>) -> Result<Value> {
        let m = self.find_supported(id)?;
        if values.is_empty() {
            return Err(Error::invalid_arg("no defaults to record"));
        }
        let schema = m.schema();
        let writable: std::collections::BTreeSet<&str> = schema
            .items
            .iter()
            .filter(|item| item.ty != ItemType::Readonly)
            .map(|item| item.id.as_str())
            .collect();
        let unknown: Vec<&str> = values
            .keys()
            .map(String::as_str)
            .filter(|key| !writable.contains(key))
            .collect();
        if !unknown.is_empty() {
            return Err(Error::invalid_arg(format!(
                "cannot store defaults for non-writable items: {}",
                unknown.join(", ")
            )));
        }
        let module = id.to_string();
        let recorded = values.clone();
        self.store.update(|c| {
            let entry = c
                .defaults_snapshot
                .entry(module.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            if let Some(object) = entry.as_object_mut() {
                for (key, value) in &recorded {
                    merge_persisted(object, key, value);
                }
            }
            Ok(())
        })?;
        let summary = values
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        self.emit_event(
            NewSystemEvent::new(
                id,
                "configuration",
                "defaults_rebased",
                "info",
                format!("{id} defaults rebaselined"),
            )
            .message(format!("Recorded new defaults for {summary}."))
            .metadata(json!({ "items": values })),
        );
        Ok(json!({ "module": id, "defaults": values }))
    }

    /// Restore the default snapshot through `apply` (allowed in safe mode), then
    /// drop the module's persisted settings. The snapshot is kept if some item
    /// could not be restored, so a later reset can try again.
    ///
    /// The module's cleanup hook runs first: it clears module-local state (WALT
    /// originals, an active screen-off profile, adaptive counters) so the snapshot is not
    /// applied on top of stale dynamic state. The default hook is a no-op.
    pub fn reset(&self, id: &str) -> Result<Value> {
        let m = self.find_supported(id)?;
        let cfg = self.store.load()?;
        let snap = cfg
            .defaults_snapshot
            .get(id)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        // The settings are dropped a few lines below, so this is the last chance to tell the
        // module *which* rules it owns. Without them a reset can only undo "everything visible",
        // which also reverts rules this app never set.
        let persisted = cfg
            .modules
            .get(id)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        if let Err(e) = m.reset_with_settings(&persisted) {
            self.emit_event(
                NewSystemEvent::new(
                    id,
                    "configuration",
                    "module_reset_failed",
                    "error",
                    format!("{id} reset failed"),
                )
                .message(&e.msg)
                .metadata(json!({ "error_code": e.code.as_str() })),
            );
            return Err(e);
        }
        let snap = m.sanitize_boot(&snap);
        let mut restored = Map::new();
        let mut mismatch: Vec<String> = Vec::new();
        if !snap.is_empty() {
            // Settings are empty on purpose: reset means "defaults", not "the persisted
            // module options that are about to be dropped".
            match m.apply_with_settings(&snap, &Map::new()) {
                Ok(res) => {
                    restored = res.applied;
                    mismatch = res.mismatch;
                }
                Err(e) => {
                    self.emit_event(
                        NewSystemEvent::new(
                            id,
                            "configuration",
                            "module_reset_failed",
                            "error",
                            format!("{id} reset failed"),
                        )
                        .message(&e.msg)
                        .metadata(json!({ "error_code": e.code.as_str() })),
                    );
                    return Err(e);
                }
            }
        }
        let id_owned = id.to_string();
        let clean = mismatch.is_empty();
        self.store.update(|c| {
            c.modules.remove(&id_owned);
            if clean {
                c.defaults_snapshot.remove(&id_owned);
            }
            Ok(())
        })?;
        self.log.info(&format!("reset {id} mismatch={mismatch:?}"));
        let restored_keys: Vec<&String> = restored.keys().collect();
        if clean {
            self.emit_event(
                NewSystemEvent::new(
                    id,
                    "configuration",
                    "module_reset",
                    "info",
                    format!("{id} restored to defaults"),
                )
                .message("Module settings reset to the saved default snapshot.")
                .metadata(json!({ "restored_keys": restored_keys }))
                .dedupe("module_reset", 5),
            );
        } else {
            self.emit_event(
                NewSystemEvent::new(
                    id,
                    "configuration",
                    "module_reset_partial",
                    "warning",
                    format!("{id} partially restored to defaults"),
                )
                .message(format!("Failed to restore: {}", mismatch.join(", ")))
                .metadata(json!({
                    "restored_keys": restored_keys,
                    "mismatch": mismatch,
                })),
            );
        }
        Ok(json!({ "restored": restored, "mismatch": mismatch }))
    }

    pub fn reset_all(&self) -> Result<Value> {
        let mut done = Map::new();
        let mut errors = Map::new();
        for m in &self.modules {
            if !m.probe().supported {
                continue;
            }
            match self.reset(m.id()) {
                Ok(v) => {
                    done.insert(m.id().to_string(), v);
                }
                Err(e) => {
                    errors.insert(m.id().to_string(), err_json(&e));
                }
            }
        }
        Ok(json!({ "reset": done, "errors": errors }))
    }
}
