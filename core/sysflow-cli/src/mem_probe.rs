//! `sysflow mem probe` — hidden diagnostic: raw readings and the module's judgement of them.
//!
//! The filesystem half lives in `sysflow_modules::mem::probe` (unit-tested on a fake tree). This verb
//! adds the sources that need a shell-out — `getprop`, `getconf`, and the three places a kill record
//! might live — which cannot be tested on a host. Output is for a developer reading a device, so it
//! is not part of the protocol and may change; nothing here is cached.
//!
//! Kill-record sources are reported as **counts plus at most three sample lines**, never whole dumps:
//! `logcat` and `dmesg` can be large and may name apps.

use std::time::Duration;

use serde_json::{json, Value};

use sysflow_core::command;
use sysflow_core::error::Result;
use sysflow_core::registry::Registry;
use sysflow_modules::mem::probe;

const SHORT: Duration = Duration::from_secs(3);
const DUMP: Duration = Duration::from_secs(8);
const SAMPLE_LINES: usize = 3;
const SAMPLE_WIDTH: usize = 200;

pub fn run(reg: &Registry) -> Result<Value> {
    let mut report = probe::report(&reg.ctx().sysfs);
    report["commands"] = json!({
        "low_ram": single("getprop", &["ro.config.low_ram"]),
        "page_size": single("getconf", &["PAGESIZE"]),
        "dmesg": scan(
            "dmesg",
            &[],
            &["lowmemorykiller", "lmkd", "out of memory", "oom-kill", "killed process"],
            None,
        ),
        "logcat_events": scan("logcat", &["-b", "events", "-d"], &["am_kill", "lmk"], None),
        // The record layout (where the timestamp and package sit relative to the reason line) is
        // what Step 4 needs, so this one also returns the lines around the last match.
        "exit_info": scan(
            "dumpsys",
            &["activity", "exit-info"],
            &["low memory", "low_memory", "lmkd"],
            Some((10, 2)),
        ),
    });
    Ok(report)
}

/// A command whose whole (short) output is the answer.
fn single(bin: &str, args: &[&str]) -> Value {
    match command::run(bin, args, SHORT) {
        Ok(text) => json!({ "state": "ok", "value": text.trim() }),
        Err(err) => json!({ "state": "error", "error": err.msg }),
    }
}

/// A command whose output is large: report its size and how many lines match `needles`.
fn scan(bin: &str, args: &[&str], needles: &[&str], context: Option<(usize, usize)>) -> Value {
    let text = match command::run(bin, args, DUMP) {
        Ok(text) => text,
        Err(err) => return json!({ "state": "error", "error": err.msg }),
    };
    let matching: Vec<&str> = text
        .lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            needles.iter().any(|needle| lower.contains(needle))
        })
        .collect();
    let samples: Vec<String> = matching
        .iter()
        .rev()
        .take(SAMPLE_LINES)
        .map(|line| line.chars().take(SAMPLE_WIDTH).collect())
        .collect();
    let mut out = json!({
        "state": if text.trim().is_empty() { "empty" } else { "ok" },
        "bytes": text.len(),
        "lines": text.lines().count(),
        "matching_lines": matching.len(),
        // Last in the output, which is not necessarily the newest in time (`exit-info` groups by
        // package), hence "last" rather than "newest".
        "last_matches": samples,
    });
    if let Some((before, after)) = context {
        let lines: Vec<&str> = text.lines().collect();
        let last = lines.iter().rposition(|line| {
            let lower = line.to_lowercase();
            needles.iter().any(|needle| lower.contains(needle))
        });
        if let Some(index) = last {
            let from = index.saturating_sub(before);
            let to = (index + after + 1).min(lines.len());
            out["last_match_context"] = json!(lines[from..to]
                .iter()
                .map(|line| line.chars().take(SAMPLE_WIDTH).collect::<String>())
                .collect::<Vec<_>>());
        }
    }
    out
}
