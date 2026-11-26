//! Bounded directory-usage analysis (`storage usage`).
//!
//! `/data` can hold hundreds of thousands of files, so this scan is **time-bounded** by design: it
//! stops when the budget is exhausted and reports `partial:true` rather than freezing the caller.
//! It never follows symlinks, never crosses into another filesystem (so `/data/media` and bind
//! mounts are not counted twice), and counts allocated blocks (`st_blocks`) instead of apparent size,
//! because "how much space does this take" is the question the page answers.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::sysfs::Sysfs;

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;
pub const DEFAULT_BUDGET_MS: u64 = 3_000;
pub const MAX_BUDGET_MS: u64 = 30_000;
/// Hard cap on visited entries, so a runaway tree cannot turn the budget into unbounded memory.
const MAX_ENTRIES: u64 = 2_000_000;

#[derive(Clone, Debug)]
pub struct UsageOptions {
    pub path: String,
    pub limit: usize,
    pub budget_ms: u64,
}

impl Default for UsageOptions {
    fn default() -> Self {
        UsageOptions {
            path: "/data".to_string(),
            limit: DEFAULT_LIMIT,
            budget_ms: DEFAULT_BUDGET_MS,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Totals {
    bytes: u64,
    files: u64,
    dirs: u64,
}

#[derive(Debug)]
struct Entry {
    name: String,
    path: String,
    totals: Totals,
}

/// Scan the phone's root directory.
///
/// A plain directory walk of `/` is useless on Android: `/` is the read-only rootfs and almost every
/// real directory (`/data`, `/system`, `/cache`, …) is a separate mount. This mode therefore reports
/// **mounts as their filesystem used/total** (no recursion, so `/data` cannot hang the scan), walks
/// only entries that live on the rootfs itself, and labels symlinks / pseudo filesystems instead of
/// pretending they are directories.
pub fn scan_root(fs: &Sysfs, options: &UsageOptions) -> Result<Value> {
    use std::os::unix::fs::MetadataExt;

    let root = fs.resolve("/");
    let started = Instant::now();
    let budget = Duration::from_millis(options.budget_ms.clamp(1, MAX_BUDGET_MS));
    let root_meta = std::fs::symlink_metadata(&root)
        .map_err(|error| Error::invalid_arg(format!("/: {error}")))?;
    if !root_meta.is_dir() {
        return Err(Error::invalid_arg("/ is not a directory"));
    }
    let root_dev = root_meta.dev();
    let mounts = super::capacity::mounts(fs).unwrap_or_default();
    let mount_of = |path: &Path| {
        mounts
            .iter()
            .find(|mount| Path::new(&mount.mount) == path)
            .cloned()
    };

    let mut partial = false;
    let mut denied = 0u64;
    let mut visited = 0u64;
    let mut entries: Vec<Value> = Vec::new();

    let read_dir = std::fs::read_dir(&root).map_err(|error| Error::io(format!("/: {error}")))?;
    for entry in read_dir {
        if started.elapsed() >= budget {
            partial = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                denied += 1;
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let path_text = path.to_string_lossy().into_owned();
        let mut row = json!({
            "name": name,
            "path": path_text,
            "kb": 0,
            "files": 0,
            "dirs": 0,
            "kind": "dir",
        });

        if let Some(mount) = mount_of(&path) {
            row["fstype"] = json!(mount.fstype);
            row["device"] = json!(mount.device);
            if super::partitions::is_pseudo(&mount.fstype) {
                row["kind"] = json!("pseudo");
            } else if super::partitions::is_view(&mount.mount) {
                row["kind"] = json!("view");
            } else if let Ok(capacity) = super::capacity::capacity(&mount.mount) {
                let used = capacity.total_kb.saturating_sub(capacity.free_kb);
                row["kind"] = json!("mount");
                row["kb"] = json!(used);
                row["total_kb"] = json!(capacity.total_kb);
                row["used_pct"] = json!(used
                    .saturating_mul(100)
                    .checked_div(capacity.total_kb)
                    .unwrap_or(0));
            } else {
                row["kind"] = json!("mount");
            }
        } else {
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    denied += 1;
                    continue;
                }
            };
            if metadata.file_type().is_symlink() {
                row["kind"] = json!("symlink");
            } else if metadata.dev() != root_dev {
                row["kind"] = json!("mount");
            } else if metadata.is_file() {
                row["kind"] = json!("file");
                row["kb"] = json!(metadata.blocks().saturating_mul(512) / 1024);
                row["files"] = json!(1);
            } else if metadata.is_dir() {
                let (totals, hit_budget) =
                    walk(&path, root_dev, started, budget, &mut denied, &mut visited);
                partial |= hit_budget;
                row["kb"] = json!(totals.bytes / 1024);
                row["files"] = json!(totals.files);
                row["dirs"] = json!(totals.dirs);
            }
        }
        entries.push(row);
        if partial {
            break;
        }
    }

    entries.sort_by(|a, b| {
        let kb = |value: &Value| value.get("kb").and_then(Value::as_u64).unwrap_or(0);
        kb(b).cmp(&kb(a)).then_with(|| {
            let name = |value: &Value| {
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            name(a).cmp(&name(b))
        })
    });
    let total_kb: u64 = entries
        .iter()
        .filter(|row| row["kind"] == "dir" || row["kind"] == "file")
        .filter_map(|row| row["kb"].as_u64())
        .sum();
    let limit = options.limit.clamp(1, MAX_LIMIT);
    let returned: Vec<Value> = entries.into_iter().take(limit).collect();
    Ok(json!({
        "path": "/",
        "mode": "root",
        "partial": partial,
        "elapsed_ms": started.elapsed().as_millis() as u64,
        "denied": denied,
        "visited": visited,
        "limit": limit,
        "total_kb": total_kb,
        "entries": returned,
    }))
}

/// Scan the top-level entries of `options.path`, each with its recursive size.
pub fn scan(options: &UsageOptions) -> Result<Value> {
    let started = Instant::now();
    let budget = Duration::from_millis(options.budget_ms.clamp(1, MAX_BUDGET_MS));
    let root = Path::new(&options.path);
    let root_meta = std::fs::symlink_metadata(root)
        .map_err(|error| Error::invalid_arg(format!("{}: {error}", options.path)))?;
    if !root_meta.is_dir() {
        return Err(Error::invalid_arg(format!(
            "{} is not a directory",
            options.path
        )));
    }
    let root_dev = {
        use std::os::unix::fs::MetadataExt;
        root_meta.dev()
    };

    let mut partial = false;
    let mut denied = 0u64;
    let mut visited = 0u64;
    let mut entries: Vec<Entry> = Vec::new();

    let read_dir =
        std::fs::read_dir(root).map_err(|error| Error::io(format!("{}: {error}", options.path)))?;
    for entry in read_dir {
        if started.elapsed() >= budget {
            partial = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                denied += 1;
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let (totals, hit_budget) =
            walk(&path, root_dev, started, budget, &mut denied, &mut visited);
        partial |= hit_budget;
        entries.push(Entry {
            name,
            path: path.to_string_lossy().into_owned(),
            totals,
        });
        if partial {
            break;
        }
    }

    entries.sort_by(|a, b| {
        b.totals
            .bytes
            .cmp(&a.totals.bytes)
            .then_with(|| a.name.cmp(&b.name))
    });
    let total_bytes: u64 = entries.iter().map(|entry| entry.totals.bytes).sum();
    let limit = options.limit.clamp(1, MAX_LIMIT);
    let returned: Vec<Value> = entries
        .iter()
        .take(limit)
        .map(|entry| {
            json!({
                "name": entry.name,
                "path": entry.path,
                "kb": entry.totals.bytes / 1024,
                "files": entry.totals.files,
                "dirs": entry.totals.dirs,
            })
        })
        .collect();

    Ok(json!({
        "path": options.path,
        "partial": partial,
        "elapsed_ms": started.elapsed().as_millis() as u64,
        "denied": denied,
        "visited": visited,
        "limit": limit,
        "total_kb": total_bytes / 1024,
        "entries": returned,
    }))
}

/// Iterative depth-first walk. Returns the totals and whether the budget stopped it early.
fn walk(
    start: &Path,
    root_dev: u64,
    started: Instant,
    budget: Duration,
    denied: &mut u64,
    visited: &mut u64,
) -> (Totals, bool) {
    use std::os::unix::fs::MetadataExt;

    let mut totals = Totals::default();
    let mut stack: Vec<PathBuf> = vec![start.to_path_buf()];
    while let Some(path) = stack.pop() {
        if started.elapsed() >= budget || *visited >= MAX_ENTRIES {
            return (totals, true);
        }
        *visited += 1;
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                *denied += 1;
                continue;
            }
        };
        // Symlinks are skipped, never followed: /data/media is a view over the same blocks and
        // following it would double-count the user's storage.
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.dev() != root_dev {
            // Bind mount / other filesystem: not part of this directory's usage.
            continue;
        }
        if metadata.is_dir() {
            totals.dirs += 1;
            match std::fs::read_dir(&path) {
                Ok(children) => {
                    for child in children.flatten() {
                        stack.push(child.path());
                    }
                }
                Err(_) => *denied += 1,
            }
        } else if metadata.is_file() {
            totals.files += 1;
            totals.bytes = totals
                .bytes
                .saturating_add(metadata.blocks().saturating_mul(512));
        }
    }
    (totals, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "sysflow-storage-usage-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("big")).unwrap();
        std::fs::create_dir_all(root.join("small/nested")).unwrap();
        std::fs::write(root.join("big/blob.bin"), vec![7u8; 64 * 1024]).unwrap();
        std::fs::write(root.join("small/nested/file.txt"), b"hello").unwrap();
        root
    }

    #[test]
    fn scan_reports_sorted_entries_with_allocated_bytes() {
        let root = fixture("sorted");
        let out = scan(&UsageOptions {
            path: root.to_string_lossy().into_owned(),
            limit: 10,
            budget_ms: 10_000,
        })
        .unwrap();
        assert_eq!(out["partial"], false);
        let entries = out["entries"].as_array().unwrap();
        assert_eq!(entries[0]["name"], "big");
        assert!(entries[0]["kb"].as_u64().unwrap() >= 64);
        assert_eq!(entries[1]["name"], "small");
        assert!(out["total_kb"].as_u64().unwrap() >= 64);
        assert_eq!(out["denied"], 0);
    }

    #[test]
    fn a_spent_budget_stops_the_walk_immediately() {
        use std::os::unix::fs::MetadataExt;

        let root = fixture("budget");
        let metadata = std::fs::metadata(&root).unwrap();
        let mut denied = 0;
        let mut visited = 0;
        let (totals, partial) = walk(
            &root,
            metadata.dev(),
            Instant::now(),
            Duration::ZERO,
            &mut denied,
            &mut visited,
        );
        assert!(partial, "a zero budget must report a partial result");
        assert_eq!(totals.bytes, 0);
        assert_eq!(
            visited, 0,
            "nothing may be visited after the budget is gone"
        );
    }

    #[test]
    fn root_scan_reports_mounts_as_capacities_and_labels_pseudo_filesystems() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("sysflow-storage-root-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let data = root.join("data");
        let proc_dir = root.join("proc");
        let etc = root.join("etc");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&proc_dir).unwrap();
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(etc.join("hosts"), vec![0u8; 8192]).unwrap();
        symlink("/system/bin", root.join("bin")).unwrap();
        std::fs::write(
            root.join("proc/mounts"),
            format!(
                "/dev/block/dm-5 {} ext4 rw 0 0\nproc {} proc rw 0 0\n",
                data.display(),
                proc_dir.display()
            ),
        )
        .unwrap();

        let fs = Sysfs::new(&root);
        let out = scan_root(
            &fs,
            &UsageOptions {
                path: "/".to_string(),
                limit: 20,
                budget_ms: 10_000,
            },
        )
        .unwrap();
        assert_eq!(out["mode"], "root");
        let entries = out["entries"].as_array().unwrap();
        let row = |name: &str| {
            entries
                .iter()
                .find(|row| row["name"] == name)
                .unwrap_or_else(|| panic!("missing root entry {name}"))
        };
        assert_eq!(row("data")["kind"], "mount");
        assert!(row("data")["kb"].as_u64().unwrap() > 0);
        assert!(row("data")["total_kb"].as_u64().unwrap() > 0);
        assert_eq!(row("proc")["kind"], "pseudo");
        assert_eq!(row("etc")["kind"], "dir");
        assert!(row("etc")["kb"].as_u64().unwrap() >= 8);
        assert_eq!(row("bin")["kind"], "symlink");
    }

    #[test]
    fn a_file_is_not_a_directory() {
        let root = fixture("not-dir");
        let file = root.join("big/blob.bin");
        let error = scan(&UsageOptions {
            path: file.to_string_lossy().into_owned(),
            ..UsageOptions::default()
        })
        .unwrap_err();
        assert_eq!(error.code.as_str(), "invalid_arg");
    }

    #[test]
    fn limit_is_clamped_and_applied() {
        let root = fixture("limit");
        let out = scan(&UsageOptions {
            path: root.to_string_lossy().into_owned(),
            limit: 1,
            budget_ms: 10_000,
        })
        .unwrap();
        assert_eq!(out["entries"].as_array().unwrap().len(), 1);
        assert_eq!(out["limit"], 1);
    }
}
