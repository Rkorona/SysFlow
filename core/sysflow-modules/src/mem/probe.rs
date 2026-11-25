//! Raw readers, parsers and the device probe for the memory module.
//!
//! Everything here is pure or reads through [`Sysfs`], so it runs against a fake tree on the host.
//! The same [`Detected`] value drives `probe()`, `schema()` and the hidden `mem probe` report, so
//! the three can never disagree about what this device offers.
//!
//! Rules that hold for every reader (see `docs/modules/mem.md`):
//! * a missing source removes its items; it never becomes `0` or `null`;
//! * kernel counters are looked up **by name**, never by position;
//! * `mm_stat` is read positionally but tolerantly: only the first three columns are used.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, ErrorCode};
use sysflow_core::schema::{ItemType, SchemaItem};
use sysflow_core::sysfs::Sysfs;

pub const MEMINFO: &str = "/proc/meminfo";
pub const PSI_MEMORY: &str = "/proc/pressure/memory";
pub const SWAPS: &str = "/proc/swaps";
pub const VMSTAT: &str = "/proc/vmstat";
pub const OSRELEASE: &str = "/proc/sys/kernel/osrelease";
pub const LRU_GEN: &str = "/sys/kernel/mm/lru_gen/enabled";
pub const SYS_BLOCK: &str = "/sys/block";
pub const PACKAGES_LIST: &str = "/data/system/packages.list";

/// `/proc/sys/vm/*` knobs the tunables (M3b) will touch; read-only here, for the probe report.
pub const SYSCTLS: &[&str] = &[
    "swappiness",
    "vfs_cache_pressure",
    "dirty_ratio",
    "dirty_background_ratio",
    "dirty_bytes",
    "dirty_background_bytes",
    "min_free_kbytes",
    "watermark_scale_factor",
    "page-cluster",
];

/// Places where a memory breakdown or a reclaim policy might live. Reported as present / absent
/// (plus the first line, or the first entries of a directory) so the probe can answer "is there a
/// dmabuf / GPU number to show" and "is MGLRU on" without guessing.
const EXTRA_PATHS: &[&str] = &[
    "/sys/kernel/dmabuf/buffers",
    "/sys/kernel/dma_heap",
    "/sys/kernel/debug/dma_buf/bufinfo",
    "/sys/class/kgsl/kgsl/page_alloc",
    "/sys/kernel/mm/lru_gen/enabled",
    "/sys/kernel/mm/lru_gen/min_ttl_ms",
    "/sys/block/zram0/backing_dev",
    "/sys/block/zram0/writeback_limit_enable",
];

/// vmstat counters the history series will consume, reported verbatim by the probe.
const VMSTAT_OF_INTEREST: &[&str] = &[
    "pgscan_direct",
    "pgscan_kswapd",
    "oom_kill",
    "compact_stall",
    "pswpin",
    "pswpout",
];

/// vmstat counter families that kernels split or rename; the probe lists every name it finds.
const VMSTAT_FAMILIES: &[&str] = &["allocstall", "workingset_refault", "pgscan_direct"];

const MEMINFO_KEYS: &[&str] = &[
    "MemTotal",
    "MemAvailable",
    "MemFree",
    "Cached",
    "AnonPages",
    "Shmem",
    "Slab",
    "Dirty",
    "Writeback",
    "SwapTotal",
    "SwapFree",
];

// ---------------------------------------------------------------------------------------------
// Read outcome
// ---------------------------------------------------------------------------------------------

/// Why a source gave nothing. `Missing` and `Unreadable` are different findings: a PSI file that
/// exists but errors on read means the kernel has PSI compiled in but switched off.
#[derive(Clone, Debug, PartialEq)]
pub enum Read<T> {
    Ok(T),
    Missing,
    Unreadable(String),
    Malformed(String),
}

impl<T> Read<T> {
    pub fn ok(&self) -> Option<&T> {
        match self {
            Read::Ok(value) => Some(value),
            _ => None,
        }
    }

    pub fn state(&self) -> &'static str {
        match self {
            Read::Ok(_) => "ok",
            Read::Missing => "missing",
            Read::Unreadable(_) => "unreadable",
            Read::Malformed(_) => "malformed",
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Read::Unreadable(msg) | Read::Malformed(msg) => Some(msg),
            _ => None,
        }
    }
}

fn classify(err: Error) -> Read<String> {
    match err.code {
        ErrorCode::Unsupported => Read::Missing,
        _ => Read::Unreadable(err.msg),
    }
}

fn read_text(fs: &Sysfs, path: &str) -> Read<String> {
    match fs.read_lossy(path) {
        Ok(text) => Read::Ok(text),
        Err(err) => classify(err),
    }
}

