//! Display (screen) state detection for the enforcement daemon.
//!
//! The shell used to carry this logic in `module/screen-state.sh` plus a `screen_loop` in
//! `service.sh`. Both are gone: the daemon owns the single long-lived loop, so the transition
//! detector has to live next to it.
//!
//! Two sources are used, in this order:
//!
//! 1. **Backlight brightness** (`/sys/class/backlight/*/brightness`) — one small read, so it is
//!    used as the cheap change detector.
//! 2. **`dumpsys power`** — authoritative but expensive (tens of milliseconds), so it is only
//!    asked when the backlight candidate changed, when no backlight node exists, or on the first
//!    poll. Some ROMs keep a non-zero backlight for the always-on display while the framework is
//!    already asleep, which is why the framework answer wins when the two disagree.
//!
//! An `unknown` reading is never a transition: the previous state is preserved and the next tick
//! retries. Turning a failed `dumpsys` into "screen off" would toggle the screen-off CPU profile
//! on a transient SELinux denial.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::sysfs::Sysfs;

/// Interactive state of the display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenState {
    On,
    Off,
    Unknown,
}

impl ScreenState {
    /// History sampling context: `1` interactive, `0` not, `None` when it cannot be determined.
    pub fn as_metric(self) -> Option<i64> {
        match self {
            ScreenState::On => Some(1),
            ScreenState::Off => Some(0),
            ScreenState::Unknown => None,
        }
    }
}

/// How long `dumpsys power` may take before it is killed.
const POWER_DUMP_TIMEOUT: Duration = Duration::from_secs(3);

/// A cached decision is re-checked against the framework at least this often, so a single wrong
/// answer (or a missed transition) cannot pin the display state for the life of the process.
const FRAMEWORK_RECHECK: Duration = Duration::from_secs(10);

/// Without a readable backlight node the framework is the only source; poll it faster while the
/// display is off so a wake-up is noticed quickly.
const FRAMEWORK_FAST_POLL: Duration = Duration::from_secs(2);

/// Never call `dumpsys power` more often than this, even on rapid brightness changes.
const FRAMEWORK_MIN_INTERVAL: Duration = Duration::from_secs(2);

/// Consecutive "off" answers required before the state flips to off.
///
/// Applying the screen-off CPU profile by mistake caps frequencies while the user is looking at the
/// screen; releasing it by mistake only restores normal limits. So "on" is immediate and "off" has
/// to be confirmed, which makes a single transient `Dozing` at wake-up harmless.
const OFF_CONFIRMATIONS: u8 = 2;

/// Stateful detector: caches the backlight node and the last decided state.
#[derive(Debug)]
pub struct ScreenWatcher {
    sysfs: Sysfs,
    backlight: Option<PathBuf>,
    /// Raw brightness seen at the last poll; any change is a hint to re-ask the framework.
    last_brightness: Option<u64>,
    last: Option<ScreenState>,
    last_check: Option<Instant>,
    /// Consecutive "off" answers not yet confirmed.
    off_streak: u8,
    /// Lower bound between framework calls; tests set it to zero so they can script answers.
    min_interval: Duration,
    power_source: PowerSource,
}

/// Where `dumpsys power` output comes from. Tests inject a scripted source so the parser and the
/// transition logic can be exercised on a host without an Android device.
#[derive(Clone)]
enum PowerSource {
    Command,
    Fixture(std::sync::Arc<dyn Fn() -> Option<String> + Send + Sync>),
}

impl std::fmt::Debug for PowerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PowerSource::Command => f.write_str("Command"),
            PowerSource::Fixture(_) => f.write_str("Fixture"),
        }
    }
}

impl ScreenWatcher {
    pub fn new(sysfs: Sysfs) -> Self {
        ScreenWatcher {
            sysfs,
            backlight: None,
            last_brightness: None,
            last: None,
            last_check: None,
            off_streak: 0,
            min_interval: FRAMEWORK_MIN_INTERVAL,
            power_source: PowerSource::Command,
        }
    }

