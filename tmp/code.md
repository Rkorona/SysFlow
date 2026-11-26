

### 第 1 批（4 个文件）：`model.rs`、`mod.rs`、`process_actions.rs`、`main.rs`

#### 1. `core/sysflow-modules/src/process/model.rs`

```rust
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
```

---

#### 2. `core/sysflow-modules/src/process/mod.rs`

```rust
//! Process module — verb-only (see `docs/process-module-plan.md`).
//!
//! The process list is dynamic and expensive (two `/proc` scans ~500 ms apart plus a `dumpsys`
//! foreground lookup), so this module deliberately:
//!
//! - has **no schema items** and no `apply` path: every operation is a one-shot `sysflow process`
//!   verb, not a persisted setting;
//! - is **excluded from `snapshot`** (`in_snapshot() == false`), so the APK's home polling never
//!   pays for a process scan;
//! - never samples history and never participates in the daemon, boot replay or safe mode.
//!
//! Step 1 ships only the capability probe (`sysflow process probe`); list/get/actions arrive in
//! Steps 2–4. Keeping the module registered from the start means `caps` already tells the client
//! whether this device can support the page at all.

pub mod actions;
pub mod foreground;
pub mod model;
pub mod packages;
pub mod scan;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::module::{ApplyResult, Caps, Module};
use sysflow_core::schema::Schema;
use sysflow_core::sysfs::Sysfs;

/// Every path probed below is read through [`Sysfs::resolve`], so the host test tree can fake it.
pub struct ProcessModule {
    fs: Sysfs,
}

impl ProcessModule {
    pub fn new(fs: Sysfs) -> Self {
        ProcessModule { fs }
    }

    /// `/proc/<pid>` exists and is readable in this namespace. Nothing else can work without it.
    fn proc_readable(&self) -> bool {
        self.fs.exists("/proc/self/stat") && self.fs.exists("/proc/self/status")
    }

    fn is_dir(&self, path: &str) -> bool {
        std::fs::metadata(self.fs.resolve(path))
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
    }

    /// At least one real group directory has a `tasks` node (`/dev/cpuset/<group>/tasks`).
    ///
    /// The top of `/dev/cpuset` also holds `cpus`, `tasks`, `cgroup.procs`, … so existence alone is
    /// not enough: a file named `tasks` at the root must not be mistaken for a group.
    fn cpuset_available(&self) -> bool {
        let Ok(groups) = self.fs.list_dir("/dev/cpuset") else {
            return false;
        };
        groups.iter().any(|group| {
            let dir = format!("/dev/cpuset/{group}");
            self.is_dir(&dir) && self.fs.exists(&format!("{dir}/tasks"))
        })
    }

    /// cgroup freezer, probed cheaply.
    ///
    /// Android 12+ puts the freeze switch at `/sys/fs/cgroup/apps/uid_<uid>/pid_<pid>/cgroup.freeze`
    /// (the S25 unified cgroup path is `/apps/uid_10157/pid_12225`); some builds use the older
    /// `/sys/fs/cgroup/uid_*` layout, and Android 11 exposes `/dev/freezer`. `am freeze` is the
    /// runtime fallback, but `probe()` must not fork, so it is not used for detection.
    ///
    /// Bounded scan (2 roots × 8 UIDs × 4 PIDs): `probe()` runs on every `caps` call.
    fn freezer_available(&self) -> bool {
        if self.fs.exists("/dev/freezer") {
            return true;
        }
        for root in ["/sys/fs/cgroup/apps", "/sys/fs/cgroup"] {
            let Ok(entries) = self.fs.list_dir(root) else {
                continue;
            };
            for uid in entries.iter().filter(|e| e.starts_with("uid_")).take(8) {
                let Ok(pids) = self.fs.list_dir(&format!("{root}/{uid}")) else {
                    continue;
                };
                for pid in pids.iter().filter(|e| e.starts_with("pid_")).take(4) {
                    if self.fs.exists(&format!("{root}/{uid}/{pid}/cgroup.freeze")) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

impl Module for ProcessModule {
    fn id(&self) -> &'static str {
        "process"
    }

    fn probe(&self) -> Caps {
        if !self.proc_readable() {
            return Caps::unsupported("/proc/<pid> is not readable");
        }
        let mut features = vec!["list", "sched"];
        if self.fs.exists("/system/bin/dumpsys") {
            features.push("foreground");
        }
        if self.fs.exists("/system/bin/am") {
            features.push("stop");
        }
        if self.fs.exists("/system/bin/renice") {
            features.push("renice");
        }
        if self.fs.exists("/proc/self/oom_score_adj") {
            features.push("oom_score_adj");
        }
        if self.cpuset_available() {
            features.push("cpuset");
        }
        if self.freezer_available() {
            features.push("freeze");
        }
        Caps::supported(&features)
    }

    /// No tunable items: the process page is driven by verbs, not by `schema` / `set`.
    fn schema(&self) -> Schema {
        Schema {
            module: "process".to_string(),
            supported: self.proc_readable(),
            items: Vec::new(),
        }
    }

    /// `snapshot` never reaches this (see [`Module::in_snapshot`]); a direct `get` returns nothing.
    fn collect(&self, _keys: Option<&[String]>) -> Result<Value> {
        Ok(json!({}))
    }

    /// No persisted settings exist, so there is nothing to apply. Point callers at the verbs
    /// instead of pretending a `set` could work.
    fn apply(&self, _req: &Map<String, Value>) -> Result<ApplyResult> {
        Err(Error::invalid_arg(
            "process is verb-only: use `sysflow process list|get|stop|oom|renice|sched|cpuset|freeze|unfreeze`",
        ))
    }

    fn in_snapshot(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sysflow-process-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_missing_proc_marks_the_module_unsupported() {
        let root = temp_root("unsupported");
        let module = ProcessModule::new(Sysfs::new(&root));
        let caps = module.probe();
        assert!(!caps.supported);
        assert!(caps.reason.is_some());
        assert!(module.schema().items.is_empty());
    }

    #[test]
    fn probe_advertises_only_the_mechanisms_that_exist() {
        let root = temp_root("features");
        write(
            &root,
            "proc/self/stat",
            "1 (init) S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        );
        write(&root, "proc/self/status", "Name:\tinit\nUid:\t0\t0\t0\t0\n");
        write(&root, "proc/self/oom_score_adj", "0\n");
        write(&root, "system/bin/dumpsys", "");
        write(&root, "system/bin/am", "");
        write(&root, "system/bin/renice", "");
        write(&root, "dev/cpuset/top-app/tasks", "");
        write(
            &root,
            "sys/fs/cgroup/uid_10123/pid_8123/cgroup.freeze",
            "0\n",
        );

        let module = ProcessModule::new(Sysfs::new(&root));
        let caps = module.probe();
        assert!(caps.supported);
        for feature in [
            "list",
            "foreground",
            "stop",
            "renice",
            "oom_score_adj",
            "sched",
            "cpuset",
            "freeze",
        ] {
            assert!(
                caps.features.iter().any(|f| f == feature),
                "missing feature {feature}: {:?}",
                caps.features
            );
        }

        // The process module must stay out of snapshot and have no tunables.
        assert!(!module.in_snapshot());
        assert!(module.schema().items.is_empty());
        assert_eq!(module.collect(None).unwrap(), json!({}));
        assert!(module.apply(&Map::new()).is_err());
    }
}
```

---

#### 3. `core/sysflow-cli/src/process_actions.rs`

```rust
//! `sysflow process stop|oom|renice|sched|cpuset|freeze|unfreeze` — one-shot actions.
//!
//! The CLI only parses arguments; resolution, protection, identity re-checks and read-back all live
//! in `sysflow_modules::process::actions`, where they are covered by host tests.

use serde_json::Value;

use sysflow_core::error::{Error, Result};
use sysflow_core::registry::Registry;

use sysflow_modules::process::actions::{self, Target};

pub fn stop(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let (target, user) = target_and_user(rest, "stop")?;
    actions::stop(&reg.ctx().sysfs, &reg.ctx().data_dir, &target, user)
}

pub fn oom(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let args = positionals(rest);
    let target = parse_target(args.first(), "oom")?;
    let adjustment = parse_i64(args.get(1), "oom", "adjustment")?;
    actions::oom(&reg.ctx().sysfs, &reg.ctx().data_dir, &target, adjustment)
}

pub fn renice(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let args = positionals(rest);
    let target = parse_target(args.first(), "renice")?;
    let nice = parse_i64(args.get(1), "renice", "nice")?;
    actions::renice(&reg.ctx().sysfs, &reg.ctx().data_dir, &target, nice)
}

pub fn sched(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let args = positionals(rest);
    let target = parse_target(args.first(), "sched")?;
    let policy = args.get(1).ok_or_else(|| {
        Error::invalid_arg("process sched requires a policy (other|fifo|rr|batch|idle)")
    })?;
    let priority = args
        .get(2)
        .map(|value| parse_i64(Some(value), "sched", "priority"))
        .transpose()?;
    actions::sched(
        &reg.ctx().sysfs,
        &reg.ctx().data_dir,
        &target,
        policy,
        priority,
    )
}

pub fn cpuset(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let args = positionals(rest);
    let target = parse_target(args.first(), "cpuset")?;
    let group = args
        .get(1)
        .ok_or_else(|| Error::invalid_arg("process cpuset requires a group name"))?;
    actions::cpuset(&reg.ctx().sysfs, &reg.ctx().data_dir, &target, group)
}

pub fn freeze(reg: &Registry, rest: &[&str], frozen: bool) -> Result<Value> {
    let args = positionals(rest);
    let target = parse_target(args.first(), if frozen { "freeze" } else { "unfreeze" })?;
    actions::freeze(&reg.ctx().sysfs, &reg.ctx().data_dir, &target, frozen)
}

fn target_and_user(rest: &[&str], verb: &str) -> Result<(Target, Option<u32>)> {
    let args = positionals(rest);
    let target = parse_target(args.first(), verb)?;
    let user =
        match flag(rest, "--user") {
            Some(value) => Some(value.parse::<u32>().map_err(|_| {
                Error::invalid_arg(format!("--user expects a number, got {value:?}"))
            })?),
            None => None,
        };
    Ok((target, user))
}

fn parse_target(value: Option<&&str>, verb: &str) -> Result<Target> {
    let value = value
        .ok_or_else(|| Error::invalid_arg(format!("process {verb} requires a PID or package")))?;
    actions::parse_target(value)
}

fn parse_i64(value: Option<&&str>, verb: &str, what: &str) -> Result<i64> {
    let value =
        value.ok_or_else(|| Error::invalid_arg(format!("process {verb} requires a {what}")))?;
    value
        .parse::<i64>()
        .map_err(|_| Error::invalid_arg(format!("{what} must be an integer, got {value:?}")))
}

/// Positional arguments, skipping `--flag value` pairs (only `--user` exists today).
fn positionals<'a>(rest: &[&'a str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for arg in rest {
        if skip_next {
            skip_next = false;
            continue;
        }
        if *arg == "--user" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--") {
            continue;
        }
        out.push(*arg);
    }
    out
}

fn flag<'a>(args: &[&'a str], name: &str) -> Option<&'a str> {
    let position = args.iter().position(|arg| *arg == name)?;
    args.get(position + 1).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positionals_skip_options_and_their_values() {
        let args = ["com.example.app", "--user", "10", "--force"];
        assert_eq!(positionals(&args), vec!["com.example.app"]);
        let args = ["10123", "-5"];
        assert_eq!(positionals(&args), vec!["10123", "-5"]);
    }

    #[test]
    fn user_flag_is_read_next_to_the_target() {
        let args = ["com.example.app", "--user", "10"];
        assert_eq!(flag(&args, "--user"), Some("10"));
        assert_eq!(flag(&["com.example.app"], "--user"), None);
    }

    #[test]
    fn sched_positionals_accept_policy_and_optional_priority() {
        assert_eq!(
            positionals(&["10123", "fifo", "10"]),
            vec!["10123", "fifo", "10"]
        );
        assert_eq!(
            positionals(&["com.example.app", "other"]),
            vec!["com.example.app", "other"]
        );
    }
}
```

---

#### 4. `core/sysflow-cli/src/main.rs`

```rust
use std::io::Write;
use std::process::exit;

mod battery_apps;
mod battery_health;
mod mem_apps;
mod mem_breakdown;
mod mem_probe;
mod process_actions;
mod process_get;
mod process_list;
mod process_probe;
mod storage_io;
mod storage_partitions;
mod storage_probe;
mod storage_trim;
mod storage_usage;
mod wakelock_apps;
mod wakelock_held;
mod wakelock_wakeup;

use clap::{Parser, Subcommand};
use serde_json::{json, Map, Value};

use sysflow_core::config::CONFIG_VERSION;
use sysflow_core::ctx::Ctx;
use sysflow_core::envelope::{self, PROTOCOL_VERSION};
use sysflow_core::event::{EventFilter, EventStore, NewSystemEvent};
use sysflow_core::registry::Registry;
use sysflow_core::{Error, Result, VERSION};

#[derive(Parser)]
#[command(
    name = "sysflow",
    version,
    about = "SysFlow device manager core (module CLI)"
)]
struct Cli {
    /// Pretty-print the JSON output
    #[arg(long, global = true)]
    pretty: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Binary, protocol and config versions
    Version,
    /// Per-module capability probe
    Caps,
    /// Current state of every supported module in one call
    Snapshot,
    /// Static SoC, board, kernel and Android build information.
    Info,
    /// Read-only drift check against persisted settings (used by the phase 2 daemon)
    Check,
    /// Recorded history for a module
    History {
        module: String,
        #[arg(long, default_value = "24h")]
        since: String,
        #[arg(long)]
        step: Option<String>,
    },
    /// Configure or inspect the boot-time apply behavior.
    Boot {
        #[command(subcommand)]
        action: BootAction,
    },
    /// Apply persisted settings at boot (called by service.sh)
    ApplyBoot {
        /// Skip the configured boot delay (tests)
        #[arg(long)]
        no_delay: bool,
    },
    /// Mark the current boot as good (called by service.sh after a delay)
    BootConfirm,
    /// Safe mode status / exit
    SafeMode {
        #[command(subcommand)]
        action: SafeAction,
    },
    /// Inspect or manage isolated recovery configuration backups.
    Recovery {
        #[command(subcommand)]
        action: RecoveryAction,
    },
    /// Background enforcement and adaptive sampling daemon.
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Apply persisted settings that have drifted and advance adaptive CPU control once.
    #[command(hide = true)]
    Enforce,
    /// Append a short note to the module log (legacy compatibility).
    Note { message: String },
    /// Emit and query durable, module-owned structured system events.
    Event {
        #[command(subcommand)]
        action: EventAction,
    },
    /// Tail the module log
    Log {
        #[arg(long, default_value_t = 200)]
        tail: usize,
    },
    /// `sysflow reset --all` restores every module's defaults
    Reset {
        #[arg(long)]
        all: bool,
    },
    /// `sysflow <module> schema | get [--keys a,b] | set '<json>' [--persist] | reset`
    /// `sysflow battery apps [--limit N] [--refresh]` ranks per-app power attribution.
    #[command(external_subcommand)]
    Module(Vec<String>),
}

#[derive(Subcommand)]
enum EventAction {
    /// Emit one structured event from any current or future module.
    Emit {
        #[arg(long)]
        module: String,
        #[arg(long)]
        category: String,
        #[arg(long = "type")]
        event_type: String,
        #[arg(long, default_value = "info")]
        severity: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        message: String,
        #[arg(long, default_value = "{}")]
        metadata: String,
        #[arg(long)]
        dedupe_key: Option<String>,
        #[arg(long, default_value_t = 30)]
        dedupe_window_s: u64,
    },
    /// Read persisted events; filters are open-ended and do not depend on a module registry.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        after_ms: Option<u64>,
        #[arg(long)]
        after_id: Option<String>,
        #[arg(long)]
        before_ms: Option<u64>,
        #[arg(long)]
        before_id: Option<String>,
        #[arg(long, default_value_t = false)]
        ascending: bool,
        #[arg(long)]
        module: Option<String>,
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        severity: Option<String>,
    },
    /// Apply event journal retention policy.
    Prune {
        #[arg(long, default_value_t = 30)]
        retention_days: u64,
        #[arg(long, default_value_t = 5000)]
        max_events: usize,
    },
    /// Delete all module-side event records (requires --all explicitly).
    Clear {
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
enum SafeAction {
    Status,
    Clear,
}

#[derive(Subcommand)]
enum RecoveryAction {
    Status,
    Restore,
    Clear,
    ClearCorrupt,
}

#[derive(Subcommand)]
enum BootAction {
    Status,
    Configure {
        #[arg(long)]
        enabled: Option<bool>,
        #[arg(long)]
        delay: Option<u32>,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the detached enforcement / adaptive-sampling daemon.
    Start,
    /// Stop the running daemon.
    Stop,
    /// Show daemon state.
    Status,
    /// Configure automatic enforcement and its sampling interval.
    Configure {
        /// Enable or disable the background service.
        #[arg(long)]
        enabled: Option<bool>,
        /// Sampling/enforcement interval in seconds (2..=120).
        #[arg(long)]
        interval: Option<u64>,
    },
    /// Internal foreground loop (normally launched by `daemon start` or service.sh).
    Run,
}

fn flag_value<'a>(args: &[&'a str], flag: &str) -> Option<&'a str> {
    let pos = args.iter().position(|a| *a == flag)?;
    args.get(pos + 1).copied()
}

fn run_module(reg: &Registry, raw: &[String]) -> Result<Value> {
    let args: Vec<&str> = raw
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--pretty")
        .collect();
    let module = *args
        .first()
        .ok_or_else(|| Error::invalid_arg("missing module name"))?;
    let verb = *args
        .get(1)
        .ok_or_else(|| Error::invalid_arg("missing verb (schema | get | set | reset)"))?;
    let rest = &args[2..];
    match verb {
        // The process module is verb-only: `get` must not fall through to the generic item reader,
        // which has no schema items to validate a PID against.
        "list" if module == "process" => process_list::run(reg, rest),
        "get" if module == "process" => process_get::run(reg, rest),
        "stop" if module == "process" => process_actions::stop(reg, rest),
        "oom" if module == "process" => process_actions::oom(reg, rest),
        "renice" if module == "process" => process_actions::renice(reg, rest),
        "sched" if module == "process" => process_actions::sched(reg, rest),
        "cpuset" if module == "process" => process_actions::cpuset(reg, rest),
        "freeze" if module == "process" => process_actions::freeze(reg, rest, true),
        "unfreeze" if module == "process" => process_actions::freeze(reg, rest, false),
        "schema" => reg.schema(module),
        "get" => {
            let keys: Option<Vec<String>> = flag_value(rest, "--keys").map(|s| {
                s.split(',')
                    .filter(|k| !k.is_empty())
                    .map(str::to_string)
                    .collect()
            });
            reg.get(module, keys.as_deref())
        }
        "set" => {
            let payload = *rest
                .iter()
                .find(|a| !a.starts_with("--"))
                .ok_or_else(|| Error::invalid_arg("set requires a JSON payload"))?;
            let value: Value = serde_json::from_str(payload)?;
            reg.set(module, value, rest.contains(&"--persist"))
        }
        "reset" => reg.reset(module),
        // Re-seed the stored defaults from the kernel's own hardware floor.
        //
        // Defaults are captured once and never refreshed, so a config recorded before a kernel or
        // ROM update keeps a minimum frequency the hardware no longer honours — and `reset` keeps
        // restoring it. This records `cpuinfo_min_freq` (what the driver itself reports) as the
        // default minimum for every cluster that has one, which makes 恢复默认 mean "no artificial
        // floor" instead of "whatever the first write happened to see".
        "rebaseline" if module == "cpu" => {
            let hardware = reg.get(module, Some(&["cpu.hw_min_freq".to_string()]))?;
            let Some(by_cluster) = hardware.get("cpu.hw_min_freq").and_then(Value::as_object)
            else {
                return Err(Error::io("cpu reports no per-cluster hardware minimum"));
            };
            let floored: Map<String, Value> = by_cluster
                .iter()
                .filter(|(_, value)| value.as_i64().is_some_and(|freq| freq > 0))
                .map(|(cluster, value)| (cluster.clone(), value.clone()))
                .collect();
            if floored.is_empty() {
                return Err(Error::io(
                    "cpuinfo_min_freq is unreadable, so no hardware floor could be recorded",
                ));
            }
            let mut values = Map::new();
            values.insert("cpu.min_freq".to_string(), Value::Object(floored));
            reg.set_defaults(module, values)
        }
        // BatteryStats attribution is battery-specific and expensive, so it is a verb rather than
        // part of `snapshot`; the battery page pulls it lazily.
        "held" if module == "wakelock" => wakelock_held::run(reg, rest.contains(&"--refresh")),
        // Same cost as `held` (a `dumpsys power`), so also a cached verb rather than a snapshot item.
        "wakeup" if module == "wakelock" => wakelock_wakeup::run(reg, rest.contains(&"--refresh")),
        // BatteryStats wake-lock time is battery-scale history, so it stays out of `snapshot` for
        // the same reason `battery apps` does.
        "apps" if module == "wakelock" => {
            let limit = flag_value(rest, "--limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(20);
            wakelock_apps::run(reg, limit, rest.contains(&"--refresh"))
        }
        // System-vs-user split: measures PSS for **every** process, which is why it is a verb and
        // not part of `snapshot`.
        "breakdown" if module == "mem" => mem_breakdown::run(reg, rest.contains(&"--refresh")),
        // Per-UID memory ranking: opens one /proc entry per process, so it is a verb, not a snapshot
        // item, and its result is cached for a minute.
        "apps" if module == "mem" => {
            let limit = flag_value(rest, "--limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(mem_apps::DEFAULT_LIMIT);
            mem_apps::run(reg, limit, rest.contains(&"--refresh"))
        }
        // Hidden developer diagnostic: raw readings + the module's judgement; not in the protocol.
        "probe" if module == "mem" => mem_probe::run(reg),
        // Hidden Step 1 diagnostic for the process module (device path matrix).
        "probe" if module == "process" => process_probe::run(reg),
        // Storage verbs: live I/O needs a sampling interval, usage is a bounded tree walk, trim is
        // a one-shot action, and the probe is the hidden Step 1 diagnostic.
        "io" if module == "storage" => storage_io::run(reg, rest),
        "partitions" if module == "storage" => storage_partitions::run(reg),
        "usage" if module == "storage" => storage_usage::run(reg, rest),
        "trim" if module == "storage" => storage_trim::run(reg, rest),
        "probe" if module == "storage" => storage_probe::run(reg),
        "health" if module == "battery" => battery_health::run(reg, rest.contains(&"--refresh")),
        "apps" if module == "battery" => {
            let limit = flag_value(rest, "--limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            battery_apps::run(reg, limit, rest.contains(&"--refresh"))
        }
        other => Err(Error::invalid_arg(format!("unknown verb: {other}"))),
    }
}

fn run(cli: &Cli, reg: &Registry, events: &EventStore) -> Result<Value> {
    match &cli.cmd {
        Cmd::Version => Ok(json!({
            "sysflow_version": VERSION,
            "protocol": PROTOCOL_VERSION,
            "config_version": CONFIG_VERSION,
        })),
        Cmd::Caps => reg.caps(),
        Cmd::Snapshot => reg.snapshot(),
        Cmd::Info => Ok(reg.info()),
        Cmd::Check => reg.check(),
        Cmd::History {
            module,
            since,
            step,
        } => reg.history(module, since, step.as_deref()),
        Cmd::Boot { action } => match action {
            BootAction::Status => reg.boot_status(),
            BootAction::Configure { enabled, delay } => reg.boot_configure(*enabled, *delay),
        },
        Cmd::ApplyBoot { no_delay } => reg.apply_boot(*no_delay),
        Cmd::BootConfirm => reg.boot_confirm(),
        Cmd::SafeMode { action } => match action {
            SafeAction::Status => reg.safe_status(),
            SafeAction::Clear => reg.safe_clear(),
        },
        Cmd::Recovery { action } => match action {
            RecoveryAction::Status => reg.recovery_status(),
            RecoveryAction::Restore => reg.recovery_restore(),
            RecoveryAction::Clear => reg.recovery_clear(),
            RecoveryAction::ClearCorrupt => reg.recovery_clear_corrupt(),
        },
        Cmd::Daemon { action } => match action {
            DaemonAction::Status => reg.daemon_status(),
            DaemonAction::Start => reg.daemon_start(),
            DaemonAction::Stop => reg.daemon_stop(),
            DaemonAction::Configure { enabled, interval } => {
                reg.daemon_configure(*enabled, *interval)
            }
            DaemonAction::Run => reg.daemon_run(),
        },
        Cmd::Enforce => reg.enforce(),
        Cmd::Note { message } => reg.note(message),
        Cmd::Event { action } => match action {
            EventAction::Emit {
                module,
                category,
                event_type,
                severity,
                title,
                message,
                metadata,
                dedupe_key,
                dedupe_window_s,
            } => {
                let metadata: Value = serde_json::from_str(metadata)
                    .map_err(|e| Error::invalid_arg(format!("invalid metadata JSON: {e}")))?;
                let (event, deduplicated) = events.emit(NewSystemEvent {
                    module: module.clone(),
                    category: category.clone(),
                    event_type: event_type.clone(),
                    severity: severity.clone(),
                    title: title.clone(),
                    message: message.clone(),
                    metadata,
                    dedupe_key: dedupe_key.clone(),
                    dedupe_window_s: *dedupe_window_s,
                })?;
                Ok(json!({ "event": event, "deduplicated": deduplicated }))
            }
            EventAction::List {
                limit,
                after_ms,
                after_id,
                before_ms,
                before_id,
                ascending,
                module,
                category,
                severity,
            } => {
                let records = events.list(EventFilter {
                    limit: *limit,
                    after_ms: *after_ms,
                    after_id: after_id.as_deref(),
                    before_ms: *before_ms,
                    before_id: before_id.as_deref(),
                    ascending: *ascending,
                    module: module.as_deref(),
                    category: category.as_deref(),
                    severity: severity.as_deref(),
                })?;
                let next_cursor_ms = records.last().map(|event| event.timestamp_ms);
                let next_cursor_id = records.last().map(|event| event.id.clone());
                Ok(json!({
                    "events": records,
                    "next_cursor_ms": next_cursor_ms,
                    "next_cursor_id": next_cursor_id,
                    "directory": events.directory().display().to_string(),
                }))
            }
            EventAction::Prune {
                retention_days,
                max_events,
            } => events.prune(*retention_days, *max_events),
            EventAction::Clear { all } => {
                if !all {
                    return Err(Error::invalid_arg("event clear requires --all"));
                }
                events.clear()
            }
        },
        Cmd::Log { tail } => reg.log_tail(*tail),
        Cmd::Reset { all } => {
            if *all {
                reg.reset_all()
            } else {
                Err(Error::invalid_arg(
                    "use `sysflow reset --all` or `sysflow <module> reset`",
                ))
            }
        }
        Cmd::Module(args) => run_module(reg, args),
    }
}

fn emit(value: &Value, pretty: bool) {
    let text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .unwrap_or_else(|_| String::from("{}"));
    // ignore broken pipes instead of panicking
    let _ = writeln!(std::io::stdout().lock(), "{text}");
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            use clap::error::ErrorKind;
            match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    let _ = e.print();
                    exit(0);
                }
                _ => {
                    let msg = e.to_string();
                    emit(&envelope::err(&Error::invalid_arg(msg.trim())), false);
                    exit(2);
                }
            }
        }
    };

    let pretty =
        cli.pretty || matches!(&cli.cmd, Cmd::Module(a) if a.iter().any(|s| s == "--pretty"));

    let ctx = Ctx::from_env();
    let modules = sysflow_modules::builtin(&ctx);
    let reg = Registry::new(ctx.clone(), modules);
    let events = EventStore::new(&ctx.data_dir);

    match run(&cli, &reg, &events) {
        Ok(data) => emit(&envelope::ok(data), pretty),
        Err(e) => {
            reg.log_error(&e);
            emit(&envelope::err(&e), pretty);
            exit(1);
        }
    }
}
```

---

回复 **“继续”** 我立即发 **第 2 批**（`core/sysflow-modules/src/process/scan.rs` 完整文件）。

---

## You

继续

---

## AI Studio

### 第 2 批（1 个大文件）：`core/sysflow-modules/src/process/scan.rs`

#### `core/sysflow-modules/src/process/scan.rs`

