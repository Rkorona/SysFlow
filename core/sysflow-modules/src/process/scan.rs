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
    let name = if !cmdline.is_empty() {
        cmdline.clone()
    } else if !status.comm.is_empty() {
        status.comm.clone()
    } else {
        format!("pid {pid}")
    };
    let oom_score_adj = fs
        .read_lossy(&format!("/proc/{pid}/oom_score_adj"))
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok());
    let cgroup = fs
        .read_lossy(&format!("/proc/{pid}/cgroup"))
        .ok()
        .map(|text| model::parse_cgroup(&text));
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
    })
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
            let package = packages.primary(info.uid).map(str::to_string);
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
    let package = installed.primary(info.uid).map(str::to_string);
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
}