    /// Test constructor: `power` replaces the real `dumpsys power` call.
    pub fn with_power_source(sysfs: Sysfs, power: fn() -> Option<String>) -> Self {
        ScreenWatcher {
            min_interval: Duration::ZERO,
            power_source: PowerSource::Fixture(std::sync::Arc::new(power)),
            ..ScreenWatcher::new(sysfs)
        }
    }

    /// Test constructor with a scripted sequence: one answer per poll, the last one repeating.
    #[cfg(test)]
    pub(crate) fn with_script(sysfs: Sysfs, answers: Vec<Option<&'static str>>) -> Self {
        use std::collections::VecDeque;
        use std::sync::{Arc, Mutex};
        let queue = Arc::new(Mutex::new(VecDeque::from(answers)));
        let power: Arc<dyn Fn() -> Option<String> + Send + Sync> = Arc::new(move || {
            let mut queue = queue.lock().ok()?;
            let answer = if queue.len() > 1 {
                queue.pop_front().flatten()
            } else {
                queue.front().copied().flatten()
            };
            answer.map(str::to_string)
        });
        ScreenWatcher {
            min_interval: Duration::ZERO,
            power_source: PowerSource::Fixture(power),
            ..ScreenWatcher::new(sysfs)
        }
    }

    /// Last decided state, without polling.
    pub fn last(&self) -> ScreenState {
        self.last.unwrap_or(ScreenState::Unknown)
    }

    /// Poll the display once.
    ///
    /// The backlight is a cheap *hint*: any change in the raw value — not merely crossing zero —
    /// re-asks the framework. That distinction matters: the old bucket-only check meant one transient
    /// `Off` answer at wake-up was cached forever, leaving the screen-off CPU profile applied while
    /// the user was looking at the screen. A periodic recheck and the off-confirmation below make the
    /// same failure impossible even if the backlight node is missing or frozen.
    pub fn detect(&mut self) -> ScreenState {
        let brightness = self.brightness();
        let now = Instant::now();
        let since_last = self.last_check.map(|checked| now.duration_since(checked));
        let min_ok = since_last.is_none_or(|elapsed| elapsed >= self.min_interval);
        let interval = if brightness.is_none() && self.last == Some(ScreenState::Off) {
            FRAMEWORK_FAST_POLL
        } else {
            FRAMEWORK_RECHECK
        };
        let due = since_last.is_none_or(|elapsed| elapsed >= interval);
        let changed = brightness != self.last_brightness;
        let ask = min_ok && (self.last.is_none() || self.off_streak > 0 || changed || due);

        if !ask {
            return self.last();
        }
        self.last_check = Some(now);
        self.last_brightness = brightness;

        match self.power_state() {
            Some(ScreenState::On) => {
                // Releasing the profile is always safe, so "on" applies immediately.
                self.off_streak = 0;
                self.last = Some(ScreenState::On);
                ScreenState::On
            }
            Some(ScreenState::Off) => {
                self.off_streak = self.off_streak.saturating_add(1);
                // The first observation is not a transition: there is no previous state to protect,
                // and a wrong initial answer is corrected by the periodic recheck. Only flipping an
                // already-awake device to "off" needs confirmation.
                let established = self.last.is_none();
                if !established && self.off_streak < OFF_CONFIRMATIONS {
                    // Keep the previous state; the next poll confirms (see `wants_fast_poll`).
                    return self.last();
                }
                self.off_streak = 0;
                self.last = Some(ScreenState::Off);
                ScreenState::Off
            }
            // `dumpsys` failed or printed nothing we understand. Unknown is never a transition:
            // guessing from the backlight here would apply (or release) the screen-off profile on
            // a transient SELinux denial or before the framework is up.
            Some(ScreenState::Unknown) | None => self.last(),
        }
    }

    /// True while an "off" reading still needs confirmation, so the caller can poll again quickly.
    pub fn wants_fast_poll(&self) -> bool {
        self.off_streak > 0
    }

