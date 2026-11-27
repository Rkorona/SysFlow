//! `sysflow storage partitions` — whole-device mount overview.
//!
//! This is the honest answer to "how full is the whole phone": each real filesystem with its
//! `statvfs` numbers, pseudo filesystems and duplicate views (`/storage/*`, `/mnt/*`) filtered out.

use serde_json::Value;

use sysflow_core::error::Result;
use sysflow_core::registry::Registry;

use sysflow_modules::storage::partitions;

pub fn run(reg: &Registry) -> Result<Value> {
    partitions::partitions(&reg.ctx().sysfs)
}
