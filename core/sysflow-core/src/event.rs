//! Module-owned, extensible system event journal.
//!
//! Events are stored as one immutable JSON file each. This avoids shared-file append races when
//! the module daemon, service scripts and user-triggered CLI commands emit concurrently.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{Error, Result};

const RETENTION_DAYS: u64 = 30;
const MAX_EVENTS: usize = 5_000;
const PRUNE_INTERVAL_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_TITLE_CHARS: usize = 120;
const MAX_MESSAGE_CHARS: usize = 1_000;
const MAX_METADATA_BYTES: usize = 16 * 1024;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemEvent {
    pub id: String,
    pub module: String,
    pub category: String,
    #[serde(rename = "event_type")]
    pub event_type: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub timestamp_ms: u64,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NewSystemEvent {
    pub module: String,
    pub category: String,
    pub event_type: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub metadata: Value,
    pub dedupe_key: Option<String>,
    pub dedupe_window_s: u64,
}

impl NewSystemEvent {
    pub fn new(
        module: impl Into<String>,
        category: impl Into<String>,
        event_type: impl Into<String>,
        severity: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        Self {
            module: module.into(),
            category: category.into(),
            event_type: event_type.into(),
            severity: severity.into(),
            title: title.into(),
            message: String::new(),
            metadata: json!({}),
            dedupe_key: None,
            dedupe_window_s: 0,
        }
    }

    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    pub fn metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn dedupe(mut self, key: impl Into<String>, window_s: u64) -> Self {
        self.dedupe_key = Some(key.into());
        self.dedupe_window_s = window_s;
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct EventFilter<'a> {
    pub limit: usize,
    pub after_ms: Option<u64>,
    pub after_id: Option<&'a str>,
    pub before_ms: Option<u64>,
    pub before_id: Option<&'a str>,
    pub ascending: bool,
    pub module: Option<&'a str>,
    pub category: Option<&'a str>,
    pub severity: Option<&'a str>,
}

#[derive(Clone, Debug)]
pub struct EventStore {
    directory: PathBuf,
}

impl EventStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            directory: data_dir.join("events"),
        }
    }

    /// The directory this store actually reads and writes.
    ///
    /// Exposed because "the app and a terminal disagree about the journal" is almost always a path
    /// question, and a client needs to be able to print the answer it got.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Strict emission path used by external CLI callers (`sysflow event emit`).
    pub fn emit(&self, input: NewSystemEvent) -> Result<(SystemEvent, bool)> {
        validate(&input)?;
        fs::create_dir_all(&self.directory)
            .map_err(|e| Error::io(format!("create event store: {e}")))?;
        let timestamp_ms = now_millis();

        if let Some(key) = input.dedupe_key.as_deref() {
            if input.dedupe_window_s > 0 {
                let threshold =
                    timestamp_ms.saturating_sub(input.dedupe_window_s.saturating_mul(1_000));
                if let Some(existing) = self.find_duplicate(&input, key, threshold)? {
                    return Ok((existing, true));
                }
            }
        }

        let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let id = format!(
            "{}-{:010}-{:020}",
            timestamp_ms,
            std::process::id(),
            sequence
        );
        let event = SystemEvent {
            id: id.clone(),
            module: input.module,
            category: input.category,
            event_type: input.event_type,
            severity: input.severity,
            title: input.title,
            message: input.message,
            timestamp_ms,
            metadata: input.metadata,
            dedupe_key: input.dedupe_key,
        };
        let bytes =
            serde_json::to_vec(&event).map_err(|e| Error::io(format!("encode event: {e}")))?;
        let final_path = self.directory.join(format!("{id}.json"));
        let temporary_path = self.directory.join(format!(".{id}.tmp"));
        let write_result = (|| -> std::io::Result<()> {
            let mut file = fs::File::create(&temporary_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary_path, &final_path)?;
            // Persist the directory entry when the filesystem supports directory fsync.
            let _ = fs::File::open(&self.directory).and_then(|directory| directory.sync_all());
            Ok(())
        })();
        if let Err(e) = write_result {
            let _ = fs::remove_file(&temporary_path);
            return Err(Error::io(format!("commit event: {e}")));
        }
        // Avoid a full journal scan on every transition. Enforce the hard count cap immediately,
        // and run age-based retention at most once per day (also at module startup).
        let over_limit = self
            .count_records()
            .map(|count| count > MAX_EVENTS)
            .unwrap_or(false);
        let marker = self.directory.join(".last_prune_ms");
        let last_prune = fs::read_to_string(&marker)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(0);
        if over_limit || timestamp_ms.saturating_sub(last_prune) >= PRUNE_INTERVAL_MS {
            let _ = self.prune(RETENTION_DAYS, MAX_EVENTS);
        }
        Ok((event, false))
    }

    /// Best-effort emission path for internal Rust producers (`Registry`, `CpuModule`) so long
    /// diagnostic error messages or payloads are safely clamped rather than rejected.
    pub fn emit_sanitized(&self, mut input: NewSystemEvent) -> Result<(SystemEvent, bool)> {
        input.module = clamp_token(&input.module, 64);
        input.category = clamp_token(&input.category, 64);
        input.event_type = clamp_token(&input.event_type, 96);
        if !["info", "notice", "warning", "error", "critical"].contains(&input.severity.as_str()) {
            input.severity = "info".to_string();
        }
        input.title = clamp_text(&input.title, MAX_TITLE_CHARS);
        if input.title.is_empty() {
            input.title = "System event".to_string();
        }
        input.message = clamp_text(&input.message, MAX_MESSAGE_CHARS);
        input.metadata = clamp_metadata(input.metadata);
        input.dedupe_key = input.dedupe_key.map(|k| clamp_token(&k, 128));
        self.emit(input)
    }

    /// Scan only recent `.json` files whose filename timestamp falls within `threshold_ms`,
    /// avoiding opening and deserializing all retained records on every deduplicated event.
    fn find_duplicate(
        &self,
        input: &NewSystemEvent,
        key: &str,
        threshold_ms: u64,
    ) -> Result<Option<SystemEvent>> {
        if !self.directory.exists() {
            return Ok(None);
        }
        let mut candidates = Vec::new();
        for entry in fs::read_dir(&self.directory)
            .map_err(|e| Error::io(format!("read event store: {e}")))?
        {
            let entry = entry.map_err(|e| Error::io(format!("read event entry: {e}")))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let ts = stem.split('-').next().and_then(|p| p.parse::<u64>().ok());
            if let Some(t) = ts {
                if t < threshold_ms {
                    continue;
                }
            }
            candidates.push((ts.unwrap_or(u64::MAX), stem.to_string(), path));
        }
        candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));

        let mut best: Option<SystemEvent> = None;
        for (_, _, path) in candidates {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(event) = serde_json::from_str::<SystemEvent>(&text) else {
                continue;
            };
            if event.timestamp_ms < threshold_ms {
                continue;
            }
            if event.module == input.module
                && event.category == input.category
                && event.event_type == input.event_type
                && event.dedupe_key.as_deref() == Some(key)
            {
                let replace = best
                    .as_ref()
                    .map(|prev| {
                        event.timestamp_ms > prev.timestamp_ms
                            || (event.timestamp_ms == prev.timestamp_ms && event.id > prev.id)
                    })
                    .unwrap_or(true);
                if replace {
                    best = Some(event);
                }
            }
        }
        Ok(best)
    }

    pub fn list(&self, filter: EventFilter<'_>) -> Result<Vec<SystemEvent>> {
        if !self.directory.exists() {
            return Ok(Vec::new());
        }
        // IDs begin with a millisecond timestamp and then a padded process/sequence.
        // Sort by parsed timestamp first and deserialize only the requested page, rather than
        // opening all 5,000 JSON records on each foreground sync.
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.directory)
            .map_err(|e| Error::io(format!("read event store: {e}")))?
        {
            let entry = entry.map_err(|e| Error::io(format!("read event entry: {e}")))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                paths.push(path);
            }
        }
        paths.sort_by(|a, b| a.file_stem().cmp(&b.file_stem()));
        if !filter.ascending {
            paths.reverse();
        }

        let limit = filter.limit.clamp(1, 500);
        let mut events = Vec::with_capacity(limit);
        for path in paths {
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let id_timestamp = id
                .split('-')
                .next()
                .and_then(|part| part.parse::<u64>().ok());
            if let Some(after_ms) = filter.after_ms {
                let Some(timestamp) = id_timestamp else {
                    continue;
                };
                if timestamp < after_ms
                    || (timestamp == after_ms
                        && !filter.after_id.map(|cursor| id > cursor).unwrap_or(false))
                {
                    continue;
                }
            }
            if let Some(before_ms) = filter.before_ms {
                let Some(timestamp) = id_timestamp else {
                    continue;
                };
                if timestamp > before_ms
                    || (timestamp == before_ms
                        && !filter.before_id.map(|cursor| id < cursor).unwrap_or(false))
                {
                    continue;
                }
            }
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(_) => continue,
            };
            let event = match serde_json::from_str::<SystemEvent>(&text) {
                Ok(event) => event,
                Err(_) => continue,
            };
            if filter
                .module
                .map(|value| event.module != value)
                .unwrap_or(false)
                || filter
                    .category
                    .map(|value| event.category != value)
                    .unwrap_or(false)
                || filter
                    .severity
                    .map(|value| event.severity != value)
                    .unwrap_or(false)
            {
                continue;
            }
            events.push(event);
            if events.len() >= limit {
                break;
            }
        }
        Ok(events)
    }

    pub fn prune(&self, retention_days: u64, max_events: usize) -> Result<Value> {
        fs::create_dir_all(&self.directory)
            .map_err(|e| Error::io(format!("create event store: {e}")))?;
        let mut events = self.read_all()?;
        events.sort_by_key(|event| std::cmp::Reverse(event.timestamp_ms));
        let cutoff =
            now_millis().saturating_sub(retention_days.clamp(1, 365).saturating_mul(86_400_000));
        let max_events = max_events.clamp(100, 20_000);
        let mut removed = 0usize;
        for (index, event) in events.iter().enumerate() {
            if (event.timestamp_ms < cutoff || index >= max_events)
                && fs::remove_file(self.directory.join(format!("{}.json", event.id))).is_ok()
            {
                removed += 1;
            }
        }
        // Temporary files are never visible as events and may remain after a sudden power loss.
        if let Ok(entries) = fs::read_dir(&self.directory) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().starts_with('.')
                    && name.to_string_lossy().ends_with(".tmp")
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        // The marker throttles age scans during the module's long-lived daemon session.
        let _ = fs::write(
            self.directory.join(".last_prune_ms"),
            now_millis().to_string(),
        );
        Ok(json!({
            "removed": removed,
            "retention_days": retention_days.clamp(1, 365),
            "max_events": max_events
        }))
    }

    /// Deletes every record in the journal.
    ///
    /// A record that cannot be deleted is an error, never a silent `cleared: 0`: the caller uses the
    /// count to decide whether the delete actually happened, so swallowing failures made "nothing was
    /// removed" indistinguishable from "everything was removed twice". The resolved directory is
    /// returned as well, because when two processes disagree about the inbox the path is the answer.
    pub fn clear(&self) -> Result<Value> {
        fs::create_dir_all(&self.directory)
            .map_err(|e| Error::io(format!("create event store: {e}")))?;
        let mut removed = 0usize;
        let mut failures = 0usize;
        let mut first_error: Option<String> = None;
        for entry in fs::read_dir(&self.directory)
            .map_err(|e| Error::io(format!("read event store: {e}")))?
        {
            let entry = entry.map_err(|e| Error::io(format!("read event entry: {e}")))?;
            if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match fs::remove_file(entry.path()) {
                Ok(()) => removed += 1,
                Err(e) => {
                    failures += 1;
                    first_error.get_or_insert_with(|| format!("{e}"));
                }
            }
        }
        if failures > 0 {
            return Err(Error::io(format!(
                "could not delete {failures} event file(s) in {}: {}",
                self.directory.display(),
                first_error.unwrap_or_default()
            )));
        }
        Ok(json!({
            "cleared": removed,
            "directory": self.directory.display().to_string(),
        }))
    }

    fn count_records(&self) -> Result<usize> {
        if !self.directory.exists() {
            return Ok(0);
        }
        let mut count = 0usize;
        for entry in fs::read_dir(&self.directory)
            .map_err(|e| Error::io(format!("read event store: {e}")))?
        {
            let entry = entry.map_err(|e| Error::io(format!("read event entry: {e}")))?;
            if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                count += 1;
            }
        }
        Ok(count)
    }

    fn read_all(&self) -> Result<Vec<SystemEvent>> {
        if !self.directory.exists() {
            return Ok(Vec::new());
        }
        let entries = fs::read_dir(&self.directory)
            .map_err(|e| Error::io(format!("read event store: {e}")))?;
        let mut events = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(event) = serde_json::from_str::<SystemEvent>(&text) {
                    events.push(event);
                }
            }
        }
        Ok(events)
    }
}