fn parse_with<T>(text: Read<String>, parse: impl FnOnce(&str) -> Option<T>, what: &str) -> Read<T> {
    match text {
        Read::Ok(text) => match parse(&text) {
            Some(value) => Read::Ok(value),
            None => Read::Malformed(format!("{what} has an unexpected format")),
        },
        Read::Missing => Read::Missing,
        Read::Unreadable(msg) => Read::Unreadable(msg),
        Read::Malformed(msg) => Read::Malformed(msg),
    }
}

// ---------------------------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------------------------

/// `Key:   1234 kB` lines into a name → number map. Values keep the file's own unit (kB for sizes).
pub fn parse_meminfo(text: &str) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(number) = rest.split_whitespace().next() else {
            continue;
        };
        if let Ok(value) = number.parse::<u64>() {
            out.insert(key.trim().to_string(), value);
        }
    }
    out
}

/// One PSI line: `some avg10=0.00 avg60=0.12 avg300=0.05 total=12345`.
#[derive(Clone, Debug, PartialEq)]
pub struct PsiLine {
    pub avg10: f64,
    pub avg60: f64,
    pub avg300: f64,
    /// Cumulative stall time in microseconds; only meaningful as a difference.
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Psi {
    pub some: PsiLine,
    /// Absent on kernels that only report `some` for a resource.
    pub full: Option<PsiLine>,
}

fn parse_psi_line(line: &str) -> Option<PsiLine> {
    let mut avg10 = None;
    let mut avg60 = None;
    let mut avg300 = None;
    let mut total = None;
    for token in line.split_whitespace().skip(1) {
        let (key, value) = token.split_once('=')?;
        match key {
            "avg10" => avg10 = value.parse().ok(),
            "avg60" => avg60 = value.parse().ok(),
            "avg300" => avg300 = value.parse().ok(),
            "total" => total = value.parse().ok(),
            _ => {}
        }
    }
    Some(PsiLine {
        avg10: avg10?,
        avg60: avg60?,
        avg300: avg300?,
        total: total?,
    })
}

pub fn parse_psi(text: &str) -> Option<Psi> {
    let mut some = None;
    let mut full = None;
    for line in text.lines() {
        if line.starts_with("some ") {
            some = parse_psi_line(line);
        } else if line.starts_with("full ") {
            full = parse_psi_line(line);
        }
    }
    Some(Psi { some: some?, full })
}

/// PSI average in percent → protocol unit, hundredths of a percent.
pub fn pct_to_hundredths(percent: f64) -> i64 {
    (percent * 100.0).round() as i64
}

/// The leading columns of `/sys/block/zram*/mm_stat`, in **bytes**.
#[derive(Clone, Debug, PartialEq)]
pub struct MmStat {
    pub orig: u64,
    pub compr: u64,
    pub mem_used: u64,
    /// How many columns the kernel printed (7 on old kernels, 9 on newer); diagnostic only.
    pub columns: usize,
}

pub fn parse_mm_stat(text: &str) -> Option<MmStat> {
    let numbers: Vec<u64> = text
        .split_whitespace()
        .map(|token| token.parse::<u64>())
        .collect::<Result<_, _>>()
        .ok()?;
    if numbers.len() < 3 {
        return None;
    }
    Some(MmStat {
        orig: numbers[0],
        compr: numbers[1],
        mem_used: numbers[2],
        columns: numbers.len(),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwapEntry {
    pub name: String,
    pub is_zram: bool,
    pub size_kb: u64,
    pub used_kb: u64,
    pub priority: i64,
}

/// `/proc/swaps`: `Filename Type Size Used Priority`, sizes in kB. The header line is skipped.
pub fn parse_swaps(text: &str) -> Vec<SwapEntry> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        let (Ok(size_kb), Ok(used_kb), Ok(priority)) = (
            fields[2].parse::<u64>(),
            fields[3].parse::<u64>(),
            fields[4].parse::<i64>(),
        ) else {
            continue;
        };
        let name = fields[0].to_string();
        let leaf = name.rsplit('/').next().unwrap_or(&name);
        out.push(SwapEntry {
            is_zram: leaf.starts_with("zram"),
            name,
            size_kb,
            used_kb,
            priority,
        });
    }
    out
}

/// `/sys/kernel/mm/lru_gen/enabled` prints a bitmask (`0x0003`, or plain `1` on older backports);
/// any non-zero value means MGLRU is on.
pub fn parse_lru_gen(text: &str) -> Option<bool> {
    let text = text.trim();
    let digits = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"));
    let value = match digits {
        Some(hex) => u64::from_str_radix(hex, 16).ok()?,
        None => text.parse::<u64>().ok()?,
    };
    Some(value != 0)
}

pub fn parse_vmstat(text: &str) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(name), Some(value)) = (parts.next(), parts.next()) {
            if let Ok(value) = value.parse::<u64>() {
                out.insert(name.to_string(), value);
            }
        }
    }
    out
}

