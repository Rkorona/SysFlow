# Process module: paths, items and device notes

Implementation: `core/sysflow-modules/src/process/` (planned: `mod.rs`, `scan.rs`, `foreground.rs`,
`actions.rs`, `tests.rs`). Protocol: `docs/protocol.md` §3.25. Plan and decisions:
`docs/process-module-plan.md`.

> **Status: Steps 1–5 implemented on the host (device probe 2026-11-25); device acceptance is
> pending.** The read-only verbs (`list` / `get`), the protection gate and the one-shot actions
> (`stop` / `oom` / `renice` / `cpuset` / `freeze` / `unfreeze`) are covered by host tests against a
> fake `/proc`; the APK page code is written against the same protocol (the APK is built by the
> device owner). The write path has **not** been exercised on the device yet — that is checklist
> item 1 below.

This document is updated at the end of **every** step with the measured value or the reason a path
was dropped. The read-only half is safe (no writes); the action half is one-shot and never touches
`config.json` — see Rollback and safety.

## Paths

Probe: `sysflow process probe --pretty` on the device, saved to `/sdcard/process-probe.json`.
Reference device for earlier steps: Samsung S25 (same device as `docs/modules/mem.md`).

| feature | path | fallback if missing | observed (S25, 2026-11-25) |
|---|---|---|---|
| process enumeration | `readdir /proc/<pid>` | module `unsupported` | **1198** numeric entries, **0** unreadable, first pass **28 ms** |
| state, ppid, threads, utime/stime, starttime, nice, policy | `/proc/<pid>/stat` | process skipped | readable for every process; `comm` is truncated to 15 chars (`.wallpaper.live`); the sysflow shell itself showed `nice=-10` |
| UID, RSS, comm, threads | `/proc/<pid>/status` | process skipped; `rss_kb` omitted if `VmRSS` absent | readable; `Uid:` carries 4 columns; `VmRSS` present for apps and kernel threads; `Kthread:` marks kernel threads |
| process name | `/proc/<pid>/cmdline` (NUL-separated) | `status` `Name:` → `pid N` | app processes carry the package name in `cmdline[0]`; `kthreadd` is empty |
| oom_score_adj | `/proc/<pid>/oom_score_adj` | field omitted | present for every sampled pid: init `-1000`, background wallpaper `100`, apps `0..100` |
| cgroup (v1 controllers + v2 unified) | `/proc/<pid>/cgroup` | `cgroup` / `cpuset` omitted | v1: `freezer/cpuset/cpuacct/cpu/blkio`; v2: `0::/apps/uid_10157/pid_12225` for apps, `0::/` for init |
| PSS / SwapPss (detail only) | `/proc/<pid>/smaps_rollup` | `pss_kb` / `swap_pss_kb` omitted | not probed yet (Step 2 detail path) |
| UID → package | `/data/system/packages.list` → `cmd package list packages -U` | `package:null`, label falls back to AID / `UID <n>` | **readable**, **698** lines, **10** columns; first line `com.google.android.networkstack.tethering 1073 …` |
| foreground activity | `dumpsys activity activities` (`topResumedActivity` → `mResumedActivity`, 3 s timeout) | `foreground.available:false` | `topResumedActivity=ActivityRecord{172310012 u0 com.server.auditor.ssh.client/.ssh.terminal.TerminalActivity t2919}`, parsed package ok, **23 ms** |
| `CLK_TCK` | `/proc/self/auxv` `AT_CLKTCK` (17) | 100 | `getconf CLK_TCK` = **100**; **`sysconf(2)` returned 2048** — bionic numbering differs from glibc, so sysconf must not be used |
| page size | not needed (`VmRSS` is kB) | — | `sysconf(30)` returned **1** — another reason to avoid sysconf |
| cpuset groups | `/dev/cpuset/<dir>/tasks` | `cpuset` not advertised | cgroup **v1**, **13** group dirs with `tasks`, all owner-writable: `top-app 0-7`, `foreground 0-5`, `background 0-1,4-5`, `restricted 0-7`, … |
| freezer | `/sys/fs/cgroup/apps/uid_<uid>/pid_<pid>/cgroup.freeze` → `am freeze` | action hidden | no `/sys/fs/cgroup/uid_*/pid_*`; v2 path is `/apps/uid_…/pid_…`, so re-probe under `apps/`; `am help` lists `freeze` |
| force-stop | `am force-stop --user <user> <package>` | `unsupported` | `/system/bin/am` present |
| renice | `setpriority(2)` primary; `renice` fallback | `renice` not advertised | toybox 0.8.13: `usage: renice [-gpu] -n INCREMENT ID...` — **`-n` is an increment**, so the target nice cannot be passed directly |
| `hidepid` visibility | `/proc` mount options / unreadable peers | `skipped` counts; partial list | no restriction as root: 1198/1198 readable |
| SELinux | `getenforce`, denied reads during scan | `skipped`; `probe` records denials | `Enforcing`; no denied reads during the scan; writes not exercised yet |

