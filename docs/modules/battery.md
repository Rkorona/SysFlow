# Battery module: paths, items and device notes

Implementation: `core/sysflow-modules/src/battery/mod.rs`. Protocol: `docs/protocol.md` §3.15.

This module is **read-only by design**. It exposes no tunable, writes no node, and therefore takes
no part in `apply`, drift repair, boot restore or the safe-mode recovery path — the first non-CPU
module was chosen precisely because it cannot brick a device.

Everything below is probed; a node that does not exist removes its item from `schema` and `caps`.
The module never creates sysfs nodes.

## Paths

| feature | paths |
|---|---|
| battery node | first readable of `/sys/class/power_supply/{battery,Battery,bms}` |
| state of charge | `<node>/capacity` |
| charge state | `<node>/status` (`Charging`, `Discharging`, `Full`, `Not charging`, `Unknown`) |
| temperature | `<node>/temp` (0.1 °C) |
| voltage | `<node>/voltage_now` |
| current | `<node>/current_now` |
| power | computed from voltage × current, else `<node>/power_now` |
| health string | `<node>/health` |
| capacity design | `<node>/charge_full`, `<node>/charge_full_design` |
| cycle count | `<node>/cycle_count`, `battery_cycle`, `batt_cycle`, `batt_soc_cycle`, `fg_cycle`, `cycle`; then `POWER_SUPPLY_CYCLE_COUNT` in `<node>/uevent` |
| cycle count (vendor) | `dumpsys batteryproperties` → `mSavedBatteryUsage` ÷ 100 (Samsung). **Removed on recent One UI builds — the command returns nothing there** |
| cycle count (HAL) | `dumpsys health` → a labelled `cycle count` (Android 14+) |
| health (learned) | `dumpsys batterystats` → `Estimated battery capacity` ÷ design capacity |
| charger state | `/sys/class/power_supply/{ac,AC}/online`, `/sys/class/power_supply/{usb,USB}/online` |
| wireless charging | `/sys/class/power_supply/{wireless,wireless_charger,wc,dc}/online` |
| charge remaining | `<node>/charge_counter` (coulomb counter) |
| charger mode | `/sys/class/power_supply/usb/type` |
| input limit | `usb/{input_current_limit,current_max,input_current_max}` |

## Units

Kernels disagree per vendor, so the module normalises by magnitude and the protocol only ever
carries one unit. Clients must not re-scale.

| reading | kernel conventions | threshold | protocol |
|---|---|---|---|
| `voltage_now` | mV (`4200`) or µV (`4200000`) | > 100 000 → µV | `mV` |
| `current_now` | mA or µA | > 10 000 → µA | `mA` (sign preserved) |
| `charge_full*` | mAh or µAh | > 100 000 → µAh | `mAh` |
| `power_now` | mW or µW | > 1 000 000 → µW | `mW` |
| `temp` | tenths of °C | — | `0.1 °C`, same as `cpu.battery_temp` |

`battery.power` is computed as `voltage × current / 1000` when both readings exist, because
`power_now` is missing or plainly wrong on a number of ROMs. `power_now` is only the fallback.

`battery.current` keeps the sign the kernel reports: some platforms use negative for discharge,
others positive. A UI that shows "charging power" should use `battery.charging` plus `|power|`
rather than inferring direction from the sign.

## Device differences worth probing

- **Node name**: `battery`, `Battery` or `bms`. A file named `bms` may expose only `status` and
  `capacity`; the module then simply has fewer items.
- **Missing fields are omitted, not `null`.** A device without a temperature sensor has no
  `battery.temp` key at all; render a missing item as "not available" rather than zero.
- **`charge_full` above `charge_full_design`**: replacement cells and miscalibrated gauges can
  report more than the design capacity. `battery.health_percent` is clamped to 0–100.
- **`cycle_count`** is absent on most mid-range kernels; do not treat it as a health signal by
  default.
