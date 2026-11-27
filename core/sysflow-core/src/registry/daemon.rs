//! Daemon lifecycle and the single long-lived scheduling loop.
//!
//! Split out of `registry.rs` because the registry had grown into three unrelated jobs: the command
//! surface, the config/recovery rules and the daemon. Everything the daemon needs is expressed as
//! `Registry` methods in this file, so the parent module keeps ownership of module access.
//!
//! The loop owns three cadences:
//!
//! * **display transitions** — polled every 1 s while the screen is off and every 5 s while it is on,
//!   applying `cpu.screen_off_active` and emitting display events. This replaced the `screen_loop`
//!   shell worker in `service.sh`.
//! * **drift repair / adaptive tick** — the configured guard interval (2–120 s, default 10 s),
//!   skipped entirely while `no_enforce` exists.
//! * **history sampling** — each module's own `sample_interval()` (battery: 60 s), recorded with the
//!   screen state that was current at sample time so clients can split drain by display state.

use std::collections::BTreeMap;
use std::fs;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::event::NewSystemEvent;
use crate::history::HistoryStore;
use crate::safe;
use crate::screen::{ScreenState, ScreenWatcher};

use super::Registry;

/// Poll cadence while the display is off: a wake-up must be noticed quickly so the caps are
/// released before the user starts interacting.
const SCREEN_OFF_TICK: Duration = Duration::from_secs(1);
/// Poll cadence while the display is on. `dumpsys power` only runs when the backlight changes, so
/// this is the cost of one small sysfs read.
const SCREEN_ON_TICK: Duration = Duration::from_secs(5);
/// Default guard interval when `guard_interval` is missing or invalid.
const DEFAULT_GUARD_SECS: u64 = 10;

/// Everything the loop carries between ticks. Keeping it in one struct makes the restart behaviour
/// obvious: a new daemon process starts with no memory, detects the display immediately and rebuilds
/// the sampling schedule.
struct LoopState {
    history: HistoryStore,
    last_sampled: BTreeMap<&'static str, Instant>,
    /// `None` means "enforce on the first tick", which is what the previous loop did.
    last_guard: Option<Instant>,
    screen: ScreenWatcher,
    screen_state: ScreenState,
    /// Last screen-transition error, so a persistent failure is logged once instead of every second.
    screen_error: Option<String>,
    /// Wall-clock second of the last published `daemon-screen.json` snapshot. The status command is
    /// a different process, so the loop publishes its detection there (throttled to 5 s).
    last_status_write: Option<u64>,
}

impl LoopState {
    fn new(registry: &Registry) -> Self {
        LoopState {
            history: HistoryStore::new(registry.ctx.data_dir.join("history")),
            last_sampled: BTreeMap::new(),
            last_guard: None,
            screen: ScreenWatcher::new(registry.ctx.sysfs.clone()),
            screen_state: ScreenState::Unknown,
            screen_error: None,
            last_status_write: None,
        }
    }
}

/// Wire name of a display state in `daemon-screen.json` and `daemon status`.
fn screen_name(state: ScreenState) -> &'static str {
    match state {
        ScreenState::On => "on",
        ScreenState::Off => "off",
        ScreenState::Unknown => "unknown",
    }
}

impl Registry {
    fn daemon_pid_path(&self) -> std::path::PathBuf {
        self.ctx.data_dir.join("daemon.pid")
    }

    fn daemon_pid(&self) -> Option<u32> {
        let raw = std::fs::read_to_string(self.daemon_pid_path()).ok()?;
        let pid = raw.trim().parse::<u32>().ok()?;
        if pid == 0 || !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            return None;
        }
        let command = std::fs::read_to_string(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        if !command.contains("daemon") || !command.contains("run") {
            return None;
        }
        Some(pid)
    }

    /// Runtime snapshot of the daemon's latest display detection. Flat next to `daemon.pid` and
    /// `guard_interval`, because it describes the daemon process rather than a module's state.
    fn screen_status_path(&self) -> std::path::PathBuf {
        self.ctx.data_dir.join("daemon-screen.json")
    }

