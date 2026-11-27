//! Parsers for wake-lock sources: kernel `wakeup_sources` and userspace locks from `dumpsys power`.
//!
//! Two kernel formats exist and they are not interchangeable:
//!
//! * `/sys/kernel/debug/wakeup_sources` — **9** counters per row
//!   (`active_count`, `event_count`, `wakeup_count`, `expire_count`, `active_since`,
//!   `total_time`, `max_time`, `last_change`, `prevent_suspend_time`), times in **milliseconds**.
//! * `/proc/wakelocks` — the pre-3.6 interface with **8** counters (no `event_count`), times in
//!   **nanoseconds**.
//!
//! The parsers here are deliberately tolerant: they are fed by a vendor kernel, so anything that
//! does not look like a row is skipped rather than turned into a fake source. This module is used by
//! the `wakelock` module and (later) by the process page, so it lives in core next to
//! `batterystats.rs` rather than inside the module.

use std::time::Duration;

/// Time unit of a kernel wakeup-sources table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeUnit {
    Millis,
    Nanos,
}

impl TimeUnit {
    fn to_millis(self, raw: i64) -> i64 {
        match self {
            TimeUnit::Millis => raw,
            TimeUnit::Nanos => raw / 1_000_000,
        }
    }
}

/// One kernel wakeup source.
#[derive(Clone, Debug, PartialEq)]
pub struct KernelSource {
    pub name: String,
    pub active_count: i64,
    pub event_count: i64,
    pub wakeup_count: i64,
    pub expire_count: i64,
    /// Milliseconds since activation; `0` when the source is not active.
    pub active_since_ms: i64,
    pub total_ms: i64,
    pub max_ms: i64,
    pub last_change_ms: i64,
    pub prevent_suspend_ms: i64,
}

impl KernelSource {
    pub fn is_active(&self) -> bool {
        self.active_since_ms > 0
    }
}

/// A wake lock currently held by a userspace process (`dumpsys power`).
#[derive(Clone, Debug, PartialEq)]
pub struct HeldWakelock {
    /// Lock class, e.g. `PARTIAL_WAKE_LOCK`.
    pub kind: String,
    pub tag: String,
    /// How long the lock has been held.
    pub held_ms: i64,
    pub uid: Option<i64>,
    pub pid: Option<i64>,
    pub package: Option<String>,
}

/// Coarse category of a kernel wakeup source.
///
/// Raw names (`rmnet_ipa%d`, `IPA_CLIENT_APPS_WAN_COAL_CONS`, `smp2p-sleepstate`) are meaningless to
/// a user, and no kernel interface maps them to apps — one packet send lights up the whole network
/// driver path. Grouping by driver family is the honest middle ground: it says *what kind of hardware
/// activity* kept the device awake without pretending to know which app caused it.
///
/// The identifiers are stable and language-neutral; clients own the display strings.
pub fn classify_wakeup_source(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    let has = |needle: &str| n.contains(needle);

    // Order matters: `bluetooth_timer` is bluetooth before it is a timer, `set_mpp_cover_ws` is an
    // accessory before anything else, and `IPA_CLIENT_APPS_...` is networking before "apps".
    if has("powermanager") || has("wakeup_reason") {
        "system"
    } else if has("bluetooth") || has("bt_") || has("hci_") || has("bluez") {
        "bluetooth"
    } else if has("nfc") {
        "nfc"
    } else if has("rmnet") || has("ipa") || has("wlan") || has("wifi") || has("wwan") || has("mhi")
    {
        "network"
    } else if has("alarm") || has("hrtimer") || has("rtc") || has("timer") {
        "timer"
    } else if has("tsp")
        || has("touch")
        || has("wacom")
        || has("synaptics")
        || has("goodix")
        || has("focaltech")
    {
        "touch"
    } else if has("panel") || has("display") || has("mdss") || has("backlight") || has("dpu") {
        "display"
    } else if has("battery")
        || has("charger")
        || has("power_supply")
        || has("vbus")
        || has("fuel_gauge")
        || has("smb")
    {
        "power"
    } else if has("audio") || has("adsp") || has("asoc") || has("codec") || has("wcd") {
        "audio"
    } else if has("camera") || has("csiphy") || has("isp_") {
        "camera"
    } else if has("sensor") || has("ssp_") || has("accel") || has("gyro") || has("proximity") {
        "sensor"
    } else if has("mmc") || has("ufs") || has("sdhci") || has("scsi") || has("nvme") {
        "storage"
    } else if has("usb") || has("dwc") || has("typec") || has("extcon") {
        "usb"
    } else if has("cover") || has("mpp") || has("hall") || has("keyboard") {
        "accessory"
    } else if has("smp2p") || has("glink") || has("smem") || has("modem") || has("qpnp") {
        "subsystem"
    } else if has("uart") || has("qup") || has("i2c") || has("spi") {
        "serial"
    } else {
        "other"
    }
}

