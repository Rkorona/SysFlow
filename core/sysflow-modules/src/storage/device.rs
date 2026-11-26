//! Block-device discovery: sizes, schedulers, read-ahead and the physical device behind `/data`.
//!
//! `/data` is normally mounted from a device-mapper node (`/dev/block/dm-5`), and dm nodes do not
//! have an I/O scheduler. The tunables therefore apply to the **physical** device found through
//! `/sys/block/dm-5/slaves/`, not to the dm node the mount table names.

use sysflow_core::sysfs::Sysfs;

use super::capacity::Mount;

const SYS_BLOCK: &str = "/sys/block";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheduler {
    /// The bracketed value in `queue/scheduler`.
    pub current: String,
    /// Every value the kernel offers, in kernel order.
    pub options: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockDevice {
    pub name: String,
    pub size_kb: Option<u64>,
    pub model: Option<String>,
    pub rotational: bool,
    pub scheduler: Option<Scheduler>,
    pub read_ahead_kb: Option<u64>,
    pub nr_requests: Option<u64>,
    /// Remaining life estimate in percent, when the device exposes a JEDEC `life_time` counter.
    pub health_pct: Option<u64>,
}

impl BlockDevice {
    pub fn can_tune(&self) -> bool {
        self.scheduler.is_some() || self.read_ahead_kb.is_some()
    }

    /// Queue depth as the UI shows it; `None` when the kernel does not expose `nr_requests`.
    pub fn nr_requests_label(&self) -> Option<String> {
        self.nr_requests.map(|value| value.to_string())
    }
}

/// Parse `[mq-deadline] kyber none` into the current value and the options.
pub fn parse_scheduler(text: &str) -> Scheduler {
    let mut current = String::new();
    let mut options = Vec::new();
    for token in text.split_whitespace() {
        let (name, is_current) = match token.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            Some(name) => (name, true),
            None => (token, false),
        };
        if is_current {
            current = name.to_string();
        }
        if !name.is_empty() && !options.iter().any(|option| option == name) {
            options.push(name.to_string());
        }
    }
    // Some kernels print the current value without brackets; then it is simply the first token.
    if current.is_empty() {
        current = options.first().cloned().unwrap_or_default();
    }
    Scheduler { current, options }
}

fn read_u64(fs: &Sysfs, path: &str) -> Option<u64> {
    fs.read_str(path).ok().and_then(|text| text.parse().ok())
}

fn read_trimmed(fs: &Sysfs, path: &str) -> Option<String> {
    fs.read_str(path).ok().map(|text| text.trim().to_string())
}

/// One device by sysfs name (`sda`, `mmcblk0`, `dm-5`, …).
pub fn get(fs: &Sysfs, name: &str) -> Option<BlockDevice> {
    let base = format!("{SYS_BLOCK}/{name}");
    if !fs.exists(&base) {
        return None;
    }
    let scheduler =
        read_trimmed(fs, &format!("{base}/queue/scheduler")).map(|text| parse_scheduler(&text));
    let read_ahead_kb = read_u64(fs, &format!("{base}/queue/read_ahead_kb"));
    let nr_requests = read_u64(fs, &format!("{base}/queue/nr_requests"));
    // Size is in 512-byte sectors, independent of the logical block size.
    let size_kb = read_u64(fs, &format!("{base}/size")).map(|sectors| sectors / 2);
    let model = read_trimmed(fs, &format!("{base}/device/model")).filter(|model| !model.is_empty());
    let rotational = read_u64(fs, &format!("{base}/queue/rotational")).unwrap_or(0) == 1;
    let health_pct = read_health(fs, &base);
    Some(BlockDevice {
        name: name.to_string(),
        size_kb,
        model,
        rotational,
        scheduler,
        read_ahead_kb,
        nr_requests,
        health_pct,
    })
}

