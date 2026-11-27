//! Parser for the BatteryStats "Estimated power use" section.
//!
//! `dumpsys batterystats` is the only source of per-app power attribution on Android, and the same
//! data the system Settings battery screen shows. The section looks like this:
//!
//! ```text
//!   Estimated power use (mAh):
//!     Capacity: 5000, Rated: 4855, Typical: 5000, Computed drain: 0.782, actual drain: 0.782
//!     Global
//!     screen: 1040 apps: 1040
//!       (on battery, screen on)
//!   UID u0a404: 533 fg: 31.5 bg: 114 cached: 277 (5m 14s 110ms)
//!       screen=110 cpu=403 cpu:fg=14.9 ...
//! ```
//!
//! Two facts drive the design:
//!
//! * Attribution is per **UID**, not per process. Every process of an app is folded into one row,
//!   and system_server-style daemons appear under Android's shared system UIDs.
//! * The numbers are **estimates** produced from CPU/radio/wakelock time and the device's power
//!   profile. On devices with a missing or vendor-locked profile the per-component currents can be
//!   wildly off, so the total is not a reliable mAh reading even though the section is labelled
//!   that way. The *shares* between UIDs stay meaningful because every component's per-UID values
//!   sum exactly to the printed global values.
//!
//! Everything here is pure string handling, so it is unit-tested against real dumps.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};

/// One row of the ranking.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerEntry {
    /// Raw UID as printed (`u0a404`, `1000`, ...).
    pub uid: String,
    /// Numeric UID when the row is a plain number.
    pub numeric_uid: Option<u32>,
    /// BatteryStats' own estimate for this UID, in the dump's internal unit.
    pub raw_power: f64,
    /// UIDs in function of role, e.g. `Android 系统` for 1000. Filled by the CLI.
    pub aid_label: Option<String>,
    /// Package names sharing this UID. Filled by the CLI.
    pub packages: Vec<String>,
    /// Share of the reported total, 0..1.
    pub share: f64,
}

#[derive(Clone, Debug, Default)]
pub struct PowerReport {
    pub capacity_mah: Option<f64>,
    pub computed_drain: Option<f64>,
    pub actual_drain: Option<f64>,
    /// Sum of every UID row, in the dump's internal unit.
    pub total_raw: f64,
    pub entries: Vec<PowerEntry>,
}

impl PowerReport {
    /// The dump labels the section "(mAh)" but prints thousandths on every device seen so far:
    /// a 5000 mAh battery cannot have consumed 42 000 mAh. Dividing by 1000 matches the reported
    /// drain percentage, so that is what callers get.
    pub fn power_mah(entry: &PowerEntry) -> f64 {
        entry.raw_power / 1000.0
    }

