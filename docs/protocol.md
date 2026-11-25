# SysFlow protocol v1

The module binary `sysflow` is the only writer of system state and config. The APK and
the WebUI only call this CLI and parse its JSON.

Binary path on device: `/data/adb/modules/sysflow/bin/sysflow`
Data directory: `/data/adb/sysflow/` (`config.json`, `log/`, `events/`, and module state). The Root module owns the event journal; the APK keeps a Room mirror for fast local display/query only.

## 1. Conventions

- Output is one JSON document on stdout. `--pretty` formats it for humans (global flag).
- Exit code: `0` ok, `1` the command failed (`ok:false`), `2` bad command line (also `ok:false`).
- Units are fixed by the protocol; modules normalise sysfs units:

| quantity | unit |
|---|---|
| frequency | kHz |
| temperature | 0.1 °C (integer, `420` = 42.0 °C) |
| voltage / current / power | mV / mA / mW |
| memory / storage sizes | kB |
| time | Unix seconds (timestamps), seconds (durations) |

- Item ids are `<module>.<name>` (`cpu.online`). Values for `bool_list` are arrays of `0`/`1`.
- Unknown JSON fields must be ignored by clients; new fields are only ever added.

## 2. Envelope

```json
{"v":1,"ok":true,"data":{}}
```

```json
{"v":1,"ok":false,"error":{"code":"unsupported","msg":"…"}}
```

`error.detail` (optional object) may carry extra context.

| code | meaning | UI behaviour |
|---|---|---|
| `unsupported` | device/kernel does not offer this | hide the feature |
| `permission` | no root / SELinux denied | guide to root / module status |
| `invalid_arg` | bad module, key or value | revert the control, show message |
| `busy` | resource or config lock in use | retry shortly |
| `io` | read/write failed | show message, link to log |
| `safe_mode` | tuning disabled by safe mode | read-only banner + "exit safe mode" |

## 3. Commands

```text
sysflow version
sysflow caps
sysflow snapshot
sysflow check
sysflow info
sysflow <module> schema
sysflow <module> get [--keys a,b]
sysflow <module> set '<json>' [--persist]
sysflow <module> reset
sysflow cpu rebaseline
sysflow reset --all
sysflow history <module> [--since 24h] [--step 5m]
sysflow battery apps [--limit N] [--refresh]
sysflow battery health [--refresh]
sysflow wakelock held [--refresh]
sysflow wakelock apps [--limit N] [--refresh]
sysflow wakelock wakeup [--refresh]
sysflow mem apps [--limit N] [--refresh]
sysflow mem breakdown [--refresh]
sysflow mem probe
sysflow apply-boot [--no-delay]
sysflow boot-confirm
sysflow boot status | configure [--enabled true|false] [--delay <seconds>]
sysflow safe-mode status | clear
sysflow recovery status | restore | clear | clear-corrupt
sysflow daemon start | stop | status | configure [--enabled true|false] [--interval <2..120>]
sysflow daemon run
sysflow enforce
sysflow note <message>
sysflow event emit --module <id> --category <id> --type <id> --title <text> [--severity info|notice|warning|error|critical] [--message <text>] [--metadata <json>] [--dedupe-key <id>]
sysflow event list [--limit 100] [--after-ms <timestamp> --after-id <id>] [--before-ms <timestamp> --before-id <id>] [--ascending] [--module <id>] [--category <id>] [--severity <level>]
sysflow event prune [--retention-days 30] [--max-events 5000]
sysflow event clear --all
sysflow log [--tail 200]
```

`enforce` and `daemon run` are hidden commands (they are the daemon's own entry points, not user
operations), and `mem probe` is a hidden developer diagnostic whose output may change: it prints the
raw readings of every memory source plus the module's own include/hide decision, and is not part of
the stable contract.

The per-module *verbs* above (`battery apps|health`, `wakelock held|apps|wakeup`,
`mem apps|breakdown|probe`) are not schema items. Each one crosses into the Java system server or
walks `/proc`, so it is an explicit on-demand call with its own 60 s cache rather than something a
client can put in a 2 s snapshot poll.

### 3.1 version

```json
{"v":1,"ok":true,"data":{"sysflow_version":"0.2.1","protocol":1,"config_version":1}}
```

`sysflow_version` is the crate version from `core/Cargo.toml`; `module/module.prop` carries the same
version for the module manager.

### 3.2 info

`sysflow info` returns static platform/build properties used by the diagnostics screens. Missing
Android properties are empty strings; `kernel_release` is read from `uname -r`.

```json
{"soc_manufacturer":"Qualcomm","soc_model":"...","board_platform":"...",
 "hardware":"...","kernel_release":"...","android_release":"17",
 "device_model":"...","device_codename":"...","product_device":"...",
 "primary_abi":"arm64-v8a"}
```

### 3.3 caps

```json
{"v":1,"ok":true,"data":{
  "sysflow_version":"0.2.1","protocol":1,"safe_mode":false,
  "modules":{
    "cpu":{"supported":true,"features":["online","max_freq","min_freq","cur_freq","clusters",
           "time_in_state","load","governor","hw_range","gpu","walt","walt_guard","mem_boost",
           "screen_off","adaptive"]},
    "battery":{"supported":true,"features":["capacity","status","temp","voltage_now","current_now",
               "power_now","health","charge_full","charge_full_design","ac_online","usb_online"]},
    "wakelock":{"supported":true,"features":["kernel_sources","userspace_held","history","block"]},
    "mem":{"supported":true,"features":["meminfo","swap","swaps","psi","psi_full","zram",
           "zram_mm_stat","vmstat"]}
  }
}}
```

`features` are free-form strings owned by each module, not a protocol enum: clients must treat an
unknown feature as "advertised by a newer module" and ignore it. The battery strings are the present
sysfs fields, so they double as "which readings this device can answer". `cpuset` is not a `caps`
feature — the cpuset items appear in `cpu schema` only when `/dev/cpuset` exists.

An unsupported module reports `"supported":false,"features":[],"reason":"…"`. The wakelock module
stays supported without debugfs (blocking does not need it), only losing `kernel_sources`,
`userspace_held` and `history`.

The APK calls `caps` first, checks `protocol`, then `snapshot`.

### 3.4 snapshot

One call for every supported module that opts into snapshots (`Module::in_snapshot()`; every current
module returns `true`, and a future module whose collection is expensive can opt out).

```json
{"v":1,"ok":true,"data":{
  "ts":1767000000,"safe_mode":false,
  "modules":{"cpu":{"cpu.online":[1,1,1,1,0,0,1,1]}},
  "settings":{"cpu":{"cpu.max_freq":{"4":1800000}}},
  "defaults":{"cpu":{"cpu.max_freq":{"4":2208000}}},
  "errors":{"battery":{"code":"io","msg":"…"}}
}}
```

- `modules` is the live read-back (`get`), `settings` is the persisted intent from `config.json`,
  `defaults` is the first-write default snapshot. Keeping them apart is what lets a client show
  "静态上限 1.98 GHz / 当前生效 1.80 GHz（动态封顶）" without a second call.
- A module that fails to collect is reported in `errors`; the others are still returned, and an
  unsupported module appears in neither.

### 3.5 schema

```json
{
  "module":"cpu","supported":true,
  "items":[
    {"id":"cpu.online","type":"bool_list","scope":"global","risk":"medium",
     "persist":true,"count":8,"locked":[0]},
    {"id":"cpu.max_freq","type":"int_enum","scope":"cluster","risk":"low",
     "persist":true,"options":[1804800,2208000,2419200],"unit":"kHz"}
  ]
}
```

For an unsupported module: `"supported":false,"items":[]` plus `"reason"`.

Item fields: `id`, `type`, `scope` (`global`|`cluster`|`cpu`|`app`), `risk`
(`low`|`medium`|`high`), `persist`, and optionally `count`, `locked`, `options`,
`min`/`max`/`step`, `max_len`, `unit`, `depends_on`, `title_key`.