    fn power_state(&self) -> Option<ScreenState> {
        let text = match self.power_source {
            PowerSource::Command => read_power_dump(POWER_DUMP_TIMEOUT)?,
            PowerSource::Fixture(ref fixture) => fixture()?,
        };
        match parse_power_dump(&text) {
            ScreenState::Unknown => None,
            state => Some(state),
        }
    }

    /// Brightness of the brightest backlight node, or `None` when the device has no usable node.
    fn brightness(&mut self) -> Option<u64> {
        if let Some(path) = &self.backlight {
            if let Some(value) = read_brightness(path) {
                return Some(value);
            }
            // The node disappeared (GPU driver rebind, module unload): rescan next time.
            self.backlight = None;
        }
        let path = self.scan_backlight()?;
        let value = read_brightness(&path)?;
        self.backlight = Some(path);
        Some(value)
    }

    /// Pick the node with the highest brightness, mirroring the old shell `bl_find`.
    fn scan_backlight(&self) -> Option<PathBuf> {
        let dir = self.sysfs.resolve("/sys/class/backlight");
        let entries = fs::read_dir(dir).ok()?;
        let mut best: Option<(u64, PathBuf)> = None;
        for entry in entries.flatten() {
            let path = entry.path().join("brightness");
            let Some(value) = read_brightness(&path) else {
                continue;
            };
            if best.as_ref().is_none_or(|(seen, _)| value > *seen) {
                best = Some((value, path));
            }
        }
        best.map(|(_, path)| path)
    }
}

fn read_brightness(path: &std::path::Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse::<u64>().ok()
}

/// Parse the interesting lines of `dumpsys power`.
///
/// Deliberately tolerant: OEM builds rename fields, so both `mIsInteractive: true` and
/// `mIsInteractive=true` are accepted, and `Wakefulness` only decides when the interactive flag is
/// absent.
pub fn parse_power_dump(text: &str) -> ScreenState {
    let mut interactive: Option<ScreenState> = None;
    let mut wakefulness: Option<ScreenState> = None;
    let mut display: Option<ScreenState> = None;
    for line in text.lines() {
        if let Some(rest) = strip_field(line, "mIsInteractive") {
            interactive = Some(if rest.starts_with("true") {
                ScreenState::On
            } else {
                ScreenState::Off
            });
        }
        if let Some(rest) = strip_field(line, "mWakefulness") {
            // `mWakefulnessChanging=false` also starts with `mWakefulness`; only a real state counts.
            if let Some(state) = match rest {
                "Awake" => Some(ScreenState::On),
                "Asleep" | "Dozing" => Some(ScreenState::Off),
                _ => None,
            } {
                wakefulness = Some(state);
            }
        }
        if display.is_none() {
            // Some builds print only `Display Power: state=ON/OFF`.
            if let Some(at) = line.find("Display Power") {
                if let Some(value) = line[at..].split("state=").nth(1) {
                    display = Some(if value.trim_start().starts_with("ON") {
                        ScreenState::On
                    } else if value.trim_start().starts_with("OFF") {
                        ScreenState::Off
                    } else {
                        ScreenState::Unknown
                    });
                }
            }
        }
    }
    interactive
        .or(wakefulness)
        .or(display)
        .unwrap_or(ScreenState::Unknown)
}

/// `mIsInteractive=true`, `mIsInteractive: true`, `mWakefulness=Awake` -> the value part.
fn strip_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.trim().strip_prefix(field)?;
    let rest = rest
        .trim_start()
        .strip_prefix(|c: char| c == ':' || c == '=')?;
    Some(rest.trim())
}