- **Multiple batteries** (some tablets expose `battery` and `battery2`): only the first matching
  node is reported today.

## Rollback and safety

None needed: the module cannot write. It has no persisted settings, no default snapshot entries and
no transient state file, so `reset` is a no-op and safe mode needs no special handling.

Deliberately **not** exposed (each vendor-specific and high risk, to be added behind an explicit
experimental switch if ever):

- `charge_control_limit`, `charge_control_end_threshold` — charge limiting
- `input_suspend`, `charge_disable` — charge path switching
- `batt_protect` / `battery_protect` — vendor charge protection

## Host test tree

`core/sysflow-modules/src/battery/tests.rs` builds a fake `power_supply` tree under a temp directory
and drives `probe`/`schema`/`collect` against it: unit normalisation both ways, derived power and
health, absent fields, the `bms` fallback, the read-only contract, and the rejection of `apply`.

To exercise the real CLI without a device:

```sh
mkdir -p /tmp/sf/sys/class/power_supply/battery
printf '83\n'  > /tmp/sf/sys/class/power_supply/battery/capacity
printf 'Charging\n' > /tmp/sf/sys/class/power_supply/battery/status
printf '4200000\n'  > /tmp/sf/sys/class/power_supply/battery/voltage_now
SYSFLOW_SYSFS_ROOT=/tmp/sf SYSFLOW_DATA_DIR=/tmp/sf-data sysflow battery get --pretty
```

## Page (APK)

`app/.../feature/home/BatteryScreen.kt` renders this page; the overview `BATTERY` card links here
unconditionally, so with no battery data the page explains why instead of staying unreachable. The
module WebUI is a rescue console only and has no battery page.

| block | content |
|---|---|
| hero | charge %, status pill, wavy meter, charger, remaining mAh, estimated time to full / to empty; turns error-toned at ≤15 % off the charger |
| live readings | voltage, current, power, temperature as a connected 2×2 tile grid, each with a session sparkline (temperature tile turns amber at 45 °C, red at 50 °C) |
| 24 小时电量 | daemon history area curve with charging bands, drain since the last charge, and the screen-on / screen-off drain split |
| 应用耗电排行 | top-3 non-system apps as connected tiles plus a「查看完整排行」tile that opens the ranking page (§3.16); no sheet, no modal |
| 健康度 | tertiary hero tile with a 270° health ring, cycle pill and data sources; capacities and capacity lost in a segmented list |
| notice | read-only statement |

**Ranking page** (`BatteryRankingScreen`, a sub-page of the battery page): summary card with the
estimated total and a stacked top-3 share bar, an 全部 / 仅应用 segmented filter plus refresh, then one
connected list of every UID. Row bars are scaled to the largest visible share; the printed
percentage is the true share. Rank badge colours match the stacked bar.

**The 2-minute sparkline is session-only.** Points are appended from the periodic snapshot refresh
(one per ~2 s, 60 points ≈ 2 minutes) into memory and disappear when the page closes. That block is
deliberately ephemeral. The durable curve is the separate "24 小时电量" block, which reads
`sysflow history battery --since 24h --step 1h` from the daemon-written JSONL (§3.9) and shows a
"daemon not running" hint instead of inventing points when the series stops growing.

**Derived values are estimates, not measurements.** Time-to-full/empty divides `charge_full ×
capacity%` by `|current|`; `current` is an instantaneous reading, so the UI labels the result
"粗估". Direction comes from `battery.charging`, never from the sign of `battery.current`, because
kernels disagree about which sign means discharge.

## Per-app power ranking

`sysflow battery apps` (protocol §3.16) parses BatteryStats' per-UID power attribution. The parser
lives in `core/sysflow-core/src/batterystats.rs` — **not** in the module — because the deferred
wake-lock module and the process module need the same dump.

Three constraints shaped it:

- **UID granularity, not process.** BatteryStats folds every process of an app into one row and
  groups daemons under shared system UIDs (uid 0 is kernel work).
