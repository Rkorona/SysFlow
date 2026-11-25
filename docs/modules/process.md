# Process module: paths, items and device notes

Implementation: `core/sysflow-modules/src/process/` (planned: `mod.rs`, `scan.rs`, `foreground.rs`,
`actions.rs`, `tests.rs`). Protocol: `docs/protocol.md` §3.25. Plan and decisions:
`docs/process-module-plan.md`.

> **Status: Step 0 (contract frozen) and the Step 1 probe verb implemented (`sysflow process probe`,
> host-tested); list/get/actions are not written yet.** This matrix is a **checklist**, not a
> measurement: every `待测` cell is filled by running the probe on the device, and nothing here may be
> cited as device-verified until it is. The S25 columns are left empty on purpose — copying the
> memory module's observed values would be inventing data.

This document is updated at the end of **every** step with the measured value or the reason a path
was dropped. The read-only half is safe (no writes); the action half is one-shot and never touches
`config.json` — see Rollback and safety.

## Paths

Probe: `sysflow process probe --pretty` on the device, saved to `/sdcard/process-probe.json`.
Reference device for earlier steps: Samsung S25 (same device as `docs/modules/mem.md`).

| feature | path | fallback if missing | observed |
|---|---|---|---|
| process enumeration | `readdir /proc/<pid>` | module `unsupported` | 待测 |
| state, ppid, threads, utime/stime, starttime, nice, policy | `/proc/<pid>/stat` | process skipped | 待测 |
| UID, RSS, comm, threads | `/proc/<pid>/status` | process skipped; `rss_kb` omitted if `VmRSS` absent | 待测 |
| process name | `/proc/<pid>/cmdline` (NUL-separated) | `status` `Name:` → `pid N` | 待测 |
| oom_score_adj | `/proc/<pid>/oom_score_adj` | field omitted | 待测 |
| cgroup (v2 unified / v1 cpuset) | `/proc/<pid>/cgroup` | `cgroup` / `cpuset` omitted | 待测 |
| PSS / SwapPss (detail only) | `/proc/<pid>/smaps_rollup` | `pss_kb` / `swap_pss_kb` omitted | 待测 |
| UID → package | `/data/system/packages.list` → `cmd package list packages -U` | `package:null`, label falls back to AID / `UID <n>` | 待测 |
| foreground activity | `dumpsys activity activities` (`topResumedActivity` → `mResumedActivity`, 3 s timeout) | `foreground.available:false` | 待测 |
| `CLK_TCK` | `sysconf(_SC_CLK_TCK)` via `extern "C"` | 100 | 待测 |
| cpuset groups | `/dev/cpuset/<group>/{cpus,tasks}` | `actions.cpuset` not advertised | 待测 |
| freezer (cgroup v2) | `/sys/fs/cgroup/uid_<uid>/pid_<pid>/cgroup.freeze` | `am freeze`/`am unfreeze` if present, else action hidden | 待测 |
| force-stop | `am force-stop --user <user> <package>` | `unsupported` | 待测 |
| renice | `renice -n <nice> -p <pid>` (toybox) | `actions.renice` not advertised | 待测 |
| `hidepid` visibility | `/proc` mount options / unreadable peers | `skipped` counts; partial list | 待测 |
| SELinux | `getenforce`, denied reads during scan | `skipped`; `probe` records denials | 待测 |

All `/proc` and `/sys` reads go through `Sysfs::resolve`, so the host test tree
(`SYSFLOW_SYSFS_ROOT`) keeps `proc/` and `dev/` next to `sys/`.

## Running the probe (Step 1)

`sysflow process probe` is a hidden developer verb (not in the protocol; output may change). It
prints the raw readings (`proc`, `libc`, `samples`, `foreground`, `packages_list`, `cpuset`,
`freezer`, `commands`) plus wall-clock timings; the feature set those readings imply is what `caps`
advertises for the module.

```sh
/data/adb/modules/sysflow/bin/sysflow process probe --pretty > /sdcard/process-probe.json
```

Required probe output (checked off as it lands):

- [ ] `/proc`: total numeric entries vs readable; first unreadable pid with errno
- [ ] `CLK_TCK` / page size
- [ ] one full `/proc/<pid>/stat` and `status` for: an app process, `system_server`, a kernel thread
- [ ] `dumpsys activity activities` foreground lines (raw, both `topResumedActivity` and
      `mResumedActivity` variants if present) + parsed package
