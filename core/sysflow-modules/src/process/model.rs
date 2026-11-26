//! `/proc` parsing and policy primitives for the process module.
//!
//! Everything here is pure except [`clock_ticks`] (reads `/proc/self/auxv` through [`Sysfs`]), so
//! the parsing rules can be tested against fixtures without a device.

use sysflow_core::sysfs::Sysfs;

/// Fallback `USER_HZ` when the auxiliary vector is unavailable. Every Android ABI uses 100.
pub const CLOCK_FALLBACK: u64 = 100;

/// `AT_CLKTCK` in the ELF auxiliary vector.
const AT_CLKTCK: u64 = 17;

/// One `/proc/<pid>/stat` line, field numbers from `proc(5)` (1-based, `pid` is 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stat {
    pub pid: u32,
    pub comm: String,
    pub state: char,
    pub ppid: u32,
    pub utime: u64,
    pub stime: u64,
    pub threads: u32,
    pub starttime: u64,
    pub priority: i64,
    pub nice: i64,
    pub rt_priority: u32,
    pub policy: u32,
}

impl Stat {
    /// Total CPU ticks consumed by this process so far (`utime + stime`).
    pub fn ticks(&self) -> u64 {
        self.utime.saturating_add(self.stime)
    }
}

/// The subset of `/proc/<pid>/status` this module needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub uid: u32,
    pub rss_kb: Option<u64>,
    pub comm: String,
    pub kthread: bool,
}

/// cgroup membership from `/proc/<pid>/cgroup` (v1 controllers and the v2 unified line).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cgroup {
    /// The cgroup v2 unified path (`0::/apps/uid_10157/pid_12225`), if present.
    pub unified: Option<String>,
    /// The cpuset controller path from a v1 hierarchy, if present.
    pub cpuset: Option<String>,
    /// The freezer controller path from a v1 hierarchy (`/dev/freezer`), if present.
    pub freezer: Option<String>,
}

impl Cgroup {
    /// The cpuset group name (last path component), e.g. `top-app`; `None` for `/`.
    pub fn cpuset_group(&self) -> Option<String> {
        let path = self.cpuset.as_deref()?;
        let name = path.trim_end_matches('/').rsplit('/').next()?;
        (!name.is_empty()).then(|| name.to_string())
    }
}

/// Parse one `/proc/<pid>/stat` line.
///
/// `comm` is inside parentheses and may itself contain spaces and `)`, so the split is anchored on
/// the **last** `)` — splitting on the first one or by column index is the classic `/proc` bug.
pub fn parse_stat(text: &str) -> Option<Stat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close <= open {
        return None;
    }
    let pid = text[..open].trim().parse::<u32>().ok()?;
    let comm = text[open + 1..close].to_string();
    let tail: Vec<&str> = text[close + 1..].split_whitespace().collect();
    // tail[0] is field 3 (`state`); field n is tail[n - 3].
    let at = |field: usize| tail.get(field - 3).copied();
    Some(Stat {
        pid,
        comm,
        state: at(3)?.chars().next()?,
        ppid: at(4)?.parse().ok()?,
        utime: at(14)?.parse().ok()?,
        stime: at(15)?.parse().ok()?,
        priority: at(18)?.parse().ok()?,
        nice: at(19)?.parse().ok()?,
        threads: at(20)?.parse().ok()?,
        starttime: at(22)?.parse().ok()?,
        rt_priority: at(40).and_then(|v| v.parse().ok()).unwrap_or(0),
        policy: at(41).and_then(|v| v.parse().ok()).unwrap_or(0),
    })
}

/// Parse the fields of `/proc/<pid>/status` this module uses. A missing `Uid:` line is a hard
/// failure (returns `None`), because an unknown UID cannot be classified or protected.
pub fn parse_status(text: &str) -> Option<Status> {
    let mut status = Status::default();
    let mut saw_uid = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            status.uid = rest.split_whitespace().next()?.parse().ok()?;
            saw_uid = true;
        } else if let Some(rest) = line.strip_prefix("VmRSS:") {
            status.rss_kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok());
        } else if let Some(rest) = line.strip_prefix("Name:") {
            status.comm = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("Kthread:") {
            status.kthread = rest.trim() == "1";
        }
    }
    saw_uid.then_some(status)
}

