//! `sysflow mem apps` — per-UID memory ranking.
//!
//! Two-stage by design (protocol §3.23): every process is ranked by `VmRSS`, which is one small read
//! of `/proc/<pid>/status`, and only the top `limit` are read from `smaps_rollup` for `Pss` and
//! `SwapPss`. Reading PSS for a few hundred processes would blow the latency budget; RSS is a good
//! enough ordering key and is what the unmeasured tail is estimated from (that is exactly what
//! `pss_for_top_n_only` tells the client).
//!
//! Nothing here writes system state: it only reads `/proc` and `/data/system/packages.list`, and the
//! result is cached for a minute under `<data_dir>/state/mem_apps.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use sysflow_core::batterystats::{aid_label, read_package_uids};
use sysflow_core::cache;
use sysflow_core::error::Result;
use sysflow_core::registry::Registry;
use sysflow_core::sysfs::Sysfs;

pub const DEFAULT_LIMIT: usize = 30;
const MAX_LIMIT: usize = 100;
/// The package database is a plain file on every Android release so far; it is the primary source.
const PACKAGES_LIST: &str = "/data/system/packages.list";
/// UID → package changes slowly (install/uninstall), so its cache outlives the 60 s result cache.
/// It also keeps the `cmd package` fallback from paying a binder call on every cold ranking.
const PACKAGES_TTL_SECS: u64 = 10 * 60;

/// One process as far as the scan cares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Proc {
    pub(crate) pid: u32,
    pub(crate) uid: u32,
    pub(crate) rss_kb: u64,
}

/// Per-UID aggregate accumulated across that app's processes.
#[derive(Debug, Default, Clone)]
struct Agg {
    pss_kb: u64,
    swap_pss_kb: u64,
    procs: u32,
    /// At least one process was estimated from RSS because it fell below the PSS cut.
    estimated: bool,
}

pub fn run(reg: &Registry, limit: usize, refresh: bool) -> Result<Value> {
    run_with(&reg.ctx().sysfs, &reg.ctx().data_dir, limit, refresh)
}

/// The testable core: everything `run` does, without needing a `Registry`.
fn run_with(fs: &Sysfs, data_dir: &Path, limit: usize, refresh: bool) -> Result<Value> {
    let limit = limit.clamp(1, MAX_LIMIT);
    let path = cache_path(data_dir);
    if !refresh {
        if let Some(cached) = read_cache(&path, limit) {
            return Ok(cached);
        }
    }

    let mut skipped = 0usize;
    let mut processes = scan_processes(fs, &mut skipped);
    if processes.is_empty() {
        return Ok(json!({
            "available": false,
            "reason": "/proc is not readable",
            "limit": limit,
            "entries": [],
            "skipped": skipped,
        }));
    }

    // Stage one: RSS decides who is worth measuring.
    processes.sort_by(|a, b| b.rss_kb.cmp(&a.rss_kb).then_with(|| a.pid.cmp(&b.pid)));
    let measured = processes.len().min(limit);
    let mut pss = BTreeMap::new();
    for proc in &processes[..measured] {
        match fs.read_lossy(&format!("/proc/{}/smaps_rollup", proc.pid)) {
            Ok(text) => {
                // A rollup without `Pss:` is unusable; such a process keeps its RSS estimate.
                if let Some((pss_kb, swap_kb)) = parse_smaps_rollup(&text) {
                    pss.insert(proc.pid, (pss_kb, swap_kb));
                }
            }
            // A process that exited mid-scan is normal, not an error. Counted, never fatal.
            Err(_) => skipped += 1,
        }
    }

    // Stage two: aggregate by UID, estimating the tail from RSS.
    let mut by_uid: BTreeMap<u32, Agg> = BTreeMap::new();
    for proc in &processes {
        let agg = by_uid.entry(proc.uid).or_default();
        agg.procs += 1;
        match pss.get(&proc.pid) {
            Some((pss_kb, swap_kb)) => {
                agg.pss_kb += pss_kb;
                agg.swap_pss_kb += swap_kb;
            }
            None => {
                agg.pss_kb += proc.rss_kb;
                agg.estimated = true;
            }
        }
    }

    let total_pss_kb: u64 = by_uid.values().map(|agg| agg.pss_kb).sum();
    let packages = read_packages(fs, data_dir);
    let mut rows: Vec<(&u32, &Agg)> = by_uid.iter().collect();
    rows.sort_by(|a, b| b.1.pss_kb.cmp(&a.1.pss_kb).then_with(|| a.0.cmp(b.0)));

    let entries: Vec<Value> = rows
        .iter()
        .take(limit)
        .map(|(uid, agg)| {
            let names = packages.get(&(*uid % 100_000)).cloned().unwrap_or_default();
            let mut object = Map::new();
            object.insert("uid".into(), json!(uid_label(**uid)));
            object.insert("numeric_uid".into(), json!(uid));
            object.insert("name".into(), json!(label(**uid, &names)));
            object.insert("packages".into(), json!(names));
            object.insert("pss_kb".into(), json!(agg.pss_kb));
            object.insert("swap_pss_kb".into(), json!(agg.swap_pss_kb));
            object.insert("procs".into(), json!(agg.procs));
            object.insert(
                "share".into(),
                json!(if total_pss_kb == 0 {
                    0.0
                } else {
                    (agg.pss_kb as f64 * 1000.0 / total_pss_kb as f64).round() / 1000.0
                }),
            );
            Value::Object(object)
        })
        .collect();

    let value = json!({
        "available": true,
        "cached": false,
        "generated_at": cache::now_secs(),
        "limit": limit,
        "total_pss_kb": total_pss_kb,
        "skipped": skipped,
        // True when the tail was estimated from RSS instead of measured: the client must not present
        // those numbers as exact PSS.
        "pss_for_top_n_only": by_uid.values().any(|agg| agg.estimated),
        "entries": entries,
    });
    if let Some(object) = value.as_object() {
        write_cache(&path, object);
    }
    Ok(value)
}