/// Parse a kernel wakeup-sources table.
pub fn parse_kernel_sources(text: &str, unit: TimeUnit) -> Vec<KernelSource> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("name") {
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        let numbers: Vec<i64> = fields
            .take(9)
            .map_while(|field| field.parse::<i64>().ok())
            .collect();
        // 9 counters: debugfs layout. 8: the legacy /proc/wakelocks layout.
        let source = match numbers.len() {
            9 => KernelSource {
                name: name.to_string(),
                active_count: numbers[0],
                event_count: numbers[1],
                wakeup_count: numbers[2],
                expire_count: numbers[3],
                active_since_ms: unit.to_millis(numbers[4]),
                total_ms: unit.to_millis(numbers[5]),
                max_ms: unit.to_millis(numbers[6]),
                last_change_ms: unit.to_millis(numbers[7]),
                prevent_suspend_ms: unit.to_millis(numbers[8]),
            },
            8 => KernelSource {
                name: name.to_string(),
                active_count: numbers[0],
                // The legacy interface has no event counter; it is not the same number as
                // `active_count`, so report 0 instead of inventing one.
                event_count: 0,
                wakeup_count: numbers[2],
                expire_count: numbers[1],
                active_since_ms: unit.to_millis(numbers[3]),
                total_ms: unit.to_millis(numbers[4]),
                max_ms: unit.to_millis(numbers[5]),
                last_change_ms: unit.to_millis(numbers[6]),
                prevent_suspend_ms: unit.to_millis(numbers[7]),
            },
            _ => continue,
        };
        out.push(source);
    }
    out
}

/// Parse the `Wake Locks:` block of `dumpsys power`.
///
/// A typical entry:
///
/// ```text
/// Wake Locks: size=2
///   PARTIAL_WAKE_LOCK              'AudioMix' ACQ=-1m2s (uid=1041, pid=1234, pkg=com.android.systemui)
/// ```
pub fn parse_power_wakelocks(text: &str) -> Vec<HeldWakelock> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Wake Locks:") {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if trimmed.is_empty() {
            // A blank line ends the section; entries are contiguous in every observed build.
            if !out.is_empty() {
                break;
            }
            continue;
        }
        // The section ends at the next label (`Locks held:`, `Global wake locks:` ...).
        if !trimmed.contains('\'') {
            break;
        }
        let Some((kind, rest)) = split_kind(trimmed) else {
            continue;
        };
        let Some(tag) = quoted(rest) else {
            continue;
        };
        let held_ms = rest
            .find("ACQ=")
            .and_then(|at| parse_acq_ms(&rest[at + 4..]))
            .unwrap_or(0);
        let (uid, pid, package) = parse_attribution(rest);
        out.push(HeldWakelock {
            kind,
            tag,
            held_ms,
            uid,
            pid,
            package,
        });
    }
    out
}

/// `PARTIAL_WAKE_LOCK 'tag' ...` -> (`PARTIAL_WAKE_LOCK`, `'tag' ...`).
fn split_kind(line: &str) -> Option<(String, &str)> {
    let quote = line.find('\'')?;
    let head = line[..quote].trim();
    // `WindowManager` style headers appear with flags such as `PARTIAL_WAKE_LOCK   'WindowManager'`
    let head = head.split_whitespace().last()?;
    if head.is_empty() {
        return None;
    }
    Some((head.to_string(), &line[quote..]))
}

