//! Rendering of the read half: kernel sources, the 24 h ranking and the summary.
//!
//! These are pure functions over the parsed table and the stored window, so they can be tested
//! without a filesystem and without the module's write half.

use serde_json::{json, Value};

use sysflow_core::wakeup_sources::{classify_wakeup_source, KernelSource};

use super::window::WindowState;

/// How many sources `kernel_sources` returns (sorted by accumulated time).
pub const MAX_SOURCES: usize = 24;
/// How many entries the 24 hour ranking returns.
pub const RANK_LIMIT: usize = 15;

/// `1h30m` / `45s`, for event messages (clients render their own strings from the metadata).
pub fn format_duration(ms: i64) -> String {
    let seconds = ms.max(0) / 1_000;
    match seconds {
        0 => "under a second".to_string(),
        s if s < 60 => format!("{s}s"),
        s if s < 3_600 => format!("{}m{}s", s / 60, s % 60),
        s => format!("{}h{}m", s / 3_600, (s % 3_600) / 60),
    }
}

pub fn sources_json(sources: &[KernelSource]) -> Value {
    let mut sorted: Vec<&KernelSource> = sources.iter().collect();
    // Active sources are what a user is looking for; then whoever accumulated the most time.
    sorted.sort_by(|a, b| {
        b.is_active()
            .cmp(&a.is_active())
            .then_with(|| b.total_ms.cmp(&a.total_ms))
            .then_with(|| a.name.cmp(&b.name))
    });
    let items: Vec<Value> = sorted
        .into_iter()
        .take(MAX_SOURCES)
        .map(|source| {
            json!({
                "name": source.name,
                // Coarse driver family; clients render the label (see protocol §3.18).
                "category": classify_wakeup_source(&source.name),
                "active": source.is_active(),
                "active_count": source.active_count,
                "event_count": source.event_count,
                "wakeup_count": source.wakeup_count,
                "expire_count": source.expire_count,
                "active_since_ms": source.active_since_ms,
                "total_ms": source.total_ms,
                "max_ms": source.max_ms,
                "prevent_suspend_ms": source.prevent_suspend_ms,
            })
        })
        .collect();
    Value::Array(items)
}

pub fn ranked_json(state: &WindowState) -> Value {
    let deltas = state.deltas();
    // The denominator covers *every* source, not just the rows about to be displayed: summing a
    // truncated top-15 made each share too large on a real device with dozens of movers.
    let measured: i64 = deltas.iter().map(|delta| delta.ms).sum();
    let items: Vec<Value> = deltas
        .iter()
        .take(RANK_LIMIT)
        .map(|delta| {
            json!({
                "name": delta.name,
                "category": classify_wakeup_source(&delta.name),
                "awake_ms": delta.ms,
                // Increments over the same window: "who wakes us most often" and "who keeps us
                // awake longest" are different questions and answer different complaints.
                "wakeups": delta.wakeups,
                // Legacy `/proc/wakelocks` has no event counter; `0` means "not reported".
                "events": delta.events,
                "share": if measured > 0 { delta.ms as f64 / measured as f64 } else { 0.0 },
            })
        })
        .collect();
    Value::Array(items)
}

pub fn summary_json(
    available: bool,
    reason: Option<&str>,
    sources: &[KernelSource],
    state: &WindowState,
    uptime: Option<u64>,
) -> Value {
    let now = sysflow_core::history::now_secs();
    let deltas = state.deltas();
    let total_wakeups: i64 = deltas.iter().map(|delta| delta.wakeups).sum();
    let measured_ms: i64 = deltas.iter().map(|delta| delta.ms).sum();
    json!({
        "available": available,
        "known": sources.len(),
        "active": sources.iter().filter(|source| source.is_active()).count(),
        // Never larger than the current boot: after a reboot the window restarts, and saying
        // "24 h" while it is three minutes old is what made the empty state misleading.
        "window_s": state.window_s(now, uptime),
        // `fresh` / `reboot` / `rollover` / `baseline` (state written before the reason was
        // recorded). Clients word their empty state from this.
        "window_reason": state.reason(),
        "updated_s": state.updated_ts,
        "since_boot_s": uptime,
        // Total measured wake time, i.e. the denominator behind every `share`.
        "measured_ms": measured_ms,
        // Same window, counted instead of timed.
        "total_wakeups": total_wakeups,
        // True once a sample has established a baseline; the list itself can still be empty
        // when nothing moved since then, which the client renders as "no wakeups observed".
        "ranking_available": state.baseline_ts > 0 && !state.last.is_empty(),
        // Present only when the kernel table could not be read, so the client can say why
        // instead of a generic "unavailable".
        "reason": reason,
    })
}