```rust
//! Process enumeration and the `list` / `get` payloads.
//!
//! The list is a **two-pass** scan: the first pass reads `/proc/<pid>/{stat,status,cmdline,
//! oom_score_adj,cgroup}` for every process, the second pass re-reads only `stat` after the sampling
//! window to derive CPU usage. On the reference device (S25, 1198 processes) the first pass costs
//! ~28 ms, so a 500 ms window dominates the latency budget, not the parsing.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use sysflow_core::cache;
use sysflow_core::error::{Error, Result};
use sysflow_core::sysfs::Sysfs;

use super::{foreground, model, packages};

pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 200;
pub const DEFAULT_SAMPLE_MS: u64 = 500;
pub const MIN_SAMPLE_MS: u64 = 200;
pub const MAX_SAMPLE_MS: u64 = 2_000;
/// `get` is a single-process call, so it samples briefly instead of taking the full list window.
const DETAIL_SAMPLE_MS: u64 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Cpu,
    Rss,
    Name,
    Pid,
}

impl SortKey {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "cpu" => Some(Self::Cpu),
            "rss" => Some(Self::Rss),
            "name" => Some(Self::Name),
            "pid" => Some(Self::Pid),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    User,
    System,
}

impl Filter {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "all" => Some(Self::All),
            "user" => Some(Self::User),
            "system" => Some(Self::System),
            _ => None,
        }
    }

    fn keeps(self, uid: u32) -> bool {
        match self {
            Filter::All => true,
            Filter::User => uid >= 10_000,
            Filter::System => uid < 10_000,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ListOptions {
    pub limit: usize,
    pub sort: SortKey,
    pub filter: Filter,
    pub query: Option<String>,
    pub sample_ms: u64,
    pub with_foreground: bool,
}

impl Default for ListOptions {
    fn default() -> Self {
        ListOptions {
            limit: DEFAULT_LIMIT,
            sort: SortKey::Cpu,
            filter: Filter::All,
            query: None,
            sample_ms: DEFAULT_SAMPLE_MS,
            with_foreground: true,
        }
    }
}

/// One process as read from `/proc` in the first pass.
#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub stat: model::Stat,
    pub uid: u32,
    pub rss_kb: Option<u64>,
    pub comm: String,
    pub cmdline: String,
    pub name: String,
    pub kthread: bool,
    pub oom_score_adj: Option<i64>,
    pub cgroup: Option<model::Cgroup>,
    pub frozen: Option<bool>,
}

pub(crate) fn read_stat(fs: &Sysfs, pid: u32) -> Option<model::Stat> {
    model::parse_stat(&fs.read_lossy(&format!("/proc/{pid}/stat")).ok()?)
}

pub(crate) fn read_one(fs: &Sysfs, pid: u32) -> Option<ProcessInfo> {
    let stat = read_stat(fs, pid)?;
    let status = model::parse_status(&fs.read_lossy(&format!("/proc/{pid}/status")).ok()?)?;
    let cmdline = model::parse_cmdline(
        &fs.read_lossy(&format!("/proc/{pid}/cmdline"))
            .unwrap_or_default(),
    );
    let name = display_name(&cmdline, &status.comm, pid);
    let oom_score_adj = fs
        .read_lossy(&format!("/proc/{pid}/oom_score_adj"))
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok());
    let cgroup = fs
        .read_lossy(&format!("/proc/{pid}/cgroup"))
        .ok()
        .map(|text| model::parse_cgroup(&text));
    let frozen = read_frozen(fs, cgroup.as_ref());
    Some(ProcessInfo {
        pid,
        stat,
        uid: status.uid,
        rss_kb: status.rss_kb,
        comm: status.comm,
        cmdline,
        name,
        kthread: status.kthread,
        oom_score_adj,
        cgroup,
        frozen,
    })
}

/// Whether the process is currently frozen (`cgroup.freeze` / `cgroup.events` on cgroup v2, or
/// `/dev/freezer/<path>/freezer.state` on cgroup v1). Returns `None` when no freezer interface
/// is exposed for this process.
pub(crate) fn read_frozen(fs: &Sysfs, cgroup: Option<&model::Cgroup>) -> Option<bool> {
    let cgroup = cgroup?;
    if let Some(unified) = cgroup.unified.as_deref() {
        let trimmed = unified.trim_end_matches('/');
        if !trimmed.is_empty() {
            let base = format!("/sys/fs/cgroup{trimmed}");
            let freeze_file = format!("{base}/cgroup.freeze");
            let events_file = format!("{base}/cgroup.events");
            if let Ok(raw) = fs.read_str(&freeze_file) {
                if raw.trim() == "1" {
                    return Some(true);
                }
                if let Ok(events) = fs.read_str(&events_file) {
                    for line in events.lines() {
                        if let Some(val) = line
                            .strip_prefix("frozen")
                            .filter(|rest| rest.starts_with(char::is_whitespace))
                        {
                            return Some(val.trim() == "1");
                        }
                    }
                }
                return Some(false);
            }
            if let Ok(events) = fs.read_str(&events_file) {
                for line in events.lines() {
                    if let Some(val) = line
                        .strip_prefix("frozen")
                        .filter(|rest| rest.starts_with(char::is_whitespace))
                    {
                        return Some(val.trim() == "1");
                    }
                }
            }
        }
    }
    if let Some(v1_path) = cgroup.freezer.as_deref() {
        let trimmed = v1_path.trim_end_matches('/');
        let state_file = if trimmed.is_empty() {
            "/dev/freezer/freezer.state".to_string()
        } else {
            format!("/dev/freezer{trimmed}/freezer.state")
        };
        if let Ok(raw) = fs.read_str(&state_file) {
            let upper = raw.trim().to_ascii_uppercase();
            if upper == "FROZEN" || upper == "FREEZING" {
                return Some(true);
            }
            if upper == "THAWED" {
                return Some(false);
            }
        }
    }
    None
}

/// The name a UI should show: the package for app processes, the basename for daemon paths.
///
/// `/proc/<pid>/cmdline` is the package name for zygote children, an absolute path for native
/// daemons and empty for kernel threads. Showing `/vendor/bin/hw/...composer-service` in a row is
/// noise, so only the last path component is used there; the full cmdline stays in its own field.
fn display_name(cmdline: &str, comm: &str, pid: u32) -> String {
    if cmdline.is_empty() {
        return if comm.is_empty() {
            format!("pid {pid}")
        } else {
            comm.to_string()
        };
    }
    if cmdline.starts_with('/') {
        if let Some(name) = std::path::Path::new(cmdline).file_name() {
            return name.to_string_lossy().into_owned();
        }
    }
    cmdline.to_string()
}

/// Whether a cmdline token can be the process's own package name.
///
/// App processes carry the package in `cmdline[0]`; remote processes append `:service`, sandboxed
/// processes append `:sandboxed_process…`, and native daemons carry an absolute path. Only the
/// plain dotted form is treated as a package.
fn looks_like_package(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 255
        && text.contains('.')
        && !text.contains([':', '/', '@', ' '])
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_'))
}

/// The package this process belongs to, or `None` when that question has no honest answer.
///
/// The UID→package map alone is **not** enough: platform UIDs are shared by many services
/// (`uid 1000` owns `system_server`, `surfaceflinger`, …), and a shared app UID can own several
/// packages. The process's own cmdline is authoritative when it is a package; otherwise the UID map
/// is only consulted for app UIDs (>= 10 000). A real S25 run showed `system_server` labelled
/// `com.samsung.android.provider.filterprovider` before this rule.
pub fn package_of(info: &ProcessInfo, packages: &packages::Packages) -> Option<String> {
    if looks_like_package(&info.cmdline) {
        return Some(info.cmdline.clone());
    }
    if info.uid < 10_000 {
        return None;
    }
    packages.primary(info.uid).map(str::to_string)
}

/// Existing `/dev/cpuset` groups (directories with a `tasks` node), for the UI's group picker.
pub fn cpuset_groups(fs: &Sysfs) -> Vec<String> {
    let Ok(entries) = fs.list_dir("/dev/cpuset") else {
        return Vec::new();
    };
    let mut groups: Vec<String> = entries
        .into_iter()
        .filter(|group| {
            let dir = format!("/dev/cpuset/{group}");
            std::fs::metadata(fs.resolve(&dir))
                .map(|meta| meta.is_dir())
                .unwrap_or(false)
                && fs.exists(&format!("{dir}/tasks"))
        })
        .collect();
    groups.sort();
    groups
}

/// Every numeric `/proc` entry, plus how many had to be skipped (exited mid-scan, unreadable).
pub(crate) fn read_all(fs: &Sysfs) -> Result<(Vec<ProcessInfo>, u32)> {
    let entries = fs.list_dir("/proc")?;
    let mut out = Vec::new();
    let mut skipped = 0u32;
    for entry in entries {
        let Ok(pid) = entry.parse::<u32>() else {
            continue;
        };
        match read_one(fs, pid) {
            Some(info) => out.push(info),
            None => skipped += 1,
        }
    }
    Ok((out, skipped))
}

/// Per-process CPU result: percentage of one core and the raw tick delta.
type CpuSample = (Option<f64>, Option<u64>);

fn sample_cpu(
    fs: &Sysfs,
    infos: &[ProcessInfo],
    clock_ticks: u64,
    sample_ms: u64,
) -> HashMap<u32, CpuSample> {
    let mut out = HashMap::new();
    if sample_ms == 0 || infos.is_empty() {
        return out;
    }
    let started = Instant::now();
    std::thread::sleep(Duration::from_millis(sample_ms));
    for info in infos {
        let Some(second) = read_stat(fs, info.pid) else {
            continue;
        };
        // A different start time means the PID was recycled: the delta would belong to another
        // process, so report "unknown" instead of a fabricated percentage.
        if second.starttime != info.stat.starttime {
            continue;
        }
        let delta = second.ticks().saturating_sub(info.stat.ticks());
        let elapsed = started.elapsed().as_secs_f64().max(0.001);
        let pct = (delta as f64) * 100.0 / (elapsed * clock_ticks as f64);
        out.insert(info.pid, (Some(round1(pct)), Some(delta)));
    }
    out
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

struct Row {
    info: ProcessInfo,
    cpu_pct: Option<f64>,
    cpu_ticks: Option<u64>,
    package: Option<String>,
    system: bool,
    protected: Option<&'static str>,
    foreground: bool,
    sort_name: String,
}

fn build_rows(
    infos: Vec<ProcessInfo>,
    cpu: &HashMap<u32, CpuSample>,
    packages: &packages::Packages,
    foreground_package: Option<&str>,
    with_foreground: bool,
) -> Vec<Row> {
    infos
        .into_iter()
        .map(|info| {
            let package = package_of(&info, packages);
            let system = info.uid < 10_000;
            let protected = model::protected_reason(
                info.uid,
                info.pid,
                info.kthread,
                &info.name,
                package.as_deref(),
            );
            let foreground = with_foreground
                && foreground_package.is_some_and(|pkg| {
                    info.name == pkg || info.cmdline == pkg || package.as_deref() == Some(pkg)
                });
            let (cpu_pct, cpu_ticks) = cpu.get(&info.pid).copied().unwrap_or((None, None));
            let sort_name = info.name.to_lowercase();
            Row {
                info,
                cpu_pct,
                cpu_ticks,
                package,
                system,
                protected,
                foreground,
                sort_name,
            }
        })
        .collect()
}

fn cpu_order(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => y.partial_cmp(&x).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn sort_rows(rows: &mut [Row], sort: SortKey) {
    rows.sort_by(|a, b| match sort {
        SortKey::Cpu => cpu_order(a.cpu_pct, b.cpu_pct).then_with(|| a.info.pid.cmp(&b.info.pid)),
        SortKey::Rss => b
            .info
            .rss_kb
            .unwrap_or(0)
            .cmp(&a.info.rss_kb.unwrap_or(0))
            .then_with(|| a.info.pid.cmp(&b.info.pid)),
        SortKey::Name => a
            .sort_name
            .cmp(&b.sort_name)
            .then_with(|| a.info.pid.cmp(&b.info.pid)),
        SortKey::Pid => a.info.pid.cmp(&b.info.pid),
    });
}

fn row_json(row: &Row) -> Value {
    let info = &row.info;
    let mut object = Map::new();
    object.insert("pid".into(), json!(info.pid));
    object.insert("ppid".into(), json!(info.stat.ppid));
    object.insert("uid".into(), json!(info.uid));
    object.insert("user".into(), json!(info.uid / 100_000));
    object.insert("app_id".into(), json!(info.uid % 100_000));
    object.insert(
        "package".into(),
        row.package
            .as_ref()
            .map_or(Value::Null, |package| json!(package)),
    );
    object.insert("name".into(), json!(info.name));
    object.insert("cmdline".into(), json!(info.cmdline));
    object.insert("state".into(), json!(info.stat.state.to_string()));
    object.insert("threads".into(), json!(info.stat.threads));
    if let Some(rss_kb) = info.rss_kb {
        object.insert("rss_kb".into(), json!(rss_kb));
    }
    object.insert(
        "cpu_pct".into(),
        row.cpu_pct.map_or(Value::Null, |pct| json!(pct)),
    );
    object.insert(
        "cpu_ticks".into(),
        row.cpu_ticks.map_or(Value::Null, |ticks| json!(ticks)),
    );
    if let Some(oom) = info.oom_score_adj {
        object.insert("oom_score_adj".into(), json!(oom));
    }
    if let Some(cgroup) = &info.cgroup {
        if let Some(path) = &cgroup.unified {
            object.insert("cgroup".into(), json!(path));
        }
        if let Some(group) = cgroup.cpuset_group() {
            object.insert("cpuset".into(), json!(group));
        }
    }
    if let Some(frozen) = info.frozen {
        object.insert("frozen".into(), json!(frozen));
    }
    object.insert("foreground".into(), json!(row.foreground));
    object.insert("system".into(), json!(row.system));
    object.insert("protected".into(), json!(row.protected.is_some()));
    if let Some(reason) = row.protected {
        object.insert("protected_reason".into(), json!(reason));
    }
    Value::Object(object)
}

/// `sysflow process list` payload. Never fails for a readable `/proc`; an unreadable one is reported
/// as `available:false` so the client can show a reason instead of an empty list.
pub fn list(fs: &Sysfs, data_dir: &Path, opts: &ListOptions) -> Value {
    let (infos, skipped) = match read_all(fs) {
        Ok(pair) => pair,
        Err(e) => {
            return json!({
                "available": false,
                "reason": e.to_string(),
                "generated_at": cache::now_secs(),
                "processes": [],
                "skipped": 0,
            });
        }
    };
    let total = infos.len();
    let clock_ticks = model::clock_ticks(fs);
    let cpuset_groups = cpuset_groups(fs);
    let cpu = sample_cpu(fs, &infos, clock_ticks, opts.sample_ms);
    let packages = packages::read(fs, data_dir);
    let fg = if opts.with_foreground {
        foreground::current(data_dir)
    } else {
        foreground::Foreground::default()
    };
    let foreground_package = fg.package.clone();
    let foreground_pid = foreground_package.as_deref().and_then(|pkg| {
        infos
            .iter()
            .filter(|info| {
                info.name == pkg || info.cmdline == pkg || packages.primary(info.uid) == Some(pkg)
            })
            .map(|info| info.pid)
            .min()
    });

    let mut rows = build_rows(
        infos,
        &cpu,
        &packages,
        foreground_package.as_deref(),
        opts.with_foreground,
    );

    rows.retain(|row| opts.filter.keeps(row.info.uid));
    if let Some(query) = opts.query.as_deref().map(str::to_lowercase) {
        rows.retain(|row| {
            row.sort_name.contains(&query)
                || row.info.pid.to_string().contains(&query)
                || row
                    .package
                    .as_deref()
                    .is_some_and(|package| package.to_lowercase().contains(&query))
        });
    }

    let matching = rows.len();
    sort_rows(&mut rows, opts.sort);
    let limit = opts.limit.clamp(1, MAX_LIMIT);
    let processes: Vec<Value> = rows.iter().take(limit).map(row_json).collect();

    json!({
        "available": true,
        "generated_at": cache::now_secs(),
        "sample_ms": opts.sample_ms,
        "clock_ticks": clock_ticks,
        "total": total,
        "returned": processes.len(),
        "truncated": matching > processes.len(),
        "skipped": skipped,
        "cpuset_groups": cpuset_groups,
        "foreground": {
            "available": fg.available,
            "source": if opts.with_foreground { "dumpsys" } else { "none" },
            "package": foreground_package,
            "user": fg.user,
            "pid": foreground_pid,
            "cached": fg.cached,
            "disabled": !opts.with_foreground,
        },
        "processes": processes,
    })
}

pub fn detail(fs: &Sysfs, data_dir: &Path, pid: u32) -> Result<Value> {
    detail_with_sample(fs, data_dir, pid, DETAIL_SAMPLE_MS)
}

fn detail_with_sample(fs: &Sysfs, data_dir: &Path, pid: u32, sample_ms: u64) -> Result<Value> {
    let info = read_one(fs, pid).ok_or_else(|| {
        Error::invalid_arg(format!("process {pid} does not exist"))
            .with_detail(json!({"gone": true, "pid": pid}))
    })?;
    let installed = packages::read(fs, data_dir);
    let package = package_of(&info, &installed);
    let package_names: Vec<String> = installed
        .names(info.uid)
        .map(<[String]>::to_vec)
        .unwrap_or_default();
    let clock_ticks = model::clock_ticks(fs);
    let cpu = sample_cpu(fs, std::slice::from_ref(&info), clock_ticks, sample_ms);
    let (cpu_pct, cpu_ticks) = cpu.get(&pid).copied().unwrap_or((None, None));
    let fg = foreground::current(data_dir);
    let foreground = fg.package.as_deref().is_some_and(|pkg| {
        info.name == pkg || info.cmdline == pkg || package.as_deref() == Some(pkg)
    });
    let system = info.uid < 10_000;
    let protected = model::protected_reason(
        info.uid,
        info.pid,
        info.kthread,
        &info.name,
        package.as_deref(),
    );
    let cmdline: Vec<String> = fs
        .read_lossy(&format!("/proc/{pid}/cmdline"))
        .unwrap_or_default()
        .split('\0')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    let (pss_kb, swap_pss_kb) = fs
        .read_lossy(&format!("/proc/{pid}/smaps_rollup"))
        .ok()
        .and_then(|text| parse_smaps_rollup(&text))
        .map_or((None, None), |(pss, swap)| (Some(pss), Some(swap)));

    let mut object = Map::new();
    object.insert("available".into(), json!(true));
    object.insert("pid".into(), json!(info.pid));
    object.insert("ppid".into(), json!(info.stat.ppid));
    object.insert("uid".into(), json!(info.uid));
    object.insert("user".into(), json!(info.uid / 100_000));
    object.insert("app_id".into(), json!(info.uid % 100_000));
    object.insert(
        "package".into(),
        package.map_or(Value::Null, |package| json!(package)),
    );
    object.insert("packages".into(), json!(package_names));
    object.insert("name".into(), json!(info.name));
    object.insert("cmdline".into(), json!(cmdline));
    object.insert("state".into(), json!(info.stat.state.to_string()));
    object.insert("threads".into(), json!(info.stat.threads));
    if let Some(rss_kb) = info.rss_kb {
        object.insert("rss_kb".into(), json!(rss_kb));
    }
    if let Some(pss_kb) = pss_kb {
        object.insert("pss_kb".into(), json!(pss_kb));
    }
    if let Some(swap_pss_kb) = swap_pss_kb {
        object.insert("swap_pss_kb".into(), json!(swap_pss_kb));
    }
    object.insert(
        "cpu_pct".into(),
        cpu_pct.map_or(Value::Null, |pct| json!(pct)),
    );
    object.insert(
        "cpu_ticks".into(),
        cpu_ticks.map_or(Value::Null, |ticks| json!(ticks)),
    );
    object.insert("sample_ms".into(), json!(sample_ms));
    object.insert("clock_ticks".into(), json!(clock_ticks));
    if let Some(oom) = info.oom_score_adj {
        object.insert("oom_score_adj".into(), json!(oom));
    }
    object.insert("nice".into(), json!(info.stat.nice));
    object.insert("rt_priority".into(), json!(info.stat.rt_priority));
    object.insert("policy".into(), json!(info.stat.policy));
    object.insert(
        "policy_name".into(),
        json!(model::sched_policy_name(info.stat.policy)),
    );
    object.insert("start_time_ticks".into(), json!(info.stat.starttime));
    if let Some(cgroup) = &info.cgroup {
        object.insert(
            "cgroup".into(),
            cgroup
                .unified
                .clone()
                .map_or(Value::Null, |path| json!(path)),
        );
        object.insert(
            "cpuset".into(),
            cgroup
                .cpuset_group()
                .map_or(Value::Null, |group| json!(group)),
        );
    }
    if let Some(frozen) = info.frozen {
        object.insert("frozen".into(), json!(frozen));
    }
    object.insert("foreground".into(), json!(foreground));
    object.insert("system".into(), json!(system));
    object.insert("protected".into(), json!(protected.is_some()));
    if let Some(reason) = protected {
        object.insert("protected_reason".into(), json!(reason));
    }
    Ok(Value::Object(object))
}

/// `Pss` / `SwapPss` from one `smaps_rollup`; `None` when the rollup has no `Pss:` line.
fn parse_smaps_rollup(text: &str) -> Option<(u64, u64)> {
    let mut pss = None;
    let mut swap_pss = 0u64;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Pss:") {
            pss = rest.split_whitespace().next().and_then(|v| v.parse().ok());
        } else if let Some(rest) = line.strip_prefix("SwapPss:") {
            swap_pss = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
    }
    pss.map(|pss| (pss, swap_pss))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sysflow-process-scan-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn stat_line(pid: u32, comm: &str, uid_ticks: (u64, u64)) -> String {
        // 39+ tail fields so rt_priority/policy are present.
        format!(
            "{pid} ({comm}) S 1 {pid} 0 0 -1 4194560 100 0 0 0 {} {} 0 0 20 0 1 0 2588536 10000000 500 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
            uid_ticks.0, uid_ticks.1
        )
    }

    fn add_process(root: &Path, pid: u32, comm: &str, uid: u32, rss_kb: u64) {
        let stat = stat_line(pid, comm, (1, 3));
        write(root, &format!("proc/{pid}/stat"), &stat);
        write(
            root,
            &format!("proc/{pid}/status"),
            &format!("Name:\t{comm}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nVmRSS:\t{rss_kb} kB\n"),
        );
    }

    #[test]
    fn list_reads_processes_and_sorts_by_pid() {
        let root = temp("list-pid");
        add_process(&root, 1, "init", 0, 1000);
        add_process(&root, 10123, "com.example.app", 10123, 20_000);
        write(
            &root,
            "data/system/packages.list",
            "com.example.app 10123 0 /data 0 0 1 0 0 0\n",
        );
        write(&root, "dev/cpuset/top-app/tasks", "");
        write(&root, "dev/cpuset/background/tasks", "");
        write(&root, "dev/cpuset/cpus", "0-7\n");

        let fs = Sysfs::new(&root);
        let data = root.join("state-dir");
        let opts = ListOptions {
            sort: SortKey::Pid,
            sample_ms: 0,
            with_foreground: false,
            ..ListOptions::default()
        };
        let out = list(&fs, &data, &opts);
        assert_eq!(out["available"], true);
        assert_eq!(out["total"], 2);
        assert_eq!(out["returned"], 2);
        assert_eq!(out["skipped"], 0);
        let rows = out["processes"].as_array().unwrap();
        assert_eq!(rows[0]["pid"], 1);
        assert_eq!(rows[0]["name"], "init");
        assert_eq!(rows[0]["system"], true);
        assert_eq!(rows[0]["protected"], true);
        assert_eq!(rows[1]["pid"], 10123);
        assert_eq!(rows[1]["package"], "com.example.app");
        assert_eq!(rows[1]["user"], 0);
        assert_eq!(rows[1]["app_id"], 10123);
        assert_eq!(rows[1]["rss_kb"], 20_000);
        assert_eq!(rows[1]["system"], false);
        assert_eq!(rows[1]["protected"], false);
        assert_eq!(rows[1]["cpu_pct"], Value::Null);
        assert_eq!(out["foreground"]["disabled"], true);
        assert_eq!(out["cpuset_groups"], json!(["background", "top-app"]));
    }

    #[test]
    fn package_and_name_rules_match_real_device_shapes() {
        let root = temp("package-rules");
        // Platform UID shared by system_server, surfaceflinger, …: no package can be named.
        write(
            &root,
            "proc/3913/stat",
            &stat_line(3913, "system_server", (1, 3)),
        );
        write(
            &root,
            "proc/3913/status",
            "Name:\tsystem_server\nUid:\t1000\t1000\t1000\t1000\nVmRSS:\t500000 kB\n",
        );
        write(&root, "proc/3913/cmdline", "system_server\u{0}");
        // Shared app UID: the process cmdline (systemui) wins over the first package in the map.
        write(
            &root,
            "proc/5354/stat",
            &stat_line(5354, "com.android.systemui", (1, 3)),
        );
        write(
            &root,
            "proc/5354/status",
            "Name:\tcom.android.systemui\nUid:\t10056\t10056\t10056\t10056\nVmRSS:\t400000 kB\n",
        );
        write(&root, "proc/5354/cmdline", "com.android.systemui\u{0}");
        // Remote process: the suffix is not a package, the UID map names the base package.
        write(
            &root,
            "proc/16023/stat",
            &stat_line(16023, "com.tencent.mm:push", (1, 3)),
        );
        write(
            &root,
            "proc/16023/status",
            "Name:\tcom.tencent.mm:push\nUid:\t10367\t10367\t10367\t10367\nVmRSS:\t200000 kB\n",
        );
        write(&root, "proc/16023/cmdline", "com.tencent.mm:push\u{0}");
        // Native daemon: the UI shows the basename, the full path stays in `cmdline`.
        write(&root, "proc/2341/stat", &stat_line(2341, "netd", (1, 3)));
        write(
            &root,
            "proc/2341/status",
            "Name:\tnetd\nUid:\t0\t0\t0\t0\nVmRSS:\t10000 kB\n",
        );
        write(&root, "proc/2341/cmdline", "/system/bin/netd\u{0}");
        // Isolated sandbox UID: no package at all.
        write(&root, "proc/7787/stat", &stat_line(7787, "sandbox", (1, 3)));
        write(
            &root,
            "proc/7787/status",
            "Name:\tsandbox\nUid:\t90002\t90002\t90002\t90002\nVmRSS:\t1000 kB\n",
        );
        write(
            &root,
            "proc/7787/cmdline",
            "net.quetta.browser:sandboxed_process0:org.chromium.app\u{0}",
        );
        write(
            &root,
            "data/system/packages.list",
            "com.samsung.filter 1000 0 /data 0 0 1 0 0 0\ncom.samsung.aod 10056 0 /data 0 0 1 0 0 0\ncom.tencent.mm 10367 0 /data 0 0 1 0 0 0\n",
        );

        let fs = Sysfs::new(&root);
        let out = list(
            &fs,
            &root.join("state-dir"),
            &ListOptions {
                limit: 200,
                sort: SortKey::Pid,
                sample_ms: 0,
                with_foreground: false,
                ..ListOptions::default()
            },
        );
        let rows = out["processes"].as_array().unwrap();
        let row = |pid: i64| rows.iter().find(|row| row["pid"] == pid).unwrap();

        assert_eq!(row(3913)["package"], Value::Null);
        assert_eq!(row(3913)["name"], "system_server");
        assert_eq!(row(5354)["package"], "com.android.systemui");
        assert_eq!(row(16023)["package"], "com.tencent.mm");
        assert_eq!(row(16023)["name"], "com.tencent.mm:push");
        assert_eq!(row(2341)["name"], "netd");
        assert_eq!(row(2341)["cmdline"], "/system/bin/netd");
        assert_eq!(row(2341)["package"], Value::Null);
        assert_eq!(row(7787)["package"], Value::Null);
    }

    #[test]
    fn list_filters_and_searches() {
        let root = temp("filters");
        add_process(&root, 1, "init", 0, 1000);
        add_process(&root, 10123, "com.example.app", 10123, 20_000);
        add_process(&root, 10456, "com.example.other", 10456, 5_000);
        write(
            &root,
            "data/system/packages.list",
            "com.example.app 10123 0 /data 0 0 1 0 0 0\ncom.example.other 10456 0 /data 0 0 1 0 0 0\n",
        );
        let fs = Sysfs::new(&root);
        let data = root.join("state-dir");

        let user = list(
            &fs,
            &data,
            &ListOptions {
                filter: Filter::User,
                sort: SortKey::Pid,
                sample_ms: 0,
                with_foreground: false,
                ..ListOptions::default()
            },
        );
        assert_eq!(user["returned"], 2);

        let system = list(
            &fs,
            &data,
            &ListOptions {
                filter: Filter::System,
                sort: SortKey::Pid,
                sample_ms: 0,
                with_foreground: false,
                ..ListOptions::default()
            },
        );
        assert_eq!(system["returned"], 1);
        assert_eq!(system["processes"][0]["pid"], 1);

        let query = list(
            &fs,
            &data,
            &ListOptions {
                query: Some("OTHER".to_string()),
                sort: SortKey::Pid,
                sample_ms: 0,
                with_foreground: false,
                ..ListOptions::default()
            },
        );
        assert_eq!(query["returned"], 1);
        assert_eq!(query["processes"][0]["pid"], 10456);
    }

    #[test]
    fn list_counts_unreadable_processes_and_truncates() {
        let root = temp("skipped");
        add_process(&root, 1, "init", 0, 1000);
        add_process(&root, 10123, "app", 10123, 10);
        // A pid directory whose stat cannot be read (process exited mid-scan).
        std::fs::create_dir_all(root.join("proc/99999")).unwrap();
        write(
            &root,
            "data/system/packages.list",
            "app 10123 0 /data 0 0 1 0 0 0\n",
        );
        let fs = Sysfs::new(&root);
        let data = root.join("state-dir");
        let out = list(
            &fs,
            &data,
            &ListOptions {
                limit: 1,
                sort: SortKey::Pid,
                sample_ms: 0,
                with_foreground: false,
                ..ListOptions::default()
            },
        );
        assert_eq!(out["skipped"], 1);
        assert_eq!(out["returned"], 1);
        assert_eq!(out["truncated"], true);
    }

    #[test]
    fn missing_proc_is_reported_as_unavailable() {
        let root = temp("no-proc");
        let fs = Sysfs::new(&root);
        let out = list(&fs, &root.join("state-dir"), &ListOptions::default());
        assert_eq!(out["available"], false);
        assert!(out["reason"].as_str().unwrap().contains("/proc"));
    }

    #[test]
    fn detail_includes_pss_cgroup_and_scheduler_fields() {
        let root = temp("detail");
        add_process(&root, 10123, "com.example.app", 10123, 20_000);
        write(
            &root,
            "proc/10123/stat",
            &stat_line(10123, "com.example.app", (10, 20)),
        );
        write(
            &root,
            "proc/10123/cmdline",
            "com.example.app\u{0}--flag\u{0}",
        );
        write(&root, "proc/10123/oom_score_adj", "100\n");
        write(
            &root,
            "proc/10123/cgroup",
            "4:cpuset:/top-app\n0::/apps/uid_10123/pid_10123\n",
        );
        write(
            &root,
            "proc/10123/smaps_rollup",
            "Rss:    20000 kB\nPss:     8000 kB\nPss_Anon: 1000 kB\nSwapPss:  128 kB\n",
        );
        write(
            &root,
            "data/system/packages.list",
            "com.example.app 10123 0 /data 0 0 1 0 0 0\n",
        );
        let fs = Sysfs::new(&root);
        let data = root.join("state-dir");
        let out = detail_with_sample(&fs, &data, 10123, 0).unwrap();
        assert_eq!(out["pid"], 10123);
        assert_eq!(out["package"], "com.example.app");
        assert_eq!(out["packages"], json!(["com.example.app"]));
        assert_eq!(out["pss_kb"], 8000);
        assert_eq!(out["swap_pss_kb"], 128);
        assert_eq!(out["rss_kb"], 20_000);
        assert_eq!(out["oom_score_adj"], 100);
        assert_eq!(out["nice"], 0);
        assert_eq!(out["policy_name"], "SCHED_OTHER");
        assert_eq!(out["cgroup"], "/apps/uid_10123/pid_10123");
        assert_eq!(out["cpuset"], "top-app");
        assert_eq!(out["cmdline"], json!(["com.example.app", "--flag"]));
        assert_eq!(out["protected"], false);
        assert_eq!(out["foreground"], false);
    }

    #[test]
    fn detail_of_a_dead_process_is_a_gone_error() {
        let root = temp("gone");
        std::fs::create_dir_all(root.join("proc")).unwrap();
        let fs = Sysfs::new(&root);
        let error = detail_with_sample(&fs, &root.join("state-dir"), 424242, 0).unwrap_err();
        assert_eq!(error.code.as_str(), "invalid_arg");
        assert_eq!(error.detail.unwrap()["gone"], true);
    }

    #[test]
    fn smaps_rollup_requires_pss_and_defaults_swap_to_zero() {
        assert_eq!(parse_smaps_rollup("Rss: 1 kB\nPss: 42 kB\n"), Some((42, 0)));
        assert_eq!(
            parse_smaps_rollup("Rss: 1 kB\nPss: 42 kB\nSwapPss: 7 kB\n"),
            Some((42, 7))
        );
        assert_eq!(parse_smaps_rollup("Rss: 1 kB\n"), None);
    }

    #[test]
    fn cpu_math_is_per_core_and_pid_reuse_is_safe() {
        let root = temp("cpu");
        add_process(&root, 10123, "app", 10123, 10);
        let fs = Sysfs::new(&root);
        let info = read_one(&fs, 10123).unwrap();
        assert_eq!(info.stat.ticks(), 4);
        // 50 ticks over 0.5 s at 100 Hz = 100 % of one core.
        let pct = (50.0) * 100.0 / (0.5 * 100.0);
        assert_eq!(round1(pct), 100.0);
        // A different starttime means the PID was recycled; sample_cpu must skip it.
        let recycled = stat_line(10123, "app", (60, 90)).replace(" 2588536 ", " 9999999 ");
        write(&root, "proc/10123/stat", &recycled);
        let sample = sample_cpu(&fs, std::slice::from_ref(&info), 100, 1);
        assert!(
            sample.is_empty(),
            "recycled pid must not produce a percentage"
        );
    }

    #[test]
    fn reads_frozen_state_from_cgroup_v2_and_v1() {
        let root = temp("frozen-state");
        add_process(&root, 101, "com.example.v2", 10101, 1000);
        write(&root, "proc/101/cgroup", "0::/apps/uid_10101/pid_101\n");
        write(
            &root,
            "sys/fs/cgroup/apps/uid_10101/pid_101/cgroup.freeze",
            "1\n",
        );

        add_process(&root, 102, "com.example.events", 10102, 1000);
        write(&root, "proc/102/cgroup", "0::/apps/uid_10102/pid_102\n");
        write(
            &root,
            "sys/fs/cgroup/apps/uid_10102/pid_102/cgroup.freeze",
            "0\n",
        );
        write(
            &root,
            "sys/fs/cgroup/apps/uid_10102/pid_102/cgroup.events",
            "populated 1\nfrozen 1\n",
        );

        add_process(&root, 103, "com.example.v1", 10103, 1000);
        write(&root, "proc/103/cgroup", "5:freezer:/frozen_app\n");
        write(&root, "dev/freezer/frozen_app/freezer.state", "THAWED\n");

        let fs = Sysfs::new(&root);
        assert_eq!(read_one(&fs, 101).unwrap().frozen, Some(true));
        assert_eq!(read_one(&fs, 102).unwrap().frozen, Some(true));
        assert_eq!(read_one(&fs, 103).unwrap().frozen, Some(false));
        let detail = detail_with_sample(&fs, &root.join("state-dir"), 101, 0).unwrap();
        assert_eq!(detail["frozen"], true);
    }
}
```

---

回复 **“继续”** 我发 **第 3 批**（`core/sysflow-modules/src/process/actions.rs` 完整文件）。

---

## You

继续

---

## AI Studio

### 第 3 批（1 个大文件）：`core/sysflow-modules/src/process/actions.rs`

#### `core/sysflow-modules/src/process/actions.rs`

```rust
//! One-shot process actions: `stop`, `oom`, `renice`, `sched`, `cpuset`, `freeze` / `unfreeze`.
//!
//! Every action follows the same order, and the order is the safety property:
//!
//! 1. resolve the target to concrete processes (PID or package expansion);
//! 2. reject the **whole** request if any resolved process is protected;
//! 3. re-check identity (`starttime`) before writing, so a recycled PID is never touched;
//! 4. perform the write / command;
//! 5. read the result back and report `before` / `applied` / `verified`.
//!
//! Nothing here writes `config.json`, so an action can never affect boot, drift or safe mode.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use sysflow_core::command;
use sysflow_core::error::{Error, Result};
use sysflow_core::event::{EventStore, NewSystemEvent};
use sysflow_core::log::Logger;
use sysflow_core::sysfs::Sysfs;

use super::model;
use super::packages;
use super::scan;

const AM_TIMEOUT: Duration = Duration::from_secs(8);
const TOOL_TIMEOUT: Duration = Duration::from_secs(3);
const STOP_VERIFY: Duration = Duration::from_secs(2);
const CPUSET_ROOT: &str = "/dev/cpuset";
const FREEZE_ROOT: &str = "/sys/fs/cgroup";
const V1_FREEZE_ROOT: &str = "/dev/freezer";

/// `PRIO_PROCESS` for `setpriority(2)`.
const PRIO_PROCESS: i32 = 0;

#[repr(C)]
struct SchedParam {
    sched_priority: i32,
}

extern "C" {
    fn setpriority(which: i32, who: u32, prio: i32) -> i32;
    fn sched_setscheduler(pid: i32, policy: i32, param: *const SchedParam) -> i32;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Pid(u32),
    Package(String),
}

impl Target {
    pub fn text(&self) -> String {
        match self {
            Target::Pid(pid) => pid.to_string(),
            Target::Package(package) => package.clone(),
        }
    }
}

/// `<pid|package>`: all-digits is a PID, anything else must be a valid package name.
pub fn parse_target(text: &str) -> Result<Target> {
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()) {
        let pid = text
            .parse::<u32>()
            .map_err(|_| Error::invalid_arg(format!("invalid PID: {text:?}")))?;
        if pid == 0 {
            return Err(Error::invalid_arg("PID 0 is not a process"));
        }
        return Ok(Target::Pid(pid));
    }
    validate_package(text)?;
    Ok(Target::Package(text.to_string()))
}