    /// Publish the watchdog's view of the display so `daemon status` can tell "off and healthy"
    /// from "active but detection unavailable / daemon gone". The age of the successful answer
    /// keeps growing on its own while detection is broken, because the timestamp is persisted.
    fn publish_screen_status(&self, state: &mut LoopState, force: bool) {
        let now = crate::history::now_secs();
        if !force
            && state
                .last_status_write
                .is_some_and(|at| now.saturating_sub(at) < 5)
        {
            return;
        }
        state.last_status_write = Some(now);
        let payload = json!({
            "screen": screen_name(state.screen_state),
            "checked_at": state.screen.last_poll_ts(),
            "success_at": state.screen.last_success_ts(),
        });
        let path = self.screen_status_path();
        // Atomic replace: a killed daemon must never leave a half-written snapshot behind.
        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, payload.to_string()).is_ok() {
            let _ = fs::rename(&tmp, path);
        }
    }

    /// Latest published detection, read by `daemon status` in a separate CLI process. Missing or
    /// corrupt snapshots are reported as `unknown` with no timestamps.
    fn read_screen_status(&self) -> (String, Option<u64>, Option<u64>) {
        let Some(text) = fs::read_to_string(self.screen_status_path()).ok() else {
            return ("unknown".into(), None, None);
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return ("unknown".into(), None, None);
        };
        let screen = value
            .get("screen")
            .and_then(Value::as_str)
            .filter(|name| matches!(*name, "on" | "off" | "unknown"))
            .unwrap_or("unknown")
            .to_string();
        let checked_at = value.get("checked_at").and_then(Value::as_u64);
        let success_at = value.get("success_at").and_then(Value::as_u64);
        (screen, checked_at, success_at)
    }

    /// Guard interval in seconds, clamped to the documented 2–120 s range.
    fn guard_interval_secs(&self) -> u64 {
        std::fs::read_to_string(self.ctx.data_dir.join("guard_interval"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .filter(|n| (2..=120).contains(n))
            .unwrap_or(DEFAULT_GUARD_SECS)
    }

    pub fn daemon_status(&self) -> Result<Value> {
        let cfg = self.store.load()?;
        let enabled = cfg.boot.enabled
            && !cfg.safe.safe_mode
            && !safe::rescue_marker_present(&self.ctx)
            && !self.ctx.data_dir.join("no_enforce").exists();
        let (screen, checked_at, success_at) = self.read_screen_status();
        // Age is computed at read time: if the daemon died, the persisted timestamp stops advancing
        // and the number grows, which is exactly what a client needs to distinguish stale detection
        // from a healthy "off" decision.
        let screen_age_s = success_at
            .filter(|at| *at <= crate::history::now_secs())
            .map(|at| crate::history::now_secs() - at);
        Ok(json!({
            "running": self.daemon_pid().is_some(),
            "implemented": true,
            "enabled": enabled,
            "interval_s": self.guard_interval_secs(),
            "screen": screen,
            "screen_age_s": screen_age_s,
            "screen_checked_at": checked_at,
        }))
    }

    /// Spawn the enforcement loop as a detached child. The module service also uses this command,
    /// so the same PID/lock semantics apply whether the daemon is started manually or at boot.
    pub fn daemon_start(&self) -> Result<Value> {
        let cfg = self.store.load()?;
        if !cfg.boot.enabled {
            return Ok(
                json!({ "running": false, "enabled": false, "reason": "boot_apply_disabled" }),
            );
        }
        if cfg.safe.safe_mode || safe::rescue_marker_present(&self.ctx) {
            return Ok(json!({ "running": false, "enabled": false, "reason": "safe_mode" }));
        }
        if self.ctx.data_dir.join("no_enforce").exists() {
            return Ok(json!({ "running": false, "enabled": false, "reason": "disabled" }));
        }
        if let Some(pid) = self.daemon_pid() {
            // A module upgrade replaces the binary on disk but leaves the old process running. The
            // old process then keeps serving stale code (and, before this check, silently ignored
            // newly added features such as history sampling until the device rebooted).
            if daemon_binary_is_stale(pid) {
                self.log.info(&format!(
                    "daemon pid={pid} runs a replaced binary; restarting it"
                ));
                let _ = self.daemon_stop();
            } else {
                return Ok(json!({ "running": true, "pid": pid, "already_running": true }));
            }
        }
        let exe = std::env::current_exe().map_err(|e| Error::io(e.to_string()))?;
        let log_path = self.ctx.data_dir.join("log").join("daemon.log");
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(e.to_string()))?;
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| Error::io(e.to_string()))?;
        let err_log = log.try_clone().map_err(|e| Error::io(e.to_string()))?;
        let child = Command::new(exe)
            .args(["daemon", "run"])
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(err_log))
            .spawn()
            .map_err(|e| Error::io(format!("start daemon: {e}")))?;
        let pid = child.id();
        // The child writes the same file once its loop starts; this early write closes the small
        // status race between `spawn()` returning and the first daemon iteration.
        let _ = std::fs::create_dir_all(&self.ctx.data_dir);
        let _ = std::fs::write(self.daemon_pid_path(), format!("{pid}\n"));
        self.emit_event(
            NewSystemEvent::new(
                "system",
                "service",
                "daemon_started",
                "info",
                "SysFlow daemon started",
            )
            .message("The module enforcement daemon is running.")
            .metadata(json!({ "pid": pid }))
            .dedupe("daemon_started", 5),
        );
        Ok(json!({ "running": true, "pid": pid, "already_running": false }))
    }

    pub fn daemon_stop(&self) -> Result<Value> {
        // The daemon owns display transitions, so stopping it while the screen-off profile is active
        // would otherwise leave a stale cap (and possibly offlined cores) behind. Release first.
        self.release_screen_profile_if_active();
        let Some(pid) = self.daemon_pid() else {
            let _ = std::fs::remove_file(self.daemon_pid_path());
            return Ok(json!({ "running": false, "stopped": false }));
        };
        let status = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .map_err(|e| Error::io(format!("stop daemon: {e}")))?;
        if !status.success() {
            return Err(Error::io("could not send SIGTERM to daemon"));
        }
        let _ = std::fs::remove_file(self.daemon_pid_path());
        self.emit_event(
            NewSystemEvent::new(
                "system",
                "service",
                "daemon_stopped",
                "info",
                "SysFlow daemon stopped",
            )
            .message("The module enforcement daemon was stopped.")
            .metadata(json!({ "pid": pid }))
            .dedupe("daemon_stopped", 5),
        );
        Ok(json!({ "running": false, "stopped": true, "pid": pid }))
    }

    /// Update the periodic enforcement service. The default is enabled with a 10 second interval.
    pub fn daemon_configure(
        &self,
        enabled: Option<bool>,
        interval_s: Option<u64>,
    ) -> Result<Value> {
        if let Some(interval) = interval_s {
            if !(2..=120).contains(&interval) {
                return Err(Error::invalid_arg(
                    "daemon interval must be between 2 and 120 seconds",
                ));
            }
            std::fs::create_dir_all(&self.ctx.data_dir).map_err(|e| Error::io(e.to_string()))?;
            std::fs::write(
                self.ctx.data_dir.join("guard_interval"),
                format!("{interval}\n"),
            )
            .map_err(|e| Error::io(format!("write daemon interval: {e}")))?;
            self.emit_event(
                NewSystemEvent::new(
                    "system",
                    "configuration",
                    "daemon_config_updated",
                    "info",
                    "Daemon interval updated",
                )
                .message(format!("Guard interval set to {interval} seconds."))
                .metadata(json!({ "interval_s": interval })),
            );
        }
        match enabled {
            Some(true) => {
                let cfg = self.store.load()?;
                if !cfg.boot.enabled {
                    return Err(Error::invalid_arg(
                        "continuous guard requires boot-time apply to be enabled",
                    ));
                }
                if cfg.safe.safe_mode || safe::rescue_marker_present(&self.ctx) {
                    return Err(Error::safe_mode(
                        "continuous guard is blocked while safe mode is active",
                    ));
                }
                let _ = std::fs::remove_file(self.ctx.data_dir.join("no_enforce"));
                let _ = self.daemon_start()?;
            }
            Some(false) => {
                std::fs::create_dir_all(&self.ctx.data_dir)
                    .map_err(|e| Error::io(e.to_string()))?;
                std::fs::write(self.ctx.data_dir.join("no_enforce"), "disabled\n")
                    .map_err(|e| Error::io(format!("disable daemon: {e}")))?;
                let _ = self.daemon_stop()?;
            }
            None => {}
        }
        self.daemon_status()
    }

    /// Called by `daemon run`. The daemon is intentionally module-owned rather than APK-owned:
    /// CPU adaptive sampling and drift repair continue after the UI process exits.
    pub fn daemon_run(&self) -> Result<Value> {
        let _ = std::fs::create_dir_all(&self.ctx.data_dir);
        let pid = std::process::id();
        std::fs::write(self.daemon_pid_path(), format!("{pid}\n"))
            .map_err(|e| Error::io(format!("write daemon pid: {e}")))?;
        self.log
            .info(&format!("enforcement daemon started pid={pid}"));
        // A restart must not inherit an active screen-off profile. `screen_off_active` lives in the
        // CPU state file and is re-applied by *any* later apply while it is true, so a stale flag
        // (crash, OOM kill, daemon restart while the display was off) would cap the CPU even after
        // the screen comes on. Releasing here is always safe: the next confirmed "off" re-applies it
        // within a couple of seconds, and "on" can never be harmed.
        self.release_screen_profile_if_active();
        let mut state = LoopState::new(self);
        loop {
            self.daemon_tick(&mut state);
            thread::sleep(self.tick_delay(&state));
        }
    }

    /// One pass of the scheduler. Screen detection runs first because both the tick delay and the
    /// sampling context depend on it.
    fn daemon_tick(&self, state: &mut LoopState) {
        let previous = state.screen_state;
        state.screen_state = state.screen.detect();
        self.screen_profile_tick(state);
        self.publish_screen_status(state, previous != state.screen_state);
        if !self.ctx.data_dir.join("no_enforce").exists() {
            let guard = Duration::from_secs(self.guard_interval_secs());
            let due = state
                .last_guard
                .map(|last| last.elapsed() >= guard)
                .unwrap_or(true);
            if due {
                state.last_guard = Some(Instant::now());
                if let Err(e) = self.enforce() {
                    self.log.error(&format!("enforcement tick failed: {e}"));
                }
            }
        }
        self.sample_due_modules(state);
    }

    /// Keep the screen-off CPU profile in sync with the display, and clean up after the feature is
    /// disabled. Returns quickly: the only sysfs write happens on a real transition.
    fn screen_profile_tick(&self, state: &mut LoopState) {
        let keys = [
            "cpu.screen_off".to_string(),
            "cpu.screen_off_active".to_string(),
        ];
        let live = match self.get("cpu", Some(&keys)) {
            Ok(value) => value,
            // CPU module missing or unreadable: nothing to manage here.
            Err(_) => return,
        };
        let enabled = live
            .get("cpu.screen_off")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let active = live
            .get("cpu.screen_off_active")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !enabled {
            if active {
                self.release_screen_profile(state);
            }
            return;
        }
        let want = match state.screen_state {
            ScreenState::Off => true,
            ScreenState::On => false,
            // Unknown is never a transition; retry on the next tick.
            ScreenState::Unknown => return,
        };
        if want == active {
            return;
        }
        // A release forced by the stale-unknown safety net deserves its own warning: the display
        // was not *observed* as on, the detector simply stopped trusting its last "off" answer.
        let fallback = active && !want && state.screen.safety_on();
        match self.set("cpu", json!({ "cpu.screen_off_active": want }), false) {
            Ok(_) => {
                state.screen_error = None;
                self.log.info(&format!(
                    "screen transition state={} -> screen_off_active={want}",
                    if want { "off" } else { "on" }
                ));
                if fallback {
                    let age = state
                        .screen
                        .unknown_age()
                        .map(|age| age.as_secs())
                        .unwrap_or(0);
                    self.log.warn(&format!(
                        "screen detection unavailable for {}s; released screen-off profile as a safety net",
                        age
                    ));
                    self.emit_event(
                        NewSystemEvent::new(
                            "display",
                            "power",
                            "screen_detection_fallback",
                            "warning",
                            "Screen-off profile released after detection outage",
                        )
                        .message(
                            "dumpsys power stayed unreadable; normal CPU limits were restored until the display is confirmed off.",
                        )
                        .metadata(json!({
                            "reason": "framework_unknown_timeout",
                            "unknown_age_s": age,
                            "screen": screen_name(state.screen_state),
                        }))
                        .dedupe("display.screen_detection_fallback", 300),
                    );
                    self.publish_screen_status(state, true);
                }
                self.emit_display_event(want);
            }
            Err(e) => self.report_screen_error(state, &format!("apply screen-off profile: {e}")),
        }
    }

    /// Restore normal limits after the screen-off feature was disabled while its profile was active.
    fn release_screen_profile(&self, state: &mut LoopState) {
        match self.set("cpu", json!({ "cpu.screen_off_active": false }), false) {
            Ok(_) => {
                state.screen_error = None;
                self.log
                    .info("screen transition: screen-off profile released (feature disabled)");
                self.emit_event(
                    NewSystemEvent::new(
                        "cpu",
                        "power",
                        "screen_off_profile_released",
                        "notice",
                        "CPU screen-off profile released",
                    )
                    .message(
                        "Restored normal CPU limits and cores after screen-off profile was disabled.",
                    )
                    .metadata(json!({ "reason": "stale_profile_cleanup" }))
                    .dedupe("cpu.screen_off.released", 60),
                );
            }
            Err(e) => self.report_screen_error(state, &format!("release screen-off profile: {e}")),
        }
    }

    /// Release an active profile without a loop state: used when the process is about to stop.
    fn release_screen_profile_if_active(&self) {
        let keys = ["cpu.screen_off_active".to_string()];
        let active = self
            .get("cpu", Some(&keys))
            .ok()
            .and_then(|value| value.get("cpu.screen_off_active").and_then(Value::as_bool))
            .unwrap_or(false);
        if active {
            let _ = self.set("cpu", json!({ "cpu.screen_off_active": false }), false);
        }
    }

    /// Report a screen-transition failure once per distinct cause.
    ///
    /// A failure here means the CPU profile could not be applied or released, which is exactly the
    /// situation where the user needs to see it: the log line is for `sysflow log`, the event is for
    /// the APK's system-events page.
    fn report_screen_error(&self, state: &mut LoopState, message: &str) {
        if state.screen_error.as_deref() != Some(message) {
            self.log.warn(message);
            self.emit_event(
                NewSystemEvent::new(
                    "display",
                    "power",
                    "screen_transition_failed",
                    "warning",
                    "Screen-off profile transition failed",
                )
                .message(message.to_string())
                .dedupe("display.screen_transition_failed", 300),
            );
            state.screen_error = Some(message.to_string());
        }
    }

    fn emit_display_event(&self, screen_off: bool) {
        let (event_type, title, message, dedupe) = if screen_off {
            (
                "screen_turned_off",
                "Screen turned off",
                "Display entered the off state.",
                "display.screen_off",
            )
        } else {
            (
                "screen_turned_on",
                "Screen turned on",
                "Display entered the on state.",
                "display.screen_on",
            )
        };
        self.emit_event(
            NewSystemEvent::new("display", "power", event_type, "info", title)
                .message(message)
                .dedupe(dedupe, 5),
        );
    }

    /// How long to sleep before the next tick: fast while the display is off or an "off" reading is
    /// still waiting for confirmation, and never slower than the guard interval so a 2 s setting is
    /// still honoured.
    fn tick_delay(&self, state: &LoopState) -> Duration {
        let base = match state.screen_state {
            ScreenState::Off => SCREEN_OFF_TICK,
            _ if state.screen.wants_fast_poll() => SCREEN_OFF_TICK,
            _ => SCREEN_ON_TICK,
        };
        base.min(Duration::from_secs(self.guard_interval_secs()))
    }

    /// Collect a sample from every module whose interval has elapsed and append it to its history.
    ///
    /// Best-effort by contract: a failing sample must never disturb drift repair or the adaptive
    /// controller, so failures are logged and the loop continues.
    fn sample_due_modules(&self, state: &mut LoopState) {
        let LoopState {
            history,
            last_sampled,
            screen_state,
            ..
        } = state;
        let screen = *screen_state;
        let now = Instant::now();
        for module in &self.modules {
            let Some(interval) = module.sample_interval() else {
                continue;
            };
            let first = !last_sampled.contains_key(module.id());
            let due = last_sampled
                .get(module.id())
                .map(|previous| now.duration_since(*previous) >= interval)
                .unwrap_or(true);
            if !due {
                continue;
            }
            last_sampled.insert(module.id(), now);
            match module.sample() {
                Some(Ok(sample)) => {
                    if first {
                        // One line per module per daemon lifetime: enough to prove sampling is alive
                        // in `sysflow log` without logging every interval.
                        self.log.info(&format!(
                            "history sampling started module={} interval={}s metrics={}",
                            module.id(),
                            interval.as_secs(),
                            sample.len()
                        ));
                    }
                    if let Err(e) = history.append_with_screen(
                        module.id(),
                        crate::history::now_secs(),
                        &sample,
                        screen,
                    ) {
                        self.log
                            .error(&format!("history append {} failed: {e}", module.id()));
                    }
                }
                Some(Err(e)) => self
                    .log
                    .error(&format!("history sample {} failed: {e}", module.id())),
                None => {}
            }
        }
    }
}

