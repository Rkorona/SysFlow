use std::path::PathBuf;

use crate::sysfs::Sysfs;

/// Runtime paths. All overridable by env vars so everything can be tested on a host.
#[derive(Clone, Debug)]
pub struct Ctx {
    /// SYSFLOW_DATA_DIR, default /data/adb/sysflow
    pub data_dir: PathBuf,
    /// SYSFLOW_MODULE_DIR, default /data/adb/modules/sysflow
    pub module_dir: PathBuf,
    /// SYSFLOW_SYSFS_ROOT, default /
    pub sysfs: Sysfs,
}

impl Ctx {
    pub fn from_env() -> Self {
        let data_dir = std::env::var_os("SYSFLOW_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/data/adb/sysflow"));
        let module_dir = std::env::var_os("SYSFLOW_MODULE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/data/adb/modules/sysflow"));
        Ctx {
            data_dir,
            module_dir,
            sysfs: Sysfs::from_env(),
        }
    }

    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn log_path(&self) -> PathBuf {
        self.data_dir.join("log").join("sysflow.log")
    }
}