fn quoted(text: &str) -> Option<String> {
    let start = text.find('\'')? + 1;
    let rest = &text[start..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

fn parse_attribution(text: &str) -> (Option<i64>, Option<i64>, Option<String>) {
    let Some(open) = text.find('(') else {
        return (None, None, None);
    };
    let Some(close) = matching_paren(text, open) else {
        return (None, None, None);
    };
    let mut uid = None;
    let mut pid = None;
    let mut package = None;
    // Layouts differ: AOSP prints `(uid=1041, pid=1234, pkg=com.android.systemui)`, Samsung prints
    // space-separated pairs with a nested work source and no `pkg=`
    // (`(uid=1041 isFrozen=false ws=WorkSource{ chains=WorkChain{(10390), (1041)}} lock=4c24045)`).
    // Splitting on commas *and* whitespace covers both, and one value never contains a space.
    for field in text[open + 1..close].split([',', ' ']) {
        let Some((key, value)) = field.trim().split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key {
            "uid" => uid = value.parse::<i64>().ok(),
            "pid" => pid = value.parse::<i64>().ok(),
            "pkg" => package = Some(value.to_string()).filter(|value| !value.is_empty()),
            _ => {}
        }
    }
    if let Some(name) = package.as_deref() {
        // A work source can be printed as `pkg=com.foo:remote`; the package is the part before it.
        if let Some((base, _)) = name.split_once(':') {
            package = Some(base.to_string());
        }
    }
    (uid, pid, package)
}

/// Index of the `)` that closes the `(` at `open`, counting nesting.
///
/// A plain `find(')')` stops at the first inner pair, which on vendor builds is inside
/// `WorkSource{ chains=WorkChain{(10390), (1041)}}` — the attribution was cut off before `pid`/`pkg`.
fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in text[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// `-1m2s`, `-512ms`, `-1h2m3s512ms` -> milliseconds.
fn parse_acq_ms(text: &str) -> Option<i64> {
    let text = text.trim_start_matches('-');
    let mut total = 0i64;
    let mut number = String::new();
    let mut rest = text;
    let mut matched_any = false;
    while !rest.is_empty() {
        let ch = rest.chars().next()?;
        if ch.is_ascii_digit() {
            number.push(ch);
            rest = &rest[1..];
            continue;
        }
        if number.is_empty() {
            // A stray flag character (`ON_AFTER_RELEASE`) means the duration already ended.
            break;
        }
        let (factor, consumed) = unit_factor(rest)?;
        total = total.saturating_add(number.parse::<i64>().ok()?.saturating_mul(factor));
        matched_any = true;
        number.clear();
        rest = &rest[consumed..];
    }
    matched_any.then_some(total)
}

/// Milliseconds per unit, plus how many characters the unit consumed.
fn unit_factor(text: &str) -> Option<(i64, usize)> {
    // `ACQ=` durations use these units only; micro/nanoseconds never appear in `dumpsys power`.
    for (unit, factor) in [
        ("ms", 1i64),
        ("h", 3_600_000),
        ("m", 60_000),
        ("s", 1_000),
        ("d", 86_400_000),
    ] {
        if text.starts_with(unit) {
            return Some((factor, unit.len()));
        }
    }
    None
}

/// The most recent wakeup `dumpsys power` reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastWakeup {
    /// Raw reason string. Kernel reason codes are vendor-specific, so this is not translated.
    pub reason: String,
    /// Milliseconds since boot when the wakeup happened, when the build prints it.
    pub uptime_ms: Option<i64>,
    /// `(802127 ms ago)` — Samsung prints the age next to `mLastWakeTime` instead of a usable
    /// uptime, so this is read directly rather than recomputed against `/proc/uptime`.
    pub age_ms: Option<i64>,
}

/// Keys that carry the last wakeup reason, most specific first.
///
/// Builds differ (`Last wakeup reason: …`, `mLastWakeupReason=…`) and several print neither; the
/// parser returns `None` rather than inventing an answer, so the client can say "this ROM does not
/// report it" instead of showing something meaningless.
const WAKEUP_REASON_KEYS: &[&str] = &[
    "last wakeup reason",
    "mlastwakeupreason",
    "mlastwakereason",
    "wakeup reason",
];

/// Keys that carry the wakeup's uptime (milliseconds since boot).
const WAKEUP_UPTIME_KEYS: &[&str] = &[
    "mlastwakeupuptimems",
    "last wakeup uptime",
    "mlastwakeuptime",
];

/// Keys whose value may carry an already-computed `(N ms ago)` age.
const WAKEUP_AGE_KEYS: &[&str] = &["mlastwaketime", "last wake time"];

/// Parse the last wakeup out of a `dumpsys power` dump.
pub fn parse_last_wakeup(dump: &str) -> Option<LastWakeup> {
    let mut reason: Option<String> = None;
    let mut uptime_ms: Option<i64> = None;
    let mut age_ms: Option<i64> = None;
    for line in dump.lines() {
        let trimmed = line.trim();
        let Some((raw_key, value)) = split_field(trimmed) else {
            continue;
        };
        let key = raw_key.to_ascii_lowercase();
        if reason.is_none() && WAKEUP_REASON_KEYS.contains(&key.as_str()) {
            if !value.is_empty() {
                reason = Some(value.to_string());
            }
        } else if uptime_ms.is_none() && WAKEUP_UPTIME_KEYS.contains(&key.as_str()) {
            uptime_ms = value.parse::<i64>().ok().filter(|value| *value >= 0);
        } else if age_ms.is_none() && WAKEUP_AGE_KEYS.contains(&key.as_str()) {
            age_ms = parse_age_hint(value);
        }
    }
    reason.map(|reason| LastWakeup {
        reason,
        uptime_ms,
        age_ms,
    })
}

/// `"87574287 (802127 ms ago)"` -> `802127`.
fn parse_age_hint(value: &str) -> Option<i64> {
    let start = value.find('(')? + 1;
    let rest = &value[start..];
    let end = rest.find("ms ago")?;
    rest[..end]
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|age| *age >= 0)
}

/// `"mLastWakeupReason=abc"` or `"Last wakeup reason: abc"` -> (`"mlastwakeupreason"`, `"abc"`).
fn split_field(line: &str) -> Option<(&str, &str)> {
    let at = line.find([':', '='])?;
    let key = line[..at].trim();
    if key.is_empty() {
        return None;
    }
    Some((key, line[at + 1..].trim()))
}

/// `/proc/uptime` is `"12345.67 45678.90"`; only the first field matters.
///
/// Lives here rather than in the wakelock module because both the window (reboot detection) and the
/// `wakelock wakeup` verb (aging a wakeup) need it, and a second copy would drift.
pub fn parse_uptime(text: &str) -> Option<u64> {
    let first = text.split_whitespace().next()?;
    let seconds = first.parse::<f64>().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(seconds as u64)
}

/// The default timeout for a `dumpsys`-backed wake-lock query.
pub const HOLD_QUERY_TIMEOUT: Duration = Duration::from_secs(8);

#[cfg(test)]
mod tests {
    use super::*;

    const DEBUGFS: &str = "name\t\tactive_count\tevent_count\twakeup_count\texpire_count\tactive_since\ttotal_time\tmax_time\tlast_change\tprevent_suspend_time\n\
PowerManagerService.WakeLocks\t4\t6\t2\t0\t1234\t482348\t300000\t482348\t0\n\
wlan_wow_wl\t0\t0\t5\t0\t0\t12044\t12000\t12044\t900\n\
qpnp-power-on\t0\t0\t0\t0\t0\t0\t0\t0\t0\n";

    const PROC: &str = "name\t\tactive_count\texpire_count\twakeup_count\tactive_since\ttotal_time\tmax_time\tlast_change\tprevent_suspend_time\n\
event0-123\t1\t0\t0\t1500000000\t4000000000\t4000000000\t1500000000\t0\n";

    #[test]
    fn parses_debugfs_rows_in_milliseconds() {
        let rows = parse_kernel_sources(DEBUGFS, TimeUnit::Millis);
        assert_eq!(rows.len(), 3);
        let first = &rows[0];
        assert_eq!(first.name, "PowerManagerService.WakeLocks");
        assert_eq!(first.active_count, 4);
        assert_eq!(first.event_count, 6);
        assert_eq!(first.wakeup_count, 2);
        assert_eq!(first.total_ms, 482_348);
        assert_eq!(first.prevent_suspend_ms, 0);
        assert!(first.is_active());
        assert!(!rows[1].is_active());
    }

    #[test]
    fn parses_legacy_proc_rows_in_nanoseconds() {
        let rows = parse_kernel_sources(PROC, TimeUnit::Nanos);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.name, "event0-123");
        assert_eq!(row.active_count, 1);
        assert_eq!(row.event_count, 0, "legacy format has no event counter");
        assert_eq!(row.total_ms, 4_000, "4 s of nanoseconds");
        assert!(row.is_active());
    }

    #[test]
    fn skips_malformed_rows_instead_of_inventing_sources() {
        let text = "name active_count\nbroken\nx 1 2 3\n";
        assert!(parse_kernel_sources(text, TimeUnit::Millis).is_empty());
    }

    #[test]
    fn parses_userspace_wake_locks() {
        let dump = "Power Manager State:\n  mWakefulness=Awake\nWake Locks: size=2\n\
  PARTIAL_WAKE_LOCK              'AudioMix' ACQ=-1m2s (uid=1041, pid=1234, pkg=com.android.systemui)\n\
  PARTIAL_WAKE_LOCK              'LocationManagerService' ACQ=-512ms (uid=1000, pid=999, pkg=android)\n\
\n  mIsPowered=true\n";
        let locks = parse_power_wakelocks(dump);
        assert_eq!(locks.len(), 2);
        assert_eq!(locks[0].kind, "PARTIAL_WAKE_LOCK");
        assert_eq!(locks[0].tag, "AudioMix");
        assert_eq!(locks[0].held_ms, 62_000);
        assert_eq!(locks[0].uid, Some(1041));
        assert_eq!(locks[0].package.as_deref(), Some("com.android.systemui"));
        assert_eq!(locks[1].tag, "LocationManagerService");
        assert_eq!(locks[1].held_ms, 512);
    }

    #[test]
    fn parses_samsung_attribution_without_commas_or_pkg() {
        // Verbatim from a Samsung device: space-separated pairs, a nested work source, no `pkg=`.
        let dump = "Wake Locks: size=2\n\
            \x20 PARTIAL_WAKE_LOCK                 'AudioMix' ACQ=-145ms (uid=1041 isFrozen=false isAttributedUidCached=false ws=WorkSource{ chains=WorkChain{(10390), (1041)}} powerGroupId=-1 displayId=-1 lock=4c24045)\n\
            \x20 PARTIAL_WAKE_LOCK                 '*vibrator*' ACQ=-39ms (uid=1000 isFrozen=false isAttributedUidCached=false pid=14922 ws=WorkSource{10522} powerGroupId=0 displayId=0 lock=e2ca7f1)\n";
        let locks = parse_power_wakelocks(dump);
        assert_eq!(locks.len(), 2);
        assert_eq!(locks[0].tag, "AudioMix");
        assert_eq!(locks[0].held_ms, 145);
        assert_eq!(
            locks[0].uid,
            Some(1041),
            "the nested parens must not cut parsing short"
        );
        assert_eq!(locks[0].pid, None);
        assert_eq!(locks[0].package, None, "this layout prints no package");

        assert_eq!(locks[1].tag, "*vibrator*");
        assert_eq!(locks[1].uid, Some(1000));
        assert_eq!(locks[1].pid, Some(14_922));
    }

    #[test]
    fn empty_wake_lock_section_yields_nothing() {
        assert!(parse_power_wakelocks("Wake Locks: size=0\n").is_empty());
        assert!(parse_power_wakelocks("Power Manager State:\n").is_empty());
    }

    #[test]
    fn uptime_parsing_is_tolerant() {
        assert_eq!(parse_uptime("12345.67 45678.90\n"), Some(12_345));
        assert_eq!(parse_uptime("0.00 0.00"), Some(0));
        assert_eq!(parse_uptime(""), None);
        assert_eq!(parse_uptime("garbage"), None);
        assert_eq!(parse_uptime("nan 1"), None);
    }

    #[test]
    fn parses_the_last_wakeup_from_common_builds() {
        let aosp = "Power Manager State:\n  mWakefulness=Awake\n  Last wakeup reason: 1234\n";
        let wakeup = parse_last_wakeup(aosp).expect("reason on its own line");
        assert_eq!(wakeup.reason, "1234");
        assert_eq!(wakeup.uptime_ms, None);

        let vendor = "mLastWakeupReason=wakeup from IRQ\nmLastWakeupUptimeMs=987654\n";
        let wakeup = parse_last_wakeup(vendor).expect("vendor style");
        assert_eq!(wakeup.reason, "wakeup from IRQ");
        assert_eq!(wakeup.uptime_ms, Some(987_654));
    }

    #[test]
    fn parses_a_samsung_last_wakeup_with_its_printed_age() {
        // Verbatim: the reason key has a capital "Up", and the age is printed next to the time.
        let dump = "  mLastWakeUpReason: 19\n  mLastWakeTime=87574287 (802127 ms ago)\n\
            \x20 mLastWakeReason=1\n";
        let wakeup = parse_last_wakeup(dump).expect("reason");
        assert_eq!(wakeup.reason, "19");
        assert_eq!(wakeup.age_ms, Some(802_127));
        assert_eq!(wakeup.uptime_ms, None, "no uptime field on this build");
    }

    #[test]
    fn a_dump_without_a_wakeup_reason_yields_nothing() {
        assert!(parse_last_wakeup("Power Manager State:\n  mWakefulness=Awake\n").is_none());
        assert!(
            parse_last_wakeup("Last wakeup reason: \n").is_none(),
            "empty value"
        );
        // `mLastWakeTime` is a different field and must not be mistaken for the reason.
        assert!(parse_last_wakeup("mLastWakeTime=12345\n").is_none());
    }

    #[test]
    fn parses_composite_durations() {
        assert_eq!(parse_acq_ms("-1h2m3s512ms"), Some(3_723_512));
        assert_eq!(parse_acq_ms("-45s"), Some(45_000));
        assert_eq!(parse_acq_ms("garbage"), None);
    }
}