/// `6.6.30-android15-8-g123` → `(6, 6)`.
pub fn kernel_version(release: &str) -> Option<(u32, u32)> {
    fn leading_number(part: &str) -> Option<u32> {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }
    let mut parts = release.trim().split('.');
    let major = leading_number(parts.next()?)?;
    let minor = leading_number(parts.next()?)?;
    Some((major, minor))
}

/// Highest value the kernel accepts for `vm.swappiness`: 200 from 5.8, 100 before. An unknown or
/// unparsable release gets the conservative 100.
pub fn swappiness_max(release: Option<&str>) -> i64 {
    match release.and_then(kernel_version) {
        Some(version) if version >= (5, 8) => 200,
        _ => 100,
    }
}

// ---------------------------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ZramDevice {
    pub name: String,
    pub disksize_bytes: Option<u64>,
    pub mm_stat: Read<MmStat>,
    pub comp_algorithm: Option<String>,
    /// Present when the device can write pages back to flash (RAM Plus on Samsung).
    pub backing_dev: Option<String>,
}

/// What this device offers, gathered once and shared by `probe`, `schema` and the probe report.
#[derive(Clone, Debug)]
pub struct Detected {
    pub meminfo: Read<BTreeMap<String, u64>>,
    pub psi: Read<Psi>,
    pub swaps: Read<Vec<SwapEntry>>,
    pub zram: Vec<ZramDevice>,
    pub vmstat: Read<BTreeMap<String, u64>>,
    pub osrelease: Option<String>,
    /// Multi-generational LRU, when the kernel exposes the switch. It changes what `swappiness`
    /// means, so the UI says so instead of promising results.
    pub mglru: Option<bool>,
}

pub fn detect(fs: &Sysfs) -> Detected {
    let meminfo = parse_with(
        read_text(fs, MEMINFO),
        |text| Some(parse_meminfo(text)).filter(|map| !map.is_empty()),
        MEMINFO,
    );
    let psi = parse_with(read_text(fs, PSI_MEMORY), parse_psi, PSI_MEMORY);
    let swaps = match read_text(fs, SWAPS) {
        Read::Ok(text) => Read::Ok(parse_swaps(&text)),
        Read::Missing => Read::Missing,
        Read::Unreadable(msg) => Read::Unreadable(msg),
        Read::Malformed(msg) => Read::Malformed(msg),
    };
    let vmstat = parse_with(
        read_text(fs, VMSTAT),
        |text| Some(parse_vmstat(text)).filter(|map| !map.is_empty()),
        VMSTAT,
    );
    let osrelease = fs.read_str(OSRELEASE).ok().filter(|s| !s.is_empty());
    let mglru = fs
        .read_str(LRU_GEN)
        .ok()
        .and_then(|text| parse_lru_gen(&text));

    let mut zram = Vec::new();
    if let Ok(names) = fs.list_dir(SYS_BLOCK) {
        for name in names {
            let is_zram = name
                .strip_prefix("zram")
                .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()));
            if !is_zram {
                continue;
            }
            let base = format!("{SYS_BLOCK}/{name}");
            zram.push(ZramDevice {
                disksize_bytes: fs.read_u64(&format!("{base}/disksize")).ok(),
                mm_stat: parse_with(
                    read_text(fs, &format!("{base}/mm_stat")),
                    parse_mm_stat,
                    "mm_stat",
                ),
                comp_algorithm: fs
                    .read_str(&format!("{base}/comp_algorithm"))
                    .ok()
                    .filter(|s| !s.is_empty()),
                backing_dev: fs
                    .read_str(&format!("{base}/backing_dev"))
                    .ok()
                    .filter(|value| !value.is_empty()),
                name,
            });
        }
    }

    Detected {
        meminfo,
        psi,
        swaps,
        zram,
        vmstat,
        osrelease,
        mglru,
    }
}

impl Detected {
    fn meminfo_value(&self, key: &str) -> Option<u64> {
        self.meminfo.ok()?.get(key).copied()
    }

    /// The module needs `/proc/meminfo` with at least `MemTotal`; nothing else is mandatory.
    pub fn supported(&self) -> bool {
        self.meminfo_value("MemTotal").is_some()
    }

