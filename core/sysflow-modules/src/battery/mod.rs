//! Battery module — a **read-only** view of the platform power supply.
//!
//! Deliberately write-free: no tunable is exposed, so this module never touches `apply`, the
//! enforcement loop, or the boot/safe-mode recovery paths. Charge limiting nodes
//! (`charge_control_limit`, `input_suspend`, ...) are vendor-specific and high risk; they are
//! intentionally left out until they can ship behind an explicit experimental switch.
//!
//! Units are normalised here, never in the clients: kernels report `voltage_now` in µV on most
//! devices but mV on some, and the same split exists for current (µA/mA) and charge (µAh/mAh).
//! The protocol therefore only ever carries mV / mA / mAh / mW / 0.1 °C.

use std::time::Duration;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::module::{ApplyResult, Caps, Module, Sample};
use sysflow_core::schema::{ItemType, Schema, SchemaItem};
use sysflow_core::sysfs::Sysfs;

/// Battery power-supply nodes, in probe order. Most Android kernels use `battery`; a few expose
/// the charge controller as `bms` or with a capitalised directory name.
const BATTERY_NODES: &[&str] = &[
    "/sys/class/power_supply/battery",
    "/sys/class/power_supply/Battery",
    "/sys/class/power_supply/bms",
];

const AC_NODES: &[&str] = &["/sys/class/power_supply/ac", "/sys/class/power_supply/AC"];

const USB_NODES: &[&str] = &["/sys/class/power_supply/usb", "/sys/class/power_supply/USB"];

/// Wireless charging is exposed under different names per vendor.
const WIRELESS_NODES: &[&str] = &[
    "/sys/class/power_supply/wireless",
    "/sys/class/power_supply/wireless_charger",
    "/sys/class/power_supply/wc",
    "/sys/class/power_supply/dc",
];

/// Conventional "fast charge" line, matching what Android's own battery UI treats as fast.
const FAST_CHARGE_MW: i64 = 12_000;
const FAST_CHARGE_MA: i64 = 2_400;

/// Above these thresholds a reading is in micro-units and is divided down. The gap between the
/// two conventions is enormous (4.2 V is either `4200` or `4200000`), so a magnitude test is
/// safer than guessing from a file name that differs per vendor.
const MICRO_VOLT_THRESHOLD: i64 = 100_000;
const MICRO_AMP_THRESHOLD: i64 = 10_000;
const MICRO_AMP_HOUR_THRESHOLD: i64 = 100_000;
const MICRO_WATT_THRESHOLD: i64 = 1_000_000;

/// Cycle-count nodes in probe order. Vendors disagree wildly; these are the names observed in the
/// wild (Samsung, Xiaomi, Qualcomm FG, MTK). `soh` and `count` are deliberately excluded: they carry
/// health or unrelated counters, and reading them as cycles would report fiction.
const CYCLE_NODES: &[&str] = &[
    "cycle_count",
    "battery_cycle",
    "batt_cycle",
    "batt_soc_cycle",
    "fg_cycle",
    "cycle",
];

/// `uevent` is the most portable source: most power-supply drivers publish their attributes there
/// even when the per-attribute file is not readable.
const CYCLE_UEVENT_KEYS: &[&str] = &[
    "POWER_SUPPLY_CYCLE_COUNT",
    "POWER_SUPPLY_BATTERY_CYCLE",
    "POWER_SUPPLY_FG_CYCLE",
];

pub struct BatteryModule {
    fs: Sysfs,
}

impl BatteryModule {
    pub fn new(fs: Sysfs) -> Self {
        BatteryModule { fs }
    }

    /// First power-supply directory that looks like a battery.
    fn node(&self) -> Option<&'static str> {
        BATTERY_NODES.iter().copied().find(|node| {
            self.fs.exists(&format!("{node}/capacity")) || self.fs.exists(&format!("{node}/status"))
        })
    }

    fn read_text(&self, node: &str, field: &str) -> Option<String> {
        self.fs
            .read_str(&format!("{node}/{field}"))
            .ok()
            .filter(|value| !value.is_empty())
    }

    fn read_number(&self, node: &str, field: &str) -> Option<i64> {
        self.fs.read_i64(&format!("{node}/{field}")).ok()
    }

    /// `usb/type` names the negotiated mode (`USB_PD`, `USB_DCP`, `USB_CDP`, `USB_HVDCP`, …).
    fn charger_type(&self) -> Option<String> {
        USB_NODES.iter().find_map(|node| {
            self.fs
                .read_str(&format!("{node}/type"))
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.trim().to_string())
        })
    }

    /// Negotiated input current ceiling, in mA. Read from whichever node exposes it.
    fn input_current_limit(&self) -> Option<i64> {
        for node in [USB_NODES, AC_NODES].concat() {
            for field in ["input_current_limit", "current_max", "input_current_max"] {
                if let Ok(raw) = self.fs.read_i64(&format!("{node}/{field}")) {
                    let normalized = normalize_current_ma(raw);
                    if normalized > 0 {
                        return Some(normalized);
                    }
                }
            }
        }
        None
    }

    fn online(&self, nodes: &[&str]) -> Option<bool> {
        nodes.iter().find_map(|node| {
            self.fs
                .read_str(&format!("{node}/online"))
                .ok()
                .map(|value| value.trim() == "1")
        })
    }
}

