//! Time-series history for module samples.
//!
//! One append-only JSONL file per module under `<data_dir>/history/`, because the phase-2 plan asked
//! for something a human can inspect with `tail`/`grep` before optimising for size. Each line is
//! `{"ts":<unix seconds>,"v":{...}}`.
//!
//! Retention is enforced on append: an age cutoff plus a hard line cap. Pruning rewrites the file,
//! which is acceptable at this volume (a module writing one sample a minute reaches the cap only
//! after weeks) and keeps reads simple.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use crate::error::Result;
use crate::module::Sample;
use crate::screen::ScreenState;

/// Samples older than this are dropped.
pub const RETENTION_SECS: u64 = 30 * 24 * 3600;
/// Hard cap per module; one sample a minute is ~44 000 lines in 30 days, so this trims the longest.
pub const MAX_LINES: usize = 20_000;

pub struct HistoryStore {
    dir: PathBuf,
}

impl HistoryStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        HistoryStore { dir: dir.into() }
    }

    fn path(&self, module: &str) -> PathBuf {
        self.dir.join(format!("{module}.jsonl"))
    }

    /// Append one sample and apply retention. Failure must never break the sampling loop, so
    /// callers treat this as best-effort telemetry.
    pub fn append(&self, module: &str, ts: u64, sample: &Sample) -> Result<()> {
        self.append_with_screen(module, ts, sample, ScreenState::Unknown)
    }

    /// Append a sample together with the display state observed at sample time.
    ///
    /// Screen state belongs to the sampling context, not to a module's metrics, so it is stored as
    /// a top-level `screen` field (`1` interactive, `0` not) rather than inside `v`. It is omitted
    /// when the daemon could not determine it, which keeps old files and unknown-state samples
    /// readable. Readers surface it as a `screen` series (see `read`).
    pub fn append_with_screen(
        &self,
        module: &str,
        ts: u64,
        sample: &Sample,
        screen: ScreenState,
    ) -> Result<()> {
        if sample.is_empty() {
            return Ok(());
        }
        // A clock jump (or a bad caller) must not inject a future timestamp: it would pin every
        // "latest value" query to the bogus sample forever.
        let now = now_secs();
        if ts > now.saturating_add(300) {
            return Ok(());
        }
        fs::create_dir_all(&self.dir)?;
        let path = self.path(module);
        let mut record = json!({ "ts": ts, "v": Value::Object(sample.clone()) });
        if let Some(flag) = screen.as_metric() {
            record["screen"] = json!(flag);
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(file, "{record}")?;
        drop(file);
        self.prune(module, ts)?;
        Ok(())
    }

    fn prune(&self, module: &str, now: u64) -> Result<()> {
        let path = self.path(module);
        let Ok(text) = fs::read_to_string(&path) else {
            return Ok(());
        };
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        let cutoff = now.saturating_sub(RETENTION_SECS);
        let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
        for line in &lines {
            // A truncated line from a power loss is dropped rather than poisoning the file.
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                let ts = value.get("ts").and_then(Value::as_u64).unwrap_or(0);
                if ts >= cutoff {
                    kept.push(line);
                }
            }
        }
        if kept.len() > MAX_LINES {
            kept.drain(0..kept.len() - MAX_LINES);
        }
        if kept.len() == lines.len() {
            return Ok(());
        }
        let tmp = path.with_extension("jsonl.tmp");
        let mut out = fs::File::create(&tmp)?;
        for line in kept {
            writeln!(out, "{line}")?;
        }
        out.sync_all()?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Points newer than `since_ts`, downsampled to at most one point per `step_s` bucket.
    ///
    /// Each bucket takes the **last** sample in it, which is what a chart wants: the most recent
    /// observation of that interval rather than an average that hides peaks.
    pub fn read(&self, module: &str, since_ts: u64, step_s: u64) -> Result<Vec<SamplePoint>> {
        let path = self.path(module);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(_) => return Ok(Vec::new()),
        };
        let step = step_s.max(1);
        let mut buckets: BTreeMap<u64, (u64, Map<String, Value>)> = BTreeMap::new();
        for line in BufReader::new(file)
            .lines()
            .map_while(std::result::Result::ok)
        {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let ts = value.get("ts").and_then(Value::as_u64).unwrap_or(0);
            if ts < since_ts {
                continue;
            }
            let Some(metrics) = value.get("v").and_then(Value::as_object) else {
                continue;
            };
            // The sampling context rides next to the metrics; surface it as one more series so a
            // client can split battery drain by display state without a second query.
            let mut metrics = metrics.clone();
            if let Some(screen) = value.get("screen").and_then(Value::as_i64) {
                metrics.insert("screen".to_string(), json!(screen));
            }
            let bucket = ts / step * step;
            // Keep the newest sample of the bucket. The file is append-only and normally
            // chronological, but a clock adjustment can reorder entries, so compare timestamps
            // instead of trusting read order.
            match buckets.get(&bucket).map(|(existing, _)| *existing) {
                Some(existing) if existing >= ts => {}
                _ => {
                    buckets.insert(bucket, (ts, metrics));
                }
            }
        }
        Ok(buckets
            .into_iter()
            .map(|(bucket, (ts, v))| {
                let _ = bucket;
                SamplePoint { ts, v }
            })
            .collect())
    }

    /// Column-oriented series keyed by metric, the shape `sysflow history` promises.
    pub fn series(&self, module: &str, since_ts: u64, step_s: u64) -> Result<Value> {
        let points = self.read(module, since_ts, step_s)?;
        let mut series: Map<String, Value> = Map::new();
        for point in &points {
            for (metric, value) in &point.v {
                let entry = series
                    .entry(metric.clone())
                    .or_insert_with(|| Value::Array(Vec::new()));
                if let Value::Array(list) = entry {
                    list.push(json!([point.ts, value]));
                }
            }
        }
        Ok(json!({
            "module": module,
            "step_s": step_s,
            "points": points.len(),
            "series": Value::Object(series),
        }))
    }

    /// Newest sample at or before `ts`, used for "drain since the last full charge" style stats.
    pub fn last_before(&self, module: &str, ts: u64) -> Result<Option<SamplePoint>> {
        let points = self.read(module, 0, 1)?;
        Ok(points.into_iter().rev().find(|point| point.ts <= ts))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SamplePoint {
    pub ts: u64,
    pub v: Map<String, Value>,
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse `90s`, `5m`, `24h`, `7d` into seconds. Bare numbers are seconds.
pub fn parse_duration(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (digits, factor) = match text.chars().last()? {
        's' => (&text[..text.len() - 1], 1u64),
        'm' => (&text[..text.len() - 1], 60),
        'h' => (&text[..text.len() - 1], 3600),
        'd' => (&text[..text.len() - 1], 86_400),
        _ => (text, 1),
    };
    digits
        .parse::<u64>()
        .ok()
        .map(|value| value.saturating_mul(factor))
        .filter(|secs| *secs > 0)
}

/// Remove every module's history (used by `reset --all` and diagnostics).
pub fn clear_all(dir: &Path) -> Result<usize> {
    let mut removed = 0;
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(0),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Convenience for the daemon: append, and report which module/metrics were stored.
pub fn record(store: &HistoryStore, module: &str, sample: &Sample) -> Result<usize> {
    store.append(module, now_secs(), sample)?;
    Ok(sample.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sysflow-history-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample(level: i64) -> Sample {
        let mut s = Map::new();
        s.insert("level".into(), json!(level));
        s.insert("temp".into(), json!(400));
        s
    }

    #[test]
    fn appends_and_reads_back() {
        let dir = temp_dir("append");
        let store = HistoryStore::new(&dir);
        let now = now_secs();
        store.append("battery", now - 120, &sample(80)).unwrap();
        store.append("battery", now - 60, &sample(79)).unwrap();
        store.append("battery", now, &sample(78)).unwrap();

        let points = store.read("battery", 0, 1).unwrap();
        assert_eq!(points.len(), 3);
        assert_eq!(points[2].v["level"], json!(78));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_only_the_last_sample_per_bucket() {
        let dir = temp_dir("downsample");
        let store = HistoryStore::new(&dir);
        let now = now_secs();
        // Align to a bucket boundary so all five samples land in the same 300 s bucket.
        let base = now / 300 * 300;
        for offset in [290u64, 280, 270, 260, 250] {
            store
                .append("battery", base + offset, &sample(offset as i64))
                .unwrap();
        }
        let points = store.read("battery", 0, 300).unwrap();
        assert_eq!(points.len(), 1);
        // The last sample in the bucket wins, not the first.
        assert_eq!(points[0].v["level"], json!(290));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn filters_by_since() {
        let dir = temp_dir("since");
        let store = HistoryStore::new(&dir);
        let now = now_secs();
        store.append("battery", now - 1000, &sample(90)).unwrap();
        store.append("battery", now - 10, &sample(80)).unwrap();
        let points = store.read("battery", now - 100, 1).unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].v["level"], json!(80));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let dir = temp_dir("missing");
        let store = HistoryStore::new(&dir);
        assert!(store.read("battery", 0, 60).unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn series_is_column_oriented() {
        let dir = temp_dir("series");
        let store = HistoryStore::new(&dir);
        let now = now_secs();
        store.append("battery", now - 60, &sample(80)).unwrap();
        store.append("battery", now, &sample(79)).unwrap();
        let value = store.series("battery", 0, 1).unwrap();
        let series = value["series"].as_object().unwrap();
        let levels = series["level"].as_array().unwrap();
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1][1], json!(79));
        assert_eq!(value["module"], json!("battery"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn records_screen_state_as_sampling_context() {
        let dir = temp_dir("screen");
        let store = HistoryStore::new(&dir);
        let now = now_secs();
        store
            .append_with_screen("battery", now - 60, &sample(80), ScreenState::Off)
            .unwrap();
        store
            .append_with_screen("battery", now, &sample(79), ScreenState::On)
            .unwrap();
        // The legacy entry point records no context, so the series simply lacks that point.
        store.append("battery", now - 30, &sample(795)).unwrap();

        let series = store.series("battery", 0, 1).unwrap();
        let screen = series["series"]["screen"].as_array().unwrap();
        assert_eq!(
            screen.len(),
            2,
            "only samples with a known state carry screen"
        );
        assert_eq!(screen[0][1], json!(0));
        assert_eq!(screen[1][1], json!(1));

        // The metrics themselves are untouched by the extra context field.
        let levels = series["series"]["level"].as_array().unwrap();
        assert_eq!(levels.len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_screen_state_is_omitted_from_the_file() {
        let dir = temp_dir("screen-unknown");
        let store = HistoryStore::new(&dir);
        store
            .append_with_screen("battery", now_secs(), &sample(70), ScreenState::Unknown)
            .unwrap();
        let raw = fs::read_to_string(dir.join("battery.jsonl")).unwrap();
        assert!(!raw.contains("\"screen\""), "raw line: {raw}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prunes_lines_beyond_the_cap() {
        let dir = temp_dir("cap");
        let store = HistoryStore::new(&dir);
        let now = now_secs();
        // Write one over the cap by pre-seeding the file.
        let path = dir.join("bulk.jsonl");
        let mut text = String::new();
        for i in 0..(MAX_LINES + 5) {
            // Strictly increasing and safely in the past, so only the line cap bites.
            let ts = now - (MAX_LINES + 20) as u64 + i as u64;
            let level = i;
            text.push_str(&format!("{{\"ts\":{ts},\"v\":{{\"level\":{level}}}}}\n"));
        }
        fs::write(&path, text).unwrap();
        store.append("bulk", now, &sample(50)).unwrap();

        let lines = fs::read_to_string(&path).unwrap().lines().count();
        assert!(lines <= MAX_LINES + 1, "kept {lines} lines");
        let points = store.read("bulk", 0, 1).unwrap();
        // The newest sample must survive the trim.
        assert_eq!(points.last().unwrap().v["level"], json!(50));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn drops_samples_older_than_the_retention_window() {
        let dir = temp_dir("retention");
        let store = HistoryStore::new(&dir);
        let now = now_secs();
        store
            .append("battery", now - RETENTION_SECS - 100, &sample(10))
            .unwrap();
        store.append("battery", now, &sample(20)).unwrap();
        let points = store.read("battery", 0, 1).unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].v["level"], json!(20));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_empty_samples() {
        let dir = temp_dir("empty");
        let store = HistoryStore::new(&dir);
        store.append("battery", now_secs(), &Map::new()).unwrap();
        assert!(store.read("battery", 0, 1).unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("90s"), Some(90));
        assert_eq!(parse_duration("5m"), Some(300));
        assert_eq!(parse_duration("24h"), Some(86_400));
        assert_eq!(parse_duration("7d"), Some(604_800));
        assert_eq!(parse_duration("120"), Some(120));
        assert_eq!(parse_duration("0s"), None);
        assert_eq!(parse_duration("abc"), None);
    }
}
