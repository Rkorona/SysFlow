//! The CPU schema: which items the UI may show, their risk and whether they persist.
//!
//! Pure description, no sysfs writes. Separated from `mod.rs` because "what is adjustable" changes
//! for UI reasons, while "how to read and write it" changes for kernel reasons; keeping them in one
//! file made both edits look riskier than they are.

use super::*;

impl CpuModule {
    pub(super) fn adaptive_schema_items() -> Vec<SchemaItem> {
        let mut out = vec![
            SchemaItem::new("cpu.adaptive", ItemType::Bool)
                .risk(Risk::Medium)
                .persist(true),
            SchemaItem::new("cpu.adaptive.smart", ItemType::Bool)
                .risk(Risk::Low)
                .persist(true)
                .depends_on(vec!["cpu.adaptive".into()]),
            SchemaItem::new("cpu.adaptive.thermal", ItemType::Bool)
                .risk(Risk::Low)
                .persist(true)
                .depends_on(vec!["cpu.adaptive".into()]),
            SchemaItem::new("cpu.adaptive.battery", ItemType::Bool)
                .risk(Risk::Low)
                .persist(true)
                .depends_on(vec!["cpu.adaptive".into()]),
            SchemaItem::new("cpu.adaptive.psi", ItemType::Bool)
                .risk(Risk::Low)
                .persist(true)
                .depends_on(vec!["cpu.adaptive".into()]),
            SchemaItem::new(ITEM_ADAPTIVE_STATE, ItemType::Readonly),
        ];
        let ints: &[(&str, i64, i64)] = &[
            ("cpu.adaptive.load_t1", 1, 98),
            ("cpu.adaptive.load_t2", 2, 99),
            ("cpu.adaptive.load_t3", 3, 100),
            ("cpu.adaptive.load_cap1", 20, 100),
            ("cpu.adaptive.load_cap2", 20, 100),
            ("cpu.adaptive.load_cap3", 20, 100),
            ("cpu.adaptive.load_cap4", 20, 100),
            ("cpu.adaptive.load_down_rounds", 1, 10),
            ("cpu.adaptive.temp_high", 40, 100),
            ("cpu.adaptive.temp_low", 30, 99),
            ("cpu.adaptive.temp_step", 1, 20),
            ("cpu.adaptive.temp_max", 0, 40),
            ("cpu.adaptive.temp_release_rounds", 1, 10),
            ("cpu.adaptive.batt_low", 1, 50),
            ("cpu.adaptive.batt_critical", 1, 49),
            ("cpu.adaptive.batt_low_cut", 0, 30),
            ("cpu.adaptive.batt_critical_cut", 0, 30),
            ("cpu.adaptive.batt_hyst", 0, 20),
            ("cpu.adaptive.psi_threshold", 1, 100),
            ("cpu.adaptive.psi_cap", 20, 100),
        ];
        for (id, min, max) in ints {
            out.push(
                SchemaItem::new(id, ItemType::IntRange)
                    .range(*min, *max, 1)
                    .risk(Risk::Low)
                    .persist(true)
                    .depends_on(vec!["cpu.adaptive".into()]),
            );
        }
        out
    }

