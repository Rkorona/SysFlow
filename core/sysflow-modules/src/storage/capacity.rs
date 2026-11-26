//! Capacity and mount discovery.
//!
//! Two things live here: a tolerant `/proc/mounts` parser (testable, no side effects) and a direct
//! `statvfs(3)` query for the numbers. `statvfs` is declared through a `repr(C)` struct instead of
//! pulling in `libc`: the module already avoids external crates, and the 64-bit layout is identical
//! on bionic and glibc, which is all SysFlow ships (arm64 module, x86_64 host tests).

use std::ffi::{c_char, c_int, CString};

use sysflow_core::error::{Error, Result};
use sysflow_core::sysfs::Sysfs;

/// One line of `/proc/mounts`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub device: String,
    pub mount: String,
    pub fstype: String,
}

/// `statvfs` result, already converted to kB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    pub total_kb: u64,
    pub free_kb: u64,
    pub avail_kb: u64,
}

/// 64-bit `struct statvfs`. Field order is the Linux/bionic one; `f_spare` is only there to keep
/// the struct the same size as the C definition.
#[repr(C)]
struct Statvfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    f_spare: [u32; 6],
}

extern "C" {
    fn statvfs(path: *const c_char, buf: *mut Statvfs) -> c_int;
}

/// Query the filesystem containing `path`.
pub fn capacity(path: &str) -> Result<Capacity> {
    let c_path = CString::new(path)
        .map_err(|_| Error::invalid_arg(format!("path contains NUL: {path:?}")))?;
    let mut raw = Statvfs {
        f_bsize: 0,
        f_frsize: 0,
        f_blocks: 0,
        f_bfree: 0,
        f_bavail: 0,
        f_files: 0,
        f_ffree: 0,
        f_favail: 0,
        f_fsid: 0,
        f_flag: 0,
        f_namemax: 0,
        f_spare: [0; 6],
    };
    // SAFETY: `path` is a valid NUL-terminated string and `raw` is a properly sized, aligned
    // out-parameter matching the C `struct statvfs`.
    let rc = unsafe { statvfs(c_path.as_ptr(), &mut raw) };
    if rc != 0 {
        return Err(Error::io(format!(
            "statvfs {path}: {}",
            std::io::Error::last_os_error()
        )));
    }
    let block = if raw.f_frsize > 0 {
        raw.f_frsize
    } else {
        raw.f_bsize
    };
    if block == 0 {
        return Err(Error::io(format!("statvfs {path}: zero block size")));
    }
    Ok(Capacity {
        total_kb: raw.f_blocks.saturating_mul(block) / 1024,
        free_kb: raw.f_bfree.saturating_mul(block) / 1024,
        avail_kb: raw.f_bavail.saturating_mul(block) / 1024,
    })
}

/// Parse `/proc/mounts` into the entries SysFlow cares about.
///
/// The file escapes spaces and a few control characters as octal (`\040` and friends), so a mount
/// point like `/mnt/my\040disk` must be decoded before it can be compared with the paths we probe.
pub fn parse_mounts(text: &str) -> Vec<Mount> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(device), Some(mount), Some(fstype)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        out.push(Mount {
            device: unescape(device),
            mount: unescape(mount),
            fstype: fstype.to_string(),
        });
    }
    out
}

fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 3 < bytes.len() {
            let octal = &text[index + 1..index + 4];
            if let Ok(value) = u8::from_str_radix(octal, 8) {
                out.push(value as char);
                index += 4;
                continue;
            }
        }
        out.push(bytes[index] as char);
        index += 1;
    }
    out
}

/// Read and parse the real mount table through [`Sysfs`] (the fake root holds `proc/mounts`).
pub fn mounts(fs: &Sysfs) -> Result<Vec<Mount>> {
    let text = fs.read_lossy("/proc/mounts")?;
    Ok(parse_mounts(&text))
}

/// `data_path` (`/data` in production), plus `/storage/emulated/0` when it is a different device
/// (usually it is not: Android's emulated storage lives on the same filesystem, and reporting it
/// twice would double the numbers).
pub fn storage_mounts(fs: &Sysfs, data_path: &str) -> Result<(Mount, Option<Mount>)> {
    let mounts = mounts(fs)?;
    let data = mounts
        .iter()
        .find(|mount| mount.mount == data_path)
        .cloned()
        .ok_or_else(|| Error::unsupported("/proc/mounts has no /data"))?;
    let sdcard = mounts
        .iter()
        .find(|mount| mount.mount == "/storage/emulated/0" || mount.mount == "/sdcard")
        .filter(|mount| mount.device != data.device)
        .cloned();
    Ok((data, sdcard))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounts_decode_octal_escapes_and_keep_the_interesting_fields() {
        let text = "/dev/block/dm-5 /data ext4 rw,seclabel,noatime 0 0\n\
                    /dev/block/dm-5 /storage/emulated/0\\040foo ext4 rw 0 0\n\
                    tmpfs /mnt/scratch tmpfs rw 0 0\n\
                    broken line\n";
        let mounts = parse_mounts(text);
        assert_eq!(mounts.len(), 3);
        assert_eq!(mounts[0].device, "/dev/block/dm-5");
        assert_eq!(mounts[0].mount, "/data");
        assert_eq!(mounts[1].mount, "/storage/emulated/0 foo");
        assert_eq!(mounts[2].fstype, "tmpfs");
    }

    #[test]
    fn emulated_storage_on_the_same_device_is_not_reported_twice() {
        let root =
            std::env::temp_dir().join(format!("sysflow-storage-mounts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proc")).unwrap();
        std::fs::write(
            root.join("proc/mounts"),
            "/dev/block/dm-5 /data ext4 rw 0 0\n/dev/block/dm-5 /storage/emulated/0 ext4 rw 0 0\n",
        )
        .unwrap();
        let fs = Sysfs::new(&root);
        let (data, sdcard) = storage_mounts(&fs, "/data").unwrap();
        assert_eq!(data.mount, "/data");
        assert_eq!(
            sdcard, None,
            "same device must not be reported as a second storage"
        );
    }

    #[test]
    fn statvfs_reports_a_plausible_capacity_for_a_real_directory() {
        let dir = std::env::temp_dir();
        let capacity = capacity(dir.to_str().unwrap()).expect("statvfs works on /tmp");
        assert!(capacity.total_kb > 0);
        assert!(capacity.free_kb <= capacity.total_kb);
        assert!(capacity.avail_kb <= capacity.free_kb);
    }
}
