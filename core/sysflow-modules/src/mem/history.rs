//! History sampling for the memory module: `state/mem.json` baseline plus interval deltas.
//!
//! The series are specified in `docs/protocol.md` §3.22. Two rules shape this file:
//!
//! * **Cumulative counters are only useful as differences.** The baseline is persisted, so a daemon
//!   restart can still compute the next interval, but the first point after a reboot is *omitted* —
//!   a gap in the curve — rather than written as `0`, which would read as "the device was quiet".
//! * **Rates divide by the measured interval**, not by the nominal 300 s. A phone that slept for six
//!   hours would otherwise report a burst that never happened, or a burst spread over the wrong
//!   window.
//!
//! Instantaneous values (available, swap used, zRAM compressed) are reported on every call, so a
//! fresh install still gets a usable curve for those three while the delta series start one point
//! later.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use sysflow_core::command;
use sysflow_core::event::{EventStore, NewSystemEvent};
use sysflow_core::module::Sample;
use sysflow_core::sysfs::Sysfs;

use super::probe;

/// File inside the module state directory.
pub const FILE_NAME: &str = "mem.json";
/// One sample every five minutes: 8 640 lines over the 30 day window, inside the 20 000 line cap.
pub const SAMPLE_INTERVAL_S: u64 = 300;
/// Bumped when the shape of the counters changes; an older file is treated as "no baseline".
const STATE_VERSION: u32 = 1;

/// Placeholder trigger: `full` PSI at 3.00 % of wall time, for three samples (15 minutes).
///
/// The S25 long-run mean is ≈ 0.52 % (`docs/modules/mem.md`), so this is ≈ 6× the baseline. It stays
/// a constant until a quiet-window measurement replaces it — one line to change.
pub const HIGH_PCT: u64 = 300;
/// Recovery has a lower bar than the trigger, otherwise a curve hovering at the threshold would emit
/// an event every few minutes.
pub const RECOVER_PCT: u64 = 100;
pub const HIGH_SAMPLES: u32 = 3;
pub const RECOVER_SAMPLES: u32 = 3;
/// A sample taken after the device slept longer than this still records its numbers but raises no
/// event: an "average" over a six hour gap says nothing about the last five minutes.
pub const MAX_EVENT_INTERVAL_S: u64 = 3 * SAMPLE_INTERVAL_S;
/// Dedupe window per event family, as a second line of defence behind the state machine.
pub const EVENT_WINDOW_S: u64 = 6 * 3600;
/// `dumpsys activity exit-info` prints ~2.5 MB on S25, so it is only read when a `high` event fires.
const EXIT_INFO_TIMEOUT: Duration = Duration::from_secs(8);

/// The counters that only mean something as a difference.
///
/// Every field is `Option`, because a kernel may lack PSI, the zone-split `allocstall` or the old
/// `workingset_refault`: a counter that cannot be read removes its series instead of reporting `0`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Counters {
    #[serde(default)]
    pub psi_some_us: Option<u64>,
    #[serde(default)]
    pub psi_full_us: Option<u64>,
    #[serde(default)]
    pub pgscan_direct: Option<u64>,
    #[serde(default)]
    pub allocstall: Option<u64>,
    #[serde(default)]
    pub pswpin: Option<u64>,
    #[serde(default)]
    pub pswpout: Option<u64>,
    #[serde(default)]
    pub refault: Option<u64>,
    #[serde(default)]
    pub oom_kill: Option<u64>,
}

impl Counters {
    /// Whether any counter moved backwards, which means the machine rebooted or the kernel reset it.
    ///
    /// A single regressed counter is enough: the differences for that interval would be garbage, and
    /// a negative "reclaim count" in the curve is worse than a gap.
    pub fn regressed(&self, previous: &Counters) -> bool {
        let lower = |now: Option<u64>, before: Option<u64>| matches!((now, before), (Some(now), Some(before)) if now < before);
        lower(self.psi_some_us, previous.psi_some_us)
            || lower(self.psi_full_us, previous.psi_full_us)
            || lower(self.pgscan_direct, previous.pgscan_direct)
            || lower(self.allocstall, previous.allocstall)
            || lower(self.pswpin, previous.pswpin)
            || lower(self.pswpout, previous.pswpout)
            || lower(self.refault, previous.refault)
            || lower(self.oom_kill, previous.oom_kill)
    }
}

/// Hysteresis state for the pressure events, persisted with the baseline so a daemon restart in the
/// middle of a pressure episode does not re-announce it (and so recovery is still reported later).
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PressureState {
    /// A `high` event has been emitted and `recovered` has not.
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub high_streak: u32,
    #[serde(default)]
    pub low_streak: u32,
}

