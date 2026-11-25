//! debugfs mounting for the kernel wakeup-source table.
//!
//! Android stopped mounting `/sys/kernel/debug` by default, so the module mounts it on demand. The
//! mount itself is cheap, but it is a process spawn: a `snapshot` poll runs every couple of seconds
//! on the overview screen, so an attempt on every poll would be absurd. [`MountGate`] owns the
//! rate-limit rule and is testable on a host because it never talks to the kernel itself — the
//! caller passes the clock.

use std::path::{Path, PathBuf};

/// Where debugfs is conventionally mounted; the kernel table lives under it.
pub const DEBUGFS_ROOT: &str = "/sys/kernel/debug";

/// The kernel wakeup-source table inside [`DEBUGFS_ROOT`].
pub const TABLE_PATH: &str = "/sys/kernel/debug/wakeup_sources";

/// A failed mount is retried at most this often, so a SELinux denial cannot spawn a mount attempt
/// on every 2 s snapshot.
pub const MOUNT_RETRY_SECS: u64 = 3600;

/// Rate-limits mount attempts with a marker file next to the window state.
///
/// Only attempts that did **not** make the table readable are recorded. A successful mount is not
/// rate-limited, because it may be visible only inside the calling process's mount namespace: the
/// daemon is a different process and must still be able to mount its own.
pub struct MountGate {
    marker: PathBuf,
}

impl MountGate {
    pub fn new(state_dir: &Path) -> Self {
        MountGate {
            marker: state_dir.join("wakelock_debugfs.marker"),
        }
    }

    /// Path of the marker file. Used by tests to simulate a damaged or future-dated marker.
    #[cfg(test)]
    pub fn marker(&self) -> &Path {
        &self.marker
    }

    /// Whether an attempt is due at `now`.
    ///
    /// A missing or unparsable marker means "no record", which allows an attempt: losing the file
    /// must not disable the feature.
    pub fn should_attempt(&self, now: u64) -> bool {
        let Ok(text) = std::fs::read_to_string(&self.marker) else {
            return true;
        };
        let Ok(last) = text.trim().parse::<u64>() else {
            return true;
        };
        now.saturating_sub(last) >= MOUNT_RETRY_SECS
    }

    /// Record an attempt that left the table unreadable.
    pub fn record_unreadable(&self, now: u64) {
        if let Some(dir) = self.marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.marker, format!("{now}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sysflow-debugfs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_marker_allows_the_first_attempt() {
        let dir = temp_dir("missing");
        let gate = MountGate::new(&dir);
        assert!(gate.should_attempt(1_000));
    }

    #[test]
    fn a_recorded_attempt_is_rate_limited_for_an_hour() {
        let dir = temp_dir("rate");
        let gate = MountGate::new(&dir);
        gate.record_unreadable(1_000);

        assert!(!gate.should_attempt(1_000));
        assert!(!gate.should_attempt(1_000 + MOUNT_RETRY_SECS - 1));
        assert!(gate.should_attempt(1_000 + MOUNT_RETRY_SECS));
    }

    #[test]
    fn a_corrupt_marker_allows_an_attempt() {
        let dir = temp_dir("corrupt");
        let gate = MountGate::new(&dir);
        std::fs::write(gate.marker(), "not-a-timestamp\n").unwrap();
        assert!(
            gate.should_attempt(1_000),
            "a damaged marker must not disable the feature permanently"
        );
    }

    #[test]
    fn a_backwards_clock_does_not_block_forever() {
        let dir = temp_dir("clock");
        let gate = MountGate::new(&dir);
        // Recorded in the "future" relative to the next observation (wall clock moved back).
        gate.record_unreadable(10_000);
        // saturating_sub means the elapsed time is 0, so the retry window has not passed...
        assert!(!gate.should_attempt(9_000));
        // ...but an hour later it is due again, so the gate cannot wedge shut.
        assert!(gate.should_attempt(10_000 + MOUNT_RETRY_SECS));
    }
}
