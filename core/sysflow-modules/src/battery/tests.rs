//! Battery module tests on a fake sysfs tree.

use super::*;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Tree {
    root: PathBuf,
}

impl Tree {
    /// A plausible phone battery: 4.2 V, 1.5 A charging, 4000/5000 mAh, 42.5 °C.
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sysflow-battery-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let t = Tree { root };
        t.put("sys/class/power_supply/battery/capacity", "83\n");
        t.put("sys/class/power_supply/battery/status", "Charging\n");
        t.put("sys/class/power_supply/battery/temp", "425\n");
        t.put("sys/class/power_supply/battery/voltage_now", "4200000\n");
        t.put("sys/class/power_supply/battery/current_now", "1500000\n");
        t.put("sys/class/power_supply/battery/charge_full", "4000000\n");
        t.put(
            "sys/class/power_supply/battery/charge_full_design",
            "5000000\n",
        );
        t.put("sys/class/power_supply/battery/health", "Good\n");
        t.put("sys/class/power_supply/battery/cycle_count", "212\n");
        t.put("sys/class/power_supply/usb/online", "1\n");
        t.put("sys/class/power_supply/battery/charge_counter", "3800000\n");
        t.put("sys/class/power_supply/usb/type", "USB_PD\n");
        t.put(
            "sys/class/power_supply/usb/input_current_limit",
            "3000000\n",
        );
        t.put("sys/class/power_supply/wireless/online", "0\n");
        t.put("sys/class/power_supply/ac/online", "0\n");
        t
    }

    fn put(&self, rel: &str, value: &str) {
        let path = self.root.join(rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(path, value).unwrap();
    }

    fn remove(&self, rel: &str) {
        let _ = fs::remove_file(self.root.join(rel));
    }

    fn module(&self) -> BatteryModule {
        BatteryModule::new(Sysfs::new(&self.root))
    }

    fn collect(&self) -> Value {
        self.module().collect(None).unwrap()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn probe_detects_the_battery_and_its_fields() {
    let t = Tree::new();
    let caps = t.module().probe();
    assert!(caps.supported);
    assert!(caps.reason.is_none());
    for feature in ["capacity", "status", "temp", "voltage_now", "current_now"] {
        assert!(
            caps.features.iter().any(|f| f == feature),
            "missing feature {feature} in {:?}",
            caps.features
        );
    }
    assert!(caps.features.iter().any(|f| f == "usb_online"));
}

#[test]
fn probe_reports_unsupported_without_a_battery_node() {
    let t = Tree::new();
    t.remove("sys/class/power_supply/battery/capacity");
    t.remove("sys/class/power_supply/battery/status");
    let caps = t.module().probe();
    assert!(!caps.supported);
    assert!(caps.reason.is_some());
}

#[test]
fn probe_falls_back_to_the_bms_node() {
    let t = Tree::new();
    t.remove("sys/class/power_supply/battery/capacity");
    t.remove("sys/class/power_supply/battery/status");
    t.put("sys/class/power_supply/bms/status", "Discharging\n");
    let caps = t.module().probe();
    assert!(caps.supported);
    let values = t.collect();
    assert_eq!(values["battery.status"], json!("Discharging"));
    // The bms node has no capacity, so the item is simply absent rather than null.
    assert!(values.get("battery.capacity").is_none());
}

#[test]
fn collect_normalizes_micro_units_and_derives_power() {
    let t = Tree::new();
    let values = t.collect();
    assert_eq!(values["battery.capacity"], json!(83));
    assert_eq!(values["battery.temp"], json!(425));
    // 4 200 000 µV -> 4 200 mV, 1 500 000 µA -> 1 500 mA, so 4 200 × 1 500 / 1000 = 6 300 mW.
    assert_eq!(values["battery.voltage"], json!(4200));
    assert_eq!(values["battery.current"], json!(1500));
    assert_eq!(values["battery.power"], json!(6300));
    // 4 000 000 µAh -> 4 000 mAh, and 4000/5000 = 80 %.
    assert_eq!(values["battery.charge_full"], json!(4000));
    assert_eq!(values["battery.charge_full_design"], json!(5000));
    assert_eq!(values["battery.health_percent"], json!(80));
}

#[test]
fn collect_leaves_already_normal_units_alone() {
    let t = Tree::new();
    t.put("sys/class/power_supply/battery/voltage_now", "4200\n");
    t.put("sys/class/power_supply/battery/current_now", "1500\n");
    t.put("sys/class/power_supply/battery/charge_full", "4000\n");
    t.put(
        "sys/class/power_supply/battery/charge_full_design",
        "5000\n",
    );
    let values = t.collect();
    assert_eq!(values["battery.voltage"], json!(4200));
    assert_eq!(values["battery.current"], json!(1500));
    assert_eq!(values["battery.power"], json!(6300));
    assert_eq!(values["battery.health_percent"], json!(80));
}

#[test]
fn collect_falls_back_to_power_now_when_voltage_is_missing() {
    let t = Tree::new();
    t.remove("sys/class/power_supply/battery/voltage_now");
    t.put("sys/class/power_supply/battery/power_now", "5000000\n");
    let values = t.collect();
    assert!(values.get("battery.voltage").is_none());
    assert_eq!(values["battery.power"], json!(5000));
}

#[test]
fn collect_omits_fields_the_device_does_not_expose() {
    let t = Tree::new();
    t.remove("sys/class/power_supply/battery/temp");
    t.remove("sys/class/power_supply/battery/cycle_count");
    t.remove("sys/class/power_supply/battery/health");
    let values = t.collect();
    assert!(values.get("battery.temp").is_none());
    assert!(values.get("battery.cycle_count").is_none());
    assert!(values.get("battery.health").is_none());
    // Everything else is still reported.
    assert_eq!(values["battery.capacity"], json!(83));
}

#[test]
fn collect_filters_by_requested_keys() {
    let t = Tree::new();
    let keys = vec!["battery.capacity".to_string(), "battery.temp".to_string()];
    let values = t.module().collect(Some(&keys)).unwrap();
    let obj = values.as_object().unwrap();
    assert_eq!(obj.len(), 2);
    assert_eq!(obj["battery.capacity"], json!(83));
    assert_eq!(obj["battery.temp"], json!(425));
}

#[test]
fn charging_flag_follows_status() {
    for (status, expected) in [
        ("Charging", true),
        ("Full", true),
        ("Discharging", false),
        ("Not charging", false),
        ("Unknown", false),
    ] {
        let t = Tree::new();
        t.put("sys/class/power_supply/battery/status", status);
        assert_eq!(
            t.collect()["battery.charging"],
            json!(expected),
            "status {status}"
        );
    }
}

#[test]
fn health_percent_is_clamped_and_needs_a_positive_design_capacity() {
    let t = Tree::new();
    t.put("sys/class/power_supply/battery/charge_full_design", "0\n");
    assert!(t.collect().get("battery.health_percent").is_none());

    // A replacement cell can read larger than the design capacity; never report over 100 %.
    t.put(
        "sys/class/power_supply/battery/charge_full_design",
        "3000000\n",
    );
    t.put("sys/class/power_supply/battery/charge_full", "4000000\n");
    assert_eq!(t.collect()["battery.health_percent"], json!(100));
}

#[test]
fn ac_and_usb_online_are_reported_when_present() {
    let t = Tree::new();
    let values = t.collect();
    assert_eq!(values["battery.ac_online"], json!(false));
    assert_eq!(values["battery.usb_online"], json!(true));

    t.remove("sys/class/power_supply/ac/online");
    let values = t.collect();
    assert!(values.get("battery.ac_online").is_none());
}

#[test]
fn every_schema_item_is_read_only_and_rejects_a_set() {
    let t = Tree::new();
    let schema = t.module().schema();
    assert_eq!(schema.module, "battery");
    assert!(!schema.items.is_empty());
    for item in &schema.items {
        assert_eq!(item.ty, ItemType::Readonly, "{} is not read-only", item.id);
        assert!(!item.persist, "{} must not persist", item.id);
        let request = json!({ item.id.clone(): 1 }).as_object().cloned().unwrap();
        assert!(
            schema.validate_request(&request).is_err(),
            "{} accepted a write",
            item.id
        );
    }
}

#[test]
fn apply_is_rejected_because_nothing_is_writable() {
    let t = Tree::new();
    let err = t.module().apply(&Map::new()).unwrap_err();
    assert_eq!(err.code.as_str(), "invalid_arg");
}

#[test]
fn schema_ids_are_all_reported_by_collect_on_a_fully_featured_device() {
    let t = Tree::new();
    let schema = t.module().schema();
    let values = t.collect();
    let reported = values.as_object().unwrap();
    for item in &schema.items {
        assert!(
            reported.contains_key(&item.id),
            "{} is in the schema but never collected",
            item.id
        );
    }
}

#[test]
fn cycle_count_reads_the_dedicated_node_first() {
    let t = Tree::new();
    let values = t.collect();
    // Tree::new() ships `cycle_count=212`, so this is the plain path.
    assert_eq!(values["battery.cycle_count"], json!(212));
}

#[test]
fn cycle_count_falls_back_to_vendor_node_names() {
    for node in [
        "battery_cycle",
        "batt_cycle",
        "batt_soc_cycle",
        "fg_cycle",
        "cycle",
    ] {
        let t = Tree::new();
        t.remove("sys/class/power_supply/battery/cycle_count");
        t.put(&format!("sys/class/power_supply/battery/{node}"), "87\n");
        assert_eq!(
            t.collect()["battery.cycle_count"],
            json!(87),
            "{node} was not read"
        );
    }
}

#[test]
fn cycle_count_falls_back_to_uevent() {
    let t = Tree::new();
    t.remove("sys/class/power_supply/battery/cycle_count");
    // A plausible driver uevent; the attribute file itself is absent.
    t.put(
        "sys/class/power_supply/battery/uevent",
        "POWER_SUPPLY_NAME=battery\nPOWER_SUPPLY_STATUS=Charging\nPOWER_SUPPLY_CYCLE_COUNT=341\n",
    );
    assert_eq!(t.collect()["battery.cycle_count"], json!(341));
}

#[test]
fn cycle_count_ignores_placeholder_values() {
    let t = Tree::new();
    // 0 is what drivers report when they do not track cycles; 99999 is nonsense.
    t.put("sys/class/power_supply/battery/cycle_count", "0\n");
    assert!(t.collect().get("battery.cycle_count").is_none());
    t.put("sys/class/power_supply/battery/cycle_count", "99999\n");
    assert!(t.collect().get("battery.cycle_count").is_none());
}

#[test]
fn soh_node_is_not_mistaken_for_cycles() {
    let t = Tree::new();
    t.remove("sys/class/power_supply/battery/cycle_count");
    // `soh` is state-of-health, not a cycle counter.
    t.put("sys/class/power_supply/battery/soh", "93\n");
    assert!(t.collect().get("battery.cycle_count").is_none());
}

#[test]
fn vendor_soh_outranks_the_capacity_ratio() {
    let t = Tree::new();
    // Full == design, so the ratio is 100 %; the driver's own SOH says 99.
    t.put("sys/class/power_supply/battery/charge_full", "5000000\n");
    t.put(
        "sys/class/power_supply/battery/uevent",
        "POWER_SUPPLY_NAME=battery\nPOWER_SUPPLY_SOH=99\n",
    );
    assert_eq!(t.collect()["battery.health_percent"], json!(99));
}

#[test]
fn capacity_ratio_is_used_when_no_soh_is_published() {
    let t = Tree::new();
    // 4000/5000 = 80 %.
    assert_eq!(t.collect()["battery.health_percent"], json!(80));
}

#[test]
fn implausible_soh_is_ignored() {
    let t = Tree::new();
    t.put(
        "sys/class/power_supply/battery/uevent",
        "POWER_SUPPLY_SOH=0\n",
    );
    // Falls back to the 80 % ratio rather than reporting 0.
    assert_eq!(t.collect()["battery.health_percent"], json!(80));
}

#[test]
fn wireless_charging_is_detected() {
    let t = Tree::new();
    assert_eq!(t.collect()["battery.wireless_online"], json!(false));

    t.put("sys/class/power_supply/wireless/online", "1\n");
    assert_eq!(t.collect()["battery.wireless_online"], json!(true));
}

#[test]
fn wireless_node_vendor_names_are_probed() {
    for node in ["wireless_charger", "wc", "dc"] {
        let t = Tree::new();
        t.remove("sys/class/power_supply/wireless/online");
        t.put(&format!("sys/class/power_supply/{node}/online"), "1\n");
        assert_eq!(
            t.collect()["battery.wireless_online"],
            json!(true),
            "{node} was not probed"
        );
    }
}

#[test]
fn charge_counter_gives_the_real_remaining_charge() {
    let t = Tree::new();
    // 3 800 000 µAh -> 3 800 mAh (the battery is at 83 %, cf. the 4 600 mAh full capacity below).
    assert_eq!(t.collect()["battery.charge_mah"], json!(3800));

    t.put("sys/class/power_supply/battery/charge_counter", "3800\n");
    assert_eq!(t.collect()["battery.charge_mah"], json!(3800));
}

#[test]
fn charger_type_and_current_limit_are_reported() {
    let t = Tree::new();
    let values = t.collect();
    assert_eq!(values["battery.usb_type"], json!("USB_PD"));
    // 3 000 000 µA -> 3 000 mA.
    assert_eq!(values["battery.input_current_limit"], json!(3000));
}

#[test]
fn fast_charging_follows_power_and_current_thresholds() {
    let t = Tree::new();
    // 1 500 mA at 4.2 V is 6.3 W: connected but not fast.
    assert_eq!(t.collect()["battery.fast_charging"], json!(false));

    // 3 000 mA at 4.2 V is 12.6 W -> fast.
    t.put("sys/class/power_supply/battery/current_now", "3000000\n");
    assert_eq!(t.collect()["battery.fast_charging"], json!(true));

    // Not charging means never "fast".
    t.put("sys/class/power_supply/battery/status", "Discharging\n");
    assert_eq!(t.collect()["battery.fast_charging"], json!(false));
}
