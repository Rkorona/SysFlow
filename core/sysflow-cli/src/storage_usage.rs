//! `sysflow storage usage` — bounded directory-size ranking.
//!
//! This is the heavy, on-demand half of the storage page: it walks a directory tree with a time
//! budget and reports the top-level entries by allocated bytes. It is never part of `snapshot`.

use serde_json::Value;

use sysflow_core::error::{Error, Result};
use sysflow_core::registry::Registry;

use sysflow_modules::storage::usage::{self, UsageOptions};

pub fn run(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let path = flag(rest, "--path").unwrap_or("/data").to_string();
    let limit = flag(rest, "--limit")
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| Error::invalid_arg(format!("--limit expects a number, got {value:?}")))
        })
        .transpose()?
        .unwrap_or(usage::DEFAULT_LIMIT);
    let budget_ms = flag(rest, "--budget-ms")
        .map(|value| {
            value.parse::<u64>().map_err(|_| {
                Error::invalid_arg(format!("--budget-ms expects a number, got {value:?}"))
            })
        })
        .transpose()?
        .unwrap_or(usage::DEFAULT_BUDGET_MS);

    let options = UsageOptions {
        path,
        limit,
        budget_ms,
    };
    if options.path == "/" {
        // `/` is the read-only rootfs plus a set of mounts; a plain walk would see almost nothing.
        return usage::scan_root(&reg.ctx().sysfs, &options);
    }
    usage::scan(&options)
}

fn flag<'a>(args: &[&'a str], name: &str) -> Option<&'a str> {
    let position = args.iter().position(|arg| *arg == name)?;
    args.get(position + 1).copied()
}
