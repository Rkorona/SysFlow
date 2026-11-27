# CPU module: paths, items and device notes

Implementation: `core/sysflow-modules/src/cpu/` (`mod.rs`, `walt.rs`, `gpu.rs`, `mem.rs`,
`cpuset.rs`, `adaptive.rs`, `state.rs`, `schema.rs`, `screen.rs`, `tests.rs`). Protocol:
`docs/protocol.md` §3.11 (item list) and §3.13 (daemon / display transitions). Decision record:
`docs/development-plan.md` §3, §7.1 and "当前进度" — this tree has no `docs/adr/` directory.

Everything below is probed; a node that does not exist removes its item from `schema` and `caps`.
The module never creates sysfs nodes.

## Paths

| feature | paths |
|---|---|
| cores | `/sys/devices/system/cpu/cpuN/online` (cpu0 usually has no node) |
| clusters | `/sys/devices/system/cpu/cpufreq/policyN/{related_cpus,affected_cpus,scaling_*}` |
| hardware range | `policyN/cpuinfo_{min,max}_freq` |
| frequency table | `policyN/scaling_available_frequencies`, else `policyN/stats/time_in_state` |
| time in state | `policyN/stats/time_in_state` |
| load | `/proc/stat` (`cpuN ...` fields, exposed per core and as an aggregate for UI deltas; also aggregated per cluster for adaptive control) |
| CPU temperature | `/sys/class/thermal/thermal_zone*/{type,temp}`; CPU `cpu.temp` filters CPU-labelled zones (not `trip`) |
| adaptive temperatures | same thermal zones; CPU/GPU/SoC types feed control; `cpu.thermal_sensors_all` returns all valid zones |
| cooling diagnostics | `/sys/class/thermal/cooling_device*/{type,cur_state,max_state}` (read-only) |
| GPU busy | KGSL `gpu_busy_percentage` / `gpu_busy`, or `/sys/kernel/gpu/gpu_busy_percentage` |
| battery temperature | `/sys/class/power_supply/battery/temp` (0.1 °C) |
| WALT | `policyN/walt/{up_rate_limit_us,...,adaptive_high_freq,adaptive_low_freq}` |
| GPU | `/sys/class/kgsl/kgsl-3d0/devfreq`, else a matching `/sys/class/devfreq/*{kgsl,mali,g3d,gpu}*` |
| GPU extras | `/sys/class/kgsl/kgsl-3d0/dcvs_boost`, `kgsl-busmon/devfreq/max_freq` |
| DDR / LLCC | `/sys/devices/system/cpu/bus_dcvs/{DDR,LLCC}/{boost_freq,available_frequencies}` |
| cpuset | `/dev/cpuset/cpus` and `/dev/cpuset/<group>/cpus` |
| screen state | `/sys/class/backlight/*/brightness`, confirmed by `dumpsys power`; polled by the enforcement daemon (`core/sysflow-core/src/screen.rs`) |
| adaptive inputs | `/proc/stat`, `/sys/class/thermal`, `/sys/class/power_supply/battery`, `/proc/pressure/cpu` |
| daemon | `/data/adb/sysflow/guard_interval`, `no_enforce`, `daemon.pid`; default 10 s, allowed 2–120 s |

## Units

- frequency: kHz (GPU devfreq is Hz on disk, converted by `/1000`; the original Hz string is kept
  for writes)
- temperature: protocol 0.1 °C for `cpu.temp`, battery and thermal-sensor telemetry; the adaptive controller uses whole °C internally
- WALT values: raw kernel integers / the `target_loads` string
- time in state: kernel ticks as reported

`cpu.time_in_state` and `cpu.usage_stat` are cumulative counters, not percentages. The client must retain the previous snapshot: difference the time-in-state ticks to get the weighted
policy frequency, and difference `busy` / `total` to get the per-core utilization. A first sample
only establishes the baseline. The module-owned daemon advances the adaptive load / thermal /
battery / PSI hysteresis on its configured interval; it does not depend on the APK being alive.

## Device differences worth probing

- **Read-only cpufreq nodes** (Samsung and some vendor kernels): `Sysfs::write_str` retries once
  after relaxing the mode to `0644` and restores the old mode.
- **WALT presence/keys**: only some MediaTek/Qualcomm kernels expose `policyN/walt`; each key is
  probed separately. `adaptive_high_freq` / `adaptive_low_freq` are forced to 0 while the cluster's
  max is below `cpuinfo_max_freq`.
- **WALT guard**: some ROM tuning services rewrite WALT nodes; `cpu.walt_guard` sets the guarded
  nodes to `0444`. Re-apply after every reboot (the mode is not persistent in the kernel).
- **GPU devfreq names**: Adreno uses kgsl; Mali/G3D names vary. Nodes matching `bw`, `bus`, `llcc`,
  `ddr` or `cache` are excluded so a bus/cache devfreq is not mistaken for the GPU.
- **cpuset root**: Android's `/dev/cpuset` is used. A cpuset write naming an offline core fails, so
  the requested set is intersected with `possible` and the currently online cores.
