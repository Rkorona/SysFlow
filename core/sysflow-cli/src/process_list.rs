//! `sysflow process list` — the process page's data source.
//!
//! All the parsing and sampling lives in `sysflow_modules::process::scan`; this file only turns the
//! command line into [`ListOptions`]. That split keeps the behaviour testable with a fake `/proc`
//! and keeps the CLI free of `/proc` knowledge.

use serde_json::Value;

use sysflow_core::error::{Error, Result};
use sysflow_core::registry::Registry;

use sysflow_modules::process::scan::{self, Filter, ListOptions, SortKey};

pub fn run(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let limit = match flag(rest, "--limit") {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| Error::invalid_arg(format!("--limit expects a number, got {value:?}")))?,
        None => scan::DEFAULT_LIMIT,
    };
    let sort = match flag(rest, "--sort") {
        Some(value) => SortKey::parse(value).ok_or_else(|| {
            Error::invalid_arg(format!("--sort expects cpu|rss|name|pid, got {value:?}"))
        })?,
        None => SortKey::Cpu,
    };
    let filter = match flag(rest, "--filter") {
        Some(value) => Filter::parse(value).ok_or_else(|| {
            Error::invalid_arg(format!("--filter expects all|user|system, got {value:?}"))
        })?,
        None => Filter::All,
    };
    let sample_ms = match flag(rest, "--sample-ms") {
        Some(value) => value
            .parse::<u64>()
            .map_err(|_| {
                Error::invalid_arg(format!("--sample-ms expects milliseconds, got {value:?}"))
            })?
            .clamp(scan::MIN_SAMPLE_MS, scan::MAX_SAMPLE_MS),
        None => scan::DEFAULT_SAMPLE_MS,
    };

    let options = ListOptions {
        limit: limit.clamp(1, scan::MAX_LIMIT),
        sort,
        filter,
        query: flag(rest, "--query").map(str::to_string),
        sample_ms,
        with_foreground: !rest.contains(&"--no-foreground"),
    };
    Ok(scan::list(&reg.ctx().sysfs, &reg.ctx().data_dir, &options))
}

/// `--flag value` (the CLI style used by the other verbs; no `--flag=value` form).
fn flag<'a>(args: &[&'a str], name: &str) -> Option<&'a str> {
    let position = args.iter().position(|arg| *arg == name)?;
    args.get(position + 1).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_returns_the_value_after_the_name() {
        let args = ["--limit", "10", "--sort", "rss"];
        assert_eq!(flag(&args, "--limit"), Some("10"));
        assert_eq!(flag(&args, "--sort"), Some("rss"));
        assert_eq!(flag(&args, "--query"), None);
        assert_eq!(flag(&["--limit"], "--limit"), None);
    }
}
