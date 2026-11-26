//! `sysflow storage trim` — one-shot `fstrim -v /data`.
//!
//! TRIM is intentionally a verb, not a persisted item: it is a maintenance action, and replaying it
//! at boot or through drift would be both useless and surprising. Only `/data` is accepted; trimming
//! an arbitrary path the user typed is not a risk worth taking.

use serde_json::Value;

use sysflow_core::error::{Error, Result};
use sysflow_core::registry::Registry;

use sysflow_modules::storage;

pub fn run(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let data_path = sysflow_modules::storage::default_data_path();
    let path = flag(rest, "--path").unwrap_or(&data_path);
    if path != "/data" && path != "/storage/emulated/0" && path != data_path {
        return Err(Error::invalid_arg(
            "storage trim only accepts /data (or /storage/emulated/0)",
        ));
    }
    if !storage::trim_available(&reg.ctx().sysfs) {
        return Err(Error::unsupported("this ROM does not ship fstrim"));
    }
    storage::trim(&reg.ctx().sysfs, path)
}

fn flag<'a>(args: &[&'a str], name: &str) -> Option<&'a str> {
    let position = args.iter().position(|arg| *arg == name)?;
    args.get(position + 1).copied()
}