/// First NUL-separated token of `/proc/<pid>/cmdline`, with trailing padding removed.
///
/// App processes carry the package name here; kernel threads have an empty file.
pub fn parse_cmdline(text: &str) -> String {
    text.split('\0')
        .map(str::trim)
        .find(|part| !part.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Parse `/proc/<pid>/cgroup`: the v2 unified line and any v1 `cpuset`/`freezer` lines.
pub fn parse_cgroup(text: &str) -> Cgroup {
    let mut cgroup = Cgroup::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(path) = line.strip_prefix("0::") {
            cgroup.unified = Some(path.to_string());
            continue;
        }
        // v1 format: "<id>:<comma,separated,controllers>:<path>"
        let mut parts = line.splitn(3, ':');
        let (Some(_id), Some(controllers), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        for controller in controllers.split(',') {
            if controller == "cpuset" {
                cgroup.cpuset = Some(path.to_string());
            } else if controller == "freezer" {
                cgroup.freezer = Some(path.to_string());
            }
        }
    }
    cgroup
}

/// `AT_CLKTCK` from `/proc/self/auxv`, falling back to [`CLOCK_FALLBACK`].
///
/// `sysconf(_SC_CLK_TCK)` must not be used: bionic's `_SC_*` numbering is not glibc's, and a real
/// S25 returned 2048 for `_SC_CLK_TCK` and 1 for `_SC_PAGESIZE` (see `docs/modules/process.md`).
pub fn clock_ticks(fs: &Sysfs) -> u64 {
    std::fs::read(fs.resolve("/proc/self/auxv"))
        .ok()
        .and_then(|bytes| parse_auxv(&bytes, AT_CLKTCK))
        .filter(|ticks| *ticks > 0)
        .unwrap_or(CLOCK_FALLBACK)
}

/// Read one 64-bit auxiliary-vector entry. The module ships only arm64 and the host tests are
/// 64-bit, where entries are `(type, value)` pairs of native-endian `u64`s terminated by type 0.
pub fn parse_auxv(bytes: &[u8], key: u64) -> Option<u64> {
    let (pairs, _) = bytes.as_chunks::<16>();
    for pair in pairs {
        let kind = u64::from_ne_bytes(pair[..8].try_into().ok()?);
        let value = u64::from_ne_bytes(pair[8..].try_into().ok()?);
        if kind == key {
            return Some(value);
        }
        if kind == 0 {
            break;
        }
    }
    None
}

/// Linux scheduling policy constants (`<linux/sched.h>`).
pub const SCHED_OTHER: i32 = 0;
pub const SCHED_FIFO: i32 = 1;
pub const SCHED_RR: i32 = 2;
pub const SCHED_BATCH: i32 = 3;
pub const SCHED_IDLE: i32 = 5;

/// Human-readable Linux scheduling policy name for `/proc/<pid>/stat` policy codes.
pub fn sched_policy_name(policy: u32) -> &'static str {
    match policy {
        0 => "SCHED_OTHER",
        1 => "SCHED_FIFO",
        2 => "SCHED_RR",
        3 => "SCHED_BATCH",
        5 => "SCHED_IDLE",
        6 => "SCHED_DEADLINE",
        _ => "UNKNOWN",
    }
}

/// Parse a scheduling policy name or numeric identifier (`other|fifo|rr|batch|idle`).
pub fn parse_sched_policy(text: &str) -> Option<i32> {
    match text.trim().to_ascii_lowercase().as_str() {
        "0" | "other" | "normal" | "sched_other" | "sched_normal" => Some(SCHED_OTHER),
        "1" | "fifo" | "sched_fifo" => Some(SCHED_FIFO),
        "2" | "rr" | "sched_rr" => Some(SCHED_RR),
        "3" | "batch" | "sched_batch" => Some(SCHED_BATCH),
        "5" | "idle" | "sched_idle" => Some(SCHED_IDLE),
        _ => None,
    }
}

/// Whether `policy` is a real-time policy (`SCHED_FIFO` or `SCHED_RR`) requiring priority `1..=99`.
pub fn is_realtime_policy(policy: i32) -> bool {
    policy == SCHED_FIFO || policy == SCHED_RR
}

/// Packages whose termination breaks the device badly enough that they are never offered as an
/// action target. UID < 10 000 already covers most platform processes; this list is the supplement
/// for system apps that run under an app UID. Mirrors `docs/process-module-plan.md`〈保护名单〉.
pub const PROTECTED_PACKAGES: &[&str] = &[
    "android",
    "com.android.systemui",
    "com.android.phone",
    "com.android.server.telecom",
    "com.android.bluetooth",
    "com.android.nfc",
];

/// Why a process may not be the target of `stop` / `oom` / `renice` / `sched` / `cpuset` / `freeze`.
///
/// The order is deliberate: kernel identity first, then the platform UID gate, then the package
/// list, then SysFlow itself.
pub fn protected_reason(
    uid: u32,
    pid: u32,
    kthread: bool,
    name: &str,
    package: Option<&str>,
) -> Option<&'static str> {
    if pid <= 2 {
        return Some("pid ≤ 2 is the kernel");
    }
    if kthread {
        return Some("kernel thread");
    }
    if uid < 10_000 {
        return Some("uid < 10000 (platform)");
    }
    let is_protected_package = |candidate: &str| {
        PROTECTED_PACKAGES
            .iter()
            .any(|pkg| candidate == *pkg || candidate.starts_with(&format!("{pkg}:")))
    };
    if is_protected_package(name) || package.is_some_and(is_protected_package) {
        return Some("system critical package");
    }
    if name == "sysflow" {
        return Some("sysflow itself");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_parses_the_fields_and_survives_parens_in_comm() {
        // Real line shape from the S25 probe, with `comm` forced to contain spaces and a `)`.
        let line = "26422 (my weird)) proc) R 26315 26422 26162 34816 26422 4194560 668 0 7 0 1 3 0 0 10 -10 1 0 2588536 11077787648 930 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0";
        let stat = parse_stat(line).expect("stat parses");
        assert_eq!(stat.pid, 26422);
        assert_eq!(stat.comm, "my weird)) proc");
        assert_eq!(stat.state, 'R');
        assert_eq!(stat.ppid, 26315);
        assert_eq!((stat.utime, stat.stime), (1, 3));
        assert_eq!(stat.priority, 10);
        assert_eq!(stat.nice, -10);
        assert_eq!(stat.threads, 1);
        assert_eq!(stat.starttime, 2588536);
        assert_eq!(stat.ticks(), 4);
    }

    #[test]
    fn stat_rejects_garbage() {
        assert!(parse_stat("").is_none());
        assert!(parse_stat("1234 no parens").is_none());
        assert!(parse_stat("notapid (x) S 1").is_none());
    }

    #[test]
    fn status_parses_uid_rss_and_kthread() {
        let text = "Name:\t.wallpaper.live\nState:\tS (sleeping)\nUid:\t10157\t10157\t10157\t10157\nVmRSS:\t  136684 kB\nThreads:\t19\n";
        let status = parse_status(text).expect("status parses");
        assert_eq!(status.uid, 10157);
        assert_eq!(status.rss_kb, Some(136_684));
        assert_eq!(status.comm, ".wallpaper.live");
        assert!(!status.kthread);
        assert!(
            parse_status("Name:\tfoo\n").is_none(),
            "no Uid means unusable"
        );
    }

    #[test]
    fn kernel_thread_status_has_kthread_flag_and_no_rss() {
        let text = "Name:\tkthreadd\nUid:\t0\t0\t0\t0\nKthread:\t1\n";
        let status = parse_status(text).unwrap();
        assert!(status.kthread);
        assert_eq!(status.rss_kb, None);
    }

    #[test]
    fn cmdline_takes_the_first_token_and_skips_padding() {
        assert_eq!(
            parse_cmdline("com.samsung.android.wallpaper.live\u{0}\u{0}"),
            "com.samsung.android.wallpaper.live"
        );
        assert_eq!(parse_cmdline("\u{0}\u{0}"), "");
    }

    #[test]
    fn cgroup_reads_both_the_v2_line_and_the_v1_cpuset_controller() {
        let text = "5:freezer:/bg\n4:cpuset:/top-app\n3:cpuacct:/top-app\n2:cpu:/top-app\n1:blkio:/top\n0::/apps/uid_10157/pid_12225\n";
        let cgroup = parse_cgroup(text);
        assert_eq!(cgroup.unified.as_deref(), Some("/apps/uid_10157/pid_12225"));
        assert_eq!(cgroup.cpuset.as_deref(), Some("/top-app"));
        assert_eq!(cgroup.freezer.as_deref(), Some("/bg"));
        assert_eq!(cgroup.cpuset_group().as_deref(), Some("top-app"));
    }

    #[test]
    fn cpuset_root_has_no_group_name() {
        let text = "4:cpuset:/\n0::/\n";
        assert_eq!(parse_cgroup(text).cpuset_group(), None);
    }

    #[test]
    fn auxv_finds_clktck_and_ignores_unknown_entries() {
        let mut bytes = Vec::new();
        for (kind, value) in [(6u64, 4096u64), (AT_CLKTCK, 100), (0, 0)] {
            bytes.extend_from_slice(&kind.to_ne_bytes());
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        assert_eq!(parse_auxv(&bytes, AT_CLKTCK), Some(100));
        assert_eq!(parse_auxv(&bytes, 12), None);
        assert_eq!(parse_auxv(&bytes[..8], AT_CLKTCK), None, "truncated pair");
    }

    #[test]
    fn protection_is_ordered_and_package_aware() {
        assert_eq!(
            protected_reason(0, 1, false, "init", None),
            Some("pid ≤ 2 is the kernel")
        );
        assert_eq!(
            protected_reason(0, 42, true, "kworker/0:1", None),
            Some("kernel thread")
        );
        assert_eq!(
            protected_reason(1000, 900, false, "system_server", Some("android")),
            Some("uid < 10000 (platform)")
        );
        assert_eq!(
            protected_reason(
                10201,
                9000,
                false,
                "com.android.systemui",
                Some("com.android.systemui")
            ),
            Some("system critical package")
        );
        assert_eq!(
            protected_reason(10346, 9001, false, "sysflow", None),
            Some("sysflow itself")
        );
        assert_eq!(
            protected_reason(
                10346,
                9002,
                false,
                "com.example.app",
                Some("com.example.app")
            ),
            None
        );
    }

    #[test]
    fn sched_policy_helpers_cover_aliases_and_names() {
        assert_eq!(parse_sched_policy("other"), Some(SCHED_OTHER));
        assert_eq!(parse_sched_policy("SCHED_NORMAL"), Some(SCHED_OTHER));
        assert_eq!(parse_sched_policy("FIFO"), Some(SCHED_FIFO));
        assert_eq!(parse_sched_policy("2"), Some(SCHED_RR));
        assert_eq!(parse_sched_policy("batch"), Some(SCHED_BATCH));
        assert_eq!(parse_sched_policy("idle"), Some(SCHED_IDLE));
        assert_eq!(parse_sched_policy("deadline"), None);
        assert!(is_realtime_policy(SCHED_FIFO));
        assert!(is_realtime_policy(SCHED_RR));
        assert!(!is_realtime_policy(SCHED_OTHER));
        assert!(!is_realtime_policy(SCHED_BATCH));
        assert!(!is_realtime_policy(SCHED_IDLE));
        assert_eq!(sched_policy_name(0), "SCHED_OTHER");
        assert_eq!(sched_policy_name(1), "SCHED_FIFO");
        assert_eq!(sched_policy_name(5), "SCHED_IDLE");
        assert_eq!(sched_policy_name(99), "UNKNOWN");
    }
}
