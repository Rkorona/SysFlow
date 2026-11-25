//! The rolling window over the kernel's cumulative wakeup-source counters.
//!
//! Kernel counters are monotonic **since boot**, so "who kept the device awake in the last 24 h"
//! needs a stored baseline. This module owns that state, the delta arithmetic and the reboot
//! detection; the `wakelock` module only renders the result.
//!
//! The reboot case is the one that used to break the page: the state file survives a reboot while
//! the counters restart at zero, so every delta clamps to zero for the rest of the window and the
//! UI reported "the device was basically idle" for up to a day. Detection therefore relies on the
//! kernel's own boot identity first (`/proc/sys/kernel/random/boot_id`), the system uptime second
//! and a counter regression only as a last resort — in that order, because a wall-clock jump must
//! not be mistaken for a reboot and a single driver reload must not reset a 24 h window.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use sysflow_core::wakeup_sources::KernelSource;

/// Ranking window. Longer than this and the baseline is rolled forward.
pub const WINDOW_SECS: u64 = 24 * 60 * 60;

/// Slack when comparing system uptime with the window age: NTP can move the wall clock by a few
/// seconds after boot without a reboot having happened.
const REBOOT_MARGIN_SECS: u64 = 60;

/// Hard cap on tracked sources in the stored window. Driver churn over a day can add hundreds of
/// one-off sources, and the ranking only ever displays the top ones, so the state file must not grow
/// without bound.
const MAX_TRACKED_SOURCES: usize = 512;

/// One source's cumulative counters at a point in time.
///
/// Deserialization accepts a bare integer as well, which is the v1 state-file shape (milliseconds
/// only), so an upgrade does not throw away the running window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counter {
    pub ms: i64,
    pub wakeups: i64,
    pub events: i64,
}

impl Counter {
    pub fn from_source(source: &KernelSource) -> Self {
        Counter {
            ms: source.total_ms,
            wakeups: source.wakeup_count,
            events: source.event_count,
        }
    }
}

impl<'de> Deserialize<'de> for Counter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CounterVisitor;

        impl<'de> Visitor<'de> for CounterVisitor {
            type Value = Counter;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a millisecond counter or an object with ms/wakeups/events")
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Counter, E> {
                Ok(Counter {
                    ms: value,
                    wakeups: 0,
                    events: 0,
                })
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Counter, E> {
                Ok(Counter {
                    ms: value.min(i64::MAX as u64) as i64,
                    wakeups: 0,
                    events: 0,
                })
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Counter, A::Error> {
                #[derive(Deserialize)]
                struct Full {
                    #[serde(default)]
                    ms: i64,
                    #[serde(default)]
                    wakeups: i64,
                    #[serde(default)]
                    events: i64,
                }
                let full = Full::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(Counter {
                    ms: full.ms,
                    wakeups: full.wakeups,
                    events: full.events,
                })
            }
        }

        deserializer.deserialize_any(CounterVisitor)
    }
}

/// Why the current window started. Clients turn this into a sentence; unknown values fall back to
/// a generic label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowReason {
    /// First sample ever: nothing can be ranked until the next one.
    Fresh,
    /// The counters predate this boot, so the window restarts at the reboot.
    Reboot,
    /// The window aged past 24 h and slid forward to the previous observation.
    Rollover,
}

impl WindowReason {
    pub fn as_str(self) -> &'static str {
        match self {
            WindowReason::Fresh => "fresh",
            WindowReason::Reboot => "reboot",
            WindowReason::Rollover => "rollover",
        }
    }
}

/// The stored window: one baseline and the last observation, both keyed by source name.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WindowState {
    #[serde(default)]
    pub version: u32,
    /// When the baseline was taken; `0` means the window has not started yet.
    #[serde(default)]
    pub baseline_ts: u64,
    #[serde(default)]
    pub baseline: BTreeMap<String, Counter>,
    /// Counters as of the last sample.
    #[serde(default)]
    pub last: BTreeMap<String, Counter>,
    #[serde(default)]
    pub updated_ts: u64,
    /// Why the current window started; `None` for a state file written before v2.
    #[serde(default)]
    pub reason: Option<WindowReason>,
    /// Boot identity of the observation the window belongs to.
    #[serde(default)]
    pub boot_id: Option<String>,
}