pub fn clamp_text(value: &str, max_chars: usize) -> String {
    let clean: String = value.chars().filter(|c| *c != '\0').collect();
    if clean.chars().count() <= max_chars {
        return clean;
    }
    if max_chars == 0 {
        return String::new();
    }
    let mut out: String = clean.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

pub fn clamp_token(value: &str, max_len: usize) -> String {
    let mut out: String = value
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(max_len)
        .collect();
    if out.is_empty() {
        out.push_str("event");
    }
    out
}

pub fn clamp_metadata(value: Value) -> Value {
    if serde_json::to_vec(&value)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
        <= MAX_METADATA_BYTES
    {
        value
    } else {
        json!({ "truncated": true })
    }
}

fn validate(input: &NewSystemEvent) -> Result<()> {
    validate_token("module", &input.module, 64)?;
    validate_token("category", &input.category, 64)?;
    validate_token("event_type", &input.event_type, 96)?;
    if !["info", "notice", "warning", "error", "critical"].contains(&input.severity.as_str()) {
        return Err(Error::invalid_arg(
            "severity must be info, notice, warning, error or critical",
        ));
    }
    validate_text("title", &input.title, MAX_TITLE_CHARS, true)?;
    validate_text("message", &input.message, MAX_MESSAGE_CHARS, false)?;
    if serde_json::to_vec(&input.metadata)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
        > MAX_METADATA_BYTES
    {
        return Err(Error::invalid_arg("event metadata must be at most 16 KiB"));
    }
    if let Some(key) = &input.dedupe_key {
        validate_token("dedupe_key", key, 128)?;
    }
    Ok(())
}

fn validate_token(name: &str, value: &str, max: usize) -> Result<()> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > max
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return Err(Error::invalid_arg(format!(
            "{name} must be 1..={max} ASCII letters, digits, dot, underscore or hyphen"
        )));
    }
    Ok(())
}

