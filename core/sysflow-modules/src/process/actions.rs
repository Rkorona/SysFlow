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

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use sysflow_core::command;
use sysflow_core::error::{Error, Result};
use sysflow_core::event::{EventStore, NewSystemEvent};
use sysflow_core::log::Logger;
use sysflow_core::sysfs::Sysfs;

use super::foreground;
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
            return Err(
                Error::invalid_arg(format!("{canonical_name} requires priority 0"))
                    .with_detail(json!({"range": [0, 0]})),
            );
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

/// One `stop-all` candidate set: every app package whose running processes all pass the
/// protection gate, keyed by package, plus what was left out and why.
pub(crate) struct StopAllSelection {
    pub packages: BTreeMap<String, Vec<scan::ProcessInfo>>,
    /// Processes with a package that the gate refused (denylist, SysFlow itself, ...). Kernel
    /// threads and platform processes have no package, so they are skipped silently above.
    pub skipped_protected: usize,
    /// The foreground package, recorded when it was kept out (the default).
    pub foreground: Option<String>,
    pub unreadable: usize,
}

/// Everything `stop-all` may stop: a live app package, not protected, and — unless asked — not the
/// package that is on screen right now.
pub(crate) fn select_stoppable(
    fs: &Sysfs,
    data_dir: &Path,
    include_foreground: bool,
) -> Result<StopAllSelection> {
    let (infos, unreadable) = scan::read_all(fs)?;
    let packages = packages::read(fs, data_dir);
    let foreground = if include_foreground {
        None
    } else {
        foreground::current(data_dir).package
    };

    let mut selection = StopAllSelection {
        packages: BTreeMap::new(),
        skipped_protected: 0,
        foreground,
        unreadable: unreadable as usize,
    };
    for info in infos {
        let Some(package) = package_of(&info, &packages) else {
            continue;
        };
        if model::protected_reason(info.uid, info.pid, info.kthread, &info.name, Some(&package))
            .is_some()
        {
            selection.skipped_protected += 1;
            continue;
        }
        if selection.foreground.as_deref() == Some(package.as_str()) {
            continue;
        }
        selection.packages.entry(package).or_default().push(info);
    }
    Ok(selection)
}

