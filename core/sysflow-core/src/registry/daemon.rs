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
        }
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
        Ok(json!({
            "running": self.daemon_pid().is_some(),
            "implemented": true,
            "enabled": enabled,
            "interval_s": self.guard_interval_secs(),
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
        state.screen_state = state.screen.detect();
        self.screen_profile_tick(state);
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
        match self.set("cpu", json!({ "cpu.screen_off_active": want }), false) {
            Ok(_) => {
                state.screen_error = None;
                self.log.info(&format!(
                    "screen transition state={} -> screen_off_active={want}",
                    if want { "off" } else { "on" }
                ));
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
