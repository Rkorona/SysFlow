//! Memory module tests on a fake procfs/sysfs tree.

use super::probe::*;
use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use sysflow_core::event::{EventStore, SystemEvent};
use sysflow_core::schema::ItemType;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

const MEMINFO_TEXT: &str = "\
MemTotal:       11800000 kB
MemFree:          400000 kB
MemAvailable:    4200000 kB
Buffers:           20000 kB
Cached:          3500000 kB
SwapCached:        10000 kB
AnonPages:       5200000 kB
Dirty:               120 kB
Writeback:             0 kB
Shmem:            300000 kB
Slab:             450000 kB
SwapTotal:       8388604 kB
SwapFree:        7000000 kB
HugePages_Total:       0
";

const PSI_TEXT: &str = "\
some avg10=1.25 avg60=0.50 avg300=0.10 total=123456
full avg10=0.40 avg60=0.20 avg300=0.05 total=65432
";

const SWAPS_TEXT: &str = "\
Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority
/dev/block/zram0                        partition\t8388604\t\t1388604\t\t-2
";

const VMSTAT_TEXT: &str = "\
nr_free_pages 100000
pgscan_kswapd 5000
pgscan_direct 120
allocstall_normal 3
allocstall_movable 4
workingset_refault_anon 11
workingset_refault_file 22
oom_kill 2
compact_stall 7
pswpin 900
pswpout 1800
";

struct Tree {
    root: PathBuf,
    /// Outside the fake root: the module's own state directory, as on a device.
    state_dir: PathBuf,
}

impl Tree {
    /// A plausible phone: PSI on, one zRAM device with a 9-column `mm_stat`, kernel 6.6.
    fn new() -> Self {
        let serial = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "sysflow-mem-test-{}-{}",
            std::process::id(),
            serial
        ));
        let state_dir = std::env::temp_dir().join(format!(
            "sysflow-mem-state-{}-{}",
            std::process::id(),
            serial
        ));
        let _ = fs::remove_dir_all(&state_dir);
        let t = Tree { root, state_dir };
        t.put("proc/meminfo", MEMINFO_TEXT);
        t.put("proc/pressure/memory", PSI_TEXT);
        t.put("proc/swaps", SWAPS_TEXT);
        t.put("proc/vmstat", VMSTAT_TEXT);
        t.put("proc/sys/kernel/osrelease", "6.6.30-android15-8-g1234\n");
        t.put("proc/sys/kernel/random/boot_id", "boot-a\n");
        t.put("proc/uptime", "1000.00 2000.00\n");
        t.put_bytes("proc/self/auxv", &auxv(&[(6, 4096)]));
        t.put("sys/block/zram0/disksize", "8589934592\n");
        t.put(
            "sys/block/zram0/mm_stat",
            "2147483648 536870912 570425344 0 600000000 12345 6789 100 200\n",
        );
        t.put("sys/block/zram0/comp_algorithm", "lz4 [zstd] lzo\n");
        t.put("sys/block/loop0/size", "0\n");
        for (name, value) in [
            ("swappiness", "100"),
            ("vfs_cache_pressure", "100"),
            ("dirty_ratio", "20"),
            ("dirty_background_ratio", "5"),
            ("dirty_bytes", "0"),
            ("dirty_background_bytes", "0"),
            ("min_free_kbytes", "27386"),
        ] {
            t.put(&format!("proc/sys/vm/{name}"), &format!("{value}\n"));
        }
        t
    }

    fn put(&self, rel: &str, content: &str) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn put_bytes(&self, rel: &str, content: &[u8]) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn remove(&self, rel: &str) {
        let path = self.root.join(rel);
        if path.is_dir() {
            let _ = fs::remove_dir_all(path);
        } else {
            let _ = fs::remove_file(path);
        }
    }

    fn module(&self) -> MemModule {
        MemModule::new(Sysfs::new(&self.root))
    }

    /// The production shape: a state directory, so `sample()` has a baseline.
    fn module_with_state(&self) -> MemModule {
        MemModule::with_state_dir(Sysfs::new(&self.root), &self.state_dir)
    }

    fn fs(&self) -> Sysfs {
        Sysfs::new(&self.root)
    }
}