/// `4200000` µV and `4200` mV both become `4200` mV.
fn normalize_voltage_mv(raw: i64) -> i64 {
    if raw.abs() > MICRO_VOLT_THRESHOLD {
        raw / 1000
    } else {
        raw
    }
}

/// `1500000` µA and `1500` mA both become `1500` mA. The sign is preserved: kernels report
/// discharging current as negative on some platforms and positive on others.
fn normalize_current_ma(raw: i64) -> i64 {
    if raw.abs() > MICRO_AMP_THRESHOLD {
        raw / 1000
    } else {
        raw
    }
}

fn normalize_charge_mah(raw: i64) -> i64 {
    if raw.abs() > MICRO_AMP_HOUR_THRESHOLD {
        raw / 1000
    } else {
        raw
    }
}

fn normalize_power_mw(raw: i64) -> i64 {
    if raw.abs() > MICRO_WATT_THRESHOLD {
        raw / 1000
    } else {
        raw
    }
}

fn is_charging_status(status: &str) -> bool {
    matches!(status.to_ascii_lowercase().as_str(), "charging" | "full")
}

/// A cycle count plus where it came from, so callers can be honest about provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CycleCount {
    pub value: u32,
    pub source: &'static str,
}

/// The power-supply directory this module reads, for callers outside the module (the CLI's
/// `battery health` fallback needs to look at the same node).
pub fn battery_node(fs: &Sysfs) -> Option<&'static str> {
    BATTERY_NODES
        .iter()
        .copied()
        .find(|node| fs.exists(&format!("{node}/capacity")) || fs.exists(&format!("{node}/status")))
}

/// Cycle count from the dedicated nodes first, then from `uevent`.
pub fn read_cycle_count(fs: &Sysfs, node: &str) -> Option<CycleCount> {
    for candidate in CYCLE_NODES {
        if let Ok(raw) = fs.read_i64(&format!("{node}/{candidate}")) {
            if let Some(value) = sane_cycles(raw) {
                return Some(CycleCount {
                    value,
                    source: "sysfs 循环计数寄存器",
                });
            }
        }
    }
    let uevent = read_uevent(fs, node)?;
    let raw = uevent_value(&uevent, CYCLE_UEVENT_KEYS)?;
    sane_cycles(raw).map(|value| CycleCount {
        value,
        source: "内核电源驱动 uevent",
    })
}

/// State of health as a percentage, when the driver publishes one.
///
/// This is the vendor's own calibration and outranks a raw capacity ratio, which reads 100 % on any
/// device whose `charge_full` simply mirrors the design capacity — common on Samsung, where the
/// calibrated value lives in BSOH instead.
pub fn read_soh(fs: &Sysfs, node: &str) -> Option<u32> {
    let uevent = read_uevent(fs, node)?;
    let raw = uevent_value(
        &uevent,
        &["POWER_SUPPLY_SOH", "POWER_SUPPLY_HEALTH_PERCENT"],
    )?;
    (20..=105).contains(&raw).then_some(raw as u32)
}

fn read_uevent(fs: &Sysfs, node: &str) -> Option<String> {
    fs.read_str(&format!("{node}/uevent")).ok()
}

fn uevent_value(uevent: &str, keys: &[&str]) -> Option<i64> {
    for line in uevent.lines() {
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        if !keys
            .iter()
            .any(|candidate| key.trim().eq_ignore_ascii_case(candidate))
        {
            continue;
        }
        if let Ok(value) = raw.trim().trim_matches(['[', ']']).parse::<i64>() {
            return Some(value);
        }
    }
    None
}

/// Guard against placeholder values: some drivers expose 0 or absurd counts.
fn sane_cycles(raw: i64) -> Option<u32> {
    (1..=9999).contains(&raw).then_some(raw as u32)
}

