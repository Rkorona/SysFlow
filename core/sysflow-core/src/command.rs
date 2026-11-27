//! Run an external command with a kill-after-deadline watchdog.
//!
//! Every privileged shell-out in SysFlow goes through here: `dumpsys`, `cmd appops`, `cmd package`.
//! They are binder calls into the system server, so they can take hundreds of milliseconds and, if
//! the server is wedged, never return. A bare `Command::output()` would hang the daemon with it.
//!
//! The watchdog waits on a channel instead of sleeping the whole timeout, so a fast command does not
//! pay for the deadline.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::error::{Error, Result};

/// Default allowance for a shell-out.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);

/// Run `bin args...`, returning its stdout, killing it after `timeout`.
///
/// A killed command is reported as an `io` timeout error rather than as empty output, so callers can
/// tell "nothing to report" from "the call failed".
pub fn run(bin: &str, args: &[&str], timeout: Duration) -> Result<String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Error::io(format!("cannot run {bin} {}: {e}", args.join(" "))))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::io(format!("{bin} produced no stdout")))?;
    let guard = Arc::new(Mutex::new(Some(child)));
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let watchdog = {
        let guard = Arc::clone(&guard);
        thread::spawn(move || match done_rx.recv_timeout(timeout) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(mut slot) = guard.lock() {
                    if let Some(child) = slot.as_mut() {
                        let _ = child.kill();
                    }
                }
            }
        })
    };

    let mut text = String::new();
    let read = std::io::Read::read_to_string(&mut stdout, &mut text);
    let _ = done_tx.send(());
    let _ = watchdog.join();
    read.map_err(|e| Error::io(format!("read {bin} output: {e}")))?;

    let status = match guard.lock() {
        Ok(mut slot) => slot.take().and_then(|mut child| child.wait().ok()),
        Err(_) => None,
    };
    match status {
        Some(status) if status.success() => Ok(text),
        Some(status) => Err(Error::io(format!(
            "{bin} {} exited with {status}",
            args.join(" ")
        ))),
        // Only reachable if the watchdog panicked; non-empty output is still usable.
        None if !text.is_empty() => Ok(text),
        None => Err(Error::io(format!("{bin} was killed after the timeout"))),
    }
}