/// Run `dumpsys power`, giving up after `timeout` so a wedged framework cannot stall the daemon.
fn read_power_dump(timeout: Duration) -> Option<String> {
    let mut child = Command::new("dumpsys")
        .arg("power")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let output = child.wait_with_output().ok()?;
                return Some(String::from_utf8_lossy(&output.stdout).into_owned());
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn power_on() -> Option<String> {
        Some(ON.to_string())
    }

    fn power_off() -> Option<String> {
        Some(OFF.to_string())
    }

    const ON: &str =
        "Power Manager State:\n  mIsPowered=true\n  mWakefulness=Awake\n  mIsInteractive: true\n";
    const OFF: &str = "Power Manager State:\n  mWakefulness=Asleep\n  mIsInteractive: false\n";

    fn temp_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sysflow-screen-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp root");
        dir
    }

    #[test]
    fn parses_both_interactive_spellings() {
        assert_eq!(parse_power_dump(ON), ScreenState::On);
        assert_eq!(parse_power_dump(OFF), ScreenState::Off);
        assert_eq!(
            parse_power_dump("  mIsInteractive=true\n"),
            ScreenState::On,
            "colon-less field"
        );
        assert_eq!(
            parse_power_dump("  mWakefulness=Dozing\n"),
            ScreenState::Off,
            "dozing is not interactive"
        );
    }

    #[test]
    fn wakefulness_changing_is_not_a_state() {
        // `mWakefulnessChanging=false` starts with `mWakefulness`; reading it as an unknown state
        // used to swallow the real line that follows (or precedes) it.
        assert_eq!(
            parse_power_dump("  mWakefulnessChanging=false\n  mWakefulness=Awake\n"),
            ScreenState::On
        );
        assert_eq!(
            parse_power_dump("  mWakefulness=Awake\n  mWakefulnessChanging=true\n"),
            ScreenState::On
        );
    }

    #[test]
    fn display_power_state_is_a_fallback() {
        // Some builds print neither `mIsInteractive` nor `mWakefulness` in `dumpsys power`.
        assert_eq!(
            parse_power_dump("Display Power: state=ON\n"),
            ScreenState::On
        );
        assert_eq!(
            parse_power_dump("Display Power: state=OFF\n"),
            ScreenState::Off
        );
        // The framework answer wins over the display hint when both are present.
        assert_eq!(
            parse_power_dump("Display Power: state=OFF\n  mWakefulness=Awake\n"),
            ScreenState::On
        );
    }

    #[test]
    fn unknown_when_nothing_matches() {
        assert_eq!(parse_power_dump(""), ScreenState::Unknown);
        assert_eq!(parse_power_dump("garbage\n"), ScreenState::Unknown);
        assert_eq!(parse_power_dump("mWakefulness=Vr\n"), ScreenState::Unknown);
    }

    #[test]
    fn backlight_zero_means_off() {
        let root = temp_root("backlight-zero");
        let node = root.join("sys/class/backlight/panel");
        fs::create_dir_all(&node).unwrap();
        fs::write(node.join("brightness"), "0\n").unwrap();

        let mut watcher = ScreenWatcher::with_power_source(Sysfs::new(&root), power_off);
        assert_eq!(watcher.detect(), ScreenState::Off);
    }

    #[test]
    fn backlight_nonzero_means_on() {
        let root = temp_root("backlight-on");
        let node = root.join("sys/class/backlight/panel");
        fs::create_dir_all(&node).unwrap();
        fs::write(node.join("brightness"), "2047\n").unwrap();

        let mut watcher = ScreenWatcher::with_power_source(Sysfs::new(&root), power_on);
        assert_eq!(watcher.detect(), ScreenState::On);
    }

    #[test]
    fn framework_answer_wins_when_backlight_disagrees() {
        // Always-on display keeps a dim backlight while the framework is already asleep.
        let root = temp_root("aod");
        let node = root.join("sys/class/backlight/panel");
        fs::create_dir_all(&node).unwrap();
        fs::write(node.join("brightness"), "12\n").unwrap();

        let mut watcher = ScreenWatcher::with_power_source(Sysfs::new(&root), power_off);
        assert_eq!(watcher.detect(), ScreenState::Off);
    }

    #[test]
    fn framework_failure_is_not_a_transition() {
        let root = temp_root("transient");
        let node = root.join("sys/class/backlight/panel");
        fs::create_dir_all(&node).unwrap();
        fs::write(node.join("brightness"), "0\n").unwrap();

        let mut watcher = ScreenWatcher::with_power_source(Sysfs::new(&root), || None);
        assert_eq!(
            watcher.detect(),
            ScreenState::Unknown,
            "no source agrees yet"
        );

        // Once the framework answers, the transition is applied.
        watcher.power_source = PowerSource::Fixture(std::sync::Arc::new(power_off));
        assert_eq!(watcher.detect(), ScreenState::Off);
    }

    #[test]
    fn a_single_stale_off_does_not_flip_an_awake_device() {
        let root = temp_root("wake-transient");
        let node = root.join("sys/class/backlight/panel");
        fs::create_dir_all(&node).unwrap();
        fs::write(node.join("brightness"), "800\n").unwrap();

        // Awake device: one stale "off" answer arrives while the user is looking at the screen,
        // then the framework agrees it is awake again. The profile must not be applied.
        let mut watcher =
            ScreenWatcher::with_script(Sysfs::new(&root), vec![Some(ON), Some(OFF), Some(ON)]);
        assert_eq!(watcher.detect(), ScreenState::On);
        // Auto-brightness nudges the panel, which re-asks the framework; the answer is a stale "off".
        fs::write(node.join("brightness"), "900\n").unwrap();
        assert_eq!(
            watcher.detect(),
            ScreenState::On,
            "unconfirmed off must not flip"
        );
        assert!(
            watcher.wants_fast_poll(),
            "the confirmation must be retried quickly"
        );
        assert_eq!(watcher.detect(), ScreenState::On);
        assert!(!watcher.wants_fast_poll());
    }

    #[test]
    fn off_is_applied_after_two_consecutive_answers() {
        let root = temp_root("off-confirm");
        let node = root.join("sys/class/backlight/panel");
        fs::create_dir_all(&node).unwrap();
        fs::write(node.join("brightness"), "800\n").unwrap();

        let mut watcher =
            ScreenWatcher::with_script(Sysfs::new(&root), vec![Some(ON), Some(OFF), Some(OFF)]);
        assert_eq!(watcher.detect(), ScreenState::On);
        fs::write(node.join("brightness"), "0\n").unwrap();
        assert_eq!(watcher.detect(), ScreenState::On, "first off is not enough");
        assert_eq!(watcher.detect(), ScreenState::Off, "second off confirms");
    }

    #[test]
    fn a_changed_brightness_value_re_asks_the_framework() {
        // Regression: the bucket-only check used to cache one transient answer forever, so the
        // screen-off profile stayed applied while the screen was on.
        let root = temp_root("brightness-change");
        let node = root.join("sys/class/backlight/panel");
        fs::create_dir_all(&node).unwrap();
        fs::write(node.join("brightness"), "12\n").unwrap();

        // Always-on display: non-zero brightness while the framework is asleep.
        let mut watcher = ScreenWatcher::with_script(Sysfs::new(&root), vec![Some(OFF), Some(ON)]);
        assert_eq!(watcher.detect(), ScreenState::Off);
        // Wake-up: still "non-zero", but the value jumped, so the framework is asked again.
        fs::write(node.join("brightness"), "800\n").unwrap();
        assert_eq!(watcher.detect(), ScreenState::On);
    }

    #[test]
    fn unchanged_backlight_does_not_call_the_framework() {
        let root = temp_root("cache");
        let node = root.join("sys/class/backlight/panel");
        fs::create_dir_all(&node).unwrap();
        fs::write(node.join("brightness"), "900\n").unwrap();

        let mut watcher = ScreenWatcher::with_power_source(Sysfs::new(&root), power_on);
        assert_eq!(watcher.detect(), ScreenState::On);
        // Second poll: same brightness, so the expensive source must not be consulted.
        watcher.power_source = PowerSource::Fixture(std::sync::Arc::new(|| {
            panic!("cached state must be reused")
        }));
        assert_eq!(watcher.detect(), ScreenState::On);
    }
}