/// Every numeric `/proc` entry with a readable UID and `VmRSS`.
pub(crate) fn scan_processes(fs: &Sysfs, skipped: &mut usize) -> Vec<Proc> {
    let mut out = Vec::new();
    let Ok(names) = fs.list_dir("/proc") else {
        return out;
    };
    for name in names {
        if !name.chars().all(|character| character.is_ascii_digit()) {
            continue;
        }
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        match fs.read_lossy(&format!("/proc/{pid}/status")) {
            Ok(text) => {
                let Some(uid) = parse_uid(&text) else {
                    *skipped += 1;
                    continue;
                };
                // Kernel threads have no `VmRSS`; they are not memory holders, and counting them as
                // "skipped" would make the number meaningless on a device with hundreds of them.
                let Some(rss_kb) = parse_vm_rss(&text) else {
                    continue;
                };
                out.push(Proc { pid, uid, rss_kb });
            }
            Err(_) => *skipped += 1,
        }
    }
    out
}

/// `Uid:\t1000\t1000\t1000\t1000` — the first field is the real UID.
fn parse_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// `VmRSS:\t  1234 kB`.
fn parse_vm_rss(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// `Pss:` and `SwapPss:` from `smaps_rollup`, both in kB.
pub(crate) fn parse_smaps_rollup(text: &str) -> Option<(u64, u64)> {
    let mut pss = None;
    let mut swap = 0u64;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Pss:") {
            pss = rest.split_whitespace().next()?.parse().ok();
        } else if let Some(rest) = line.strip_prefix("SwapPss:") {
            swap = rest
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
        }
    }
    pss.map(|pss| (pss, swap))
}

/// `com.example.app 10346 0 /data/user/0/...` → app id 10346 → package name.
fn parse_packages_list(text: &str) -> BTreeMap<u32, Vec<String>> {
    let mut out: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(package), Some(uid)) = (fields.next(), fields.next()) else {
            continue;
        };
        let Ok(uid) = uid.parse::<u32>() else {
            continue;
        };
        out.entry(uid).or_default().push(package.to_string());
    }
    out
}

