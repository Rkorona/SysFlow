# SysFlow

Unified system manager for rooted Android: a root **module** (Rust binary `sysflow`, module-owned
enforcement/adaptive daemon and KernelSU WebUI) plus an **M3 Expressive APK** that is pure UI
and never runs in the background.

Current module version: `v0.3.2` (`module/module.prop`).

- Plan and decisions: [docs/development-plan.md](docs/development-plan.md)
- Protocol: [docs/protocol.md](docs/protocol.md)
- Event system: [Root-owned open event journal + APK Room read-model cache](docs/event-system.md)
- CPU paths and device notes: [docs/modules/cpu.md](docs/modules/cpu.md)
- Battery paths and device notes: [docs/modules/battery.md](docs/modules/battery.md)
- Wakelock paths and device notes: [docs/modules/wakelock.md](docs/modules/wakelock.md)
- Memory paths, items and device notes: [docs/modules/mem.md](docs/modules/mem.md)
- Memory module plan (delivery order and step record): [docs/mem-module-plan.md](docs/mem-module-plan.md)
- Wakelock hardening record (P0–P3 fixes, completed): [docs/wakelock-hardening-plan.md](docs/wakelock-hardening-plan.md)
- WebUI scope: [docs/webui-console.md](docs/webui-console.md)

> There is no `docs/adr/` directory in this tree. Decisions that older notes call "ADR 0003/0005/0006"
> are recorded inline in `docs/development-plan.md` (the "当前进度" section and §3) and in the
> per-module documents; nothing links to an ADR file any more.

## Layout

```
core/      Rust workspace (sysflow-core, sysflow-modules, sysflow-cli, sysflow-types)
module/    KernelSU / Magisk / APatch module template + built WebUI
module/webroot/  static rescue console (plain HTML/CSS/JS, KernelSU exec API)
app/       Material 3 Expressive Android APK (Koin + libsu + Compose + Room event cache)
tools/     pack-module.sh, gen-types.sh, tests/
docs/      plan, protocol, per-module notes and module plans
dist/      packaged module zip + SHA256
```

## Try it on a host (no device, mock module)

```sh
cd core
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test
export SYSFLOW_DATA_DIR=/tmp/sysflow SYSFLOW_MOCK=1
cargo run -p sysflow-cli -- caps --pretty
cargo run -p sysflow-cli -- mock set '{"mock.level":100}'
```

The mock module is only registered when `SYSFLOW_MOCK=1`, so it never appears on a device. It
exercises every item type, the read-back `mismatch` path (`mock.level` 100 is clamped to 95) and a
module-enforced lock (`mock.cores[0]` stays on).

## CPU module

`core/sysflow-modules/src/cpu/` contains the migrated CPU implementation: core on/off, per-cluster min/max and
governor, temperature, `time_in_state`/`/proc/stat` readings, WALT parameters and
`adaptive_*` coupling, the WALT write guard, GPU devfreq, DDR/LLCC boost, cpuset groups, the
screen-off profile and the adaptive cap controller. Transient state (WALT originals, screen-off
range, adaptive hysteresis) lives in `<SYSFLOW_DATA_DIR>/state/cpu.json`.

```sh
sysflow cpu schema                       # every item this device offers
sysflow cpu get --keys cpu.max_freq,cpu.gpu_max_freq
sysflow cpu set '{"cpu.max_freq":{"4":1800000}}' --persist
sysflow cpu rebaseline                   # re-seed cpu.min_freq defaults from cpuinfo_min_freq
sysflow check                            # read-only drift check for the enforcement daemon
```

See `docs/protocol.md` §3.11 for the item list and `docs/modules/cpu.md` for paths, units and
rollback rules.

## Battery module

`core/sysflow-modules/src/battery/` is the first **read-only** module: battery, charger and health
telemetry with no writable item, so it never touches `apply`, drift repair, boot restore or safe
mode. Units are normalised in Rust (µV/mA/µAh/mW → mV/mA/mAh/mW), and a missing sysfs field simply
omits its item instead of reporting zero. See `docs/protocol.md` §3.15–§3.17 and
`docs/modules/battery.md`.

