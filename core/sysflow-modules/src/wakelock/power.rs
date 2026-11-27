//! `dumpsys power` behind an injectable runner.
//!
//! The long-held-lock watchdog needs the userspace wake-lock table, which only `dumpsys power`
//! provides. Injecting the runner keeps the watchdog testable on a host with no Android tooling, and
//! lets the `with_runner` constructor avoid spawning `dumpsys` at all.

use std::sync::Arc;
use std::time::Duration;

use sysflow_core::dumpsys;
use sysflow_core::error::{Error, Result};

/// Same budget as the `wakelock held` verb: never stall the daemon on a wedged system server.
const TIMEOUT: Duration = Duration::from_secs(8);

/// Returns the `dumpsys power` text.
pub type DumpRunner = dyn Fn() -> Result<String> + Send + Sync;

pub struct Power {
    runner: Arc<DumpRunner>,
}

impl Power {
    /// Production runner. On a host there is no `dumpsys`, so it degrades to
    /// [`Power::unavailable`] instead of spawning a missing binary on every sample.
    pub fn real() -> Self {
        if !cfg!(target_os = "android") {
            return Power::unavailable();
        }
        Power {
            runner: Arc::new(|| dumpsys::run_with_timeout(&["power"], TIMEOUT)),
        }
    }

    /// A runner that always fails: used by tests that do not exercise the watchdog.
    pub fn unavailable() -> Self {
        Power {
            runner: Arc::new(|| Err(Error::io("dumpsys power is not available here"))),
        }
    }

    /// Test hook: a runner that returns canned `dumpsys power` text.
    #[cfg(test)]
    pub fn with_runner(runner: Arc<DumpRunner>) -> Self {
        Power { runner }
    }

    pub fn dump(&self) -> Result<String> {
        (self.runner)()
    }
}