#[cfg(test)]
mod classify_tests {
    use super::classify_wakeup_source;

    #[test]
    fn classifies_the_rows_seen_on_a_galaxy_device() {
        // Names taken verbatim from a `wakelock.ranked` payload on a Qualcomm Samsung device.
        let cases = [
            ("rmnet_ipa%d", "network"),
            ("IPA_CLIENT_APPS_WAN_COAL_CONS", "network"),
            ("rmnet_ctl", "network"),
            ("898000.qcom,qup_uart", "serial"),
            ("hal_bluetooth_lock", "bluetooth"),
            ("bluetooth_timer", "bluetooth"),
            ("smp2p-sleepstate", "subsystem"),
            ("nfc_wake_lock", "nfc"),
            ("sec-battery-monitor", "power"),
            ("TSP", "touch"),
            ("wacom_wakelock", "touch"),
            ("alarmtimer.0.auto", "timer"),
            ("set_mpp_cover_ws", "accessory"),
            ("PowerManagerService.WakeLocks", "system"),
            ("something_unknown_here", "other"),
        ];
        for (name, expected) in cases {
            assert_eq!(
                classify_wakeup_source(name),
                expected,
                "misclassified {name}"
            );
        }
    }

    #[test]
    fn classification_is_case_insensitive() {
        assert_eq!(classify_wakeup_source("NFC_WAKE_LOCK"), "nfc");
        assert_eq!(classify_wakeup_source("Wlan_wow_wl"), "network");
    }
}
