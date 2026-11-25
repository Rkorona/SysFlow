//! `sysflow process probe` — hidden Step 1 diagnostic for the process module.
//!
//! Not part of the protocol and its output may change between steps: it exists to replace the
//! "待测" cells in `docs/modules/process.md` with measured facts (does `/proc` expose every process,
//! what is `CLK_TCK`, how does this ROM print the resumed activity, is there a freezer, which
//! `am` / `renice` subcommands exist, how long does a scan take). Run it on the device, save the
//! JSON, and fill the matrix before writing the list/action code.

use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use sysflow_core::command;
use sysflow_core::error::Result;
use sysflow_core::registry::Registry;
use sysflow_core::sysfs::Sysfs;

/// `sysconf(3)` names; declared directly so the binary keeps its zero-external-crate dependency set.
const SC_CLK_TCK: i32 = 2;
const SC_PAGESIZE: i32 = 30;

extern "C" {
    fn sysconf(name: i32) -> i64;
}

pub fn run(reg: &Registry) -> Result<Value> {
    Ok(collect(&reg.ctx().sysfs))
}

fn collect(fs: &Sysfs) -> Value {
    let started = Instant::now();
    let mut out = Map::new();
    out.insert("proc".into(), proc_section(fs));
    out.insert("libc".into(), libc_section());
    out.insert("samples".into(), samples_section(fs));
    out.insert("foreground".into(), foreground_section());
    out.insert("packages_list".into(), packages_section(fs));
    out.insert("cpuset".into(), cpuset_section(fs));
    out.insert("freezer".into(), freezer_section(fs));
    out.insert("commands".into(), commands_section(fs));
    out.insert(
        "total_ms".into(),
        json!(started.elapsed().as_millis() as u64),
    );
    Value::Object(out)
}

/// Is `/proc/<pid>/stat` readable for every numeric entry? This is the module's hard requirement.
fn proc_section(fs: &Sysfs) -> Value {
    let started = Instant::now();
    let mut total = 0usize;
    let mut readable = 0usize;
    let mut more_unreadable: Vec<Value> = Vec::new();
    let mut first_unreadable = Value::Null;

    for name in fs.list_dir("/proc").unwrap_or_default() {
        if !name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        total += 1;
        match fs.read_lossy(&format!("/proc/{name}/stat")) {
            Ok(_) => readable += 1,
            Err(e) => {
                if first_unreadable.is_null() {
                    first_unreadable = json!({"pid": name, "error": e.to_string()});
                } else if more_unreadable.len() < 4 {
                    more_unreadable.push(json!({"pid": name, "error": e.to_string()}));
                }
            }
        }
    }

    json!({
        "total": total,
        "readable": readable,
        "unreadable": total.saturating_sub(readable),
        "first_unreadable": first_unreadable,
        "more_unreadable": more_unreadable,
        "scan_ms": started.elapsed().as_millis() as u64,
    })
}

fn libc_section() -> Value {
    json!({
        "clock_ticks": sysconf_value(SC_CLK_TCK, 100),
        "page_size": sysconf_value(SC_PAGESIZE, 4096),
    })
}

fn sysconf_value(name: i32, fallback: i64) -> Value {
    // SAFETY: `sysconf` is a thread-safe libc query with no memory arguments.
    let value = unsafe { sysconf(name) };
    if value > 0 {
        json!({"value": value, "source": "sysconf"})
    } else {
        json!({"value": fallback, "source": "fallback"})
    }
}

/// Raw files for representative processes: self, init, kthreadd, and the first app process found.
fn samples_section(fs: &Sysfs) -> Value {
    let mut pids = vec![
        std::process::id().to_string(),
        "1".to_string(),
        "2".to_string(),
    ];
    if let Some(pid) = first_app_pid(fs) {
        if !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    Value::Array(pids.iter().map(|pid| sample(fs, pid)).collect())
}

fn first_app_pid(fs: &Sysfs) -> Option<String> {
    for name in fs.list_dir("/proc").ok()?.into_iter().take(300) {
        if !name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(status) = fs.read_lossy(&format!("/proc/{name}/status")) else {
            continue;
        };
        let uid = status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|uid| uid.parse::<u32>().ok());
        if uid.is_some_and(|uid| uid >= 10_000) {
            return Some(name);
        }
    }
    None
}