/// An ELF auxiliary vector holding the given `(type, value)` pairs.
///
/// Built at the host's pointer width, which is what `probe::page_size` reads.
fn auxv(pairs: &[(u64, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (kind, value) in pairs {
        out.extend_from_slice(&(*kind as usize).to_ne_bytes());
        out.extend_from_slice(&(*value as usize).to_ne_bytes());
    }
    out
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(&self.state_dir);
    }
}

fn ids(module: &MemModule) -> Vec<String> {
    module.schema().items.into_iter().map(|i| i.id).collect()
}

// ---- parsers -------------------------------------------------------------------------------

#[test]
fn meminfo_parses_sizes_and_ignores_unitless_lines() {
    let map = parse_meminfo(MEMINFO_TEXT);
    assert_eq!(map["MemTotal"], 11_800_000);
    assert_eq!(map["SwapFree"], 7_000_000);
    assert_eq!(map["HugePages_Total"], 0);
    assert!(parse_meminfo("garbage\n::\n").is_empty());
}

#[test]
fn psi_parses_both_lines_and_tolerates_a_missing_full() {
    let psi = parse_psi(PSI_TEXT).unwrap();
    assert_eq!(psi.some.avg10, 1.25);
    assert_eq!(psi.some.total, 123_456);
    assert_eq!(psi.full.unwrap().avg300, 0.05);

    let some_only = parse_psi("some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n").unwrap();
    assert!(some_only.full.is_none());

    assert!(parse_psi("full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n").is_none());
    assert!(parse_psi("some avg10=oops\n").is_none());
    assert!(parse_psi("").is_none());
}

#[test]
fn psi_percent_becomes_integer_hundredths() {
    assert_eq!(pct_to_hundredths(1.23), 123);
    assert_eq!(pct_to_hundredths(0.0), 0);
    assert_eq!(pct_to_hundredths(0.005), 1);
    assert_eq!(pct_to_hundredths(99.999), 10_000);
}

#[test]
fn mm_stat_reads_the_first_three_columns_whatever_the_width() {
    let old = parse_mm_stat("100 50 60 0 70 1 2\n").unwrap();
    assert_eq!(
        (old.orig, old.compr, old.mem_used, old.columns),
        (100, 50, 60, 7)
    );
    let new = parse_mm_stat("100 50 60 0 70 1 2 3 4\n").unwrap();
    assert_eq!(new.columns, 9);
    assert!(parse_mm_stat("100 50\n").is_none());
    assert!(parse_mm_stat("100 x 60\n").is_none());
    assert!(parse_mm_stat("").is_none());
}

#[test]
fn swaps_split_zram_from_other_devices() {
    let text = format!(
        "{SWAPS_TEXT}/data/vendor/swap/swapfile                  file\t\t4194304\t\t100\t\t-3\n"
    );
    let entries = parse_swaps(&text);
    assert_eq!(entries.len(), 2);
    assert!(entries[0].is_zram);
    assert_eq!(
        (entries[0].size_kb, entries[0].used_kb, entries[0].priority),
        (8_388_604, 1_388_604, -2)
    );
    assert!(!entries[1].is_zram);
    assert!(parse_swaps("Filename Type Size Used Priority\n").is_empty());
}

#[test]
fn vmstat_is_looked_up_by_name() {
    let map = parse_vmstat(VMSTAT_TEXT);
    assert_eq!(map["pgscan_direct"], 120);
    assert_eq!(map["pswpout"], 1800);
    assert!(!map.contains_key("nonexistent"));
}

#[test]
fn kernel_release_decides_the_swappiness_ceiling() {
    assert_eq!(kernel_version("6.6.30-android15-8-g1234"), Some((6, 6)));
    assert_eq!(kernel_version("5.4.233"), Some((5, 4)));
    assert_eq!(kernel_version("garbage"), None);
    assert_eq!(swappiness_max(Some("6.6.30-android15")), 200);
    assert_eq!(swappiness_max(Some("5.8.0")), 200);
    assert_eq!(swappiness_max(Some("5.4.233")), 100);
    assert_eq!(swappiness_max(Some("garbage")), 100);
    assert_eq!(swappiness_max(None), 100);
}

// ---- detection, probe and schema -----------------------------------------------------------

#[test]
fn a_full_device_exposes_every_readonly_item_once() {
    let tree = Tree::new();
    let module = tree.module();
    let caps = module.probe();
    assert!(caps.supported);
    for feature in [
        "meminfo",
        "swap",
        "swaps",
        "psi",
        "psi_full",
        "zram",
        "zram_mm_stat",
        "vmstat",
    ] {
        assert!(
            caps.features.contains(&feature.to_string()),
            "missing feature {feature}"
        );
    }

    let schema = module.schema();
    assert!(schema.supported);
    let all = ids(&module);
    let mut unique = all.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(all.len(), unique.len(), "duplicate item ids");
    // The telemetry half is read-only; only the two tunables may be writable (Step 8).
    assert!(schema.items.iter().all(|item| item.ty == ItemType::Readonly
        || item.id == "mem.swappiness"
        || item.id == "mem.vfs_cache_pressure"));
    assert!(all.iter().all(|id| id.starts_with("mem.")));
    for expected in [
        "mem.total",
        "mem.available",
        "mem.free",
        "mem.cached",
        "mem.anon",
        "mem.shmem",
        "mem.slab",
        "mem.dirty",
        "mem.swap_total",
        "mem.swap_free",
        "mem.swap.zram_kb",
        "mem.swap.other_kb",
        "mem.psi.some_avg10",
        "mem.psi.some_avg60",
        "mem.psi.some_avg300",
        "mem.psi.full_avg10",
        "mem.psi.full_avg60",
        "mem.psi.full_avg300",
        "mem.zram.count",
        "mem.zram.disksize",
        "mem.zram.orig",
        "mem.zram.compr",
        "mem.zram.mem_used",
        "mem.zram.ratio",
    ] {
        assert!(
            all.contains(&expected.to_string()),
            "missing item {expected}"
        );
    }
}

#[test]
fn units_follow_the_protocol() {
    let tree = Tree::new();
    let items = tree.module().schema().items;
    let unit = |id: &str| {
        items
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.unit.clone())
    };
    assert_eq!(unit("mem.total").as_deref(), Some("kB"));
    assert_eq!(unit("mem.psi.full_avg60").as_deref(), Some("0.01%"));
    assert_eq!(unit("mem.zram.ratio").as_deref(), Some("%"));
    assert_eq!(unit("mem.zram.count"), None);
}

#[test]
fn no_meminfo_means_the_whole_module_is_unsupported() {
    let tree = Tree::new();
    tree.remove("proc/meminfo");
    let module = tree.module();
    let caps = module.probe();
    assert!(!caps.supported);
    assert!(caps.reason.unwrap().contains("meminfo"));
    let schema = module.schema();
    assert!(!schema.supported);
    assert!(schema.items.is_empty());
}

#[test]
fn a_kernel_without_psi_loses_only_the_psi_items() {
    let tree = Tree::new();
    tree.remove("proc/pressure/memory");
    let module = tree.module();
    let all = ids(&module);
    assert!(all.iter().all(|id| !id.starts_with("mem.psi.")));
    assert!(all.contains(&"mem.total".to_string()));
    assert!(!module.probe().features.contains(&"psi".to_string()));
    let hidden = probe::detect(&tree.fs()).hidden();
    assert!(hidden
        .iter()
        .any(|(item, reason)| *item == "mem.psi.*" && reason.contains("missing")));
}

#[test]
fn a_psi_file_that_exists_but_cannot_be_read_is_reported_differently() {
    let tree = Tree::new();
    // A directory in place of the file: `open` succeeds, `read` fails — the same shape as a kernel
    // with PSI compiled in but switched off (`psi=0`).
    tree.remove("proc/pressure/memory");
    fs::create_dir_all(tree.root.join("proc/pressure/memory")).unwrap();
    let detected = probe::detect(&tree.fs());
    assert_eq!(detected.psi.state(), "unreadable");
    assert!(ids(&tree.module())
        .iter()
        .all(|id| !id.starts_with("mem.psi.")));
    assert!(detected
        .hidden()
        .iter()
        .any(|(item, reason)| *item == "mem.psi.*" && reason.contains("cannot be read")));
}

#[test]
fn a_malformed_psi_file_is_not_trusted() {
    let tree = Tree::new();
    tree.put("proc/pressure/memory", "nonsense\n");
    let detected = probe::detect(&tree.fs());
    assert_eq!(detected.psi.state(), "malformed");
    assert!(ids(&tree.module())
        .iter()
        .all(|id| !id.starts_with("mem.psi.")));
}

#[test]
fn psi_without_a_full_line_keeps_only_the_some_items() {
    let tree = Tree::new();
    tree.put(
        "proc/pressure/memory",
        "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n",
    );
    let all = ids(&tree.module());
    assert!(all.contains(&"mem.psi.some_avg10".to_string()));
    assert!(!all.contains(&"mem.psi.full_avg10".to_string()));
}

#[test]
fn a_device_without_zram_has_no_zram_items_but_keeps_swap_totals() {
    let tree = Tree::new();
    tree.remove("sys/block/zram0");
    let all = ids(&tree.module());
    assert!(all.iter().all(|id| !id.starts_with("mem.zram.")));
    assert!(all.contains(&"mem.swap_total".to_string()));
}

#[test]
fn zram_without_mm_stat_reports_capacity_only() {
    let tree = Tree::new();
    tree.remove("sys/block/zram0/mm_stat");
    let all = ids(&tree.module());
    assert!(all.contains(&"mem.zram.disksize".to_string()));
    assert!(all.contains(&"mem.zram.count".to_string()));
    for gone in [
        "mem.zram.orig",
        "mem.zram.compr",
        "mem.zram.mem_used",
        "mem.zram.ratio",
    ] {
        assert!(!all.contains(&gone.to_string()), "{gone} should be hidden");
    }
}

#[test]
fn usage_items_need_mm_stat_on_every_zram_device() {
    let tree = Tree::new();
    tree.put("sys/block/zram1/disksize", "1073741824\n");
    // zram1 has no mm_stat: summing only zram0 would understate the total.
    let all = ids(&tree.module());
    assert!(all.contains(&"mem.zram.disksize".to_string()));
    assert!(!all.contains(&"mem.zram.ratio".to_string()));
    tree.put("sys/block/zram1/mm_stat", "10 5 6 0 7 1 2\n");
    assert!(ids(&tree.module()).contains(&"mem.zram.ratio".to_string()));
}

