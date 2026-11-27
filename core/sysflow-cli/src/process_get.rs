//! `sysflow process get <pid>` — one process in detail.

use serde_json::Value;

use sysflow_core::error::{Error, Result};
use sysflow_core::registry::Registry;

use sysflow_modules::process::scan;

pub fn run(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let pid = rest
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .ok_or_else(|| Error::invalid_arg("process get requires a PID"))?
        .parse::<u32>()
        .map_err(|_| Error::invalid_arg("process get expects a numeric PID"))?;
    scan::detail(&reg.ctx().sysfs, &reg.ctx().data_dir, pid)
}