fn validate_package(package: &str) -> Result<()> {
    let valid = !package.is_empty()
        && package.len() <= 255
        && package.contains('.')
        && package
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_'));
    if valid {
        Ok(())
    } else {
        Err(Error::invalid_arg(format!(
            "invalid package name: {package:?}"
        )))
    }
}

#[derive(Clone, Debug)]
struct Resolved {
    info: scan::ProcessInfo,
    package: Option<String>,
}

fn package_of(info: &scan::ProcessInfo, map: &packages::Packages) -> Option<String> {
    // One rule for the whole module: never label a shared/system UID with an arbitrary package.
    scan::package_of(info, map)
}

fn resolve(
    fs: &Sysfs,
    data_dir: &Path,
    target: &Target,
    user: Option<u32>,
) -> Result<Vec<Resolved>> {
    let map = packages::read(fs, data_dir);
    match target {
        Target::Pid(pid) => {
            let info = scan::read_one(fs, *pid).ok_or_else(|| {
                Error::invalid_arg(format!("process {pid} does not exist"))
                    .with_detail(json!({"gone": true, "pid": pid}))
            })?;
            if user.is_some_and(|user| info.uid / 100_000 != user) {
                return Ok(Vec::new());
            }
            let package = package_of(&info, &map);
            Ok(vec![Resolved { info, package }])
        }
        Target::Package(package) => {
            let (infos, _) = scan::read_all(fs)?;
            let suffix = format!("{package}:");
            let resolved = infos
                .into_iter()
                .filter_map(|info| {
                    let resolved_package = package_of(&info, &map);
                    let matches = info.name == *package
                        || info.cmdline == *package
                        || info.cmdline.starts_with(&suffix)
                        || resolved_package.as_deref() == Some(package.as_str());
                    let same_user = user.is_none_or(|user| info.uid / 100_000 == user);
                    (matches && same_user).then_some(Resolved {
                        info,
                        package: resolved_package,
                    })
                })
                .collect();
            Ok(resolved)
        }
    }
}

/// Reject the request as a whole if any resolved process is protected: a package action must never
/// half-apply because one of its processes happened to be `systemui`.
fn ensure_actionable(resolved: &[Resolved]) -> Result<()> {
    for target in resolved {
        if let Some(reason) = model::protected_reason(
            target.info.uid,
            target.info.pid,
            target.info.kthread,
            &target.info.name,
            target.package.as_deref(),
        ) {
            return Err(Error::invalid_arg(format!(
                "{} is protected ({reason})",
                target.info.name
            ))
            .with_detail(json!({
                "protected": true,
                "reason": reason,
                "pid": target.info.pid,
                "name": target.info.name,
            })));
        }
    }
    Ok(())
}

fn still_same_process(fs: &Sysfs, info: &scan::ProcessInfo) -> bool {
    scan::read_stat(fs, info.pid).is_some_and(|stat| stat.starttime == info.stat.starttime)
}

fn log_action(data_dir: &Path, message: String) {
    Logger::new(data_dir.join("log").join("sysflow.log")).info(&message);
}

fn emit_process_event(
    data_dir: &Path,
    severity: &str,
    title: impl Into<String>,
    message: impl Into<String>,
    metadata: Value,
) {
    let store = EventStore::new(data_dir);
    let event = NewSystemEvent::new("process", "action", "process_action", severity, title)
        .message(message)
        .metadata(metadata);
    // Best-effort telemetry: event store failures must never fail the process action.
    let _ = store.emit_sanitized(event);
}