#[test]
fn non_zram_block_devices_are_not_mistaken_for_zram() {
    let tree = Tree::new();
    tree.put("sys/block/zramfs/disksize", "1\n");
    tree.put("sys/block/zram/disksize", "1\n");
    let detected = probe::detect(&tree.fs());
    assert_eq!(detected.zram.len(), 1);
    assert_eq!(detected.zram[0].name, "zram0");
}

#[test]
fn a_missing_mem_available_hides_only_that_item() {
    let tree = Tree::new();
    let without: String = MEMINFO_TEXT
        .lines()
        .filter(|l| !l.starts_with("MemAvailable"))
        .map(|l| format!("{l}\n"))
        .collect();
    tree.put("proc/meminfo", &without);
    let module = tree.module();
    let all = ids(&module);
    assert!(!all.contains(&"mem.available".to_string()));
    assert!(all.contains(&"mem.total".to_string()));
    assert!(module.probe().supported);
    assert!(probe::detect(&tree.fs())
        .hidden()
        .iter()
        .any(|(item, _)| *item == "mem.available"));
}

#[test]
fn missing_swaps_file_keeps_the_meminfo_swap_totals() {
    let tree = Tree::new();
    tree.remove("proc/swaps");
    let all = ids(&tree.module());
    assert!(all.contains(&"mem.swap_total".to_string()));
    assert!(!all.contains(&"mem.swap.zram_kb".to_string()));
    assert!(!all.contains(&"mem.swap.other_kb".to_string()));
}

#[test]
fn a_device_with_no_swap_at_all_still_reports_zero_split() {
    let tree = Tree::new();
    tree.put("proc/swaps", "Filename Type Size Used Priority\n");
    // A readable but empty table is a real answer ("no swap"), unlike a missing file.
    assert!(ids(&tree.module()).contains(&"mem.swap.zram_kb".to_string()));
}

// ---- the probe report ----------------------------------------------------------------------

#[test]
fn the_report_carries_raw_readings_and_the_decision() {
    let tree = Tree::new();
    let report = report(&tree.fs());

    assert_eq!(report["kernel"]["swappiness_max"], 200);
    assert_eq!(report["kernel"]["version"], serde_json::json!([6, 6]));
    assert_eq!(report["meminfo"]["state"], "ok");
    assert_eq!(report["meminfo"]["values_kb"]["MemTotal"], 11_800_000);
    assert_eq!(report["psi"]["some"]["avg10"], 1.25);
    assert_eq!(report["psi"]["full"]["total_us"], 65_432);
    assert_eq!(report["swaps"]["devices"][0]["kind"], "zram");
    assert_eq!(report["zram"]["devices"][0]["mm_stat"]["columns"], 9);
    assert_eq!(
        report["zram"]["devices"][0]["comp_algorithm"],
        "lz4 [zstd] lzo"
    );
    assert_eq!(report["vmstat"]["counters"]["pswpin"], 900);
    assert_eq!(
        report["vmstat"]["families"]["allocstall"],
        serde_json::json!(["allocstall_movable", "allocstall_normal"])
    );
    assert_eq!(report["sysctl"]["swappiness"]["value"], 100);
    assert_eq!(report["sysctl"]["dirty_bytes"]["value"], 0);
    assert_eq!(report["packages_list"]["state"], "missing");
    assert_eq!(report["decision"]["supported"], true);
    assert!(report["decision"]["items"].as_array().unwrap().len() >= 20);
    assert_eq!(report["decision"]["hidden"], serde_json::json!([]));
}

#[test]
fn the_report_explains_what_is_missing() {
    let tree = Tree::new();
    tree.remove("proc/pressure/memory");
    tree.remove("sys/block/zram0");
    tree.remove("proc/sys/vm/dirty_bytes");
    let report = report(&tree.fs());
    assert_eq!(report["psi"]["state"], "missing");
    assert_eq!(report["sysctl"]["dirty_bytes"]["error"], "unsupported");
    let hidden: Vec<String> = report["decision"]["hidden"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["item"].as_str().unwrap().to_string())
        .collect();
    assert!(hidden.contains(&"mem.psi.*".to_string()));
    assert!(hidden.contains(&"mem.zram.*".to_string()));
}

#[test]
fn the_report_lists_every_meminfo_key_and_probes_extra_paths() {
    let tree = Tree::new();
    tree.put(
        "proc/meminfo",
        &format!("{MEMINFO_TEXT}KernelStack:  40000 kB\nPageTables:  90000 kB\n"),
    );
    tree.put("sys/kernel/mm/lru_gen/enabled", "0x0007\n");
    tree.put("sys/kernel/dma_heap/system", "");
    let report = report(&tree.fs());
    assert_eq!(report["meminfo"]["values_kb"]["PageTables"], 90_000);
    assert_eq!(report["meminfo"]["values_kb"]["KernelStack"], 40_000);
    assert_eq!(
        report["paths"]["/sys/kernel/mm/lru_gen/enabled"]["state"],
        "ok"
    );
    assert_eq!(
        report["paths"]["/sys/kernel/mm/lru_gen/enabled"]["value"],
        "0x0007"
    );
    assert_eq!(report["paths"]["/sys/kernel/dma_heap"]["state"], "dir");
    assert_eq!(report["paths"]["/sys/kernel/dma_heap"]["entries"], 1);
    assert_eq!(
        report["paths"]["/sys/class/kgsl/kgsl/page_alloc"]["state"],
        "missing"
    );
}

#[test]
fn the_report_measures_packages_list_when_readable() {
    let tree = Tree::new();
    tree.put(
        "data/system/packages.list",
        "com.example.app 10346 0 /data/user/0/com.example.app default 3002\ncom.other 10347 0 /data/user/0/com.other default none\n",
    );
    let report = report(&tree.fs());
    assert_eq!(report["packages_list"]["state"], "ok");
    assert_eq!(report["packages_list"]["lines"], 2);
    assert_eq!(report["packages_list"]["first_line_fields"], 6);
}

// ---- module contract -----------------------------------------------------------------------

#[test]
fn step_two_module_is_in_snapshot_and_still_read_only() {
    let tree = Tree::new();
    let module = tree.module();
    assert_eq!(module.id(), "mem");
    assert!(module.in_snapshot());
    let mut req = Map::new();
    req.insert("mem.total".into(), serde_json::json!(1));
    assert!(module.apply(&req).is_err());
    assert!(module.sample().is_none());
}

// ---- collect / snapshot --------------------------------------------------------------------

#[test]
fn collect_reports_exactly_the_items_the_schema_lists() {
    let tree = Tree::new();
    let module = tree.module();
    let mut schema: Vec<String> = module
        .schema()
        .items
        .into_iter()
        .map(|item| item.id)
        .collect();
    let mut collected: Vec<String> = module
        .collect(None)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    schema.sort();
    collected.sort();
    assert_eq!(
        schema, collected,
        "collect and schema must agree about this device"
    );
}

