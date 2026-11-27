//! Wake-lock module: kernel wakeup sources (read-only) plus AppOps `WAKE_LOCK` blocking.
//!
//! **Two halves with different safety postures:**
//!
//! * The **read** half (`kernel_sources`, `ranked`, `summary`) is telemetry: one kernel `seq_file`
//!   read, no writes, safe to put in `snapshot`.
//! * The **write** half (`wakelock.blocked`) changes per-app AppOps state. Blocking an app's wake
//!   locks can delay its notifications, so it is `risk: medium`, it is persisted and replayed at
//!   boot, and a hardcoded protection list refuses to touch dialer/telephony/SystemUI/`android`.
//!
//! Ranking is computed from **kernel counters**, not from the history file: per-source cumulative
//! milliseconds would otherwise have to be written as an unbounded number of history metrics. The
//! module keeps one baseline snapshot in `state/wakelock.json`, refreshes it on every `sample()`
//! (the daemon calls it every 5 minutes) and rolls it forward after 24 hours.
//!
//! `wakelock.blocked` is deliberately **absent from `snapshot`**: answering it costs a binder call,
//! and `snapshot` runs every couple of seconds. Clients ask for it explicitly
//! (`get --keys wakelock.blocked`). Userspace wake locks (`dumpsys power`) are also a separate verb —
//! `sysflow wakelock held` — for the same reason, as is `sysflow wakelock wakeup`.
//!
//! **Layout.** `window` owns the rolling counters and the reboot detection, `debugfs` the mount
//! gate, `appops` the write half, `policy` the protection rule (published as
//! `wakelock.protected`), `read` the JSON rendering, `state` the window file, and `power` the
//! injectable `dumpsys power` used by the long-held-lock watchdog in `sample()`.

mod appops;
mod debugfs;
mod policy;
mod power;
mod read;
mod state;
mod window;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::event::{EventStore, NewSystemEvent};
use sysflow_core::module::{ApplyResult, Caps, Module, Sample};
use sysflow_core::schema::{ItemType, Risk, Schema, SchemaItem, Scope};
use sysflow_core::sysfs::Sysfs;
use sysflow_core::wakeup_sources::{
    parse_kernel_sources, parse_power_wakelocks, parse_uptime, KernelSource, TimeUnit,
};

use appops::{AppOps, Runner};
use debugfs::{MountGate, DEBUGFS_ROOT, TABLE_PATH};
use policy::{is_package_name, is_protected, PROTECTED};
use power::Power;
use read::{ranked_json, sources_json, summary_json};
use state::StateFile;
use window::WindowState;

/// Kernel wakeup-source tables in probe order, with the unit each one reports.
const KERNEL_SOURCES: &[(&str, TimeUnit)] = &[
    (TABLE_PATH, TimeUnit::Millis),
    ("/proc/wakelocks", TimeUnit::Nanos),
];

const ITEM_SOURCES: &str = "wakelock.kernel_sources";
const ITEM_RANKED: &str = "wakelock.ranked";
const ITEM_SUMMARY: &str = "wakelock.summary";
const ITEM_PROTECTED: &str = "wakelock.protected";
const ITEM_BLOCKED: &str = "wakelock.blocked";
const ITEM_EXEMPT: &str = "wakelock.exempt";

/// Sampling cadence: wakeup sources change slowly and the table is a kernel seq_file.
const SAMPLE_SECS: u64 = 300;
/// A lock held longer than this is worth reporting: normal operations last seconds to minutes.
const LONG_HELD_MS: i64 = 30 * 60 * 1000;
/// One report per lock per this window, so a genuinely stuck app does not flood the journal.
const LONG_HELD_DEDUPE_S: u64 = 6 * 3_600;

pub struct WakelockModule {
    fs: Sysfs,
    state: StateFile,
    appops: AppOps,
    mount_gate: MountGate,
    power: Power,
    events: Option<EventStore>,
}

impl WakelockModule {
    pub fn new(fs: Sysfs, state_dir: impl Into<PathBuf>) -> Self {
        let state_dir = state_dir.into();
        let events = state_dir.parent().map(EventStore::new);
        WakelockModule {
            fs,
            state: StateFile::new(&state_dir),
            mount_gate: MountGate::new(&state_dir),
            power: Power::real(),
            events,
            appops: AppOps::real(),
        }
    }