On a device the binary lives at `/data/adb/modules/sysflow/bin/sysflow`; the installer links it into
the manager's `bin` directory (`/data/adb/ksu/bin`, `/data/adb/ap/bin`, `/data/adb/magisk`) so plain
`sysflow` works in a root shell.

```sh
sysflow battery get --pretty       # capacity, status, temp, voltage, current, power, health, cycles
sysflow battery health --pretty    # cycle count + health, with the source that answered
sysflow battery apps --limit 20    # per-UID power attribution from BatteryStats
sysflow history battery --since 24h --step 1h
sysflow battery set '{"battery.capacity":50}'   # rejected: read-only
```

Charging events come from `module/service.sh`'s `battery_loop` (charging started/stopped, fully
charged, low battery, over-temperature, all deduplicated), and the module records a 60 s sample the
enforcement daemon appends to `<data_dir>/history/battery.jsonl` (30 days / 20 000 lines). The APK
battery page draws that durable 24 h curve next to the session-only sparklines.

## Wakelock module

`core/sysflow-modules/src/wakelock/` has two halves. The read half parses the kernel wakeup-source
table (`/sys/kernel/debug/wakeup_sources`, or the legacy `/proc/wakelocks`) and reports who kept the
device awake, with `wakelock.ranked` as a rolling 24 h ranking computed from stored counter baselines.
`sysflow wakelock held` lists the userspace locks from `dumpsys power` on demand (60 s cache), and
`sysflow wakelock apps` ranks the time each app held partial wake locks since the last charge
(streamed out of `dumpsys batterystats`, 60 s cache). `sysflow wakelock wakeup` reports the last
wakeup reason where the ROM exposes one.

The write half blocks an app's wake locks through AppOps:

```sh
sysflow wakelock set '{"wakelock.blocked":{"com.example.app":true}}' --persist
sysflow wakelock get --keys wakelock.blocked
```

Rules persist, are replayed at boot and drift-checked by the daemon; dialer, telephony, SystemUI,
`android`, Settings and SysFlow itself cannot be blocked. Blocking can delay notifications, so it is
risk medium and the UI says so. See `docs/protocol.md` §3.18–§3.21 and `docs/modules/wakelock.md`.

## Memory module

`core/sysflow-modules/src/mem/` implements both halves:

- **Telemetry** (read-only, in `snapshot`): `/proc/meminfo` totals and composition (`anon`, `cached`,
  `slab`, `dirty`, `kernel_stack`, `page_tables`, `percpu`, `vmalloc_used`, `gpu`, vendor pools),
  swap (`/proc/swaps`, split zRAM vs other), zRAM geometry/compression ratio/writeback, PSI pressure,
  and the MGLRU switch.
- **Tunables** (`risk: low`, persisted, drift-repaired): `mem.swappiness` and
  `mem.vfs_cache_pressure`. `dirty_*`, `min_free_kbytes` and zRAM reconfiguration are deliberately
  not exposed (see `docs/mem-module-plan.md`).

```sh
sysflow mem schema --pretty          # exactly what this kernel offers
sysflow mem get --pretty
sysflow mem set '{"mem.swappiness":130}' --persist
sysflow mem apps --limit 30          # per-UID PSS ranking (top-N by RSS)
sysflow mem breakdown                # system vs user application PSS
sysflow history mem --since 24h --step 30m
```

The module samples every 300 s into `<data_dir>/history/mem.jsonl` (instantaneous values plus
counter differences taken against a baseline from the same boot) and emits `mem_pressure_high`,
`mem_pressure_recovered`, `mem_oom_kill` and `mem_setting_contested` from inside `sample()`. A value
that a ROM keeps rewriting is abandoned for 6 hours after three repairs inside 30 minutes instead of
being fought every tick. See `docs/protocol.md` §3.22–§3.24 and `docs/modules/mem.md`.

## Process module

`core/sysflow-modules/src/process/` is verb-only: no schema items, no `snapshot` entry, no history
(it is already excluded via `in_snapshot() == false`). The list is a two-pass `/proc` scan (CPU% is
**per core**, 0–100 × cores) and is only called while the APK page is visible.

