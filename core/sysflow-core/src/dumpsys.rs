//! `dumpsys` helpers.
//!
//! `dumpsys` is the only source for several values (battery health fallbacks, per-app power
//! attribution, userspace wake locks). It is a Java service call, so every invocation goes through
//! the shared watchdog in [`crate::command`] rather than a bare `Command::output()`.

use std::time::Duration;

use crate::command;
use crate::error::Result;

/// Default allowance for a `dumpsys` subcommand.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);

/// Run `dumpsys <args>` and return its stdout.
pub fn run(args: &[&str]) -> Result<String> {
    run_with_timeout(args, DEFAULT_TIMEOUT)
}

/// Run `dumpsys <args>`, killing the child when it exceeds `timeout`.
pub fn run_with_timeout(args: &[&str], timeout: Duration) -> Result<String> {
    command::run("dumpsys", args, timeout)
}