- [ ] `packages.list` readability, line count, column count
- [ ] `/dev/cpuset` group list and `tasks` writability
- [ ] cgroup v2 freezer path existence; `am help | grep freeze`
- [ ] `renice` / `chrt` / `am` presence and version lines
- [ ] scan timing for N processes (first pass, second pass, `get`, foreground dumpsys)

## Parsing rules (fixed now, they do not need a device)

- **`/proc/<pid>/stat`** — `comm` sits in parentheses and may contain spaces and `)`; parse from the
  **last** `)` in the line, then split the tail on whitespace. Fields are numbered from 1 with
  `pid` = 1: `comm` = 2, `state` = 3, `ppid` = 4, `utime` = 14, `stime` = 15, `threads` = 20,
  `starttime` = 22, `priority` = 18, `nice` = 19, `rt_priority` = 40, `policy` = 41.
- **CPU%** = `(Δutime + Δstime) / (Δseconds × CLK_TCK) × 100`, **per-core** (0–100 × cores); a
  process busy on four cores may read 400.
- **`/proc/<pid>/status`** — `Uid:` has four numbers (real, effective, saved, fs); use the first.
  `VmRSS` and `Threads` are in kB / count. A missing `VmRSS` omits `rss_kb` rather than writing 0.
- **`cmdline`** is NUL-separated; the first token is the app package for zygote children, empty for
  kernel threads.
- **UID math** — `app_id = uid % 100000`, `user = uid / 100000`; package lookup uses `app_id`.
- **Identity** — `(pid, starttime)`; a second pass that sees a different `starttime` marks
  `cpu_pct:null` instead of attributing another process's ticks.

## Units

| quantity | unit |
|---|---|
| CPU usage | percent of one core (one decimal) |
| CPU ticks | clock ticks (`CLK_TCK`) |
| memory | kB (`VmRSS`, `Pss`, `SwapPss`) |
| time windows | ms (`sample_ms`) |
| timestamps | Unix seconds (`generated_at`) |

## Protection

Hardcoded in Step 2/3 (`docs/process-module-plan.md`〈保护名单〉): `pid ≤ 2`, kernel threads,
`uid < 10000`, the package denylist (`com.android.systemui`, `com.android.phone`,
`com.android.server.telecom`, `com.android.bluetooth`, `com.android.nfc`, `android`), and SysFlow
itself. A package action that expands to any protected PID is rejected as a whole.

## Measured costs

Filled per step; do not guess.

| path | target | measured |
|---|---|---|
| scan 800 processes (pass 1) | < 60 ms | 待测 |
| pass 2 (stat only, 800) | < 25 ms | 待测 |
| `list` cold / no-foreground | < 600 ms / < 250 ms | 待测 |
| `get` (one process) | < 50 ms | 待测 |
| foreground dumpsys (cold / cached) | < 400 ms / 0 | 待测 |

## Degradation reasons

Filled by the step that first sees the condition, with the exact errno / SELinux denial when known.

| condition | symptom in output | first seen |
|---|---|---|
| `/proc` unreadable | `available:false` + `reason` | 待测 |
| single process unreadable | `skipped++`, others listed | 待测 |
| `packages.list` unreadable | `package:null`, AID label | 待测 |
| `dumpsys activity` timeout | `foreground.available:false` | 待测 |
| `smaps_rollup` denied | `pss_kb` omitted | 待测 |
| no freezer / cpuset | respective feature absent from `caps.features` | 待测 |

## Rollback and safety

- Steps 0–2 are read-only; removing the module registration is a complete rollback.
- Actions (Steps 3–4) never write `config.json`, so they cannot affect boot, drift or safe mode.
  Rollback = remove the verb branch + registration; a stopped app restarts when opened, an adjusted
  `oom_score_adj` / nice / cpuset is reset by AMS on the next state change, a frozen process is
  thawed by `unfreeze` or a reboot.
- No action is allowed on a protected target; verification happens against the **current** process
  state, not the list snapshot the UI is showing.

## Host test tree

Planned (Step 2): `core/sysflow-modules/src/process/tests.rs` with a fake root holding
`proc/<pid>/{stat,status,cmdline,oom_score_adj,cgroup}`, `dev/cpuset/<group>/tasks`,
`sys/fs/cgroup/.../cgroup.freeze` and `data/system/packages.list`. Fixture cases are listed in
`docs/process-module-plan.md`〈测试计划〉.