impl Module for BatteryModule {
    fn id(&self) -> &'static str {
        "battery"
    }

    fn probe(&self) -> Caps {
        let Some(node) = self.node() else {
            return Caps::unsupported("no battery power supply found");
        };
        let mut features = Vec::new();
        for field in [
            "capacity",
            "status",
            "temp",
            "voltage_now",
            "current_now",
            "power_now",
            "health",
            "cycle_count",
            "charge_full",
            "charge_full_design",
        ] {
            if self.fs.exists(&format!("{node}/{field}")) {
                features.push(field.to_string());
            }
        }
        if self.online(AC_NODES).is_some() {
            features.push("ac_online".to_string());
        }
        if self.online(USB_NODES).is_some() {
            features.push("usb_online".to_string());
        }
        Caps {
            supported: true,
            features,
            reason: None,
        }
    }

    fn schema(&self) -> Schema {
        Schema {
            module: "battery".to_string(),
            supported: true,
            items: vec![
                SchemaItem::new("battery.capacity", ItemType::Readonly).unit("%"),
                SchemaItem::new("battery.status", ItemType::Readonly),
                SchemaItem::new("battery.charging", ItemType::Readonly),
                SchemaItem::new("battery.temp", ItemType::Readonly).unit("0.1C"),
                SchemaItem::new("battery.voltage", ItemType::Readonly).unit("mV"),
                SchemaItem::new("battery.current", ItemType::Readonly).unit("mA"),
                SchemaItem::new("battery.power", ItemType::Readonly).unit("mW"),
                SchemaItem::new("battery.health", ItemType::Readonly),
                SchemaItem::new("battery.health_percent", ItemType::Readonly).unit("%"),
                SchemaItem::new("battery.cycle_count", ItemType::Readonly),
                SchemaItem::new("battery.charge_full", ItemType::Readonly).unit("mAh"),
                SchemaItem::new("battery.charge_full_design", ItemType::Readonly).unit("mAh"),
                SchemaItem::new("battery.ac_online", ItemType::Readonly),
                SchemaItem::new("battery.usb_online", ItemType::Readonly),
                SchemaItem::new("battery.wireless_online", ItemType::Readonly),
                SchemaItem::new("battery.charge_mah", ItemType::Readonly).unit("mAh"),
                SchemaItem::new("battery.usb_type", ItemType::Readonly),
                SchemaItem::new("battery.input_current_limit", ItemType::Readonly).unit("mA"),
                SchemaItem::new("battery.fast_charging", ItemType::Readonly),
            ],
        }
    }

    fn collect(&self, keys: Option<&[String]>) -> Result<Value> {
        let Some(node) = self.node() else {
            return Ok(json!({}));
        };
        let mut out = Map::new();

        if let Some(capacity) = self.read_number(node, "capacity") {
            out.insert("battery.capacity".to_string(), json!(capacity));
        }
        if let Some(status) = self.read_text(node, "status") {
            out.insert(
                "battery.charging".to_string(),
                json!(is_charging_status(&status)),
            );
            out.insert("battery.status".to_string(), json!(status));
        }
        if let Some(temp) = self.read_number(node, "temp") {
            out.insert("battery.temp".to_string(), json!(temp));
        }

        let voltage = self
            .read_number(node, "voltage_now")
            .map(normalize_voltage_mv);
        let current = self
            .read_number(node, "current_now")
            .map(normalize_current_ma);
        if let Some(mv) = voltage {
            out.insert("battery.voltage".to_string(), json!(mv));
        }
        if let Some(ma) = current {
            out.insert("battery.current".to_string(), json!(ma));
        }
        // Compute from V×I when both halves exist: `power_now` is missing or wrong on many ROMs.
        let power = match (voltage, current) {
            (Some(mv), Some(ma)) => Some((mv as f64 * ma as f64 / 1000.0).round() as i64),
            _ => self.read_number(node, "power_now").map(normalize_power_mw),
        };
        if let Some(power) = power {
            out.insert("battery.power".to_string(), json!(power));
        }

        if let Some(health) = self.read_text(node, "health") {
            out.insert("battery.health".to_string(), json!(health));
        }
        let full = self
            .read_number(node, "charge_full")
            .map(normalize_charge_mah);
        let design = self
            .read_number(node, "charge_full_design")
            .map(normalize_charge_mah);
        if let Some(full) = full {
            out.insert("battery.charge_full".to_string(), json!(full));
        }
        if let Some(design) = design {
            out.insert("battery.charge_full_design".to_string(), json!(design));
        }
        // Vendor SOH first: a capacity ratio reads 100 % whenever `charge_full` mirrors the design
        // capacity, which is exactly what Samsung reports.
        if let Some(soh) = read_soh(&self.fs, node) {
            out.insert("battery.health_percent".to_string(), json!(soh));
        } else if let (Some(full), Some(design)) = (full, design) {
            if full > 0 && design > 0 {
                let percent = (full * 100 / design).clamp(0, 100);
                out.insert("battery.health_percent".to_string(), json!(percent));
            }
        }
        if let Some(cycles) = read_cycle_count(&self.fs, node) {
            out.insert("battery.cycle_count".to_string(), json!(cycles.value));
        }
        if let Some(ac) = self.online(AC_NODES) {
            out.insert("battery.ac_online".to_string(), json!(ac));
        }
        if let Some(usb) = self.online(USB_NODES) {
            out.insert("battery.usb_online".to_string(), json!(usb));
        }
        if let Some(wireless) = self.online(WIRELESS_NODES) {
            out.insert("battery.wireless_online".to_string(), json!(wireless));
        }
        // Coulomb counter: the real charge remaining, unlike `capacity% × charge_full`.
        if let Some(charge) = self
            .read_number(node, "charge_counter")
            .map(normalize_charge_mah)
        {
            if charge > 0 {
                out.insert("battery.charge_mah".to_string(), json!(charge));
            }
        }
        if let Some(usb_type) = self.charger_type() {
            out.insert("battery.usb_type".to_string(), json!(usb_type));
        }
        if let Some(limit) = self.input_current_limit() {
            out.insert("battery.input_current_limit".to_string(), json!(limit));
        }
        if let Some(charging) = out.get("battery.charging").and_then(Value::as_bool) {
            // Same heuristic Android's own battery UI uses: wattage or current above the USB 2.0 line.
            let fast = charging
                && (power.map(|p| p.abs() >= FAST_CHARGE_MW).unwrap_or(false)
                    || current.map(|c| c.abs() >= FAST_CHARGE_MA).unwrap_or(false));
            out.insert("battery.fast_charging".to_string(), json!(fast));
        }

        let filtered = match keys {
            None => out,
            Some(keys) => {
                let mut m = Map::new();
                for key in keys {
                    if let Some(value) = out.get(key) {
                        m.insert(key.clone(), value.clone());
                    }
                }
                m
            }
        };
        Ok(Value::Object(filtered))
    }

    /// History sampling every 60 s, matching the phase 2 plan. Metrics are the ones a battery chart
    /// needs; `charging` is stored as 0/1 so a chart can shade charging windows.
    fn sample(&self) -> Option<sysflow_core::Result<Sample>> {
        let node = self.node()?;
        let mut sample = Map::new();
        if let Some(capacity) = self.read_number(node, "capacity") {
            sample.insert("level".into(), json!(capacity));
        }
        if let Some(temp) = self.read_number(node, "temp") {
            sample.insert("temp".into(), json!(temp));
        }
        if let Some(mv) = self
            .read_number(node, "voltage_now")
            .map(normalize_voltage_mv)
        {
            sample.insert("voltage_mv".into(), json!(mv));
        }
        if let Some(ma) = self
            .read_number(node, "current_now")
            .map(normalize_current_ma)
        {
            sample.insert("current_ma".into(), json!(ma));
        }
        if let (Some(mv), Some(ma)) = (
            self.read_number(node, "voltage_now")
                .map(normalize_voltage_mv),
            self.read_number(node, "current_now")
                .map(normalize_current_ma),
        ) {
            sample.insert(
                "power_mw".into(),
                json!((mv as f64 * ma as f64 / 1000.0).round() as i64),
            );
        }
        if let Some(status) = self.read_text(node, "status") {
            sample.insert(
                "charging".into(),
                json!(i64::from(is_charging_status(&status))),
            );
        }
        if sample.is_empty() {
            return None;
        }
        Some(Ok(sample))
    }

    fn sample_interval(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
    }

    /// Every item is `Readonly`, so `schema().validate_request()` rejects a `set` before it can
    /// reach this point. Failing loudly keeps that contract obvious if a writable item is ever
    /// added without adding the corresponding write path here.
    fn apply(&self, _req: &Map<String, Value>) -> Result<ApplyResult> {
        Err(Error::invalid_arg(
            "battery module exposes no writable items",
        ))
    }
}

#[cfg(test)]
mod tests;