#[test]
fn collect_converts_the_sources_to_protocol_units() {
    let tree = Tree::new();
    let value = tree.module().collect(None).unwrap();
    assert_eq!(value["mem.total"], serde_json::json!(11_800_000));
    assert_eq!(value["mem.available"], serde_json::json!(4_200_000));
    // By contract `mem.dirty` is Dirty + Writeback.
    assert_eq!(value["mem.dirty"], serde_json::json!(120));
    // `/proc/swaps` usage, split by device kind.
    assert_eq!(value["mem.swap.zram_kb"], serde_json::json!(1_388_604));
    assert_eq!(value["mem.swap.other_kb"], serde_json::json!(0));
    // PSI percent -> hundredths of a percent.
    assert_eq!(value["mem.psi.some_avg10"], serde_json::json!(125));
    assert_eq!(value["mem.psi.some_avg60"], serde_json::json!(50));
    assert_eq!(value["mem.psi.full_avg300"], serde_json::json!(5));
    // zRAM bytes -> kB; ratio as an integer percentage of the sums.
    assert_eq!(value["mem.zram.count"], serde_json::json!(1));
    assert_eq!(value["mem.zram.disksize"], serde_json::json!(8_388_608));
    assert_eq!(value["mem.zram.orig"], serde_json::json!(2_097_152));
    assert_eq!(value["mem.zram.compr"], serde_json::json!(524_288));
    assert_eq!(value["mem.zram.mem_used"], serde_json::json!(557_056));
    assert_eq!(value["mem.zram.ratio"], serde_json::json!(400));
}

#[test]
fn zram_ratio_is_recomputed_from_the_sums() {
    let tree = Tree::new();
    tree.put("sys/block/zram1/disksize", "1073741824\n");
    // A second device that compresses 1:1. Averaging per-device ratios would give 250.
    tree.put(
        "sys/block/zram1/mm_stat",
        "1000000 1000000 100000 0 100000 0 0\n",
    );
    let value = tree.module().collect(None).unwrap();
    assert_eq!(value["mem.zram.count"], serde_json::json!(2));
    let orig = 2_147_483_648u64 + 1_000_000;
    let compr = 536_870_912u64 + 1_000_000;
    let expected = (orig as f64 * 100.0 / compr as f64).round() as i64;
    assert_eq!(value["mem.zram.ratio"], serde_json::json!(expected));
    assert_ne!(value["mem.zram.ratio"], serde_json::json!(250));
}

#[test]
fn collect_filters_by_key_and_ignores_unknown_ones() {
    let tree = Tree::new();
    let keys = vec!["mem.total".to_string(), "mem.nope".to_string()];
    let value = tree.module().collect(Some(&keys)).unwrap();
    let map = value.as_object().unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map["mem.total"], serde_json::json!(11_800_000));
}

#[test]
fn a_source_that_is_absent_removes_its_items_from_collect_too() {
    let tree = Tree::new();
    tree.remove("proc/pressure/memory");
    tree.remove("sys/block/zram0");
    let value = tree.module().collect(None).unwrap();
    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    assert!(
        keys.iter().all(|key| !key.starts_with("mem.psi.")),
        "{keys:?}"
    );
    assert!(
        keys.iter().all(|key| !key.starts_with("mem.zram.")),
        "{keys:?}"
    );
    assert!(value.get("mem.total").is_some());
}

/// Diagnostic, not a gate (host timings do not predict the device):
/// `cargo test -p sysflow-modules mem -- --ignored --nocapture`.
#[test]
#[ignore = "diagnostic: prints collect() cost on the fake tree"]
fn diagnostic_collect_cost() {
    let tree = Tree::new();
    let module = tree.module();
    let iterations = 200;
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = std::time::Instant::now();
        let _ = module.collect(None).unwrap();
        samples.push(start.elapsed());
    }
    samples.sort();
    println!(
        "collect() median over {iterations} runs: {:?} (fake tree)",
        samples[samples.len() / 2]
    );

    // `sample()` also reads the counters, the auxv, and rewrites the state file, so it is the
    // realistic number for the daemon's five-minute tick.
    let mut sample_times = Vec::with_capacity(iterations);
    for index in 0..iterations {
        let start = std::time::Instant::now();
        let _ = history::sample(&tree.fs(), &tree.state_dir, 1_000_000 + index as u64, None);
        sample_times.push(start.elapsed());
    }
    sample_times.sort();
    println!(
        "sample() median over {iterations} runs: {:?} (fake tree, includes the state write)",
        sample_times[sample_times.len() / 2]
    );
}

// ---- memory composition (step 2b) ----------------------------------------------------------

/// The vendor / kernel fields behind the composition bar. Each one is optional: it exists only when
/// `/proc/meminfo` reports it, because a kernel that does not know the field must not get a `0`.
#[test]
fn composition_items_appear_only_when_the_kernel_reports_them() {
    let tree = Tree::new();
    let plain = tree.module();
    assert!(
        !ids(&plain).iter().any(|id| id.starts_with("mem.vendor.")),
        "no vendor keys in the base tree"
    );
    assert!(!ids(&plain).contains(&"mem.gpu".to_string()));
    assert!(plain.collect(None).unwrap().get("mem.gpu").is_none());

    let with_extra = format!(
        "{MEMINFO_TEXT}\
KernelStack: 116456 kB
PageTables: 244788 kB
Percpu: 13344 kB
GpuTotal: 407392 kB
RbinTotal: 409600 kB
CmaTotal: 638976 kB
DmaHeapPool: 22528 kB
VmallocUsed: 358584 kB
"
    );
    tree.put("proc/meminfo", &with_extra);
    let module = tree.module();
    let all = ids(&module);
    for expected in [
        "mem.kernel_stack",
        "mem.page_tables",
        "mem.percpu",
        "mem.gpu",
        "mem.vendor.rbin",
        "mem.vendor.cma",
        "mem.vendor.dmaheap",
        "mem.vmalloc_used",
    ] {
        assert!(all.contains(&expected.to_string()), "missing {expected}");
    }

    let snapshot = module.collect(None).unwrap();
    assert_eq!(snapshot["mem.gpu"], serde_json::json!(407_392));
    assert_eq!(snapshot["mem.vendor.cma"], serde_json::json!(638_976));
    assert_eq!(snapshot["mem.kernel_stack"], serde_json::json!(116_456));
    assert_eq!(snapshot["mem.vmalloc_used"], serde_json::json!(358_584));
}

#[test]
fn mglru_is_published_only_when_the_kernel_exposes_it() {
    let tree = Tree::new();
    // The base tree has no `lru_gen` node: no item, and no invented `0`.
    assert!(!ids(&tree.module()).contains(&"mem.mglru".to_string()));

    tree.put("sys/kernel/mm/lru_gen/enabled", "0x0003\n");
    let module = tree.module();
    assert!(ids(&module).contains(&"mem.mglru".to_string()));
    assert_eq!(
        module.collect(None).unwrap()["mem.mglru"],
        serde_json::json!(1)
    );

    tree.put("sys/kernel/mm/lru_gen/enabled", "0x0000\n");
    assert_eq!(
        tree.module().collect(None).unwrap()["mem.mglru"],
        serde_json::json!(0)
    );
}

#[test]
fn zram_writeback_is_published_as_a_flag() {
    let tree = Tree::new();
    // A plain zram device has no backing store.
    assert_eq!(
        tree.module().collect(None).unwrap()["mem.zram.writeback"],
        serde_json::json!(0)
    );

    tree.put("sys/block/zram0/backing_dev", "/dev/block/loop37\n");
    let backed = tree.module().collect(None).unwrap();
    assert_eq!(backed["mem.zram.writeback"], serde_json::json!(1));
    // The flag rides next to the numbers it qualifies, but it is not a tunable.
    assert!(ids(&tree.module()).contains(&"mem.zram.writeback".to_string()));
}