/// One source's increment since the baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delta {
    pub name: String,
    pub ms: i64,
    pub wakeups: i64,
    pub events: i64,
}

impl WindowState {
    /// Fold one observation into the window.
    ///
    /// `uptime` and `boot_id` come from the kernel; either may be `None` when the node is not
    /// readable, and the detection degrades in the documented order. Returns `true` when the
    /// baseline was re-established (fresh, reboot or rollover), so callers can tell "nothing moved
    /// yet" from "nothing moved at all".
    pub fn advance(
        &mut self,
        sources: &[KernelSource],
        now: u64,
        uptime: Option<u64>,
        boot_id: Option<&str>,
    ) -> bool {
        let current: BTreeMap<String, Counter> = sources
            .iter()
            .map(|source| (source.name.clone(), Counter::from_source(source)))
            .collect();
        self.version = 2;

        let mut restarted = false;
        if self.baseline_ts == 0 {
            // First sample ever: the window starts here, seeded with what we just read, so the
            // next sample can subtract. Nothing can be ranked until then.
            self.baseline = current.clone();
            self.baseline_ts = now;
            self.reason = Some(WindowReason::Fresh);
            restarted = true;
        } else if now.saturating_sub(self.baseline_ts) >= WINDOW_SECS {
            // A day has passed: roll the baseline forward to the previous observation instead of
            // dropping the first 24 h of data that we do have.
            self.baseline = self.last.clone();
            self.baseline_ts = now;
            self.reason = Some(WindowReason::Rollover);
            restarted = true;
        } else if self.rebooted(now, uptime, boot_id, &current) {
            // Counters are since-boot: with a pre-boot baseline every delta clamps to zero for the
            // rest of the window, which is worse than losing the window and saying so.
            self.baseline = current.clone();
            self.baseline_ts = now;
            self.reason = Some(WindowReason::Reboot);
            restarted = true;
        }

        if let Some(id) = boot_id {
            self.boot_id = Some(id.to_string());
        }
        self.last = current;
        if self.baseline.len() > MAX_TRACKED_SOURCES {
            // Keep only sources still present in this observation: they are the only ones that can
            // ever grow a delta again.
            let live = &self.last;
            self.baseline.retain(|name, _| live.contains_key(name));
        }
        self.updated_ts = now;
        restarted
    }

    /// Whether the stored baseline belongs to a previous boot.
    fn rebooted(
        &self,
        now: u64,
        uptime: Option<u64>,
        boot_id: Option<&str>,
        current: &BTreeMap<String, Counter>,
    ) -> bool {
        // Primary: the kernel names the boot. Exact, and immune to clock changes.
        if let (Some(known), Some(seen)) = (self.boot_id.as_deref(), boot_id) {
            return known != seen;
        }

        // Secondary: a window older than the current uptime cannot belong to this boot. A clock
        // jump forward looks the same, but rebaselining is the cheap failure mode — the alternative
        // is a window that reports zeros for a day.
        let age = now.saturating_sub(self.baseline_ts);
        if let Some(up) = uptime {
            return up.saturating_add(REBOOT_MARGIN_SECS) < age;
        }

        // Last resort: the counters themselves went backwards. Only a *widespread* regression is a
        // reboot; a driver that re-registers one source must not throw the window away.
        let base_total: i64 = self.baseline.values().map(|counter| counter.ms).sum();
        let current_total: i64 = current.values().map(|counter| counter.ms).sum();
        base_total > 0 && current_total.saturating_mul(2) < base_total
    }

    /// Time the window actually covers, never more than the current boot.
    pub fn window_s(&self, now: u64, uptime: Option<u64>) -> u64 {
        if self.baseline_ts == 0 {
            return 0;
        }
        let age = now.saturating_sub(self.baseline_ts).min(WINDOW_SECS);
        match uptime {
            Some(up) => age.min(up),
            None => age,
        }
    }

