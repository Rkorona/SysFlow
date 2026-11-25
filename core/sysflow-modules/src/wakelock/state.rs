//! Persistence of the rolling window: `<data_dir>/state/wakelock.json`.
//!
//! The file is tiny and written once per sample, but it is the only thing that survives a reboot, so
//! it is written atomically: a killed daemon must never leave half a baseline behind.

use std::path::{Path, PathBuf};

use super::window::WindowState;

/// File name inside the module state directory.
pub const FILE_NAME: &str = "wakelock.json";

pub fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join(FILE_NAME)
}

/// The window state file.
pub struct StateFile {
    path: PathBuf,
}

impl StateFile {
    pub fn new(state_dir: &Path) -> Self {
        StateFile {
            path: state_path(state_dir),
        }
    }

    /// Path of the state file. Used by tests to seed a pre-existing window.
    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the window, or a default one when the file is missing or unreadable.
    ///
    /// A corrupt file is treated as "no window yet" rather than an error: losing the ranking is
    /// recoverable, refusing to start the module is not.
    pub fn load(&self) -> WindowState {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return WindowState::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Replace the state atomically so a killed daemon cannot leave half a baseline behind.
    pub fn store(&self, state: &WindowState) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(body) = serde_json::to_string(state) else {
            return;
        };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wakelock::window::WindowState;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sysflow-wakelock-state-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_file_loads_a_default_window() {
        let dir = temp_dir("missing");
        let file = StateFile::new(&dir);
        let state = file.load();
        assert_eq!(state.baseline_ts, 0);
        assert!(state.baseline.is_empty());
    }

    #[test]
    fn store_then_load_round_trips() {
        let dir = temp_dir("roundtrip");
        let file = StateFile::new(&dir);
        let state = WindowState {
            baseline_ts: 1_234,
            reason: Some(crate::wakelock::window::WindowReason::Reboot),
            ..WindowState::default()
        };
        file.store(&state);

        let loaded = file.load();
        assert_eq!(loaded.baseline_ts, 1_234);
        assert_eq!(loaded.reason(), "reboot");
        assert!(!dir.join("wakelock.json.tmp").exists(), "no temp file left");
    }

    #[test]
    fn a_corrupt_file_loads_a_default_window() {
        let dir = temp_dir("corrupt");
        let file = StateFile::new(&dir);
        std::fs::write(file.path(), b"{not json").unwrap();
        assert_eq!(
            file.load().baseline_ts,
            0,
            "a corrupt state file must not fail the module"
        );
    }
}
