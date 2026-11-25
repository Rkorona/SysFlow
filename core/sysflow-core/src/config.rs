use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{from_io, Error, Result};

pub const CONFIG_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct BootConfig {
    pub enabled: bool,
    /// Seconds `apply-boot` waits before applying (gives the system time to settle).
    pub delay_s: u32,
}

impl Default for BootConfig {
    fn default() -> Self {
        BootConfig {
            enabled: true,
            delay_s: 30,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SafeConfig {
    /// Consecutive boots whose tuning was applied but never confirmed.
    pub boot_count: u32,
    /// When boot_count reaches this at the start of apply-boot, enter safe mode.
    pub threshold: u32,
    pub safe_mode: bool,
}

impl Default for SafeConfig {
    fn default() -> Self {
        SafeConfig {
            boot_count: 0,
            threshold: 3,
            safe_mode: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub boot: BootConfig,
    pub safe: SafeConfig,
    /// module id -> { item id -> persisted value }
    pub modules: Map<String, Value>,
    /// module id -> { item id -> value before SysFlow first touched it }
    pub defaults_snapshot: Map<String, Value>,
    /// Settings automatically isolated after repeated unconfirmed boots. This replaces the old
    /// per-file `.bad` backups while keeping the JSON config atomic.
    pub recovery_backup: Option<Map<String, Value>>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CONFIG_VERSION,
            boot: BootConfig::default(),
            safe: SafeConfig::default(),
            modules: Map::new(),
            defaults_snapshot: Map::new(),
            recovery_backup: None,
        }
    }
}

fn migrate(c: &mut Config) {
    // Future migrations go here, one step per version.
    if c.version < CONFIG_VERSION {
        c.version = CONFIG_VERSION;
    }
}

/// Advisory lock implemented as an exclusive-create lock file.
/// Stale locks (older than 10 s) are stolen so a crashed process can't wedge the config.
struct FileLock {
    path: PathBuf,
}

impl FileLock {
    fn acquire(path: PathBuf, timeout: Duration) -> Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| from_io(&e, "config dir"))?;
        }
        let start = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(FileLock { path }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .map(|age| age > Duration::from_secs(10))
                        .unwrap_or(false);
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed() > timeout {
                        return Err(Error::busy("config is locked by another process"));
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(from_io(&e, "config lock")),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Clone, Debug)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        ConfigStore { path }
    }

    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("json.lock")
    }

    /// Missing file -> defaults. Unparseable file -> moved aside as `.corrupt`, defaults.
    pub fn load(&self) -> Result<Config> {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(from_io(&e, "config")),
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(mut c) => {
                if c.version > CONFIG_VERSION {
                    return Err(Error::io(format!(
                        "config version {} is newer than supported {}",
                        c.version, CONFIG_VERSION
                    )));
                }
                migrate(&mut c);
                Ok(c)
            }
            Err(_) => {
                let _ = fs::rename(&self.path, self.path.with_extension("json.corrupt"));
                Ok(Config::default())
            }
        }
    }

    /// Atomic save: temp file + fsync + rename.
    pub fn save(&self, c: &Config) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| from_io(&e, "config dir"))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(c).map_err(|e| Error::io(e.to_string()))?;
        {
            let mut f = fs::File::create(&tmp).map_err(|e| from_io(&e, "config tmp"))?;
            f.write_all(text.as_bytes())
                .map_err(|e| from_io(&e, "config tmp"))?;
            f.sync_all().map_err(|e| from_io(&e, "config tmp"))?;
        }
        fs::rename(&tmp, &self.path).map_err(|e| from_io(&e, "config rename"))
    }

    /// Locked read-modify-write. Keep the closure short and free of device I/O.
    pub fn update<R>(&self, f: impl FnOnce(&mut Config) -> Result<R>) -> Result<R> {
        let _guard = FileLock::acquire(self.lock_path(), Duration::from_secs(5))?;
        let mut c = self.load()?;
        let r = f(&mut c)?;
        self.save(&c)?;
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p =
            std::env::temp_dir().join(format!("sysflow-cfg-{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tmp("missing");
        let store = ConfigStore::new(dir.join("config.json"));
        let c = store.load().unwrap();
        assert_eq!(c.version, CONFIG_VERSION);
        assert_eq!(c.safe.threshold, 3);
        assert!(c.boot.enabled);
    }

    #[test]
    fn update_roundtrips() {
        let dir = tmp("roundtrip");
        let store = ConfigStore::new(dir.join("config.json"));
        store
            .update(|c| {
                c.safe.boot_count = 2;
                Ok(())
            })
            .unwrap();
        assert_eq!(store.load().unwrap().safe.boot_count, 2);
        assert!(!dir.join("config.json.lock").exists());
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let dir = tmp("corrupt");
        let path = dir.join("config.json");
        fs::write(&path, "{ not json").unwrap();
        let store = ConfigStore::new(path.clone());
        let c = store.load().unwrap();
        assert_eq!(c.safe.boot_count, 0);
        assert!(dir.join("config.json.corrupt").exists());
    }

    #[test]
    fn recovery_backup_roundtrips_and_old_configs_default_it() {
        let dir = tmp("recovery");
        let path = dir.join("config.json");
        // A pre-recovery config must still deserialize using serde defaults.
        fs::write(
            &path,
            r#"{"version":1,"modules":{"cpu":{"cpu.max_freq":{"0":1800000}}}}"#,
        )
        .unwrap();
        let store = ConfigStore::new(path.clone());
        assert!(store.load().unwrap().recovery_backup.is_none());
        store
            .update(|cfg| {
                cfg.recovery_backup = Some(cfg.modules.clone());
                cfg.modules.clear();
                Ok(())
            })
            .unwrap();
        let loaded = store.load().unwrap();
        assert!(loaded.modules.is_empty());
        assert!(loaded.recovery_backup.as_ref().unwrap().contains_key("cpu"));
    }

    #[test]
    fn newer_version_is_rejected() {
        let dir = tmp("newer");
        let path = dir.join("config.json");
        fs::write(&path, r#"{"version": 99}"#).unwrap();
        let store = ConfigStore::new(path);
        assert!(store.load().is_err());
    }
}