pub fn stop(fs: &Sysfs, data_dir: &Path, target: &Target, user: Option<u32>) -> Result<Value> {
    let resolved = resolve(fs, data_dir, target, user)?;
    let package = match target {
        Target::Package(package) => package.clone(),
        Target::Pid(_) => resolved
            .first()
            .and_then(|target| target.package.clone())
            .ok_or_else(|| Error::invalid_arg("cannot resolve a package for this process"))?,
    };
    validate_package(&package)?;

    if resolved.is_empty() {
        return Ok(json!({
            "package": package,
            "user": user.unwrap_or(0),
            "pids_before": [],
            "pids_after": [],
            "verified": true,
            "already_stopped": true,
        }));
    }
    ensure_actionable(&resolved)?;

    let user = user.unwrap_or(resolved[0].info.uid / 100_000);
    let pids_before: Vec<u32> = resolved.iter().map(|target| target.info.pid).collect();
    command::run(
        "am",
        &["force-stop", "--user", &user.to_string(), &package],
        AM_TIMEOUT,
    )?;

    let started = Instant::now();
    let pids_after = loop {
        let remaining: Vec<u32> = resolved
            .iter()
            .filter(|target| still_same_process(fs, &target.info))
            .map(|target| target.info.pid)
            .collect();
        if remaining.is_empty() || started.elapsed() >= STOP_VERIFY {
            break remaining;
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let verified = pids_after.is_empty();
    log_action(
        data_dir,
        format!(
            "process stop package={package} user={user} before={pids_before:?} after={pids_after:?}"
        ),
    );
    emit_process_event(
        data_dir,
        if verified { "info" } else { "warning" },
        if verified {
            "Process stopped"
        } else {
            "Process stop incomplete"
        },
        if verified {
            format!("Force-stopped {package} (user {user}).")
        } else {
            format!(
                "Force-stop requested for {package} (user {user}), surviving pids: {pids_after:?}."
            )
        },
        json!({
            "action": "stop",
            "target": package,
            "user": user,
            "pids_before": pids_before,
            "pids_after": pids_after,
            "verified": verified,
        }),
    );
    Ok(json!({
        "package": package,
        "user": user,
        "pids_before": pids_before,
        "pids_after": pids_after,
        "verified": verified,
        "already_stopped": false,
    }))
}

pub fn oom(fs: &Sysfs, data_dir: &Path, target: &Target, adjustment: i64) -> Result<Value> {
    if !(-1000..=1000).contains(&adjustment) {
        return Err(
            Error::invalid_arg("oom_score_adj must be within -1000..=1000")
                .with_detail(json!({"range": [-1000, 1000]})),
        );
    }
    let resolved = resolve(fs, data_dir, target, None)?;
    if resolved.is_empty() {
        return Err(Error::invalid_arg(
            "no running process matches this package",
        ));
    }
    ensure_actionable(&resolved)?;

    let mut targets = Vec::new();
    let mut failure: Option<Error> = None;
    let mut applied_count = 0;
    for target in &resolved {
        if !still_same_process(fs, &target.info) {
            if failure.is_none() {
                failure = Some(
                    Error::invalid_arg(format!(
                        "process {} ({}) exited or PID was reused",
                        target.info.pid, target.info.name
                    ))
                    .with_detail(json!({"gone": true, "pid": target.info.pid})),
                );
            }
            targets.push(json!({
                "pid": target.info.pid,
                "before": target.info.oom_score_adj,
                "applied": Value::Null,
                "verified": false,
                "error": "process exited or PID was reused",
            }));
            continue;
        }
        let path = format!("/proc/{}/oom_score_adj", target.info.pid);
        let result = fs.write_str(&path, &adjustment.to_string());
        let (applied, error) = match result {
            Ok(()) => (
                fs.read_str(&path)
                    .ok()
                    .and_then(|value| value.trim().parse::<i64>().ok()),
                None,
            ),
            Err(e) => (None, Some(e.to_string())),
        };
        if error.is_none() && applied == Some(adjustment) {
            applied_count += 1;
        } else if failure.is_none() {
            failure = Some(Error::permission(format!(
                "cannot set oom_score_adj on pid {}: {}",
                target.info.pid,
                error
                    .clone()
                    .unwrap_or_else(|| format!("read back {applied:?}"))
            )));
        }
        targets.push(json!({
            "pid": target.info.pid,
            "before": target.info.oom_score_adj,
            "applied": applied,
            "verified": applied == Some(adjustment),
            "error": error,
        }));
    }
    if applied_count == 0 {
        return Err(failure.unwrap_or_else(|| Error::io("no oom_score_adj write succeeded")));
    }

    let all_verified = applied_count == resolved.len();
    log_action(
        data_dir,
        format!(
            "process oom target={} adjustment={adjustment} pids={} verified={applied_count}",
            target.text(),
            resolved.len()
        ),
    );
    emit_process_event(
        data_dir,
        if all_verified { "info" } else { "warning" },
        "Process OOM adjustment updated",
        format!(
            "Set oom_score_adj={adjustment} for {} ({applied_count}/{} verified).",
            target.text(),
            resolved.len()
        ),
        json!({
            "action": "oom",
            "target": target.text(),
            "adjustment": adjustment,
            "applied": adjustment,
            "verified": all_verified,
            "pids": resolved.len(),
        }),
    );
    Ok(json!({
        "target": target.text(),
        "adjustment": adjustment,
        "targets": targets,
    }))
}

fn set_priority(pid: u32, nice: i32) -> std::io::Result<()> {
    // SAFETY: setpriority is a thread-safe libc wrapper over a syscall; the arguments are integers
    // and there is no memory ownership involved.
    let result = unsafe { setpriority(PRIO_PROCESS, pid, nice) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// toybox `renice -n` takes an **increment**, not an absolute nice, so the delta is computed here.
fn renice_tool(pid: u32, current: i64, target: i64) -> Result<()> {
    let delta = target - current;
    command::run(
        "renice",
        &["-n", &delta.to_string(), "-p", &pid.to_string()],
        TOOL_TIMEOUT,
    )
    .map(|_| ())
}

pub fn renice(fs: &Sysfs, data_dir: &Path, target: &Target, nice: i64) -> Result<Value> {
    if !(-20..=19).contains(&nice) {
        return Err(Error::invalid_arg("nice must be within -20..=19")
            .with_detail(json!({"range": [-20, 19]})));
    }
    let resolved = resolve(fs, data_dir, target, None)?;
    if resolved.is_empty() {
        return Err(Error::invalid_arg(
            "no running process matches this package",
        ));
    }
    ensure_actionable(&resolved)?;

    let mut targets = Vec::new();
    let mut failure: Option<Error> = None;
    let mut verified_count = 0;
    for target in &resolved {
        if !still_same_process(fs, &target.info) {
            if failure.is_none() {
                failure = Some(
                    Error::invalid_arg(format!(
                        "process {} ({}) exited or PID was reused",
                        target.info.pid, target.info.name
                    ))
                    .with_detail(json!({"gone": true, "pid": target.info.pid})),
                );
            }
            targets.push(json!({
                "pid": target.info.pid,
                "before": target.info.stat.nice,
                "applied": Value::Null,
                "verified": false,
                "mechanism": "gone",
                "error": "process exited or PID was reused",
            }));
            continue;
        }
        let before = target.info.stat.nice;
        let mut mechanism = "setpriority";
        if let Err(error) = set_priority(target.info.pid, nice as i32) {
            match renice_tool(target.info.pid, before, nice) {
                Ok(()) => mechanism = "renice",
                Err(_) => {
                    if failure.is_none() {
                        failure = Some(Error::permission(format!(
                            "cannot renice pid {}: {error}",
                            target.info.pid
                        )));
                    }
                    mechanism = "failed";
                }
            }
        }
        let applied = scan::read_stat(fs, target.info.pid).map(|stat| stat.nice);
        if applied == Some(nice) {
            verified_count += 1;
        }
        targets.push(json!({
            "pid": target.info.pid,
            "before": before,
            "applied": applied,
            "verified": applied == Some(nice),
            "mechanism": mechanism,
        }));
    }
    if verified_count == 0 {
        return Err(failure.unwrap_or_else(|| Error::io("no renice succeeded")));
    }

    let all_verified = verified_count == resolved.len();
    log_action(
        data_dir,
        format!(
            "process renice target={} nice={nice} pids={} verified={verified_count}",
            target.text(),
            resolved.len()
        ),
    );
    emit_process_event(
        data_dir,
        if all_verified { "info" } else { "warning" },
        "Process nice priority updated",
        format!(
            "Set nice={nice} for {} ({verified_count}/{} verified).",
            target.text(),
            resolved.len()
        ),
        json!({
            "action": "renice",
            "target": target.text(),
            "nice": nice,
            "applied": nice,
            "verified": all_verified,
            "pids": resolved.len(),
        }),
    );
    Ok(json!({
        "target": target.text(),
        "nice": nice,
        "targets": targets,
    }))
}

fn set_scheduler(pid: u32, policy: i32, priority: i32) -> std::io::Result<()> {
    let param = SchedParam {
        sched_priority: priority,
    };
    // SAFETY: `sched_setscheduler` is a standard Linux syscall taking a PID, scheduling policy
    // integer, and valid pointer to `SchedParam`.
    let result = unsafe { sched_setscheduler(pid as i32, policy, &param) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub fn sched(
    fs: &Sysfs,
    data_dir: &Path,
    target: &Target,
    policy_name: &str,
    priority: Option<i64>,
) -> Result<Value> {
    let policy = model::parse_sched_policy(policy_name).ok_or_else(|| {
        Error::invalid_arg(format!(
            "unknown scheduling policy: {policy_name:?}; expected other|fifo|rr|batch|idle"
        ))
    })?;
    let canonical_name = model::sched_policy_name(policy as u32);
    let rt_prio = if model::is_realtime_policy(policy) {
        let p = priority.ok_or_else(|| {
            Error::invalid_arg(format!("{canonical_name} requires priority within 1..=99"))
                .with_detail(json!({"range": [1, 99]}))
        })?;
        if !(1..=99).contains(&p) {
            return Err(Error::invalid_arg(format!(
                "{canonical_name} priority must be within 1..=99"
            ))
            .with_detail(json!({"range": [1, 99]})));
        }
        p as i32
    } else {
        let p = priority.unwrap_or(0);
        if p != 0 {
            return Err(Error::invalid_arg(format!(
                "{canonical_name} requires priority 0"
            ))
            .with_detail(json!({"range": [0, 0]})));
        }
        0
    };

    let resolved = resolve(fs, data_dir, target, None)?;
    if resolved.is_empty() {
        return Err(Error::invalid_arg(
            "no running process matches this package",
        ));
    }
    ensure_actionable(&resolved)?;

    let mut targets = Vec::new();
    let mut failure: Option<Error> = None;
    let mut write_count = 0usize;
    let mut verified_count = 0usize;

    for item in &resolved {
        let before = format!(
            "{} ({})",
            model::sched_policy_name(item.info.stat.policy),
            item.info.stat.rt_priority
        );
        if !still_same_process(fs, &item.info) {
            if failure.is_none() {
                failure = Some(
                    Error::invalid_arg(format!(
                        "process {} ({}) exited or PID was reused",
                        item.info.pid, item.info.name
                    ))
                    .with_detail(json!({"gone": true, "pid": item.info.pid})),
                );
            }
            targets.push(json!({
                "pid": item.info.pid,
                "before": before,
                "applied": Value::Null,
                "verified": false,
                "mechanism": "gone",
                "error": "process exited or PID was reused",
            }));
            continue;
        }
        match set_scheduler(item.info.pid, policy, rt_prio) {
            Ok(()) => {
                write_count += 1;
                let readback = scan::read_stat(fs, item.info.pid);
                let verified = readback.as_ref().is_some_and(|stat| {
                    stat.policy == policy as u32 && stat.rt_priority == rt_prio as u32
                });
                if verified {
                    verified_count += 1;
                }
                let applied = readback.as_ref().map(|stat| {
                    format!(
                        "{} ({})",
                        model::sched_policy_name(stat.policy),
                        stat.rt_priority
                    )
                });
                targets.push(json!({
                    "pid": item.info.pid,
                    "before": before,
                    "applied": applied,
                    "verified": verified,
                    "mechanism": "sched_setscheduler",
                }));
            }
            Err(error) => {
                if failure.is_none() {
                    let mapped = if matches!(error.raw_os_error(), Some(1 | 13)) {
                        Error::permission(format!(
                            "cannot set scheduling policy on pid {}: {error}",
                            item.info.pid
                        ))
                    } else {
                        Error::io(format!(
                            "cannot set scheduling policy on pid {}: {error}",
                            item.info.pid
                        ))
                    };
                    failure = Some(mapped);
                }
                targets.push(json!({
                    "pid": item.info.pid,
                    "before": before,
                    "applied": Value::Null,
                    "verified": false,
                    "mechanism": "failed",
                    "error": error.to_string(),
                }));
            }
        }
    }

    if write_count == 0 {
        return Err(failure.unwrap_or_else(|| Error::io("no scheduling policy change succeeded")));
    }

    let all_verified = verified_count == resolved.len();
    log_action(
        data_dir,
        format!(
            "process sched target={} policy={canonical_name} priority={rt_prio} pids={} verified={verified_count}",
            target.text(),
            resolved.len()
        ),
    );
    emit_process_event(
        data_dir,
        if all_verified { "info" } else { "warning" },
        "Process scheduling policy updated",
        format!(
            "Set {canonical_name} (priority {rt_prio}) for {} ({verified_count}/{} verified).",
            target.text(),
            resolved.len()
        ),
        json!({
            "action": "sched",
            "target": target.text(),
            "policy": canonical_name,
            "policy_id": policy,
            "priority": rt_prio,
            "verified": all_verified,
            "pids": resolved.len(),
        }),
    );
    Ok(json!({
        "target": target.text(),
        "policy": canonical_name,
        "policy_id": policy,
        "priority": rt_prio,
        "persisted": false,
        "targets": targets,
        "note": "Scheduling policy is one-shot and may be reset by ActivityManager",
    }))
}

pub fn cpuset(fs: &Sysfs, data_dir: &Path, target: &Target, group: &str) -> Result<Value> {
    let dir = format!("{CPUSET_ROOT}/{group}");
    let valid_group = std::fs::metadata(fs.resolve(&dir))
        .map(|meta| meta.is_dir())
        .unwrap_or(false)
        && fs.exists(&format!("{dir}/tasks"));
    if !valid_group {
        return Err(Error::invalid_arg(format!(
            "unknown cpuset group: {group:?}"
        )));
    }
    let resolved = resolve(fs, data_dir, target, None)?;
    if resolved.is_empty() {
        return Err(Error::invalid_arg(
            "no running process matches this package",
        ));
    }
    ensure_actionable(&resolved)?;

    let mut targets = Vec::new();
    let mut write_count = 0usize;
    let mut verified_count = 0;
    let mut failure: Option<Error> = None;
    for target in &resolved {
        if !still_same_process(fs, &target.info) {
            if failure.is_none() {
                failure = Some(
                    Error::invalid_arg(format!(
                        "process {} ({}) exited or PID was reused",
                        target.info.pid, target.info.name
                    ))
                    .with_detail(json!({"gone": true, "pid": target.info.pid})),
                );
            }
            targets.push(json!({
                "pid": target.info.pid,
                "before": target
                    .info
                    .cgroup
                    .as_ref()
                    .and_then(model::Cgroup::cpuset_group),
                "applied": Value::Null,
                "verified": false,
                "error": "process exited or PID was reused",
            }));
            continue;
        }
        let before = target
            .info
            .cgroup
            .as_ref()
            .and_then(model::Cgroup::cpuset_group);
        let write = fs.write_str(&format!("{dir}/tasks"), &target.info.pid.to_string());
        let applied = match write {
            Ok(()) => {
                write_count += 1;
                scan::read_one(fs, target.info.pid)
                    .and_then(|info| info.cgroup)
                    .and_then(|cgroup| cgroup.cpuset_group())
            }
            Err(e) => {
                return Err(Error::io(format!(
                    "cannot move pid {} to {group}: {e}",
                    target.info.pid
                )));
            }
        };
        if applied.as_deref() == Some(group) {
            verified_count += 1;
        }
        targets.push(json!({
            "pid": target.info.pid,
            "before": before,
            "applied": applied,
            "verified": applied.as_deref() == Some(group),
        }));
    }
    if write_count == 0 {
        return Err(failure.unwrap_or_else(|| Error::io("no cpuset move succeeded")));
    }

    let all_verified = verified_count == resolved.len();
    log_action(
        data_dir,
        format!(
            "process cpuset target={} group={group} pids={} verified={verified_count}",
            target.text(),
            resolved.len()
        ),
    );
    emit_process_event(
        data_dir,
        if all_verified { "info" } else { "warning" },
        "Process cpuset updated",
        format!(
            "Moved {} to cpuset {group} ({verified_count}/{} verified).",
            target.text(),
            resolved.len()
        ),
        json!({
            "action": "cpuset",
            "target": target.text(),
            "group": group,
            "verified": all_verified,
            "pids": resolved.len(),
        }),
    );
    Ok(json!({
        "group": group,
        "persisted": false,
        "targets": targets,
        "note": "ActivityManager may move the process back on its next state change",
    }))
}

fn freeze_path(cgroup: Option<&model::Cgroup>) -> Option<String> {
    let unified = cgroup?.unified.as_deref()?;
    let base = format!("{FREEZE_ROOT}{}", unified.trim_end_matches('/'));
    Some(format!("{base}/cgroup.freeze"))
}

fn v1_freeze_path(cgroup: Option<&model::Cgroup>) -> Option<String> {
    let path = cgroup?.freezer.as_deref()?;
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        Some(format!("{V1_FREEZE_ROOT}/freezer.state"))
    } else {
        Some(format!("{V1_FREEZE_ROOT}{trimmed}/freezer.state"))
    }
}

pub fn freeze(fs: &Sysfs, data_dir: &Path, target: &Target, frozen: bool) -> Result<Value> {
    let resolved = resolve(fs, data_dir, target, None)?;
    if resolved.is_empty() {
        return Err(Error::invalid_arg(
            "no running process matches this package",
        ));
    }
    ensure_actionable(&resolved)?;

    let value = if frozen { "1" } else { "0" };
    let use_cgroup = resolved.iter().all(|target| {
        freeze_path(target.info.cgroup.as_ref()).is_some_and(|path| fs.exists(&path))
    });
    let action_name = if frozen { "freeze" } else { "unfreeze" };

    if use_cgroup {
        let mut targets = Vec::new();
        let mut write_count = 0usize;
        let mut verified_count = 0usize;
        let mut failure: Option<Error> = None;
        for target in &resolved {
            if !still_same_process(fs, &target.info) {
                if failure.is_none() {
                    failure = Some(
                        Error::invalid_arg(format!(
                            "process {} ({}) exited or PID was reused",
                            target.info.pid, target.info.name
                        ))
                        .with_detail(json!({"gone": true, "pid": target.info.pid})),
                    );
                }
                targets.push(json!({
                    "pid": target.info.pid,
                    "before": Value::Null,
                    "applied": Value::Null,
                    "verified": false,
                    "error": "process exited or PID was reused",
                }));
                continue;
            }
            let path = freeze_path(target.info.cgroup.as_ref()).expect("checked above");
            let before = fs.read_str(&path).ok();
            fs.write_str(&path, value)?;
            write_count += 1;
            let applied = fs.read_str(&path).ok();
            let verified = applied.as_deref() == Some(value);
            if verified {
                verified_count += 1;
            }
            targets.push(json!({
                "pid": target.info.pid,
                "before": before,
                "applied": applied,
                "verified": verified,
            }));
        }
        if write_count == 0 {
            return Err(failure.unwrap_or_else(|| Error::io("no cgroup freeze write succeeded")));
        }
        let all_verified = verified_count == resolved.len();
        log_action(
            data_dir,
            format!(
                "process {action_name} target={} mechanism=cgroup-v2 pids={}",
                target.text(),
                resolved.len()
            ),
        );
        emit_process_event(
            data_dir,
            if all_verified { "info" } else { "warning" },
            if frozen {
                "Process frozen"
            } else {
                "Process unfrozen"
            },
            format!(
                "{} {} via cgroup-v2 ({verified_count}/{} verified).",
                if frozen { "Frozen" } else { "Unfroze" },
                target.text(),
                resolved.len()
            ),
            json!({
                "action": action_name,
                "target": target.text(),
                "frozen": frozen,
                "mechanism": "cgroup-v2",
                "verified": all_verified,
                "pids": resolved.len(),
            }),
        );
        return Ok(json!({
            "frozen": frozen,
            "mechanism": "cgroup-v2",
            "targets": targets,
        }));
    }

    let use_cgroup_v1 = resolved.iter().all(|target| {
        v1_freeze_path(target.info.cgroup.as_ref()).is_some_and(|path| fs.exists(&path))
    });
    if use_cgroup_v1 {
        let v1_value = if frozen { "FROZEN" } else { "THAWED" };
        let mut targets = Vec::new();
        let mut write_count = 0usize;
        let mut verified_count = 0usize;
        let mut failure: Option<Error> = None;
        for target in &resolved {
            if !still_same_process(fs, &target.info) {
                if failure.is_none() {
                    failure = Some(
                        Error::invalid_arg(format!(
                            "process {} ({}) exited or PID was reused",
                            target.info.pid, target.info.name
                        ))
                        .with_detail(json!({"gone": true, "pid": target.info.pid})),
                    );
                }
                targets.push(json!({
                    "pid": target.info.pid,
                    "before": Value::Null,
                    "applied": Value::Null,
                    "verified": false,
                    "error": "process exited or PID was reused",
                }));
                continue;
            }
            let path = v1_freeze_path(target.info.cgroup.as_ref()).expect("checked above");
            let before = fs.read_str(&path).ok();
            fs.write_str(&path, v1_value)?;
            write_count += 1;
            let applied = fs.read_str(&path).ok();
            let verified = applied.as_deref().is_some_and(|state| {
                let upper = state.trim().to_ascii_uppercase();
                if frozen {
                    upper == "FROZEN" || upper == "FREEZING"
                } else {
                    upper == "THAWED"
                }
            });
            if verified {
                verified_count += 1;
            }
            targets.push(json!({
                "pid": target.info.pid,
                "before": before,
                "applied": applied,
                "verified": verified,
            }));
        }
        if write_count == 0 {
            return Err(failure.unwrap_or_else(|| Error::io("no cgroup-v1 freezer write succeeded")));
        }
        let all_verified = verified_count == resolved.len();
        log_action(
            data_dir,
            format!(
                "process {action_name} target={} mechanism=cgroup-v1 pids={}",
                target.text(),
                resolved.len()
            ),
        );
        emit_process_event(
            data_dir,
            if all_verified { "info" } else { "warning" },
            if frozen {
                "Process frozen"
            } else {
                "Process unfrozen"
            },
            format!(
                "{} {} via cgroup-v1 ({verified_count}/{} verified).",
                if frozen { "Frozen" } else { "Unfroze" },
                target.text(),
                resolved.len()
            ),
            json!({
                "action": action_name,
                "target": target.text(),
                "frozen": frozen,
                "mechanism": "cgroup-v1",
                "verified": all_verified,
                "pids": resolved.len(),
            }),
        );
        return Ok(json!({
            "frozen": frozen,
            "mechanism": "cgroup-v1",
            "targets": targets,
        }));
    }

    // Fallback: `am freeze|unfreeze <package>`; the command's exit status is the only verifiable
    // signal, so `verified` is null rather than a guessed `true`.
    let package = match target {
        Target::Package(package) => package.clone(),
        Target::Pid(_) => resolved
            .first()
            .and_then(|target| target.package.clone())
            .ok_or_else(|| Error::unsupported("cannot resolve a package for this process"))?,
    };
    command::run("am", &[action_name, &package], AM_TIMEOUT)?;
    log_action(
        data_dir,
        format!("process {action_name} package={package} mechanism=am"),
    );
    emit_process_event(
        data_dir,
        "info",
        if frozen {
            "Process freeze requested"
        } else {
            "Process unfreeze requested"
        },
        format!("Requested {action_name} for {package} via am."),
        json!({
            "action": action_name,
            "target": package,
            "frozen": frozen,
            "mechanism": "am",
            "verified": Value::Null,
        }),
    );
    Ok(json!({
        "frozen": frozen,
        "mechanism": "am",
        "targets": resolved.iter().map(|target| json!({
            "pid": target.info.pid,
            "before": Value::Null,
            "applied": Value::Null,
            "verified": Value::Null,
        })).collect::<Vec<_>>(),
        "note": "verified by the am command's exit status only",
    }))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sysflow-process-actions-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn stat_line(pid: u32, comm: &str) -> String {
        format!(
            "{pid} ({comm}) S 1 {pid} 0 0 -1 4194560 100 0 0 0 1 3 0 0 20 0 1 0 2588536 10000000 500 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n"
        )
    }

    fn add_process(root: &Path, pid: u32, comm: &str, uid: u32) {
        write(root, &format!("proc/{pid}/stat"), &stat_line(pid, comm));
        write(
            root,
            &format!("proc/{pid}/status"),
            &format!("Name:\t{comm}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nVmRSS:\t100 kB\n"),
        );
        write(
            root,
            &format!("proc/{pid}/cmdline"),
            &format!("{comm}\u{0}"),
        );
        write(root, &format!("proc/{pid}/oom_score_adj"), "0\n");
        write(
            root,
            &format!("proc/{pid}/cgroup"),
            "4:cpuset:/top-app\n0::/apps/uid_10123/pid_10123\n",
        );
    }

    fn app_fixture(name: &str) -> (PathBuf, PathBuf) {
        let root = temp(name);
        add_process(&root, 10123, "com.example.app", 10123);
        write(
            &root,
            "data/system/packages.list",
            "com.example.app 10123 0 /data 0 0 1 0 0 0\n",
        );
        let data = root.join("data-dir");
        (root, data)
    }

    #[test]
    fn targets_parse_as_pid_or_package_and_reject_garbage() {
        assert_eq!(parse_target("10123").unwrap(), Target::Pid(10123));
        assert_eq!(
            parse_target("com.example.app").unwrap(),
            Target::Package("com.example.app".to_string())
        );
        assert!(parse_target("0").is_err());
        assert!(parse_target("nodots").is_err());
        assert!(parse_target("bad/name").is_err());
        assert!(parse_target("").is_err());
    }

    #[test]
    fn oom_writes_and_reads_back_the_value() {
        let (root, data) = app_fixture("oom");
        let fs = Sysfs::new(&root);
        let out = oom(&fs, &data, &Target::Pid(10123), 500).unwrap();
        assert_eq!(out["targets"][0]["pid"], 10123);
        assert_eq!(out["targets"][0]["before"], 0);
        assert_eq!(out["targets"][0]["applied"], 500);
        assert_eq!(out["targets"][0]["verified"], true);
        assert_eq!(
            std::fs::read_to_string(root.join("proc/10123/oom_score_adj")).unwrap(),
            "500"
        );
        assert!(out["targets"][0].get("error").unwrap().is_null());
    }

    #[test]
    fn oom_rejects_out_of_range_before_writing() {
        let (root, data) = app_fixture("oom-range");
        let fs = Sysfs::new(&root);
        let error = oom(&fs, &data, &Target::Pid(10123), 5000).unwrap_err();
        assert_eq!(error.code.as_str(), "invalid_arg");
        assert_eq!(error.detail.unwrap()["range"], json!([-1000, 1000]));
        assert_eq!(
            std::fs::read_to_string(root.join("proc/10123/oom_score_adj"))
                .unwrap()
                .trim(),
            "0",
            "a rejected request must not write"
        );
    }

    #[test]
    fn protected_targets_are_rejected_with_a_reason() {
        let root = temp("protected");
        add_process(&root, 1, "init", 0);
        write(&root, "data/system/packages.list", "");
        let fs = Sysfs::new(&root);
        let error = oom(&fs, &root.join("data-dir"), &Target::Pid(1), 100).unwrap_err();
        assert_eq!(error.code.as_str(), "invalid_arg");
        let detail = error.detail.unwrap();
        assert_eq!(detail["protected"], true);
        assert_eq!(detail["reason"], "pid ≤ 2 is the kernel");
    }

    #[test]
    fn stop_of_a_package_with_no_process_is_a_no_op() {
        let root = temp("stop-empty");
        std::fs::create_dir_all(root.join("proc")).unwrap();
        write(
            &root,
            "data/system/packages.list",
            "com.example.app 10123 0 /data 0 0 1 0 0 0\n",
        );
        let fs = Sysfs::new(&root);
        let out = stop(
            &fs,
            &root.join("data-dir"),
            &Target::Package("com.example.ghost".to_string()),
            None,
        )
        .unwrap();
        assert_eq!(out["already_stopped"], true);
        assert_eq!(out["verified"], true);
        assert_eq!(out["pids_before"], json!([]));
    }

    #[test]
    fn cpuset_rejects_unknown_groups() {
        let (root, data) = app_fixture("cpuset-bad");
        let fs = Sysfs::new(&root);
        let error = cpuset(&fs, &data, &Target::Pid(10123), "nope").unwrap_err();
        assert_eq!(error.code.as_str(), "invalid_arg");
    }

    #[test]
    fn cpuset_moves_the_pid_and_verifies_the_readback() {
        let (root, data) = app_fixture("cpuset");
        write(&root, "dev/cpuset/top-app/tasks", "");
        write(&root, "dev/cpuset/restricted/tasks", "");
        let fs = Sysfs::new(&root);
        let out = cpuset(&fs, &data, &Target::Pid(10123), "restricted").unwrap();
        assert_eq!(out["group"], "restricted");
        assert_eq!(out["persisted"], false);
        assert_eq!(out["targets"][0]["before"], "top-app");
        // The fake /proc cgroup is static, so the read-back still says top-app: the action must
        // report verified=false rather than claiming success.
        assert_eq!(out["targets"][0]["verified"], false);
        assert_eq!(
            std::fs::read_to_string(root.join("dev/cpuset/restricted/tasks")).unwrap(),
            "10123"
        );
    }

    #[test]
    fn freeze_uses_the_cgroup_v2_leaf_when_present() {
        let (root, data) = app_fixture("freeze");
        write(
            &root,
            "sys/fs/cgroup/apps/uid_10123/pid_10123/cgroup.freeze",
            "0\n",
        );
        let fs = Sysfs::new(&root);
        let out = freeze(&fs, &data, &Target::Pid(10123), true).unwrap();
        assert_eq!(out["mechanism"], "cgroup-v2");
        assert_eq!(out["targets"][0]["before"], "0");
        assert_eq!(out["targets"][0]["applied"], "1");
        assert_eq!(out["targets"][0]["verified"], true);
        assert_eq!(
            std::fs::read_to_string(
                root.join("sys/fs/cgroup/apps/uid_10123/pid_10123/cgroup.freeze")
            )
            .unwrap(),
            "1"
        );
    }

    #[test]
    fn freeze_supports_cgroup_v1_dev_freezer() {
        let (root, data) = app_fixture("freeze-v1");
        write(
            &root,
            "proc/10123/cgroup",
            "5:freezer:/uid_10123\n4:cpuset:/top-app\n",
        );
        write(&root, "dev/freezer/uid_10123/freezer.state", "THAWED\n");
        let fs = Sysfs::new(&root);
        let out = freeze(&fs, &data, &Target::Pid(10123), true).unwrap();
        assert_eq!(out["mechanism"], "cgroup-v1");
        assert_eq!(out["targets"][0]["before"], "THAWED");
        assert_eq!(out["targets"][0]["applied"], "FROZEN");
        assert_eq!(out["targets"][0]["verified"], true);
        assert_eq!(
            std::fs::read_to_string(root.join("dev/freezer/uid_10123/freezer.state")).unwrap(),
            "FROZEN"
        );
    }

    #[test]
    fn renice_rejects_out_of_range_values() {
        let (root, data) = app_fixture("renice-range");
        let fs = Sysfs::new(&root);
        let error = renice(&fs, &data, &Target::Pid(10123), 99).unwrap_err();
        assert_eq!(error.code.as_str(), "invalid_arg");
        assert_eq!(error.detail.unwrap()["range"], json!([-20, 19]));
    }

    #[test]
    fn sched_validates_policy_priority_and_protection() {
        let (root, data) = app_fixture("sched-validate");
        let fs = Sysfs::new(&root);

        let bad_policy = sched(&fs, &data, &Target::Pid(10123), "deadline", None).unwrap_err();
        assert_eq!(bad_policy.code.as_str(), "invalid_arg");

        let missing_prio = sched(&fs, &data, &Target::Pid(10123), "fifo", None).unwrap_err();
        assert_eq!(missing_prio.code.as_str(), "invalid_arg");
        assert_eq!(missing_prio.detail.unwrap()["range"], json!([1, 99]));

        let bad_rt_prio = sched(&fs, &data, &Target::Pid(10123), "rr", Some(100)).unwrap_err();
        assert_eq!(bad_rt_prio.code.as_str(), "invalid_arg");

        let bad_other_prio =
            sched(&fs, &data, &Target::Pid(10123), "other", Some(10)).unwrap_err();
        assert_eq!(bad_other_prio.code.as_str(), "invalid_arg");
        assert_eq!(bad_other_prio.detail.unwrap()["range"], json!([0, 0]));

        add_process(&root, 1, "init", 0);
        let protected = sched(&fs, &data, &Target::Pid(1), "fifo", Some(10)).unwrap_err();
        assert_eq!(protected.detail.unwrap()["protected"], true);
    }

    #[test]
    fn actions_emit_structured_process_events() {
        let (root, data) = app_fixture("events");
        let fs = Sysfs::new(&root);
        oom(&fs, &data, &Target::Pid(10123), 300).unwrap();
        let store = EventStore::new(&data);
        let events = store
            .list(sysflow_core::event::EventFilter {
                limit: 10,
                module: Some("process"),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "process_action");
        assert_eq!(events[0].category, "action");
        assert_eq!(events[0].metadata["action"], "oom");
        assert_eq!(events[0].metadata["applied"], 300);
    }
}
```

---

回复 **“继续”** 我发 **第 4 批**（APK 客户端与事件/模型层：`SysFlowClient.kt`、`EventCopy.kt`、`EventUi.kt`、`ProcessModels.kt` 完整文件）。

---

## You

继续

---

## AI Studio

### 第 4 批（4 个文件）：`SysFlowClient.kt`、`EventCopy.kt`、`EventUi.kt`、`ProcessModels.kt`

#### 1. `app/src/main/kotlin/io/sysflow/core/root/SysFlowClient.kt`

```kotlin
package io.sysflow.core.root

import io.sysflow.core.model.ApplyResult
import io.sysflow.core.model.CapsData
import io.sysflow.core.model.CheckData
import io.sysflow.core.model.Envelope
import io.sysflow.core.model.ErrorCode
import io.sysflow.core.model.ProtocolError
import io.sysflow.core.model.ModuleEventPage
import io.sysflow.core.model.SafeStatus
import io.sysflow.core.model.Schema
import io.sysflow.core.model.SnapshotData
import io.sysflow.core.model.VersionData
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject

sealed class SysFlowException(message: String) : Exception(message) {
    class Unsupported(message: String) : SysFlowException(message)
    class Permission(message: String) : SysFlowException(message)
    class InvalidArg(message: String) : SysFlowException(message)
    class Busy(message: String) : SysFlowException(message)
    class Io(message: String) : SysFlowException(message)
    class SafeMode(message: String) : SysFlowException(message)
    class ProtocolMismatch(message: String) : SysFlowException(message)
    class NoBinary(message: String) : SysFlowException(message)
}

/**
 * Thin, typed facade over the `sysflow` CLI. The APK never touches sysfs directly: every read and
 * write goes through this class and the binary's JSON envelope.
 */
class SysFlowClient(private val root: RootExecutor) {

    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        isLenient = true
    }
    private val writeMutex = Mutex()
    @Volatile
    private var binary: String? = null

    suspend fun hasRoot(): Boolean = root.hasAccess()

    suspend fun version(): VersionData = parseEnvelope(run("version"))

    suspend fun caps(): CapsData = parseEnvelope(run("caps"))

    suspend fun snapshot(): SnapshotData = parseEnvelope(run("snapshot"))

    /** Reads the module-owned event journal. Module/category values intentionally remain open strings. */
    suspend fun events(
        limit: Int = 200,
        afterTimestampMs: Long? = null,
        afterId: String? = null,
        beforeTimestampMs: Long? = null,
        beforeId: String? = null,
        ascending: Boolean = false,
    ): ModuleEventPage {
        val args = buildList {
            add("event"); add("list"); add("--limit"); add(limit.coerceIn(1, 500).toString())
            afterTimestampMs?.let { add("--after-ms"); add(it.toString()) }
            afterId?.let { add("--after-id"); add(it) }
            beforeTimestampMs?.let { add("--before-ms"); add(it.toString()) }
            beforeId?.let { add("--before-id"); add(it) }
            if (ascending) add("--ascending")
        }
        return parseEnvelope(run(*args.toTypedArray()))
    }

    /**
     * Delete every event in the module-owned journal.
     *
     * This only clears the Root journal; the APK's Room mirror keeps its own copy, so callers must
     * also clear the local cache or the list would appear unchanged.
     */
    suspend fun clearEvents(): JsonObject = writeMutex.withLock {
        // Every mutation shares this mutex so a clear cannot interleave with a concurrent apply.
        parseEnvelope<JsonObject>(run("event", "clear", "--all")).jsonObject
    }

    /**
     * Run a command and return the *raw* stdout without envelope parsing.
     *
     * Used by the diagnostics sheet on the events screen: when the app and a terminal disagree about
     * the journal, the only way to settle it is to see exactly what this process executes and what
     * comes back.
     */
    suspend fun raw(command: List<String>): String = try {
        val bin = resolveBinary()
        val line = (listOf(bin) + command).joinToString(" ") { quote(it) }
        val result = root.exec(line)
        "\$ $line\nexit=${result.exitCode}\n${result.stdout}"
            .let { if (result.stderr.isBlank()) it else "$it\nstderr: ${result.stderr}" }
    } catch (cause: Exception) {
        "failed: ${cause::class.simpleName}: ${cause.message}"
    }

    suspend fun info(): JsonObject = parseEnvelope<JsonObject>(run("info")).jsonObject

    /**
     * Cycle count and health with a vendor fallback (Samsung keeps its counter outside sysfs).
     * On demand because the fallback shells out to `dumpsys batteryproperties`.
     */
    suspend fun batteryHealth(refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("battery"); add("health")
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** Recorded history series for a module (`step` accepts `60`, `5m`, `1h`). */
    suspend fun history(module: String, since: String = "24h", step: String = "1h"): JsonObject {
        val args = buildList {
            add("history"); add(module); add("--since"); add(since); add("--step"); add(step)
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * Per-UID power attribution from BatteryStats. Expensive (a `dumpsys batterystats` run behind a
     * 60 s module-side cache), so the battery page pulls it on demand instead of every snapshot.
     */
    suspend fun batteryApps(limit: Int = 30, refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("battery"); add("apps"); add("--limit"); add(limit.coerceAtLeast(0).toString())
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * Per-UID memory ranking (protocol §3.23). Expensive — it opens `/proc` per process — so the
     * module caches it for a minute and the page pulls it on demand.
     */
    suspend fun memApps(limit: Int = 10, refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("mem"); add("apps"); add("--limit"); add(limit.coerceAtLeast(1).toString())
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * System-vs-user memory split (protocol §3.24). Measures PSS for every process, so it is the
     * heaviest read-only call in the app and is always pulled on demand.
     */
    suspend fun memBreakdown(refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("mem"); add("breakdown")
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * Live process list (protocol §3.25). Expensive (a two-pass `/proc` scan), so it is a verb and
     * never part of `snapshot`; the page calls it only while visible.
     *
     * @param sampleMs CPU sampling window, 200–2000 ms. 300 ms keeps the 2 s page poll responsive;
     *   the pull-to-refresh path uses a longer window for a steadier number.
     */
    suspend fun processList(
        limit: Int = 80,
        sort: String = "cpu",
        filter: String = "all",
        query: String? = null,
        sampleMs: Int = 300,
        foreground: Boolean = true,
    ): JsonObject {
        val args = buildList {
            add("process"); add("list")
            add("--limit"); add(limit.coerceIn(1, 200).toString())
            add("--sort"); add(sort)
            add("--filter"); add(filter)
            add("--sample-ms"); add(sampleMs.coerceIn(200, 2000).toString())
            if (!foreground) add("--no-foreground")
            if (!query.isNullOrBlank()) {
                add("--query"); add(query.trim())
            }
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** One process in detail (protocol §3.25); includes PSS and scheduler/cgroup fields. */
    suspend fun processGet(pid: Int): JsonObject =
        parseEnvelope<JsonObject>(run("process", "get", pid.toString())).jsonObject

    /** Force-stop a package (protocol §3.25). `user` is only needed for multi-user ambiguity. */
    suspend fun processStop(target: String, user: Int? = null): JsonObject {
        val args = buildList {
            add("process"); add("stop"); add(target)
            user?.let { add("--user"); add(it.toString()) }
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** Set `oom_score_adj` (`-1000..=1000`), high risk and never persisted. */
    suspend fun processOom(target: String, adjustment: Int): JsonObject =
        parseEnvelope<JsonObject>(
            run("process", "oom", target, adjustment.coerceIn(-1000, 1000).toString()),
        ).jsonObject

    /** Set nice (`-20..=19`); the module uses `setpriority(2)` first. */
    suspend fun processRenice(target: String, nice: Int): JsonObject =
        parseEnvelope<JsonObject>(
            run("process", "renice", target, nice.coerceIn(-20, 19).toString()),
        ).jsonObject

    /** Set scheduling policy (`other|fifo|rr|batch|idle`) and optional RT priority (`1..=99`). */
    suspend fun processSched(target: String, policy: String, priority: Int? = null): JsonObject {
        val args = buildList {
            add("process"); add("sched"); add(target); add(policy)
            priority?.let { add(it.toString()) }
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** Move every process of the target into an existing `/dev/cpuset/<group>` (experimental). */
    suspend fun processCpuset(target: String, group: String): JsonObject =
        parseEnvelope<JsonObject>(run("process", "cpuset", target, group)).jsonObject

    /** Freeze / unfreeze (experimental; cgroup v2/v1 first, `am freeze` as fallback). */
    suspend fun processFreeze(target: String, frozen: Boolean): JsonObject =
        parseEnvelope<JsonObject>(
            run("process", if (frozen) "freeze" else "unfreeze", target),
        ).jsonObject

    /** Live storage throughput (protocol §3.26); needs an interval, so it is a verb. */
    suspend fun storageIo(intervalMs: Int = 1_000): JsonObject {
        val args = buildList {
            add("storage"); add("io")
            add("--interval-ms"); add(intervalMs.coerceIn(300, 5_000).toString())
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** Bounded directory-size ranking; never part of `snapshot` (it walks the tree). */
    suspend fun storageUsage(
        path: String = "/data",
        limit: Int = 20,
        budgetMs: Int = 3_000,
    ): JsonObject {
        val args = buildList {
            add("storage"); add("usage")
            add("--path"); add(path)
            add("--limit"); add(limit.coerceIn(1, 100).toString())
            add("--budget-ms"); add(budgetMs.coerceIn(100, 30_000).toString())
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /** Whole-device mount overview (protocol §3.26): one row per real filesystem. */
    suspend fun storagePartitions(): JsonObject =
        parseEnvelope<JsonObject>(run("storage", "partitions")).jsonObject

    /** One-shot `fstrim -v /data`. */
    suspend fun storageTrim(path: String = "/data"): JsonObject =
        parseEnvelope<JsonObject>(run("storage", "trim", "--path", path)).jsonObject

    /**
     * Userspace wake locks currently held (protocol §3.19).
     *
     * Expensive: it enters the Java system server through `dumpsys power`, so the module caches the
     * answer for a minute and the page pulls it on demand instead of every snapshot.
     */
    suspend fun wakelockHeld(refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("wakelock"); add("held")
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * The last wakeup reason (protocol §3.21).
     *
     * Same cost as [wakelockHeld]: one `dumpsys power` behind the module's 60 s cache.
     */
    suspend fun wakelockWakeup(refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("wakelock"); add("wakeup")
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    /**
     * Per-UID wake-lock time since the last charge (protocol §3.20).
     *
     * Expensive: a `dumpsys batterystats --charged` run, cached for a minute by the module.
     */
    suspend fun wakelockApps(limit: Int = 20, refresh: Boolean = false): JsonObject {
        val args = buildList {
            add("wakelock"); add("apps"); add("--limit"); add(limit.coerceAtLeast(0).toString())
            if (refresh) add("--refresh")
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    suspend fun schema(module: String): Schema = parseEnvelope(run(module, "schema"))

    suspend fun get(module: String, keys: List<String> = emptyList()): JsonObject {
        val args = buildList {
            add(module)
            add("get")
            if (keys.isNotEmpty()) {
                add("--keys")
                add(keys.joinToString(","))
            }
        }
        return parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    suspend fun set(module: String, payload: JsonObject, persist: Boolean): ApplyResult =
        writeMutex.withLock {
            val args = buildList {
                add(module)
                add("set")
                add(payload.toString())
                if (persist) add("--persist")
            }
            parseEnvelope(run(*args.toTypedArray()))
        }

    suspend fun reset(module: String): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run(module, "reset")).jsonObject
    }

    /**
     * Re-seed the stored defaults for CPU from the kernel's own hardware floor.
     *
     * The defaults are what 恢复默认 restores, and they are recorded once and never refreshed, so a
     * baseline captured before a kernel/ROM update can leave the app restoring a minimum frequency
     * the hardware no longer honours. Only defaults change here — the live values are untouched.
     */
    suspend fun cpuRebaseline(): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run("cpu", "rebaseline")).jsonObject
    }

    suspend fun check(): CheckData = parseEnvelope(run("check"))

    suspend fun daemonStatus(): JsonObject = parseEnvelope<JsonObject>(run("daemon", "status")).jsonObject

    suspend fun daemonConfigure(enabled: Boolean? = null, intervalSeconds: Int? = null): JsonObject = writeMutex.withLock {
        val args = buildList {
            add("daemon")
            add("configure")
            enabled?.let { add("--enabled"); add(it.toString()) }
            intervalSeconds?.let { add("--interval"); add(it.toString()) }
        }
        parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    suspend fun bootStatus(): JsonObject = parseEnvelope<JsonObject>(run("boot", "status")).jsonObject

    suspend fun bootConfigure(enabled: Boolean? = null, delaySeconds: Int? = null): JsonObject = writeMutex.withLock {
        val args = buildList {
            add("boot")
            add("configure")
            enabled?.let { add("--enabled"); add(it.toString()) }
            delaySeconds?.let { add("--delay"); add(it.toString()) }
        }
        parseEnvelope<JsonObject>(run(*args.toTypedArray())).jsonObject
    }

    suspend fun logTail(lines: Int = 200): JsonObject =
        parseEnvelope<JsonObject>(run("log", "--tail", lines.coerceIn(1, 1000).toString())).jsonObject

    suspend fun resetAll(): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run("reset", "--all")).jsonObject
    }

    suspend fun safeStatus(): SafeStatus = parseEnvelope(run("safe-mode", "status"))

    suspend fun safeClear(): SafeStatus = writeMutex.withLock {
        parseEnvelope(run("safe-mode", "clear"))
    }

    suspend fun recoveryStatus(): JsonObject = parseEnvelope<JsonObject>(run("recovery", "status")).jsonObject

    suspend fun recoveryRestore(): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run("recovery", "restore")).jsonObject
    }

    suspend fun recoveryClear(): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run("recovery", "clear")).jsonObject
    }

    suspend fun recoveryClearCorrupt(): JsonObject = writeMutex.withLock {
        parseEnvelope<JsonObject>(run("recovery", "clear-corrupt")).jsonObject
    }

    /** Check the protocol before any screen trusts the payload. */
    suspend fun verifyProtocol(): VersionData {
        val v = version()
        if (v.protocol != io.sysflow.core.model.PROTOCOL_VERSION) {
            throw SysFlowException.ProtocolMismatch(
                "协议版本不匹配：模块 ${v.protocol}，应用 ${io.sysflow.core.model.PROTOCOL_VERSION}。请更新模块或应用。",
            )
        }
        return v
    }

    private suspend fun run(vararg args: String): String {
        val bin = resolveBinary()
        val command = (listOf(bin) + args).joinToString(" ") { quote(it) }
        val result = root.exec(command)
        val stdout = result.stdout.trim()
        if (stdout.isEmpty()) {
            val detail = result.stderr.trim()
            if (detail.contains("not found", ignoreCase = true) || detail.contains("No such file")) {
                throw SysFlowException.NoBinary("找不到 sysflow 二进制，请重新安装模块。")
            }
            if (detail.contains("permission", ignoreCase = true) ||
                detail.contains("Permission denied", ignoreCase = true) ||
                detail.contains("root", ignoreCase = true)
            ) {
                throw SysFlowException.Permission("没有 root 权限，请在 KernelSU / APatch 中授权。")
            }
            throw SysFlowException.Io(detail.ifEmpty { "sysflow 退出码 ${result.exitCode}" })
        }
        return stdout
    }

    private suspend fun resolveBinary(): String {
        binary?.let { return it }
        for (candidate in BIN_CANDIDATES) {
            val probe = if (candidate.startsWith('/')) {
                "[ -x ${quote(candidate)} ]"
            } else {
                "command -v ${quote(candidate)}"
            }
            val result = runCatching { root.exec(probe) }.getOrNull() ?: continue
            if (result.exitCode == 0) {
                binary = candidate
                return candidate
            }
        }
        throw SysFlowException.NoBinary(
            "找不到 sysflow 二进制。请确认模块已安装（/data/adb/modules/sysflow/bin/sysflow）。",
        )
    }

    private inline fun <reified T> parseEnvelope(text: String): T {
        val envelope: Envelope<T> = try {
            json.decodeFromString(text)
        } catch (e: Exception) {
            throw SysFlowException.Io("sysflow 返回了无法解析的输出：${e.message}")
        }
        if (!envelope.ok) {
            throw envelope.error.toException()
        }
        return envelope.data ?: throw SysFlowException.Io("sysflow 返回了空数据。")
    }

    private fun ProtocolError?.toException(): SysFlowException {
        val error = this ?: return SysFlowException.Io("未知错误")
        return when (error.code) {
            ErrorCode.Unsupported -> SysFlowException.Unsupported(error.msg)
            ErrorCode.Permission -> SysFlowException.Permission(error.msg)
            ErrorCode.InvalidArg -> SysFlowException.InvalidArg(error.msg)
            ErrorCode.Busy -> SysFlowException.Busy(error.msg)
            ErrorCode.Io -> SysFlowException.Io(error.msg)
            ErrorCode.SafeMode -> SysFlowException.SafeMode(error.msg)
        }
    }

    private fun quote(value: String): String = "'" + value.replace("'", "'\\''") + "'"

    companion object {
        private val BIN_CANDIDATES = listOf(
            "/data/adb/modules/sysflow/bin/sysflow",
            "sysflow",
        )
    }
}
```

---

#### 2. `app/src/main/kotlin/io/sysflow/feature/home/EventCopy.kt`

```kotlin
package io.sysflow.feature.home

import io.sysflow.core.event.SystemEventEntity
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlin.math.abs

/**
 * Presentation-only mapping for the overview activity feed, aligned with WebUI `event-copy.ts`.
 *
 * The event protocol is open-world: producers may use any module id, category and event type.
 * Nothing here may hide an unknown event — every helper falls back to the producer's own title and
 * message, so a future module still renders even when none of these tables know it.
 */

enum class ModuleTone { CPU, MEMORY, POWER, NEUTRAL }

data class ActivityEntry(
    val event: SystemEventEntity,
    /** Module-free event name, because the card header already names the module. */
    val label: String,
    val detail: String,
    /** How many identical records this row stands for. */
    val count: Int,
)

/** One module section of the overview: only modules that actually produced events appear. */
data class ModuleActivity(
    val module: String,
    val label: String,
    val tone: ModuleTone,
    val entries: List<ActivityEntry>,
    /** Events of this module inside the window that was fetched. */
    val recentCount: Int,
    val latestMs: Long,
)

private val moduleLabels = mapOf(
    "cpu" to "CPU 管理",
    "system" to "系统服务",
    "display" to "显示",
    "battery" to "电池管理",
    "memory" to "内存管理",
    "mem" to "内存管理",
    "storage" to "储存管理",
    "io" to "储存管理",
    "process" to "进程管理",
    "proc" to "进程管理",
    "wakelock" to "唤醒锁",
    "mock" to "模拟模块",
)

private val moduleTones = mapOf(
    "cpu" to ModuleTone.CPU,
    "memory" to ModuleTone.MEMORY,
    "mem" to ModuleTone.MEMORY,
    "storage" to ModuleTone.MEMORY,
    "io" to ModuleTone.MEMORY,
    "display" to ModuleTone.MEMORY,
    "system" to ModuleTone.POWER,
    "battery" to ModuleTone.POWER,
    "wakelock" to ModuleTone.POWER,
)

/** Short Chinese event names, keyed by the stable machine identifier. */
private val eventLabels = mapOf(
    "wakelock_long_held" to "唤醒锁持有过久",
    "mem_pressure_high" to "内存压力偏高",
    "mem_pressure_recovered" to "内存压力已恢复",
    "mem_oom_kill" to "内核内存不足",
    "mem_setting_contested" to "内存设置被反复改回",
    "storage_low_space" to "储存空间不足",
    "storage_space_recovered" to "储存空间已恢复",
    "settings_applied" to "设置已更新",
    "settings_partially_applied" to "设置部分生效",
    "settings_apply_failed" to "设置写入失败",
    "boot_settings_applied" to "开机设置已应用",
    "boot_apply_partial" to "开机设置部分生效",
    "boot_apply_failed" to "开机设置应用失败",
    "boot_apply_crashed" to "开机设置应用失败",
    "module_service_started" to "模块服务已启动",
    "boot_config_updated" to "开机策略已更新",
    "boot_confirmed" to "开机自检通过",
    "boot_reset_failed" to "开机前清理失败",
    "module_reset" to "已恢复默认设置",
    "module_reset_partial" to "部分恢复默认设置",
    "module_reset_failed" to "恢复默认设置失败",
    "stale_settings_dropped" to "残留设置已丢弃",
    "drift_repaired" to "配置漂移已修复",
    "drift_repair_incomplete" to "配置漂移未修复完整",
    "drift_repair_failed" to "配置漂移修复失败",
    "drift_check_failed" to "配置漂移检查失败",
    "enforcement_validation_failed" to "读回校验未通过",
    "daemon_enforcement_failed" to "守护进程执行失败",
    "adaptive_tick_failed" to "自适应采样失败",
    "thermal_protection_transition" to "温控保护变更",
    "battery_saving_transition" to "省电策略变更",
    "psi_guard_transition" to "内存压力护栏变更",
    "screen_off_profile_released" to "熄屏限制已关闭",
    "screen_off_restore_failed" to "熄屏限制关闭失败",
    "charging_started" to "开始充电",
    "charging_stopped" to "停止充电",
    "fully_charged" to "电池已充满",
    "low_battery" to "电量偏低",
    "battery_temp_high" to "电池温度偏高",
    "screen_turned_on" to "屏幕已点亮",
    "screen_turned_off" to "屏幕已熄灭",
    "walt_guard_boot_applied" to "WALT 护栏已恢复",
    "walt_guard_boot_failed" to "WALT 护栏恢复失败",
    "walt_guard_boot_mismatch" to "WALT 护栏未对齐",
    "runtime_state_mismatch" to "运行时状态不一致",
    "daemon_started" to "守护服务已启动",
    "daemon_stopped" to "守护服务已停止",
    "daemon_config_updated" to "守护间隔已更新",
    "safe_mode_entered" to "已进入安全模式",
    "safe_mode_cleared" to "已退出安全模式",
    "corrupt_config_cleared" to "损坏配置已清理",
    "recovery_restored" to "隔离配置已恢复",
    "recovery_discarded" to "隔离配置已丢弃",
    "process_action" to "进程操作",
)

/** Two identical records closer together than this are the same occurrence, reported twice. */
private const val DUPLICATE_WINDOW_MS = 10 * 60 * 1000L

fun moduleLabel(module: String): String =
    moduleLabels[module.lowercase()] ?: module.uppercase()

fun moduleTone(module: String): ModuleTone =
    moduleTones[module.lowercase()] ?: ModuleTone.NEUTRAL

/** The event's own name, falling back to the producer's wording for unknown types. */
fun activityLabel(event: SystemEventEntity): String =
    eventLabels[event.eventType.lowercase()] ?: event.title

/** `CPU 管理 · 设置已更新` — used where a row has no module heading above it. */
fun activityTitle(event: SystemEventEntity): String {
    val label = activityLabel(event)
    return if (label.isBlank()) moduleLabel(event.module) else "${moduleLabel(event.module)} · $label"
}

/** A single Chinese line explaining what happened, or the producer's message when unknown. */
fun activityDetail(event: SystemEventEntity): String {
    val meta = parseMetadata(event.metadataJson)
    val type = event.eventType.lowercase()
    val keys = meta.listPhrase("keys")
    val mismatch = meta.listPhrase("mismatch")
    val drift = meta.listPhrase("drift")
    val cap = meta.number("cap_percent")
    val level = meta.number("level") ?: meta.number("previous_level")
    val persist = (meta["persist"] as? JsonPrimitive)?.booleanOrNull

    return when (type) {
        "settings_applied" -> when {
            keys != null -> {
                val prefix = if (persist == false) "本次生效" else "已持久化"
                "$prefix：$keys"
            }
            persist == false -> "本次生效，重启后失效"
            else -> "已写入并读回校验"
        }
        "settings_partially_applied" -> mismatch?.let { "未对齐：$it" } ?: "部分设置未能生效"
        "settings_apply_failed" -> "写入失败，请查看模块日志"
        "boot_settings_applied" -> keys?.let { "开机后已恢复：$it" } ?: "开机后已自动恢复保存的设置"
        "boot_apply_partial" -> mismatch?.let { "开机后有未对齐项：$it" } ?: "开机应用后仍有未对齐项"
        "boot_apply_failed", "boot_apply_crashed" -> "开机自动应用未完成，请查看模块日志"
        "module_service_started" -> "Root 模块服务已进入开机后阶段"
        "boot_config_updated" -> "开机自动应用策略已更新"
        "boot_confirmed" -> "模块确认本次开机稳定"
        "boot_reset_failed" -> "开机前的清理未完成"
        "module_reset" -> "模块设置已回到默认值"
        "module_reset_partial" -> "部分设置未能回到默认值"
        "module_reset_failed" -> "恢复默认设置失败，请查看模块日志"
        "stale_settings_dropped" -> "无法使用的持久化设置已被丢弃"
        "drift_repaired" -> drift?.let { "已自动修复：$it" } ?: "检测到设置漂移，已自动修复"
        "drift_repair_incomplete" -> drift?.let { "仍有设置未修复：$it" } ?: "仍有设置未修复"
        "drift_repair_failed", "daemon_enforcement_failed" -> "守护进程修复未完成，请查看模块日志"
        "drift_check_failed" -> "本轮漂移检查未完成"
        "enforcement_validation_failed" -> "读回校验未通过，本次未执行"
        "adaptive_tick_failed" -> "自适应采样失败，本轮已跳过"
        "thermal_protection_transition" -> when {
            level != null && level > 0 ->
                cap?.let { "温度升高，已收紧至 ${it.roundPercent()}%" } ?: "温度升高，已触发温控保护"
            else -> "温度回落，温控保护已解除"
        }
        "battery_saving_transition" -> when {
            level != null && level > 0 ->
                cap?.let { "电量偏低，已收紧至 ${it.roundPercent()}%" } ?: "电量偏低，已启用省电限制"
            else -> "省电限制已解除"
        }
        "psi_guard_transition" -> if (level != null && level > 0) {
            "内存压力升高，已启用护栏"
        } else {
            "内存压力回落，护栏已解除"
        }
        "charging_started" -> "电源已接入，开始充电"
        "charging_stopped" -> "电源已断开或充电结束"
        "fully_charged" -> "电池已充至 100%"
        "low_battery" -> (meta.number("battery_percent")?.let { "电量已降至 ${it.roundPercent()}%" }
            ?: "电量偏低，建议尽快充电")
        "battery_temp_high" -> (meta.number("temperature_c")?.let { "电池温度达到 ${it.roundPercent()} °C" }
            ?: "电池温度偏高，建议暂停充电")
        "screen_off_profile_released" -> "熄屏限制已关闭，CPU 恢复正常上限"
        "screen_off_restore_failed" -> "熄屏限制关闭失败，可能仍占用核心"
        "screen_turned_on" -> "显示进入点亮状态"
        "screen_turned_off" -> "显示进入熄灭状态"
        "walt_guard_boot_applied" -> "开机后 WALT 护栏已恢复到原始值"
        "wakelock_long_held" -> if (meta.string("kind") == "userspace_lock") {
            val tag = meta.string("tag") ?: "唤醒锁"
            "「$tag」已被持有超过 30 分钟"
        } else {
            val name = meta.string("name") ?: "未知"
            "唤醒源「$name」持续活跃超过 30 分钟"
        }
        "walt_guard_boot_failed" -> "开机后 WALT 护栏未能写入"
        "walt_guard_boot_mismatch" -> "开机后 WALT 护栏与原始值不一致"
        "runtime_state_mismatch" -> "运行时状态与设置不一致"
        "daemon_started" -> "后台守护进程已开始运行"
        "daemon_stopped" -> "后台守护进程已停止"
        "daemon_config_updated" -> meta.number("interval_s")
            ?.let { "采样间隔已设为 ${it.roundPercent()} 秒" } ?: "守护间隔已更新"
        "safe_mode_entered" -> "调节功能受限，等待人工确认"
        "safe_mode_cleared" -> "已恢复正常调节"
        "corrupt_config_cleared" -> "损坏的配置备份已删除"
        "recovery_restored" -> "隔离的配置已恢复使用"
        "recovery_discarded" -> "隔离的配置已丢弃"
        "storage_low_space" -> meta.number("used_pct")
            ?.let { "存储空间已使用 ${it.roundPercent()}%，请清理空间" }
            ?: "存储空间不足，请清理空间"
        "storage_space_recovered" -> meta.number("used_pct")
            ?.let { "存储空间使用率已回落到 ${it.roundPercent()}%" }
            ?: "存储空间已恢复"
        "mem_pressure_high" -> "内存压力偏高，已持续 15 分钟" + psiDetail(meta)
        "mem_pressure_recovered" -> "内存压力已恢复正常" + psiDetail(meta)
        "mem_oom_kill" -> meta.number("count")?.roundPercent()?.takeIf { it > 0 }
            ?.let { "内核因内存不足杀死了 $it 个进程" }
            ?: "内核因内存不足杀死了进程"
        "mem_setting_contested" -> {
            // The module gave up on this value: something else keeps resetting it, so SysFlow stops
            // fighting instead of rewriting it every few seconds.
            val name = when (meta.string("item")) {
                "mem.swappiness" -> "swappiness"
                "mem.vfs_cache_pressure" -> "vfs_cache_pressure"
                else -> "内存参数"
            }
            val live = meta.number("live")?.roundPercent()?.let { " $it" } ?: ""
            val hours = meta.number("suppress_s")?.div(3600.0)?.roundPercent()?.takeIf { it > 0 } ?: 6L
            "系统反复把 $name 改回$live，已暂停重新应用 $hours 小时"
        }
        "process_action" -> processActionDetail(meta, event.message)
        else -> event.message
    }
}

/**
 * Collapses identical consecutive records into one row that carries a count. Producers re-emit the
 * same observation on every tick, and the overview should not repeat it.
 */
private fun collapseEntries(events: List<SystemEventEntity>): List<ActivityEntry> {
    val collapsed = mutableListOf<ActivityEntry>()
    for (event in events) {
        val previous = collapsed.lastOrNull()
        if (
            previous != null &&
            previous.event.module == event.module &&
            previous.event.eventType == event.eventType &&
            previous.event.title == event.title &&
            previous.event.message == event.message &&
            abs(previous.event.timestampMs - event.timestampMs) <= DUPLICATE_WINDOW_MS
        ) {
            collapsed[collapsed.lastIndex] = previous.copy(count = previous.count + 1)
            continue
        }
        collapsed += ActivityEntry(
            event = event,
            label = activityLabel(event),
            detail = activityDetail(event),
            count = 1,
        )
    }
    return collapsed
}

/**
 * Groups the fetched window into one section per module, newest module first.
 *
 * The module list is derived from the events themselves, never from a registry: a module appears as
 * soon as it emits its first event, and disappears when the local cache no longer holds any of its
 * records. Modules are therefore open-world, exactly like the protocol.
 */
fun buildModuleActivity(
    events: List<SystemEventEntity>,
    perModule: Int = 3,
): List<ModuleActivity> {
    if (events.isEmpty()) return emptyList()
    val previews = LinkedHashMap<String, MutableList<ActivityEntry>>()
    val totals = LinkedHashMap<String, Int>()
    val latest = LinkedHashMap<String, Long>()
    for (entry in collapseEntries(events)) {
        val module = entry.event.module
        totals[module] = (totals[module] ?: 0) + 1
        latest.getOrPut(module) { entry.event.timestampMs }
        val shown = previews.getOrPut(module) { mutableListOf() }
        // Two lines reading exactly the same tell the reader nothing, so only distinct previews show.
        val duplicate = shown.any { it.label == entry.label && it.detail == entry.detail }
        if (!duplicate && shown.size < perModule) shown += entry
    }
    return previews.map { (module, entries) ->
        ModuleActivity(
            module = module,
            label = moduleLabel(module),
            tone = moduleTone(module),
            entries = entries,
            recentCount = totals[module] ?: entries.size,
            latestMs = latest[module] ?: entries.first().event.timestampMs,
        )
    }.sortedByDescending { it.latestMs }
}

private val metadataParser = Json { ignoreUnknownKeys = true; isLenient = true }

private fun parseMetadata(metadataJson: String): JsonObject {
    val trimmed = metadataJson.trim()
    if (trimmed.isEmpty() || trimmed == "{}" || trimmed == "null") return JsonObject(emptyMap())
    return runCatching { metadataParser.parseToJsonElement(trimmed) }.getOrNull() as? JsonObject
        ?: JsonObject(emptyMap())
}

private fun JsonObject.string(key: String): String? =
    (this[key] as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }

private fun JsonObject.number(key: String): Double? =
    (this[key] as? JsonPrimitive)?.doubleOrNull

/** `a、b 等 3 项` — bounded so a long key list cannot overflow the row. */
private fun JsonObject.listPhrase(key: String): String? {
    val array = this[key] as? JsonArray ?: return null
    val items = array.mapNotNull { (it as? JsonPrimitive)?.contentOrNull?.takeIf(String::isNotBlank) }
    if (items.isEmpty()) return null
    return if (items.size <= 2) items.joinToString("、") else "${items[0]}、${items[1]} 等 ${items.size} 项"
}

private fun Double.roundPercent(): Long = kotlin.math.round(this).toLong()

/**
 * "（PSI 3%，可用 4.6 GB）" for the memory-pressure events.
 *
 * `psi_full_pct` arrives in hundredths of a percent (protocol §3.22), so 300 is 3 %.
 */
private fun psiDetail(meta: JsonObject): String {
    val parts = mutableListOf<String>()
    meta.number("psi_full_pct")?.div(100.0)?.roundPercent()?.let { parts.add("PSI $it%") }
    meta.number("available_kb")?.div(1_048_576.0)?.roundPercent()?.let { parts.add("可用 $it GB") }
    meta.number("swap_used_kb")?.div(1_048_576.0)?.roundPercent()?.let { parts.add("swap $it GB") }
    return if (parts.isEmpty()) "" else "（${parts.joinToString("，")}）"
}

private fun processActionDetail(meta: JsonObject, fallback: String): String {
    val action = meta.string("action") ?: return fallback
    val target = meta.string("target") ?: return fallback
    val verified = (meta["verified"] as? JsonPrimitive)?.booleanOrNull
    val suffix = if (verified == false) "（未完全生效）" else ""
    return when (action) {
        "stop" -> "强制停止 $target$suffix"
        "oom" -> meta.number("applied")
            ?.let { "调整 $target 的 oom_score_adj 为 ${it.roundPercent()}$suffix" }
            ?: "调整 $target 的 oom_score_adj$suffix"
        "renice" -> meta.number("applied")
            ?.let { "调整 $target 的 nice 为 ${it.roundPercent()}$suffix" }
            ?: "调整 $target 的 nice$suffix"
        "sched" -> {
            val policy = meta.string("policy") ?: "调度策略"
            val prio = meta.number("priority")?.roundPercent()
            if (prio != null && prio > 0) "设置 $target 为 $policy（优先级 $prio）$suffix"
            else "设置 $target 为 $policy$suffix"
        }
        "cpuset" -> meta.string("group")
            ?.let { "将 $target 移入 cpuset $it$suffix" }
            ?: "调整 $target 的 cpuset$suffix"
        "freeze" -> "冻结 $target$suffix"
        "unfreeze" -> "解冻 $target$suffix"
        else -> fallback
    }
}
```

---

#### 3. `app/src/main/kotlin/io/sysflow/feature/home/EventUi.kt`

```kotlin
package io.sysflow.feature.home

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AutoGraph
import androidx.compose.material.icons.rounded.BatteryFull
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Storage
import androidx.compose.material.icons.rounded.PowerSettingsNew
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.Thermostat
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import io.sysflow.core.ui.theme.sysFlowColors
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.longOrNull
import java.util.Locale

/** Shared icon, severity, and metadata presentation helpers aligned with WebUI `event-ui.ts`. */
fun eventCategoryIcon(category: String): ImageVector {
    val normalized = category.lowercase()
    return when {
        "thermal" in normalized || "temperature" in normalized -> Icons.Rounded.Thermostat
        "battery" in normalized -> Icons.Rounded.BatteryFull
        "power" in normalized -> Icons.Rounded.Bolt
        "pressure" in normalized -> Icons.Rounded.Timeline
        "safety" in normalized || "security" in normalized -> Icons.Rounded.Shield
        "config" in normalized -> Icons.Rounded.Tune
        "enforcement" in normalized -> Icons.Rounded.AutoGraph
        "lifecycle" in normalized || "service" in normalized || "daemon" in normalized ->
            Icons.Rounded.PowerSettingsNew
        else -> Icons.Rounded.Timeline
    }
}

/**
 * Module-level glyph for the overview cards. Derived from the event's own module id, so a module
 * that starts emitting events gets a card without any registry edit.
 */
/**
 * Icon container/tint for a module row. Severity still wins on individual event rows, so this is
 * only the module identity colour (teal CPU, blue memory/display/domain, amber system).
 */
@Composable
fun moduleColors(tone: ModuleTone): Pair<Color, Color> {
    val scheme = MaterialTheme.colorScheme
    val extended = MaterialTheme.sysFlowColors
    return when (tone) {
        ModuleTone.CPU ->
            scheme.secondaryContainer.copy(alpha = 0.72f) to scheme.onSecondaryContainer
        ModuleTone.MEMORY -> extended.infoContainer to extended.onInfoContainer
        ModuleTone.POWER -> extended.warningContainer to extended.onWarningContainer
        ModuleTone.NEUTRAL -> scheme.surfaceContainerHighest to scheme.onSurfaceVariant
    }
}

fun moduleIcon(module: String): ImageVector = when (module.lowercase()) {
    "cpu" -> Icons.Rounded.Memory
    "system" -> Icons.Rounded.Shield
    "display" -> Icons.Rounded.Bolt
    "battery" -> Icons.Rounded.BatteryFull
    "memory", "mem" -> Icons.Rounded.Dns
    "storage", "io" -> Icons.Rounded.Storage
    "process", "proc" -> Icons.Rounded.Timeline
    "wakelock" -> Icons.Rounded.Lock
    else -> Icons.Rounded.Timeline
}

fun eventSeverityLabel(severity: String): String = when (severity) {
    "notice" -> "提醒"
    "warning" -> "警告"
    "error" -> "错误"
    "critical" -> "严重"
    else -> "信息"
}

@Composable
fun eventSeverityColors(severity: String, neutralForIconBox: Boolean = false): Pair<Color, Color> {
    val scheme = MaterialTheme.colorScheme
    val extended = MaterialTheme.sysFlowColors
    return when (severity) {
        "critical", "error" -> scheme.errorContainer to scheme.onErrorContainer
        "warning" -> extended.warningContainer to extended.onWarningContainer
        "notice" -> extended.infoContainer to extended.onInfoContainer
        else -> if (neutralForIconBox) {
            scheme.secondaryContainer.copy(alpha = 0.72f) to scheme.onSecondaryContainer
        } else {
            scheme.surfaceContainerHighest to scheme.onSurfaceVariant
        }
    }
}

private val metadataJsonParser = Json { ignoreUnknownKeys = true; isLenient = true }

fun formatEventMetadata(metadataJson: String): String? {
    val trimmed = metadataJson.trim()
    if (trimmed.isEmpty() || trimmed == "{}" || trimmed == "null") return null
    val element = runCatching { metadataJsonParser.parseToJsonElement(trimmed) }.getOrNull()
        ?: return trimmed
    val obj = element as? JsonObject ?: return trimmed.takeIf { it != "[]" }
    if (obj.isEmpty()) return null

    val mismatchList = obj["mismatch"].stringList()
    val hasMismatch = mismatchList.isNotEmpty()
    val parts = mutableListOf<String>()
    val handled = mutableSetOf<String>()

    fun mark(key: String) {
        handled += key
    }

    obj["persist"]?.let { value ->
        mark("persist")
        (value as? JsonPrimitive)?.booleanOrNull?.let { persist ->
            parts += if (persist) "持久化: 是" else "持久化: 否（仅本次生效）"
        }
    }
    obj["keys"]?.let { value ->
        mark("keys")
        val items = value.stringList()
        if (items.isNotEmpty()) parts += "涉及项: ${items.joinToString(", ")}"
    }
    obj["drift"]?.let { value ->
        mark("drift")
        val items = value.stringList()
        if (items.isNotEmpty()) parts += "漂移项: ${items.joinToString(", ")}"
    }
    obj["mismatch"]?.let {
        mark("mismatch")
        if (mismatchList.isNotEmpty()) parts += "未对齐: ${mismatchList.joinToString(", ")}"
    }
    obj["dropped_keys"]?.let { value ->
        mark("dropped_keys")
        val items = value.stringList()
        if (items.isNotEmpty()) parts += "已丢弃: ${items.joinToString(", ")}"
    }
    obj["isolated_modules"]?.let { value ->
        mark("isolated_modules")
        val items = value.stringList()
        if (items.isNotEmpty()) parts += "已隔离模块: ${items.joinToString(", ")}"
    }
    obj["modules"]?.let { value ->
        mark("modules")
        val items = value.stringList()
        if (items.isNotEmpty()) parts += "模块: ${items.joinToString(", ")}"
    }
    obj["temperature_c"]?.let { value ->
        mark("temperature_c")
        (value as? JsonPrimitive)?.longOrNull?.takeIf { it >= 0 }?.let { temp ->
            parts += "温度: ${temp}°C"
        }
    }
    obj["sensor"]?.let { value ->
        mark("sensor")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { sensor ->
            parts += "传感器: $sensor"
        }
    }
    obj["battery_percent"]?.let { value ->
        mark("battery_percent")
        (value as? JsonPrimitive)?.longOrNull?.takeIf { it >= 0 }?.let { cap ->
            parts += "电量: ${cap}%"
        }
    }
    obj["cap_percent"]?.let { value ->
        mark("cap_percent")
        (value as? JsonPrimitive)?.longOrNull?.let { cut ->
            parts += "收紧幅度: ${cut}%"
        }
    }
    obj["psi_avg10"]?.let { value ->
        mark("psi_avg10")
        (value as? JsonPrimitive)?.doubleOrNull?.let { avg ->
            parts += String.format(Locale.US, "PSI avg10: %.2f", avg)
        }
    }
    obj["threshold"]?.let { value ->
        mark("threshold")
        (value as? JsonPrimitive)?.longOrNull?.let { threshold ->
            parts += "阈值: $threshold"
        }
    }
    obj["psi_floor_percent"]?.let { value ->
        mark("psi_floor_percent")
        (value as? JsonPrimitive)?.longOrNull?.takeIf { it > 0 }?.let { floor ->
            parts += "护栏上限: ${floor}%"
        }
    }
    obj["previous_floor_percent"]?.let { value ->
        mark("previous_floor_percent")
        (value as? JsonPrimitive)?.longOrNull?.takeIf { it > 0 }?.let { prev ->
            parts += "原护栏上限: ${prev}%"
        }
    }
    obj["level"]?.let { value ->
        mark("level")
        (value as? JsonPrimitive)?.longOrNull?.let { level ->
            parts += "档位: $level"
        }
    }
    obj["previous_level"]?.let { value ->
        mark("previous_level")
        (value as? JsonPrimitive)?.longOrNull?.let { prev ->
            parts += "原档位: $prev"
        }
    }
    obj["reason"]?.let { value ->
        mark("reason")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { reason ->
            parts += "原因: ${reasonLabel(reason)}"
        }
    }
    obj["error_code"]?.let { value ->
        mark("error_code")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { code ->
            parts += "错误码: $code"
        }
    }
    obj["interval_s"]?.let { value ->
        mark("interval_s")
        (value as? JsonPrimitive)?.longOrNull?.let { sec -> parts += "间隔: ${sec}s" }
    }
    obj["delay_s"]?.let { value ->
        mark("delay_s")
        (value as? JsonPrimitive)?.longOrNull?.let { sec -> parts += "延迟: ${sec}s" }
    }
    obj["boot_count"]?.let { value ->
        mark("boot_count")
        (value as? JsonPrimitive)?.longOrNull?.let { count -> parts += "开机计数: $count" }
    }
    obj["pid"]?.let { value ->
        mark("pid")
        (value as? JsonPrimitive)?.longOrNull?.let { pid -> parts += "PID: $pid" }
    }
    obj["action"]?.let { value ->
        mark("action")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { action ->
            parts += "动作: $action"
        }
    }
    obj["target"]?.let { value ->
        mark("target")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { target ->
            parts += "目标: $target"
        }
    }
    obj["mechanism"]?.let { value ->
        mark("mechanism")
        (value as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotBlank() }?.let { mechanism ->
            parts += "机制: $mechanism"
        }
    }
    obj["verified"]?.let { value ->
        mark("verified")
        (value as? JsonPrimitive)?.booleanOrNull?.let { verified ->
            parts += if (verified) "读回校验: 已生效" else "读回校验: 未完全生效"
        }
    }

    mark("requested")
    mark("applied")
    if (hasMismatch) {
        obj["requested"]?.takeIf { it !is JsonNull }?.let { parts += "请求: ${compactJsonValue(it)}" }
        obj["applied"]?.takeIf { it !is JsonNull }?.let { parts += "读回: ${compactJsonValue(it)}" }
    }

    obj.forEach { (key, value) ->
        if (key in handled || value is JsonNull) return@forEach
        val formatted = compactJsonValue(value)
        if (formatted.isNotEmpty() && formatted != "{}" && formatted != "[]") {
            parts += "$key: $formatted"
        }
    }

    return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
}

private fun reasonLabel(reason: String): String = when (reason) {
    "adaptive_disabled" -> "关闭自适应引擎"
    "module_reset" -> "恢复模块默认设置"
    "screen_off_disabled" -> "关闭熄屏限制"
    "stale_profile_cleanup" -> "清理残留熄屏状态"
    else -> reason
}

private fun JsonElement?.stringList(): List<String> = (this as? JsonArray)
    ?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull?.takeIf(String::isNotBlank) }
    .orEmpty()

private fun compactJsonValue(value: JsonElement): String = when (value) {
    is JsonPrimitive -> if (value.isString) value.content else value.toString()
    is JsonArray -> value.joinToString(", ", prefix = "[", postfix = "]") { compactJsonValue(it) }
    is JsonObject -> value.entries.joinToString(", ", prefix = "{", postfix = "}") { (k, v) ->
        "$k=${compactJsonValue(v)}"
    }
}
```

---

#### 4. `app/src/main/kotlin/io/sysflow/feature/process/ProcessModels.kt`

```kotlin
package io.sysflow.feature.process

import io.sysflow.core.model.CapsData
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import java.util.Locale

/**
 * Model layer for the process page.
 *
 * Pure functions over the protocol JSON (`docs/protocol.md` §3.25), so the screen never sees key
 * names and the parsing can be unit-tested without Android. Everything the module may omit is
 * nullable here: a kernel thread really has no package and no RSS.
 */

data class ProcessEntry(
    val pid: Int,
    val ppid: Int,
    val uid: Int,
    val user: Int,
    val appId: Int,
    val packageName: String?,
    val name: String,
    val cmdline: String,
    val state: String,
    val threads: Int,
    val rssKb: Long?,
    val cpuPct: Double?,
    val cpuTicks: Long?,
    val oomScoreAdj: Int?,
    val cgroup: String?,
    val cpuset: String?,
    val frozen: Boolean? = null,
    val foreground: Boolean,
    val system: Boolean,
    val protected: Boolean,
    val protectedReason: String?,
) {
    /** A label that is always printable, even for kernel threads. */
    val displayName: String get() = name.ifBlank { "pid $pid" }
}

data class ProcessForeground(
    val available: Boolean,
    val packageName: String?,
    val pid: Int?,
    val user: Int,
    val cached: Boolean,
    val disabled: Boolean,
)

data class ProcessList(
    val available: Boolean,
    val reason: String?,
    val total: Int,
    val returned: Int,
    val truncated: Boolean,
    val skipped: Int,
    val sampleMs: Long,
    val clockTicks: Long,
    /** Existing `/dev/cpuset` groups; empty hides the cpuset action. */
    val cpusetGroups: List<String>,
    val foreground: ProcessForeground,
    val entries: List<ProcessEntry>,
)

/** One `get` response: a superset of [ProcessEntry] plus PSS, scheduler and cgroup detail. */
data class ProcessDetail(
    val pid: Int,
    val ppid: Int,
    val uid: Int,
    val user: Int,
    val appId: Int,
    val packageName: String?,
    val packages: List<String>,
    val name: String,
    val cmdline: List<String>,
    val state: String,
    val threads: Int,
    val rssKb: Long?,
    val pssKb: Long?,
    val swapPssKb: Long?,
    val cpuPct: Double?,
    val sampleMs: Long,
    val oomScoreAdj: Int?,
    val nice: Int,
    val rtPriority: Int,
    val policy: Int,
    val policyName: String? = null,
    val cgroup: String?,
    val cpuset: String?,
    val startTimeTicks: Long,
    val frozen: Boolean? = null,
    val foreground: Boolean,
    val system: Boolean,
    val protected: Boolean,
    val protectedReason: String?,
)

/** One process affected by an action. `verified == null` means "not verifiable", not "failed". */
data class ProcessActionTarget(
    val pid: Int,
    val before: String?,
    val applied: String?,
    val verified: Boolean?,
    val error: String?,
    val mechanism: String?,
)

data class ProcessActionResult(
    val pkg: String?,
    val target: String?,
    val adjustment: Int?,
    val nice: Int?,
    val policy: String?,
    val policyId: Int?,
    val priority: Int?,
    val group: String?,
    val frozen: Boolean?,
    val mechanism: String?,
    val alreadyStopped: Boolean,
    val pidsBefore: List<Int>,
    val pidsAfter: List<Int>,
    val verified: Boolean?,
    val note: String?,
    val targets: List<ProcessActionTarget>,
)

/** Parsing of `sysflow caps`, kept here so the screen can hide actions the device lacks. */
fun processFeatures(caps: CapsData): Set<String> =
    caps.modules["process"]?.features?.toSet() ?: emptySet()

fun parseProcessList(json: JsonObject): ProcessList {
    val foreground = json.obj("foreground")
    return ProcessList(
        available = json.bool("available") ?: false,
        reason = json.string("reason"),
        total = json.int("total") ?: 0,
        returned = json.int("returned") ?: 0,
        truncated = json.bool("truncated") ?: false,
        skipped = json.int("skipped") ?: 0,
        sampleMs = json.long("sample_ms") ?: 0L,
        clockTicks = json.long("clock_ticks") ?: 100L,
        cpusetGroups = json.array("cpuset_groups").mapNotNull { (it as? JsonPrimitive)?.contentOrNull },
        foreground = ProcessForeground(
            available = foreground?.bool("available") ?: false,
            packageName = foreground?.string("package"),
            pid = foreground?.int("pid"),
            user = foreground?.int("user") ?: 0,
            cached = foreground?.bool("cached") ?: false,
            disabled = foreground?.bool("disabled") ?: false,
        ),
        entries = json.array("processes").mapNotNull { (it as? JsonObject)?.let(::parseProcessEntry) },
    )
}

fun parseProcessEntry(json: JsonObject): ProcessEntry = ProcessEntry(
    pid = json.int("pid") ?: 0,
    ppid = json.int("ppid") ?: 0,
    uid = json.int("uid") ?: 0,
    user = json.int("user") ?: 0,
    appId = json.int("app_id") ?: 0,
    packageName = json.string("package"),
    name = json.string("name").orEmpty(),
    cmdline = json.string("cmdline").orEmpty(),
    state = json.string("state").orEmpty(),
    threads = json.int("threads") ?: 0,
    rssKb = json.long("rss_kb"),
    cpuPct = json.double("cpu_pct"),
    cpuTicks = json.long("cpu_ticks"),
    oomScoreAdj = json.int("oom_score_adj"),
    cgroup = json.string("cgroup"),
    cpuset = json.string("cpuset"),
    frozen = json.bool("frozen"),
    foreground = json.bool("foreground") ?: false,
    system = json.bool("system") ?: false,
    protected = json.bool("protected") ?: false,
    protectedReason = json.string("protected_reason"),
)

fun parseProcessDetail(json: JsonObject): ProcessDetail = ProcessDetail(
    pid = json.int("pid") ?: 0,
    ppid = json.int("ppid") ?: 0,
    uid = json.int("uid") ?: 0,
    user = json.int("user") ?: 0,
    appId = json.int("app_id") ?: 0,
    packageName = json.string("package"),
    packages = json.array("packages").mapNotNull { (it as? JsonPrimitive)?.contentOrNull },
    name = json.string("name").orEmpty(),
    cmdline = json.array("cmdline").mapNotNull { (it as? JsonPrimitive)?.contentOrNull },
    state = json.string("state").orEmpty(),
    threads = json.int("threads") ?: 0,
    rssKb = json.long("rss_kb"),
    pssKb = json.long("pss_kb"),
    swapPssKb = json.long("swap_pss_kb"),
    cpuPct = json.double("cpu_pct"),
    sampleMs = json.long("sample_ms") ?: 0L,
    oomScoreAdj = json.int("oom_score_adj"),
    nice = json.int("nice") ?: 0,
    rtPriority = json.int("rt_priority") ?: 0,
    policy = json.int("policy") ?: 0,
    policyName = json.string("policy_name"),
    cgroup = json.string("cgroup"),
    cpuset = json.string("cpuset"),
    startTimeTicks = json.long("start_time_ticks") ?: 0L,
    frozen = json.bool("frozen"),
    foreground = json.bool("foreground") ?: false,
    system = json.bool("system") ?: false,
    protected = json.bool("protected") ?: false,
    protectedReason = json.string("protected_reason"),
)

fun parseProcessAction(json: JsonObject): ProcessActionResult = ProcessActionResult(
    pkg = json.string("package"),
    target = json.string("target"),
    adjustment = json.int("adjustment"),
    nice = json.int("nice"),
    policy = json.string("policy"),
    policyId = json.int("policy_id"),
    priority = json.int("priority"),
    group = json.string("group"),
    frozen = json.bool("frozen"),
    mechanism = json.string("mechanism"),
    alreadyStopped = json.bool("already_stopped") ?: false,
    pidsBefore = json.array("pids_before").mapNotNull { (it as? JsonPrimitive)?.intOrNull },
    pidsAfter = json.array("pids_after").mapNotNull { (it as? JsonPrimitive)?.intOrNull },
    verified = json.bool("verified"),
    note = json.string("note"),
    targets = json.array("targets").mapNotNull { element ->
        (element as? JsonObject)?.let { target ->
            ProcessActionTarget(
                pid = target.int("pid") ?: 0,
                before = target.string("before"),
                applied = target.string("applied"),
                verified = target.bool("verified"),
                error = target.string("error"),
                mechanism = target.string("mechanism"),
            )
        }
    },
)

// ---------------------------------------------------------------------------------------------
// Formatting shared by the screen and its tests
// ---------------------------------------------------------------------------------------------

/** 136_684 kB → "133.5 MB"; memory rows only need two significant units. */
fun formatProcessKb(kb: Long?): String {
    if (kb == null) return "—"
    return when {
        kb >= 1024L * 1024L -> String.format(Locale.ROOT, "%.1f GB", kb / (1024.0 * 1024.0))
        kb >= 1024L -> String.format(Locale.ROOT, "%.1f MB", kb / 1024.0)
        else -> "$kb KB"
    }
}

/** Per-core CPU percentage: one decimal, and never a fake `0.0` when the value is unknown. */
fun formatProcessCpu(pct: Double?): String =
    if (pct == null) "—" else String.format(Locale.ROOT, "%.1f%%", pct)

/** The one-line subtitle of a row: UID, threads and OOM score when the device reports them. */
fun processSubtitle(entry: ProcessEntry): String = buildString {
    append("UID ")
    append(entry.uid)
    if (entry.threads > 0) append(" · ${entry.threads} 线程")
    entry.oomScoreAdj?.let { append(" · OOA $it") }
    if (entry.state.isNotBlank() && entry.state != "S") append(" · ${entry.state}")
}

/** Human-readable Linux scheduling policy label. */
fun schedPolicyLabel(policy: Int, policyName: String? = null): String =
    policyName?.takeIf { it.isNotBlank() && it != "UNKNOWN" } ?: when (policy) {
        0 -> "SCHED_OTHER"
        1 -> "SCHED_FIFO"
        2 -> "SCHED_RR"
        3 -> "SCHED_BATCH"
        5 -> "SCHED_IDLE"
        6 -> "SCHED_DEADLINE"
        else -> "POLICY($policy)"
    }

/** CLI argument name for `sysflow process sched`. */
fun schedPolicyArg(policy: Int): String = when (policy) {
    1 -> "fifo"
    2 -> "rr"
    3 -> "batch"
    5 -> "idle"
    else -> "other"
}

fun isRealtimeSchedPolicy(policy: Int): Boolean = policy == 1 || policy == 2

private fun JsonObject.obj(key: String): JsonObject? = this[key] as? JsonObject
private fun JsonObject.array(key: String): JsonArray = this[key] as? JsonArray ?: JsonArray(emptyList())
private fun JsonObject.string(key: String): String? = (this[key] as? JsonPrimitive)?.contentOrNull
private fun JsonObject.int(key: String): Int? = (this[key] as? JsonPrimitive)?.intOrNull
private fun JsonObject.long(key: String): Long? = (this[key] as? JsonPrimitive)?.longOrNull
private fun JsonObject.double(key: String): Double? = (this[key] as? JsonPrimitive)?.doubleOrNull
private fun JsonObject.bool(key: String): Boolean? = (this[key] as? JsonPrimitive)?.booleanOrNull
```

---

回复 **“继续”** 我发 **第 5 批**（`ProcessViewModel.kt` 和 `ProcessOverview.kt` 完整文件）。

---

## You

继续

---

## AI Studio

### 第 5 批（2 个文件）：`ProcessViewModel.kt`、`ProcessOverview.kt`

#### 1. `app/src/main/kotlin/io/sysflow/feature/process/ProcessViewModel.kt`

```kotlin
package io.sysflow.feature.process

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.sysflow.core.root.SysFlowClient
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.util.Locale

/**
 * State holder for the process page.
 *
 * The list is pulled from `sysflow process list` only while the page is visible (the screen owns
 * the polling loop, as on every other page — the APK has no background work). `get` and the actions
 * are pull-on-demand. Nothing here persists: the module treats every action as one-shot.
 */
class ProcessViewModel(private val client: SysFlowClient) : ViewModel() {

    data class UiState(
        val loading: Boolean = true,
        val list: ProcessList? = null,
        val detail: ProcessDetail? = null,
        val detailLoading: Boolean = false,
        /** `caps.process.features`; an absent feature hides its control instead of failing later. */
        val features: Set<String> = emptySet(),
        val sort: String = "cpu",
        val filter: String = "all",
        val query: String = "",
        val limit: Int = DEFAULT_LIMIT,
        /** A user-initiated refresh uses a longer sampling window than the 2 s poll. */
        val refreshing: Boolean = false,
        /** Human label of the action in flight, or `null`. */
        val actionRunning: String? = null,
        val message: String? = null,
        val error: String? = null,
        /** Experimental actions (OOM / sched / cpuset / freeze) stay hidden until explicitly enabled. */
        val experimental: Boolean = false,
        val selectionMode: Boolean = false,
        val selectedPids: Set<Int> = emptySet(),
    ) {
        val hasExperimental: Boolean
            get() = "oom_score_adj" in features || "sched" in features ||
                "cpuset" in features || "freeze" in features
    }

    private val _state = MutableStateFlow(UiState())
    private var detailJob: Job? = null
    val state = _state.asStateFlow()

    init {
        refresh()
        loadFeatures()
    }

    /** Silent polls use the short sample window; pull-to-refresh uses 800 ms for a steadier value. */
    fun refresh(silent: Boolean = false) {
        viewModelScope.launch {
            val current = _state.value
            if (!silent) _state.update { it.copy(refreshing = true) }
            runCatching {
                client.processList(
                    limit = current.limit.coerceIn(DEFAULT_LIMIT, MAX_LIMIT),
                    sort = current.sort,
                    filter = current.filter,
                    query = current.query.ifBlank { null },
                    sampleMs = if (silent) POLL_SAMPLE_MS else REFRESH_SAMPLE_MS,
                    foreground = true,
                )
            }.onSuccess { json ->
                val parsed = parseProcessList(json)
                _state.update {
                    val validPids = parsed.entries.map { entry -> entry.pid }.toSet()
                    it.copy(
                        loading = false,
                        refreshing = false,
                        list = parsed,
                        error = null,
                        selectedPids = if (it.selectionMode) {
                            it.selectedPids.intersect(validPids)
                        } else {
                            emptySet()
                        },
                    )
                }
            }.onFailure { error ->
                _state.update {
                    it.copy(
                        loading = false,
                        refreshing = false,
                        error = error.message ?: "读取进程列表失败",
                    )
                }
            }
        }
    }

    /** Capability detection: never show an action the module did not advertise. */
    fun loadFeatures() {
        viewModelScope.launch {
            runCatching { client.caps() }.onSuccess { caps ->
                _state.update { it.copy(features = processFeatures(caps)) }
            }
        }
    }

    fun setSort(sort: String) {
        _state.update { it.copy(sort = sort) }
        refresh()
    }

    fun setFilter(filter: String) {
        _state.update { it.copy(filter = filter) }
        refresh()
    }

    fun setQuery(query: String) {
        val cleared = query.isEmpty() && _state.value.query.isNotEmpty()
        _state.update { it.copy(query = query) }
        if (cleared) refresh()
    }

    fun submitQuery() {
        refresh()
    }

    fun toggleExperimental() {
        _state.update { it.copy(experimental = !it.experimental) }
    }

    fun loadMore() {
        val nextLimit = (_state.value.limit + LIMIT_STEP).coerceAtMost(MAX_LIMIT)
        if (nextLimit == _state.value.limit) return
        _state.update { it.copy(limit = nextLimit) }
        refresh()
    }

    fun toggleSelectionMode() {
        _state.update {
            val next = !it.selectionMode
            it.copy(selectionMode = next, selectedPids = emptySet())
        }
    }

    fun enterSelectionWith(pid: Int) {
        val entry = _state.value.list?.entries?.firstOrNull { it.pid == pid } ?: return
        if (entry.protected) return
        _state.update { it.copy(selectionMode = true, selectedPids = setOf(pid)) }
    }

    fun toggleSelected(pid: Int) {
        val entry = _state.value.list?.entries?.firstOrNull { it.pid == pid } ?: return
        if (entry.protected) return
        _state.update { current ->
            val next = current.selectedPids.toMutableSet().apply {
                if (!add(pid)) remove(pid)
            }.toSet()
            current.copy(selectedPids = next)
        }
    }

    fun clearSelection() {
        _state.update { it.copy(selectionMode = false, selectedPids = emptySet()) }
    }

    /**
     * Load one process into the detail sheet.
     *
     * [reload] refreshes the open sheet in place after an action, instead of blanking it to a
     * spinner. If the process is gone by then (a successful stop), the sheet closes quietly and the
     * action's own message stays — "读取进程失败" would wrongly read as if the stop had failed.
     */
    fun loadDetail(pid: Int, reload: Boolean = false) {
        detailJob?.cancel()
        if (!reload) _state.update { it.copy(detailLoading = true, detail = null) }
        detailJob = viewModelScope.launch {
            runCatching { client.processGet(pid) }
                .onSuccess { json ->
                    _state.update {
                        it.copy(detailLoading = false, detail = parseProcessDetail(json))
                    }
                }
                .onFailure { error ->
                    if (error is CancellationException) throw error
                    _state.update {
                        if (reload) {
                            it.copy(detailLoading = false, detail = null)
                        } else {
                            it.copy(
                                detailLoading = false,
                                error = error.message ?: "读取进程 $pid 失败",
                            )
                        }
                    }
                }
        }
    }

    /** Closing the sheet also drops a load still in flight, so it cannot pop the sheet back open. */
    fun dismissDetail() {
        detailJob?.cancel()
        _state.update { it.copy(detail = null, detailLoading = false) }
    }

    fun stop(target: String, user: Int? = null) = runAction("停止 $target") {
        val json = client.processStop(target, user)
        val result = parseProcessAction(json)
        when {
            result.alreadyStopped -> "$target 没有运行中的进程"
            result.verified == true -> "已停止 $target"
            else -> "已请求停止 $target，但仍有进程存活"
        }
    }

    fun oom(target: String, adjustment: Int) = runAction("调整 $target 的 OOA") {
        val result = parseProcessAction(client.processOom(target, adjustment))
        val failed = result.targets.count { it.verified == false }
        if (failed == 0) {
            "$target 的 oom_score_adj 已设为 $adjustment"
        } else {
            "$failed 个进程未能应用 oom_score_adj"
        }
    }

    fun renice(target: String, nice: Int) = runAction("调整 $target 的优先级") {
        val result = parseProcessAction(client.processRenice(target, nice))
        val failed = result.targets.count { it.verified == false }
        if (failed == 0) "$target 的 nice 已设为 $nice" else "$failed 个进程未能调整 nice"
    }

    fun sched(target: String, policy: String, priority: Int?) =
        runAction("调整 $target 的调度策略") {
            val result = parseProcessAction(client.processSched(target, policy, priority))
            val failed = result.targets.count { it.verified == false }
            val label = result.policy ?: policy
            val prio = result.priority?.takeIf { it > 0 }?.let { "（优先级 $it）" }.orEmpty()
            if (failed == 0) "$target 的调度策略已设为 $label$prio"
            else "$failed 个进程未能应用调度策略"
        }

    fun cpuset(target: String, group: String) = runAction("把 $target 移入 $group") {
        val result = parseProcessAction(client.processCpuset(target, group))
        val moved = result.targets.count { it.verified == true }
        "已把 $target 的 $moved 个进程移入 $group（ActivityManager 可能随后改回）"
    }

    fun freeze(target: String, frozen: Boolean) = runAction(
        if (frozen) "冻结 $target" else "解冻 $target",
    ) {
        val result = parseProcessAction(client.processFreeze(target, frozen))
        val verb = if (frozen) "冻结" else "解冻"
        val unverified = result.targets.count { it.verified == null }
        if (unverified > 0) {
            "$verb $target 已提交（am 路径只能看退出码，未逐进程读回）"
        } else {
            "$verb $target 完成"
        }
    }

    fun batchStop() {
        val stateNow = _state.value
        val targets = stateNow.list?.entries.orEmpty()
            .filter { it.pid in stateNow.selectedPids && !it.protected && it.packageName != null }
            .mapNotNull { entry -> entry.packageName?.let { it to entry.user } }
            .distinct()
        if (targets.isEmpty()) return
        viewModelScope.launch {
            _state.update {
                it.copy(actionRunning = "批量强停 ${targets.size} 个应用", message = null, error = null)
            }
            var stopped = 0
            var skipped = 0
            var failed = 0
            var lastError: String? = null
            for ((pkg, user) in targets) {
                runCatching { parseProcessAction(client.processStop(pkg, user)) }
                    .onSuccess { res ->
                        when {
                            res.alreadyStopped -> skipped += 1
                            res.verified == true -> stopped += 1
                            else -> failed += 1
                        }
                    }
                    .onFailure { err ->
                        failed += 1
                        lastError = err.message
                    }
            }
            val summary = buildString {
                append("批量强停完成：已停止 $stopped 个")
                if (skipped > 0) append("，已不在运行 $skipped 个")
                if (failed > 0) append("，未确认或失败 $failed 个")
                if (lastError != null && failed > 0) append("（$lastError）")
            }
            _state.update {
                it.copy(
                    actionRunning = null,
                    selectionMode = false,
                    selectedPids = emptySet(),
                    message = if (failed == 0) summary else null,
                    error = if (failed > 0) summary else null,
                )
            }
            refresh(silent = true)
        }
    }

    fun batchFreeze(frozen: Boolean) {
        val stateNow = _state.value
        val targets = stateNow.list?.entries.orEmpty()
            .filter { it.pid in stateNow.selectedPids && !it.protected }
            .map { entry -> entry.packageName ?: entry.pid.toString() }
            .distinct()
        if (targets.isEmpty()) return
        val verb = if (frozen) "冻结" else "解冻"
        viewModelScope.launch {
            _state.update {
                it.copy(actionRunning = "批量${verb} ${targets.size} 项", message = null, error = null)
            }
            var ok = 0
            var failed = 0
            var lastError: String? = null
            for (target in targets) {
                runCatching { client.processFreeze(target, frozen) }
                    .onSuccess { ok += 1 }
                    .onFailure { err ->
                        failed += 1
                        lastError = err.message
                    }
            }
            val summary = buildString {
                append("批量${verb}完成：成功 $ok 项")
                if (failed > 0) append("，失败 $failed 项")
                if (lastError != null && failed > 0) append("（$lastError）")
            }
            _state.update {
                it.copy(
                    actionRunning = null,
                    selectionMode = false,
                    selectedPids = emptySet(),
                    message = if (failed == 0) summary else null,
                    error = if (failed > 0) summary else null,
                )
            }
            refresh(silent = true)
        }
    }

    fun dismissMessage() {
        _state.update { it.copy(message = null, error = null) }
    }

    private fun runAction(label: String, block: suspend () -> String) {
        viewModelScope.launch {
            _state.update { it.copy(actionRunning = label, message = null, error = null) }
            runCatching { block() }
                .onSuccess { message ->
                    _state.update { it.copy(actionRunning = null, message = message) }
                }
                .onFailure { error ->
                    _state.update {
                        it.copy(
                            actionRunning = null,
                            error = error.message ?: String.format(Locale.ROOT, "%s 失败", label),
                        )
                    }
                }
            // The action changed live process state; refresh the list and the open detail if any.
            refresh(silent = true)
            _state.value.detail?.let { loadDetail(it.pid, reload = true) }
        }
    }

    companion object {
        const val DEFAULT_LIMIT = 80
        const val LIMIT_STEP = 60
        const val MAX_LIMIT = 200
        const val POLL_SAMPLE_MS = 300
        const val REFRESH_SAMPLE_MS = 800
    }
}
```

---

#### 2. `app/src/main/kotlin/io/sysflow/feature/process/ProcessOverview.kt`

```kotlin
@file:OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalFoundationApi::class)

package io.sysflow.feature.process

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Apps
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Person
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.InfoPill
import io.sysflow.core.ui.components.rememberAppIdentities
import io.sysflow.core.ui.components.segmentedShape
import io.sysflow.core.ui.theme.sysFlowColors

/**
 * The process list, read top to bottom as a question chain: how big is the picture → who is in
 * front → how do I narrow it down → who is heavy.
 *
 * The rows are one connected list (soft inner corners, rounded outer ones) instead of a pile of
 * separate cards, and each carries a thin usage bar drawn relative to the busiest row, so the
 * heavy processes read at a glance without opening anything.
 */
@Composable
internal fun ProcessListContent(
    state: ProcessViewModel.UiState,
    list: ProcessList,
    bottomInset: Dp,
    onRefresh: () -> Unit,
    onQuery: (String) -> Unit,
    onSubmitQuery: () -> Unit,
    onFilter: (String) -> Unit,
    onSort: (String) -> Unit,
    onToggleExperimental: () -> Unit,
    onLoadMore: () -> Unit,
    onToggleSelectionMode: () -> Unit,
    onEnterSelection: (Int) -> Unit,
    onToggleSelected: (Int) -> Unit,
    onOpen: (Int) -> Unit,
) {
    val foregroundPackage = list.foreground.packageName
    val packages = remember(list.entries, foregroundPackage) {
        (list.entries.mapNotNull { it.packageName } + listOfNotNull(foregroundPackage)).distinct()
    }
    val identities = rememberAppIdentities(packages)
    val maxCpu = remember(list.entries) { list.entries.maxOfOrNull { it.cpuPct ?: 0.0 } ?: 0.0 }
    val maxRss = remember(list.entries) { list.entries.maxOfOrNull { it.rssKb ?: 0L } ?: 0L }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(
            start = 16.dp,
            end = 16.dp,
            top = 8.dp,
            bottom = (if (state.selectionMode) 104.dp else 24.dp) + bottomInset,
        ),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        item(key = "hero") {
            ProcessHero(
                list = list,
                foregroundLabel = foregroundPackage?.let { identities[it]?.label ?: it },
                foregroundIcon = foregroundPackage?.let { identities[it]?.icon },
                modifier = Modifier.padding(bottom = 8.dp),
            )
        }

        item(key = "controls") {
            Column(
                modifier = Modifier.padding(bottom = 8.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                ProcessSearchField(
                    query = state.query,
                    refreshing = state.refreshing,
                    onQuery = onQuery,
                    onSubmit = onSubmitQuery,
                    onRefresh = onRefresh,
                )
                ConnectedChoiceRow(
                    options = listOf("all", "user", "system"),
                    selected = state.filter,
                    label = { filterLabel(it) },
                    onSelect = onFilter,
                    icon = { filterIcon(it) },
                )
                ConnectedChoiceRow(
                    options = listOf("cpu", "rss", "name", "pid"),
                    selected = state.sort,
                    label = { sortLabel(it) },
                    onSelect = onSort,
                    compact = true,
                )
            }
        }

        if (state.hasExperimental) {
            item(key = "experimental") {
                ExperimentalTile(
                    enabled = state.experimental,
                    features = state.features,
                    onToggle = onToggleExperimental,
                    modifier = Modifier.padding(bottom = 8.dp),
                )
            }
        }

        item(key = "header") {
            SectionHeader(
                title = "进程",
                supporting = if (state.selectionMode) {
                    "点按勾选要批量处理的进程（受保护进程不可选）"
                } else {
                    "按${sortLabel(state.sort)}排序 · 点按查看详情，长按多选"
                },
                action = {
                    FilterChip(
                        selected = state.selectionMode,
                        onClick = onToggleSelectionMode,
                        label = { Text(if (state.selectionMode) "完成" else "多选") },
                        leadingIcon = if (state.selectionMode) {
                            {
                                Icon(
                                    Icons.Rounded.Check,
                                    contentDescription = null,
                                    modifier = Modifier.size(16.dp),
                                )
                            }
                        } else {
                            null
                        },
                    )
                },
                modifier = Modifier.padding(bottom = 8.dp),
            )
        }

        if (list.entries.isEmpty()) {
            item(key = "empty") {
                EmptyActionTile(
                    icon = Icons.Rounded.Search,
                    text = "没有匹配的进程。",
                    actionLabel = "刷新",
                    onAction = onRefresh,
                )
            }
        }

        items(list.entries.size, key = { index ->
            val entry = list.entries[index]
            "${entry.pid}-${entry.name}"
        }) { index ->
            val entry = list.entries[index]
            ProcessRow(
                entry = entry,
                icon = entry.packageName?.let { identities[it]?.icon },
                shape = segmentedShape(index, list.entries.size),
                usage = usageFraction(entry, state.sort, maxCpu, maxRss),
                selectionMode = state.selectionMode,
                selected = entry.pid in state.selectedPids,
                onLongClick = { onEnterSelection(entry.pid) },
                onToggleSelect = { onToggleSelected(entry.pid) },
                onClick = { onOpen(entry.pid) },
                modifier = Modifier.animateItem(),
            )
        }

        item(key = "footer") {
            Column(
                modifier = Modifier.padding(top = 8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (list.truncated && state.limit < ProcessViewModel.MAX_LIMIT) {
                    FilledTonalButton(
                        onClick = onLoadMore,
                        shapes = ButtonDefaults.shapes(),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("加载更多（当前 ${list.returned} / ${list.total}）")
                    }
                }
                NoteTile(
                    text = buildString {
                        append("共 ${list.total} 个进程")
                        if (list.truncated) append("，当前显示前 ${list.returned} 个")
                        if (list.skipped > 0) append("，${list.skipped} 个进程读取失败")
                        append(" · 采样 ${list.sampleMs} ms")
                    },
                )
            }
        }
    }
}

// ---- hero ------------------------------------------------------------------------------------

/**
 * Size of the picture: how many processes, what the listed ones add up to, and who is in front.
 *
 * The sums cover the rows the module returned (at most [ProcessViewModel.MAX_LIMIT]), and say so —
 * CPU is per core, so it can pass 100%, and RSS counts shared pages once per process.
 */
@Composable
private fun ProcessHero(
    list: ProcessList,
    foregroundLabel: String?,
    foregroundIcon: ImageBitmap?,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val accent = MaterialTheme.sysFlowColors.containerAccent
    val cpuSum = remember(list.entries) { list.entries.sumOf { it.cpuPct ?: 0.0 } }
    val rssSum = remember(list.entries) { list.entries.sumOf { it.rssKb ?: 0L } }

    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraExtraLarge,
        color = scheme.primaryContainer,
        contentColor = scheme.onPrimaryContainer,
    ) {
        Column(
            modifier = Modifier.padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Box(
                    modifier = Modifier
                        .size(96.dp)
                        .clip(MaterialShapes.Cookie12Sided.toShape())
                        .background(accent),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        "${list.total}",
                        style = MaterialTheme.typography.headlineMediumEmphasized,
                        color = scheme.primaryContainer,
                        maxLines = 1,
                    )
                }
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    Text(
                        "运行中的进程",
                        style = MaterialTheme.typography.labelLarge,
                        color = scheme.onPrimaryContainer.copy(alpha = 0.75f),
                    )
                    Text(
                        if (list.truncated) "列出前 ${list.returned} 个" else "全部已列出",
                        style = MaterialTheme.typography.headlineSmallEmphasized,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        "每 2 秒刷新 · 采样 ${list.sampleMs} ms",
                        style = MaterialTheme.typography.bodySmall,
                        color = scheme.onPrimaryContainer.copy(alpha = 0.75f),
                    )
                }
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                HeroStat(
                    label = "列表 CPU",
                    value = formatProcessCpu(cpuSum),
                    supporting = "单核计，可超 100%",
                    modifier = Modifier.weight(1f),
                )
                HeroStat(
                    label = "列表 RSS",
                    value = formatProcessKb(rssSum),
                    supporting = "共享页会重复计",
                    modifier = Modifier.weight(1f),
                )
            }

            if (foregroundLabel != null) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(CircleShape)
                        .background(scheme.onPrimaryContainer.copy(alpha = 0.12f))
                        .padding(start = 8.dp, end = 16.dp, top = 8.dp, bottom = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    AppIcon(icon = foregroundIcon, size = 36.dp, highlight = true)
                    Column(Modifier.weight(1f)) {
                        Text(
                            "前台应用",
                            style = MaterialTheme.typography.labelSmall,
                            color = scheme.onPrimaryContainer.copy(alpha = 0.75f),
                        )
                        Text(
                            foregroundLabel,
                            style = MaterialTheme.typography.titleSmallEmphasized,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    list.foreground.pid?.let { pid ->
                        InfoPill(
                            text = "PID $pid",
                            container = scheme.onPrimaryContainer,
                            content = scheme.primaryContainer,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun HeroStat(
    label: String,
    value: String,
    supporting: String,
    modifier: Modifier = Modifier,
) {
    val content = MaterialTheme.colorScheme.onPrimaryContainer
    Column(
        modifier = modifier
            .clip(MaterialTheme.shapes.large)
            .background(content.copy(alpha = 0.12f))
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = content.copy(alpha = 0.75f),
            maxLines = 1,
        )
        Text(
            value,
            style = MaterialTheme.typography.titleLargeEmphasized,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            supporting,
            style = MaterialTheme.typography.labelSmall,
            color = content.copy(alpha = 0.75f),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

// ---- search / experimental -------------------------------------------------------------------

@Composable
private fun ProcessSearchField(
    query: String,
    refreshing: Boolean,
    onQuery: (String) -> Unit,
    onSubmit: () -> Unit,
    onRefresh: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    TextField(
        value = query,
        onValueChange = onQuery,
        modifier = Modifier.fillMaxWidth(),
        singleLine = true,
        shape = CircleShape,
        colors = TextFieldDefaults.colors(
            focusedContainerColor = scheme.surfaceContainerHigh,
            unfocusedContainerColor = scheme.surfaceContainerHigh,
            disabledContainerColor = scheme.surfaceContainerHigh,
            focusedIndicatorColor = Color.Transparent,
            unfocusedIndicatorColor = Color.Transparent,
            disabledIndicatorColor = Color.Transparent,
            errorIndicatorColor = Color.Transparent,
        ),
        leadingIcon = { Icon(Icons.Rounded.Search, contentDescription = null) },
        trailingIcon = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (query.isNotEmpty()) {
                    IconButton(onClick = { onQuery("") }) {
                        Icon(Icons.Rounded.Close, contentDescription = "清除")
                    }
                }
                if (refreshing) {
                    Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
                        LoadingIndicator(modifier = Modifier.size(24.dp))
                    }
                } else {
                    IconButton(onClick = onRefresh) {
                        Icon(Icons.Rounded.Refresh, contentDescription = "刷新")
                    }
                }
            }
        },
        placeholder = { Text("搜索进程、包名或 PID") },
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        keyboardActions = KeyboardActions(onSearch = { onSubmit() }),
    )
}

/** Opt-in switch for the one-shot actions that can hurt: OOM adjust, RT sched, cpuset, freeze. */
@Composable
private fun ExperimentalTile(
    enabled: Boolean,
    features: Set<String>,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val container = if (enabled) scheme.tertiaryContainer else scheme.surfaceContainerHigh
    val content = if (enabled) scheme.onTertiaryContainer else scheme.onSurface
    val available = features.any {
        it == "oom_score_adj" || it == "sched" || it == "cpuset" || it == "freeze"
    }

    Surface(
        onClick = onToggle,
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        color = container,
        contentColor = content,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .clip(MaterialShapes.Cookie9Sided.toShape())
                    .background(if (enabled) scheme.tertiary else scheme.secondaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    Icons.Rounded.Tune,
                    contentDescription = null,
                    tint = if (enabled) scheme.onTertiary else scheme.onSecondaryContainer,
                    modifier = Modifier.size(22.dp),
                )
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("实验功能", style = MaterialTheme.typography.titleMediumEmphasized)
                Text(
                    buildString {
                        append("OOM 调节、实时调度、cpuset 绑核、冻结")
                        if (!available) append("（本机不可用）")
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = content.copy(alpha = 0.75f),
                )
            }
            Switch(checked = enabled, onCheckedChange = { onToggle() })
        }
    }
}

// ---- rows ------------------------------------------------------------------------------------

@Composable
private fun ProcessRow(
    entry: ProcessEntry,
    icon: ImageBitmap?,
    shape: Shape,
    usage: Float,
    selectionMode: Boolean,
    selected: Boolean,
    onLongClick: () -> Unit,
    onToggleSelect: () -> Unit,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = MaterialTheme.colorScheme
    val accent = MaterialTheme.sysFlowColors.containerAccent
    val container = when {
        selected -> scheme.secondaryContainer
        entry.foreground -> scheme.primaryContainer
        else -> scheme.surfaceContainerHigh
    }
    val content = when {
        selected -> scheme.onSecondaryContainer
        entry.foreground -> scheme.onPrimaryContainer
        else -> scheme.onSurface
    }
    val secondary = content.copy(alpha = 0.75f)

    Surface(
        modifier = modifier
            .fillMaxWidth()
            .clip(shape)
            .combinedClickable(
                onClick = {
                    if (selectionMode) {
                        if (!entry.protected) onToggleSelect()
                    } else {
                        onClick()
                    }
                },
                onLongClick = {
                    if (!selectionMode && !entry.protected) onLongClick()
                },
            ),
        shape = shape,
        color = container,
        contentColor = content,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            if (selectionMode) {
                Checkbox(
                    checked = selected,
                    onCheckedChange = null,
                    enabled = !entry.protected,
                )
            }
            AppIcon(icon = icon, size = 48.dp, highlight = entry.foreground)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        entry.displayName,
                        modifier = Modifier.weight(1f, fill = false),
                        style = MaterialTheme.typography.titleMediumEmphasized,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (entry.foreground) {
                        InfoPill(
                            text = "前台",
                            container = scheme.onPrimaryContainer,
                            content = scheme.primaryContainer,
                        )
                    }
                    if (entry.frozen == true) {
                        InfoPill(
                            text = "已冻结",
                            container = scheme.tertiaryContainer,
                            content = scheme.onTertiaryContainer,
                        )
                    }
                    if (entry.protected) {
                        Icon(
                            Icons.Rounded.Lock,
                            contentDescription = entry.protectedReason ?: "受保护",
                            modifier = Modifier.size(16.dp),
                            tint = secondary,
                        )
                    }
                }
                Text(
                    processSubtitle(entry),
                    style = MaterialTheme.typography.bodySmall,
                    color = secondary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                UsageBar(
                    fraction = usage,
                    color = if (entry.foreground) accent else scheme.primary,
                    track = if (entry.foreground) {
                        scheme.onPrimaryContainer.copy(alpha = 0.16f)
                    } else {
                        scheme.surfaceContainerHighest
                    },
                    modifier = Modifier.padding(top = 6.dp),
                )
            }
            Column(
                horizontalAlignment = Alignment.End,
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    formatProcessCpu(entry.cpuPct),
                    style = MaterialTheme.typography.titleMediumEmphasized,
                    maxLines = 1,
                )
                Text(
                    formatProcessKb(entry.rssKb),
                    style = MaterialTheme.typography.bodySmall,
                    color = secondary,
                    maxLines = 1,
                )
            }
        }
    }
}

// ---- helpers ---------------------------------------------------------------------------------

/** The bar follows the sort key: memory for 内存, CPU for everything else. */
private fun usageFraction(entry: ProcessEntry, sort: String, maxCpu: Double, maxRss: Long): Float =
    if (sort == "rss") {
        if (maxRss <= 0L) 0f else ((entry.rssKb ?: 0L).toDouble() / maxRss).toFloat()
    } else {
        if (maxCpu <= 0.0) 0f else ((entry.cpuPct ?: 0.0) / maxCpu).toFloat()
    }

private fun filterLabel(value: String) = when (value) {
    "user" -> "用户"
    "system" -> "系统"
    else -> "全部"
}

private fun filterIcon(value: String) = when (value) {
    "user" -> Icons.Rounded.Person
    "system" -> Icons.Rounded.Memory
    else -> Icons.Rounded.Apps
}

private fun sortLabel(value: String) = when (value) {
    "rss" -> "内存"
    "name" -> "名称"
    "pid" -> "PID"
    else -> "CPU"
}
```

---

回复 **“继续”** 我发最后一批 **第 6 批**（`ProcessDetail.kt`、`ProcessScreen.kt`、`ProcessModelsTest.kt` 以及 `docs/modules/process.md`；`docs/protocol.md` §3.25 修改说明一并附上）。

---

## You

继续

---

## AI Studio

### 第 6 批（3 个文件）：`ProcessDetail.kt`、`ProcessScreen.kt`、`ProcessModelsTest.kt`

#### 1. `app/src/main/kotlin/io/sysflow/feature/process/ProcessDetail.kt`

```kotlin
@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.process

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AccountTree
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.DeleteForever
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.sysflow.core.ui.components.AppIcon
import io.sysflow.core.ui.components.SegmentedValueList
import io.sysflow.core.ui.components.StatusPill
import io.sysflow.core.ui.components.StatusTone
import io.sysflow.core.ui.components.rememberAppIdentities
import kotlin.math.roundToInt

/**
 * Detail sheet of one process: who it is, what it costs, how it is scheduled — and what SysFlow
 * can do to it.
 *
 * Reads top to bottom as identity → cost → scheduling → actions. Every action group shows the
 * value the process has *now* as the checked segment, so a tap is always a change, never a
 * guess. All actions stay one-shot and say so: ActivityManager may reset them afterwards.
 */
@Composable
internal fun ProcessDetailSheet(
    state: ProcessViewModel.UiState,
    onDismiss: () -> Unit,
    onDismissMessage: () -> Unit,
    onStop: (String, Int?) -> Unit,
    onOom: (String, Int) -> Unit,
    onRenice: (String, Int) -> Unit,
    onSched: (String, String, Int?) -> Unit,
    onCpuset: (String, String) -> Unit,
    onFreeze: (String, Boolean) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val detail = state.detail

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = scheme.surfaceContainerLow,
    ) {
        if (detail == null) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 48.dp),
                contentAlignment = Alignment.Center,
            ) { LoadingIndicator() }
            return@ModalBottomSheet
        }

        val identities = rememberAppIdentities(listOfNotNull(detail.packageName))
        val identity = detail.packageName?.let { identities[it] }
        val title = identity?.label ?: detail.packageName ?: detail.name.ifBlank { "pid ${detail.pid}" }
        val subtitle = detail.packageName?.takeIf { it != title } ?: detail.name.takeIf { it != title }
        var scopeByPackage by rememberSaveable(detail.pid) { mutableStateOf(false) }

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .imePadding()
                .verticalScroll(rememberScrollState())
                .padding(start = 16.dp, end = 16.dp, bottom = 24.dp + bottomInset),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // Action feedback lives here too: the page behind the scrim would hide it.
            val message = state.error ?: state.message
            if (message != null) {
                val danger = state.error != null
                NoteTile(
                    text = message,
                    icon = Icons.Rounded.Info,
                    container = if (danger) scheme.errorContainer else scheme.secondaryContainer,
                    content = if (danger) scheme.onErrorContainer else scheme.onSecondaryContainer,
                    onDismiss = onDismissMessage,
                )
            }

            DetailHeader(
                detail = detail,
                title = title,
                subtitle = subtitle,
                icon = identity?.icon,
            )

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                StatTile(
                    label = "CPU",
                    value = formatProcessCpu(detail.cpuPct),
                    container = scheme.primaryContainer,
                    content = scheme.onPrimaryContainer,
                    modifier = Modifier.weight(1f),
                )
                StatTile(
                    label = "RSS",
                    value = formatProcessKb(detail.rssKb),
                    container = scheme.secondaryContainer,
                    content = scheme.onSecondaryContainer,
                    modifier = Modifier.weight(1f),
                )
                StatTile(
                    label = "PSS",
                    value = formatProcessKb(detail.pssKb),
                    container = scheme.tertiaryContainer,
                    content = scheme.onTertiaryContainer,
                    modifier = Modifier.weight(1f),
                )
            }

            SectionHeader(title = "进程信息")
            SegmentedValueList(
                rows = buildList {
                    add("PID / PPID" to "${detail.pid} / ${detail.ppid}")
                    add("UID / 用户" to "${detail.uid} / user ${detail.user}")
                    add("线程" to detail.threads.toString())
                    add("状态" to detail.state.ifBlank { "—" })
                    add(
                        "冻结状态" to when (detail.frozen) {
                            true -> "已冻结"
                            false -> "运行中"
                            null -> "未知"
                        },
                    )
                    add("SwapPss" to formatProcessKb(detail.swapPssKb))
                    add("oom_score_adj" to (detail.oomScoreAdj?.toString() ?: "—"))
                    add("调度优先级 nice" to detail.nice.toString())
                    add(
                        "调度策略" to "${schedPolicyLabel(detail.policy, detail.policyName)} (${detail.policy}) · 实时优先级 ${detail.rtPriority}",
                    )
                    add("cgroup" to (detail.cgroup ?: "—"))
                    add("cpuset" to (detail.cpuset ?: "—"))
                    if (detail.cmdline.isNotEmpty()) add("命令行" to detail.cmdline.joinToString(" "))
                    if (detail.packages.size > 1) add("共享包名" to detail.packages.joinToString("\n"))
                },
            )

            DetailActions(
                detail = detail,
                scopeByPackage = scopeByPackage,
                onScopeChange = { scopeByPackage = it },
                features = state.features,
                experimental = state.experimental,
                groups = state.list?.cpusetGroups.orEmpty(),
                runningLabel = state.actionRunning,
                onStop = onStop,
                onOom = onOom,
                onRenice = onRenice,
                onSched = onSched,
                onCpuset = onCpuset,
                onFreeze = onFreeze,
            )
        }
    }
}

@Composable
private fun DetailHeader(
    detail: ProcessDetail,
    title: String,
    subtitle: String?,
    icon: ImageBitmap?,
) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            AppIcon(icon = icon, size = 64.dp, highlight = detail.foreground)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    title,
                    style = MaterialTheme.typography.headlineSmallEmphasized,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                if (subtitle != null) {
                    Text(
                        subtitle,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
        if (detail.foreground || detail.system || detail.protected || detail.frozen == true) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (detail.foreground) StatusPill("前台", StatusTone.Brand)
                if (detail.frozen == true) StatusPill("已冻结", StatusTone.Warning)
                if (detail.system) StatusPill("系统", StatusTone.Info)
                if (detail.protected) StatusPill("受保护", StatusTone.Error)
            }
        }
    }
}

// ---- actions ---------------------------------------------------------------------------------

@Composable
private fun DetailActions(
    detail: ProcessDetail,
    scopeByPackage: Boolean,
    onScopeChange: (Boolean) -> Unit,
    features: Set<String>,
    experimental: Boolean,
    groups: List<String>,
    runningLabel: String?,
    onStop: (String, Int?) -> Unit,
    onOom: (String, Int) -> Unit,
    onRenice: (String, Int) -> Unit,
    onSched: (String, String, Int?) -> Unit,
    onCpuset: (String, String) -> Unit,
    onFreeze: (String, Boolean) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val busy = runningLabel != null
    val packageTarget = detail.packageName
    val target = if (scopeByPackage && packageTarget != null) packageTarget else detail.pid.toString()

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionHeader(title = "操作", supporting = "均为一次性操作，系统可能随后改回")

        if (detail.protected) {
            NoteTile(
                text = "系统进程受保护：${detail.protectedReason ?: "系统进程"}，所有操作已禁用。",
                icon = Icons.Rounded.Lock,
                container = scheme.errorContainer,
                content = scheme.onErrorContainer,
            )
            return@Column
        }

        if (packageTarget != null) {
            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Text("作用范围", style = MaterialTheme.typography.titleSmallEmphasized)
                    ConnectedChoiceRow(
                        options = listOf(false, true),
                        selected = scopeByPackage,
                        label = { if (it) "整个应用" else "仅此进程 (PID ${detail.pid})" },
                        onSelect = onScopeChange,
                        enabled = !busy,
                        compact = true,
                    )
                    if (scopeByPackage) {
                        Text(
                            "调整将同时作用于 $packageTarget 下的全部运行中进程。",
                            style = MaterialTheme.typography.bodySmall,
                            color = scheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }

        if (busy) BusyTile(label = runningLabel.orEmpty())

        if ("stop" in features && packageTarget != null) {
            Button(
                onClick = { onStop(packageTarget, detail.user) },
                enabled = !busy,
                modifier = Modifier.fillMaxWidth(),
                shapes = ButtonDefaults.shapes(),
                colors = ButtonDefaults.buttonColors(
                    containerColor = scheme.errorContainer,
                    contentColor = scheme.onErrorContainer,
                ),
            ) {
                Icon(
                    Icons.Rounded.DeleteForever,
                    contentDescription = null,
                    modifier = Modifier.size(ButtonDefaults.IconSize),
                )
                Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                Text("强制停止应用", maxLines = 1)
            }
        }

        if ("renice" in features) {
            var niceText by rememberSaveable(detail.pid, detail.nice) {
                mutableStateOf(detail.nice.toString())
            }
            val parsedNice = niceText.toIntOrNull()
            val niceValid = parsedNice != null && parsedNice in -20..19

            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Speed,
                        title = "调度优先级 nice",
                        supporting = "越小越优先，当前 ${detail.nice}",
                    )
                    ConnectedChoiceRow(
                        options = listOf(-10, -5, 0, 5, 10),
                        selected = detail.nice,
                        label = { it.toString() },
                        onSelect = { value -> if (scopeByPackage || value != detail.nice) onRenice(target, value) },
                        enabled = !busy,
                        compact = true,
                    )
                    Slider(
                        value = (parsedNice ?: detail.nice).coerceIn(-20, 19).toFloat(),
                        onValueChange = { niceText = it.roundToInt().toString() },
                        valueRange = -20f..19f,
                        enabled = !busy,
                    )
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        FilledTonalIconButton(
                            onClick = { niceText = toggleNumberSign(niceText) },
                            enabled = !busy,
                        ) {
                            Text("±", style = MaterialTheme.typography.titleMediumEmphasized)
                        }
                        OutlinedTextField(
                            value = niceText,
                            onValueChange = { niceText = filterSignedDigits(it, 3) },
                            label = { Text("自定义 (-20..19)") },
                            isError = niceText.isNotEmpty() && !niceValid,
                            singleLine = true,
                            enabled = !busy,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
                            modifier = Modifier.weight(1f),
                        )
                        FilledTonalButton(
                            onClick = { parsedNice?.let { onRenice(target, it) } },
                            enabled = !busy && niceValid && (scopeByPackage || parsedNice != detail.nice),
                            shapes = ButtonDefaults.shapes(),
                        ) {
                            Text("应用", maxLines = 1)
                        }
                    }
                }
            }
        }

        if (experimental && "oom_score_adj" in features) {
            var oomText by rememberSaveable(detail.pid, detail.oomScoreAdj) {
                mutableStateOf((detail.oomScoreAdj ?: 0).toString())
            }
            val parsedOom = oomText.toIntOrNull()
            val oomValid = parsedOom != null && parsedOom in -1000..1000

            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Tune,
                        title = "oom_score_adj",
                        supporting = "高风险 · 当前 ${detail.oomScoreAdj ?: "—"}",
                        container = scheme.errorContainer,
                        content = scheme.onErrorContainer,
                    )
                    ConnectedChoiceRow(
                        options = listOf(-1000, 0, 100, 500),
                        selected = detail.oomScoreAdj,
                        label = { it.toString() },
                        onSelect = { value -> if (scopeByPackage || value != detail.oomScoreAdj) onOom(target, value) },
                        enabled = !busy,
                        compact = true,
                    )
                    Slider(
                        value = (parsedOom ?: detail.oomScoreAdj ?: 0).coerceIn(-1000, 1000).toFloat(),
                        onValueChange = { oomText = it.roundToInt().toString() },
                        valueRange = -1000f..1000f,
                        enabled = !busy,
                    )
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        FilledTonalIconButton(
                            onClick = { oomText = toggleNumberSign(oomText) },
                            enabled = !busy,
                        ) {
                            Text("±", style = MaterialTheme.typography.titleMediumEmphasized)
                        }
                        OutlinedTextField(
                            value = oomText,
                            onValueChange = { oomText = filterSignedDigits(it, 5) },
                            label = { Text("自定义 (-1000..1000)") },
                            isError = oomText.isNotEmpty() && !oomValid,
                            singleLine = true,
                            enabled = !busy,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
                            modifier = Modifier.weight(1f),
                        )
                        FilledTonalButton(
                            onClick = { parsedOom?.let { onOom(target, it) } },
                            enabled = !busy && oomValid && (scopeByPackage || parsedOom != detail.oomScoreAdj),
                            shapes = ButtonDefaults.shapes(),
                        ) {
                            Text("应用", maxLines = 1)
                        }
                    }
                }
            }
        }

        if (experimental && "sched" in features) {
            var selectedPolicy by rememberSaveable(detail.pid, detail.policy) {
                mutableIntStateOf(if (detail.policy in listOf(0, 1, 2, 3, 5)) detail.policy else 0)
            }
            var rtPrioText by rememberSaveable(detail.pid, detail.rtPriority) {
                mutableStateOf((if (detail.rtPriority in 1..99) detail.rtPriority else 10).toString())
            }
            val isRt = isRealtimeSchedPolicy(selectedPolicy)
            val parsedRtPrio = rtPrioText.toIntOrNull()
            val rtPrioValid = !isRt || (parsedRtPrio != null && parsedRtPrio in 1..99)
            val schedChanged = scopeByPackage ||
                selectedPolicy != detail.policy ||
                (isRt && parsedRtPrio != detail.rtPriority)

            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Bolt,
                        title = "实时调度策略",
                        supporting = "高风险 · 当前 ${schedPolicyLabel(detail.policy, detail.policyName)} (${detail.rtPriority})",
                        container = scheme.errorContainer,
                        content = scheme.onErrorContainer,
                    )
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        ConnectedChoiceRow(
                            options = listOf(0, 3, 5),
                            selected = selectedPolicy,
                            label = { schedOptionLabel(it) },
                            onSelect = { selectedPolicy = it },
                            enabled = !busy,
                            compact = true,
                        )
                        ConnectedChoiceRow(
                            options = listOf(1, 2),
                            selected = selectedPolicy,
                            label = { schedOptionLabel(it) },
                            onSelect = { selectedPolicy = it },
                            enabled = !busy,
                            compact = true,
                            slots = 3,
                        )
                    }
                    if (isRt) {
                        ConnectedChoiceRow(
                            options = listOf(1, 10, 50, 99),
                            selected = parsedRtPrio,
                            label = { it.toString() },
                            onSelect = { rtPrioText = it.toString() },
                            enabled = !busy,
                            compact = true,
                        )
                        Slider(
                            value = (parsedRtPrio ?: 10).coerceIn(1, 99).toFloat(),
                            onValueChange = { rtPrioText = it.roundToInt().toString() },
                            valueRange = 1f..99f,
                            enabled = !busy,
                        )
                        OutlinedTextField(
                            value = rtPrioText,
                            onValueChange = { rtPrioText = it.filter(Char::isDigit).take(2) },
                            label = { Text("实时优先级 (1..99)") },
                            isError = rtPrioText.isNotEmpty() && !rtPrioValid,
                            singleLine = true,
                            enabled = !busy,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                    Button(
                        onClick = {
                            onSched(target, schedPolicyArg(selectedPolicy), if (isRt) parsedRtPrio else 0)
                        },
                        enabled = !busy && rtPrioValid && schedChanged,
                        modifier = Modifier.fillMaxWidth(),
                        shapes = ButtonDefaults.shapes(),
                    ) {
                        Text("应用调度策略", maxLines = 1)
                    }
                }
            }
        }

        if (experimental && "cpuset" in features && groups.isNotEmpty()) {
            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.AccountTree,
                        title = "cpuset 绑核",
                        supporting = "当前 ${detail.cpuset ?: "—"}",
                        container = scheme.tertiaryContainer,
                        content = scheme.onTertiaryContainer,
                    )
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        groups.chunked(2).forEach { row ->
                            ConnectedChoiceRow(
                                options = row,
                                selected = detail.cpuset,
                                label = { it },
                                onSelect = { group ->
                                    if (scopeByPackage || group != detail.cpuset) onCpuset(target, group)
                                },
                                enabled = !busy,
                                compact = true,
                                slots = 2,
                            )
                        }
                    }
                }
            }
        }

        if (experimental && "freeze" in features) {
            val freezeStatus = when (detail.frozen) {
                true -> "当前：已冻结 · 系统可能随时解冻"
                false -> "当前：运行中 · 冻结后应用停止运行"
                null -> "状态未知 · 冻结后应用不运行，系统可能随时解冻"
            }
            SectionSurface {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    ActionGroupHeader(
                        icon = Icons.Rounded.Bedtime,
                        title = "冻结",
                        supporting = freezeStatus,
                        container = scheme.errorContainer,
                        content = scheme.onErrorContainer,
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(
                            onClick = { onFreeze(target, true) },
                            enabled = !busy && (scopeByPackage || detail.frozen != true),
                            modifier = Modifier.weight(1f),
                            shapes = ButtonDefaults.shapes(),
                        ) {
                            Icon(
                                Icons.Rounded.Bedtime,
                                contentDescription = null,
                                modifier = Modifier.size(ButtonDefaults.IconSize),
                            )
                            Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                            Text("冻结", maxLines = 1)
                        }
                        FilledTonalButton(
                            onClick = { onFreeze(target, false) },
                            enabled = !busy && (scopeByPackage || detail.frozen != false),
                            modifier = Modifier.weight(1f),
                            shapes = ButtonDefaults.shapes(),
                        ) {
                            Icon(
                                Icons.Rounded.PlayArrow,
                                contentDescription = null,
                                modifier = Modifier.size(ButtonDefaults.IconSize),
                            )
                            Spacer(Modifier.size(ButtonDefaults.IconSpacing))
                            Text("解冻", maxLines = 1)
                        }
                    }
                }
            }
        }

        if (features.isEmpty()) {
            NoteTile(text = "模块未报告任何动作能力，本机只能查看。")
        }
    }
}