    /// Test constructor: `runner` replaces `cmd appops`/`cmd package`.
    pub fn with_runner(fs: Sysfs, state_dir: impl Into<PathBuf>, runner: Arc<Runner>) -> Self {
        let state_dir = state_dir.into();
        WakelockModule {
            fs,
            state: StateFile::new(&state_dir),
            mount_gate: MountGate::new(&state_dir),
            power: Power::unavailable(),
            events: None,
            appops: AppOps::with_runner(runner),
        }
    }

    /// Test hook: replace the `dumpsys power` runner (watchdog tests).
    #[cfg(test)]
    pub fn with_power(mut self, power: Power) -> Self {
        self.power = power;
        self
    }

    /// Test hook: attach an event store so the watchdog output can be asserted.
    #[cfg(test)]
    pub fn with_events(mut self, events: EventStore) -> Self {
        self.events = Some(events);
        self
    }

    /// The first readable kernel table: path, unit and its contents.
    ///
    /// One read serves both `probe()` and `collect()`: the table is a kernel `seq_file`, and a
    /// `snapshot` polls every module every couple of seconds, so reading it twice would double the
    /// cost for no benefit.
    /// First readable table, or a message explaining why there is none.
    ///
    /// Uses a lossy read: a wakeup-source name with one malformed byte must not hide the whole
    /// table, which would look like "the device does not support this" instead of "one row is odd".
    fn read_table(&self) -> std::result::Result<(&'static str, TimeUnit, String), String> {
        // Keep the *first* failure: it is the debugfs path the user can act on, while the legacy
        // `/proc/wakelocks` fallback failing too is expected on modern kernels. `Error::msg` already
        // names the path, so it is not prefixed again.
        let mut first_error: Option<String> = None;
        for (path, unit) in KERNEL_SOURCES {
            match self.fs.read_lossy(path) {
                Ok(text) => return Ok((path, *unit, text)),
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e.msg);
                    }
                }
            }
        }
        Err(first_error.unwrap_or_else(|| "no wakeup-source node found".to_string()))
    }

    /// `(readable, sources, reason)` — `reason` explains an unreadable table.
    fn sources(&self) -> (bool, Vec<KernelSource>, Option<String>) {
        match self.read_table() {
            Ok((_, unit, text)) => (true, parse_kernel_sources(&text, unit), None),
            Err(reason) => {
                // Android does not mount debugfs by default; a root module has to do it before the
                // kernel wakeup-source table exists at all.
                match self.ensure_debugfs() {
                    Ok(true) => {
                        if let Ok((_, unit, text)) = self.read_table() {
                            return (true, parse_kernel_sources(&text, unit), None);
                        }
                        // The mount succeeded but the table is still unreadable (`CONFIG_PM_DEBUG`
                        // off, or SELinux denying the node). Record it, otherwise the 2 s overview
                        // poll would fork `mount` on every poll, forever.
                        self.mount_gate
                            .record_unreadable(sysflow_core::history::now_secs());
                        (
                            false,
                            Vec::new(),
                            Some(format!("{reason}（已挂载 debugfs，但仍不可读）")),
                        )
                    }
                    Ok(false) => (false, Vec::new(), Some(reason)),
                    Err(e) => (
                        false,
                        Vec::new(),
                        Some(format!("{reason}；挂载 debugfs 失败：{e}")),
                    ),
                }
            }
        }
    }

    /// Try `mount -t debugfs` once (and not again for an hour) when the table is missing.
    ///
    /// Only on Android: host tests must never attempt a real mount. The marker file lives next to the
    /// window state so `reset --all`/uninstall cleans it up with the rest.
    fn ensure_debugfs(&self) -> std::result::Result<bool, String> {
        if !cfg!(target_os = "android") {
            return Ok(false);
        }
        if !std::path::Path::new(DEBUGFS_ROOT).is_dir() {
            return Ok(false);
        }
        let now = sysflow_core::history::now_secs();
        if !self.mount_gate.should_attempt(now) {
            return Ok(false);
        }
        // Only attempts that left the table unreadable are rate-limited: a successful mount might
        // live in this process's mount namespace only, and the daemon (a different process) must
        // still be able to mount its own. Re-mounting is idempotent and stops as soon as the table
        // becomes readable.
        match sysflow_core::command::run(
            "mount",
            &["-t", "debugfs", "debugfs", DEBUGFS_ROOT],
            std::time::Duration::from_secs(5),
        ) {
            Ok(_) => Ok(true),
            Err(e) => {
                self.mount_gate.record_unreadable(now);
                Err(e.msg)
            }
        }
    }

    /// Load the stored window and fold in this observation. Callers persist the result.
    ///
    /// A reboot restarts the window instead of leaving it dead: the state file survives reboots
    /// while the kernel counters do not, so without this the ranking reported zeros for up to a
    /// day and the page explained it as "the device was idle".
    fn observe(&self, sources: &[KernelSource]) -> WindowState {
        let now = sysflow_core::history::now_secs();
        let boot_id = self.boot_id();
        let mut state = self.state.load();
        state.advance(sources, now, self.uptime_secs(), boot_id.as_deref());
        state
    }

    /// Seconds since boot: the reboot signal that survives a wall-clock jump.
    fn uptime_secs(&self) -> Option<u64> {
        self.fs
            .read_lossy("/proc/uptime")
            .ok()
            .and_then(|text| parse_uptime(&text))
    }

    /// The kernel's boot identity; `None` when the node is missing or unreadable.
    fn boot_id(&self) -> Option<String> {
        self.fs
            .read_str("/proc/sys/kernel/random/boot_id")
            .ok()
            .filter(|text| !text.is_empty())
    }

    /// Apps whose `WAKE_LOCK` op is currently ignored. One binder call, hence opt-in only.
    fn blocked_json(&self) -> Result<Value> {
        let blocked = self.appops.query_blocked()?;
        let mut out = Map::new();
        for package in blocked {
            out.insert(package, json!(true));
        }
        Ok(Value::Object(out))
    }

    /// Validate a request before touching the system, so a bad entry cannot leave half the apps
    /// changed.
    ///
    /// Protection is decided per entry, not per request: releasing a protected app in the same call
    /// that blocks another one is legitimate, and used to be rejected as a whole.
    fn validate_rules(
        &self,
        rules: &Map<String, Value>,
        exempted: &BTreeSet<String>,
    ) -> Result<()> {
        for (package, want) in rules {
            let Some(blocked) = want.as_bool() else {
                return Err(Error::invalid_arg(format!(
                    "{ITEM_BLOCKED}.{package}: expected true/false"
                )));
            };
            if !is_package_name(package) {
                return Err(Error::invalid_arg(format!(
                    "{ITEM_BLOCKED}: not a package name: {package:?}"
                )));
            }
            if !blocked {
                // Unblocking is always allowed: a user may need to undo an OEM default.
                continue;
            }
            if is_protected(package) {
                return Err(Error::invalid_arg(format!(
                    "{package} is protected: blocking its wake locks can drop calls, alarms or system UI state"
                )));
            }
            if exempted.contains(package) {
                return Err(Error::invalid_arg(format!(
                    "{package} is on the user whitelist: an exempt app must never be blocked"
                )));
            }
        }
        Ok(())
    }

    /// Apply `{pkg: blocked}`: validate everything first, then write, then read back once.
    ///
    /// `requested`/`applied` stay keyed by **item id** (`wakelock.blocked`) with the package map
    /// inside, because the registry persists `requested` verbatim under `config.modules.wakelock`.
    /// Keying them by package instead would flatten the config into `{"com.foo": true}` and break
    /// boot replay, which looks the rules up by item id.
    fn apply_rules(
        &self,
        rules: &Map<String, Value>,
        exempted: &BTreeSet<String>,
    ) -> Result<ApplyResult> {
        self.validate_rules(rules, exempted)?;
        if rules.is_empty() {
            return Ok(ApplyResult::from_readback(
                &item_payload(rules),
                item_payload(&Map::new()),
            ));
        }

        // Snapshot first so a mid-way failure can be rolled back to the previous state rather than
        // leaving the user with half the apps blocked.
        let before = self.appops.query_blocked()?;
        let mut changed: Vec<(String, bool)> = Vec::new();
        let mut failure: Option<Error> = None;
        for (package, want) in rules {
            let blocked = want.as_bool().unwrap_or(false);
            if before.contains(package) == blocked {
                // Already in the requested state: nothing to write, nothing to roll back.
                changed.push((package.clone(), blocked));
                continue;
            }
            match self.appops.set_blocked(package, blocked) {
                Ok(()) => changed.push((package.clone(), blocked)),
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = failure {
            let mut stuck: Vec<&String> = Vec::new();
            for (package, want) in &changed {
                if before.contains(package) == *want {
                    continue;
                }
                if self
                    .appops
                    .set_blocked(package, before.contains(package))
                    .is_err()
                {
                    // A rollback that itself fails leaves an app in a state the user did not ask
                    // for. Returning the original error as if everything were undone hides it.
                    stuck.push(package);
                }
            }
            if stuck.is_empty() {
                return Err(e);
            }
            let names: Vec<&str> = stuck.iter().map(|package| package.as_str()).collect();
            return Err(Error::new(
                e.code,
                format!(
                    "{}; rollback failed for {} (manual repair needed)",
                    e.msg,
                    names.join(", ")
                ),
            ));
        }

        // Read back what the system actually reports instead of echoing the request.
        let after = self.appops.query_blocked()?;
        let mut applied = Map::new();
        for package in rules.keys() {
            applied.insert(package.clone(), json!(after.contains(package)));
        }
        Ok(ApplyResult::from_readback(
            &item_payload(rules),
            item_payload(&applied),
        ))
    }

    /// Release exactly the packages this module persisted as blocked.
    ///
    /// The previous implementation freed *every* non-protected app it saw ignored, which also
    /// undid rules the user set through another tool or an OEM default. Only packages that are both
    /// persisted by us and currently ignored are touched; anything else is not ours to change.
    fn release_owned_rules(&self, settings: &Map<String, Value>) -> Result<()> {
        let Some(Value::Object(rules)) = settings.get(ITEM_BLOCKED) else {
            return Ok(());
        };
        let ours: Vec<&String> = rules
            .iter()
            .filter(|(package, want)| {
                want.as_bool() == Some(true) && is_package_name(package) && !is_protected(package)
            })
            .map(|(package, _)| package)
            .collect();
        if ours.is_empty() {
            return Ok(());
        }
        let blocked = self.appops.query_blocked()?;
        let mut failure: Option<Error> = None;
        for package in ours {
            if !blocked.contains(package) {
                // Already released (a reboot cleared it, or the user did it): nothing to write.
                continue;
            }
            if let Err(e) = self.appops.set_blocked(package, false) {
                failure.get_or_insert(e);
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    /// Apply `wakelock.blocked` and/or `wakelock.exempt` in one request.
    ///
    /// An exemption implies the release of an existing block: leaving an app both exempt and
    /// blocked would make the whitelist a lie. The implied release is part of `requested`, so it is
    /// persisted exactly like an explicit one.
    fn apply_with_exempt(
        &self,
        req: &Map<String, Value>,
        persisted_exempt: &BTreeSet<String>,
    ) -> Result<ApplyResult> {
        let blocked = req.get(ITEM_BLOCKED);
        let exempt = req.get(ITEM_EXEMPT);
        if blocked.is_none() && exempt.is_none() {
            return Err(Error::invalid_arg(format!(
                "wakelock has no writable item other than {ITEM_BLOCKED} and {ITEM_EXEMPT}"
            )));
        }

        let mut exempt_rules = Map::new();
        if let Some(value) = exempt {
            let Value::Object(rules) = value else {
                return Err(Error::invalid_arg(format!(
                    "{ITEM_EXEMPT}: expected an object keyed by package"
                )));
            };
            for (package, want) in rules {
                if want.as_bool().is_none() {
                    return Err(Error::invalid_arg(format!(
                        "{ITEM_EXEMPT}.{package}: expected true/false"
                    )));
                }
                if !is_package_name(package) {
                    return Err(Error::invalid_arg(format!(
                        "{ITEM_EXEMPT}: not a package name: {package:?}"
                    )));
                }
                exempt_rules.insert(package.clone(), want.clone());
            }
        }

        let mut exempted = persisted_exempt.clone();
        for (package, want) in &exempt_rules {
            if want.as_bool() == Some(true) {
                exempted.insert(package.clone());
            } else {
                exempted.remove(package);
            }
        }

        // Effective block map: what the user asked for, plus a release for every app being exempted.
        let mut effective = Map::new();
        if let Some(value) = blocked {
            let Value::Object(rules) = value else {
                return Err(Error::invalid_arg(format!(
                    "{ITEM_BLOCKED}: expected an object keyed by package"
                )));
            };
            for (package, want) in rules {
                effective.insert(package.clone(), want.clone());
            }
        }
        for (package, want) in &exempt_rules {
            if want.as_bool() == Some(true) {
                effective.entry(package.clone()).or_insert(json!(false));
            }
        }

        let mut result = if effective.is_empty() {
            ApplyResult::from_readback(&Map::new(), Map::new())
        } else {
            self.apply_rules(&effective, &exempted)?
        };

        if !exempt_rules.is_empty() {
            let mut requested = result.requested.clone();
            let mut applied = result.applied.clone();
            requested.insert(ITEM_EXEMPT.to_string(), Value::Object(exempt_rules.clone()));
            applied.insert(ITEM_EXEMPT.to_string(), Value::Object(exempt_rules));
            result = ApplyResult::from_readback(&requested, applied);
        }
        Ok(result)
    }

    /// Report wake locks held far longer than any normal operation.
    ///
    /// Informational only: it never blocks anything, and the user whitelist is deliberately not
    /// consulted — "never block this app" is a policy about *blocking*, not about a lock that is
    /// already pinning the device.
    fn watchdog(&self, sources: &[KernelSource]) {
        let Some(events) = &self.events else {
            return;
        };
        for source in sources {
            if source.active_since_ms < LONG_HELD_MS {
                continue;
            }
            let _ = events.emit(
                NewSystemEvent::new(
                    "wakelock",
                    "power",
                    "wakelock_long_held",
                    "warning",
                    "wakeup source held the device awake",
                )
                .message(format!(
                    "{} has been active for {}",
                    source.name,
                    crate::wakelock::read::format_duration(source.active_since_ms)
                ))
                .metadata(json!({
                    "kind": "kernel_source",
                    "name": source.name,
                    "held_ms": source.active_since_ms,
                }))
                .dedupe(
                    format!("wakelock.long_held.kernel.{}", source.name),
                    LONG_HELD_DEDUPE_S,
                ),
            );
        }

        // Userspace locks are the actionable ones because they name an app. A failed dump is
        // skipped silently: the daemon must not warn every five minutes on a ROM without `dumpsys`.
        let Ok(dump) = self.power.dump() else {
            return;
        };
        for lock in parse_power_wakelocks(&dump) {
            if lock.held_ms < LONG_HELD_MS || !lock.kind.contains("PARTIAL") {
                continue;
            }
            if lock.package.as_deref().is_some_and(is_protected) {
                continue;
            }
            let owner = lock.package.clone().unwrap_or_else(|| lock.tag.clone());
            let _ = events.emit(
                NewSystemEvent::new(
                    "wakelock",
                    "power",
                    "wakelock_long_held",
                    "warning",
                    "app kept a wake lock too long",
                )
                .message(format!(
                    "{} has held '{}' for {}",
                    owner,
                    lock.tag,
                    crate::wakelock::read::format_duration(lock.held_ms)
                ))
                .metadata(json!({
                    "kind": "userspace_lock",
                    "package": lock.package,
                    "tag": lock.tag,
                    "held_ms": lock.held_ms,
                }))
                .dedupe(
                    format!("wakelock.long_held.partial.{}", lock.tag),
                    LONG_HELD_DEDUPE_S,
                ),
            );
        }
    }
}

impl Module for WakelockModule {
    fn id(&self) -> &'static str {
        "wakelock"
    }

    fn probe(&self) -> Caps {
        // Blocking works on every supported Android version, with or without debugfs, so the module
        // stays available when the kernel table is unreadable; only the read features drop out.
        // Read-only check: `probe()` must stay side-effect free, so the debugfs mount happens in
        // `collect()`/`sample()` where a missing table is actually being reported on.
        match self.read_table() {
            Ok(_) => Caps::supported(&["kernel_sources", "userspace_held", "history", "block"]),
            Err(_) => Caps::supported(&["block", "userspace_held"]),
        }
    }

    fn schema(&self) -> Schema {
        Schema {
            module: "wakelock".to_string(),
            supported: true,
            items: vec![
                SchemaItem::new(ITEM_SOURCES, ItemType::Readonly),
                SchemaItem::new(ITEM_RANKED, ItemType::Readonly),
                SchemaItem::new(ITEM_SUMMARY, ItemType::Readonly),
                // Read-only policy, published so clients can render "protected" without keeping a
                // second copy of the rule that the module enforces.
                SchemaItem::new(ITEM_PROTECTED, ItemType::Readonly),
                // Scope `app`: the value is an object keyed by package, `true` = WAKE_LOCK ignored.
                SchemaItem::new(ITEM_BLOCKED, ItemType::Bool)
                    .scope(Scope::App)
                    .risk(Risk::Medium)
                    .persist(true)
                    .title_key("wakelock.blocked"),
                // User-managed whitelist: `true` = never block this app. Configuration only — there
                // is no system state to read back — so clients read the persisted intent from
                // `snapshot.settings`, exactly like `wakelock.blocked`.
                SchemaItem::new(ITEM_EXEMPT, ItemType::Bool)
                    .scope(Scope::App)
                    .risk(Risk::Low)
                    .persist(true)
                    .title_key("wakelock.exempt"),
            ],
        }
    }

    fn collect(&self, keys: Option<&[String]>) -> Result<Value> {
        let wanted = |key: &str| keys.is_none_or(|keys| keys.iter().any(|k| k == key));
        let (available, sources, reason) = self.sources();
        let state = self.state.load();
        let uptime = self.uptime_secs();
        let mut out = Map::new();
        if wanted(ITEM_SOURCES) {
            out.insert(ITEM_SOURCES.to_string(), sources_json(&sources));
        }
        if wanted(ITEM_RANKED) {
            out.insert(ITEM_RANKED.to_string(), ranked_json(&state));
        }
        if wanted(ITEM_SUMMARY) {
            out.insert(
                ITEM_SUMMARY.to_string(),
                summary_json(available, reason.as_deref(), &sources, &state, uptime),
            );
        }
        // The protection policy as data, so clients stop hardcoding their own copy of it: a copy
        // drifts, and a drifted copy either blocks a protected app or disables a working switch.
        if wanted(ITEM_PROTECTED) {
            out.insert(ITEM_PROTECTED.to_string(), json!(PROTECTED));
        }
        // Expensive (one binder call): `snapshot` never asks for it, and `get` has to name it.
        if keys.is_some_and(|keys| keys.iter().any(|k| k == ITEM_BLOCKED)) {
            out.insert(ITEM_BLOCKED.to_string(), self.blocked_json()?);
        }
        Ok(Value::Object(out))
    }

    fn apply(&self, req: &Map<String, Value>) -> Result<ApplyResult> {
        self.apply_with_exempt(req, &BTreeSet::new())
    }

    /// Apply knowing the persisted settings: an exemption stored earlier still forbids blocking
    /// that app, even when the current request does not mention it.
    fn apply_with_settings(
        &self,
        req: &Map<String, Value>,
        settings: &Map<String, Value>,
    ) -> Result<ApplyResult> {
        self.apply_with_exempt(req, &exempt_set(settings.get(ITEM_EXEMPT)))
    }

    /// Compare the persisted intent with the live AppOps state.
    ///
    /// Two kinds of drift: a block that no longer matches AppOps, and a block that contradicts a
    /// whitelist entry — the daemon then releases it through `apply`, so an exemption always wins.
    fn drift(&self, settings: &Map<String, Value>) -> Result<Vec<String>> {
        let Some(Value::Object(rules)) = settings.get(ITEM_BLOCKED) else {
            return Ok(Vec::new());
        };
        let exempt = exempt_set(settings.get(ITEM_EXEMPT));
        let mut drifted = rules
            .iter()
            .any(|(package, want)| want.as_bool() == Some(true) && exempt.contains(package));
        if !drifted && !rules.is_empty() {
            let blocked = self.appops.query_blocked()?;
            drifted = rules
                .iter()
                .any(|(package, want)| blocked.contains(package) != (want.as_bool() == Some(true)));
        }
        Ok(if drifted {
            vec![ITEM_BLOCKED.to_string()]
        } else {
            Vec::new()
        })
    }

    /// Drop boot-time rules that no longer make sense: protected apps, whitelisted apps, and apps
    /// that were uninstalled while the rule was stored. A dropped rule is better than a boot that
    /// keeps retrying it.
    fn sanitize_boot(&self, req: &Map<String, Value>) -> Map<String, Value> {
        let exempt = exempt_set(req.get(ITEM_EXEMPT));
        let mut out = req.clone();

        // The whitelist is configuration, so only its shape has to survive.
        if let Some(Value::Object(rules)) = req.get(ITEM_EXEMPT) {
            let mut kept = Map::new();
            for (package, want) in rules {
                if !is_package_name(package) || want.as_bool().is_none() {
                    continue;
                }
                kept.insert(package.clone(), want.clone());
            }
            out.insert(ITEM_EXEMPT.to_string(), Value::Object(kept));
        }

        let Some(Value::Object(rules)) = req.get(ITEM_BLOCKED) else {
            return out;
        };
        let mut kept = Map::new();
        for (package, want) in rules {
            if !is_package_name(package) || is_protected(package) || exempt.contains(package) {
                continue;
            }
            if want.as_bool() == Some(true) && !self.appops.package_exists(package) {
                continue;
            }
            kept.insert(package.clone(), want.clone());
        }
        out.insert(ITEM_BLOCKED.to_string(), Value::Object(kept));
        out
    }

    /// Undo the blocks this module persisted, and only those.
    ///
    /// Without the persisted settings there is no safe set to undo — releasing "everything that is
    /// ignored" would also clear rules SysFlow never set — so the settings-less entry point stays a
    /// no-op and the registry uses the settings-aware one.
    fn reset_with_settings(&self, settings: &Map<String, Value>) -> Result<()> {
        self.release_owned_rules(settings)
    }

    /// Five minutes: the table is a kernel seq_file and per-source counters move slowly.
    fn sample(&self) -> Option<sysflow_core::Result<Sample>> {
        let (available, sources, _reason) = self.sources();
        // Before the availability check: the userspace watchdog works even when debugfs does not.
        self.watchdog(&sources);
        if !available {
            return None;
        }
        let state = self.observe(&sources);
        self.state.store(&state);
        let deltas = state.deltas();
        let mut sample = Map::new();
        sample.insert("known".into(), json!(sources.len()));
        sample.insert(
            "active".into(),
            json!(sources.iter().filter(|source| source.is_active()).count()),
        );
        sample.insert(
            "top_ms".into(),
            json!(deltas.first().map(|delta| delta.ms).unwrap_or(0)),
        );
        // Same window, counted: the trend page can plot either dimension from one series.
        sample.insert(
            "wakeups".into(),
            json!(deltas.iter().map(|delta| delta.wakeups).sum::<i64>()),
        );
        Some(Ok(sample))
    }

    fn sample_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(SAMPLE_SECS))
    }
}

/// Packages the user has whitelisted (`true` entries of `wakelock.exempt`).
///
/// A whitelist entry means "never block this app". It is also consulted by `drift`, so a stored
/// block that contradicts an exemption is released instead of being re-applied forever.
fn exempt_set(value: Option<&Value>) -> BTreeSet<String> {
    let Some(Value::Object(rules)) = value else {
        return BTreeSet::new();
    };
    rules
        .iter()
        .filter(|(_, want)| want.as_bool() == Some(true))
        .map(|(package, _)| package.clone())
        .collect()
}

/// Wrap an app-scoped rule map in the item-id envelope the registry expects.
fn item_payload(rules: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert(ITEM_BLOCKED.to_string(), Value::Object(rules.clone()));
    out
}

#[cfg(test)]
mod tests;