/// UFS/eMMC remaining-life estimate, when a JEDEC `life_time` counter is exposed.
///
/// The JEDEC encoding maps `0x01..0x0a` to 0–10 % … 90–100 % of rated write endurance consumed and
/// `0x0b` to "rated life exceeded". Two values (SLC/MLC) are printed on some devices; the worse one
/// wins. Vendors that expose a different format make this return `None` and the item is hidden —
/// guessing a percentage from an unknown format would be worse than showing nothing.
fn read_health(fs: &Sysfs, base: &str) -> Option<u64> {
    for name in ["life_time", "life_time_estimation"] {
        let path = format!("{base}/device/{name}");
        let Ok(text) = fs.read_str(&path) else {
            continue;
        };
        if let Some(health) = parse_life_time(&text) {
            return Some(health);
        }
    }
    None
}

/// Parse `0x01 0x02` (or decimal) into remaining life percent.
fn parse_life_time(text: &str) -> Option<u64> {
    let mut worst_used = 0u64;
    let mut seen = false;
    for token in text.split_whitespace() {
        let value = if let Some(hex) = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"))
        {
            u64::from_str_radix(hex, 16)
        } else {
            token.parse::<u64>()
        }
        .ok()?;
        // Values outside the JEDEC range mean this is not a life_time counter.
        if !(1..=11).contains(&value) {
            return None;
        }
        seen = true;
        // 0x0b means the rated endurance is exhausted.
        let used = if value >= 11 { 100 } else { value * 10 };
        worst_used = worst_used.max(used);
    }
    seen.then(|| 100u64.saturating_sub(worst_used))
}