#[test]
fn a_device_without_zram_has_no_writeback_item() {
    let tree = Tree::new();
    tree.remove("sys/block/zram0");
    let module = tree.module();
    assert!(!ids(&module).contains(&"mem.zram.writeback".to_string()));
    assert!(module
        .collect(None)
        .unwrap()
        .get("mem.zram.writeback")
        .is_none());
}

// ---- writable half (step 8) ----------------------------------------------------------------

fn max_of(module: &MemModule, id: &str) -> i64 {
    module
        .schema()
        .items
        .into_iter()
        .find(|item| item.id == id)
        .and_then(|item| item.max)
        .unwrap_or(-1)
}

#[test]
fn the_swappiness_ceiling_follows_the_kernel() {
    let tree = Tree::new();
    assert_eq!(max_of(&tree.module(), "mem.swappiness"), 200); // 6.6
    tree.put("proc/sys/kernel/osrelease", "5.4.233\n");
    assert_eq!(max_of(&tree.module(), "mem.swappiness"), 100);
    // An unparsable release keeps the conservative ceiling instead of guessing 200.
    tree.put("proc/sys/kernel/osrelease", "garbage\n");
    assert_eq!(max_of(&tree.module(), "mem.swappiness"), 100);
}

#[test]
fn tunables_are_writable_and_show_up_in_the_snapshot() {
    let tree = Tree::new();
    let module = tree.module();
    let ids = ids(&module);
    for expected in ["mem.swappiness", "mem.vfs_cache_pressure"] {
        assert!(ids.contains(&expected.to_string()), "missing {expected}");
    }
    let snapshot = module.collect(None).unwrap();
    assert_eq!(snapshot["mem.swappiness"], serde_json::json!(100));
    assert_eq!(snapshot["mem.vfs_cache_pressure"], serde_json::json!(100));

    let mut request = Map::new();
    request.insert("mem.swappiness".into(), serde_json::json!(130));
    let result = module.apply(&request).unwrap();
    assert!(result.mismatch.is_empty(), "{:?}", result.mismatch);
    assert_eq!(result.applied["mem.swappiness"], serde_json::json!(130));
    assert_eq!(tree.fs().read_i64("/proc/sys/vm/swappiness").unwrap(), 130);
}

#[test]
fn an_out_of_range_value_is_rejected_without_half_writing() {
    let tree = Tree::new();
    let module = tree.module();
    let mut request = Map::new();
    request.insert("mem.swappiness".into(), serde_json::json!(150));
    request.insert("mem.vfs_cache_pressure".into(), serde_json::json!(9999));
    assert!(module.apply(&request).is_err());
    // Validation runs before any write: both stayed at their original values.
    assert_eq!(tree.fs().read_i64("/proc/sys/vm/swappiness").unwrap(), 100);
    assert_eq!(
        tree.fs()
            .read_i64("/proc/sys/vm/vfs_cache_pressure")
            .unwrap(),
        100
    );
}

#[test]
fn a_read_only_item_is_not_writable() {
    let tree = Tree::new();
    let mut request = Map::new();
    request.insert("mem.total".into(), serde_json::json!(1));
    assert!(tree.module().apply(&request).is_err());
}

#[test]
fn drift_reports_a_mismatch_and_forgets_it_when_the_value_returns() {
    let tree = Tree::new();
    let module = tree.module();
    let settings = serde_json::json!({"mem.swappiness": 130})
        .as_object()
        .unwrap()
        .clone();

    assert_eq!(module.drift(&settings).unwrap(), vec!["mem.swappiness"]);
    tree.put("proc/sys/vm/swappiness", "130\n");
    assert!(module.drift(&settings).unwrap().is_empty());
}

#[test]
fn repairs_that_keep_being_undone_are_suppressed_and_reported_once() {
    let tree = Tree::new();
    let store = EventStore::new(&tree.root);
    let module = tree.module_with_state().with_events(store);
    let settings = serde_json::json!({"mem.swappiness": 130})
        .as_object()
        .unwrap()
        .clone();

    // Three episodes: something resets the value, we notice, we put it back.
    for attempt in 1..=3 {
        tree.put("proc/sys/vm/swappiness", "60\n");
        let drifted = module.drift(&settings).unwrap();
        if attempt < 3 {
            assert_eq!(drifted, vec!["mem.swappiness"], "attempt {attempt}");
        } else {
            assert!(
                drifted.is_empty(),
                "the third episode must stop and report instead of repairing"
            );
        }
        tree.put("proc/sys/vm/swappiness", "130\n");
        let _ = module.drift(&settings).unwrap(); // our repair landed
    }

    // The give-up is announced exactly once...
    let events = mem_events(&tree);
    assert_eq!(
        events.len(),
        1,
        "{:?}",
        events.iter().map(|e| &e.event_type).collect::<Vec<_>>()
    );
    assert_eq!(events[0].event_type, "mem_setting_contested");
    assert_eq!(
        events[0].metadata["item"],
        serde_json::json!("mem.swappiness")
    );

    // ...and the item stays alone for the rest of the window.
    tree.put("proc/sys/vm/swappiness", "60\n");
    assert!(module.drift(&settings).unwrap().is_empty());
    assert_eq!(mem_events(&tree).len(), 1);
}

#[test]
fn sanitize_boot_drops_values_this_kernel_cannot_accept() {
    let tree = Tree::new();
    tree.put("proc/sys/kernel/osrelease", "5.4.233\n");
    let module = tree.module();
    let settings = serde_json::json!({
        "mem.swappiness": 180,          // 5.4 caps at 100
        "mem.vfs_cache_pressure": 200,  // fine
        "mem.total": 1,                 // not a tunable
    })
    .as_object()
    .unwrap()
    .clone();

    let kept = module.sanitize_boot(&settings);
    assert!(!kept.contains_key("mem.swappiness"));
    assert_eq!(kept["mem.vfs_cache_pressure"], serde_json::json!(200));
}

// ---- history sampling ----------------------------------------------------------------------

fn sample_at(tree: &Tree, ts: u64) -> Option<Map<String, Value>> {
    history::sample(&tree.fs(), &tree.state_dir, ts, None)
}

/// Write a PSI file with the given cumulative totals; `avg60` stays at 2.77 % for the metadata.
fn set_psi(tree: &Tree, some_total: u64, full_total: u64) {
    tree.put(
        "proc/pressure/memory",
        &format!(
            "some avg10=1.25 avg60=5.00 avg300=0.10 total={some_total}\n\
             full avg10=3.17 avg60=2.77 avg300=0.89 total={full_total}\n"
        ),
    );
}

fn mem_events(tree: &Tree) -> Vec<SystemEvent> {
    // `EventFilter::default().limit == 0`, which `list` clamps to 1: ask for a page explicitly.
    let filter = sysflow_core::event::EventFilter {
        limit: 100,
        ascending: true,
        ..Default::default()
    };
    EventStore::new(&tree.root)
        .list(filter)
        .unwrap()
        .into_iter()
        .filter(|event| event.module == "mem")
        .collect()
}

/// One interval: `stall_us` of `full` stall spread over `elapsed_s` seconds.
fn advance_psi(tree: &Tree, some: &mut u64, full: &mut u64, stall_us: u64) {
    *full += stall_us;
    *some += stall_us;
    set_psi(tree, *some, *full);
}