```sh
sysflow process list --limit 50 --sort cpu --filter all --sample-ms 500
sysflow process list --query com.example --no-foreground
sysflow process get 8123                       # PSS, nice/policy, cgroup/cpuset
sysflow process stop com.example.app           # one-shot force-stop
sysflow process oom com.example.app 100        # oom_score_adj, high risk, not persisted
sysflow process renice com.example.app -5      # setpriority(2), toybox renice as fallback
sysflow process cpuset com.example.app top-app # experimental, existing group only
sysflow process freeze com.example.app         # experimental: cgroup v2 leaf, am fallback
```

Protection is evaluated against the **current** process state: `pid ≤ 2`, kernel threads,
`uid < 10000`, a small package denylist and SysFlow itself are refused, and a package whose
expansion contains any protected PID is rejected as a whole. Nothing here writes `config.json`, so an
action cannot affect boot, drift or safe mode. See `docs/protocol.md` §3.25,
`docs/process-module-plan.md` and `docs/modules/process.md`.

## Storage module

`core/sysflow-modules/src/storage/` puts capacity, block-device geometry, I/O throughput, the two
queue tunables and maintenance verbs in one place. It participates in `snapshot` (capacity and
boot-cumulative counters are cheap); throughput needs an interval and is a verb + 5-minute history.

```sh
sysflow storage get --pretty                    # capacity, device, scheduler, read_ahead, IO totals
sysflow storage io --interval-ms 1000           # live read/write KB/s, IOPS, utilisation
sysflow storage usage --path /storage/emulated/0 --budget-ms 3000  # user-visible internal storage
sysflow storage usage --path /data --budget-ms 3000               # whole partition (includes the above)
sysflow storage usage --path / --budget-ms 3000                    # phone root: mounts as used/total
sysflow storage partitions                                        # every real mount, statvfs used/total
sysflow storage trim                            # fstrim -v /data (one-shot, not persisted)
sysflow storage set '{"storage.scheduler":"kyber"}' --persist
sysflow storage set '{"storage.read_ahead_kb":1024}' --persist
sysflow history storage --since 24h --step 1h
```

The tunables apply to the **physical** device behind `/data` (resolved through
`/sys/block/<dm>/slaves/`), because device-mapper nodes have no I/O scheduler. Values are validated
against the kernel's own option list before writing, read back, and dropped at boot if this kernel no
longer accepts them. `storage usage` is time-bounded and reports `partial:true` rather than freezing.
See `docs/protocol.md` §3.26, `docs/storage-module-plan.md` and `docs/modules/storage.md`.

## Adaptive control and enforcement

The CPU page links to a dedicated adaptive-management screen with smart load tiers, thermal
caps, battery conservation, PSI guard, live CPU/GPU utilization, thermal sensors and reset-to-
default controls. The module service starts a separate Rust daemon (default 10 s, configurable
2–120 s) for persisted-setting drift repair, history sampling and adaptive sampling. Neither APK nor
WebUI needs to remain alive. See `docs/modules/cpu.md` for the sysfs/path matrix and the
intentionally deferred process module.

## Clients

Protocol types are generated from Rust, never hand-written:

```sh
tools/gen-types.sh     # app/.../core/model/Protocol.kt (Kotlin models)
```

The generator (`core/sysflow-types`) also supports `--ts`; the module WebUI consumes the JSON
envelope directly, so no TypeScript model is generated today.

### WebUI (module rescue console)

Plain HTML + CSS + JavaScript (`module/webroot/`), calling KernelSU's injected `ksu.exec` directly —
no framework, no npm, no build step.

**The WebUI is not a second client.** It is a single-screen rescue console that exposes only status,
safe mode, reset, drift check, diagnostics and the log tail — the operations that must stay reachable
when the APK cannot be installed, updated or launched, or when a bad configuration stopped the device
from booting. Feature controls (CPU, battery, wakelock, memory, adaptive, screen-off, cpuset, events)
live in the Android APK only, and the console renders no events at all.

Keeping it this small is deliberate: full feature parity cost roughly 7 000 lines of parallel
TypeScript and four duplicated model/copy files that had to be kept in sync by hand, and it made the
module build depend on `npm ci`. The console is 527 lines in three files (295 JS, 160 CSS, 72 HTML),
ships as ~17 KB of static assets, and the module build needs only Rust, the NDK and `zip`. See
[docs/webui-console.md](docs/webui-console.md).

