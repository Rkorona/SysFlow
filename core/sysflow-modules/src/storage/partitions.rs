//! Whole-device partition overview (`storage partitions`).
//!
//! A directory walk of `/` cannot answer "how much space does the whole phone use": `/` is the
//! read-only rootfs, and every real filesystem (`/data`, `/system`, `/vendor`, …) is a separate
//! mount. Walking across mounts would also double-count emulated storage and stumble into
//! `/proc`/`/sys`. This module lists the mounts themselves with `statvfs` numbers instead.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use sysflow_core::error::Result;
use sysflow_core::sysfs::Sysfs;

use super::capacity;

/// Filesystems that do not represent storage the user can reason about.
const PSEUDO_FSTYPES: &[&str] = &[
    "proc",
    "sysfs",
    "devtmpfs",
    "devpts",
    "tmpfs",
    "cgroup",
    "cgroup2",
    "bpf",
    "tracefs",
    "debugfs",
    "securityfs",
    "pstore",
    "configfs",
    "functionfs",
    "fusectl",
    "selinuxfs",
    "binder",
    "none",
    "ramfs",
];

pub(crate) fn is_pseudo(fstype: &str) -> bool {
    PSEUDO_FSTYPES.contains(&fstype)
}

/// Views of storage that already appear under another mount, and mount trees that would flood the
/// list: `/storage/emulated/0` is `/data/media`, `/mnt/*` are FUSE/runtime views, and every APEX
/// package is its own small mount under `/apex/`. Reporting them would double-count or bury the
/// real partitions.
pub(crate) fn is_view(mount: &str) -> bool {
    mount.starts_with("/storage/") || mount.starts_with("/mnt/") || mount.starts_with("/apex/")
}

/// Every real mount with its capacity, heaviest first.
pub fn partitions(fs: &Sysfs) -> Result<Value> {
    let mounts = capacity::mounts(fs)?;
    let mut entries: BTreeMap<String, Value> = BTreeMap::new();
    for mount in &mounts {
        if is_pseudo(&mount.fstype) || is_view(&mount.mount) {
            continue;
        }
        let Ok(capacity) = capacity::capacity(&mount.mount) else {
            continue;
        };
        if capacity.total_kb == 0 {
            continue;
        }
        let used_kb = capacity.total_kb.saturating_sub(capacity.free_kb);
        let used_pct = used_kb
            .saturating_mul(100)
            .checked_div(capacity.total_kb)
            .unwrap_or(0);
        entries.insert(
            mount.mount.clone(),
            json!({
                "mount": mount.mount,
                "device": mount.device,
                "fstype": mount.fstype,
                "total_kb": capacity.total_kb,
                "used_kb": used_kb,
                "free_kb": capacity.free_kb,
                "avail_kb": capacity.avail_kb,
                "used_pct": used_pct,
            }),
        );
    }

    let mut rows: Vec<Value> = entries.into_values().collect();
    rows.sort_by(|a, b| {
        let used = |value: &Value| value.get("used_kb").and_then(Value::as_u64).unwrap_or(0);
        used(b).cmp(&used(a)).then_with(|| {
            let mount = |value: &Value| {
                value
                    .get("mount")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            mount(a).cmp(&mount(b))
        })
    });

    if rows.is_empty() {
        return Ok(json!({
            "available": false,
            "reason": "/proc/mounts exposes no real filesystem",
            "partitions": [],
        }));
    }
    Ok(json!({
        "available": true,
        "partitions": rows,
    }))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sysflow-storage-partitions-{}-{name}",
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
    fn pseudo_and_duplicate_views_are_filtered_out() {
        let root = temp("filter");
        let data = root.join("data");
        let system = root.join("system");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        write(
            &root,
            "proc/mounts",
            &format!(
                "/dev/block/dm-5 {} f2fs rw 0 0\n\
                 /dev/block/sda1 {} ext4 ro 0 0\n\
                 tmpfs {} tmpfs rw 0 0\n\
                 proc /proc proc rw 0 0\n",
                data.display(),
                system.display(),
                root.join("tmp").display(),
            ),
        );

        let fs = Sysfs::new(&root);
        let value = partitions(&fs).unwrap();
        assert_eq!(value["available"], true);
        let rows = value["partitions"].as_array().unwrap();
        let mounts: Vec<&str> = rows
            .iter()
            .filter_map(|row| row["mount"].as_str())
            .collect();
        assert!(mounts.contains(&data.to_string_lossy().as_ref()));
        assert!(mounts.contains(&system.to_string_lossy().as_ref()));
        assert!(!mounts.iter().any(|mount| mount.contains("tmpfs")));
        assert!(!mounts.contains(&"/proc"), "pseudo filesystems are skipped");
        for row in rows {
            assert!(row["total_kb"].as_u64().unwrap() > 0);
            assert!(row["used_pct"].as_u64().unwrap() <= 100);
        }
    }
}