/// `process stop-all` — one-shot force-stop of every stoppable app package.
///
/// The protection gate decides the set, so "all" never touches protected processes: kernel and
/// platform processes have no package to stop, and the denylist / SysFlow itself are refused by
/// `select_stoppable`. The foreground package is kept unless `include_foreground`. All `am` calls
/// run first, then one shared verification window covers every targeted PID; a package whose `am`
/// call failed is reported in `failed` and excluded from the verification.
pub fn stop_all(fs: &Sysfs, data_dir: &Path, include_foreground: bool) -> Result<Value> {
    let started = Instant::now();
    let selection = select_stoppable(fs, data_dir, include_foreground)?;
    let package_count = selection.packages.len();

    if package_count == 0 {
        log_action(data_dir, "process stop-all packages=0".to_string());
        return Ok(json!({
            "stopped": [],
            "packages": 0,
            "skipped_protected": selection.skipped_protected,
            "skipped_unreadable": selection.unreadable,
            "skipped_foreground": selection.foreground,
            "failed": [],
            "pids_after": [],
            "verified": true,
            "elapsed_ms": started.elapsed().as_millis() as u64,
        }));
    }

    let mut stopped: Vec<String> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    let mut pids_before = 0usize;
    for (package, procs) in &selection.packages {
        pids_before += procs.len();
        if let Err(error) = validate_package(package) {
            failed.push(json!({"package": package, "error": error.to_string()}));
            continue;
        }
        let user = procs[0].uid / 100_000;
        match command::run(
            "am",
            &["force-stop", "--user", &user.to_string(), package],
            AM_TIMEOUT,
        ) {
            Ok(_) => stopped.push(package.clone()),
            Err(error) => failed.push(json!({"package": package, "error": error.to_string()})),
        }
    }

    let stopped_set: HashSet<&String> = stopped.iter().collect();
    let targets: Vec<&scan::ProcessInfo> = selection
        .packages
        .iter()
        .filter(|(package, _)| stopped_set.contains(package))
        .flat_map(|(_, procs)| procs.iter())
        .collect();
    let verify_started = Instant::now();
    let pids_after = loop {
        let remaining: Vec<u32> = targets
            .iter()
            .filter(|info| still_same_process(fs, info))
            .map(|info| info.pid)
            .collect();
        if remaining.is_empty() || verify_started.elapsed() >= STOP_VERIFY {
            break remaining;
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let verified = pids_after.is_empty();
    log_action(
        data_dir,
        format!(
            "process stop-all packages={package_count} stopped={} failed={} before={pids_before} after={}",
            stopped.len(),
            failed.len(),
            pids_after.len()
        ),
    );
    emit_process_event(
        data_dir,
        if verified { "info" } else { "warning" },
        if verified {
            "Processes stopped"
        } else {
            "Stop-all incomplete"
        },
        if verified {
            format!("Force-stopped {} packages.", stopped.len())
        } else {
            format!(
                "Stop-all requested for {} packages, {} pids survived.",
                stopped.len(),
                pids_after.len()
            )
        },
        json!({
            "action": "stop_all",
            "target": "stop-all",
            "packages": package_count,
            "stopped": stopped.len(),
            "skipped_protected": selection.skipped_protected,
            "verified": verified,
        }),
    );

    Ok(json!({
        "stopped": stopped,
        "packages": package_count,
        "skipped_protected": selection.skipped_protected,
        "skipped_unreadable": selection.unreadable,
        "skipped_foreground": selection.foreground,
        "failed": failed,
        "pids_after": pids_after,
        "verified": verified,
        "elapsed_ms": started.elapsed().as_millis() as u64,
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
            return Err(
                failure.unwrap_or_else(|| Error::io("no cgroup-v1 freezer write succeeded"))
            );
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
    use sysflow_core::cache;

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
    fn stop_all_selects_only_unprotected_packages_and_keeps_the_foreground() {
        let root = temp("stop-all-select");
        add_process(&root, 10101, "com.example.game", 10101);
        add_process(&root, 10102, "com.example.notes", 10102);
        add_process(&root, 10213, "com.android.systemui", 10157);
        add_process(&root, 10500, "sysflow", 10100);
        add_process(&root, 300, "netd", 1000);
        write(
            &root,
            "data/system/packages.list",
            "com.example.game 10101 0 /data 0 0 1 0 0 0\ncom.example.notes 10102 0 /data 0 0 1 0 0 0\ncom.android.systemui 10157 0 /data 0 0 1 0 0 0\n",
        );
        let fs = Sysfs::new(&root);
        let data = root.join("data-dir");

        let selection = select_stoppable(&fs, &data, true).unwrap();
        let names: Vec<&String> = selection.packages.keys().collect();
        assert_eq!(names, ["com.example.game", "com.example.notes"]);
        // Only processes with a package the gate refused are counted: systemui (denylist). The
        // sysflow binary's cmdline maps to no package, so it is skipped silently, like netd and
        // pid 1 (no package either) — none of them can be force-stopped anyway.
        assert_eq!(selection.skipped_protected, 1);

        // With the foreground lookup, com.example.game is the resumed app and drops out. The
        // cache lives under the data dir, which is where `foreground::current` reads it.
        write(
            &data,
            "state/process_foreground.json",
            &format!(
                "{{\"package\":\"com.example.game\",\"user\":0,\"generated_at\":{}}}",
                cache::now_secs()
            ),
        );
        let with_fg = select_stoppable(&fs, &data, false).unwrap();
        assert_eq!(
            with_fg.packages.keys().collect::<Vec<_>>(),
            ["com.example.notes"]
        );
        assert_eq!(with_fg.foreground.as_deref(), Some("com.example.game"));
    }

    #[test]
    fn stop_all_without_candidates_is_an_empty_success() {
        let root = temp("stop-all-empty");
        add_process(&root, 10213, "com.android.systemui", 10157);
        write(
            &root,
            "data/system/packages.list",
            "com.android.systemui 10157 0 /data 0 0 1 0 0 0\n",
        );
        let fs = Sysfs::new(&root);
        let out = stop_all(&fs, &root.join("data-dir"), true).unwrap();
        assert_eq!(out["packages"], 0);
        assert_eq!(out["stopped"], serde_json::json!([]));
        assert_eq!(out["verified"], true);
        assert_eq!(out["skipped_protected"], 1);
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

        let bad_other_prio = sched(&fs, &data, &Target::Pid(10123), "other", Some(10)).unwrap_err();
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