/// App ids → packages: a ten-minute cache, then the database file, then the package service.
fn read_packages(fs: &Sysfs, data_dir: &Path) -> BTreeMap<u32, Vec<String>> {
    let path = data_dir.join("state").join("mem_packages.json");
    if let Some(cached) = cache::read_fresh(&path, PACKAGES_TTL_SECS) {
        if let Some(map) = cached.get("packages").and_then(parse_packages_json) {
            return map;
        }
    }

    let fresh = match fs.read_lossy(PACKAGES_LIST) {
        Ok(text) => {
            let map = parse_packages_list(&text);
            if map.is_empty() {
                read_packages_from_service()
            } else {
                map
            }
        }
        Err(_) => read_packages_from_service(),
    };

    if !fresh.is_empty() {
        let packages: Map<String, Value> = fresh
            .iter()
            .map(|(uid, names)| (uid.to_string(), json!(names)))
            .collect();
        cache::write(
            &path,
            &json!({
                "generated_at": cache::now_secs(),
                "packages": Value::Object(packages),
            }),
        );
    }
    fresh
}

/// The cached map is stored as `{"10346": ["com.example.app"]}` because JSON object keys are strings.
fn parse_packages_json(value: &Value) -> Option<BTreeMap<u32, Vec<String>>> {
    let object = value.as_object()?;
    let mut out = BTreeMap::new();
    for (uid, names) in object {
        let uid: u32 = uid.parse().ok()?;
        let names: Vec<String> = names
            .as_array()?
            .iter()
            .filter_map(|name| name.as_str().map(str::to_string))
            .collect();
        out.insert(uid, names);
    }
    Some(out)
}

/// Fallback for ROMs that hide the database: `cmd package list packages -U`, through the shared
/// helper so the call has a timeout (a stuck package service must not wedge the ranking).
fn read_packages_from_service() -> BTreeMap<u32, Vec<String>> {
    read_package_uids()
}

/// Android's short UID form, the inverse of `batterystats::resolve_uid`: ordinary app ids carry a
/// 10 000 base (`10346` → `u0a346`, `1010007` → `u10a7`), system uids do not (`1000` → `u0a1000`).
fn uid_label(uid: u32) -> String {
    let user = uid / 100_000;
    let app = uid % 100_000;
    match app.checked_sub(10_000) {
        Some(relative) => format!("u{user}a{relative}"),
        None => format!("u{user}a{app}"),
    }
}

/// A readable name. Work profiles and the secure folder carry a user prefix, so the package lookup
/// is done on the app id and the label says which user it belongs to.
fn label(uid: u32, packages: &[String]) -> String {
    let user = uid / 100_000;
    match packages.first() {
        Some(package) if user == 0 => package.clone(),
        Some(package) => format!("{package} (user {user})"),
        None => match aid_label(uid) {
            Some(name) => name.to_string(),
            None => format!("UID {uid}"),
        },
    }
}

fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("state").join("mem_apps.json")
}

/// Cached results are reused for a minute; `--refresh` bypasses them. A cached page computed for a
/// different `limit` is not reusable, because the PSS cut-off moves with it.
fn read_cache(path: &Path, limit: usize) -> Option<Value> {
    let mut value = cache::read_fresh(path, cache::TTL_SECS)?;
    if value.get("limit").and_then(Value::as_u64) != Some(limit as u64) {
        return None;
    }
    value["cached"] = json!(true);
    Some(value)
}