/// The persisted baseline: which boot it belongs to, how long the machine had been up, when it was
/// taken, and the counter values at that moment.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub boot_id: String,
    #[serde(default)]
    pub uptime_s: f64,
    #[serde(default)]
    pub ts: u64,
    #[serde(default)]
    pub counters: Counters,
    #[serde(default)]
    pub pressure: PressureState,
}

/// Baseline file. Rewritten once per sample and written atomically: a killed daemon must never
/// leave half a baseline behind, because the next run would then compute nonsense differences.
pub struct StateFile {
    path: PathBuf,
}

impl StateFile {
    pub fn new(state_dir: &Path) -> Self {
        StateFile {
            path: state_dir.join(FILE_NAME),
        }
    }

    /// Path of the state file, for tests that seed a previous baseline.
    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A missing or corrupt file reads as "no baseline yet": losing one interval is recoverable,
    /// refusing to sample is not.
    pub fn load(&self) -> Baseline {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Baseline::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn store(&self, baseline: &Baseline) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(body) = serde_json::to_string(baseline) else {
            return;
        };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

/// Read every cumulative counter this device offers.
pub fn current_counters(fs: &Sysfs) -> Counters {
    let psi = probe::psi_totals(fs);
    let table = probe::vmstat(fs);
    let get = |name: &str| table.as_ref().and_then(|table| table.get(name).copied());
    Counters {
        psi_some_us: psi.map(|(some, _)| some),
        psi_full_us: psi.and_then(|(_, full)| full),
        pgscan_direct: get("pgscan_direct"),
        allocstall: table.as_ref().and_then(probe::allocstall_total),
        pswpin: get("pswpin"),
        pswpout: get("pswpout"),
        refault: table.as_ref().and_then(probe::refault_total),
        oom_kill: get("oom_kill"),
    }
}

/// Build one sample, persist the new baseline, and return what should be appended to history.
///
/// `None` means "nothing worth recording" (the module is unsupported), which is different from an
/// empty map: the daemon skips `None` silently.
pub fn sample(
    fs: &Sysfs,
    state_dir: &Path,
    now_ts: u64,
    events: Option<&EventStore>,
) -> Option<Sample> {
    let detected = probe::detect(fs);
    if !detected.supported() {
        return None;
    }

    let file = StateFile::new(state_dir);
    let previous = file.load();
    let boot_id = probe::boot_id(fs).unwrap_or_default();
    let uptime = probe::uptime_seconds(fs).unwrap_or_default();
    let counters = current_counters(fs);

    let mut out = Map::new();

    // --- instantaneous: meaningful on the very first sample too -------------------------------
    if let Some(meminfo) = detected.meminfo.ok() {
        if let Some(available) = meminfo.get("MemAvailable") {
            out.insert("available_kb".into(), json!(available));
        }
        if let (Some(total), Some(free)) = (meminfo.get("SwapTotal"), meminfo.get("SwapFree")) {
            out.insert("swap_used_kb".into(), json!(total.saturating_sub(*free)));
        }
    }
    if let Some((_, compr, _)) = detected.zram_totals() {
        out.insert("zram_compr_kb".into(), json!((compr / 1024) as i64));
    }

    // --- interval differences: only with a baseline that belongs to this boot -----------------
    let same_boot = !boot_id.is_empty() && boot_id == previous.boot_id;
    let usable = previous.version == STATE_VERSION
        && same_boot
        && previous.ts > 0
        && now_ts > previous.ts
        && uptime >= previous.uptime_s
        && !counters.regressed(&previous.counters);

    // Measured on the boot clock when both samples have one: the wall clock jumps whenever the phone
    // resyncs time, and a rate divided by a jumped interval is wrong in a way nothing downstream can
    // detect. The wall clock is only the fallback for a device without `/proc/uptime`.
    let elapsed_s = if previous.uptime_s > 0.0 && uptime > previous.uptime_s {
        uptime - previous.uptime_s
    } else {
        now_ts.saturating_sub(previous.ts) as f64
    };
    if usable {
        let delta = |now: Option<u64>, before: Option<u64>| match (now, before) {
            (Some(now), Some(before)) if now >= before => Some(now - before),
            _ => None,
        };

        // PSI: stalled microseconds out of elapsed microseconds, in hundredths of a percent.
        for (name, now, before) in [
            (
                "psi_some_pct",
                counters.psi_some_us,
                previous.counters.psi_some_us,
            ),
            (
                "psi_full_pct",
                counters.psi_full_us,
                previous.counters.psi_full_us,
            ),
        ] {
            if let Some(stalled_us) = delta(now, before) {
                let hundredths = (stalled_us as f64 * 10_000.0 / (elapsed_s * 1_000_000.0)).round();
                // Clamped at 100 %: a clock that jumps backwards mid-interval cannot produce more
                // stall time than elapsed time.
                let hundredths = hundredths.clamp(0.0, 10_000.0) as u64;
                out.insert(name.to_string(), json!(hundredths));
            }
        }
        if let Some(pages) = delta(counters.pgscan_direct, previous.counters.pgscan_direct) {
            out.insert("pgscan_direct".into(), json!(pages));
        }
        if let Some(count) = delta(counters.allocstall, previous.counters.allocstall) {
            out.insert("allocstall".into(), json!(count));
        }
        if let Some(pages) = delta(counters.refault, previous.counters.refault) {
            out.insert("refault".into(), json!(pages));
        }
        if let Some(count) = delta(counters.oom_kill, previous.counters.oom_kill) {
            out.insert("oom_kill".into(), json!(count));
        }
        // Swap traffic is counted in pages; without a trustworthy page size the series is omitted
        // rather than scaled by a guess.
        if let Some(page) = probe::page_size(fs) {
            for (name, now, before) in [
                ("swap_in_kb", counters.pswpin, previous.counters.pswpin),
                ("swap_out_kb", counters.pswpout, previous.counters.pswpout),
            ] {
                if let Some(pages) = delta(now, before) {
                    out.insert(name.to_string(), json!(pages * page / 1024));
                }
            }
        }
    }

    // --- events: only from a trustworthy interval ----------------------------------------------
    //
    // A reboot resets the episode: what was true before it cannot be compared with now, and
    // announcing `recovered` for pressure that ended in a reboot would be a lie.
    let mut pressure = if usable {
        previous.pressure
    } else {
        PressureState::default()
    };
    let event_window_ok = usable && elapsed_s <= MAX_EVENT_INTERVAL_S as f64;
    let psi_full = out.get("psi_full_pct").and_then(Value::as_u64);
    let psi_full_avg60 = detected
        .psi
        .ok()
        .and_then(|psi| psi.full.as_ref())
        .map(|line| probe::pct_to_hundredths(line.avg60));

    if event_window_ok {
        if let Some(pct) = psi_full {
            if pct >= HIGH_PCT {
                pressure.high_streak += 1;
                pressure.low_streak = 0;
            } else if pct < RECOVER_PCT {
                pressure.low_streak += 1;
                pressure.high_streak = 0;
            } else {
                // The dead band resets both streaks: "three consecutive samples" means three in a
                // row, not three eventually.
                pressure.high_streak = 0;
                pressure.low_streak = 0;
            }

            if !pressure.active && pressure.high_streak >= HIGH_SAMPLES {
                pressure.active = true;
                pressure.high_streak = 0;
                emit(
                    events,
                    NewSystemEvent::new(
                        "mem",
                        "pressure",
                        "mem_pressure_high",
                        "warning",
                        "Memory pressure is high",
                    )
                    .message(format!(
                        "full PSI at or above {:.2} % for {} consecutive samples",
                        HIGH_PCT as f64 / 100.0,
                        HIGH_SAMPLES
                    ))
                    // The one place the module shells out while sampling, and only on an event that
                    // will actually be recorded.
                    .metadata({
                        let metadata = context(&out, psi_full, psi_full_avg60);
                        if events.is_some() {
                            with_exit_info(metadata)
                        } else {
                            metadata
                        }
                    })
                    .dedupe("mem.pressure.high", EVENT_WINDOW_S),
                );
            } else if pressure.active && pressure.low_streak >= RECOVER_SAMPLES {
                pressure.active = false;
                pressure.low_streak = 0;
                emit(
                    events,
                    NewSystemEvent::new(
                        "mem",
                        "pressure",
                        "mem_pressure_recovered",
                        "info",
                        "Memory pressure recovered",
                    )
                    .message(format!(
                        "full PSI below {:.2} % for {} consecutive samples",
                        RECOVER_PCT as f64 / 100.0,
                        RECOVER_SAMPLES
                    ))
                    .metadata(context(&out, psi_full, psi_full_avg60))
                    .dedupe("mem.pressure.recovered", EVENT_WINDOW_S),
                );
            }
        }

        if let Some(count) = out.get("oom_kill").and_then(Value::as_u64) {
            if count > 0 {
                emit(
                    events,
                    NewSystemEvent::new(
                        "mem",
                        "pressure",
                        "mem_oom_kill",
                        "notice",
                        "The kernel killed processes for memory",
                    )
                    .message(format!(
                        "{count} kernel OOM kill(s) since the previous sample"
                    ))
                    .metadata({
                        // The page words this as "N processes", so the count travels with the event.
                        let mut metadata = context(&out, psi_full, psi_full_avg60);
                        metadata["count"] = json!(count);
                        metadata
                    })
                    .dedupe("mem.oom_kill", EVENT_WINDOW_S),
                );
            }
        }
    }

    // Persist before returning: the next interval is measured from *this* sample even if the caller
    // fails to write the history line.
    file.store(&Baseline {
        version: STATE_VERSION,
        boot_id,
        uptime_s: uptime,
        ts: now_ts,
        counters,
        pressure,
    });

    (!out.is_empty()).then_some(out)
}

/// The numbers that make an event explainable afterwards: was memory really short, or was the
/// threshold just sensitive?
fn context(sample: &Map<String, Value>, psi_full: Option<u64>, avg60: Option<i64>) -> Value {
    json!({
        "available_kb": sample.get("available_kb").cloned().unwrap_or(Value::Null),
        "swap_used_kb": sample.get("swap_used_kb").cloned().unwrap_or(Value::Null),
        "psi_full_pct": psi_full,
        "psi_full_avg60": avg60,
    })
}

/// Best-effort attribution for a `high` event: the newest `LOW_MEMORY` record from the exit journal.
fn with_exit_info(mut metadata: Value) -> Value {
    if let Some(record) = last_low_memory_exit() {
        metadata["last_low_memory_exit"] = record;
    }
    metadata
}

fn last_low_memory_exit() -> Option<Value> {
    let text = command::run("dumpsys", &["activity", "exit-info"], EXIT_INFO_TIMEOUT).ok()?;
    parse_newest_low_memory(&text)
}

/// Pull the **most recent** `reason=3 (LOW_MEMORY)` record out of an `exit-info` dump.
///
/// The layout on S25 (`docs/modules/mem.md`):
///
/// ```text
/// timestamp=2026-11-23 23:26:09.526 pid=4434 … user=0
/// process=com.example reason=3 (LOW_MEMORY) subreason=0 (UNKNOWN) status=0
/// importance=400 pss=0.00 rss=103MB state=empty trace=null
/// ```
///
/// The dump is grouped per package, so the *last* match in the text is just the last package listed,
/// not the latest kill: "who was killed most recently" has to be decided by the timestamps. They are
/// fixed-width (`YYYY-MM-DD HH:MM:SS.mmm`), so comparing them as text compares them as times. A
/// record without a readable timestamp loses to any record that has one; among equals the later
/// line wins, which keeps the answer deterministic. `pss` reads `0.00` once the process is gone, so
/// `rss` is the only usable size.
pub fn parse_newest_low_memory(text: &str) -> Option<Value> {
    let lines: Vec<&str> = text.lines().collect();
    let mut newest: Option<(String, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        if !line.contains("reason=3 (LOW_MEMORY)") {
            continue;
        }
        let stamp = record_timestamp(&lines, index).unwrap_or_default();
        if newest.as_ref().is_none_or(|(seen, _)| stamp >= *seen) {
            newest = Some((stamp, index));
        }
    }
    let (stamp, index) = newest?;

    let mut record = Map::new();
    record.insert("package".into(), json!(field(lines[index], "process=")?));
    if !stamp.is_empty() {
        record.insert("timestamp".into(), json!(stamp));
    }
    if let Some(header) = timestamp_line(&lines, index) {
        if let Some(uid) = field(header, "packageUid=") {
            record.insert("package_uid".into(), json!(uid));
        }
    }

    // The `importance=… pss=… rss=…` line follows immediately.
    for ahead in 1..=2 {
        let Some(line) = lines.get(index + ahead) else {
            break;
        };
        if let Some(rss) = field(line, "rss=") {
            record.insert("rss".into(), json!(rss));
            break;
        }
    }
    Some(Value::Object(record))
}

/// The header line of the record whose `process=` line is at `index`: one to three lines above it.
fn timestamp_line<'a>(lines: &[&'a str], index: usize) -> Option<&'a str> {
    (1..=3)
        .filter_map(|back| index.checked_sub(back).and_then(|at| lines.get(at)))
        .find(|line| line.contains("timestamp="))
        .copied()
}

/// `2026-11-23 23:26:09.526` from that header line.
fn record_timestamp(lines: &[&str], index: usize) -> Option<String> {
    let header = timestamp_line(lines, index)?;
    let mut parts = header.split("timestamp=").nth(1)?.split_whitespace();
    let date = parts.next()?;
    let time = parts.next().unwrap_or("");
    Some(format!("{date} {time}").trim().to_string())
}

/// `key=value` from a whitespace-separated dump line.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(&rest[..end]).filter(|value| !value.is_empty())
}

fn emit(events: Option<&EventStore>, event: NewSystemEvent) {
    let Some(store) = events else {
        return;
    };
    // Event logging is best-effort by contract: it must never interrupt sampling or drift repair.
    let _ = store.emit(event);
}