/// The counters one interval later: +0.6 s of `some` stall, +5 direct scans, +2 allocstalls,
/// 2 048 pages in, 4 096 pages out, +10 refaults, one kernel OOM.
const VMSTAT_LATER: &str = "\
nr_free_pages 100000
pgscan_kswapd 5000
pgscan_direct 125
allocstall_normal 5
allocstall_movable 4
workingset_refault_anon 11
workingset_refault_file 32
oom_kill 3
compact_stall 7
pswpin 2948
pswpout 5896
";

const PSI_LATER: &str = "\
some avg10=1.25 avg60=0.50 avg300=0.10 total=723456
full avg10=0.40 avg60=0.20 avg300=0.05 total=65432
";

#[test]
fn the_first_sample_reports_instantaneous_values_only() {
    let tree = Tree::new();
    let first = sample_at(&tree, 1_000_000).expect("a sample");
    assert!(first.contains_key("available_kb"));
    assert!(first.contains_key("swap_used_kb"));
    assert!(first.contains_key("zram_compr_kb"));
    for needs_baseline in [
        "psi_some_pct",
        "psi_full_pct",
        "pgscan_direct",
        "allocstall",
        "swap_in_kb",
        "swap_out_kb",
        "refault",
        "oom_kill",
    ] {
        assert!(
            !first.contains_key(needs_baseline),
            "{needs_baseline} cannot be known yet"
        );
    }
}

#[test]
fn the_second_sample_reports_differences_over_the_measured_interval() {
    let tree = Tree::new();
    sample_at(&tree, 1_000_000);
    tree.put("proc/pressure/memory", PSI_LATER);
    tree.put("proc/vmstat", VMSTAT_LATER);

    let second = sample_at(&tree, 1_000_060).expect("a sample");
    // 0.6 s of stall in a 60 s interval is exactly 1 %, i.e. 100 hundredths.
    assert_eq!(second["psi_some_pct"], serde_json::json!(100));
    assert_eq!(second["psi_full_pct"], serde_json::json!(0));
    assert_eq!(second["pgscan_direct"], serde_json::json!(5));
    // Only the zone suffixes are summed: 2 of allocstall_normal, none of the others.
    assert_eq!(second["allocstall"], serde_json::json!(2));
    assert_eq!(second["refault"], serde_json::json!(10));
    assert_eq!(second["oom_kill"], serde_json::json!(1));
    assert_eq!(second["swap_in_kb"], serde_json::json!(2_048 * 4096 / 1024));
    assert_eq!(
        second["swap_out_kb"],
        serde_json::json!(4_096 * 4096 / 1024)
    );
}

#[test]
fn a_reboot_omits_the_first_difference_point() {
    let tree = Tree::new();
    sample_at(&tree, 1_000_000);
    tree.put("proc/sys/kernel/random/boot_id", "boot-b\n");
    tree.put("proc/vmstat", VMSTAT_LATER);

    let after_reboot = sample_at(&tree, 1_000_060).expect("a sample");
    assert!(after_reboot.contains_key("available_kb"));
    assert!(
        !after_reboot.contains_key("pgscan_direct"),
        "a reboot must produce a gap, not a delta against the previous boot"
    );

    // The next interval works again, measured from the post-reboot baseline.
    let next = sample_at(&tree, 1_000_120).expect("a sample");
    assert_eq!(next["pgscan_direct"], serde_json::json!(0));
}

#[test]
fn a_counter_regression_is_treated_as_a_reboot() {
    let tree = Tree::new();
    sample_at(&tree, 1_000_000);
    // Same boot id, but the kernel counters went backwards.
    tree.put(
        "proc/vmstat",
        &VMSTAT_TEXT.replace("pgscan_direct 120", "pgscan_direct 3"),
    );
    let after = sample_at(&tree, 1_000_060).expect("a sample");
    assert!(!after.contains_key("pgscan_direct"));
    assert!(!after.contains_key("allocstall"));
}

#[test]
fn an_uptime_regression_is_treated_as_a_reboot() {
    let tree = Tree::new();
    sample_at(&tree, 1_000_000);
    tree.put("proc/uptime", "10.00 20.00\n");
    let after = sample_at(&tree, 1_000_060).expect("a sample");
    assert!(!after.contains_key("pgscan_direct"));
}

#[test]
fn a_missing_vmstat_removes_only_its_own_series() {
    let tree = Tree::new();
    sample_at(&tree, 1_000_000);
    tree.remove("proc/vmstat");
    let after = sample_at(&tree, 1_000_060).expect("a sample");
    assert!(!after.contains_key("pgscan_direct"));
    assert!(!after.contains_key("swap_in_kb"));
    assert!(
        after.contains_key("psi_some_pct"),
        "PSI is a separate source and must survive"
    );
}

#[test]
fn swap_rates_use_the_kernel_page_size() {
    let tree = Tree::new();
    tree.put_bytes("proc/self/auxv", &auxv(&[(6, 16_384)]));
    sample_at(&tree, 1_000_000);
    tree.put("proc/vmstat", VMSTAT_LATER);

    let after = sample_at(&tree, 1_000_060).expect("a sample");
    // 2 048 pages of 16 kB, not of an assumed 4 kB.
    assert_eq!(
        after["swap_in_kb"],
        serde_json::json!(2_048 * 16_384 / 1024)
    );
}

#[test]
fn without_a_page_size_the_swap_rate_series_is_omitted() {
    let tree = Tree::new();
    tree.remove("proc/self/auxv");
    sample_at(&tree, 1_000_000);
    tree.put("proc/vmstat", VMSTAT_LATER);

    let after = sample_at(&tree, 1_000_060).expect("a sample");
    assert!(!after.contains_key("swap_in_kb"));
    assert!(
        after.contains_key("pgscan_direct"),
        "only the series that needs the page size is dropped"
    );
}

#[test]
fn the_baseline_survives_a_daemon_restart() {
    let tree = Tree::new();
    sample_at(&tree, 1_000_000);

    // A new module instance, as after `daemon start`, reads the same state file.
    let reopened = history::StateFile::new(&tree.state_dir).load();
    assert_eq!(reopened.version, 1);
    assert_eq!(reopened.boot_id, "boot-a");
    assert_eq!(reopened.ts, 1_000_000);
    assert_eq!(reopened.counters.pgscan_direct, Some(120));

    tree.put("proc/vmstat", VMSTAT_LATER);
    let after = sample_at(&tree, 1_000_060).expect("a sample");
    assert_eq!(after["pgscan_direct"], serde_json::json!(5));
}

#[test]
fn a_module_without_a_state_directory_does_not_sample() {
    let tree = Tree::new();
    assert!(tree.module().sample().is_none());
    assert!(tree.module().sample_interval().is_none());

    let with_state = tree.module_with_state();
    assert_eq!(
        with_state.sample_interval(),
        Some(std::time::Duration::from_secs(300))
    );
}

// ---- pressure events -----------------------------------------------------------------------

const BASE_SOME: u64 = 123_456;
const BASE_FULL: u64 = 65_432;
/// 9 s of `full` stall in a 300 s interval is exactly 3.00 %.
const HIGH_STALL_US: u64 = 9_000_000;