- **DDR/LLCC** `bus_dcvs` is chipset specific; the item disappears when the node is missing.
- **Screen-off and stale-detection fallback**: `dumpsys power` formats differ (`mIsInteractive` vs
  `mWakefulness`); an ordinary `unknown` output is never a transition — the last decision is kept and
  the next tick retries. The profile saves and restores the pre-off range and cores. One deliberate
  exception is a *stale cached `Off`*: when no successful framework answer has arrived for at least
  30 s (3 × the 10 s `FRAMEWORK_RECHECK`), the detector switches to the safe direction `On` and
  releases the profile regardless of whether a backlight node exists — a non-zero backlight alone is
  not proof of "on", because some ROMs keep it lit for the always-on display while the framework is
  asleep. A backlight change only triggers an earlier framework re-check; a successful framework
  answer of `on` releases the profile immediately. The release writes a `warn` log and emits a
  `screen_detection_fallback` event (category `power`, warning, dedupe key
  `display.screen_detection_fallback`, 300 s), with `reason: framework_unknown_timeout` and
  `unknown_age_s` in metadata. A genuine `Off` still needs two consecutive framework answers before
  the profile is re-applied; ordinary `unknown` is never a transition.
- **Thermal**: millidegree and whole-degree zones are both accepted. CPU/GPU/SoC-labelled zones
  other than `trip` feed adaptive control; `cpu.thermal_sensors_all` and `cpu.cooling_devices`
  expose the broader read-only diagnostic view.

## Rollback and safety

- `min <= max` is validated before writes; the kernel never sees `min > max` because the write order
  lowers the floor first when the new ceiling is below the current floor.
- Cluster writes are restored if a later cluster fails; online changes are restored too.
- Downstream failures (WALT, GPU, DDR/LLCC, cpuset) roll the cluster plan back as well.
- Default snapshot records the hardware range for CPU/GPU min/max, not a transient boost/thermal cap.
- `apply-boot` drops stale persisted entries one by one (`sanitize_boot`) instead of skipping the
  whole module.
- `sysflow check` is read-only and is the daemon prerequisite; it allows a dynamically lowered max
  while adaptive/screen-off is active but reports a max above the persisted ceiling.

## Host test tree

`core/sysflow-modules/src/cpu/tests.rs` builds a fake tree with 8 cores, two policies, thermal
zones, WALT nodes, a GPU devfreq + busmon node, DDR/LLCC, cpuset groups, `/proc/stat`,
`/proc/pressure/cpu` and a battery. `core/sysflow-modules/tests/cpu_flow.rs` covers the registry
rules (persist/reset/boot/check) against a smaller tree.

## Drift semantics

`drift()` is the read-only check the enforcement daemon runs each tick; anything it returns is
re-applied. Two rules matter and are easy to get wrong:

- **`max_freq` and `min_freq` compare in opposite directions** while adaptive control or the
  screen-off profile is active. A live ceiling *below* the request is fine (thermal, adaptive), and a
  live floor *above* the request is equally fine — the kernel's own floor cannot be written past. A
  kernel that enforces `min_freq = 1000000` over a persisted `500000` used to be reported as drift
  every tick, so the daemon re-applied and logged a mismatch forever. See
  `a_kernel_min_freq_floor_is_not_reported_as_drift`.
- **`cpu.cpuset.*` uses an "effective" comparison**: a core that is offline cannot join a cpuset, so
  the request is satisfied as long as every core that *could* be online matches.

The daemon additionally re-runs `drift()` after applying and only reports mismatch keys that the
check still flags. That keeps expected lag (offline cores, kernel floors) out of the log and out of
the `drift_repair_incomplete` warning event.

## Defaults, the hardware floor, and `cpu rebaseline`

`<module> reset` restores the *default snapshot* (`config.json → defaults_snapshot`), not "the value
the device booted with". That snapshot is written once, the first time an item is written, and is
never refreshed; `reset` then drops it, so the next write re-records it. `cpu`'s
`normalize_default()` pins the recorded `min_freq`/`max_freq` to `cpuinfo_min_freq` /
`cpuinfo_max_freq` at record time.

That leaves one gap. A config whose baseline was recorded before a kernel/ROM update, or against a
driver that reported a different floor, keeps restoring a minimum frequency the hardware no longer
honours — and no client can correct it short of deleting `config.json`.

`sysflow cpu rebaseline` closes it: it reads `cpu.hw_min_freq` (per-cluster `cpuinfo_min_freq`) and
stores that as the `cpu.min_freq` default, merging per cluster so untouched clusters keep their
recorded default. It changes **only defaults** — live frequencies are not written. Read-only items
are rejected, and clusters whose node is unreadable (or reports `0`) are skipped rather than seeded
with a bogus floor.

```console
$ sysflow cpu rebaseline
{"ok":true,"data":{"module":"cpu","defaults":{"cpu.min_freq":{"0":300000,"2":500000}}}}
```

The APK exposes it as *写回硬件下限为默认* in the core-matrix overflow menu.

## Core switches are immediate

`cpu.online` is a hardware switch, not a tuning value: the APK writes it with `--persist` as soon as
a core tile is tapped (`CpuDraft` still holds it, but only to render the optimistic position). There
is deliberately no "apply" step for core switches — an earlier build kept the mask only in the draft,
while `applyCluster` sends a single cluster's frequency/governor keys and never the core mask, so a
toggled core was silently dropped on the next poll. The module rejects an all-off mask
(`cpu.online: at least one core must stay online`), and the APK blocks the last core client-side so
the tile never flips into a state the device will refuse.
