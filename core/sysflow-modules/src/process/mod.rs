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
        let mut features = vec!["list"];
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
            "process is verb-only: use `sysflow process list|get|stop|oom|renice`",
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