All `/proc` and `/sys` reads go through `Sysfs::resolve`, so the host test tree
(`SYSFLOW_SYSFS_ROOT`) keeps `proc/` and `dev/` next to `sys/`.

## Observed values worth designing around (S25, 2026-11-25)

1. **1198 processes is the normal case.** First pass 28 ms, foreground `dumpsys` 23 ms, the whole
   probe 127 ms. The two-pass `list` budget (< 600 ms) has plenty of headroom; do not pre-optimize.
2. **`sysconf` is a trap on bionic.** `_SC_CLK_TCK` returned 2048 and `_SC_PAGESIZE` returned 1,
   while `getconf CLK_TCK` said 100. Read `AT_CLKTCK` (17) from `/proc/self/auxv`; do not link the
   glibc constant values.
3. **The freezer lives under `apps/`.** The unified cgroup path is `/apps/uid_10157/pid_12225`, which
   is why the first probe reported "no freezer" — it only looked under `/sys/fs/cgroup/uid_*`.
   `am help` does list `freeze`/`unfreeze`, so there are two candidate mechanisms to test.
4. **`renice -n` is an increment** (toybox 0.8.13), not an absolute nice. Use `setpriority(2)` for
   "set to value"; only fall back to `renice -n (target − current)`.
5. **cgroup v1 cpuset is writable**: 13 groups, `tasks` owner-writable. A per-app move is feasible;
   expect AMS to move the process back on its next state change.
6. **`oom_score_adj` is readable for every pid** (init `-1000`, a cached wallpaper `100`). Writing is
   the high-risk part and stays behind the experiment gate.
7. **Foreground** is `topResumedActivity=ActivityRecord{… u0 <pkg>/<activity> …}`; the parser handles
   both this and `mResumedActivity:`.
8. **`comm` is truncated to 15 chars** (`.wallpaper.live`), so names must come from `cmdline` first.
9. **SELinux is Enforcing** and `/proc` is fully readable in the su domain; write paths are unverified.
10. **`packages.list`** is 698 lines / 10 columns, so the `batterystats::read_package_uids` reuse holds.

## Running the probe (Step 1)

`sysflow process probe` is a hidden developer verb (not in the protocol; output may change). It
prints the raw readings (`proc`, `libc`, `samples`, `foreground`, `packages_list`, `cpuset`,
`freezer`, `commands`) plus wall-clock timings; the feature set those readings imply is what `caps`
advertises for the module.

```sh
/data/adb/modules/sysflow/bin/sysflow process probe --pretty > /sdcard/process-probe.json
```

Required probe output (checked off as it lands):

- [x] `/proc`: total numeric entries vs readable; first unreadable pid with errno — 1198/1198, none
- [x] `CLK_TCK` / page size — `getconf` 100; `sysconf` values are wrong (see above)
- [x] one full `/proc/<pid>/stat` and `status` for: an app process, `system_server`, a kernel thread —
      the probe sampled sysflow itself, pid 1, pid 2, and the first app pid (`.wallpaper.live`, uid 10157)
- [x] `dumpsys activity activities` foreground lines (raw, both `topResumedActivity` and
      `mResumedActivity` variants if present) + parsed package — `topResumedActivity` only, parsed ok