fn sample(fs: &Sysfs, pid: &str) -> Value {
    let mut out = Map::new();
    out.insert("pid".into(), json!(pid));
    let mut errors = Map::new();

    let sources = [
        ("stat", format!("/proc/{pid}/stat")),
        ("status", format!("/proc/{pid}/status")),
        ("cmdline", format!("/proc/{pid}/cmdline")),
        ("oom_score_adj", format!("/proc/{pid}/oom_score_adj")),
        ("cgroup", format!("/proc/{pid}/cgroup")),
    ];
    for (key, path) in sources {
        match fs.read_lossy(&path) {
            Ok(text) => {
                let text = if key == "cmdline" {
                    text.replace('\0', " ")
                } else {
                    text
                };
                out.insert(key.into(), json!(limit(&text, 4096)));
            }
            Err(e) => {
                errors.insert(path, json!(e.to_string()));
            }
        }
    }
    if !errors.is_empty() {
        out.insert("errors".into(), Value::Object(errors));
    }
    Value::Object(out)
}

fn foreground_section() -> Value {
    let started = Instant::now();
    match command::run(
        "dumpsys",
        &["activity", "activities"],
        Duration::from_secs(3),
    ) {
        Ok(text) => {
            let lines: Vec<&str> = text
                .lines()
                .filter(|l| l.contains("topResumedActivity") || l.contains("mResumedActivity"))
                .take(3)
                .collect();
            let parsed = lines.iter().find_map(|line| parse_foreground(line));
            json!({
                "available": true,
                "source": "dumpsys",
                "lines": lines,
                "parsed": parsed.map(|(package, user)| json!({
                    "package": package,
                    "user": user,
                })),
                "ms": started.elapsed().as_millis() as u64,
            })
        }
        Err(e) => json!({
            "available": false,
            "source": "none",
            "error": e.to_string(),
            "ms": started.elapsed().as_millis() as u64,
        }),
    }
}

/// `topResumedActivity=ActivityRecord{e8a3f1 u0 com.android.settings/.Settings t42}`
fn parse_foreground(line: &str) -> Option<(String, u32)> {
    let user = line
        .split_whitespace()
        .find(|token| {
            token.len() > 1
                && token.starts_with('u')
                && token[1..].chars().all(|c| c.is_ascii_digit())
        })
        .and_then(|token| token[1..].parse().ok())
        .unwrap_or(0);
    let token = line.split_whitespace().find(|token| {
        token.contains('/')
            && token
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
    })?;
    let package = token.split('/').next()?;
    (!package.is_empty()).then(|| (package.to_string(), user))
}

fn packages_section(fs: &Sysfs) -> Value {
    let started = Instant::now();
    const PATH: &str = "/data/system/packages.list";
    match fs.read_lossy(PATH) {
        Ok(text) => {
            let non_empty = || text.lines().filter(|line| !line.trim().is_empty());
            let first = non_empty().next().unwrap_or("");
            json!({
                "readable": true,
                "path": PATH,
                "lines": non_empty().count(),
                "columns_first_line": first.split_whitespace().count(),
                "first_line": first,
                "ms": started.elapsed().as_millis() as u64,
            })
        }
        Err(e) => json!({
            "readable": false,
            "path": PATH,
            "error": e.to_string(),
            "ms": started.elapsed().as_millis() as u64,
        }),
    }
}

fn cpuset_section(fs: &Sysfs) -> Value {
    let groups = fs.list_dir("/dev/cpuset").unwrap_or_default();
    let mut with_tasks = Vec::new();
    for group in &groups {
        let tasks = format!("/dev/cpuset/{group}/tasks");
        if !fs.exists(&tasks) {
            continue;
        }
        with_tasks.push(json!({
            "group": group,
            "cpus": fs.read_lossy(&format!("/dev/cpuset/{group}/cpus"))
                .ok()
                .map(|text| text.trim().to_string()),
            "owner_write": fs.mode(&tasks).map(|mode| mode & 0o200 != 0).unwrap_or(false),
        }));
    }
    json!({
        "root": "/dev/cpuset",
        "groups": groups,
        "with_tasks": with_tasks,
    })
}

fn freezer_section(fs: &Sysfs) -> Value {
    let started = Instant::now();
    let mut candidates = Vec::new();
    let roots = ["/sys/fs/cgroup/apps", "/sys/fs/cgroup"];
    let mut roots_seen = Vec::new();
    for root in roots {
        if !fs.exists(root) {
            roots_seen.push(json!({"root": root, "exists": false}));
            continue;
        }
        roots_seen.push(json!({"root": root, "exists": true}));
        for uid in fs
            .list_dir(root)
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.starts_with("uid_"))
            .take(8)
        {
            let Ok(pids) = fs.list_dir(&format!("{root}/{uid}")) else {
                continue;
            };
            for pid in pids
                .into_iter()
                .filter(|entry| entry.starts_with("pid_"))
                .take(4)
            {
                let path = format!("{root}/{uid}/{pid}/cgroup.freeze");
                if fs.exists(&path) {
                    let value = fs
                        .read_lossy(&path)
                        .ok()
                        .map(|text| text.trim().to_string());
                    candidates.push(json!({"path": path, "value": value}));
                }
            }
        }
    }
    json!({
        "roots": roots_seen,
        "candidates": candidates,
        "available": !candidates.is_empty(),
        "v1_dev_freezer": fs.exists("/dev/freezer"),
        "scan_ms": started.elapsed().as_millis() as u64,
    })
}