#[test]
fn pressure_needs_three_consecutive_high_samples() {
    let tree = Tree::new();
    let store = EventStore::new(&tree.root);
    history::sample(&tree.fs(), &tree.state_dir, 1_000_000, Some(&store)).unwrap();

    let (mut some, mut full) = (BASE_SOME, BASE_FULL);
    for step in 1..=3u64 {
        advance_psi(&tree, &mut some, &mut full, HIGH_STALL_US);
        let ts = 1_000_000 + 300 * step;
        let sample = history::sample(&tree.fs(), &tree.state_dir, ts, Some(&store)).unwrap();
        assert_eq!(sample["psi_full_pct"], serde_json::json!(300));

        let events = mem_events(&tree);
        if step < 3 {
            assert!(events.is_empty(), "fired after {step} sample(s)");
        } else {
            assert_eq!(events.len(), 1, "one event per episode");
            assert_eq!(events[0].event_type, "mem_pressure_high");
            assert_eq!(events[0].category, "pressure");
            assert_eq!(events[0].severity, "warning");
            assert_eq!(events[0].metadata["psi_full_pct"], serde_json::json!(300));
            assert_eq!(
                events[0].metadata["available_kb"],
                serde_json::json!(4_200_000)
            );
        }
    }
}

#[test]
fn a_dead_band_sample_breaks_the_streak() {
    let tree = Tree::new();
    let store = EventStore::new(&tree.root);
    history::sample(&tree.fs(), &tree.state_dir, 1_000_000, Some(&store)).unwrap();
    let (mut some, mut full) = (BASE_SOME, BASE_FULL);

    // Two highs, one middling sample (2.00 %), then two highs: never three in a row.
    for (index, stall) in [
        HIGH_STALL_US,
        HIGH_STALL_US,
        6_000_000,
        HIGH_STALL_US,
        HIGH_STALL_US,
    ]
    .into_iter()
    .enumerate()
    {
        advance_psi(&tree, &mut some, &mut full, stall);
        let ts = 1_000_000 + 300 * (index as u64 + 1);
        history::sample(&tree.fs(), &tree.state_dir, ts, Some(&store)).unwrap();
        assert!(mem_events(&tree).is_empty(), "after sample {}", index + 1);
    }

    // The sixth consecutive high sample completes the run.
    advance_psi(&tree, &mut some, &mut full, HIGH_STALL_US);
    history::sample(&tree.fs(), &tree.state_dir, 1_001_800, Some(&store)).unwrap();
    assert_eq!(mem_events(&tree).len(), 1);
}

#[test]
fn recovery_is_reported_once_after_three_quiet_samples() {
    let tree = Tree::new();
    let store = EventStore::new(&tree.root);
    history::sample(&tree.fs(), &tree.state_dir, 1_000_000, Some(&store)).unwrap();
    let (mut some, mut full) = (BASE_SOME, BASE_FULL);

    for step in 1..=3u64 {
        advance_psi(&tree, &mut some, &mut full, HIGH_STALL_US);
        history::sample(
            &tree.fs(),
            &tree.state_dir,
            1_000_000 + 300 * step,
            Some(&store),
        )
        .unwrap();
    }
    assert_eq!(mem_events(&tree).len(), 1);

    for step in 4..=6u64 {
        advance_psi(&tree, &mut some, &mut full, 0);
        history::sample(
            &tree.fs(),
            &tree.state_dir,
            1_000_000 + 300 * step,
            Some(&store),
        )
        .unwrap();
        let events = mem_events(&tree);
        if step < 6 {
            assert_eq!(events.len(), 1, "still inside the quiet run");
        } else {
            assert_eq!(events.len(), 2);
            assert_eq!(events[1].event_type, "mem_pressure_recovered");
            assert_eq!(events[1].severity, "info");
        }
    }

    // A quiet sample after the event must not repeat it.
    advance_psi(&tree, &mut some, &mut full, 0);
    history::sample(&tree.fs(), &tree.state_dir, 1_002_100, Some(&store)).unwrap();
    assert_eq!(mem_events(&tree).len(), 2);
}

#[test]
fn a_sample_after_a_long_sleep_raises_no_event() {
    let tree = Tree::new();
    let store = EventStore::new(&tree.root);
    history::sample(&tree.fs(), &tree.state_dir, 1_000_000, Some(&store)).unwrap();
    let (mut some, mut full) = (BASE_SOME, BASE_FULL);

    // Two hours later: enough stall to average 3 %, but the interval says nothing about pressure.
    advance_psi(&tree, &mut some, &mut full, 216_000_000);
    let sample = history::sample(&tree.fs(), &tree.state_dir, 1_007_200, Some(&store)).unwrap();
    assert_eq!(sample["psi_full_pct"], serde_json::json!(300));
    assert!(mem_events(&tree).is_empty());
}

#[test]
fn a_reboot_does_not_announce_recovery() {
    let tree = Tree::new();
    let store = EventStore::new(&tree.root);
    history::sample(&tree.fs(), &tree.state_dir, 1_000_000, Some(&store)).unwrap();
    let (mut some, mut full) = (BASE_SOME, BASE_FULL);
    for step in 1..=3u64 {
        advance_psi(&tree, &mut some, &mut full, HIGH_STALL_US);
        history::sample(
            &tree.fs(),
            &tree.state_dir,
            1_000_000 + 300 * step,
            Some(&store),
        )
        .unwrap();
    }
    assert_eq!(mem_events(&tree).len(), 1);

    // Reboot, then quiet samples: the episode ended with the reboot, not with a recovery.
    tree.put("proc/sys/kernel/random/boot_id", "boot-b\n");
    tree.put("proc/vmstat", VMSTAT_TEXT);
    for step in 4..=6u64 {
        history::sample(
            &tree.fs(),
            &tree.state_dir,
            1_000_000 + 300 * step,
            Some(&store),
        )
        .unwrap();
    }
    let events = mem_events(&tree);
    assert_eq!(events.len(), 1, "no recovered event across a reboot");
}

#[test]
fn a_kernel_oom_kill_emits_a_notice() {
    let tree = Tree::new();
    let store = EventStore::new(&tree.root);
    history::sample(&tree.fs(), &tree.state_dir, 1_000_000, Some(&store)).unwrap();

    tree.put(
        "proc/vmstat",
        &VMSTAT_TEXT.replace("oom_kill 2", "oom_kill 3"),
    );
    history::sample(&tree.fs(), &tree.state_dir, 1_000_300, Some(&store)).unwrap();

    let events = mem_events(&tree);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "mem_oom_kill");
    assert_eq!(events[0].severity, "notice");
    assert_eq!(events[0].metadata["psi_full_pct"], serde_json::json!(0));
    // The page words the event as "N processes", so the count must travel with it.
    assert_eq!(events[0].metadata["count"], serde_json::json!(1));
}

// ---- exit-info parsing ---------------------------------------------------------------------

const EXIT_INFO_TEXT: &str = "\
        ApplicationExitInfo #14:
          timestamp=2026-11-23 23:26:09.526 pid=4434 realUid=1000 packageUid=1000 definingUid=1000 user=0
          process=com.example.last reason=3 (LOW_MEMORY) subreason=0 (UNKNOWN) status=0
          importance=400 pss=0.00 rss=103MB state=empty trace=null
        ApplicationExitInfo #15:
          timestamp=2026-11-23 22:47:05.034 pid=27389 realUid=1000 packageUid=10123 definingUid=10123 user=0
          process=com.example.newest reason=3 (LOW_MEMORY) subreason=0 (UNKNOWN) status=0
          importance=400 pss=0.00 rss=87MB state=empty trace=null