/** The action in flight; every control is disabled until it finishes. */
@Composable
private fun BusyTile(label: String) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            LoadingIndicator(modifier = Modifier.size(32.dp))
            Text(
                "正在${label}…",
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private fun schedOptionLabel(policy: Int): String = when (policy) {
    0 -> "默认 OTHER"
    1 -> "实时 FIFO"
    2 -> "实时 RR"
    3 -> "批处理 BATCH"
    5 -> "空闲 IDLE"
    else -> policy.toString()
}

internal fun filterSignedDigits(raw: String, maxLen: Int): String {
    val builder = StringBuilder()
    raw.trim().forEachIndexed { index, ch ->
        if ((ch == '-' && index == 0) || ch.isDigit()) {
            builder.append(ch)
        }
    }
    return builder.toString().take(maxLen)
}

internal fun toggleNumberSign(raw: String): String {
    val trimmed = raw.trim()
    if (trimmed.isEmpty() || trimmed == "0") return "-"
    if (trimmed == "-") return ""
    return if (trimmed.startsWith("-")) trimmed.drop(1) else "-$trimmed"
}
```

---

#### 2. `app/src/main/kotlin/io/sysflow/feature/process/ProcessScreen.kt`

```kotlin
@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package io.sysflow.feature.process

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AccountTree
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.DeleteForever
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material.icons.rounded.Tune
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalFloatingToolbar
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import org.koin.androidx.compose.koinViewModel

/**
 * Process page: who is running, how much they use, and the few one-shot actions SysFlow offers.
 *
 * This file only hosts the page — polling, the loading / empty states, the detail sheet and the
 * confirmation dialog. The list lives in `ProcessOverview.kt`, the sheet in `ProcessDetail.kt`.
 *
 * The page owns the polling loop (2 s while STARTED) and passes a 300 ms sampling window, so the
 * CPU number is fresh without the page feeling heavy. Every action is one-shot: the module never
 * persists OOM/nice/sched/cpuset state, and ActivityManager is free to reset it afterwards — the UI
 * says so where it matters instead of pretending otherwise.
 *
 * `SysFlowApp` wraps this route in `FlexibleSubPage`, so no heading is drawn here.
 */
@Composable
fun ProcessScreen(viewModel: ProcessViewModel = koinViewModel()) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val lifecycleOwner = LocalLifecycleOwner.current
    val bottomInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    var confirm by remember { mutableStateOf<ProcessConfirm?>(null) }

    val sheetOpen = state.detailLoading || state.detail != null
    BackHandler(enabled = state.selectionMode && !sheetOpen) { viewModel.clearSelection() }

    // Only while the page is visible; a running action or multi-select pauses the poll so the list
    // does not jump under the user's finger.
    LaunchedEffect(Unit) {
        lifecycleOwner.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                delay(2_000)
                val current = viewModel.state.value
                if (current.actionRunning == null && !current.selectionMode) {
                    viewModel.refresh(silent = true)
                }
            }
        }
    }

    val list = state.list

    Column(Modifier.fillMaxSize()) {
        // The sheet shows its own feedback; the page banner would sit behind the scrim.
        val feedback = state.error ?: state.message
        AnimatedVisibility(visible = feedback != null && !sheetOpen && list != null) {
            val danger = state.error != null
            NoteTile(
                text = feedback.orEmpty(),
                icon = Icons.Rounded.Info,
                container = if (danger) {
                    MaterialTheme.colorScheme.errorContainer
                } else {
                    MaterialTheme.colorScheme.secondaryContainer
                },
                content = if (danger) {
                    MaterialTheme.colorScheme.onErrorContainer
                } else {
                    MaterialTheme.colorScheme.onSecondaryContainer
                },
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                onDismiss = viewModel::dismissMessage,
            )
        }
        Box(Modifier.weight(1f)) {
            when {
                list == null && state.loading -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    LoadingIndicator()
                }
                list == null -> Box(Modifier.fillMaxSize(), Alignment.Center) {
                    EmptyActionTile(
                        icon = Icons.Rounded.Timeline,
                        text = state.error ?: "设备没有提供 /proc，或模块尚未加载。",
                        actionLabel = "重试",
                        onAction = { viewModel.refresh() },
                        modifier = Modifier.padding(16.dp),
                    )
                }
                else -> ProcessListContent(
                    state = state,
                    list = list,
                    bottomInset = bottomInset,
                    onRefresh = { viewModel.refresh() },
                    onQuery = viewModel::setQuery,
                    onSubmitQuery = viewModel::submitQuery,
                    onFilter = viewModel::setFilter,
                    onSort = viewModel::setSort,
                    onToggleExperimental = viewModel::toggleExperimental,
                    onLoadMore = viewModel::loadMore,
                    onToggleSelectionMode = viewModel::toggleSelectionMode,
                    onEnterSelection = viewModel::enterSelectionWith,
                    onToggleSelected = viewModel::toggleSelected,
                    onOpen = viewModel::loadDetail,
                )
            }
            if (state.selectionMode && list != null) {
                val selectedEntries = list.entries.filter { it.pid in state.selectedPids }
                val packageCount = selectedEntries.mapNotNull { it.packageName }.distinct().size
                val canBatchStop = "stop" in state.features && packageCount > 0
                val canBatchFreeze = state.experimental && "freeze" in state.features && selectedEntries.isNotEmpty()
                HorizontalFloatingToolbar(
                    expanded = true,
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(bottom = 16.dp + bottomInset),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text(
                            "已选 ${state.selectedPids.size}",
                            style = MaterialTheme.typography.labelLarge,
                            modifier = Modifier.padding(horizontal = 8.dp),
                            maxLines = 1,
                        )
                        if (canBatchStop) {
                            FilledTonalButton(
                                onClick = {
                                    confirm = batchStopConfirm(packageCount) { viewModel.batchStop() }
                                },
                                enabled = state.actionRunning == null,
                                shapes = ButtonDefaults.shapes(),
                                contentPadding = ButtonDefaults.ExtraSmallContentPadding,
                            ) { Text("强停", maxLines = 1) }
                        }
                        if (canBatchFreeze) {
                            FilledTonalButton(
                                onClick = {
                                    confirm = batchFreezeConfirm(selectedEntries.size, true) {
                                        viewModel.batchFreeze(true)
                                    }
                                },
                                enabled = state.actionRunning == null,
                                shapes = ButtonDefaults.shapes(),
                                contentPadding = ButtonDefaults.ExtraSmallContentPadding,
                            ) { Text("冻结", maxLines = 1) }
                            FilledTonalButton(
                                onClick = {
                                    confirm = batchFreezeConfirm(selectedEntries.size, false) {
                                        viewModel.batchFreeze(false)
                                    }
                                },
                                enabled = state.actionRunning == null,
                                shapes = ButtonDefaults.shapes(),
                                contentPadding = ButtonDefaults.ExtraSmallContentPadding,
                            ) { Text("解冻", maxLines = 1) }
                        }
                        IconButton(onClick = viewModel::clearSelection) {
                            Icon(Icons.Rounded.Close, contentDescription = "退出多选")
                        }
                    }
                }
            }
        }
    }

    if (sheetOpen) {
        ProcessDetailSheet(
            state = state,
            onDismiss = viewModel::dismissDetail,
            onDismissMessage = viewModel::dismissMessage,
            onStop = { target, user -> confirm = stopConfirm(target) { viewModel.stop(target, user) } },
            onOom = { target, value -> confirm = oomConfirm(target, value) { viewModel.oom(target, value) } },
            onRenice = { target, value ->
                confirm = reniceConfirm(target, value) { viewModel.renice(target, value) }
            },
            onSched = { target, policy, priority ->
                confirm = schedConfirm(target, policy, priority) {
                    viewModel.sched(target, policy, priority)
                }
            },
            onCpuset = { target, group ->
                confirm = cpusetConfirm(target, group) { viewModel.cpuset(target, group) }
            },
            onFreeze = { target, frozen ->
                confirm = freezeConfirm(target, frozen) { viewModel.freeze(target, frozen) }
            },
        )
    }

    confirm?.let { request -> ConfirmDialog(request = request, onDismiss = { confirm = null }) }
}