    /// Per-source increments since the baseline, biggest first.
    ///
    /// Counters that went backwards clamp to zero instead of producing a negative "time held"; a
    /// source without a baseline was added after the window started and has no known increment.
    pub fn deltas(&self) -> Vec<Delta> {
        let mut out: Vec<Delta> = self
            .last
            .iter()
            .filter_map(|(name, current)| {
                let baseline = self.baseline.get(name)?;
                let ms = (current.ms - baseline.ms).max(0);
                let wakeups = (current.wakeups - baseline.wakeups).max(0);
                let events = (current.events - baseline.events).max(0);
                (ms > 0).then(|| Delta {
                    name: name.clone(),
                    ms,
                    wakeups,
                    events,
                })
            })
            .collect();
        out.sort_by(|a, b| b.ms.cmp(&a.ms).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// The reason to report to clients, defaulting for state written before v2.
    pub fn reason(&self) -> &'static str {
        match self.reason {
            Some(reason) => reason.as_str(),
            None if self.baseline_ts == 0 => "fresh",
            None => "baseline",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(name: &str, total_ms: i64, wakeups: i64) -> KernelSource {
        KernelSource {
            name: name.to_string(),
            active_count: 0,
            event_count: wakeups,
            wakeup_count: wakeups,
            expire_count: 0,
            active_since_ms: 0,
            total_ms,
            max_ms: 0,
            last_change_ms: 0,
            prevent_suspend_ms: 0,
        }
    }

    fn state_at(now: u64, sources: &[KernelSource]) -> WindowState {
        let mut state = WindowState::default();
        state.advance(sources, now, Some(now), Some("boot-a"));
        state
    }

    #[test]
    fn first_sample_is_fresh_and_ranks_nothing() {
        let now = 1_000_000;
        let state = state_at(now, &[source("src", 100, 1)]);
        assert_eq!(state.reason(), "fresh");
        assert!(state.deltas().is_empty());
        assert_eq!(state.window_s(now, Some(now)), 0);
        assert_eq!(state.version, 2);
    }

    #[test]
    fn deltas_rank_by_time_and_carry_counts() {
        let now = 1_000_000;
        let mut state = state_at(now, &[source("alpha", 1_000, 2), source("beta", 500, 1)]);
        state.advance(
            &[source("alpha", 4_000, 9), source("beta", 500, 1)],
            now + 300,
            Some(now + 300),
            Some("boot-a"),
        );
        let deltas = state.deltas();
        assert_eq!(deltas.len(), 1, "beta did not move");
        assert_eq!(deltas[0].name, "alpha");
        assert_eq!(deltas[0].ms, 3_000);
        assert_eq!(deltas[0].wakeups, 7);
        let measured: i64 = state.deltas().iter().map(|delta| delta.ms).sum();
        assert_eq!(measured, 3_000);
    }

    #[test]
    fn a_new_boot_id_restarts_the_window() {
        let now = 1_000_000;
        let mut state = state_at(now, &[source("src", 5_000, 10)]);
        // Counters are back near zero after a reboot, and the kernel names a new boot.
        let restarted = state.advance(
            &[source("src", 40, 1)],
            now + 600,
            Some(600),
            Some("boot-b"),
        );
        assert!(restarted, "a new boot id must rebaseline");
        assert_eq!(state.reason(), "reboot");
        assert!(
            state.deltas().is_empty(),
            "no increment since the reboot yet"
        );
        assert_eq!(state.window_s(now + 600, Some(600)), 0);
    }

    #[test]
    fn a_short_uptime_against_an_old_window_is_a_reboot() {
        // v1 state carries no boot id, so uptime is the only signal.
        let now = 1_000_000;
        let mut state = WindowState {
            version: 1,
            baseline_ts: now - 7_200,
            baseline: [(
                "src".to_string(),
                Counter {
                    ms: 9_000,
                    ..Counter::default()
                },
            )]
            .into_iter()
            .collect(),
            last: BTreeMap::new(),
            updated_ts: now - 7_200,
            reason: None,
            boot_id: None,
        };
        state.advance(&[source("src", 30, 1)], now, Some(120), None);
        assert_eq!(state.reason(), "reboot");
    }

    #[test]
    fn a_clock_jump_with_a_long_uptime_is_not_a_reboot() {
        // Wall clock jumped 3 h forward, but the device has been up for a day: same boot.
        let now = 1_000_000;
        let mut state = state_at(now, &[source("src", 1_000, 1)]);
        let restarted = state.advance(
            &[source("src", 2_000, 1)],
            now + 10_800,
            Some(97_200),
            Some("boot-a"),
        );
        assert!(!restarted, "a clock jump must not discard the window");
        assert_eq!(state.deltas()[0].ms, 1_000);
    }

    #[test]
    fn a_widespread_counter_regression_is_a_reboot_without_other_signals() {
        let now = 1_000_000;
        let mut state = WindowState {
            baseline_ts: now - 600,
            baseline: [
                (
                    "a".to_string(),
                    Counter {
                        ms: 5_000,
                        ..Counter::default()
                    },
                ),
                (
                    "b".to_string(),
                    Counter {
                        ms: 3_000,
                        ..Counter::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..WindowState::default()
        };
        state.advance(&[source("a", 20, 0), source("b", 10, 0)], now, None, None);
        assert_eq!(state.reason(), "reboot");
    }

    #[test]
    fn one_reloaded_driver_does_not_reset_the_window() {
        let now = 1_000_000;
        let mut state = WindowState {
            baseline_ts: now - 600,
            baseline: [
                (
                    "a".to_string(),
                    Counter {
                        ms: 5_000,
                        ..Counter::default()
                    },
                ),
                (
                    "b".to_string(),
                    Counter {
                        ms: 3_000,
                        ..Counter::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..WindowState::default()
        };
        // `b` re-registered (its counter restarted), `a` kept growing; total did not collapse.
        state.advance(
            &[source("a", 6_000, 0), source("b", 10, 0)],
            now,
            None,
            None,
        );
        assert_ne!(state.reason(), "reboot");
        assert_eq!(state.deltas()[0].name, "a");
        assert_eq!(state.deltas()[0].ms, 1_000);
    }

    #[test]
    fn a_window_older_than_a_day_rolls_forward() {
        let now = 1_000_000;
        let mut state = WindowState {
            baseline_ts: now - WINDOW_SECS - 60,
            baseline: [(
                "src".to_string(),
                Counter {
                    ms: 1_000,
                    ..Counter::default()
                },
            )]
            .into_iter()
            .collect(),
            last: [(
                "src".to_string(),
                Counter {
                    ms: 1_500,
                    ..Counter::default()
                },
            )]
            .into_iter()
            .collect(),
            ..WindowState::default()
        };
        state.advance(
            &[source("src", 2_000, 0)],
            now,
            Some(400_000),
            Some("boot-a"),
        );
        assert_eq!(state.reason(), "rollover");
        assert_eq!(
            state.deltas()[0].ms,
            500,
            "rolls to the previous observation"
        );
    }

    #[test]
    fn window_is_capped_by_the_current_boot() {
        let state = WindowState {
            baseline_ts: 1_000,
            ..WindowState::default()
        };
        assert_eq!(state.window_s(10_000, Some(300)), 300);
        assert_eq!(state.window_s(10_000, None), 9_000);
    }

    #[test]
    fn v1_state_files_still_load() {
        let text = r#"{"version":1,"baseline_ts":1000,"baseline":{"src":1234},
                       "last":{"src":1500},"updated_ts":1000}"#;
        let state: WindowState = serde_json::from_str(text).unwrap();
        assert_eq!(state.baseline["src"].ms, 1_234);
        assert_eq!(state.baseline["src"].wakeups, 0);
        assert_eq!(state.reason(), "baseline");
    }

    #[test]
    fn stale_baselines_are_pruned_when_the_window_grows_too_large() {
        let now = 1_000_000;
        let mut state = WindowState {
            baseline_ts: now - 600,
            baseline: (0..MAX_TRACKED_SOURCES + 100)
                .map(|index| {
                    (
                        format!("stale{index}"),
                        Counter {
                            ms: 10,
                            ..Counter::default()
                        },
                    )
                })
                .chain(std::iter::once((
                    "live".to_string(),
                    Counter {
                        ms: 1,
                        ..Counter::default()
                    },
                )))
                .collect(),
            ..WindowState::default()
        };
        // A readable uptime keeps this out of the reboot branch, so pruning is what gets exercised.
        state.advance(&[source("live", 1, 0)], now, Some(now), None);
        assert!(state.baseline.len() <= MAX_TRACKED_SOURCES);
        assert!(state.baseline.contains_key("live"));
        assert!(!state.baseline.contains_key("stale0"));
    }
}