### Structured system events

The Root module is the event producer and source of truth. Producers can emit events for any
module ID using `sysflow event emit`; module IDs and categories are open strings so adding future
modules does not require an APK schema change. Events are atomically stored under
`/data/adb/sysflow/events/`, retained for 30 days / 5,000 records, and read via `sysflow event list`.
Persisted settings changes, module resets, safe-mode and service lifecycle transitions are already
recorded. Service-side producers continue to work when the APK is closed.

The APK imports the journal into Room (`sysflow-events.db`) while the dashboard is foregrounded.
Room provides indexed Flow queries, filters and a bounded 30-day / 5,000-event local cache; it is
a read model only. The WebUI does not read the journal — it is a rescue console — so the Root module
remains operable through KernelSU WebUI if the APK is unavailable. The APK does not infer or emit
events and does not run background monitoring.

### APK

Material 3 Expressive (migrated SoCFlow brand palette, typography, shapes and motion), Koin, libsu;
pure UI, no background service. The `SysFlowClient` calls the CLI envelope. Screens:

| screen | notes |
|---|---|
| 总览 Overview | device status cards, recent system activity, runtime status |
| 模块 Module hub | CPU, battery, wakelock, memory (others marked as planned) |
| CPU | core matrix, per-cluster range/governor, plus 自适应性能管理 / 熄屏管理 / 进程组 sub-pages |
| 电池 Battery | wavy charge meter, live electrical readings, 24 h curve, health, per-app ranking |
| 唤醒锁 Wakelock | 唤醒源 / 应用 / 持锁 tabs, AppOps switches, protected list |
| 内存 Memory | available memory hero, composition bar, swap/zRAM, PSI curve, per-app ranking, tunables |
| 系统事件 Events | Room-backed filterable, paginated history |
| 设置 + 诊断与日志 | boot/daemon configuration, safe mode, recovery, diagnostics |

```sh
ANDROID_HOME=$ANDROID_SDK_ROOT ./gradlew :app:assembleDebug
# app/build/outputs/apk/debug/app-debug.apk
```

## Build the module zip

```sh
./build-sysflow-module.sh
# dist/sysflow-module-v0.3.2.zip (+ .sha256)
```

The script installs `cargo-ndk` and the `aarch64-linux-android` target when missing, builds the
ARM64 release binary, checks the static WebUI assets and packages the zip. Equivalent manual steps:

```sh
cargo install cargo-ndk && rustup target add aarch64-linux-android
(cd core && cargo ndk -t arm64-v8a build --release -p sysflow-cli)
tools/pack-module.sh core/target/aarch64-linux-android/release/sysflow
```

The WebUI needs no build step, so the module build never touches Node or npm. The APK is built
separately.

## M3 Expressive theme contrast fix

The source includes a light/dark contrast correction for root content color, primary-container
adaptive-engine text, CPU hero wave accent, and overview icon badge foregrounds. See §10.6 of
[docs/development-plan.md](docs/development-plan.md) and the `ThemeContrastTest` unit test.

### Cpuset process groups

The Android CPU screen exposes cpuset as a dedicated **进程组** page matching the former SoCFlow card
workflow. WALT manual tuning entries are not exposed in the Android UI: the Rust WALT subsystem
remains internal because adaptive frequency caps coordinate with WALT state. Screen-off core
selection (`cpu.screen_off.cores`) is a separate feature and is not a process-group control.

The `caps` feature list does not publish a `cpuset` flag; the cpuset items appear in `cpu schema`
only when `/dev/cpuset` is present.

### Screen-off CPU limits

The screen-off profile is available from the CPU page as a secondary page (not a bottom-navigation
item). It supports per-policy discrete caps, an unlimited detent, selected core shutdown while the
display is off, status reporting, save/clear actions, SoCFlow-compatible discrete frequency steps
(including generated steps when the raw kernel table is unavailable), and automatic restoration
driven by the enforcement daemon's display watcher. See §3.11 of
[docs/protocol.md](docs/protocol.md) for the state and recovery contract.