| type | extra fields | value |
|---|---|---|
| `bool` | | `true`/`false` |
| `bool_list` | `count`, `locked[]` | array of 0/1 |
| `int_range` | `min`,`max`,`step`,`unit` | integer, `(v-min) % step == 0` |
| `int_enum` | `options[]`,`unit` | one of `options` |
| `enum` | `options[]` | string, one of `options` |
| `text` | `max_len` (optional) | non-empty printable string |
| `readonly` | `unit` | cannot be set |
| `action` | | trigger with `true` |

For `scope` other than `global` the value is an object keyed by cluster/cpu/app id:
`{"0":1804800,"4":2419200}`. `risk` tells the UI whether to confirm: `low` direct,
`medium` first-time notice, `high` confirm every time (and hidden unless experimental
features are on).

### 3.6 get

Returns a flat object of item id to value (readonly items included).

```json
{"v":1,"ok":true,"data":{"cpu.online":[1,1,1,1,0,0,1,1],"cpu.max_freq":{"0":1804800,"4":2419200}}}
```

### 3.7 set

The payload is a JSON object, a subset of the items. Every key is validated against the
schema (unknown key, readonly item, wrong type or out-of-range value → `invalid_arg`)
before anything is written. The reply is the **read-back** state, not an echo:

```json
{"v":1,"ok":true,"data":{
  "applied":{"mock.level":95},
  "requested":{"mock.level":100},
  "mismatch":["mock.level"]
}}
```

- `mismatch` lists items whose read-back does not satisfy the request (for object values the
  request must be a subset of the read-back). When dynamic capping (adaptive governor or screen-off)
  is active and live `scaling_max_freq` reflects the planned cap within the user's static ceiling,
  it is not reported as a `cpu.max_freq` mismatch. Clients format mismatch entries with detailed
  requested vs. readback values (e.g. `cpu.max_freq[6] 请求 1.98 GHz，读回 1.80 GHz`).
- When a requested cluster or policy does not exist, the module explicitly rejects the request with
  `invalid_arg: policy{id} does not exist` rather than silently skipping it.
- `--persist` stores each **requested** item in `config.json` after its read-back has been verified.
  Items in `mismatch` are normally excluded, except items where `persist_on_mismatch` is true
  (such as `cpu.max_freq`, `cpu.min_freq`, and `cpu.cpuset.*`), ensuring user static limits
  remain persisted even under temporary live capping or kernel drift. `apply-boot` re-applies
  the saved requests at boot.
- Before the first change to an item, its current value is saved as the default snapshot.
- `set` fails with `safe_mode` while safe mode is on.

### 3.8 reset

`sysflow <module> reset` restores the default snapshot through the module (allowed in safe
mode), then drops the module's persisted settings.

```json
{"v":1,"ok":true,"data":{"restored":{"mock.level":50},"mismatch":[]}}
```

If some item could not be restored, it is listed in `mismatch` and the snapshot is kept so a
later reset can retry. A fully successful reset drops the snapshot, so the next write re-records it
from the device. `sysflow reset --all` returns `{"reset":{…},"errors":{…}}`.

### 3.8.1 cpu rebaseline

`sysflow cpu rebaseline` re-seeds the stored defaults for `cpu.min_freq` from the kernel's own
per-cluster hardware floor (`cpuinfo_min_freq`), which is the only baseline the device can honour
after a kernel or ROM update moved it. Only defaults change; live values are not written. The values
merge per cluster, clusters reporting an unreadable or non-positive floor are skipped, and read-only
items are rejected.

```json
{"v":1,"ok":true,"data":{"module":"cpu","defaults":{"cpu.min_freq":{"0":300000,"2":500000}}}}
```

### 3.9 history

```json
{"v":1,"ok":true,"data":{
  "module":"battery","step_s":300,"daemon_running":true,"points":2,
  "series":{"level":[[1767000000,83],[1767000300,82]]}
}}
```

Without `--step`, `step_s` is the module's own sampling interval (60 s for `battery`, 300 s for
`wakelock` and `mem`).

Timestamps are Unix seconds; missing points are `null`. `--since`/`--step` accept `90s`, `5m`,
`24h`, `7d`.

The enforcement daemon records one sample per module interval (`battery` every 60 s, `wakelock`
and `mem` every 300 s) into `<data_dir>/history/<module>.jsonl`, one JSON object per line:
`{"ts":<secs>,"v":{...}}`. Retention
is 30 days / 20 000 lines, applied on append; a truncated line from a power loss is dropped rather
than poisoning the file. Timestamps more than 5 minutes in the future are rejected so a clock jump
cannot pin every "latest value" query.

Each line may also carry the display state observed at sample time as a top-level `"screen"` field
(`1` interactive, `0` not; omitted when the daemon could not determine it). It is sampling context,
not a module metric, and `history` surfaces it as an extra `screen` series alongside the metrics so a
client can split battery drain by display state without a second query.

Downsampling keeps the **newest** sample of each `--step` bucket rather than averaging, so peaks
survive. `points` reports how many buckets were returned and `daemon_running` reports whether the
daemon that writes them is alive (a stopped daemon simply means the series stops growing).


### 3.10 Structured events (open-world protocol)

The event journal is owned by the Root module at `/data/adb/sysflow/events/`. Each event is an
immutable JSON record written atomically as its own file so concurrent producers (module daemon,
service shell, CLI and future modules) do not contend on a shared append-only file. The default
retention is 30 days and at most 5,000 records. `event list` returns newest first and supports
cursor-based incremental reads (`after-ms` + `after-id`) and optional open-string filters.

Both `event list` and `event clear --all` report `directory`, the events path the CLI resolved, so a
client can tell a path disagreement (APK and terminal using different data directories) apart from a
failed delete without guessing. `event clear --all` fails when a record cannot be removed rather than
reporting a partial count as success.

```json
{
  "id":"1795400000000-1234-1",
  "module":"battery",
  "category":"charging",
  "event_type":"charging_started",
  "severity":"notice",
  "title":"Charging started",
  "message":"USB power source connected",
  "timestamp_ms":1795400000000,
  "metadata":{"source":"usb","voltage_mv":9000}
}
```

`module`, `category` and `event_type` are identifiers, not enums registered by the APK. Any
current or future module may emit records through the generic `sysflow event emit` command or
through `sysflow_core::event::EventStore`; adding a module does not require changes to this
protocol, the APK model, or the Room schema. `metadata` accepts any JSON value and is preserved
verbatim by clients. Severity is one of `info`, `notice`, `warning`, `error`, or `critical`.

The module automatically emits generic `settings_applied` and `module_reset` events for persisted
module changes, plus core lifecycle/safety/daemon events. Service scripts may emit their own
records while the APK is closed. Producers should use a stable `--dedupe-key` for transition events
that might be observed repeatedly. Event emission errors are non-fatal to the system-management
operation that caused them.

The APK polls `event list` only while the dashboard is foregrounded, incrementally imports records
into `sysflow-events.db` (Room), and renders the Room `Flow`. Room is a display/query cache, never
the source of truth and never a monitor/producer. The local cache retains up to 5,000 events for
30 days; clearing the Room cache does not delete the Root module journal.

### 3.11 CPU extras (`cpu.*`)

The CPU module also owns the pieces the shell module had under one name. They are normal
schema items, so a client can hide or group them without a separate protocol:

| item | type / scope | meaning |
|---|---|---|
| `cpu.cluster_cur_freq` | readonly / cluster | `scaling_cur_freq` per policy |
| `cpu.cluster_hw_cur_freq` | readonly / cluster | hardware-reported `cpuinfo_cur_freq` per policy; fallback between time-in-state average and scaling frequency |
| `cpu.hw_min_freq` / `cpu.hw_max_freq` | readonly / cluster | advertised hardware range |
| `cpu.clusters` | readonly / global | policy → CPU-list topology |
| `cpu.cur_freq` | readonly / cpu | per-core `scaling_cur_freq` |
| `cpu.governor_table` | readonly / cluster | `scaling_available_governors` per policy |
| `cpu.freq_table` | readonly / cluster | available frequencies per policy. Clients must strictly use this per-cluster table instead of the schema options union. If missing or <2 entries, tuning for that cluster is disabled |
| `cpu.time_in_state` | readonly | `{policy: {kHz: ticks}}` from `stats/time_in_state`; a UI can difference two samples into a weighted average frequency |
| `cpu.stat` | readonly | `{policy: {busy, total}}` from `/proc/stat`, for load readings (difference two samples) |
| `cpu.usage_stat` | readonly | `{cpuN: {busy, total}, cpu: {busy, total}}` from `/proc/stat`; difference two samples for per-core and total CPU utilization |
| `cpu.temp` | readonly / global | CPU-labelled thermal zones, 0.1 °C |
| `cpu.thermal_sensors` / `cpu.thermal_sensors_all` | readonly | the CPU/GPU/SoC zones that feed adaptive control, and every valid zone |
| `cpu.cooling_devices` | readonly | `/sys/class/thermal/cooling_device*` type / cur / max state |
| `cpu.gpu_busy` | readonly | GPU busy percentage (KGSL `gpu_busy_percentage`, else `/sys/kernel/gpu`) |
| `cpu.gpu_info` | readonly | GPU devfreq detail: governor, available frequencies, current |
| `cpu.battery_temp` | readonly | battery temperature, 0.1 °C |
| `cpu.gpu_min_freq` / `cpu.gpu_max_freq` | `int_enum` global | GPU devfreq range in kHz |
| `cpu.gpu_governor` | `enum` global | GPU devfreq governor |
| `cpu.gpu_cur_freq` | readonly | current GPU frequency in kHz |
| `cpu.gpu_dcvs_boost` / `cpu.gpu_bus_boost` | `bool` global | optional KGSL extras |
| `cpu.walt.<key>` | `int_range` or `text`, cluster | the 12 WALT keys (`up_rate_limit_us`, `down_rate_limit_us`, `hispeed_freq`, `hispeed_load`, `hispeed_cond_freq`, `target_loads`, `boost`, `pl`, `rtg_boost_freq`, `adaptive_low_freq`, `adaptive_high_freq`, `adaptive_level_1`); each key is probed separately |
| `cpu.walt_guard` | `bool` global | lock the 7 guarded WALT nodes to `0444` |
| `cpu.ddr_boost_freq` / `cpu.llcc_boost_freq` | `int_enum` global | `bus_dcvs` boost frequency |
| `cpu.cpuset.<group>` | `bool_list` global | cores usable by a cpuset group |
| `cpu.screen_off` | `bool` global | enable the screen-off profile |
| `cpu.screen_off.max_freq` | `int_range` cluster (0 = no cap) | per-cluster cap while the screen is off; positive values are validated against that policy's hardware min/max and snapped down to a supported live frequency |
| `cpu.screen_off.freq_table` | readonly / cluster | raw `scaling_available_frequencies`; empty/one-entry tables tell the UI to generate SoCFlow-compatible 24-step choices |
| `cpu.screen_off.cores` | `bool_list` global | extra cores to keep offline while the screen is off |
| `cpu.screen_off_active` | `bool` global, not persisted | runtime switch; only the enforcement daemon sets it, from a confirmed display transition (see §3.13) |
| `cpu.screen_off_saved_online` | readonly / global (`count` CPUs) | pre-screen-off online baseline while the profile is active; includes currently-offline cores so the UI can still edit selections safely |
| `cpu.adaptive` | `bool` global | enable the adaptive cap controller |
| `cpu.adaptive.*` | `bool` / `int_range` global | 4 strategy switches, 20 threshold/cap/hysteresis params (see `docs/modules/cpu.md`) |
| `cpu.adaptive_state` | readonly | last controller decision: adjustments, temperature, battery, per-policy tier/load |

`set` is applied on top of the module's persisted settings (the registry deep-merges the stored
config with the request). The CPU module uses that to enforce the persisted static max while the
adaptive cap or screen-off profile is active, without storing a dynamic value in `config.json`.
`cpu.screen_off_active` is deliberately not persisted: after a reboot the module starts awake and
normal settings apply. The **enforcement daemon** owns display transitions (§3.13) — `service.sh` has
no screen watcher, because a shell loop and the daemon would fight over the same flag and only one of
them can hold the two-consecutive-answers rule.

The module keeps transient state in `<data_dir>/state/cpu.json` and serializes concurrent applies
with `<data_dir>/state/cpu.lock`. Deleting both while no CLI is running is safe.

The module-owned enforcement daemon runs independently of the APK. At the configured interval
(default 10 seconds, range 2–120), it repairs drift in persisted settings and advances the adaptive
CPU controller even when no static setting has drifted. Each round it also compares every module's
`sample_interval()` and appends an overdue `sample()` to `history/<module>.jsonl` (§3.9). Sampling is
best-effort: a failing sample only logs and never disturbs drift repair or the adaptive controller.

### 3.12 check

Read-only drift check for the enforcement loop. It compares every persisted module setting with the
live read-back and writes nothing:

```json
{"v":1,"ok":true,"data":{
  "clean":false,
  "drift":{"cpu":["cpu.governor","cpu.max_freq"]},
  "errors":{}
}}
```

A module whose `collect` fails is reported in `errors` and does not stop the others. Dynamic CPU
caps (adaptive / screen-off) are expected to make the live max lower than the persisted static max,
so those keys are not reported while the controller is active.

### 3.13 boot, recovery, daemon and safe mode

`boot status` reports the two boot-apply settings only:

```json
{"v":1,"ok":true,"data":{"enabled":true,"delay_s":30}}
```

`boot configure --enabled true|false --delay N` changes them and returns the same payload; `N` is
validated to `0..=300` seconds (`invalid_arg` above that). Safe-mode
state and the unconfirmed-boot count are deliberately **not** part of `boot status`; `safe-mode
status` answers those, so each fact has exactly one owner.

`recovery status` reports the isolation backup and the config-file health:

```json
{"v":1,"ok":true,"data":{"backup_modules":[],"boot_count":0,"has_backup":false,
 "has_corrupt_config":false,"safe_mode":false,"threshold":3}}
```

`recovery restore` restores the backup to the active configuration but does not immediately write
sysfs; exit safe mode and reboot (or explicitly apply the settings) after reviewing it.
`recovery clear` discards a valid backup; `recovery clear-corrupt` removes a malformed
`config.json.corrupt` left behind by a damaged file.

`daemon status` reports `running`, `implemented`, `enabled` and `interval_s`:

```json
{"v":1,"ok":true,"data":{"running":false,"implemented":true,"enabled":true,"interval_s":10}}
```

`running` is whether a live daemon process passed the PID + `/proc/<pid>/cmdline` check, `enabled`
is the persisted intent, and `implemented` is always `true` on this binary — it exists so a client
talking to an older module can tell "not implemented" apart from "not running".

`daemon configure --enabled true` starts automatic drift repair and adaptive sampling; `--enabled
false` stops it. Continuous guard requires boot-time apply to remain enabled. The interval may be set
with `--interval N` (2–120 s). `daemon start` / `daemon stop` are lower-level lifecycle operations;
normal users should use `daemon configure` or the Settings screen.

The daemon also owns display transitions. Each round it reads `/sys/class/backlight/*/brightness` as
a cheap change detector and asks `dumpsys power` (3 s timeout) when **the raw brightness value
changed** — not merely when it crossed zero — when no backlight node exists, when the last decision is
older than 10 s, or while an "off" reading is waiting for confirmation. The framework answer wins,
because some devices keep a dim backlight for the always-on display while the framework is already
asleep; `unknown` is never a transition, and `mWakefulnessChanging` is not mistaken for a state
(`Display Power: state=ON/OFF` is the last-resort signal).

Turning the display **on** is applied immediately: releasing the caps can never hurt. Turning it
**off** requires two consecutive answers (at least 2 s apart), because a single stale `Dozing` at
wake-up must not cap the CPU while the user is looking at the screen. This is a hard-won invariant:
an earlier version cached the zero/non-zero backlight bucket, so one transient "off" at wake-up was
cached forever and the screen-off profile stayed applied until reboot.

