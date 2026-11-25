//! `sysflow mem breakdown` — how the app-visible memory splits between system and user apps.
//!
//! Unlike `mem apps` (ranked by RSS, PSS only for the head of the list), this measures **every**
//! process: a "system vs user" split computed from a truncated list would lie in whichever direction
//! the cut fell. That costs a few hundred milliseconds, so it is a verb with a 60 s cache and never a
//! `snapshot` item.
//!
//! The split is by UID, which is the only classification the kernel gives for free:
//!
//! * `uid < 10 000` — system / native daemons (`system_server`, `surfaceflinger`, `init`, …);
//! * `uid >= 10 000` — apps, including work-profile copies (their UID carries a user prefix).
//!
//! The numbers are PSS, so they are **not** a share of "used memory": shared pages are counted
//! fractionally per process, and kernel / GPU / vendor pools are not app memory at all. The page
//! presents this as its own layer for that reason (protocol §3.24).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use sysflow_core::cache;
use sysflow_core::error::Result;
use sysflow_core::registry::Registry;
use sysflow_core::sysfs::Sysfs;

use super::mem_apps;

/// First UID that belongs to an installed app rather than to the platform.
const FIRST_APP_UID: u32 = 10_000;

pub fn run(reg: &Registry, refresh: bool) -> Result<Value> {
    run_with(&reg.ctx().sysfs, &reg.ctx().data_dir, refresh)
}

fn run_with(fs: &Sysfs, data_dir: &Path, refresh: bool) -> Result<Value> {
    let path = cache_path(data_dir);
    if !refresh {
        if let Some(cached) = cache::read_fresh(&path, cache::TTL_SECS) {
            let mut value = cached;
            value["cached"] = json!(true);
            return Ok(value);
        }
    }

    let mut skipped = 0usize;
    let processes = mem_apps::scan_processes(fs, &mut skipped);
    if processes.is_empty() {
        return Ok(json!({
            "available": false,
            "reason": "/proc is not readable",
            "system_pss_kb": 0,
            "user_pss_kb": 0,
            "total_pss_kb": 0,
            "processes": 0,
        }));
    }

    let mut system_pss_kb = 0u64;
    let mut user_pss_kb = 0u64;
    let mut measured = 0usize;
    let mut estimated = 0usize;
    for proc in &processes {
        let pss_kb = match fs.read_lossy(&format!("/proc/{}/smaps_rollup", proc.pid)) {
            Ok(text) => match mem_apps::parse_smaps_rollup(&text) {
                Some((pss_kb, _swap)) => {
                    measured += 1;
                    pss_kb
                }
                // A rollup without `Pss:` still has a `VmRSS` behind it; using it keeps the sum
                // complete, and `estimated` tells the client how much of it is approximate.
                None => {
                    estimated += 1;
                    proc.rss_kb
                }
            },
            // No rollup (an old kernel, or a domain SELinux will not let us read): keep the sum
            // complete with `VmRSS` and say how much of it is approximate.
            Err(_) => {
                estimated += 1;
                proc.rss_kb
            }
        };
        if proc.uid < FIRST_APP_UID {
            system_pss_kb += pss_kb;
        } else {
            user_pss_kb += pss_kb;
        }
    }

    let mut value = json!({
        "available": true,
        "cached": false,
        "generated_at": cache::now_secs(),
        "system_pss_kb": system_pss_kb,
        "user_pss_kb": user_pss_kb,
        "total_pss_kb": system_pss_kb + user_pss_kb,
        "processes": processes.len(),
        "measured": measured,
        "estimated": estimated,
        "skipped": skipped,
    });
    if let Some(object) = value.as_object_mut() {
        cache::write(&path, &Value::Object(object.clone()));
    }
    Ok(value)
}

fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("state").join("mem_breakdown.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tree {
        base: std::path::PathBuf,
        root: std::path::PathBuf,
        data: std::path::PathBuf,
    }

    impl Tree {
        fn new(name: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "sysflow-mem-breakdown-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            let tree = Tree {
                base: base.clone(),
                root: base.join("root"),
                data: base.join("data"),
            };
            std::fs::create_dir_all(tree.root.join("proc")).unwrap();
            std::fs::create_dir_all(&tree.data).unwrap();
            tree
        }

        fn proc(&self, pid: u32, uid: u32, rss_kb: u64, pss_kb: Option<u64>) {
            let dir = self.root.join(format!("proc/{pid}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("status"),
                format!("Name:\tp{pid}\nPid:\t{pid}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nVmRSS:\t{rss_kb} kB\n"),
            )
            .unwrap();
            if let Some(pss) = pss_kb {
                std::fs::write(
                    dir.join("smaps_rollup"),
                    format!("Rss: 1 kB\nPss: {pss} kB\nSwapPss: 0 kB\n"),
                )
                .unwrap();
            }
        }

        fn fs(&self) -> Sysfs {
            Sysfs::new(&self.root)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    fn the_split_is_by_uid_and_covers_every_process() {
        let tree = Tree::new("split");
        tree.proc(100, 1000, 500_000, Some(480_000)); // system_server
        tree.proc(200, 10346, 412_000, Some(380_000)); // a user app
        tree.proc(300, 9_510_346, 100_000, Some(90_000)); // the same app in a work profile
        tree.proc(400, 1000, 50_000, None); // rollup missing: estimated from RSS

        let value = run_with(&tree.fs(), &tree.data, false).unwrap();
        assert_eq!(value["available"], true);
        // 480 000 measured + 50 000 estimated share the system side.
        assert_eq!(value["system_pss_kb"], 530_000);
        // Both user uids land on the user side, work profile included.
        assert_eq!(value["user_pss_kb"], 470_000);
        assert_eq!(value["total_pss_kb"], 1_000_000);
        assert_eq!(value["processes"], 4);
        assert_eq!(value["estimated"], 1);
        assert_eq!(value["skipped"], 0);
    }

    #[test]
    fn results_are_cached_and_refresh_bypasses() {
        let tree = Tree::new("cache");
        tree.proc(100, 1000, 1_000, Some(900));
        let first = run_with(&tree.fs(), &tree.data, false).unwrap();
        assert_eq!(first["cached"], false);
        let second = run_with(&tree.fs(), &tree.data, false).unwrap();
        assert_eq!(second["cached"], true);
        assert_eq!(second["system_pss_kb"], first["system_pss_kb"]);
        let refreshed = run_with(&tree.fs(), &tree.data, true).unwrap();
        assert_eq!(refreshed["cached"], false);
    }

    #[test]
    fn an_unreadable_proc_is_unavailable_rather_than_empty() {
        let tree = Tree::new("empty");
        std::fs::remove_dir_all(tree.root.join("proc")).unwrap();
        let value = run_with(&tree.fs(), &tree.data, false).unwrap();
        assert_eq!(value["available"], false);
        assert_eq!(value["total_pss_kb"], 0);
    }

    #[test]
    fn a_vanished_process_is_skipped_not_counted_as_zero() {
        let tree = Tree::new("skipped");
        tree.proc(100, 10346, 10_000, Some(9_000));
        // A pid directory with no `status` cannot contribute anything.
        std::fs::create_dir_all(tree.root.join("proc/999")).unwrap();
        let value = run_with(&tree.fs(), &tree.data, false).unwrap();
        assert_eq!(value["skipped"], 1);
        assert_eq!(value["user_pss_kb"], 9_000);
    }
}