fn commands_section(fs: &Sysfs) -> Value {
    let mut out = Map::new();
    for bin in [
        "am",
        "dumpsys",
        "renice",
        "chrt",
        "getenforce",
        "getconf",
        "top",
    ] {
        let path = format!("/system/bin/{bin}");
        let present = fs.exists(&path);
        out.insert(bin.to_string(), json!({"path": path, "present": present}));
    }
    out.insert("getconf_clk_tck".into(), run_cmd("getconf", &["CLK_TCK"]));
    out.insert("getenforce".into(), run_cmd("getenforce", &[]));
    out.insert("renice_help".into(), run_cmd("renice", &["--help"]));
    out.insert(
        "am_help_freeze".into(),
        match command::run("am", &["help"], Duration::from_secs(3)) {
            Ok(text) => json!({
                "ok": true,
                "has_freeze": text.contains("freeze") || text.contains("unfreeze"),
                "first_lines": head(&text, 5),
            }),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        },
    );
    Value::Object(out)
}

fn run_cmd(bin: &str, args: &[&str]) -> Value {
    let started = Instant::now();
    match command::run(bin, args, Duration::from_secs(3)) {
        Ok(text) => json!({
            "ok": true,
            "ms": started.elapsed().as_millis() as u64,
            "first_lines": head(&text, 3),
        }),
        Err(e) => json!({
            "ok": false,
            "ms": started.elapsed().as_millis() as u64,
            "error": e.to_string(),
        }),
    }
}

fn head(text: &str, lines: usize) -> Vec<String> {
    text.lines()
        .take(lines)
        .map(|line| line.trim_end().to_string())
        .collect()
}

fn limit(text: &str, chars: usize) -> String {
    let mut out: String = text.chars().take(chars).collect();
    if text.chars().count() > chars {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sysflow-process-probe-{}-{name}",
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

    #[test]
    fn foreground_parses_both_android_line_shapes() {
        assert_eq!(
            parse_foreground(
                "  topResumedActivity=ActivityRecord{e8a3f1 u10 com.example/.Main t42}"
            ),
            Some(("com.example".to_string(), 10))
        );
        assert_eq!(
            parse_foreground(
                "  mResumedActivity: ActivityRecord{1a2b3c u0 com.android.settings/.Settings t7}"
            ),
            Some(("com.android.settings".to_string(), 0))
        );
        assert_eq!(parse_foreground("  no activity here"), None);
    }

    #[test]
    fn probe_reports_raw_sources_and_degrades_without_dumpsys() {
        let root = temp_root("sources");
        write(
            &root,
            "proc/1/stat",
            "1 (init) S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        );
        write(&root, "proc/1/status", "Name:\tinit\nUid:\t0\t0\t0\t0\n");
        write(&root, "proc/1/cmdline", "/system/bin/init\0");
        write(&root, "proc/1/cgroup", "0::/\n");
        write(&root, "proc/1/oom_score_adj", "0\n");
        write(
            &root,
            "data/system/packages.list",
            "com.example 10123 0 /data/user/0/com.example default:targetSdkVersion=34 0 1 0 0 0\n",
        );
        write(&root, "system/bin/am", "");
        write(&root, "dev/cpuset/top-app/tasks", "");
        write(&root, "dev/cpuset/top-app/cpus", "0-7\n");

        let fs = Sysfs::new(&root);
        let out = collect(&fs);

        assert_eq!(out["proc"]["total"], 1);
        assert_eq!(out["proc"]["readable"], 1);
        assert_eq!(out["proc"]["unreadable"], 0);
        assert_eq!(out["packages_list"]["readable"], true);
        assert_eq!(out["packages_list"]["lines"], 1);
        assert_eq!(out["packages_list"]["columns_first_line"], 10);
        // No dumpsys on the host: the section degrades instead of failing the probe.
        assert_eq!(out["foreground"]["available"], false);
        assert_eq!(out["cpuset"]["with_tasks"].as_array().unwrap().len(), 1);
        assert_eq!(out["commands"]["am"]["present"], true);

        let samples = out["samples"].as_array().unwrap();
        assert_eq!(samples.len(), 3, "self, pid 1 and pid 2 entries");
        assert!(samples.iter().any(|s| s["pid"] == "1"));
    }
}
