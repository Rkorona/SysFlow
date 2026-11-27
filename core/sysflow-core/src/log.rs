use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use crate::util::now_secs;

const MAX_BYTES: u64 = 512 * 1024;
const KEEP: usize = 3;

/// Minimal best-effort file logger with size-based rotation. Never fails the caller.
#[derive(Clone, Debug)]
pub struct Logger {
    path: PathBuf,
}

impl Logger {
    pub fn new(path: PathBuf) -> Self {
        Logger { path }
    }

    pub fn info(&self, msg: &str) {
        self.write("INFO", msg);
    }

    pub fn warn(&self, msg: &str) {
        self.write("WARN", msg);
    }

    pub fn error(&self, msg: &str) {
        self.write("ERROR", msg);
    }

    fn write(&self, level: &str, msg: &str) {
        let _ = self.try_write(level, msg);
    }

    fn numbered(&self, n: usize) -> PathBuf {
        let mut s = self.path.clone().into_os_string();
        s.push(format!(".{n}"));
        PathBuf::from(s)
    }

    fn rotate(&self) {
        for i in (1..KEEP).rev() {
            let _ = fs::rename(self.numbered(i), self.numbered(i + 1));
        }
        let _ = fs::rename(&self.path, self.numbered(1));
    }

    fn try_write(&self, level: &str, msg: &str) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        if let Ok(md) = fs::metadata(&self.path) {
            if md.len() >= MAX_BYTES {
                self.rotate();
            }
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = msg.replace('\n', " ");
        writeln!(f, "{} {} {}", now_secs(), level, line)
    }

    /// Last `n` lines of the current log file (empty if there is none).
    pub fn tail(&self, n: usize) -> Vec<String> {
        match fs::read_to_string(&self.path) {
            Ok(text) => {
                let lines: Vec<&str> = text.lines().collect();
                let start = lines.len().saturating_sub(n);
                lines[start..].iter().map(|s| s.to_string()).collect()
            }
            Err(_) => Vec::new(),
        }
    }
}