fn write_cache(path: &Path, value: &Map<String, Value>) {
    cache::write(path, &json!(value));
}
#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "\
Name:\tcom.example.app
State:\tS (sleeping)
Pid:\t4242
Uid:\t10346\t10346\t10346\t10346
Gid:\t10346\t10346\t10346\t10346
VmPeak:\t 4100000 kB
VmRSS:\t  412000 kB
RssAnon:\t  300000 kB
";

    const KERNEL_THREAD: &str = "\
Name:\tkworker/u16:1
State:\tI (idle)
Pid:\t42
Uid:\t0\t0\t0\t0
";

    #[test]
    fn status_gives_the_real_uid_and_rss() {
        assert_eq!(parse_uid(STATUS), Some(10_346));
        assert_eq!(parse_vm_rss(STATUS), Some(412_000));
        // A kernel thread has a UID but no RSS: skipped silently, not counted as unreadable.
        assert_eq!(parse_uid(KERNEL_THREAD), Some(0));
        assert_eq!(parse_vm_rss(KERNEL_THREAD), None);
        assert_eq!(parse_uid("garbage\n"), None);
    }

    #[test]
    fn smaps_rollup_reports_pss_and_swap_pss() {
        // The real file is label-first, in kB.
        let text = "\
Rss:               412000 kB
Pss:               380000 kB
Pss_Anon:          300000 kB
SwapPss:            96000 kB
";
        assert_eq!(parse_smaps_rollup(text), Some((380_000, 96_000)));
        // Without `Pss:` the file is useless for ranking.
        assert_eq!(parse_smaps_rollup("Rss: 4100 kB\n"), None);
        // A rollup with no swap is still valid.
        assert_eq!(parse_smaps_rollup("Pss: 1234 kB\n"), Some((1234, 0)));
    }

    #[test]
    fn packages_list_maps_app_ids_to_names() {
        let text = "\
com.example.app 10346 0 /data/user/0/com.example.app default 3002 0 1 default
com.example.other 10400 0 /data/user/0/com.example.other default none 0 1 default
";
        let map = parse_packages_list(text);
        assert_eq!(map[&10_346], vec!["com.example.app".to_string()]);
        assert_eq!(map[&10_400], vec!["com.example.other".to_string()]);
    }

    #[test]
    fn uid_labels_invert_the_batterystats_short_form() {
        assert_eq!(uid_label(10_346), "u0a346");
        assert_eq!(uid_label(1_010_007), "u10a7");
        // System uids have no 10 000 base.
        assert_eq!(uid_label(1000), "u0a1000");
    }

    #[test]
    fn work_profile_uids_are_found_by_app_id_and_say_which_user() {
        let packages = vec!["com.example.app".to_string()];
        assert_eq!(label(10_346, &packages), "com.example.app");
        // 9510346 is the same app in the work profile.
        assert_eq!(label(9_510_346, &packages), "com.example.app (user 95)");
        // No package: the AID table, then a bare UID. Never empty.
        assert_eq!(label(1000, &[]), "Android 系统");
        assert_eq!(label(99_999, &[]), "UID 99999");
    }

    // ---- the scan, on a fake /proc ---------------------------------------------------------

    struct Tree {
        base: std::path::PathBuf,
        root: std::path::PathBuf,
        data: std::path::PathBuf,
    }

    impl Tree {
        fn new(name: &str) -> Self {
            let base = std::env::temp_dir()
                .join(format!("sysflow-mem-apps-{}-{name}", std::process::id()));
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

        fn put(&self, rel: &str, content: &str) {
            let path = self.root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
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

    fn status(pid: u32, uid: u32, rss_kb: Option<u64>) -> String {
        let mut text = format!(
            "Name:\tproc{pid}\nState:\tS (sleeping)\nPid:\t{pid}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"
        );
        if let Some(rss) = rss_kb {
            text.push_str(&format!("VmRSS:\t  {rss} kB\n"));
        }
        text
    }

    fn rollup(pss_kb: u64, swap_kb: u64) -> String {
        format!("Rss: 1 kB\nPss: {pss_kb} kB\nPss_Anon: 1 kB\nSwapPss: {swap_kb} kB\n")
    }

    /// Three apps, one kernel thread, one vanished process and a non-numeric entry:
    /// pid 300 (uid 1000) holds 480 000 kB of PSS, pid 100 (uid 10346) 380 000, and pid 200
    /// (also 10346) is below the cut so its 100 000 kB of RSS is used as an estimate.
    fn fixture(name: &str) -> Tree {
        let tree = Tree::new(name);
        tree.put("proc/100/status", &status(100, 10_346, Some(412_000)));
        tree.put("proc/100/smaps_rollup", &rollup(380_000, 96_000));
        tree.put("proc/200/status", &status(200, 10_346, Some(100_000)));
        tree.put("proc/300/status", &status(300, 1000, Some(500_000)));
        tree.put("proc/300/smaps_rollup", &rollup(480_000, 0));
        tree.put("proc/42/status", &status(42, 0, None));
        tree.put("proc/self/status", &status(1, 0, Some(1)));
        // A process that vanished between listing and reading: status is a directory, so the read
        // fails. That is the one entry counted in `skipped`.
        std::fs::create_dir_all(tree.root.join("proc/999/status")).unwrap();
        tree.put(
            "data/system/packages.list",
            "com.example.app 10346 0 /data/user/0/com.example.app default 3002 0 1 default\n",
        );
        tree
    }

    fn entries(value: &Value) -> &Vec<Value> {
        value["entries"].as_array().unwrap()
    }

    #[test]
    fn the_scan_aggregates_by_uid_and_estimates_the_tail() {
        let tree = fixture("scan");
        let value = run_with(&tree.fs(), &tree.data, 2, false).unwrap();

        assert_eq!(value["available"], true);
        assert_eq!(value["cached"], false);
        assert_eq!(
            value["skipped"], 1,
            "the vanished pid is the only skipped one"
        );
        assert_eq!(value["total_pss_kb"], 960_000);
        assert_eq!(value["pss_for_top_n_only"], true);

        let rows = entries(&value);
        assert_eq!(rows.len(), 2);
        // Both hold 480 000 kB; the tie is broken by UID and the AID table names the system one.
        assert_eq!(rows[0]["numeric_uid"], 1000);
        assert_eq!(rows[0]["name"], "Android 系统");
        assert_eq!(rows[0]["pss_kb"], 480_000);

        // Two processes, one measured and one estimated from RSS.
        assert_eq!(rows[1]["numeric_uid"], 10_346);
        assert_eq!(rows[1]["name"], "com.example.app");
        assert_eq!(rows[1]["uid"], "u0a346");
        assert_eq!(rows[1]["procs"], 2);
        assert_eq!(rows[1]["pss_kb"], 480_000);
        assert_eq!(rows[1]["swap_pss_kb"], 96_000);
        assert_eq!(rows[1]["share"], 0.5);
        assert_eq!(rows[1]["packages"], serde_json::json!(["com.example.app"]));
    }

    #[test]
    fn the_result_is_cached_for_a_minute_and_refresh_bypasses_it() {
        let tree = fixture("cache");
        let first = run_with(&tree.fs(), &tree.data, 30, false).unwrap();
        assert_eq!(first["cached"], false);

        let cached = run_with(&tree.fs(), &tree.data, 30, false).unwrap();
        assert_eq!(cached["cached"], true);
        assert_eq!(cached["generated_at"], first["generated_at"]);

        // A different limit measured against a different cut-off, so the cache must not be reused.
        let other_limit = run_with(&tree.fs(), &tree.data, 1, false).unwrap();
        assert_eq!(other_limit["cached"], false);
        assert_eq!(other_limit["limit"], 1);

        let refreshed = run_with(&tree.fs(), &tree.data, 30, true).unwrap();
        assert_eq!(refreshed["cached"], false);
    }

    #[test]
    fn the_uid_map_is_cached_beyond_a_single_ranking() {
        let tree = fixture("uidcache");
        let first = run_with(&tree.fs(), &tree.data, 30, false).unwrap();
        assert!(entries(&first)
            .iter()
            .any(|entry| entry["name"] == "com.example.app"));

        // With the database gone and the result cache bypassed, the names must still resolve: they
        // come from the ten-minute UID cache. Without it, a ROM that hides the file would pay a
        // binder call on every cold ranking.
        std::fs::remove_file(tree.root.join("data/system/packages.list")).unwrap();
        let second = run_with(&tree.fs(), &tree.data, 30, true).unwrap();
        assert!(entries(&second)
            .iter()
            .any(|entry| entry["name"] == "com.example.app"));
    }

    #[test]
    fn an_unreadable_proc_yields_an_unavailable_result() {
        let tree = Tree::new("empty");
        std::fs::remove_dir_all(tree.root.join("proc")).unwrap();
        let value = run_with(&tree.fs(), &tree.data, 30, false).unwrap();
        assert_eq!(value["available"], false);
        assert!(entries(&value).is_empty());
    }
}