    pub fn unsupported_reason(&self) -> String {
        match &self.meminfo {
            Read::Ok(_) => "/proc/meminfo has no MemTotal".to_string(),
            Read::Missing => "/proc/meminfo is missing".to_string(),
            Read::Unreadable(msg) | Read::Malformed(msg) => {
                format!("/proc/meminfo cannot be used: {msg}")
            }
        }
    }

    /// zRAM devices that report a size. Items need at least one.
    fn zram_sized(&self) -> bool {
        !self.zram.is_empty() && self.zram.iter().all(|d| d.disksize_bytes.is_some())
    }

    /// Usage columns are only honest when **every** device reports them: a sum over a subset would
    /// understate the total and skew the ratio.
    fn zram_usage(&self) -> bool {
        self.zram_sized() && self.zram.iter().all(|d| d.mm_stat.ok().is_some())
    }

    /// Whether any zram device can write pages back to a flash device. When it does, `orig` stops
    /// being a synonym for "swap used", which is why the flag is published next to the numbers.
    pub fn zram_writeback(&self) -> bool {
        self.zram.iter().any(|device| device.backing_dev.is_some())
    }

    /// Summed `mm_stat` totals in **bytes**, or `None` when any device is missing them.
    ///
    /// Summing bytes rather than kB keeps `ratio` exact; the kB items are divided once at the end.
    pub fn zram_totals(&self) -> Option<(u64, u64, u64)> {
        if !self.zram_usage() {
            return None;
        }
        let mut orig = 0u64;
        let mut compr = 0u64;
        let mut mem_used = 0u64;
        for device in &self.zram {
            let stat = device.mm_stat.ok()?;
            orig += stat.orig;
            compr += stat.compr;
            mem_used += stat.mem_used;
        }
        Some((orig, compr, mem_used))
    }

    pub fn features(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.supported() {
            out.push("meminfo");
        }
        if self.meminfo_value("SwapTotal").is_some() && self.meminfo_value("SwapFree").is_some() {
            out.push("swap");
        }
        if self.swaps.ok().is_some() {
            out.push("swaps");
        }
        if self.psi.ok().is_some() {
            out.push("psi");
            if self.psi.ok().is_some_and(|psi| psi.full.is_some()) {
                out.push("psi_full");
            }
        }
        if self.zram_sized() {
            out.push("zram");
        }
        if self.zram_usage() {
            out.push("zram_mm_stat");
        }
        if self.vmstat.ok().is_some() {
            out.push("vmstat");
        }
        out
    }

    /// The `schema` item list for what this device actually has. Missing sources drop their items.
    pub fn items(&self) -> Vec<SchemaItem> {
        let ro = |id: &str| SchemaItem::new(id, ItemType::Readonly);
        let mut items = Vec::new();
        if !self.supported() {
            return items;
        }
        for (id, key) in [
            ("mem.total", "MemTotal"),
            ("mem.available", "MemAvailable"),
            ("mem.free", "MemFree"),
            ("mem.cached", "Cached"),
            ("mem.anon", "AnonPages"),
            ("mem.shmem", "Shmem"),
            ("mem.slab", "Slab"),
        ] {
            if self.meminfo_value(key).is_some() {
                items.push(ro(id).unit("kB"));
            }
        }
        if self.meminfo_value("Dirty").is_some() && self.meminfo_value("Writeback").is_some() {
            items.push(ro("mem.dirty").unit("kB"));
        }
        if self.meminfo_value("SwapTotal").is_some() && self.meminfo_value("SwapFree").is_some() {
            items.push(ro("mem.swap_total").unit("kB"));
            items.push(ro("mem.swap_free").unit("kB"));
        }
        if self.swaps.ok().is_some() {
            items.push(ro("mem.swap.zram_kb").unit("kB"));
            items.push(ro("mem.swap.other_kb").unit("kB"));
        }
        if let Some(psi) = self.psi.ok() {
            let mut names = vec!["some_avg10", "some_avg60", "some_avg300"];
            if psi.full.is_some() {
                names.extend(["full_avg10", "full_avg60", "full_avg300"]);
            }
            for name in names {
                items.push(ro(&format!("mem.psi.{name}")).unit("0.01%"));
            }
        }
        if self.zram_sized() {
            items.push(ro("mem.zram.count"));
            items.push(ro("mem.zram.disksize").unit("kB"));
            // 1 when a device has a writeback backing store, 0 otherwise. Published because it
            // changes how `orig` / `ratio` should be read, not because it is a tunable.
            items.push(ro("mem.zram.writeback"));
        }
        if let Some((_, compr, _)) = self.zram_totals() {
            items.push(ro("mem.zram.orig").unit("kB"));
            items.push(ro("mem.zram.compr").unit("kB"));
            items.push(ro("mem.zram.mem_used").unit("kB"));
            // A ratio needs a denominator, and a device that compressed nothing has none. `readings`
            // applies the same rule, so `schema` and `collect` stay in step.
            if compr > 0 {
                items.push(ro("mem.zram.ratio").unit("%"));
            }
        }
        // Memory composition. Every one of these is optional by design: a kernel that does not report
        // the field simply does not get the item. They exist so the "used" bar can be explained
        // instead of leaving a quarter of RAM as "other" (`docs/modules/mem.md`).
        if self.mglru.is_some() {
            items.push(ro("mem.mglru"));
        }
        for (id, key) in [
            ("mem.kernel_stack", "KernelStack"),
            ("mem.page_tables", "PageTables"),
            ("mem.percpu", "Percpu"),
            ("mem.gpu", "GpuTotal"),
            ("mem.vendor.rbin", "RbinTotal"),
            ("mem.vendor.cma", "CmaTotal"),
            ("mem.vendor.dmaheap", "DmaHeapPool"),
            ("mem.vmalloc_used", "VmallocUsed"),
        ] {
            if self.meminfo_value(key).is_some() {
                items.push(ro(id).unit("kB"));
            }
        }
        items
    }