fn validate_text(name: &str, value: &str, max: usize, required: bool) -> Result<()> {
    let len = value.chars().count();
    if (required && len == 0) || len > max || value.contains('\0') {
        return Err(Error::invalid_arg(format!(
            "{name} must be {}..={max} characters",
            if required { 1 } else { 0 }
        )));
    }
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sysflow-event-test-{}-{nonce}", std::process::id()))
    }

    fn event(module: &str, key: Option<&str>) -> NewSystemEvent {
        NewSystemEvent {
            module: module.to_string(),
            category: "state".into(),
            event_type: "changed".into(),
            severity: "info".into(),
            title: "State changed".into(),
            message: "Changed by test".into(),
            metadata: json!({ "test": true }),
            dedupe_key: key.map(str::to_string),
            dedupe_window_s: 30,
        }
    }

    #[test]
    fn arbitrary_future_module_can_emit_and_list_events() {
        let dir = temp_dir();
        let store = EventStore::new(&dir);
        let (created, duplicate) = store.emit(event("future_sensor_array", None)).unwrap();
        assert!(!duplicate);
        let listed = store
            .list(EventFilter {
                limit: 10,
                module: Some("future_sensor_array"),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_reports_the_count_and_the_directory_it_emptied() {
        let dir = temp_dir();
        let store = EventStore::new(&dir);
        store.emit(event("cpu", None)).unwrap();
        store.emit(event("battery", None)).unwrap();
        let report = store.clear().unwrap();
        assert_eq!(report["cleared"], json!(2));
        // The directory is part of the contract: two processes disagreeing about an inbox is a
        // path problem, and this is the only place the writer states its own answer.
        assert_eq!(
            report["directory"],
            json!(dir.join("events").display().to_string())
        );
        assert_eq!(store.count_records().unwrap(), 0);
        // Idempotent: a second clear is a successful no-op, not an error.
        assert_eq!(store.clear().unwrap()["cleared"], json!(0));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn dedupe_key_suppresses_repeated_transition_events() {
        let dir = temp_dir();
        let store = EventStore::new(&dir);
        let (first, first_duplicate) = store.emit(event("display", Some("screen_off"))).unwrap();
        let (second, second_duplicate) = store.emit(event("display", Some("screen_off"))).unwrap();
        assert!(!first_duplicate);
        assert!(second_duplicate);
        assert_eq!(first.id, second.id);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn expired_dedupe_window_allows_new_event() {
        let dir = temp_dir();
        let store = EventStore::new(&dir);
        let old_ts = 1_000_000_000_000u64;
        let old_id = format!("{old_ts}-0000000001-00000000000000000001");
        fs::create_dir_all(dir.join("events")).unwrap();
        let old_event = SystemEvent {
            id: old_id.clone(),
            module: "cpu".into(),
            category: "enforcement".into(),
            event_type: "drift_repaired".into(),
            severity: "notice".into(),
            title: "Old".into(),
            message: String::new(),
            timestamp_ms: old_ts,
            metadata: json!({}),
            dedupe_key: Some("cpu.drift".into()),
        };
        fs::write(
            dir.join("events").join(format!("{old_id}.json")),
            serde_json::to_vec(&old_event).unwrap(),
        )
        .unwrap();

        let (created, duplicate) = store
            .emit(
                NewSystemEvent::new("cpu", "enforcement", "drift_repaired", "notice", "New")
                    .dedupe("cpu.drift", 600),
            )
            .unwrap();
        assert!(!duplicate);
        assert_ne!(created.id, old_id);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn emit_sanitized_clamps_oversized_fields() {
        let dir = temp_dir();
        let store = EventStore::new(&dir);
        let long_title = "测".repeat(200);
        let long_msg = "x\0y".repeat(600);
        let (created, _) = store
            .emit_sanitized(
                NewSystemEvent::new("cpu", "bad category!", "ev", "unknown", long_title)
                    .message(long_msg)
                    .metadata(json!({ "blob": "a".repeat(20 * 1024) })),
            )
            .unwrap();
        assert_eq!(created.category, "bad_category_");
        assert_eq!(created.severity, "info");
        assert_eq!(created.title.chars().count(), MAX_TITLE_CHARS);
        assert_eq!(created.message.chars().count(), MAX_MESSAGE_CHARS);
        assert!(!created.message.contains('\0'));
        assert_eq!(created.metadata, json!({ "truncated": true }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cursor_reads_are_stable_for_same_millisecond_records() {
        let dir = temp_dir();
        let store = EventStore::new(&dir);
        let (first, _) = store.emit(event("future_module", None)).unwrap();
        let (second, _) = store.emit(event("future_module", None)).unwrap();
        let page = store
            .list(EventFilter {
                limit: 10,
                after_ms: Some(first.timestamp_ms),
                after_id: Some(&first.id),
                ascending: true,
                ..Default::default()
            })
            .unwrap();
        assert!(page.iter().any(|record| record.id == second.id));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_unknown_severity_and_bad_module_identifiers() {
        let dir = temp_dir();
        let store = EventStore::new(&dir);
        let mut invalid = event("cpu", None);
        invalid.severity = "fatal-ish".into();
        assert!(store.emit(invalid).is_err());
        assert!(store.emit(event("bad module", None)).is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