// ---------------------------------------------------------------------------------------------
// Confirmation
// ---------------------------------------------------------------------------------------------

/** One confirmation: the icon sits in a cookie badge, and a risky action gets the error colors. */
@Composable
private fun ConfirmDialog(request: ProcessConfirm, onDismiss: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    AlertDialog(
        onDismissRequest = onDismiss,
        shape = MaterialTheme.shapes.extraLarge,
        containerColor = scheme.surfaceContainerHigh,
        icon = {
            Box(
                modifier = Modifier
                    .size(64.dp)
                    .clip(MaterialShapes.Cookie9Sided.toShape())
                    .background(if (request.danger) scheme.errorContainer else scheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    request.icon,
                    contentDescription = null,
                    tint = if (request.danger) scheme.onErrorContainer else scheme.onPrimaryContainer,
                    modifier = Modifier.size(30.dp),
                )
            }
        },
        title = { Text(request.title, style = MaterialTheme.typography.headlineSmallEmphasized) },
        text = { Text(request.text, style = MaterialTheme.typography.bodyMedium) },
        confirmButton = {
            Button(
                onClick = {
                    onDismiss()
                    request.onConfirm()
                },
                shapes = ButtonDefaults.shapes(),
                colors = if (request.danger) {
                    ButtonDefaults.buttonColors(
                        containerColor = scheme.error,
                        contentColor = scheme.onError,
                    )
                } else {
                    ButtonDefaults.buttonColors()
                },
            ) { Text(request.confirmLabel) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, shapes = ButtonDefaults.shapes()) { Text("取消") }
        },
    )
}