/// Is the daemon's executable older than the binary on disk?
///
/// Linux reports a replaced executable as `/proc/<pid>/exe -> … (deleted)`, which is the reliable
/// signal; the mtime comparison covers the case where the manager replaced the file in place.
fn daemon_binary_is_stale(pid: u32) -> bool {
    let running = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
    let current = std::env::current_exe().ok();
    let Some(running) = running else {
        return false;
    };
    if running.to_string_lossy().ends_with("(deleted)") {
        return true;
    }
    let (Some(current), Ok(running_meta)) = (current, std::fs::metadata(&running)) else {
        return false;
    };
    // `/proc/<pid>/exe` resolves to the *current* path, so compare inode identity first: a different
    // inode under the same path means the on-disk binary was replaced.
    let Ok(current_meta) = std::fs::metadata(&current) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if current_meta.ino() != running_meta.ino() {
            return true;
        }
    }
    current_meta.modified().ok() > running_meta.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::Ctx;
    use crate::module::{ApplyResult, Caps, Module};
    use crate::schema::{ItemType, Schema, SchemaItem};
    use crate::sysfs::Sysfs;
    use serde_json::{Map, Value};
    use std::sync::{Arc, Mutex};
    use std::thread;

    const OFF: &str = "Power Manager State:\n  mWakefulness=Asleep\n  mIsInteractive: false\n";

    fn temp_ctx(name: &str) -> (std::path::PathBuf, Ctx) {
        let dir =
            std::env::temp_dir().join(format!("sysflow-daemon-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let ctx = Ctx {
            data_dir: dir.clone(),
            module_dir: dir.clone(),
            sysfs: Sysfs::new(&dir),
        };
        (dir, ctx)
    }

    /// A CPU stand-in that only owns the two screen-off flags, so `screen_profile_tick` can be
    /// driven on a host without a real kernel tree.
    struct FakeCpu {
        active: Arc<Mutex<bool>>,
    }

    impl Module for FakeCpu {
        fn id(&self) -> &'static str {
            "cpu"
        }
        fn probe(&self) -> Caps {
            Caps::supported(&[])
        }
        fn schema(&self) -> Schema {
            Schema {
                module: "cpu".into(),
                supported: true,
                items: vec![
                    SchemaItem::new("cpu.screen_off", ItemType::Readonly),
                    SchemaItem::new("cpu.screen_off_active", ItemType::Bool),
                ],
            }
        }
        fn collect(&self, keys: Option<&[String]>) -> Result<Value> {
            let wanted = |key: &str| keys.is_none_or(|keys| keys.iter().any(|k| k == key));
            let mut out = Map::new();
            if wanted("cpu.screen_off") {
                out.insert("cpu.screen_off".into(), json!(true));
            }
            if wanted("cpu.screen_off_active") {
                out.insert(
                    "cpu.screen_off_active".into(),
                    json!(*self.active.lock().unwrap()),
                );
            }
            Ok(Value::Object(out))
        }
        fn apply(&self, req: &Map<String, Value>) -> Result<ApplyResult> {
            if let Some(want) = req.get("cpu.screen_off_active").and_then(Value::as_bool) {
                *self.active.lock().unwrap() = want;
            }
            Ok(ApplyResult::from_readback(req, req.clone()))
        }
    }

    fn fixture(name: &str, backlight: Option<u64>) -> (Registry, Arc<Mutex<bool>>, LoopState) {
        let (dir, ctx) = temp_ctx(name);
        if let Some(brightness) = backlight {
            let node = dir.join("sys/class/backlight/panel");
            fs::create_dir_all(&node).unwrap();
            fs::write(node.join("brightness"), format!("{brightness}\n")).unwrap();
        }
        let active = Arc::new(Mutex::new(true));
        let reg = Registry::new(
            ctx,
            vec![Box::new(FakeCpu {
                active: active.clone(),
            })],
        );
        let mut state = LoopState::new(&reg);
        state.screen = ScreenWatcher::with_script(reg.ctx().sysfs.clone(), vec![Some(OFF), None])
            .with_unknown_timeout(Duration::from_millis(60));
        (reg, active, state)
    }

    #[test]
    fn active_profile_is_released_when_backlight_is_on_and_detection_stales() {
        let (reg, active, mut state) = fixture("release-backlight", Some(800));
        state.screen_state = state.screen.detect();
        reg.screen_profile_tick(&mut state);
        assert!(
            *active.lock().unwrap(),
            "confirmed off keeps the profile active"
        );
        thread::sleep(Duration::from_millis(90));
        state.screen_state = state.screen.detect();
        reg.screen_profile_tick(&mut state);
        assert!(
            !*active.lock().unwrap(),
            "stale unknown must release the profile even though the framework never said 'on'"
        );
    }

    #[test]
    fn active_profile_is_released_without_backlight_after_timeout() {
        let (reg, active, mut state) = fixture("release-no-backlight", None);
        state.screen_state = state.screen.detect();
        reg.screen_profile_tick(&mut state);
        assert!(*active.lock().unwrap());
        thread::sleep(Duration::from_millis(90));
        state.screen_state = state.screen.detect();
        reg.screen_profile_tick(&mut state);
        assert!(!*active.lock().unwrap());
    }

    #[test]
    fn active_profile_is_kept_before_the_timeout() {
        let (reg, active, mut state) = fixture("keep-before-timeout", Some(800));
        state.screen_state = state.screen.detect();
        reg.screen_profile_tick(&mut state);
        assert!(*active.lock().unwrap());
        // One failed answer is not enough — no sleeping, so the fallback cannot have fired.
        state.screen_state = state.screen.detect();
        reg.screen_profile_tick(&mut state);
        assert!(
            *active.lock().unwrap(),
            "transient unknown must not release"
        );
        assert!(!state.screen.safety_on());
    }

    #[test]
    fn daemon_status_reports_published_screen_detection() {
        let (_dir, ctx) = temp_ctx("status");
        let reg = Registry::new(ctx, vec![]);
        let status = reg.daemon_status().unwrap();
        assert_eq!(status["screen"], "unknown");
        assert!(status["screen_age_s"].is_null());
        assert!(status["screen_checked_at"].is_null());

        let now = crate::history::now_secs();
        fs::write(
            reg.screen_status_path(),
            json!({ "screen": "off", "checked_at": now, "success_at": now - 5 }).to_string(),
        )
        .unwrap();
        let status = reg.daemon_status().unwrap();
        assert_eq!(status["screen"], "off");
        assert_eq!(status["screen_age_s"], 5);
        assert_eq!(status["screen_checked_at"], now);

        // A corrupt snapshot degrades to unknown instead of failing the whole status command.
        fs::write(reg.screen_status_path(), "not json").unwrap();
        let status = reg.daemon_status().unwrap();
        assert_eq!(status["screen"], "unknown");
        assert!(status["screen_age_s"].is_null());
    }

    #[test]
    fn loop_publishes_the_detection_snapshot() {
        let (reg, _active, mut state) = fixture("publish", Some(800));
        state.screen_state = state.screen.detect();
        reg.publish_screen_status(&mut state, true);
        let text = fs::read_to_string(reg.screen_status_path()).unwrap();
        let snapshot: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(snapshot["screen"], "off");
        assert!(snapshot["checked_at"].is_u64());
        assert!(snapshot["success_at"].is_u64());
    }
}