    /// Items the design lists but this device cannot provide, with the reason. This is what makes a
    /// missing item explainable ("kernel has no PSI") instead of silently absent.
    pub fn hidden(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if !self.supported() {
            out.push(("mem.*", self.unsupported_reason()));
            return out;
        }
        for (id, key) in [("mem.available", "MemAvailable"), ("mem.dirty", "Dirty")] {
            if self.meminfo_value(key).is_none() {
                out.push((id, format!("/proc/meminfo has no {key}")));
            }
        }
        match &self.psi {
            Read::Ok(psi) if psi.full.is_none() => {
                out.push(("mem.psi.full_*", "PSI reports no `full` line".to_string()))
            }
            Read::Ok(_) => {}
            Read::Missing => out.push(("mem.psi.*", format!("{PSI_MEMORY} is missing"))),
            Read::Unreadable(msg) => out.push((
                "mem.psi.*",
                format!("{PSI_MEMORY} exists but cannot be read (PSI switched off?): {msg}"),
            )),
            Read::Malformed(msg) => out.push(("mem.psi.*", msg.clone())),
        }
        if self.zram.is_empty() {
            out.push(("mem.zram.*", "no /sys/block/zram* device".to_string()));
        } else if !self.zram_sized() {
            out.push((
                "mem.zram.*",
                "a zram device has no readable disksize".to_string(),
            ));
        } else if !self.zram_usage() {
            out.push((
                "mem.zram.{orig,compr,mem_used,ratio}",
                "mm_stat is missing or unusable on at least one zram device".to_string(),
            ));
        }
        if self.swaps.ok().is_none() {
            out.push((
                "mem.swap.{zram_kb,other_kb}",
                format!("{SWAPS}: {}", self.swaps.state()),
            ));
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Probe report (filesystem part)
// ---------------------------------------------------------------------------------------------

fn source(path: &str, read: &Read<impl Sized>) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("path".into(), json!(path));
    o.insert("state".into(), json!(read.state()));
    if let Some(err) = read.error() {
        o.insert("error".into(), json!(err));
    }
    o
}

fn psi_line_json(line: &PsiLine) -> Value {
    json!({
        "avg10": line.avg10, "avg60": line.avg60, "avg300": line.avg300, "total_us": line.total,
    })
}

fn path_report(fs: &Sysfs) -> Value {
    let mut out = Map::new();
    for path in EXTRA_PATHS {
        let entry = if !fs.exists(path) {
            json!({ "state": "missing" })
        } else if let Ok(names) = fs.list_dir(path) {
            json!({
                "state": "dir",
                "entries": names.len(),
                "first": names.iter().take(10).collect::<Vec<_>>(),
            })
        } else {
            match fs.read_lossy(path) {
                Ok(text) => json!({
                    "state": "ok",
                    "value": text.lines().next().unwrap_or("").chars().take(120).collect::<String>(),
                }),
                Err(err) => json!({ "state": "unreadable", "error": err.code.as_str() }),
            }
        };
        out.insert((*path).to_string(), entry);
    }
    Value::Object(out)
}

/// The raw readings and the module's judgement of them. The CLI adds the command-based sources
/// (`getprop`, `dmesg`, `logcat`) on top; those are not testable on a host.
pub fn report(fs: &Sysfs) -> Value {
    let detected = detect(fs);
    let mut root = Map::new();

    let release = detected.osrelease.as_deref();
    root.insert(
        "kernel".into(),
        json!({
            "release": release,
            "version": release.and_then(kernel_version).map(|(a, b)| json!([a, b])),
            "swappiness_max": swappiness_max(release),
        }),
    );

    let mut meminfo = source(MEMINFO, &detected.meminfo);
    if let Some(map) = detected.meminfo.ok() {
        // Every key, not just the ones the module uses: the unexplained part of a memory breakdown
        // (kernel stacks, page tables, vendor fields) can only be found by looking at all of them.
        let present: Map<String, Value> = map.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        let missing: Vec<&&str> = MEMINFO_KEYS
            .iter()
            .filter(|k| !map.contains_key(**k))
            .collect();
        meminfo.insert("values_kb".into(), Value::Object(present));
        meminfo.insert("missing_keys".into(), json!(missing));
    }
    root.insert("meminfo".into(), Value::Object(meminfo));

    let mut psi = source(PSI_MEMORY, &detected.psi);
    if let Some(p) = detected.psi.ok() {
        psi.insert("some".into(), psi_line_json(&p.some));
        if let Some(full) = &p.full {
            psi.insert("full".into(), psi_line_json(full));
        }
    }
    root.insert("psi".into(), Value::Object(psi));

    let mut swaps = source(SWAPS, &detected.swaps);
    if let Some(entries) = detected.swaps.ok() {
        let devices: Vec<Value> = entries
            .iter()
            .map(|e| {
                json!({
                    "name": e.name,
                    "kind": if e.is_zram { "zram" } else { "other" },
                    "size_kb": e.size_kb, "used_kb": e.used_kb, "priority": e.priority,
                })
            })
            .collect();
        swaps.insert("devices".into(), json!(devices));
    }
    root.insert("swaps".into(), Value::Object(swaps));

    let zram: Vec<Value> = detected
        .zram
        .iter()
        .map(|d| {
            let mut o = Map::new();
            o.insert("name".into(), json!(d.name));
            o.insert("disksize_bytes".into(), json!(d.disksize_bytes));
            o.insert("comp_algorithm".into(), json!(d.comp_algorithm));
            o.insert("backing_dev".into(), json!(d.backing_dev));
            o.insert("mm_stat_state".into(), json!(d.mm_stat.state()));
            if let Some(mm) = d.mm_stat.ok() {
                o.insert(
                    "mm_stat".into(),
                    json!({
                        "columns": mm.columns, "orig": mm.orig,
                        "compr": mm.compr, "mem_used": mm.mem_used,
                    }),
                );
            }
            Value::Object(o)
        })
        .collect();
    root.insert("zram".into(), json!({ "devices": zram }));

    let mut vmstat = source(VMSTAT, &detected.vmstat);
    if let Some(map) = detected.vmstat.ok() {
        let found: Map<String, Value> = VMSTAT_OF_INTEREST
            .iter()
            .filter_map(|name| map.get(*name).map(|v| (name.to_string(), json!(v))))
            .collect();
        let missing: Vec<&&str> = VMSTAT_OF_INTEREST
            .iter()
            .filter(|name| !map.contains_key(**name))
            .collect();
        let mut families = Map::new();
        for family in VMSTAT_FAMILIES {
            let names: Vec<&String> = map.keys().filter(|k| k.starts_with(family)).collect();
            families.insert((*family).to_string(), json!(names));
        }
        vmstat.insert("counters".into(), Value::Object(found));
        vmstat.insert("missing".into(), json!(missing));
        vmstat.insert("families".into(), Value::Object(families));
    }
    root.insert("vmstat".into(), Value::Object(vmstat));

    let mut sysctl = Map::new();
    for name in SYSCTLS {
        let path = format!("/proc/sys/vm/{name}");
        sysctl.insert(
            (*name).to_string(),
            match fs.read_str(&path) {
                Ok(text) => json!({ "value": text.parse::<i64>().ok(), "raw": text }),
                Err(err) => json!({ "error": err.code.as_str() }),
            },
        );
    }
    root.insert("sysctl".into(), Value::Object(sysctl));

    root.insert("paths".into(), path_report(fs));

    let packages = read_text(fs, PACKAGES_LIST);
    let mut pkg = source(PACKAGES_LIST, &packages);
    if let Some(text) = packages.ok() {
        pkg.insert("lines".into(), json!(text.lines().count()));
        pkg.insert(
            "first_line_fields".into(),
            json!(text.lines().next().map(|l| l.split_whitespace().count())),
        );
    }
    root.insert("packages_list".into(), Value::Object(pkg));

    let hidden: Vec<Value> = detected
        .hidden()
        .into_iter()
        .map(|(item, reason)| json!({ "item": item, "reason": reason }))
        .collect();
    root.insert(
        "decision".into(),
        json!({
            "supported": detected.supported(),
            "reason": (!detected.supported()).then(|| detected.unsupported_reason()),
            "features": detected.features(),
            "items": detected.items().iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
            "hidden": hidden,
        }),
    );
    Value::Object(root)
}

// ---------------------------------------------------------------------------------------------
// Instantaneous readings (the `collect` / `snapshot` half)
// ---------------------------------------------------------------------------------------------

/// Every item `items()` advertises, with its value, from one [`Detected`].
///
/// This is deliberately the *same* judgement as `items()`: a value is produced only where the
/// matching item exists, so `schema`, `collect` and `snapshot` cannot disagree about what this
/// device offers. A kernel without PSI produces no `mem.psi.*` value; a device without zRAM
/// produces no `mem.zram.*` value.
///
/// Sizes are converted to `kB` here (meminfo already is, `mm_stat` is bytes), PSI to hundredths of
/// a percent, and the zRAM ratio to an integer percentage recomputed from the summed byte totals —
/// never an average of per-device ratios, which would be wrong as soon as the devices differ in size.
pub fn readings(det: &Detected) -> Map<String, Value> {
    let mut out = Map::new();
    if !det.supported() {
        return out;
    }

    let kb = |key: &str| det.meminfo_value(key).map(|value| value as i64);
    for (id, key) in [
        ("mem.total", "MemTotal"),
        ("mem.available", "MemAvailable"),
        ("mem.free", "MemFree"),
        ("mem.cached", "Cached"),
        ("mem.anon", "AnonPages"),
        ("mem.shmem", "Shmem"),
        ("mem.slab", "Slab"),
    ] {
        if let Some(value) = kb(key) {
            out.insert(id.to_string(), json!(value));
        }
    }
    // Protocol §3.22 defines this one item as the sum of both fields.
    if let (Some(dirty), Some(writeback)) = (kb("Dirty"), kb("Writeback")) {
        out.insert("mem.dirty".to_string(), json!(dirty + writeback));
    }
    if let (Some(total), Some(free)) = (kb("SwapTotal"), kb("SwapFree")) {
        out.insert("mem.swap_total".to_string(), json!(total));
        out.insert("mem.swap_free".to_string(), json!(free));
    }
    // `/proc/swaps` sizes are kB and describe **usage**, which is what the item names imply.
    if let Some(entries) = det.swaps.ok() {
        let sum = |zram: bool| -> i64 {
            entries
                .iter()
                .filter(|entry| entry.is_zram == zram)
                .map(|entry| entry.used_kb as i64)
                .sum()
        };
        out.insert("mem.swap.zram_kb".to_string(), json!(sum(true)));
        out.insert("mem.swap.other_kb".to_string(), json!(sum(false)));
    }
    if let Some(psi) = det.psi.ok() {
        let mut lines: Vec<(&str, &PsiLine)> = vec![("some", &psi.some)];
        if let Some(full) = &psi.full {
            lines.push(("full", full));
        }
        for (prefix, line) in lines {
            for (suffix, value) in [
                ("avg10", line.avg10),
                ("avg60", line.avg60),
                ("avg300", line.avg300),
            ] {
                out.insert(
                    format!("mem.psi.{prefix}_{suffix}"),
                    json!(pct_to_hundredths(value)),
                );
            }
        }
    }
    if det.zram_sized() {
        let disksize: u64 = det
            .zram
            .iter()
            .filter_map(|device| device.disksize_bytes)
            .sum();
        out.insert("mem.zram.count".to_string(), json!(det.zram.len() as i64));
        out.insert(
            "mem.zram.disksize".to_string(),
            json!((disksize / 1024) as i64),
        );
    }
    if let Some((orig, compr, mem_used)) = det.zram_totals() {
        out.insert("mem.zram.orig".to_string(), json!((orig / 1024) as i64));
        out.insert("mem.zram.compr".to_string(), json!((compr / 1024) as i64));
        out.insert(
            "mem.zram.mem_used".to_string(),
            json!((mem_used / 1024) as i64),
        );
        if compr > 0 {
            // Integer percent, so 295 means 2.95×. Rounded once, from the sums.
            let ratio = (orig as f64 * 100.0 / compr as f64).round() as i64;
            out.insert("mem.zram.ratio".to_string(), json!(ratio));
        }
        out.insert(
            "mem.zram.writeback".to_string(),
            json!(i64::from(det.zram_writeback())),
        );
    }
    if let Some(enabled) = det.mglru {
        out.insert("mem.mglru".to_string(), json!(i64::from(enabled)));
    }
    for (id, key) in [
        ("mem.kernel_stack", "KernelStack"),
        ("mem.page_tables", "PageTables"),
        ("mem.percpu", "Percpu"),
        ("mem.gpu", "GpuTotal"),
        ("mem.vendor.rbin", "RbinTotal"),
        ("mem.vendor.cma", "CmaTotal"),
        ("mem.vendor.dmaheap", "DmaHeapPool"),
        ("mem.vmalloc_used", "VmallocUsed"),
    ] {
        if let Some(value) = kb(key) {
            out.insert(id.to_string(), json!(value));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Sampling helpers (the `history` half)
// ---------------------------------------------------------------------------------------------

/// vmstat `allocstall` suffixes that name a memory zone. Newer kernels also print
/// `allocstall_nomerge` / `allocstall_nosplit`, which are not zones and must not be summed in.
const ALLOCSTALL_ZONES: &[&str] = &["dma", "dma32", "normal", "movable", "device"];

/// Cumulative PSI stall time: `some` always, `full` when the kernel reports that line.
pub fn psi_totals(fs: &Sysfs) -> Option<(u64, Option<u64>)> {
    let Read::Ok(text) = read_text(fs, PSI_MEMORY) else {
        return None;
    };
    let psi = parse_psi(&text)?;
    Some((psi.some.total, psi.full.map(|line| line.total)))
}

/// The whole `/proc/vmstat` table, or `None` when it cannot be read.
pub fn vmstat(fs: &Sysfs) -> Option<BTreeMap<String, u64>> {
    let Read::Ok(text) = read_text(fs, VMSTAT) else {
        return None;
    };
    let table = parse_vmstat(&text);
    (!table.is_empty()).then_some(table)
}

/// Sum of the per-zone `allocstall_*` counters, or `None` when this kernel has none of the zones.
pub fn allocstall_total(table: &BTreeMap<String, u64>) -> Option<u64> {
    let mut total = 0u64;
    let mut seen = false;
    for zone in ALLOCSTALL_ZONES {
        if let Some(value) = table.get(&format!("allocstall_{zone}")) {
            total += value;
            seen = true;
        }
    }
    seen.then_some(total)
}

/// Refaulted pages: the split counters when present, otherwise the pre-5.9 single counter.
pub fn refault_total(table: &BTreeMap<String, u64>) -> Option<u64> {
    match (
        table.get("workingset_refault_anon"),
        table.get("workingset_refault_file"),
    ) {
        (Some(anon), Some(file)) => Some(anon + file),
        _ => table.get("workingset_refault").copied(),
    }
}

/// Boot identifier, used to tell "the counters are smaller because we rebooted" apart from a
/// kernel that reset a counter for its own reasons.
pub fn boot_id(fs: &Sysfs) -> Option<String> {
    fs.read_str("/proc/sys/kernel/random/boot_id")
        .ok()
        .filter(|id| !id.is_empty())
}

/// Seconds since boot (`/proc/uptime`, first column).
pub fn uptime_seconds(fs: &Sysfs) -> Option<f64> {
    let text = fs.read_str("/proc/uptime").ok()?;
    text.split_whitespace().next()?.parse().ok()
}

/// Kernel page size from the ELF auxiliary vector (`AT_PAGESZ`).
///
/// Never assume 4096: arm64 ships both 4 kB and 16 kB kernels, and a wrong page size scales every
/// swap rate by the same factor while still looking plausible. A host without `auxv` yields `None`,
/// which makes the caller omit the swap-rate series rather than invent one.
pub fn page_size(fs: &Sysfs) -> Option<u64> {
    /// `AT_PAGESZ` in `elf.h`.
    const AT_PAGESZ: u64 = 6;
    let bytes = std::fs::read(fs.resolve("/proc/self/auxv")).ok()?;
    let width = std::mem::size_of::<usize>();
    for pair in bytes.chunks_exact(width * 2) {
        let field = |offset: usize| -> u64 {
            let mut buf = [0u8; 8];
            buf[..width].copy_from_slice(&pair[offset..offset + width]);
            u64::from_ne_bytes(buf)
        };
        if field(0) == AT_PAGESZ {
            let value = field(width);
            return (value > 0).then_some(value);
        }
    }
    None
}