    pub fn to_json(&self, limit: usize) -> Value {
        let mut sorted = self.entries.clone();
        sorted.sort_by(|a, b| {
            b.raw_power
                .partial_cmp(&a.raw_power)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let shown = if limit == 0 {
            sorted.len()
        } else {
            sorted.len().min(limit)
        };
        let entries: Vec<Value> = sorted
            .iter()
            .take(shown)
            .map(|entry| {
                json!({
                    "uid": entry.uid,
                    "numeric_uid": entry.numeric_uid,
                    "name": entry.aid_label,
                    "packages": entry.packages,
                    "power_mah": round_power(Self::power_mah(entry)),
                    "share": round4(entry.share),
                })
            })
            .collect();
        let total_mah = self.total_raw / 1000.0;
        // The dump's own "Computed drain" is not comparable across devices: one reported `0.782`
        // and another `980` for the same concept. Compute the percentage from the capacity we
        // parsed instead, and expose the raw device values under clearly labelled keys.
        let drain_percent = self
            .capacity_mah
            .filter(|capacity| *capacity > 0.0)
            .map(|capacity| (total_mah / capacity * 100.0 * 100.0).round() / 100.0);
        json!({
            "capacity_mah": self.capacity_mah.map(round1),
            "drain_percent": drain_percent,
            "reported_computed_drain": self.computed_drain,
            "reported_actual_drain": self.actual_drain,
            "total_mah": round1(total_mah),
            "count": self.entries.len(),
            "entries": entries,
        })
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Sub-mAh rows are normal (hundreds of UIDs), so keep three decimals below 1 mAh.
fn round_power(value: f64) -> f64 {
    if value.abs() < 1.0 {
        (value * 1000.0).round() / 1000.0
    } else {
        round1(value)
    }
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// Parse the section from a full dump. Returns `None` when the device does not expose it (some
/// ROMs, and every device before the first battery event).
pub fn parse_power_section<I: IntoIterator<Item = String>>(lines: I) -> Option<PowerReport> {
    let mut report = PowerReport::default();
    let mut inside = false;
    let mut saw_section = false;

    for line in lines {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();

        if !inside {
            if trimmed.starts_with("Estimated power use") {
                inside = true;
                saw_section = true;
            }
            continue;
        }

        if trimmed.is_empty() {
            // A blank line inside the section is padding; a blank line after the UID list ends it.
            if !report.entries.is_empty() {
                break;
            }
            continue;
        }
        if indent == 0 {
            break;
        }
        if trimmed.starts_with("Capacity:") {
            parse_capacity(trimmed, &mut report);
            continue;
        }
        if trimmed.starts_with("Global") && indent <= 4 {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("UID ") {
            if let Some(entry) = parse_uid_row(rest) {
                report.entries.push(entry);
            }
            continue;
        }
        // Anything shallower than a component line and not a UID row starts the next section.
        if indent <= 2 {
            break;
        }
    }

    if !saw_section {
        return None;
    }
    report.total_raw = report.entries.iter().map(|entry| entry.raw_power).sum();
    if report.total_raw > 0.0 {
        for entry in &mut report.entries {
            entry.share = entry.raw_power / report.total_raw;
        }
    }
    Some(report)
}

fn parse_capacity(line: &str, report: &mut PowerReport) {
    for part in line.split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("Capacity:") {
            report.capacity_mah = value.trim().parse().ok();
        } else if let Some(value) = part.strip_prefix("Computed drain:") {
            report.computed_drain = value.trim().parse().ok();
        } else if let Some(value) = part.strip_prefix("actual drain:") {
            report.actual_drain = value.trim().parse().ok();
        }
    }
}

/// `u0a404: 533 fg: 31.5 ...` -> uid `u0a404`, power `533`.
fn parse_uid_row(rest: &str) -> Option<PowerEntry> {
    let (uid, tail) = rest.split_once(':')?;
    let uid = uid.trim();
    if uid.is_empty() {
        return None;
    }
    let power = tail.split_whitespace().next()?.parse::<f64>().ok()?;
    Some(PowerEntry {
        uid: uid.to_string(),
        numeric_uid: uid.parse().ok(),
        raw_power: power,
        aid_label: None,
        packages: Vec::new(),
        share: 0.0,
    })
}

/// Parse `cmd package list packages -U` output into uid -> package names.
pub fn parse_package_uids<I: IntoIterator<Item = String>>(lines: I) -> BTreeMap<u32, Vec<String>> {
    let mut map: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for line in lines {
        let mut package = None;
        let mut uids: Vec<u32> = Vec::new();
        for token in line.split_whitespace() {
            if let Some(name) = token.strip_prefix("package:") {
                package = Some(name.to_string());
            } else if let Some(value) = token.strip_prefix("uid:") {
                for part in value.split(',') {
                    if let Ok(uid) = part.trim().parse::<u32>() {
                        uids.push(uid);
                    }
                }
            }
        }
        if let Some(package) = package {
            for uid in uids {
                map.entry(uid).or_default().push(package.clone());
            }
        }
    }
    for packages in map.values_mut() {
        packages.sort();
        packages.dedup();
    }
    map
}

/// UID -> package names from `cmd package list packages -U`; empty when the call fails.
///
/// One binder-backed call serves both per-app views (`wakelock apps` and `wakelock held`), so it
/// lives here instead of being re-implemented per verb. A failure returns an empty map: labels are a
/// nicety, and the caller can still show the UID.
pub fn read_package_uids() -> BTreeMap<u32, Vec<String>> {
    let Ok(text) = crate::command::run(
        "cmd",
        &["package", "list", "packages", "-U"],
        Duration::from_secs(8),
    ) else {
        return BTreeMap::new();
    };
    parse_package_uids(text.lines().map(str::to_string))
}

/// Friendly names for Android's shared system UIDs, used when no package maps to them.
pub fn aid_label(uid: u32) -> Option<&'static str> {
    Some(match uid {
        0 => "内核 / root",
        1000 => "Android 系统",
        1001 => "电话服务",
        1002 => "蓝牙",
        1005 => "音频服务",
        1006 => "相机服务",
        1010 => "WLAN",
        1013 => "媒体服务",
        1027 => "NFC",
        1073 => "网络服务",
        _ => return None,
    })
}

/// One row of BatteryStats' "All partial wake locks" section.
///
/// The row format is:
///
/// ```text
///   Wake lock u0a346 *job*e//SessionWorker/@eM_ADDR/...: 28s 608ms (3 times) max=20422 actual=42251 realtime
/// ```
///
/// The tag itself may contain colons (`*alarm*:android.intent.action...`), so the tag/duration split
/// is the first `": "` followed by a digit — not the first colon in the line.
#[derive(Clone, Debug, PartialEq)]
pub struct WakeLockStat {
    /// Raw UID as printed (`u0a346`, `1000`, ...).
    pub uid: String,
    /// Numeric UID when it can be resolved (system UIDs are printed plainly).
    pub numeric_uid: Option<u32>,
    /// Wake-lock tag as printed, including `*job*:`/`*alarm*:` prefixes.
    pub tag: String,
    /// Total time this (uid, tag) pair held the lock.
    pub total_ms: i64,
    /// How many times it was acquired.
    pub count: u32,
}

/// Streaming reader for the "All partial wake locks" section.
///
/// The section sits hundreds of lines into a `dumpsys batterystats --charged` run (4 000+ lines on a
/// quiet device, more on a busy one), so the CLI feeds lines as they arrive and stops reading as soon
/// as [`WakeLockScanner::push`] reports the section is over.
#[derive(Debug, Default)]
pub struct WakeLockScanner {
    stats: Vec<WakeLockStat>,
    inside: bool,
    finished: bool,
}

impl WakeLockScanner {
    pub fn new() -> Self {
        WakeLockScanner::default()
    }

    /// Feed one line. Returns `true` once the section has ended and the caller may stop reading.
    pub fn push(&mut self, line: &str) -> bool {
        if self.finished {
            return true;
        }
        let trimmed = line.trim();
        if !self.inside {
            if trimmed.starts_with("All partial wake locks") {
                self.inside = true;
            }
            return false;
        }
        if trimmed.is_empty() {
            // Padding before the first row is harmless; a blank line *after* rows ends the section.
            if self.stats.is_empty() {
                return false;
            }
            self.finished = true;
            return true;
        }
        if let Some(stat) = parse_wake_lock_row(trimmed) {
            self.stats.push(stat);
            return false;
        }
        // Any non-row line after the header means the section is over (possibly with zero rows).
        self.finished = true;
        true
    }

    /// The rows, or `None` when the dump had no such section at all (older builds, or no locks yet).
    pub fn finish(self) -> Option<Vec<WakeLockStat>> {
        self.inside.then_some(self.stats)
    }
}

/// Convenience wrapper for a complete dump (tests, and any caller that already has the text).
pub fn parse_wake_lock_section<I: IntoIterator<Item = String>>(
    lines: I,
) -> Option<Vec<WakeLockStat>> {
    let mut scanner = WakeLockScanner::new();
    for line in lines {
        if scanner.push(&line) {
            break;
        }
    }
    scanner.finish()
}

/// `Wake lock 1000 AudioMix: 1m 2s 345ms (11 times) max=... realtime` -> one stat.
pub fn parse_wake_lock_row(trimmed: &str) -> Option<WakeLockStat> {
    let rest = trimmed.strip_prefix("Wake lock ")?;
    let (uid, tail) = rest.split_once(' ')?;
    let (tag, duration) = split_tag_and_duration(tail)?;
    let total_ms = parse_duration_ms(duration)?;
    let count = parse_lock_count(duration).unwrap_or(0);
    Some(WakeLockStat {
        uid: uid.to_string(),
        numeric_uid: resolve_uid(uid),
        tag: tag.to_string(),
        total_ms,
        count,
    })
}

/// Split `tag: 1m 2s 345ms (11 times) ...` at the first `": "` followed by a digit.
fn split_tag_and_duration(text: &str) -> Option<(&str, &str)> {
    let bytes = text.as_bytes();
    for index in 0..text.len().saturating_sub(1) {
        if bytes[index] == b':' && bytes[index + 1] == b' ' {
            let after = &text[index + 2..];
            if after.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                return Some((text[..index].trim(), after));
            }
        }
    }
    None
}

/// `1m 16s 158ms` -> milliseconds.
pub fn parse_duration_ms(text: &str) -> Option<i64> {
    let duration = text.split(" (").next()?.trim();
    let mut total = 0i64;
    let mut matched = false;
    for token in duration.split_whitespace() {
        let split = token.find(|c: char| !c.is_ascii_digit())?;
        let (digits, unit) = token.split_at(split);
        let value: i64 = digits.parse().ok()?;
        let factor = match unit {
            "ms" => 1,
            "s" => 1_000,
            "m" => 60_000,
            "h" => 3_600_000,
            "d" => 86_400_000,
            _ => return None,
        };
        total = total.saturating_add(value.saturating_mul(factor));
        matched = true;
    }
    matched.then_some(total)
}

/// `(11 times)` -> 11.
fn parse_lock_count(text: &str) -> Option<u32> {
    let open = text.find('(')?;
    let rest = &text[open + 1..];
    let close = rest.find(' ')?;
    rest[..close].parse().ok()
}

/// `u0a346` -> `10346`; plain numbers parse directly.
///
/// The `u<user>a<app>` form is Android's UID notation for app processes: `uid = user * 100000 +
/// 10000 + app`.
pub fn resolve_uid(uid: &str) -> Option<u32> {
    if let Ok(value) = uid.parse::<u32>() {
        return Some(value);
    }
    let rest = uid.strip_prefix('u')?;
    let (user, app) = rest.split_once('a')?;
    let user: u32 = user.parse().ok()?;
    let app: u32 = app.parse().ok()?;
    Some(
        user.saturating_mul(100_000)
            .saturating_add(10_000)
            .saturating_add(app),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed excerpt of a real Samsung/Android 16 dump.
    fn sample() -> Vec<String> {
        vec![
            "  Estimated power use (mAh):".to_string(),
            "    Capacity: 5000, Rated: 4855, Typical: 5000, Computed drain: 0.782, actual drain: 0.782".to_string(),
            "    Global".to_string(),
            "    screen: 1040 apps: 1040".to_string(),
            "    cpu: 40930 apps: 39920 duration: 6d 22h 34m 59s 501ms ".to_string(),
            "      (on battery, screen on)".to_string(),
            "      cpu: 36214 apps: 35353 duration: 6d 1h 1m 13s 163ms ".to_string(),
            "  UID 0: 12827 bg: 12827".to_string(),
            "      cpu=12827 cpu:bg=12827".to_string(),
            "      (on battery, screen on) cpu=12827 cpu:bg=12827".to_string(),
            "  UID 1000: 7894 bg: 7894".to_string(),
            "      cpu=7894 cpu:bg=7894".to_string(),
            "  UID u0a404: 533 fg: 31.5 bg: 114 cached: 277 (5m 14s 110ms)".to_string(),
            "      screen=110 cpu=403 cpu:fg=14.9 cpu:bg=113".to_string(),
            "  UID u0a166: 498 fg: 38.8 (35s 945ms) bg: 0.0898 fgs: 403 (4m 37s 533ms)".to_string(),
            "      screen=56.5 cpu=442".to_string(),
            "  UID 1083: 0.000326 bg: 0.000326".to_string(),
            "      cpu=0.000326".to_string(),
            "".to_string(),
            "  Some other section:".to_string(),
            "  UID u0a999: 999".to_string(),
        ]
    }

    #[test]
    fn parses_capacity_and_uid_rows() {
        let report = parse_power_section(sample()).expect("section");
        assert_eq!(report.capacity_mah, Some(5000.0));
        assert_eq!(report.computed_drain, Some(0.782));
        assert_eq!(report.actual_drain, Some(0.782));
        let uids: Vec<&str> = report.entries.iter().map(|e| e.uid.as_str()).collect();
        assert_eq!(uids, vec!["0", "1000", "u0a404", "u0a166", "1083"]);
        assert_eq!(report.entries[0].raw_power, 12827.0);
        assert_eq!(report.entries[2].raw_power, 533.0);
    }

    #[test]
    fn stops_at_the_next_section() {
        // The trailing `u0a999` row belongs to a later section in the dump and must not be counted.
        let report = parse_power_section(sample()).expect("section");
        assert!(!report.entries.iter().any(|e| e.uid == "u0a999"));
    }

    #[test]
    fn computes_shares_that_sum_to_one() {
        let report = parse_power_section(sample()).expect("section");
        let total: f64 = report.entries.iter().map(|e| e.share).sum();
        assert!((total - 1.0).abs() < 1e-9, "shares summed to {total}");
        // UID 0 dominates this sample: 12827 of 21752 total units.
        assert!((report.entries[0].share - 0.5897).abs() < 0.001);
    }

    #[test]
    fn converts_internal_units_to_mah() {
        let report = parse_power_section(sample()).expect("section");
        // 12827 internal units -> 12.8 mAh, not 12827 mAh.
        assert_eq!(PowerReport::power_mah(&report.entries[0]), 12.827);
        assert_eq!(
            report.total_raw,
            12827.0 + 7894.0 + 533.0 + 498.0 + 0.000326
        );
    }

    #[test]
    fn returns_none_without_the_section() {
        let lines = vec!["Something else".to_string(), "  UID 0: 1".to_string()];
        assert!(parse_power_section(lines).is_none());
    }

    #[test]
    fn ignores_a_section_with_no_uid_rows() {
        let lines = vec![
            "  Estimated power use (mAh):".to_string(),
            "    Capacity: 5000, Rated: 4855, Typical: 5000, Computed drain: 0.000, actual drain: 0.000".to_string(),
            "    Global".to_string(),
        ];
        let report = parse_power_section(lines).expect("section");
        assert!(report.entries.is_empty());
        assert_eq!(report.total_raw, 0.0);
    }

    #[test]
    fn json_output_is_sorted_and_limited() {
        let report = parse_power_section(sample()).expect("section");
        let value = report.to_json(2);
        let entries = value["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["uid"], json!("0"));
        assert_eq!(entries[0]["power_mah"], json!(12.8));
        assert_eq!(entries[1]["uid"], json!("1000"));
        assert_eq!(value["count"], json!(5));
    }

    #[test]
    fn parses_package_uid_mapping() {
        let lines = vec![
            "package:com.android.chrome uid:10123".to_string(),
            "package:com.sec.android.app.camera uid:10124".to_string(),
            "package:android uid:1000".to_string(),
            "package:com.shared.one uid:10150".to_string(),
            "package:com.shared.two uid:10150".to_string(),
            "garbage line".to_string(),
            "package:com.multi uid:10160,10161".to_string(),
        ];
        let map = parse_package_uids(lines);
        assert_eq!(
            map.get(&10123).unwrap(),
            &vec!["com.android.chrome".to_string()]
        );
        assert_eq!(map.get(&1000).unwrap(), &vec!["android".to_string()]);
        // Two packages sharing one UID are both kept, sorted.
        assert_eq!(
            map.get(&10150).unwrap(),
            &vec!["com.shared.one".to_string(), "com.shared.two".to_string()]
        );
        assert_eq!(map.get(&10161).unwrap(), &vec!["com.multi".to_string()]);
    }

    #[test]
    fn known_aid_uids_get_labels() {
        assert_eq!(aid_label(1000), Some("Android 系统"));
        assert_eq!(aid_label(1001), Some("电话服务"));
        assert_eq!(aid_label(54321), None);
    }
}

#[cfg(test)]
mod drain_scale_tests {
    use super::*;

    fn report(capacity: f64, total: f64) -> PowerReport {
        let mut report = PowerReport {
            capacity_mah: Some(capacity),
            ..Default::default()
        };
        report.total_raw = total;
        report
    }

    #[test]
    fn drain_percentage_is_computed_from_capacity() {
        // 42.9 mAh out of 5000 mAh is 0.86 %, regardless of what the device printed.
        let value = report(5000.0, 42_900.0).to_json(0);
        assert_eq!(value["drain_percent"], json!(0.86));
        assert_eq!(value["total_mah"], json!(42.9));
    }

    #[test]
    fn sub_milliamp_hour_rows_keep_three_decimals() {
        let mut report = report(5000.0, 0.0);
        report.entries.push(PowerEntry {
            uid: "u0a1".into(),
            numeric_uid: Some(10001),
            raw_power: 12.0,
            aid_label: None,
            packages: Vec::new(),
            share: 1.0,
        });
        let value = report.to_json(0);
        assert_eq!(value["entries"][0]["power_mah"], json!(0.012));
    }
}

#[cfg(test)]
mod wake_lock_tests {
    use super::*;

    const DUMP: &str = "  Estimated power use (mAh):\n\
  All partial wake locks:\n\
  Wake lock 1000 AnyMotionDetector: 1m 16s 158ms (11 times) max=9942 actual=88161 realtime\n\
  Wake lock u0a346 *job*e//SessionWorker/@eM_ADDR/androidx.work.SystemJobService: 28s 608ms (3 times) max=20422 actual=42251 realtime\n\
  Wake lock 1000 *alarm*:android.intent.action.TIME_TICK: 2s 5ms (1 times) max=2005 actual=2005 realtime\n\
\n\
  All wakeup reasons:\n";

    #[test]
    fn parses_rows_including_colons_inside_the_tag() {
        let stats = parse_wake_lock_section(DUMP.lines().map(str::to_string)).expect("section");
        assert_eq!(stats.len(), 3);
        assert_eq!(stats[0].uid, "1000");
        assert_eq!(stats[0].tag, "AnyMotionDetector");
        assert_eq!(stats[0].total_ms, 76_158);
        assert_eq!(stats[0].count, 11);
        assert_eq!(stats[1].numeric_uid, Some(10_346));
        assert_eq!(stats[2].tag, "*alarm*:android.intent.action.TIME_TICK");
        assert_eq!(stats[2].total_ms, 2_005);
    }

    #[test]
    fn scanner_stops_at_the_next_section() {
        let mut scanner = WakeLockScanner::new();
        let mut stopped_at = None;
        for (index, line) in DUMP.lines().enumerate() {
            if scanner.push(line) {
                stopped_at = Some(index);
                break;
            }
        }
        assert_eq!(
            stopped_at,
            Some(5),
            "stops on the blank line after the rows"
        );
        assert_eq!(scanner.finish().expect("section").len(), 3);
    }

    #[test]
    fn missing_section_is_reported_as_none() {
        let text = "Estimated power use (mAh):\n  UID 1000: 1\n";
        assert!(parse_wake_lock_section(text.lines().map(str::to_string)).is_none());
        // A present-but-empty section is `Some(vec![])`, which lets the UI say "nothing held".
        let empty = "All partial wake locks:\n\nNext section\n";
        assert_eq!(
            parse_wake_lock_section(empty.lines().map(str::to_string)),
            Some(Vec::new())
        );
    }

    #[test]
    fn durations_cover_every_unit() {
        assert_eq!(parse_duration_ms("1m 16s 158ms (11 times)"), Some(76_158));
        assert_eq!(parse_duration_ms("28s 608ms (3 times)"), Some(28_608));
        assert_eq!(parse_duration_ms("2h 3m 4s 5ms (1 times)"), Some(7_384_005));
        assert_eq!(parse_duration_ms("512ms (1 times)"), Some(512));
    }

    #[test]
    fn rid_rows_that_are_not_wake_locks_are_ignored() {
        assert!(parse_wake_lock_row("Wake lock 1000 NoDuration").is_none());
        assert!(parse_wake_lock_row("Wakeup reason 1000: 1s").is_none());
    }

    #[test]
    fn resolves_app_uids() {
        assert_eq!(resolve_uid("u0a346"), Some(10_346));
        assert_eq!(resolve_uid("1000"), Some(1000));
        assert_eq!(resolve_uid("u10a5"), Some(1_010_005));
        assert_eq!(resolve_uid("nonsense"), None);
    }
}