The resulting state drives `cpu.screen_off_active`, the `display` events, and the `screen` field
recorded with every history sample. Polling is 1 s while the display is off (or a confirmation is
pending) and 5 s while it is on, never slower than the guard interval; each transition is logged as
`screen transition state=… -> screen_off_active=…`. The daemon releases any active profile when it
**starts** (a stale `screen_off_active` in the CPU state file is re-applied by every later `apply`, so
a crash must not inherit it) and when it **stops**, so no stale cap is left behind in either
direction. The Android screen-off controls are a secondary
page opened from the CPU screen; they are not a bottom-navigation destination. The watcher only runs
while the daemon does: a boot whose `apply-boot` failed leaves the screen-off profile inactive instead
of enforcing it without a supervisor.

#### apply-boot, boot-confirm and safe mode

`apply-boot` (called by `service.sh`) runs its checks in this order — the order matters, because a
rescue marker must win over safe mode and safe mode must win over "boot apply disabled":

1. `disable_tuning` marker in the data dir or the module dir → `{"skipped":"disable_tuning"}`
   (boot counter untouched).
2. Already in safe mode → `{"skipped":"safe_mode"}`.
3. `boot.enabled == false` → no ordinary replay, but the independently configured WALT guard is still
   applied: `{"boot_disabled":true,"walt_guard":{<set result>}}`, or
   `{"skipped":"boot_disabled","walt_guard":"not_configured"}` when `cpu.walt_guard` was never
   persisted (`"not_applied"` when the rescue marker blocks it).
4. Wait `boot.delay_s` (unless `--no-delay`).
5. If `safe.boot_count >= safe.threshold` (default 3) → enter safe mode,
   `{"skipped":"safe_mode"}`, and isolate the current `config.modules` into `recovery_backup`.
   Otherwise `boot_count += 1` and apply persisted settings.
6. Reply `{"applied":{<module>:<set result>},"errors":{<module>:{code,msg}}}`.

Before replaying persisted settings, `apply-boot` calls each supported module's reset hook, so
module-local transient state (forced WALT values, an active screen-off profile, adaptive counters,
settings applied without `--persist` last boot) cannot leak into the new boot.

`boot-confirm` sets `boot_count` to 0 (`service.sh` calls it 60 s after apply-boot).
`safe-mode status` → `{"safe_mode":false,"boot_count":0,"threshold":3,"rescue_marker":false}`;
`safe-mode clear` leaves safe mode and resets the counter (same reply).

### 3.14 note and log

`note <message>` appends a non-empty message (up to 1024 characters) at INFO level and returns
`{"logged":true}`. This preserves SoCFlow's service/UI event annotation command.

`log` → `{"lines":["<unix> LEVEL message", …]}`. `--tail N` controls how many recent lines are
returned (default 200).

### 3.15 Battery module (`battery.*`)

The battery module is **read-only**: every item is `readonly`, so `set` on any of them fails with
`invalid_arg` (`"battery.capacity is read-only"`). It never writes a node and therefore never
participates in the enforcement, boot-apply or safe-mode recovery paths. Charge-limit nodes
(`charge_control_limit`, `input_suspend`, …) are vendor-specific and high risk; they are
intentionally not exposed yet.

Source: the first readable node among `/sys/class/power_supply/{battery,Battery,bms}`. Charger state
comes from `/sys/class/power_supply/{ac,AC}/online` and `/sys/class/power_supply/{usb,USB}/online`.

| item | type | unit | meaning |
|---|---|---|---|
| `battery.capacity` | readonly | `%` | state of charge |
| `battery.status` | readonly | | raw kernel status: `Charging`, `Discharging`, `Full`, `Not charging`, `Unknown` |
| `battery.charging` | readonly | | derived boolean: true for `Charging` and `Full` |
| `battery.temp` | readonly | `0.1C` | `temp`, same convention as `cpu.battery_temp` |
| `battery.voltage` | readonly | `mV` | normalised from `voltage_now` |
| `battery.current` | readonly | `mA` | normalised from `current_now`; sign preserved as the kernel reports it |
| `battery.power` | readonly | `mW` | `voltage × current / 1000`, falling back to `power_now` |
| `battery.health` | readonly | | raw kernel health string (`Good`, `Overheat`, …) |
| `battery.health_percent` | readonly | `%` | `charge_full / charge_full_design`, clamped to 0–100 |
| `battery.cycle_count` | readonly | | charge cycles |
| `battery.charge_full` / `battery.charge_full_design` | readonly | `mAh` | normalised from `µAh` where needed |
| `battery.ac_online` / `battery.usb_online` | readonly | | charger `online` flags |
| `battery.wireless_online` | readonly | | wireless-charging `online` flag (vendor node names probed) |
| `battery.charge_mah` | readonly | `mAh` | coulomb counter (`charge_counter`); measured charge, not inferred |
| `battery.usb_type` | readonly | | negotiated mode from `usb/type`: `USB_PD`, `USB_DCP`, `USB_HCDP`, … |
| `battery.input_current_limit` | readonly | `mA` | negotiated input current ceiling |
| `battery.fast_charging` | readonly | | derived: charging and either ≥12 W or ≥2 400 mA |

**Unit normalisation.** Kernels disagree per vendor: `voltage_now` is µV on most devices and mV on
some, and the same split exists for current (µA/mA), charge (µAh/mAh) and power (µW/mW). The module
decides by magnitude (voltage > 100 000 → µV, current > 10 000 → µA, charge > 100 000 → µAh,
power > 1 000 000 → µW) and always emits the protocol unit. Clients must never re-scale a value.

**Absent fields are omitted, not `null`.** A device without a temperature sensor simply has no
`battery.temp` key, and `battery.ac_online` only appears when an AC node exists. Clients should
render a missing item as "not available" rather than zero.

Battery events are emitted by `module/service.sh`'s `battery_loop` (charging started/stopped, fully
charged, low battery, over-temperature) with dedupe keys, and the module records a 60 s `history`
series (§3.9). Clients never infer these from dashboard snapshots.

### 3.16 Battery per-app attribution (`battery apps`)

```json
{"v":1,"ok":true,"data":{
  "available":true,"cached":false,"generated_at":1795427726,
  "capacity_mah":5000.0,"computed_drain_percent":0.782,"actual_drain_percent":0.782,
  "total_mah":41.9,"count":706,
  "entries":[
    {"uid":"0","numeric_uid":0,"name":"内核 / root","packages":[],"power_mah":12.8,"share":0.3058},
    {"uid":"u0a404","numeric_uid":10404,"name":"com.android.chrome","packages":["com.android.chrome"],
     "power_mah":0.5,"share":0.0127}
  ]
}}
```

`sysflow <module> apps` is a **verb, not a schema item**: it is battery-specific and expensive. It
runs `dumpsys batterystats --charged`, reads only the `Estimated power use (mAh)` section, and stops
the dump as soon as that section ends (the rest is megabytes; `dumpsys` then aborts with a broken
pipe on stderr, which is expected and discarded). A wall-clock watchdog kills a wedged dump after
8 s.

Flags: `--limit N` (0 = all rows, default 0) and `--refresh` (bypass the 60 s cache).

**Semantics the client must respect**

| field | meaning |
|---|---|
| `uid` | BatteryStats' own label: `1000` for system UIDs, `u0a404` for app UIDs |
| `numeric_uid` | Resolved UID (`u0a404` → 10404). `null` when it cannot be derived |
| `name` | Friendly name: AID label, else first package, else `UID <n>` — never `null` |
| `packages` | Package names sharing the UID (a shared UID can map to several) |
| `power_mah` | Estimate for this UID, in mAh |
| `share` | 0..1 fraction of `total_mah`; the robust metric |

**Attribution is per UID, not per process.** Every process of an app folds into one row;
`system_server` and friends appear under Android's shared system UIDs; kernel work appears as UID 0.

