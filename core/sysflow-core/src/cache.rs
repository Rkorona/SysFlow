//! Small on-disk JSON caches for expensive queries.
//!
//! Several commands (`battery apps`, `battery health`, `wakelock held`) shell out to `dumpsys`,
//! which costs hundreds of milliseconds. The APK may call them when a page opens, so results are
//! cached for a short TTL under `<data_dir>/state/`. The cache is advisory: a missing, unreadable or
//! stale file simply means "run the query again".

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Default lifetime for a `dumpsys`-derived result.
pub const TTL_SECS: u64 = 60;

/// Read a cached value written by [`write`], if it is younger than `ttl_s`.
///
/// The caller decides how to mark the response (the convention is to set `cached: true`).
pub fn read_fresh(path: &Path, ttl_s: u64) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let generated = value.get("generated_at")?.as_u64()?;
    if now_secs().saturating_sub(generated) > ttl_s {
        return None;
    }
    Some(value)
}

/// Write a cache file, creating the parent directory. Best-effort: a cache failure must never fail
/// the command that produced the value.
pub fn write(path: &Path, value: &Value) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(value) {
        let _ = std::fs::write(path, text);
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sysflow-cache-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("state").join("cache.json")
    }

    #[test]
    fn fresh_entries_round_trip() {
        let path = temp_path("fresh");
        write(&path, &json!({ "generated_at": now_secs(), "value": 7 }));
        let cached = read_fresh(&path, TTL_SECS).expect("fresh entry");
        assert_eq!(cached["value"], json!(7));
    }

    #[test]
    fn stale_entries_are_ignored() {
        let path = temp_path("stale");
        write(
            &path,
            &json!({ "generated_at": now_secs().saturating_sub(TTL_SECS * 2), "value": 7 }),
        );
        assert!(read_fresh(&path, TTL_SECS).is_none());
    }

    #[test]
    fn missing_or_malformed_files_read_as_none() {
        let path = temp_path("missing");
        assert!(read_fresh(&path, TTL_SECS).is_none());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json").unwrap();
        assert!(read_fresh(&path, TTL_SECS).is_none());
    }
}