private data class ProcessConfirm(
    val title: String,
    val text: String,
    val confirmLabel: String,
    val icon: ImageVector,
    val danger: Boolean = false,
    val onConfirm: () -> Unit,
)

private fun stopConfirm(target: String, onConfirm: () -> Unit) = ProcessConfirm(
    title = "强制停止？",
    text = "$target 的未保存数据可能丢失；下次打开时应用会重新启动。",
    confirmLabel = "停止",
    icon = Icons.Rounded.DeleteForever,
    danger = true,
    onConfirm = onConfirm,
)

private fun oomConfirm(target: String, value: Int, onConfirm: () -> Unit) = ProcessConfirm(
    title = "设置 oom_score_adj = $value？",
    text = "$target 的优先级会立即改变；ActivityManager 可能在状态变化后重置。",
    confirmLabel = "应用",
    icon = Icons.Rounded.Tune,
    danger = true,
    onConfirm = onConfirm,
)

private fun reniceConfirm(target: String, value: Int, onConfirm: () -> Unit) = ProcessConfirm(
    title = "设置 nice = $value？",
    text = "$target 的调度优先级会立即改变；这是高风险实验设置。",
    confirmLabel = "应用",
    icon = Icons.Rounded.Speed,
    onConfirm = onConfirm,
)