**The numbers are estimates, and the absolute scale is unreliable on some ROMs.** The dump labels
the section "(mAh)" but prints thousandths, so `power_mah` divides by 1000 (a 5000 mAh battery cannot
have consumed 42 000 mAh; dividing by 1000 matches the reported drain percentage). Even so, devices
with a missing or vendor-locked power profile produce per-component currents that are plainly wrong
— one observed device reports ~2 % for the screen over six days of screen-on time. The **shares**
stay meaningful (every component's per-UID values sum exactly to the printed global values), which
is why `share` is the primary metric and clients must label the block as an estimate.

Results are cached at `<data_dir>/state/battery_apps.json` for 60 s and returned with
`"cached":true`; the payload is otherwise identical.

### 3.17 Battery health / cycle count (`battery health`)

```json
{"v":1,"ok":true,"data":{
  "cycle_count":681,"cycle_source":"Samsung 电池 EFS",
  "health_percent":93,"health_source":"Samsung BSOH",
  "cached":false,"generated_at":1795429665
}}
```

Cycle count is the one battery value vendors love to hide. `snapshot` only reads sysfs (cheap enough
for a 2 s poll), so it can miss devices entirely. This verb adds the fallbacks and caches the answer
for 60 s; `--refresh` bypasses the cache. `cycle_count` is `null` with `cycle_source` `null` when no
source yields a plausible value.

Sources, in priority order:

| source | how |
|---|---|
| `sysfs 循环计数寄存器` | `<power_supply>/cycle_count`, then `battery_cycle`, `batt_cycle`, `batt_soc_cycle`, `fg_cycle`, `cycle` |
| `内核电源驱动 uevent` | `POWER_SUPPLY_CYCLE_COUNT` / `POWER_SUPPLY_BATTERY_CYCLE` / `POWER_SUPPLY_FG_CYCLE` in `<node>/uevent` |
| `Samsung 电池 EFS` | `dumpsys batteryproperties` → `mSavedBatteryUsage` (hundredths of a cycle, so `68142` → 681) |
| `Health HAL` | `dumpsys health` (Android 14+), scanned for a labelled cycle count. The format is not contractual, so this is the last fallback and only reads a labelled integer |

Health, in priority order — **vendor calibration before any derived ratio**:

| # | source | how |
|---|---|---|
| 1 | `厂商 SOH` | `POWER_SUPPLY_SOH` / `POWER_SUPPLY_HEALTH_PERCENT` in `<node>/uevent`, else Samsung `mSavedBatteryBsoh` |
| 2 | `BatteryStats 学习容量` | `dumpsys batterystats` → `Estimated battery capacity` / `learned battery capacity` ÷ `charge_full_design` |
| 3 | `sysfs 容量比` | `<node>/charge_full ÷ charge_full_design` |

The order matters. `charge_full` mirrors the design capacity on many Samsung devices, so the ratio
reports a flat 100 % while the calibrated value says 99 %. A derived ratio is a fallback, never the
answer when a measured source exists.

Levels 2 and 3 exist because `dumpsys batteryproperties` was removed in recent One UI builds, which
killed the Samsung EFS path. Level 2 costs a full `batterystats` dump, so it only runs when level 1
missed and the whole verb is cached for 60 s. Values outside 1..9999 cycles, or outside 20..105 % health, are
rejected as placeholder data — drivers commonly report `0`.

`soh` and `count` nodes are deliberately **not** read as cycle counts: they carry health or unrelated
counters, and treating them as cycles would report fiction.

### 3.18 Wakelock module (`wakelock.*`)

The module has two halves. The **read** half (`kernel_sources`, `ranked`, `summary`) is telemetry and
never writes. The **write** half (`wakelock.blocked`) blocks an app's wake locks through AppOps; it is
`risk: medium`, persisted, replayed at boot and drift-checked by the daemon. Kernel wakeup-source
blocking is intentionally not offered: the kernel exposes no portable writable interface for a
wakeup-source *name* (the `/sys/.../power/wakeup` files are device wakeups, and mapping names to them
is guesswork).

Sources, in probe order:

| path | layout | units |
|---|---|---|
| `/sys/kernel/debug/wakeup_sources` | 9 counters after the name (`active_count`, `event_count`, `wakeup_count`, `expire_count`, `active_since`, `total_time`, `max_time`, `last_change`, `prevent_suspend_time`) | milliseconds |
| `/proc/wakelocks` | legacy 8 counters (no `event_count`; `active_count`, `expire_count`, `wakeup_count`, …) | nanoseconds |

Neither readable → the module stays **supported** (blocking does not need debugfs) but loses the
`kernel_sources` / `history` features, `summary.available` is `false`, the kernel items are empty, and
`summary.reason` carries the underlying error (e.g. `"/sys/kernel/debug/wakeup_sources: No such file
or directory"`) so the client can explain it instead of guessing.

**debugfs is mounted on demand.** Android does not mount `/sys/kernel/debug` by default on recent
releases, so when the table is missing the module runs `mount -t debugfs debugfs /sys/kernel/debug`
once: on Android only (host builds never mount), at most once per hour *on failure* (a successful
mount is never rate-limited, because it may only be visible inside the process's mount namespace), and
only when `/sys/kernel/debug` exists as a directory. The mount is read-only by nature, idempotent, and
disappears on reboot; reading uses a lossy UTF-8 conversion so one malformed byte in a source name
cannot be reported as "unsupported".

| item | type | content |
|---|---|---|
| `wakelock.kernel_sources` | readonly list | up to 24 sources, active first then by accumulated time: `name`, `category`, `active`, `active_count`, `event_count`, `wakeup_count`, `expire_count`, `active_since_ms`, `total_ms`, `max_ms`, `prevent_suspend_ms` |
| `wakelock.ranked` | readonly list | the 24 h window, largest first, at most 15: `name`, `category`, `awake_ms`, `share` (of measured wake time, 0–1), `wakeups` (increments in the same window), `events` (`0` when the table has no event counter) |
| `wakelock.summary` | readonly object | `available`, `known`, `active`, `window_s`, `window_reason`, `since_boot_s`, `updated_s`, `measured_ms`, `total_wakeups`, `ranking_available`, and `reason` (only when `available` is false) |
| `wakelock.protected` | readonly list | packages the module refuses to block; the module is the authority, clients mirror it instead of keeping a copy |
| `wakelock.blocked` | `bool`, scope `app`, risk `medium`, persisted | object keyed by package: `true` = this app's `WAKE_LOCK` op is `ignore` |
| `wakelock.exempt` | `bool`, scope `app`, risk `low`, persisted | user whitelist: `true` = never block this app. Configuration only, so clients read the persisted intent from `snapshot.settings` |

**`category` is a driver family, not an app.** Kernel wakeup sources are named after drivers
(`rmnet_ipa%d`, `hal_bluetooth_lock`, `alarmtimer.0.auto`), and the kernel cannot say which app caused
a wakeup — one packet send lights up the whole network path. `category` is a stable, language-neutral
identifier so clients can group the raw names into something readable:

`system` (PowerManager), `bluetooth`, `nfc`, `network` (rmnet/IPA/WLAN), `timer` (alarm/rtc/hrtimer),
`touch` (TSP/Wacom), `display`, `power` (battery/charger), `audio`, `camera`, `sensor`, `storage`,
`usb`, `accessory` (covers/hall), `subsystem` (smp2p/glink/modem), `serial` (UART/QUP/I2C) and
`other`. Clients own the display strings and must fall back to a generic label for unknown values.
Per-**app** wake-lock time is a different view: §3.20 (`wakelock apps`).

**`wakelock.blocked` semantics.** It is one item whose value is a map, which is how an app-scoped
item is shaped (§3.4): `{"com.example.app": true, "com.other.app": false}`. Only the packages named in
the request are touched; `applied` reports the read-back from `cmd appops query-op WAKE_LOCK ignore`,
so a ROM that silently keeps the old mode shows up as a `mismatch` rather than as success.

- **Opt-in reads.** Answering it costs a binder call, so it is **never part of `snapshot`**: ask for it
  with `get --keys wakelock.blocked` (or read the persisted intent from `snapshot.settings`).
- **Protected apps.** Blocking is refused with `invalid_arg` for: `android`, `com.android.systemui`,
  `com.android.phone`, `com.android.server.telecom`, the AOSP/Google dialers and SMS apps,
  `com.android.settings` and `io.sysflow`. Unblocking them is allowed; `reset` skips them so a system
  default the module did not set is never flipped. The list is published as `wakelock.protected`.
- **User whitelist.** `wakelock.exempt: {pkg: true}` means "never block this app". Exempting implies
  releasing an existing block (the implied `blocked: false` is part of `applied`, so it persists);
  `sanitize_boot` drops stored blocks for exempt apps and `drift` releases one that appears later.
  The protection decision is per entry, so a request may release a protected app while blocking
  another.
- **Release writes AppOps `default`, never `allow`.** `allow` would leave a permanent explicit grant
  that overrides the app's manifest and any OEM policy — that changes the app instead of restoring it.
- **All-or-nothing.** The request is validated first (package-name shape, protection list), then
  written; if one write fails the already-written packages are rolled back to their previous mode.
- **Persistence and boot.** `--persist` stores the map in `config.modules.wakelock`; `apply-boot`
  replays it after dropping packages that were uninstalled or became protected/exempt. The daemon's
  `drift()` compares the persisted map with `query-op` and re-applies on mismatch. Persistence is
  per entry: if one package is rejected, the packages that did apply are still stored.
- **Reset.** `reset` releases exactly the packages this module persisted as blocked — not "everything
  that is ignored". A rule the user set through another tool (or an OEM default) survives, because
  the module cannot see the intent behind it and must not revert what it never set. `reset` with no
  persisted settings does nothing.
- **A failed rollback is reported.** If a write fails and undoing an earlier write also fails, the
  error names the packages that need manual repair instead of pretending everything was undone.

**How the window works.** Kernel counters are monotonic since boot, so a ranking needs a stored
baseline: `<data_dir>/state/wakelock.json` holds `baseline_ts`, one baseline map, the last observed
map and the reason the window started. The daemon's 300 s `sample()` advances it (the ranking is
read-only between samples, so a 2 s `snapshot` poll costs one small file read).

- **A window older than 24 h** rolls forward to the previous observation instead of discarding the
  data (`window_reason = "rollover"`).
- **A reboot restarts the window** (`window_reason = "reboot"`). Counters are since-boot, so a
  baseline from a previous boot would clamp every delta to zero for the rest of the window — and the
  page would explain that as "the device was idle". Detection order: the kernel boot id
  (`/proc/sys/kernel/random/boot_id`) first, system uptime second (a wall-clock jump is not a
  reboot), and a widespread counter regression only when neither is readable. `window_s` is capped by
  the current uptime, so it never claims 24 h for a window that is three minutes old.
- **Counters that moved backwards** clamp to zero rather than producing a negative duration.
- **A fresh install** has no ranking until the second sample (`window_reason = "fresh"`).
- **`share`** is relative to `measured_ms`, the sum over *every* source with an increment — not to
  wall-clock 24 h and not to the truncated top-15 the client displays.
- **Tracking is bounded**: the stored window keeps at most 512 sources, pruned to the ones still
  present, so driver churn cannot grow the state file without limit.
- **State is versioned**: a v1 file (milliseconds only) still loads; the next sample writes v2 with
  the wakeup/event counters that feed `wakeups`.

### 3.19 Userspace wake locks (`wakelock held`)

```json
{"v":1,"ok":true,"data":{
  "available":true,"count":2,"cached":false,"generated_at":1795452252,
  "held":[
    {"kind":"PARTIAL_WAKE_LOCK","tag":"AudioMix","held_ms":62000,
     "uid":1041,"pid":1234,"package":"com.android.systemui","owner":"com.android.systemui"},
    {"kind":"PARTIAL_WAKE_LOCK","tag":"*vibrator*","held_ms":39,
     "uid":1000,"pid":14922,"package":null,"owner":"Android 系统"}
  ]
}}
```

`sysflow wakelock held [--refresh]` parses the `Wake Locks:` block of `dumpsys power`, sorted by
`held_ms` descending.

- **Attribution layouts differ.** AOSP prints `(uid=1041, pid=1234, pkg=com.android.systemui)`;
  Samsung prints space-separated pairs with a nested work source and no `pkg=`
  (`(uid=1041 isFrozen=false ws=WorkSource{ chains=WorkChain{(10390), (1041)}} lock=…)`). Parsing
  scans to the *matching* close parenthesis and accepts comma or whitespace separators, so the nested
  braces cannot cut the attribution short.
- **`package` is nullable and actionable.** `owner` is the display label: the package when the UID
  maps to exactly one, otherwise the AID name (`Android 系统`) or `UID <n>`. A shared UID is never
  turned into one of its packages, because blocking one app for a UID others share would be a wrong
  action. It is a **verb, not a schema item**, because the call crosses into the Java
system server (8 s timeout) and must never sit in `snapshot`. Results are cached for 60 s under
`<data_dir>/state/wakelock_held.json`; `--refresh` bypasses the cache. When `dumpsys` fails the
command returns the usual `io` error rather than an empty list, so a SELinux denial is not reported as
"no wake locks".

### 3.20 Per-app wake-lock time (`wakelock apps`)

```json
{"v":1,"ok":true,"data":{
  "available":true,"cached":false,"generated_at":1795452252,"total_ms":145000,
  "entries":[
    {"uid":"u0a346","numeric_uid":10346,"name":"com.example.worker",
     "packages":["com.example.worker"],"total_ms":30108,"count":5,"share":0.207,
     "tags":[{"tag":"*job*e//SessionWorker/@eM_ADDR/...","total_ms":28608,"count":3}]}
  ]
}}
```

`sysflow wakelock apps [--limit N] [--refresh]` answers "which app accumulated the most wake-lock
time since the last charge", from the `All partial wake locks:` section of
`dumpsys batterystats --charged`. It complements `wakelock held` (who holds a lock right now) and
`wakelock.ranked` (which kernel wakeup source kept the device awake).

- **Streamed, then stopped.** The section sits hundreds of lines into a 4 000+ line dump, so the
  reader feeds lines to `WakeLockScanner` and kills the child as soon as the section ends.
- **Row shape.** `Wake lock <uid> <tag>: <duration> (<n> times) max=… actual=… realtime`. A tag may
  itself contain colons (`*alarm*:android.intent.action…`), so the tag/duration split is the first
  `": "` that is followed by a digit, not the first colon on the line.
- **Aggregation.** Rows are grouped by UID (all tags of one app sum together); `tags` keeps the
  three longest tags for context. `share` is relative to `total_ms`, which always covers **every**
  measured row — `--limit` only trims the displayed entries.
- **Labels.** UID → package names come from `cmd package list packages -U`, with the AID table
  (`Android 系统`, `电话服务`, …) as the fallback, and `UID <n>` as the last resort. A row is never
  labelled `null`.
- **Cost.** One `dumpsys` call behind a 60 s cache, so it is a verb and never part of `snapshot`. A
  dump without the section (some builds) returns `available: false` with a reason instead of an
  empty list.

### 3.21 Last wakeup (`wakelock wakeup`)

```json
{"v":1,"ok":true,"data":{
  "available":true,"reason":"1234","age_ms":62000,
  "generated_at":1795452252,"cached":false
}}
```

`sysflow wakelock wakeup [--refresh]` answers "what woke the device last". Same cost as
`wakelock held` — one `dumpsys power` behind a 60 s cache — so it is a verb, never a snapshot item.

- **Build variance is expected.** AOSP prints `Last wakeup reason: …`, vendor builds print
  `mLastWakeupReason=…`, and several print neither. `available: false` plus `note` means "this ROM
  does not report it" — never "nothing woke the device", which would be a different (and false)
  answer.
- **`age_ms` is optional.** A printed `(802127 ms ago)` next to `mLastWakeTime` is used as-is
  (Samsung); otherwise it is derived from `mLastWakeupUptimeMs` against `/proc/uptime`, and dropped
  when the value is missing or inconsistent (a negative age is worse than no age).
- **Key matching is case-insensitive and covers vendor spellings.** Samsung prints
  `mLastWakeUpReason:` (capital "Up"), which lowercases to the same key as `mLastWakeupReason`.
- **`reason` is raw.** Kernel reason codes are vendor-specific, so the module does not translate
  them; clients show the string as-is.

### 3.22 Memory module (`mem.*`)

> **Status: implemented — telemetry (M3a) and tunables (M3b).** Steps 0–8 of
> `docs/mem-module-plan.md` are done; only the deferred zRAM reconfiguration (Step 9) and device
> acceptance remain. Ids, units and shapes are frozen and confirmed against a device probe
> (`docs/modules/mem.md`); `snapshot` and `get` return every item below that the device offers, and
> `set` accepts the two tunables. Device timings and the event-noise thresholds are still the
> unverified part.

The memory module has two halves. **Telemetry** (`readonly` items, M3a) never writes a node and takes
no part in enforcement, boot-apply or safe-mode recovery. **Tunables** (`swappiness`,
`vfs_cache_pressure`, M3b) go through the same `apply` / `drift` / `sanitize_boot` / `persist` path
as the CPU module. Both halves are shipped: `schema` advertises the tunables only when the telemetry
they modulate is readable, so an unsupported device never shows half a page.

#### Tunables (`mem.swappiness`, `mem.vfs_cache_pressure`)

Only two knobs, both plain `/proc/sys/vm` integers, both `int_range` / `low` / `persist`.

| item | range | note |
|---|---|---|
| `mem.swappiness` | `0..=100`, or `0..=200` from kernel 5.8 | the ceiling is read from `/proc/sys/kernel/osrelease`, never assumed |
| `mem.vfs_cache_pressure` | `1..=1000` | `0` means "never reclaim dentries", so it is not offered |

`dirty_*` and `min_free_kbytes` are deliberately absent: the first is byte-configured and already
tuned on real ROMs, the second is a watermark whose wrong value causes the OOM it is meant to avoid.

- **Validate before writing.** A request is checked in full before the first write, so a rejected
  request never leaves the device half-applied.
- **Read back.** `applied` is the kernel's answer, not the request; `mismatch` lists what it refused.
- **Drift is repaired, but not forever.** When the persisted value and the live value disagree, the
  daemon writes it back. Every tick that still finds the item wrong counts as one repair attempt
  (asking twice inside one tick does not count twice, and it does not matter whether the value went
  back in between or never did). After **3 attempts on the same item within 30 minutes** SysFlow stops and
  leaves it alone for **6 hours**, emitting `mem_setting_contested` (category `settings`) once, with
  `{item, live, window_s, suppress_s}` in `metadata`. Losing a fight silently is worse than saying so.
- **Boot replay is sanitised.** A persisted value this kernel cannot accept (different kernel, edited
  config) is dropped at boot instead of failing the whole module.

Sources: `/proc/meminfo`, `/proc/pressure/memory`, `/proc/swaps`, `/sys/block/zram*/`. Item ids are
`mem.<name>`; zRAM items are `mem.zram.<name>` and PSI items are `mem.psi.<name>`.

| item | type | unit | meaning |
|---|---|---|---|
| `mem.total` / `mem.available` / `mem.free` / `mem.cached` | readonly | `kB` | `MemTotal` / `MemAvailable` / `MemFree` / `Cached` |
| `mem.anon` / `mem.shmem` / `mem.slab` | readonly | `kB` | `AnonPages` / `Shmem` / `Slab` (`Cached` already contains `Shmem`; clients must not add them) |
| `mem.dirty` | readonly | `kB` | `Dirty` + `Writeback` |
| `mem.swap_total` / `mem.swap_free` | readonly | `kB` | `SwapTotal` / `SwapFree` |
| `mem.swap.zram_kb` / `mem.swap.other_kb` | readonly | `kB` | `/proc/swaps` **used** space, split by device name (`zram*` vs everything else); a readable table with no swap gives `0`/`0`, a missing file omits both |
| `mem.psi.some_avg10` / `some_avg60` / `some_avg300` | readonly | `0.01%` | integer, `123` = 1.23 % |
| `mem.psi.full_avg10` / `full_avg60` / `full_avg300` | readonly | `0.01%` | integer |
| `mem.zram.count` | readonly | | number of `zram*` devices |
| `mem.zram.disksize` / `orig` / `compr` / `mem_used` | readonly | `kB` | summed over all zram devices; `mm_stat` bytes are divided by 1024 |
| `mem.zram.ratio` | readonly | `%` | integer percentage, `round(100 × sum(orig) / sum(compr))` from the summed byte totals (`295` = 2.95×), never an average of per-device ratios; omitted when `sum(compr)` is 0 |
| `mem.zram.writeback` | readonly | — | `1` when a zram device has a backing store (`backing_dev`), `0` otherwise. With writeback on, pages can live on flash, so `orig`/`ratio` no longer describe all swap traffic |
| `mem.kernel_stack` / `mem.page_tables` / `mem.percpu` | readonly | `kB` | kernel memory outside `Slab`; these fill part of the composition bar |
| `mem.vmalloc_used` | readonly | `kB` | vmalloc address space in use |
| `mem.gpu` | readonly | `kB` | `GpuTotal` where the kernel reports it (on S25 it equals kgsl `page_alloc` exactly) |
| `mem.mglru` | readonly | — | `1` when multi-generational LRU is enabled (`/sys/kernel/mm/lru_gen/enabled` non-zero), `0` when the switch exists and is off; absent on kernels without it. MGLRU dilutes what `swappiness` can promise, so clients should say so before offering the slider |
| `mem.vendor.rbin` / `mem.vendor.cma` / `mem.vendor.dmaheap` | readonly | `kB` | vendor **pool** sizes (`RbinTotal`, `CmaTotal`, `DmaHeapPool`). They are pools, not proven usage: clients must draw them as their own segment, apart from the measured items |

The composition items (`mem.kernel_stack` … `mem.vendor.*`) are optional: each appears only when
`/proc/meminfo` reports the field, so a kernel without it simply has fewer items.

**Absent fields are omitted, not `null` and not `0`.** A kernel without PSI has no `mem.psi.*` keys; a
device without zRAM has no `mem.zram.*` keys. Clients render a missing item as "not available" and
hide the section. `unsupported` is returned for the whole module only when `/proc/meminfo` itself is
unreadable.

**Units are final.** `mem.psi.*` is an integer in hundredths of a percent so low pressure keeps its
precision; clients must not re-scale. Sizes are `kB` as in §1.

**Cumulative counters are not exposed.** PSI `total` and `/proc/vmstat` counters are only useful as
differences, so they are consumed inside the module for `history` (below) and never appear in
`snapshot` or `get`.

#### History series

`sysflow history mem` follows §3.9. One sample per **300 s**. Series keys carry no
`mem.` prefix, like `battery`'s `level`.

| series | unit | derivation |
|---|---|---|
| `available_kb` | `kB` | `MemAvailable`, instantaneous |
| `swap_used_kb` | `kB` | `SwapTotal - SwapFree`, instantaneous |
| `zram_compr_kb` | `kB` | zRAM `compr`, instantaneous |
| `psi_some_pct` / `psi_full_pct` | `0.01%` | `Δtotal ÷ elapsed µs` over the sample interval |
| `pgscan_direct` | pages | interval difference |
| `allocstall` | count | interval difference, summed over zones |
| `swap_in_kb` / `swap_out_kb` | `kB` | `Δpswpin / Δpswpout × page size` |
| `refault` | pages | interval difference of `workingset_refault*` |
| `oom_kill` | count | interval difference (kernel OOM only) |

- **Averages, not peaks.** A difference keeps the *total* stall time of the interval but reports its
  *average* intensity. A 2 s `full` stall inside 300 s shows up as ~0.67 %, not as the 2 s burst
  it was. Clients must describe the series as "stall time in this interval", never as a peak.
- **Real elapsed time.** Rates divide by the measured time since the previous sample, not by 300 s,
  and that time is taken from the boot clock (`/proc/uptime`) so a wall-clock resync cannot distort
  it; the wall clock is only the fallback when `/proc/uptime` is unavailable.
  An interval longer than three nominal intervals (device was asleep) is still recorded but never
  raises events.
- **No sample after a rebaseline.** After a reboot (boot id change, uptime or counter regression) the
  first difference point is **omitted** (a gap in the series), not recorded as `0`.
- Counter names differ between kernels (`workingset_refault` vs `workingset_refault_file`/`_anon`);
  a counter that cannot be found removes its series rather than reporting `0`.
- A series whose *input* is unavailable is omitted rather than guessed: `zram_compr_kb` needs a zram
  device, and `swap_in_kb` / `swap_out_kb` need the kernel page size (`AT_PAGESZ`), which is read from
  the auxiliary vector because arm64 ships both 4 kB and 16 kB kernels.

#### Events

Emitted by the module's own sampling (no shell loop, no client inference), with the usual dedupe
keys (`docs/event-system.md`). Category is `pressure` for all of them.

| `event_type` | severity | trigger | dedupe |
|---|---|---|---|
| `mem_pressure_high` | `warning` | `psi_full_pct` at or above `3.00 %` for **3 consecutive** samples (15 min) | key `mem.pressure.high`, 6 h window |
| `mem_pressure_recovered` | `info` | below `1.00 %` for **3 consecutive** samples, only after a `high` | key `mem.pressure.recovered`, 6 h window |
| `mem_oom_kill` | `notice` | `oom_kill` difference > 0 | key `mem.oom_kill`, 6 h window |

A sample inside the dead band between the two thresholds resets both runs: "three consecutive" means
three in a row, not three eventually. A reboot ends the episode — neither `high` nor `recovered` is
reported across it — and a sample whose interval exceeds three nominal intervals (15 min) is recorded
but raises no event, because an average over a long sleep says nothing about the last five minutes.

The thresholds are **placeholders**, kept as constants in the module: they were chosen as ≈ 6× the
device's long-run mean (`0.52 %` on S25) and will be replaced once a quiet-window measurement exists.

`metadata` carries `available_kb`, `psi_full_pct`, `psi_full_avg60` and `swap_used_kb`; `mem_oom_kill`
adds `count` (kills since the previous sample). The display
state is not known to the module — `history` records it per sample instead (§3.9). `mem_pressure_high`
also tries to attach `last_low_memory_exit` (`package`, `timestamp`, `package_uid`, `rss`) parsed from
`dumpsys activity exit-info` — the record with the **newest timestamp**, not the last one in the dump
(the dump is grouped per package, so its order is not chronological); that is the only place a sample shells out, it happens only when the
event fires, and a failure simply omits the field.
`/proc/vmstat`'s `oom_kill` counts **kernel** OOM only; Android's `lmkd` kills are not guaranteed to
appear in it, so `mem_oom_kill` never claims "an app was killed".

### 3.23 Memory per-app ranking (`mem apps`)

```json
{"v":1,"ok":true,"data":{
  "available":true,"cached":false,"generated_at":1795452252,
  "limit":3,"total_pss_kb":1050000,"skipped":1,"pss_for_top_n_only":true,
  "entries":[
    {"uid":"u0a346","numeric_uid":10346,"name":"com.example.app",
     "packages":["com.example.app"],"pss_kb":480000,"swap_pss_kb":96000,"procs":2,"share":0.457}
  ]
}}
```

`sysflow mem apps [--limit N] [--refresh]` answers "which app holds the most memory right now". It
is a verb and never part of `snapshot`: it opens one `/proc/<pid>` entry per process.

- **Two-stage ranking.** Processes are ranked by `VmRSS`; only the top `limit` are read from
  `smaps_rollup` for `Pss` and `SwapPss`. `limit` defaults to 30 and is clamped to 1..100.
- **Estimates.** A process below the cut contributes its `VmRSS` instead of `Pss`, and the response
  sets `pss_for_top_n_only: true` so a client can present those numbers as approximate. It is
  `false` only when every ranked process was measured.
- **Aggregation** is per UID: all processes of an app sum together. `procs` is how many processes
  that UID had, and `share` is `pss_kb / total_pss_kb` rounded to three decimals.
- **Labels**: the package name from `/data/system/packages.list` (falling back to
  `cmd package list packages -U`), then the AID table, then `UID <n>`; never `null`. A UID carrying
  a user prefix (work profile, secure folder) is matched on `uid % 100000` and labelled
  `package (user N)`.
- **`uid`** is the short form (`u0a346`), the inverse of `battery apps`' `resolve_uid`;
  `numeric_uid` is the real UID.
- **Unreadable processes are skipped** and counted in `skipped`. Kernel threads have no `VmRSS` and
  are not counted, otherwise the number would be dominated by them.
- **Cost.** The ranking is cached for 60 s under `state/mem_apps.json`, keyed by `limit` because the
  PSS cut-off moves with it; the UID→package map has its own ten-minute cache
  (`state/mem_packages.json`). Measured on a five-process host tree at 5 ms cold / 8 ms served from
  cache; a device with several hundred processes is dominated by the `smaps_rollup` reads for the top
  N.

### 3.24 Memory breakdown (`mem breakdown`)

```json
{"v":1,"ok":true,"data":{
  "available":true,"cached":false,"generated_at":1795605540,
  "system_pss_kb":2100000,"user_pss_kb":2600000,"total_pss_kb":4700000,
  "processes":523,"measured":519,"estimated":3,"skipped":1
}}
```

`sysflow mem breakdown [--refresh]` splits the app-visible memory between **system** and **user**
applications. It is a verb, never a `snapshot` item: unlike `mem apps` (top-N by RSS) it measures PSS
for **every** process, because a split computed from a truncated list would be wrong in whichever
direction the cut fell. Expect a few hundred milliseconds; results are cached 60 s.

- **Classification is by UID**, the only classification the kernel gives for free: `uid < 10 000` is
  the platform (`system_server`, `surfaceflinger`, native daemons), `uid >= 10 000` is an app —
  including work-profile copies, whose UID carries a user prefix.
- **`estimated`** counts processes whose `smaps_rollup` could not be read (old kernel, SELinux): their
  `VmRSS` is used instead so the sum stays complete. **`skipped`** counts processes that disappeared
  between listing and reading.
- **These numbers do not add up to "used memory"** and must not be presented as if they did: PSS
  counts shared pages fractionally per process, and kernel / GPU / vendor pools are not app memory.
  Clients show this as its own layer, next to the physical composition rather than inside it.

## 4. Generated client types

`core/sysflow-types` is the Rust source of truth for the wire model. Regenerate the client model
instead of editing it by hand:

```sh
tools/gen-types.sh
# writes app/src/main/kotlin/io/sysflow/core/model/Protocol.kt
```

The generator (`cargo run -p sysflow-types -- --ts <file> --kotlin <file>`) still supports a
TypeScript target, but no TypeScript model is generated today: the module WebUI is plain JS over the
JSON envelope (`docs/webui-console.md` covers why), so `gen-types.sh` passes `--kotlin` only.

Kotlin uses kotlinx.serialization with `@SerialName` and leaves module payloads
(`snapshot.modules`, `settings`, `defaults`) as `JsonObject`, because each module has its own dynamic
items. `Envelope<T>` / `ProtocolError` are shared by every response.

Generated types today: `ErrorCode`, `ItemType`, `Scope`, `Risk`, `ProtocolError`, `Caps`,
`CapsData`, `SchemaItem`, `Schema`, `ApplyResult`, `SnapshotData`, `CheckData`, `VersionData`,
`SafeStatus`, `HistoryData`, `DaemonStatus`. Two known gaps: `DaemonStatus` models only `running`
and `implemented` — the CLI also returns `enabled` and `interval_s` — and there is no `BootStatus`
type, so `boot status` is still parsed as `JsonObject`.

## 5. Compatibility

- `protocol` is an integer bumped only for incompatible changes.
- The APK compares `version.protocol` with its own `PROTOCOL_VERSION` for **exact equality** and
  refuses to talk to the module when they differ. There is no `MIN_SYSFLOW_PROTOCOL` and no
  "older vs newer" split: the single message names both versions and asks the user to update the
  module *or* the app (`协议版本不匹配：模块 1，应用 2。请更新模块或应用。`).
- Because the check is equality, a protocol bump requires a module and an APK release together; the
  "additive-only" rule below is what keeps that rare.

## 6. Environment overrides (tests and host runs)

| variable | default | purpose |
|---|---|---|
| `SYSFLOW_DATA_DIR` | `/data/adb/sysflow` | config, log, history, `state/cpu.json` |
| `SYSFLOW_MODULE_DIR` | `/data/adb/modules/sysflow` | rescue marker lookup |
| `SYSFLOW_SYSFS_ROOT` | `/` | fake sysfs/procfs tree |
| `SYSFLOW_MOCK` | unset | `1` registers the mock module |