";

#[test]
fn the_newest_low_memory_record_is_chosen_by_timestamp_not_by_position() {
    let record = history::parse_newest_low_memory(EXIT_INFO_TEXT).expect("a record");
    // `com.example.last` is listed first but was killed at 23:26, after the 22:47 record below it.
    assert_eq!(record["package"], serde_json::json!("com.example.last"));
    assert_eq!(
        record["timestamp"],
        serde_json::json!("2026-11-23 23:26:09.526")
    );
    assert_eq!(record["package_uid"], serde_json::json!("1000"));
    assert_eq!(record["rss"], serde_json::json!("103MB"));
}

#[test]
fn the_order_of_the_dump_does_not_change_the_answer() {
    // The same two records with their order swapped: grouping per package is not chronological.
    let mut blocks: Vec<&str> = EXIT_INFO_TEXT
        .split("        ApplicationExitInfo")
        .collect();
    blocks.reverse();
    let swapped = blocks.join("        ApplicationExitInfo");
    let record = history::parse_newest_low_memory(&swapped).expect("a record");
    assert_eq!(record["package"], serde_json::json!("com.example.last"));
}

#[test]
fn a_record_without_a_timestamp_loses_to_one_that_has_it() {
    let text = "\
        ApplicationExitInfo #1:
          process=com.example.undated reason=3 (LOW_MEMORY) subreason=0 (UNKNOWN) status=0
          importance=400 pss=0.00 rss=10MB state=empty trace=null
        ApplicationExitInfo #2:
          timestamp=2026-11-20 01:00:00.000 pid=1 realUid=1000 packageUid=10200 definingUid=10200 user=0
          process=com.example.dated reason=3 (LOW_MEMORY) subreason=0 (UNKNOWN) status=0
          importance=400 pss=0.00 rss=20MB state=empty trace=null
        ApplicationExitInfo #3:
          process=com.example.undated2 reason=3 (LOW_MEMORY) subreason=0 (UNKNOWN) status=0
          importance=400 pss=0.00 rss=30MB state=empty trace=null
";
    let record = history::parse_newest_low_memory(text).expect("a record");
    assert_eq!(record["package"], serde_json::json!("com.example.dated"));
    assert_eq!(record["package_uid"], serde_json::json!("10200"));
}

#[test]
fn a_dump_without_low_memory_records_yields_nothing() {
    assert!(history::parse_newest_low_memory("").is_none());
    assert!(history::parse_newest_low_memory(
        "  process=com.example reason=4 (USER_REQUESTED) status=0\n"
    )
    .is_none());
}

// ---- repair guard --------------------------------------------------------------------------

const SWAP_ID: &str = "mem.swappiness";

#[test]
fn a_mismatch_that_never_clears_is_retried_every_tick_and_then_given_up() {
    use tunables::Decision::*;
    let guard = tunables::Guard::default();
    // The value never comes back between ticks (the write is refused, or reverted within the tick):
    // the old design answered "already repairing" forever and never reported anything.
    assert_eq!(guard.check(SWAP_ID, 1_000), Repair);
    assert_eq!(guard.check(SWAP_ID, 1_010), Repair);
    assert_eq!(guard.check(SWAP_ID, 1_020), JustSuppressed);
    assert_eq!(guard.check(SWAP_ID, 1_030), Suppressed);
    assert_eq!(
        guard.check(SWAP_ID, 1_020 + tunables::SUPPRESS_S - 1),
        Suppressed
    );
    // After the quiet period the item gets a fresh budget.
    assert_eq!(guard.check(SWAP_ID, 1_020 + tunables::SUPPRESS_S), Repair);
}

#[test]
fn the_two_calls_inside_one_tick_cost_one_attempt() {
    use tunables::Decision::*;
    let guard = tunables::Guard::default();
    // Before the repair, and again right after it to see what is left.
    assert_eq!(guard.check(SWAP_ID, 2_000), Repair);
    assert_eq!(guard.check(SWAP_ID, 2_000), Repair);
    assert_eq!(guard.check(SWAP_ID, 2_001), Repair);
    // Only the next tick is a second attempt, so a third tick is still needed to give up.
    assert_eq!(guard.check(SWAP_ID, 2_010), Repair);
    assert_eq!(guard.check(SWAP_ID, 2_020), JustSuppressed);
}

#[test]
fn attempts_older_than_the_window_are_forgotten() {
    use tunables::Decision::*;
    let guard = tunables::Guard::default();
    assert_eq!(guard.check(SWAP_ID, 0), Repair);
    guard.cleared(SWAP_ID);
    assert_eq!(guard.check(SWAP_ID, 100), Repair);
    guard.cleared(SWAP_ID);
    // Two attempts, but both are older than the window by now: this is a fresh problem, not the
    // third strike.
    let later = 100 + tunables::WINDOW_S + 1;
    assert_eq!(guard.check(SWAP_ID, later), Repair);
}

#[test]
fn clearing_makes_the_next_mismatch_a_new_observation() {
    use tunables::Decision::*;
    let guard = tunables::Guard::default();
    assert_eq!(guard.check(SWAP_ID, 3_000), Repair);
    guard.cleared(SWAP_ID);
    // Same second, but the value was fine in between: that is a second episode.
    assert_eq!(guard.check(SWAP_ID, 3_000), Repair);
    guard.cleared(SWAP_ID);
    assert_eq!(guard.check(SWAP_ID, 3_000), JustSuppressed);
}

#[test]
fn a_value_that_cannot_be_repaired_is_reported_instead_of_retried_forever() {
    let tree = Tree::new();
    let store = EventStore::new(&tree.root);
    let module = tree.module_with_state().with_events(store);
    let settings = serde_json::json!({"mem.swappiness": 130})
        .as_object()
        .unwrap()
        .clone();
    // The fixture sits at 100 and nothing ever fixes it.
    assert_eq!(module.drift_at(&settings, 5_000).unwrap(), vec![SWAP_ID]);
    // The residual check right after the repair is the same tick: still drifted, not "repaired".
    assert_eq!(module.drift_at(&settings, 5_000).unwrap(), vec![SWAP_ID]);
    assert_eq!(module.drift_at(&settings, 5_010).unwrap(), vec![SWAP_ID]);
    assert!(module.drift_at(&settings, 5_020).unwrap().is_empty());
    assert!(module.drift_at(&settings, 5_030).unwrap().is_empty());

    let events = mem_events(&tree);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "mem_setting_contested");
    assert_eq!(events[0].metadata["live"], serde_json::json!(100));
}

// ---- elapsed time --------------------------------------------------------------------------

#[test]
fn rates_use_the_boot_clock_so_a_wall_clock_jump_cannot_distort_them() {
    let tree = Tree::new();
    sample_at(&tree, 1_000_000);
    // 3 s of `full` stall in 300 s of real time, but the wall clock jumped an hour (a time resync).
    tree.put("proc/uptime", "1300.00 2000.00\n");
    set_psi(&tree, 123_456 + 3_000_000, 65_432 + 3_000_000);
    let second = sample_at(&tree, 1_003_600).expect("a sample");
    // 3 s / 300 s = 1 %, not 3 s / 3600 s = 0.08 %.
    assert_eq!(second["psi_full_pct"], serde_json::json!(100));
}
