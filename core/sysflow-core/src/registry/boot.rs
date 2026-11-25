//! Boot-time apply, recovery backups and the safe-mode gate.
//!
//! Split out of `registry.rs`: these commands all mutate boot/safe state and share the same
//! "isolate the configuration when the device stops booting" contract, which is worth reading in
//! one place. They remain `Registry` methods so the file is a view of the registry, not a peer.

use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::event::NewSystemEvent;
use crate::safe::{self, BootDecision};

use super::{err_json, Registry};

impl Registry {
    // ---- recovery, boot & safe mode -----------------------------------------

    pub fn recovery_status(&self) -> Result<Value> {
        let cfg = self.store.load()?;
        let backup = cfg.recovery_backup.as_ref();
        let corrupt_path = self.ctx.config_path().with_extension("json.corrupt");
        Ok(json!({
            "has_backup": backup.map(|m| !m.is_empty()).unwrap_or(false),
            "backup_modules": backup.map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),
            "has_corrupt_config": corrupt_path.is_file(),
            "safe_mode": cfg.safe.safe_mode,
            "boot_count": cfg.safe.boot_count,
            "threshold": cfg.safe.threshold,
        }))
    }

    /// Restore the isolated tuning profile to the active config. It deliberately does not leave
    /// safe mode or apply sysfs writes; the user must explicitly clear safe mode afterwards.
    pub fn recovery_restore(&self) -> Result<Value> {
        let restored_modules = self.store.update(|cfg| {
            let backup = cfg
                .recovery_backup
                .take()
                .ok_or_else(|| Error::invalid_arg("no recovery backup is available"))?;
            let modules = backup.keys().cloned().collect::<Vec<_>>();
            cfg.modules = backup;
            Ok(modules)
        })?;
        self.log.info(&format!(
            "recovery restored {} module configuration(s); safe mode unchanged",
            restored_modules.len()
        ));
        self.emit_event(
            NewSystemEvent::new(
                "system",
                "safety",
                "recovery_restored",
                "notice",
                "Isolated configuration restored",
            )
            .message(format!(
                "Restored isolated settings for: {}. Safe mode remains active until cleared.",
                if restored_modules.is_empty() {
                    "none".to_string()
                } else {
                    restored_modules.join(", ")
                }
            ))
            .metadata(json!({ "modules": restored_modules })),
        );
        self.recovery_status()
    }

    pub fn recovery_clear(&self) -> Result<Value> {
        let had_backup = self.store.update(|cfg| {
            let had = cfg
                .recovery_backup
                .take()
                .map(|m| !m.is_empty())
                .unwrap_or(false);
            Ok(had)
        })?;
        self.log.info("recovery backup discarded by user");
        if had_backup {
            self.emit_event(
                NewSystemEvent::new(
                    "system",
                    "safety",
                    "recovery_discarded",
                    "info",
                    "Isolated configuration discarded",
                )
                .message("User discarded the isolated recovery configuration backup."),
            );
        }
        self.recovery_status()
    }

    pub fn recovery_clear_corrupt(&self) -> Result<Value> {
        let path = self.ctx.config_path().with_extension("json.corrupt");
        match std::fs::remove_file(&path) {
            Ok(()) => {
                self.log.info("corrupt config backup removed by user");
                self.emit_event(
                    NewSystemEvent::new(
                        "system",
                        "safety",
                        "corrupt_config_cleared",
                        "info",
                        "Corrupt config backup deleted",
                    )
                    .message("User deleted the isolated config.json.corrupt backup."),
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(format!("remove corrupt config backup: {e}"))),
        }
        self.recovery_status()
    }

    // ---- boot & safe mode ---------------------------------------------------

    pub fn boot_status(&self) -> Result<Value> {
        let cfg = self.store.load()?;
        Ok(json!({ "enabled": cfg.boot.enabled, "delay_s": cfg.boot.delay_s }))
    }

    pub fn boot_configure(&self, enabled: Option<bool>, delay_s: Option<u32>) -> Result<Value> {
        if let Some(delay) = delay_s {
            if delay > 300 {
                return Err(Error::invalid_arg(
                    "boot delay must be between 0 and 300 seconds",
                ));
            }
        }
        let (final_enabled, final_delay) = self.store.update(|cfg| {
            if let Some(v) = enabled {
                cfg.boot.enabled = v;
            }
            if let Some(v) = delay_s {
                cfg.boot.delay_s = v;
            }
            Ok((cfg.boot.enabled, cfg.boot.delay_s))
        })?;
        if enabled.is_some() || delay_s.is_some() {
            self.emit_event(
                NewSystemEvent::new(
                    "system",
                    "configuration",
                    "boot_config_updated",
                    "info",
                    "Boot apply settings updated",
                )
                .message(format!(
                    "Boot apply: {}, delay: {}s.",
                    if final_enabled { "enabled" } else { "disabled" },
                    final_delay
                ))
                .metadata(json!({ "enabled": final_enabled, "delay_s": final_delay })),
            );
        }
        if enabled == Some(false) {
            // The old SoCFlow UI required boot-time apply for continuous guard. Disabling boot
            // apply therefore also prevents an already-running guard from rewriting settings.
            let _ = self.daemon_configure(Some(false), None)?;
        }
        self.boot_status()
    }

    fn apply_walt_guard_only(&self, modules: &Map<String, Value>) -> Result<Value> {
        let settings = modules
            .get("cpu")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let Some(enabled) = settings.get("cpu.walt_guard").and_then(Value::as_bool) else {
            return Ok(json!({ "skipped": "boot_disabled", "walt_guard": "not_configured" }));
        };
        if safe::rescue_marker_present(&self.ctx) {
            return Ok(json!({ "skipped": "disable_tuning", "walt_guard": "not_applied" }));
        }
        let module = match self.find_supported("cpu") {
            Ok(module) => module,
            Err(e) => {
                self.emit_event(
                    NewSystemEvent::new(
                        "cpu",
                        "configuration",
                        "walt_guard_boot_failed",
                        "error",
                        "CPU WALT guard boot apply failed",
                    )
                    .message(&e.msg)
                    .metadata(json!({ "enabled": enabled, "error_code": e.code.as_str() })),
                );
                return Err(e);
            }
        };
        let mut request = Map::new();
        request.insert("cpu.walt_guard".to_string(), Value::Bool(enabled));
        match module.apply_with_settings(&request, &settings) {
            Ok(result) => {
                self.log.info(&format!(
                    "boot WALT guard applied enabled={enabled} mismatch={:?}",
                    result.mismatch
                ));
                if !result.mismatch.is_empty() {
                    self.emit_event(
                        NewSystemEvent::new(
                            "cpu",
                            "configuration",
                            "walt_guard_boot_mismatch",
                            "warning",
                            "CPU WALT guard mismatch at boot",
                        )
                        .message("Failed to verify WALT write guard permissions at boot.")
                        .metadata(json!({ "enabled": enabled, "mismatch": result.mismatch })),
                    );
                } else if enabled {
                    self.emit_event(
                        NewSystemEvent::new(
                            "cpu",
                            "configuration",
                            "walt_guard_boot_applied",
                            "info",
                            "CPU WALT guard restored at boot",
                        )
                        .message("Boot-time tuning is disabled; applied WALT write guard only.")
                        .metadata(json!({ "enabled": true })),
                    );
                }
                Ok(json!({ "boot_disabled": true, "walt_guard": result.to_json() }))
            }
            Err(e) => {
                self.emit_event(
                    NewSystemEvent::new(
                        "cpu",
                        "configuration",
                        "walt_guard_boot_failed",
                        "error",
                        "CPU WALT guard boot apply failed",
                    )
                    .message(&e.msg)
                    .metadata(json!({ "enabled": enabled, "error_code": e.code.as_str() })),
                );
                Err(e)
            }
        }
    }

    pub fn apply_boot(&self, no_delay: bool) -> Result<Value> {
        if safe::rescue_marker_present(&self.ctx) {
            self.log
                .warn("apply-boot skipped: disable_tuning marker present");
            return Ok(json!({ "skipped": "disable_tuning" }));
        }
        let cfg = self.store.load()?;
        if cfg.safe.safe_mode {
            return Ok(json!({ "skipped": "safe_mode" }));
        }
        if !cfg.boot.enabled {
            // SoCFlow's WALT guard is independent of ordinary boot-time tuning. Preserve that
            // behavior when users disable the main boot-apply switch.
            return self.apply_walt_guard_only(&cfg.modules);
        }
        if !no_delay && cfg.boot.delay_s > 0 {
            std::thread::sleep(Duration::from_secs(u64::from(cfg.boot.delay_s)));
        }
        let was_safe_mode = cfg.safe.safe_mode;
        let (decision, isolated_modules, boot_count, threshold) = self.store.update(|c| {
            let decision = safe::begin_boot(c);
            let mut isolated = Vec::new();
            // After repeated unconfirmed boots, isolate the persisted tuning profile instead of
            // deleting it. The user can explicitly restore or discard it from Recovery.
            if decision == BootDecision::Skip("safe_mode")
                && c.safe.safe_mode
                && c.recovery_backup.is_none()
                && !c.modules.is_empty()
            {
                isolated = c.modules.keys().cloned().collect();
                c.recovery_backup = Some(c.modules.clone());
                c.modules.clear();
            }
            Ok((decision, isolated, c.safe.boot_count, c.safe.threshold))
        })?;
        if let BootDecision::Skip(reason) = decision {
            if reason == "safe_mode" && !was_safe_mode {
                let message = if isolated_modules.is_empty() {
                    "Repeated unconfirmed boots triggered automatic safe mode.".to_string()
                } else {
                    format!(
                        "Repeated unconfirmed boots triggered automatic safe mode; isolated configuration for: {}.",
                        isolated_modules.join(", ")
                    )
                };
                self.emit_event(
                    NewSystemEvent::new(
                        "system",
                        "safety",
                        "safe_mode_entered",
                        "warning",
                        "Safe mode enabled",
                    )
                    .message(message)
                    .metadata(json!({
                        "boot_count": boot_count,
                        "threshold": threshold,
                        "isolated_modules": isolated_modules,
                    }))
                    .dedupe("safe_mode_entered", 5),
                );
            }
            self.log.warn(&format!("apply-boot skipped: {reason}"));
            return Ok(json!({ "skipped": reason }));
        }

        let cfg = self.store.load()?;
        let mut applied = Map::new();
        let mut errors = Map::new();
        for m in &self.modules {
            if !m.probe().supported {
                continue;
            }
            let id = m.id();
            // A new boot starts from clean module-local state: transient values (WALT
            // originals, an active screen-off profile, adaptive counters, settings applied
            // without --persist last boot) must not leak into this boot's persisted config.
            // This runs even when the module has no persisted settings.
            if let Err(e) = m.reset() {
                self.log
                    .warn(&format!("apply-boot {}: module reset failed: {}", id, e));
                self.emit_event(
                    NewSystemEvent::new(
                        id,
                        "lifecycle",
                        "boot_reset_failed",
                        "warning",
                        format!("{id} pre-boot cleanup failed"),
                    )
                    .message(&e.msg)
                    .metadata(json!({ "error_code": e.code.as_str() })),
                );
            }
            let saved = match cfg.modules.get(id).and_then(|v| v.as_object()) {
                Some(r) if !r.is_empty() => r.clone(),
                _ => continue,
            };
            let req = m.sanitize_boot(&saved);
            if req != saved {
                let dropped_keys: Vec<String> = saved
                    .iter()
                    .filter(|(k, v)| req.get(*k) != Some(*v))
                    .map(|(k, _)| k.clone())
                    .collect();
                self.log.warn(&format!(
                    "apply-boot {}: dropped saved entries that no longer apply: {}",
                    id,
                    Value::Object(saved.clone())
                ));
                self.emit_event(
                    NewSystemEvent::new(
                        id,
                        "configuration",
                        "stale_settings_dropped",
                        "warning",
                        format!("{id} stale boot settings dropped"),
                    )
                    .message(format!(
                        "Dropped incompatible saved entries: {}",
                        dropped_keys.join(", ")
                    ))
                    .metadata(json!({ "dropped_keys": dropped_keys })),
                );
            }
            if req.is_empty() {
                continue;
            }
            let req_keys: Vec<String> = req.keys().cloned().collect();
            let outcome = m
                .schema()
                .validate_request(&req)
                .and_then(|_| self.apply_inner_with(&**m, &req, &req, false));
            match outcome {
                Ok(res) => {
                    if !res.mismatch.is_empty() {
                        self.emit_event(
                            NewSystemEvent::new(
                                id,
                                "configuration",
                                "boot_apply_partial",
                                "warning",
                                format!("{id} boot settings partially applied"),
                            )
                            .message(format!(
                                "Mismatch after boot apply: {}",
                                res.mismatch.join(", ")
                            ))
                            .metadata(json!({
                                "keys": req_keys,
                                "mismatch": res.mismatch,
                            })),
                        );
                    } else {
                        self.emit_event(
                            NewSystemEvent::new(
                                id,
                                "configuration",
                                "boot_settings_applied",
                                "info",
                                format!("{id} boot settings applied"),
                            )
                            .message(format!("Applied at boot: {}", req_keys.join(", ")))
                            .metadata(json!({ "keys": req_keys })),
                        );
                    }
                    applied.insert(id.to_string(), res.to_json());
                }
                Err(e) => {
                    self.log.error(&format!("apply-boot {} failed: {}", id, e));
                    self.emit_event(
                        NewSystemEvent::new(
                            id,
                            "configuration",
                            "boot_apply_failed",
                            "error",
                            format!("{id} boot apply failed"),
                        )
                        .message(&e.msg)
                        .metadata(json!({
                            "keys": req_keys,
                            "error_code": e.code.as_str(),
                        })),
                    );
                    errors.insert(id.to_string(), err_json(&e));
                }
            }
        }
        Ok(json!({ "applied": applied, "errors": errors }))
    }

    pub fn boot_confirm(&self) -> Result<Value> {
        self.store.update(|c| {
            safe::confirm_boot(c);
            Ok(())
        })?;
        self.emit_event(
            NewSystemEvent::new(
                "system",
                "lifecycle",
                "boot_confirmed",
                "info",
                "Boot confirmed",
            )
            .message("The module confirmed that the current boot is stable.")
            .dedupe("boot_confirmed", 5),
        );
        Ok(json!({ "boot_count": 0 }))
    }

    pub fn safe_status(&self) -> Result<Value> {
        let cfg = self.store.load()?;
        Ok(json!({
            "safe_mode": cfg.safe.safe_mode,
            "boot_count": cfg.safe.boot_count,
            "threshold": cfg.safe.threshold,
            "rescue_marker": safe::rescue_marker_present(&self.ctx),
        }))
    }

    pub fn safe_clear(&self) -> Result<Value> {
        self.store.update(|c| {
            safe::clear(c);
            Ok(())
        })?;
        self.log.info("safe mode cleared by user");
        self.emit_event(
            NewSystemEvent::new(
                "system",
                "safety",
                "safe_mode_cleared",
                "info",
                "Safe mode cleared",
            )
            .message("The user cleared SysFlow safe mode.")
            .dedupe("safe_mode_cleared", 5),
        );
        self.safe_status()
    }
}