/// Every whole block device except loop/ram/zram pseudo devices, sorted by name.
pub fn list(fs: &Sysfs) -> Vec<BlockDevice> {
    let Ok(entries) = fs.list_dir(SYS_BLOCK) else {
        return Vec::new();
    };
    let mut devices: Vec<BlockDevice> = entries
        .into_iter()
        .filter(|name| {
            !(name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram"))
        })
        .filter_map(|name| get(fs, &name))
        .collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    devices
}

/// `/dev/block/dm-5` → `dm-5`; `/dev/block/sda1` → `sda1`; `/dev/block/mapper/foo` is not a sysfs
/// block name, so it is rejected (the caller falls back to a scan).
pub fn device_name(device: &str) -> Option<String> {
    let name = device
        .strip_prefix("/dev/block/")
        .or_else(|| device.strip_prefix("/dev/"))?;
    if name.starts_with("mapper/") || name.starts_with("by-") {
        return None;
    }
    let name = name.rsplit('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Follow `/sys/block/<name>/slaves/*` down to the physical device (`dm-5` → `sda`).
pub fn resolve_physical(fs: &Sysfs, name: &str) -> String {
    let mut current = name.to_string();
    for _ in 0..4 {
        let slaves = format!("{SYS_BLOCK}/{current}/slaves");
        let Ok(entries) = fs.list_dir(&slaves) else {
            break;
        };
        let mut entries: Vec<String> = entries
            .into_iter()
            .filter(|entry| fs.exists(&format!("{SYS_BLOCK}/{entry}")))
            .collect();
        entries.sort();
        let Some(next) = entries.into_iter().next() else {
            break;
        };
        current = next;
    }
    current
}

/// The physical device backing a mount, preferring one that actually exposes queue knobs.
pub fn for_mount(fs: &Sysfs, mount: &Mount) -> Option<BlockDevice> {
    let name = device_name(&mount.device)?;
    let physical = resolve_physical(fs, &name);
    if let Some(device) = get(fs, &physical).filter(BlockDevice::can_tune) {
        return Some(device);
    }
    // No slaves (or the physical node lacks a queue): the mounted node itself is the best answer.
    get(fs, &name).or(Some(BlockDevice {
        name: physical,
        size_kb: None,
        model: None,
        rotational: false,
        scheduler: None,
        read_ahead_kb: None,
        nr_requests: None,
        health_pct: None,
    }))
}

pub fn scheduler_path(device: &str) -> String {
    format!("{SYS_BLOCK}/{device}/queue/scheduler")
}

pub fn read_ahead_path(device: &str) -> String {
    format!("{SYS_BLOCK}/{device}/queue/read_ahead_kb")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sysflow-storage-device-{}-{name}",
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

    fn fake_disk(root: &Path, name: &str, scheduler: &str, read_ahead: u64, size: u64) {
        write(
            root,
            &format!("sys/block/{name}/queue/scheduler"),
            scheduler,
        );
        write(
            root,
            &format!("sys/block/{name}/queue/read_ahead_kb"),
            &format!("{read_ahead}\n"),
        );
        write(root, &format!("sys/block/{name}/queue/nr_requests"), "64\n");
        write(root, &format!("sys/block/{name}/queue/rotational"), "0\n");
        write(
            root,
            &format!("sys/block/{name}/size"),
            &format!("{size}\n"),
        );
        write(
            root,
            &format!("sys/block/{name}/device/model"),
            "Samsung UFS\n",
        );
        write(
            root,
            &format!("sys/block/{name}/device/life_time"),
            "0x01 0x02\n",
        );
    }

    #[test]
    fn scheduler_parser_reads_the_bracket_and_all_options() {
        let scheduler = parse_scheduler("[mq-deadline] kyber bfq none\n");
        assert_eq!(scheduler.current, "mq-deadline");
        assert_eq!(
            scheduler.options,
            vec!["mq-deadline", "kyber", "bfq", "none"]
        );
        // Unbracketed output falls back to the first token rather than reporting an empty value.
        let scheduler = parse_scheduler("none mq-deadline");
        assert_eq!(scheduler.current, "none");
    }

    #[test]
    fn dm_node_resolves_to_the_physical_device_from_slaves() {
        let root = temp("slaves");
        fake_disk(&root, "sda", "[mq-deadline] none\n", 512, 2_000_000);
        write(&root, "sys/block/dm-5/slaves/sda", "");
        write(&root, "sys/block/loop0/queue/scheduler", "[none] none\n");
        write(&root, "sys/block/zram0/queue/scheduler", "[none] none\n");
        let fs = Sysfs::new(&root);

        assert_eq!(resolve_physical(&fs, "dm-5"), "sda");
        let names: Vec<String> = list(&fs).into_iter().map(|device| device.name).collect();
        assert_eq!(
            names,
            vec!["dm-5", "sda"],
            "loop/zram devices are filtered out"
        );

        let mount = Mount {
            device: "/dev/block/dm-5".to_string(),
            mount: "/data".to_string(),
            fstype: "ext4".to_string(),
        };
        let device = for_mount(&fs, &mount).unwrap();
        assert_eq!(device.name, "sda");
        assert_eq!(device.size_kb, Some(1_000_000));
        assert_eq!(device.scheduler.as_ref().unwrap().current, "mq-deadline");
        assert_eq!(device.read_ahead_kb, Some(512));
        assert_eq!(
            device.health_pct,
            Some(80),
            "worse of 0x01/0x02 = 20 % used"
        );
    }

    #[test]
    fn life_time_parser_rejects_unknown_formats_and_saturates_at_zero() {
        assert_eq!(parse_life_time("0x01"), Some(90));
        assert_eq!(parse_life_time("0x0a 0x03"), Some(0));
        assert_eq!(parse_life_time("0x0b"), Some(0));
        assert_eq!(parse_life_time("ok"), None);
        assert_eq!(
            parse_life_time("0x00"),
            None,
            "0 is not a JEDEC life-time value"
        );
        assert_eq!(parse_life_time(""), None);
    }

    #[test]
    fn device_name_accepts_block_paths_and_rejects_mapper_aliases() {
        assert_eq!(device_name("/dev/block/sda1").as_deref(), Some("sda1"));
        assert_eq!(device_name("/dev/block/dm-5").as_deref(), Some("dm-5"));
        assert_eq!(device_name("/dev/block/mapper/system"), None);
        assert_eq!(device_name("/dev/block/by-name/userdata"), None);
        assert_eq!(device_name("sda"), None);
    }
}