    pub(super) fn build_schema(&self) -> Schema {
        let n = self.cpu_count();
        let clusters = self.clusters();
        let mut freqs: Vec<i64> = clusters
            .iter()
            .flat_map(|c| c.freqs.iter().copied())
            .collect();
        freqs.sort_unstable();
        freqs.dedup();
        let mut govs: Vec<String> = clusters
            .iter()
            .flat_map(|c| c.governors.iter().cloned())
            .collect();
        govs.sort();
        govs.dedup();
        let freq_opts: Vec<Value> = freqs.iter().map(|f| json!(f)).collect();
        let gov_opts: Vec<Value> = govs.iter().map(|g| json!(g)).collect();

        let mut items = Vec::new();
        if n > 1 {
            // cores that cannot be switched are locked in the UI and rejected in apply
            let locked: Vec<usize> = (0..n).filter(|c| *c == 0 || !self.can_toggle(*c)).collect();
            items.push(
                SchemaItem::new(ITEM_ONLINE, ItemType::BoolList)
                    .count(n)
                    .locked(locked.clone())
                    .risk(Risk::Medium)
                    .persist(true),
            );
            items.push(
                SchemaItem::new(ITEM_SCREEN_CORES, ItemType::BoolList)
                    .count(n)
                    .locked(locked)
                    .risk(Risk::Medium)
                    .persist(true)
                    .depends_on(vec![ITEM_SCREEN_ENABLED.into()]),
            );
        }
        if !clusters.is_empty() {
            items.push(
                SchemaItem::new(ITEM_MAX, ItemType::IntEnum)
                    .scope(Scope::Cluster)
                    .options(freq_opts.clone())
                    .unit("kHz")
                    .persist(true),
            );
            items.push(
                SchemaItem::new(ITEM_MIN, ItemType::IntEnum)
                    .scope(Scope::Cluster)
                    .options(freq_opts.clone())
                    .unit("kHz")
                    .persist(true),
            );
            items.push(
                SchemaItem::new(ITEM_GOV, ItemType::Enum)
                    .scope(Scope::Cluster)
                    .options(gov_opts)
                    .persist(true),
            );
            items.push(
                SchemaItem::new(ITEM_CUR, ItemType::Readonly)
                    .scope(Scope::Cpu)
                    .unit("kHz"),
            );
            items.push(
                SchemaItem::new(ITEM_CLUSTER_CUR, ItemType::Readonly)
                    .scope(Scope::Cluster)
                    .unit("kHz"),
            );
            items.push(
                SchemaItem::new(ITEM_CLUSTER_HW_CUR, ItemType::Readonly)
                    .scope(Scope::Cluster)
                    .unit("kHz"),
            );
            items.push(SchemaItem::new(ITEM_CLUSTERS, ItemType::Readonly));
            items.push(
                SchemaItem::new(ITEM_HW_MIN, ItemType::Readonly)
                    .scope(Scope::Cluster)
                    .unit("kHz"),
            );
            items.push(
                SchemaItem::new(ITEM_HW_MAX, ItemType::Readonly)
                    .scope(Scope::Cluster)
                    .unit("kHz"),
            );
            items.push(SchemaItem::new(ITEM_TIME_IN_STATE, ItemType::Readonly));
            items.push(
                SchemaItem::new(ITEM_FREQ_TABLE, ItemType::Readonly)
                    .scope(Scope::Cluster)
                    .unit("kHz"),
            );
            items.push(
                SchemaItem::new(ITEM_SCREEN_FREQ_TABLE, ItemType::Readonly)
                    .scope(Scope::Cluster)
                    .unit("kHz"),
            );
            items.push(
                SchemaItem::new(ITEM_GOVERNOR_TABLE, ItemType::Readonly).scope(Scope::Cluster),
            );
            items.push(SchemaItem::new(ITEM_STAT, ItemType::Readonly));
            items.push(SchemaItem::new(ITEM_USAGE_STAT, ItemType::Readonly));
            items.push(
                SchemaItem::new(ITEM_SCREEN_ENABLED, ItemType::Bool)
                    .risk(Risk::Medium)
                    .persist(true),
            );
            // Unlike the normal static frequency picker, the legacy screen-off slider also
            // supports generated 1 MHz steps when a policy does not expose a frequency table.
            // Validate each cluster's value against that policy's hardware range below.
            items.push(
                SchemaItem::new(ITEM_SCREEN_MAX, ItemType::IntRange)
                    .scope(Scope::Cluster)
                    .range(0, i64::MAX, 1)
                    .unit("kHz")
                    .risk(Risk::Medium)
                    .persist(true)
                    .depends_on(vec![ITEM_SCREEN_ENABLED.into()]),
            );
            items.push(
                SchemaItem::new(ITEM_SCREEN_ACTIVE, ItemType::Bool)
                    .risk(Risk::Low)
                    .persist(false),
            );
            if n > 1 {
                items.push(SchemaItem::new(ITEM_SCREEN_SAVED_ONLINE, ItemType::Readonly).count(n));
            }
            items.extend(Self::adaptive_schema_items());
        }
        items.push(SchemaItem::new(ITEM_TEMP, ItemType::Readonly).unit("0.1C"));
        items.push(SchemaItem::new(ITEM_GPU_BUSY, ItemType::Readonly).unit("percent"));
        items.push(SchemaItem::new(ITEM_BATTERY_TEMP, ItemType::Readonly).unit("0.1C"));
        items.push(SchemaItem::new(ITEM_THERMAL_SENSORS, ItemType::Readonly));
        items.push(SchemaItem::new(
            ITEM_THERMAL_SENSORS_ALL,
            ItemType::Readonly,
        ));
        items.push(SchemaItem::new(ITEM_COOLING_DEVICES, ItemType::Readonly));
        items.push(SchemaItem::new(ITEM_GPU_INFO, ItemType::Readonly));
        items.extend(self.gpu_schema_items());
        items.extend(self.mem_schema_items());
        items.extend(self.walt_schema_items(&clusters));
        items.extend(self.cpuset_schema_items());
        Schema {
            module: "cpu".to_string(),
            supported: true,
            items,
        }
    }
}
