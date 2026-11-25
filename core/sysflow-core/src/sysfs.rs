use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{from_io, Error, Result};

/// Thin sysfs/procfs access layer. `root` lets tests point at a fake tree.
#[derive(Clone, Debug)]
pub struct Sysfs {
    root: PathBuf,
}

impl Sysfs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Sysfs { root: root.into() }
    }

    pub fn from_env() -> Self {
        let root = std::env::var_os("SYSFLOW_SYSFS_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        Sysfs { root }
    }

    /// Absolute device path ("/sys/...") -> path under the configured root.
    pub fn resolve(&self, p: &str) -> PathBuf {
        self.root.join(p.trim_start_matches('/'))
    }

    pub fn exists(&self, p: &str) -> bool {
        self.resolve(p).exists()
    }

    pub fn read_str(&self, p: &str) -> Result<String> {
        fs::read_to_string(self.resolve(p))
            .map(|s| s.trim().to_string())
            .map_err(|e| from_io(&e, p))
    }

    /// Read as UTF-8, replacing invalid sequences instead of failing.
    ///
    /// Kernel pseudo-files (wakeup-source tables, some vendor nodes) occasionally contain bytes that
    /// are not valid UTF-8; `read_to_string` would reject the whole table, which looks like "the
    /// feature is unsupported" when it is only one malformed byte.
    pub fn read_lossy(&self, p: &str) -> Result<String> {
        let bytes = fs::read(self.resolve(p)).map_err(|e| from_io(&e, p))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn read_i64(&self, p: &str) -> Result<i64> {
        let s = self.read_str(p)?;
        s.parse::<i64>()
            .map_err(|_| Error::io(format!("{p}: not an integer: {s:?}")))
    }

    pub fn read_u64(&self, p: &str) -> Result<u64> {
        let s = self.read_str(p)?;
        s.parse::<u64>()
            .map_err(|_| Error::io(format!("{p}: not an unsigned integer: {s:?}")))
    }

    /// Write without creating: the node must already exist.
    ///
    /// Some ROMs (Samsung in particular) make cpufreq nodes read-only (0444) and refuse
    /// the write even for root. On `PermissionDenied` the mode is relaxed to 0644 for the
    /// duration of the write and then restored, as the shell implementation did.
    pub fn write_str(&self, p: &str, value: &str) -> Result<()> {
        let path = self.resolve(p);
        match Self::write_file(&path, value) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                Self::write_with_chmod(&path, value).map_err(|e2| from_io(&e2, p))
            }
            Err(e) => Err(from_io(&e, p)),
        }
    }

    fn write_file(path: &Path, value: &str) -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .truncate(true) // ignored by sysfs, but keeps fake trees in tests honest
            .open(path)?;
        f.write_all(value.as_bytes())
    }

    #[cfg(unix)]
    fn write_with_chmod(path: &Path, value: &str) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let old = fs::metadata(path)?.permissions();
        let mut open = old.clone();
        open.set_mode(0o644);
        fs::set_permissions(path, open)?;
        let res = Self::write_file(path, value);
        let _ = fs::set_permissions(path, old);
        res
    }

    #[cfg(not(unix))]
    fn write_with_chmod(_path: &Path, _value: &str) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
    }

    /// Write, then read back. Callers compare the returned string to what they
    /// asked for: the kernel (or thermal HAL) may have refused or changed it.
    pub fn write_verified(&self, p: &str, value: &str) -> Result<String> {
        self.write_str(p, value)?;
        self.read_str(p)
    }

    /// Sorted entry names of a directory.
    pub fn list_dir(&self, p: &str) -> Result<Vec<String>> {
        let rd = fs::read_dir(self.resolve(p)).map_err(|e| from_io(&e, p))?;
        let mut out = Vec::new();
        for ent in rd {
            let ent = ent.map_err(|e| from_io(&e, p))?;
            out.push(ent.file_name().to_string_lossy().into_owned());
        }
        out.sort();
        Ok(out)
    }

    /// Permission bits of a node (`0o644`), used to re-apply the WALT write guard after a reboot
    /// or after the ROM reset the node modes.
    #[cfg(unix)]
    pub fn mode(&self, p: &str) -> Result<u32> {
        use std::os::unix::fs::PermissionsExt;
        let md = fs::metadata(self.resolve(p)).map_err(|e| from_io(&e, p))?;
        Ok(md.permissions().mode() & 0o777)
    }

    #[cfg(not(unix))]
    pub fn mode(&self, _p: &str) -> Result<u32> {
        Err(Error::unsupported(
            "file modes are not available on this platform",
        ))
    }

    /// Change permission bits without changing the contents. Only used on nodes whose write
    /// access is part of the feature (WALT guard); writing values goes through `write_str`.
    #[cfg(unix)]
    pub fn set_mode(&self, p: &str, mode: u32) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(self.resolve(p), fs::Permissions::from_mode(mode))
            .map_err(|e| from_io(&e, p))
    }

    #[cfg(not(unix))]
    pub fn set_mode(&self, _p: &str, _mode: u32) -> Result<()> {
        Err(Error::unsupported(
            "file modes are not available on this platform",
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn read_only_node_is_written_and_mode_restored() {
        let dir = std::env::temp_dir().join(format!("sysflow-sysfs-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let node = dir.join("scaling_max_freq");
        fs::write(&node, "1800000").unwrap();
        fs::set_permissions(&node, fs::Permissions::from_mode(0o444)).unwrap();

        let sysfs = Sysfs::new(&dir);
        sysfs.write_str("/scaling_max_freq", "1500000").unwrap();

        assert_eq!(sysfs.read_str("/scaling_max_freq").unwrap(), "1500000");
        let mode = fs::metadata(&node).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o444);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_node_is_unsupported_not_created() {
        let dir =
            std::env::temp_dir().join(format!("sysflow-sysfs-test-missing-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let sysfs = Sysfs::new(&dir);
        assert!(sysfs.write_str("/nope", "1").is_err());
        assert!(!dir.join("nope").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
