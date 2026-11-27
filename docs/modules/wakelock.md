# Wakelock module: kernel wakeup sources, AppOps blocking

Implementation: `core/sysflow-modules/src/wakelock/` (`mod.rs`, `appops.rs`, `policy.rs`, `read.rs`,
`state.rs`, `window.rs`, `debugfs.rs`, `power.rs`) and `core/sysflow-core/src/wakeup_sources.rs`
(parsers). Protocol: `docs/protocol.md` §3.18 (module), §3.19 (`wakelock held`), §3.20
(`wakelock apps`) and §3.21 (`wakelock wakeup`).

The module has two halves with different safety postures:

| half | items | writes | safety posture |
|---|---|---|---|
| read | `kernel_sources`, `ranked`, `summary` | none | telemetry, safe in `snapshot` |
| write | `blocked` | `cmd appops set <pkg> WAKE_LOCK ignore\|allow` | risk medium, persisted, replayed at boot, protection list |

Kernel wakeup-source blocking is deliberately **not** offered: the kernel publishes no portable
writable interface for a wakeup-source *name* (`/sys/.../power/wakeup` are device wakeups, and mapping
source names to device paths is guesswork). Shipping a switch that guesses would be worse than
shipping none.

## Why it is not part of the battery module

The domain is power management and the data sources overlap (`dumpsys batterystats` already feeds the
battery page's per-app ranking), but the module contracts differ:

| | battery | wakelock |
|---|---|---|
| writes | never | AppOps rules |
| config | none | `{pkg: blocked}` persisted |
| recovery paths | excluded on purpose | `drift()` + `sanitize_boot()` + `reset()` |

The battery module is the project's "cannot brick a device" module; folding a writable wake-lock
blocker into it would silently change that guarantee. Code is shared where it belongs (parsers in
core, `command`/`cache`/`dumpsys` helpers in core), not by merging modules.

## Read half

### Paths

| feature | paths |
|---|---|
| kernel wakeup sources | `/sys/kernel/debug/wakeup_sources` (9 counters, **milliseconds**) |
| legacy kernel wakeup sources | `/proc/wakelocks` (8 counters, **nanoseconds**) |
| userspace wake locks | `dumpsys power` → `Wake Locks:` block |

The two kernel formats are not interchangeable: the legacy table has no `event_count`, and its
`expire_count`/`wakeup_count` sit at different positions. `parse_kernel_sources` distinguishes them by
column count and converts times to milliseconds; `event_count` stays `0` for the legacy layout rather
than being filled with a number that means something else.

`active` comes from `active_since > 0`, not from `active_count`: the latter counts activations, so a
source that fired 200 times can be idle right now.

### Driver categories

Raw source names are unreadable and say nothing about apps, so every row carries a `category` — the
driver family the kernel module belongs to (`network`, `bluetooth`, `timer`, `touch`, `power`,
`subsystem`, …), classified by `classify_wakeup_source()` in `wakeup_sources.rs` with a table test
built from real rows of a Qualcomm/Samsung device.

The classification is deliberately coarse: the kernel records *hardware* activity, so "who did it" is
unknowable there — an app sending one packet lights up `rmnet`/IPA, an `AlarmManager` alarm lands in
`alarmtimer`, a notification over Bluetooth shows up in `hal_bluetooth_lock`. Per-app attribution is a
different data source (`batterystats`, §3.20), which is why the page has a separate 应用 segment.

Clients render the labels; an unknown identifier must fall back to a generic "其它驱动" rather than
showing the raw key, so an older APK against a newer module stays readable.

### Items

| item | type | notes |
|---|---|---|
| `wakelock.kernel_sources` | readonly | 24 sources max: active first, then by `total_ms` desc |
| `wakelock.ranked` | readonly | 24 h window: `name`, `awake_ms`, `share` (of measured wake time), `wakeups`, `events`, 15 max |
| `wakelock.summary` | readonly | `available`, `known`, `active`, `window_s`, `window_reason`, `since_boot_s`, `updated_s`, `measured_ms`, `total_wakeups`, `ranking_available` |
| `wakelock.protected` | readonly | the protection policy as data, so clients stop keeping their own copy |

When neither kernel table is readable the module stays **supported** (blocking does not need debugfs)
and reports `summary.available = false` with empty kernel items and a `reason`.

Android no longer mounts `/sys/kernel/debug` by default, so the module **mounts debugfs on demand**
the first time the table is missing: `mount -t debugfs debugfs /sys/kernel/debug`, Android builds
only, retried at most hourly *after a failure* (a successful mount is not rate-limited because it may
live only in the calling process's mount namespace). Reads are lossy UTF-8, so a single malformed byte
in a source name cannot turn the whole table into "unsupported".

### The 24 hour window

Kernel counters are cumulative since boot, so "who woke us in the last day" needs a stored baseline.
`<data_dir>/state/wakelock.json` keeps `baseline_ts`, the baseline counters and the last observation.
The daemon's 300 s `sample()` advances it; between samples the ranking is computed by a read-only file
read, which is why `snapshot` can include the module without writing on every 2 s poll.

Decisions that matter:

- **Probe intervals, not history metrics.** Per-source cumulative time would otherwise become an
  unbounded set of history columns. History keeps only `known` / `active` / `top_ms` / `wakeups`.
- **Rolling forward beats resetting to zero.** When the window is older than 24 h, the baseline
  becomes the previous observation, so the first hours after a rollover are not thrown away.
- **A reboot restarts the window.** The state file survives a reboot while the counters do not, so a
  pre-boot baseline would clamp every delta to zero for the rest of the window — and the empty state
  was explained as "the device was idle", which was untrue. Detection uses the kernel boot id first,
  system uptime second (a wall-clock jump is not a reboot) and a widespread counter regression only
  when neither is readable. `window_s` is capped by the current uptime.
- **Backwards counters clamp to zero.** A single driver that re-registers must not produce a negative
  "time held", and must not throw the whole window away either.
- **Honest emptiness.** A fresh install has no ranking until the second sample (5 minutes), and
  `share` is relative to measured wake time, never to wall-clock hours. The page distinguishes four
  cases instead of one vague "no data": *still collecting* (fewer than two samples), *just rebooted*
  (the window legitimately restarted), *window too young*, and *genuinely idle* (a full window with no
  counter movement). Until the window has increments it lists the top sources by **cumulative** time
  since boot, clearly labelled, so the tab is not an empty screen on first run.
- **The share denominator covers every source.** Summing the truncated top-15 made each percentage
  too large on a real device with dozens of movers.
- **The state file is bounded and versioned.** At most 512 sources are tracked (pruned to the ones
  still present), and a v1 file still loads; the next sample writes v2 with the wakeup counters.

## Write half: `wakelock.blocked`

One app-scoped item whose value is a map, the protocol's shape for per-app settings:

```json
{"wakelock.blocked": {"com.example.app": true, "com.other.app": false}}
```

`true` means the app's `WAKE_LOCK` op is `ignore` (its wake locks are dropped), `false` restores the
AppOps **`default`** — not `allow`, which would leave a permanent explicit grant overriding the
app's manifest or an OEM policy. Only the packages named in the request are touched.

A second, persisted item is the user whitelist:

```json
{"wakelock.exempt": {"com.example.chat": true}}
```

`true` means "never block this app". Exempting implies releasing an existing block (the implied
`blocked: false` is reported in `applied`, so it persists); `sanitize_boot` drops stored blocks for
exempt apps, and `drift` releases one that appears later. It is configuration only, so clients read
the persisted intent from `snapshot.settings` rather than a live read.

### Order of operations

1. **Validate everything first**: package-name shape and the protection list. A bad entry rejects the
   whole request before any command runs.
2. **Snapshot the current ignore set** (`cmd appops query-op WAKE_LOCK ignore`, one call).
3. **Write each change**; packages already in the requested state are skipped.
4. **On failure, roll back** the packages already written to their previous mode, then return the
   error. A half-applied request would leave the user with half of the apps blocked.
5. **Read back once** and report `applied` per package. A ROM that silently keeps the old mode shows
   up as `mismatch`, not as success.

### Protection list

The list is published as `wakelock.protected` so the APK renders the same rule the module enforces
instead of a copy that can drift. Blocking is refused with `invalid_arg` for:

`android`, `com.android.systemui`, `com.android.phone`, `com.android.server.telecom`,
`com.android.dialer`, `com.google.android.dialer`, `com.android.mms`, `com.android.messaging`,
`com.android.providers.telephony`, `com.android.settings`, `io.sysflow`.

Unblocking them is allowed (a user may want to undo an OEM default), and `reset` skips them so the
module never flips a system default it did not set. The APK mirrors this list only to disable the
switch; the module remains the authority.

### Persistence and boot

`--persist` stores the map under `config.modules.wakelock` (objects merge entry by entry, so blocking
a second app keeps the first). Persistence is **per entry**: if a ROM rejects one package, the ones
that did apply are still stored instead of the whole request being dropped. `apply-boot` runs the
settings through `sanitize_boot`, which drops packages that were uninstalled, became protected, or
were whitelisted while the rule was stored — a dropped rule beats a boot that retries a package
forever. The daemon's `drift()` compares the persisted map with a single `query-op` and re-applies on
mismatch.

`reset` releases exactly the packages this module persisted as blocked, then the registry restores its
defaults snapshot. It deliberately does **not** release "everything that is ignored": a rule the user
set through another tool (or an OEM default) is not the module's to revert, and without the persisted
intent there is no safe set to undo — so `reset` with no settings does nothing.

If a write fails and undoing an earlier write also fails, the returned error names the packages that
need manual repair instead of pretending the rollback succeeded.

### Opt-in reads

Answering `wakelock.blocked` costs a binder call, so it is **never part of `snapshot`** (which runs
every couple of seconds). Clients name it explicitly:

```sh
sysflow wakelock get --keys wakelock.blocked
```

The persisted intent is still visible cheaply in `snapshot.settings.wakelock`.

## `sysflow wakelock held`

Userspace locks live in `dumpsys power`, which is a Java service call: hundreds of milliseconds, and
unbounded if the system server is wedged. So it is a verb, not a schema item:

```sh
sysflow wakelock held            # cached for 60 s
sysflow wakelock held --refresh  # force a new dump
```

Each entry carries `kind`, `tag`, `held_ms`, `uid`, `pid`, `package` and `owner`. A failing `dumpsys`
returns the `io` error code instead of an empty list, so a SELinux denial is not shown as "nothing is
held".

Attribution formats differ by vendor, and both are supported:

```text
AOSP     PARTIAL_WAKE_LOCK 'AudioMix' ACQ=-1m2s (uid=1041, pid=1234, pkg=com.android.systemui)
Samsung  PARTIAL_WAKE_LOCK 'AudioMix' ACQ=-145ms (uid=1041 isFrozen=false
         ws=WorkSource{ chains=WorkChain{(10390), (1041)}} lock=4c24045)
```

The parser scans to the matching close parenthesis (a plain `find(')')` stops inside
`WorkChain{(10390), (1041)}` and loses `pid`/`pkg`) and splits the attribution on commas *or*
whitespace. When the layout only names a UID, `owner` is resolved through
`cmd package list packages -U` (one call, only when needed), falling back to the AID table and then
`UID <n>`. A shared UID is deliberately not mapped to one of its packages.

## APK page

`app/.../feature/wakelock/WakelockScreen.kt` is its **own destination**, a peer of the battery page —
the two modules stay separate in the UI as well as in the protocol. The page is Material 3 Expressive
throughout: a connected `ToggleButton` group at the top switches three peer views (a drawer would hide
content on a phone and add a tap to every switch), and each view is one `LazyColumn` that opens with a
tonal hero surface and continues with connected list tiles (`segmentedShape`), emphasized type,
Expressive-shaped badges, wavy/loading indicators and `animateItem` row motion.

| tab | hero | list | data |
|---|---|---|---|
| 唤醒源 | active / known sources, wavy meter, window and last-sample pills | last wakeup card, 24 h trend chart, rolling ranking grouped by driver family, share bars | `get wakelock` + `wakelock wakeup` + `history wakelock` |
| 应用 | total app wake-lock time, blocked / protected counts, cache pill | one list merging BatteryStats wake-lock time with the AppOps switch, plus a 保护名单 section | `wakelock apps` + `get --keys wakelock.blocked` + `snapshot.settings` |
| 持锁 | number of userspace locks held right now | held locks with owner icon, tag, kind, held time and a one-tap 拦截 action | `wakelock held` |

Files: `WakelockScreen.kt` (host + tab group), `WakelockSourcesTab.kt` (ranking, metric toggle, last
wakeup), `WakelockTrend.kt` (trend card + Canvas chart), `WakelockAppsTab.kt` (apps, whitelist,
release-all), `WakelockHeldTab.kt`, `WakelockAppPicker.kt` (block/whitelist sheet in two modes) and
`WakelockComponents.kt` (hero, notice, empty state, connected tile, share bar shared by the tabs).

Two behaviours worth knowing:

- **时长/次数 toggle.** The ranking is sorted by time by default; the toggle re-groups by wakeup
  count. It is only offered when the kernel table actually reports counts (the legacy
  `/proc/wakelocks` layout has no wakeup counter, and a toggle that always shows zero would lie).
- **Blocking from the held list.** The 持锁 tab's rows carry a 拦截 action for the app holding the
  lock — the moment the user can actually see who is responsible. Protected and whitelisted apps
  show nothing to tap, so the write can never be refused by a switch the UI should have disabled.

The 应用 table is the interesting merge: the ranking tells *how long* an app held locks and AppOps
tells *whether it is blocked*, but the two lists are independent — a blocked app that did not hold a
lock this cycle has no time and still has to appear (shown as `—`), which is how a user finds a rule
to release. Rows are sorted by time, then by name; protected apps show their reason and a disabled
switch. Blocking a new app opens a bottom sheet listing installed apps with search and filter chips
(全部 / 已拦截 / 用户应用 / 系统应用); installed apps are resolved off the main thread. Writes use
`set --persist` so a rule survives a reboot.

The same sheet opens in **whitelist mode** (选择拦截应用 / 保护应用), and the 保护名单 section lists
the user's exemptions with a one-tap removal. **全部恢复** releases every rule the module persisted
after a confirmation dialog; it is the phase-2 acceptance item "拦截规则…可一键恢复" that used to be
CLI-only. Whitelisting an app that is currently blocked sends both items in one request, because an
exemption that left the block behind would make the list a lie.

## `sysflow wakelock apps`

Per-app wake-lock time since the last charge, from the `All partial wake locks:` section of
`dumpsys batterystats --charged`:

```sh
sysflow wakelock apps --limit 20          # cached for 60 s
sysflow wakelock apps --refresh           # force a new dump
```

Rows are grouped by UID; each entry keeps its three longest tags. The dump is streamed and the child
is killed as soon as the section ends, because the section sits hundreds of lines into a 4 000+ line
dump. UID labels come from the package list, then the AID table, then `UID <n>` — never null.
`total_ms` always covers every measured row, so `--limit` only trims what is displayed.

The parser lives in `core/sysflow-core/src/batterystats.rs` (`WakeLockScanner`), next to the power
attribution parser, because both read the same dump and the process module will want the same
streaming helper.

## Long-held lock watchdog

`sample()` (every 5 minutes) reports locks that have been held far longer than any normal
operation — 30 minutes — as a `wakelock_long_held` event:

| kind | source | metadata |
|---|---|---|
| `kernel_source` | a wakeup source whose `active_since_ms` is over the threshold | `name`, `held_ms` |
| `userspace_lock` | a `PARTIAL_WAKE_LOCK` from `dumpsys power` over the threshold | `package`, `tag`, `held_ms` |

Each lock is reported at most once per 6 hours (`dedupe`), so a genuinely stuck app does not flood
the journal. The event system is the same one the CPU module uses; the APK renders the copy from
`EventCopy.kt`. The watchdog is **informational**: it never blocks anything, and it deliberately
ignores the user whitelist — "never block this app" is a policy about blocking, not about a lock that
is already pinning the device. Categories other than `PARTIAL_WAKE_LOCK` are skipped: a screen wake
lock is supposed to keep the device awake.

A failed `dumpsys` is skipped silently (no warning every five minutes on a ROM without it), and the
kernel half still runs when debugfs is unavailable.

## Last wakeup (`sysflow wakelock wakeup`)

```sh
sysflow wakelock wakeup            # cached for 60 s
sysflow wakelock wakeup --refresh
```

`dumpsys power` is parsed for the last wakeup reason. Builds differ — AOSP prints
`Last wakeup reason: …`, vendor builds print `mLastWakeupReason=…`, several print neither — so the
verb answers `available: false` with a `note` instead of inventing an answer, and `mLastWakeTime`
(a different field) is not mistaken for the reason. `age_ms` is derived from
`mLastWakeupUptimeMs` against `/proc/uptime` and dropped when missing or inconsistent.

## Trend

`sample()` records `known` / `active` / `top_ms` / `wakeups` into `history/wakelock.jsonl`, and the
唤醒源 tab draws the 24 h curve from `sysflow history wakelock` (30-minute buckets). The chart follows
the same 时长/次数 toggle as the ranking, and says whether the series is empty because the daemon is
stopped or because it has not sampled twice yet.

## Not implemented yet

- **Battery page integration.** The battery sheet still ranks *power* (mAh); the wake-time ranking is
  on the wake-lock page instead. Merging them into one sheet with two tabs is a UI change, not a data
  one.
- **Kernel wakeup-source blocking** — see the note at the top.
- **`wakelock.exempt` has no read-back item.** It is configuration, so its persisted intent is read
  from `snapshot.settings`; a live `get --keys wakelock.exempt` returns nothing.
- **Doze/`deviceidle` whitelist and `dumpsys alarm` attribution** are out of scope: they are about
  wakeup *causes* the wake-lock module does not own, and belong in their own module.

Deliberately **not** done, for the record:

- **Mounting from `probe()`.** `probe()` stays side-effect free (a `snapshot` polls it every couple of
  seconds); the mount happens in `collect()`/`sample()`, where an unreadable table is actually being
  reported. A mount that succeeds while the table stays unreadable is now recorded on the same hourly
  gate, so a 2 s poll cannot fork `mount` forever.