- **On demand only.** A fresh dump costs hundreds of milliseconds and prints megabytes, so the verb
  streams just the power section and kills the child once it ends. It is never part of `snapshot`.
- **Shares are trustworthy, absolute mAh is not.** Per-component values sum exactly to the printed
  globals, so relative ranking is sound; but on ROMs without a usable power profile the per-component
  currents are wrong (one device reported ~2 % for six days of screen-on). The UI therefore leads
  with the share and labels the block as an estimate.

The APK resolves localised app labels and icons through `PackageManager`; the WebUI shows the package
name the CLI supplies. Results are cached for 60 s in `<data_dir>/state/battery_apps.json`.

## Cycle count and health fallbacks

Most kernels expose no cycle counter at all, so `snapshot` reports nothing and the UI omits the row.
The `sysflow battery health` verb (protocol §3.17) adds two fallbacks that a 2 s snapshot must not
pay for:

1. **`uevent`** — most power-supply drivers publish `POWER_SUPPLY_CYCLE_COUNT` there even when the
   per-attribute file is unreadable. Cheap, so the module also does this inside `collect()`.
2. **Samsung `mSavedBatteryUsage`** — the counter lives only in `dumpsys batteryproperties`, in
   hundredths of a cycle. ~100 ms, so it is an on-demand verb with a 60 s cache.

Vendor node names were collected from the field (Samsung, Xiaomi, Qualcomm FG, MTK). `soh` and
`count` are deliberately excluded from the cycle candidates: `soh` is health, `count` is unrelated,
and reading either as cycles would invent data. Placeholder values (`0`, `>9999`) are rejected.

The UI shows the source next to the value, so "681 次 · Samsung 电池 EFS" makes clear where a number
came from instead of implying every device has a clean sysfs register.

**Health follows the same rule: vendor calibration wins.** `POWER_SUPPLY_SOH` (or Samsung
`mSavedBatteryBsoh`) is reported as-is; only when no source publishes one does the module derive
`charge_full ÷ charge_full_design`. The reason is concrete: on Samsung, `charge_full` equals the
design capacity, so the derived ratio is a constant 100 % while the calibrated health reads 99 %.
A derived number must never override a measured one.

## Events

`module/service.sh` runs a `battery_loop` (60 s cadence, independent of the tuning daemon) and emits
transitions with dedupe keys: `charging_started`, `charging_stopped`, `fully_charged`, `low_battery`
(≤15 %, once per discharge) and `battery_temp_high` (≥45 °C, re-arms below 43 °C). The module is the
producer; clients never infer these from dashboard snapshots.

## History

The module implements `Module::sample()` (level, temp, voltage, current, power, charging) with a
60 s interval, which the enforcement daemon records into `<data_dir>/history/battery.jsonl`. The
daemon adds the display state it observed at sample time as a top-level `"screen"` field (`1`
interactive, `0` not), which `sysflow history` exposes as a `screen` series. That context is what
makes the on/off drain split possible: it is recorded by the module's owner, never inferred by the
client from its own visibility.

**A module upgrade does not restart the daemon.** The running process keeps the replaced binary's
inode, so newly added sampling code is absent until the device reboots — this wasted a debugging
session once. Two safeguards now exist: `customize.sh` stops the old daemon during install, and
`daemon start` compares `/proc/<pid>/exe` with the on-disk binary and restarts it when they differ
(the log line `runs a replaced binary; restarting it` marks the restart). See
protocol §3.9 for retention and downsampling, and the APK battery page for the 24 h curve, the
"drain since last charge" statistic and the screen-on/screen-off drain split computed from it.

## Not implemented yet

- **Charge-limiting tunables** — deliberately excluded (see above).
- **Drain split before the upgrade** — records written before the daemon tracked display state carry
  no `screen` field, so the split stays empty until a few hours of new samples exist. The page says
  so instead of showing a misleading zero.