- [x] `packages.list` readability, line count, column count — 698 lines, 10 columns
- [x] `/dev/cpuset` group list and `tasks` writability — 13 groups, all owner-writable
- [x] cgroup v2 freezer path existence; `am help | grep freeze` — `apps/` path to re-probe; `am freeze` exists
- [x] `renice` / `chrt` / `am` presence and version lines — all present; renice is toybox with increment `-n`
- [x] scan timing for N processes (first pass, second pass, `get`, foreground dumpsys) — 28 ms / (Step 2) / (Step 2) / 23 ms

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

## Device checklist (Step 6)

The host tests cannot exercise SELinux, AMS reactions or the real freezer leaf. These are the items
that need a device run; each one names the command and what to send back.

1. **Write paths (the only unverified half of the module).**
   - `sysflow process oom <test-app> 500` then `cat /proc/<pid>/oom_score_adj` → expect `500`.
   - `sysflow process renice <test-app> -5` then `awk '{print $19}' /proc/<pid>/stat` → expect `-5`.
   - `sysflow process cpuset <test-app> top-app`, then `grep cpuset /proc/<pid>/cgroup` and re-run
     `sysflow process list --query <test-app>` → the `cpuset` field should flip immediately; check
     whether AMS moves it back after a few seconds.
   - `sysflow process freeze <test-app>` then `cat /sys/fs/cgroup/apps/uid_<uid>/pid_<pid>/cgroup.freeze`
     → expect `1`; `unfreeze` → `0`. If the leaf is missing, confirm the `am freeze` fallback works.
   - `sysflow process stop <disposable-app>` → the app should be gone from `process list`.
   - Try `sysflow process oom com.android.systemui 0` → expect `invalid_arg` with `protected:true`.
2. **Freezer leaf location.** The S25 probe looked only under `/sys/fs/cgroup/uid_*` and reported
   "no freezer"; the unified cgroup path shows `/apps/uid_…/pid_…`. Re-run the (updated) probe and
   send `freezer` — it now scans both `/sys/fs/cgroup/apps` and `/sys/fs/cgroup`.
3. **`renice` mechanism.** Report whether `setpriority(2)` succeeds as root or the toybox fallback
   was used (`mechanism` in the response) — SELinux may deny the syscall on some ROMs.
4. **Timing with the real load**: `time sysflow process list --limit 50 --sample-ms 300` ×3 and
   `time sysflow process get <pid>`; send the medians.
5. **Sorting cross-check**: `sysflow process list --sort rss --limit 10` vs `dumpsys meminfo`, and
   `--sort cpu` vs `top -b -n 1`.
6. **Multi-user**: if a work profile exists, check that a UID like `10xxxxx` shows `user:10` and a
   package label with the user suffix.
7. **APK page** (owner build): 2 s polling stops when the page is backgrounded, the foreground card
   matches the resumed activity, protected rows disable every action, and the experimental switch
   reveals OOM / cpuset / freeze only when `caps.features` contains them.

## Measured costs

Filled per step; do not guess.

| path | target | measured |
|---|---|---|
| scan 1198 processes (pass 1) | < 60 ms | **28 ms** (device probe 2026-11-25) |
| pass 2 (stat only, 1198) | < 25 ms | 待测 (host tests use a small fixture) |
| `list` cold / no-foreground | < 600 ms / < 250 ms | 待测 |
| `get` (one process) | < 50 ms + 200 ms sample | 待测 |
| foreground dumpsys (cold / cached) | < 400 ms / 0 | **23 ms** cold (device probe) |

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

Implemented as unit tests beside each module (`model.rs`, `scan.rs`, `actions.rs`, `foreground.rs`,
`packages.rs`) rather than one giant `tests.rs`: a fake root holds
`proc/<pid>/{stat,status,cmdline,oom_score_adj,cgroup,smaps_rollup}`,
`dev/cpuset/<group>/tasks`, `sys/fs/cgroup/apps/.../cgroup.freeze` and
`data/system/packages.list`. Fixture cases are listed in `docs/process-module-plan.md`〈测试计划〉;
`cargo test -p sysflow-modules process` is the focused run.