private fun schedConfirm(
    target: String,
    policy: String,
    priority: Int?,
    onConfirm: () -> Unit,
) = ProcessConfirm(
    title = "修改调度策略为 $policy？",
    text = "$target 的调度策略将设为 $policy${priority?.takeIf { it > 0 }?.let { "（优先级 $it）" } ?: ""}；实时调度可能影响系统响应，ActivityManager 也可能随后重置。",
    confirmLabel = "应用",
    icon = Icons.Rounded.Bolt,
    danger = policy == "fifo" || policy == "rr",
    onConfirm = onConfirm,
)

private fun cpusetConfirm(target: String, group: String, onConfirm: () -> Unit) = ProcessConfirm(
    title = "移入 $group？",
    text = "$target 的进程将只使用该 cpuset 允许的核心；ActivityManager 可能随后改回。",
    confirmLabel = "移动",
    icon = Icons.Rounded.AccountTree,
    onConfirm = onConfirm,
)

private fun freezeConfirm(target: String, frozen: Boolean, onConfirm: () -> Unit) = ProcessConfirm(
    title = if (frozen) "冻结 $target？" else "解冻 $target？",
    text = if (frozen) {
        "被冻结的应用不会运行；系统可能随时解冻或重置。"
    } else {
        "应用恢复运行。"
    },
    confirmLabel = if (frozen) "冻结" else "解冻",
    icon = Icons.Rounded.Bedtime,
    danger = frozen,
    onConfirm = onConfirm,
)

private fun batchStopConfirm(packageCount: Int, onConfirm: () -> Unit) = ProcessConfirm(
    title = "批量强制停止 $packageCount 个应用？",
    text = "所选应用未保存的数据可能丢失；下次打开时会重新启动。",
    confirmLabel = "强停",
    icon = Icons.Rounded.DeleteForever,
    danger = true,
    onConfirm = onConfirm,
)

private fun batchFreezeConfirm(count: Int, frozen: Boolean, onConfirm: () -> Unit) = ProcessConfirm(
    title = if (frozen) "批量冻结 $count 项？" else "批量解冻 $count 项？",
    text = if (frozen) "被冻结的进程将停止运行；系统可能随时解冻。" else "所选进程将恢复运行。",
    confirmLabel = if (frozen) "冻结" else "解冻",
    icon = Icons.Rounded.Bedtime,
    danger = frozen,
    onConfirm = onConfirm,
)
```

---

#### 3. `app/src/test/kotlin/io/sysflow/ProcessModelsTest.kt`（新建测试文件）

```kotlin
package io.sysflow

import io.sysflow.feature.process.filterSignedDigits
import io.sysflow.feature.process.formatProcessCpu
import io.sysflow.feature.process.formatProcessKb
import io.sysflow.feature.process.isRealtimeSchedPolicy
import io.sysflow.feature.process.parseProcessAction
import io.sysflow.feature.process.parseProcessDetail
import io.sysflow.feature.process.parseProcessList
import io.sysflow.feature.process.processSubtitle
import io.sysflow.feature.process.schedPolicyArg
import io.sysflow.feature.process.schedPolicyLabel
import io.sysflow.feature.process.toggleNumberSign
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ProcessModelsTest {

    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun parses_process_list_including_frozen_and_foreground() {
        val raw = """
            {
              "available": true,
              "total": 2,
              "returned": 1,
              "truncated": true,
              "skipped": 0,
              "sample_ms": 300,
              "clock_ticks": 100,
              "cpuset_groups": ["top-app", "background"],
              "foreground": { "available": true, "package": "com.example.app", "pid": 8123, "user": 0, "cached": false, "disabled": false },
              "processes": [
                {
                  "pid": 8123, "ppid": 701, "uid": 10123, "user": 0, "app_id": 10123,
                  "package": "com.example.app", "name": "com.example.app", "cmdline": "com.example.app",
                  "state": "S", "threads": 12, "rss_kb": 136684, "cpu_pct": 12.5, "cpu_ticks": 38,
                  "oom_score_adj": 0, "cgroup": "/apps/uid_10123/pid_8123", "cpuset": "top-app",
                  "frozen": true, "foreground": true, "system": false, "protected": false
                }
              ]
            }
        """.trimIndent()
        val parsed = parseProcessList(json.parseToJsonElement(raw).jsonObject)
        assertTrue(parsed.available)
        assertTrue(parsed.truncated)
        assertEquals(listOf("top-app", "background"), parsed.cpusetGroups)
        assertEquals(1, parsed.entries.size)
        val row = parsed.entries.first()
        assertEquals(true, row.frozen)
        assertTrue(row.foreground)
        assertEquals("133.5 MB", formatProcessKb(row.rssKb))
        assertEquals("12.5%", formatProcessCpu(row.cpuPct))
        assertEquals("UID 10123 · 12 线程 · OOA 0", processSubtitle(row))
    }

    @Test
    fun parses_detail_with_policy_name_and_frozen_state() {
        val raw = """
            {
              "pid": 8123, "ppid": 701, "uid": 10123, "user": 0, "app_id": 10123,
              "package": "com.example.app", "packages": ["com.example.app"],
              "name": "com.example.app", "cmdline": ["com.example.app", "--worker"],
              "state": "S", "threads": 8, "rss_kb": 51200, "pss_kb": 40960, "swap_pss_kb": 0,
              "cpu_pct": 1.2, "sample_ms": 200, "oom_score_adj": 100,
              "nice": -5, "rt_priority": 10, "policy": 1, "policy_name": "SCHED_FIFO",
              "cgroup": "/apps/uid_10123/pid_8123", "cpuset": "top-app", "start_time_ticks": 999,
              "frozen": false, "foreground": false, "system": false, "protected": false
            }
        """.trimIndent()
        val detail = parseProcessDetail(json.parseToJsonElement(raw).jsonObject)
        assertEquals(false, detail.frozen)
        assertEquals("SCHED_FIFO", schedPolicyLabel(detail.policy, detail.policyName))
        assertEquals("fifo", schedPolicyArg(detail.policy))
        assertTrue(isRealtimeSchedPolicy(detail.policy))
        assertFalse(isRealtimeSchedPolicy(0))
        assertEquals(listOf("com.example.app", "--worker"), detail.cmdline)
    }

    @Test
    fun parses_sched_and_stop_action_results() {
        val schedRaw = """
            {
              "target": "com.example.app",
              "policy": "SCHED_RR",
              "policy_id": 2,
              "priority": 25,
              "targets": [
                { "pid": 8123, "before": "SCHED_OTHER (0)", "applied": "SCHED_RR (25)", "verified": true, "mechanism": "sched_setscheduler" }
              ]
            }
        """.trimIndent()
        val schedResult = parseProcessAction(json.parseToJsonElement(schedRaw).jsonObject)
        assertEquals("SCHED_RR", schedResult.policy)
        assertEquals(2, schedResult.policyId)
        assertEquals(25, schedResult.priority)
        assertEquals("SCHED_OTHER (0)", schedResult.targets.first().before)
        assertEquals("SCHED_RR (25)", schedResult.targets.first().applied)

        val stopRaw = """
            { "package": "com.example.app", "user": 0, "pids_before": [1, 2], "pids_after": [], "verified": true, "already_stopped": false }
        """.trimIndent()
        val stopResult = parseProcessAction(json.parseToJsonElement(stopRaw).jsonObject)
        assertEquals("com.example.app", stopResult.pkg)
        assertNull(stopResult.target)
        assertEquals(true, stopResult.verified)
    }

    @Test
    fun custom_numeric_helpers_handle_signs_and_bounds() {
        assertEquals("-15", filterSignedDigits(" -1a5b ", 3))
        assertEquals("123", filterSignedDigits("12-34", 3))
        assertEquals("-", toggleNumberSign("0"))
        assertEquals("", toggleNumberSign("-"))
        assertEquals("-10", toggleNumberSign("10"))
        assertEquals("10", toggleNumberSign("-10"))
    }
}
```