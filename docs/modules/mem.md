# Memory module: paths, items and device notes

Implementation: `core/sysflow-modules/src/mem/` (`mod.rs`, `probe.rs`, `tunables.rs`, `history.rs`,
`tests.rs`). Protocol: `docs/protocol.md` §3.22 (module), §3.23 (`mem apps`) and §3.24
(`mem breakdown`). Plan and decisions: `docs/mem-module-plan.md`.

> **Status: M3a (telemetry) and M3b (tunables) implemented; Step 9 deferred.** The path matrix below
> is measured, not intended: every row was filled by a device probe on 2026-11-24 (Samsung S25,
> kernel `6.6.127-android15-8-…`, uptime ≥ 29.6 h) and the remaining `—`/`待补` entries are the
> open items, not placeholders. Device timings, the event-noise thresholds and the 24 h
> `swappiness` observation are still unverified; nothing here claims real-device acceptance.

This document is updated at the end of **every** step with the measured value or the reason a path was
dropped. M3a (telemetry) is read-only; the tunables (M3b) are `mem.swappiness` and
`mem.vfs_cache_pressure` — see Rollback and the Tunables section.

## Paths

Observed column: Samsung S25, kernel `6.6.127-android15-8-…-4k`, probe of 2026-11-24 (uptime ≥ 29.6 h).

| feature | path | fallback if missing | observed on S25 |
|---|---|---|---|
| totals, available, cached, anon, shmem, slab, dirty | `/proc/meminfo` | module `unsupported` | ok; all 11 keys of interest present, including `MemAvailable`; `MemTotal` 11 379 692 kB |
| swap totals | `/proc/meminfo` `SwapTotal`/`SwapFree` | — | ok; 12 582 908 kB total, equal to the zram0 size |
| swap devices | `/proc/swaps` | totals only, no zram/other split | ok; exactly one device, `/dev/block/zram0` (priority −2); **no non-zram swap** |
| pressure (some/full) | `/proc/pressure/memory` | PSI items and series removed | ok; `some` and `full` lines both present |
| pressure, file present but unreadable | same path, read returns an error | treated as unavailable | not seen on this device (file reads fine) |
| zRAM geometry | `/sys/block/zram*/disksize` | zRAM items removed | one device, `zram0`, 12 GiB; `comp_algorithm` = `lzo [lzo-rle] lz4 zstd` (active: `lzo-rle`) |
| zRAM usage | `/sys/block/zram*/mm_stat` (first three columns) | report capacity only | ok; **9 columns** |
| reclaim counters | `/proc/vmstat` by **name** | that series removed | every counter of interest present; `allocstall_{dma32,movable,nomerge,normal,nosplit}`; `workingset_refault_{anon,file}`; `pgscan_direct` plus the unrelated `pgscan_direct_throttle` |
| process RSS/UID | `/proc/<pid>/status` | process skipped | not probed yet (Step 5) |
| process PSS / SwapPss | `/proc/<pid>/smaps_rollup` | process skipped, counted in `skipped` | not probed yet (Step 5) |
| UID → package | `/data/system/packages.list` (primary) or `cmd package list packages -U` | `UID <n>` | **readable by the module**; 695 lines; **10** whitespace-separated columns on the first line |
| low-RAM flag | `getprop ro.config.low_ram` | default thresholds | property **unset** (empty value) → treated as "not low-RAM" |
| kill records | `dmesg` · `logcat -b events -d` · `dumpsys activity exit-info` (each probed separately) | pressure proxy only, never guessed | `dmesg`: readable, 14 matches, **all lmkd service restarts, no kills**; `logcat -b events`: 69 `am_kill` lines, reasons include `empty #N` (AMS cached-process limit, **not** memory pressure); `exit-info`: 2.5 MB / 40 251 lines, 3 166 lines matching `reason=3 (LOW_MEMORY)` |
| ROM dirty limits | `/proc/sys/vm/dirty_bytes`, `dirty_background_bytes` | — | **set by the ROM**: 104 857 600 (100 MiB) and 26 214 400 (25 MiB); the ratio files read `0` |

All `/proc` and `/sys` reads go through `Sysfs::resolve`, so the host test tree
(`SYSFLOW_SYSFS_ROOT`) holds `proc/` next to `sys/`.

## Running the probe (Step 1)

`sysflow mem probe` is a hidden developer verb (not in the protocol, output may change). It prints the
raw readings of every source above plus the module's own decision (`decision.items`,
`decision.hidden` with a reason per hidden item), and, on a device, the shell-out sources under
`commands` (`getprop`, `getconf`, and match counts for `dmesg`, `logcat -b events`,
`dumpsys activity exit-info`; at most three sample lines each, under `last_matches` — the *last* lines in
the output, not necessarily the newest in time). `exit_info` also returns `last_match_context`: the ten
lines before and two after its last match, to show how a `LOW_MEMORY` record is laid out. `paths` lists
candidate dmabuf / GPU / MGLRU / zram-writeback files with their first line or first directory entries, and
`meminfo.values_kb` now holds **every** key of `/proc/meminfo`.

```sh
/data/adb/modules/sysflow/bin/sysflow mem probe --pretty > /sdcard/mem-probe.json
/data/adb/modules/sysflow/bin/sysflow mem schema --pretty
```

To find out whether the ROM rewrites the tunables over time (decides the drift `contested` rule), run
this for 24 h. Two things bite, both learned the hard way on S25:

* a loop started from an interactive shell dies with the session, so it has to be detached;
* **keep every command on one line.** A multi-line heredoc whose commands ended in `\` arrived truncated
  twice: the `done` was lost and the script died with `syntax error: unmatched 'while'` right after
  writing its header line. A 24 h run that produced two lines is worse than no run.

So write the file, syntax-check it, run it in the foreground for two iterations, then detach it.

```sh
cat > /data/local/tmp/vmwatch.sh << 'EOF'
#!/system/bin/sh
LOG=/data/local/tmp/vmwatch.log
while :; do
  echo "$(date +%s) $(cat /proc/sys/vm/swappiness) $(cat /proc/sys/vm/vfs_cache_pressure) $(cat /proc/sys/vm/dirty_ratio) $(cat /proc/sys/vm/dirty_background_ratio) $(cat /proc/sys/vm/min_free_kbytes) $(cat /proc/sys/vm/dirty_bytes) $(cat /proc/sys/vm/dirty_background_bytes)" >> "$LOG"
  sleep 60
done
EOF
chmod 755 /data/local/tmp/vmwatch.sh

sh -n /data/local/tmp/vmwatch.sh        # syntax check: must print nothing
rm -f /data/local/tmp/vmwatch.log
sh /data/local/tmp/vmwatch.sh           # Ctrl-C once two lines exist
tail -2 /data/local/tmp/vmwatch.log

setsid sh /data/local/tmp/vmwatch.sh >/dev/null 2>&1 &
sleep 3; ps -A | grep -c '[v]mwatch'    # must be >= 1
```

Use `setsid sh <script>`, not `setsid <script>`: the latter needs the executable bit, and a script
saved without it exits immediately with `EACCES` — silently, whenever stderr is redirected.

`/data/local/tmp` is `0771 shell:shell`, so an app uid (plain Termux without `su`) cannot write there,
and a silenced `2>/dev/null` turns that `EACCES` into "no file at all". Check `id` and
`echo t > /data/local/tmp/t` before blaming the script.

Afterwards, and to stop it:

```sh
awk '{k=$2" "$3" "$4" "$5" "$6" "$7" "$8"; if (k!=p) { if (p!="") printf "%s  %-46s %6.1f min\n", ts, p, ($1-ts)/60; p=k; ts=$1 } last=$1} END { if (p!="") printf "%s  %-46s %6.1f min (to last sample)\n", ts, p, (last-ts)/60 }' /data/local/tmp/vmwatch.log
pkill -f vmwatch.sh
```

Module behaviour (verified on the fake tree): `caps` lists `mem`; `schema` lists the readonly items
the tree supports; `snapshot` carries a `mem` section whose keys are **exactly** those items; `get`
returns them; `set` is rejected as read-only; `check` stays clean; `reset --all` reports an empty
restore for `mem`. A source the tree lacks removes its items from `schema` and `snapshot` alike, which
is the property that keeps a kernel without PSI from looking like "0 % pressure".

## Observed values worth designing around (S25)

| finding | value | consequence |
|---|---|---|
| kernel / page size | `6.6.127-android15-8-…-4k`, `getconf PAGESIZE` = 4096 | `swappiness` ceiling is 200; swap rates can use 4 kB but must still read the page size |
| `vm.swappiness` | **130** (not the AOSP 60) | the stored default for `reset` is whatever the ROM has (130 here); never hard-code 60 |
| `vfs_cache_pressure` / `min_free_kbytes` | 100 / 11 584 | — |
| dirty limits | bytes-based (100 MiB / 25 MiB), ratios read 0 | confirms the plan: `mem.dirty_ratio` / `mem.dirty_background_ratio` are hidden here; writing a ratio would clear the ROM's bytes setting |
| zRAM | 1 device, 12 GiB, `lzo-rle`, ratio 2.95 (orig 2.74 GiB → compr 0.93 GiB, 0.96 GiB resident), swap used 2.74 GiB of 12 GiB (23 %) | `other_kb` is 0 here; the "non-zram swap" branch stays as a defensive path only |
| zRAM writeback | `/sys/block/zram0/backing_dev` = `/dev/block/loop37`, `writeback_limit_enable` = 1 | RAM Plus is zram **plus writeback**: once pages go to flash, `orig_data_size` no longer equals swap usage, so swap usage comes from `/proc/swaps` only and `ratio` must be flagged while writeback is on |
| PSI since boot | total stall `some` 699.2 s, `full` 564.4 s over ≈ 30.1 h → mean ≈ 0.65 % / ≈ 0.52 % | a 3 % `full` trigger is ≈ 6× the long-run mean; still a placeholder until Step 3 |
| PSI at probe time | `full` avg10/60/300 = 3.17 / 2.77 / 0.89 %, `some` 3.72 / 5.00 / 1.79 % | **the probe inflates this** (`dumpsys exit-info` is 2.5 MB): 3.17 % at probe time vs 0.52 % long-run mean. Event thresholds must come from a quiet window |
| swap traffic since boot | `pswpin` 69.6 M pages ≈ 278 GB, `pswpout` 113.3 M pages ≈ 453 GB (≈ 6.8 MB/s averaged) | swap churn is large on this ROM; `swap_in_kb` / `swap_out_kb` will be informative |
| reclaim | `pgscan_direct` 31.9 M vs `pgscan_kswapd` 339.4 M (direct ≈ 8.6 %), `compact_stall` 43 065 | — |
| MGLRU | `/sys/kernel/mm/lru_gen/enabled` = `0x0003` (enabled), `min_ttl_ms` = 0 | reclaim is multi-gen LRU: `swappiness` is a weaker, less predictable lever here, so the UI must not promise results |
| ROM reclaim tuning | `watermark_scale_factor` = 72 (default 10), `page-cluster` = 0 (default 3) | the vendor already tuned watermarks and swap read-ahead; writing vm sysctls fights the ROM, which argues for keeping M3b to `swappiness` + `vfs_cache_pressure` |
| GPU / vendor memory | `GpuTotal` 407 392 kB, exactly equal to `/sys/class/kgsl/kgsl/page_alloc`; `RbinTotal` 409 600, `CmaTotal` 638 976, `DmaHeapPool` 22 528, `KgslShmemUsage` 363 444 | the composition bar can be completed from `/proc/meminfo` alone — no need to walk dmabuf buffers |
| dmabuf | `/sys/kernel/dmabuf/buffers` = 138 entries, readable; `/sys/kernel/dma_heap/total_pools_kb` is a single cheap read | per-buffer totals are redundant with `GpuTotal`; on demand only, never in `snapshot` |
| commit | `Committed_AS` 146 GiB vs `CommitLimit` 17 GiB | heavy overcommit: kills are lmkd's job, which caps what any sysctl tuning can achieve |
| kernel OOM | `oom_kill` = 0 since boot, while `exit-info` holds thousands of `LOW_MEMORY` records | on Android the kernel OOM counter is nearly useless as a pressure signal; `lmkd` is where kills happen |
| exit-info layout | `process=… reason=3 (LOW_MEMORY) subreason=0 status=0`; the **previous** line carries `timestamp=… pid=… packageUid=… user=…`, the **next** carries `importance=… pss=… rss=… state=…`; `pss=0.00` after death, `rss` survives | Step 4 parses these three lines; report `rss`, never `pss`, for a killed process |
| memory breakdown | MemTotal 10.85 GiB, used (= Total − Available) 6.28 GiB: anon 2.54, slab 0.82, GPU 0.39, CMA 0.61, Rbin 0.39, kernel stack + tables + percpu 0.35, vmalloc 0.34 → **≈ 1.8 GiB unexplained** (was ≈ 2.8 GB before every meminfo key was dumped) | the "other" block is mostly vendor pools; the earlier quarter-of-RAM gap was an artefact of only reading 11 keys |

## Parsing rules (fixed now, they do not need a device)

- **Missing is omitted**, never `0` and never `null`.
- **Counters are looked up by name**, never by line or column position, and summed over zones where
  the kernel splits them (`allocstall_*`).
- **`mm_stat` is read positionally but tolerantly**: only the first three columns are used
  (`orig_data_size`, `compr_data_size`, `mem_used_total`); extra columns on newer kernels are ignored.
- **PSI** values are `avg10=12.34` style floats; the module stores `round(value × 100)` as an integer.
  `total=` is microseconds and is only used for differences.
- **zRAM aggregation**: sizes are summed first, `ratio` is recomputed from the sums.
- **Page size** comes from the system, never hard-coded as 4 kB.

## Units

| reading | source unit | protocol |
|---|---|---|
| meminfo sizes | kB | `kB` |
| zRAM `disksize`, `mm_stat` | bytes | `kB` (÷ 1024) |
| PSI averages | percent, two decimals | `0.01%` integer |
| PSI `total` | µs | consumed internally |
| vmstat counters | pages / events | pages or count; swap rates converted to `kB` |

## Tunables (Step 8)

The write half is two sysctls, on purpose (protocol §3.22):

| item | path | range | why it is offered |
|---|---|---|---|
| `mem.swappiness` | `/proc/sys/vm/swappiness` | 0..100, or 0..200 from 5.8 | the one knob users actually ask for; the ceiling comes from `osrelease` |
| `mem.vfs_cache_pressure` | `/proc/sys/vm/vfs_cache_pressure` | 1..1000 | cheap to reason about, no data-loss risk |

Not offered, and why:

- **`dirty_*`** — S25 reports `dirty_bytes`/`dirty_background_bytes` with both ratios at `0`; writing a
  ratio would clear the byte setting and fight the ROM's own tuning.
- **`min_free_kbytes`** — a watermark; too high causes the OOM it is supposed to prevent, and the
  right value changes with the workload.
- **zRAM re-sizing** — `swapoff` pulls swapped pages back into RAM, which on a loaded phone means an
  immediate OOM storm (M3c, not this release).

Three behaviours worth knowing:

- **Validate all, then write.** A request that contains one invalid value writes nothing.
- **Contested settings.** If the same item drifts back 3 times inside 30 minutes, SysFlow stops
  re-applying it for 6 hours and emits `mem_setting_contested` once. S25 showed no rewrites in ~48
  minutes of sampling (including a doze gap), but other ROMs do this, and a silent 10-second fight is
  the wrong default.
- **Boot replay is clamped.** A value the current kernel cannot accept is dropped at boot.

The APK exposes both knobs as sliders in `feature/mem/MemTunables.kt` (可调参数 section). Ranges come from
the module's `schema`, not from hard-coded numbers, so a 5.4 kernel offers `0..100` and a 6.6 kernel
offers `0..200`. Moving a slider changes nothing until 保存; the save writes with `--persist` and then
re-reads. When `mem.mglru` is `1` the card says that `swappiness` will be diluted — the honest version
of "this might not help".

## Memory composition (Step 2b)

`mem.total` and `mem.available` say how much is free, not what the rest *is*. The composition items
answer that, each one straight from `/proc/meminfo`, each optional:

| item | meminfo key | note |
|---|---|---|
| `mem.anon` / `mem.cached` / `mem.slab` | `AnonPages` / `Cached` / `Slab` | the three big buckets (`Cached` already contains `Shmem`) |
| `mem.kernel_stack` / `mem.page_tables` / `mem.percpu` | same names | kernel memory outside `Slab` |
| `mem.vmalloc_used` | `VmallocUsed` | address space in use, not necessarily resident |
| `mem.gpu` | `GpuTotal` | Samsung; equals kgsl `page_alloc` exactly |
| `mem.vendor.rbin` / `mem.vendor.cma` / `mem.vendor.dmaheap` | `RbinTotal` / `CmaTotal` / `DmaHeapPool` | **pool sizes**, not proven usage |

Measured on S25 (`used = MemTotal − MemAvailable` = 6.28 GiB): anon 40.5 %, slab 13.1 %, GPU 6.2 %,
vmalloc 5.4 %, page tables 3.7 %, kernel stack 1.8 %, percpu 0.2 % → **70.9 % accounted for**, with
29.1 % (1.82 GiB) left. The vendor pools (CMA 0.61 + Rbin 0.39 + DmaHeap 0.02 GiB) explain about half
of that remainder, and roughly 0.8 GiB stays genuinely unattributed.

That is why the pools are separate items rather than being folded into the measured buckets: a CMA
pool's *size* is fixed at boot and says nothing about how much of it is in use, so adding it to "used"
would overstate the bar.

The page draws **two** bars, because there are two questions:

| bar | slices | question it answers |
|---|---|---|
| 已用 | measured + pools + other, summing to `used` | "what is the device holding?" |
| 整机 | the same, plus 缓存（可回收）and 空闲 | "where did all the RAM go?" |

`Cached` is reclaimable and already inside `MemAvailable`, so it is drawn dimmer and labelled
可回收 — never as "used" (adding it pushed the bar to 9.3 GB of a 6.6 GB "used" on S25).

## App memory split (`mem breakdown`)

`sysflow mem breakdown` (protocol §3.24) measures PSS for **every** process and splits it by UID: `< 10 000` is the
platform, everything else is an app (work profiles included). It is a verb with a 60 s cache — a few
hundred milliseconds — and never part of `snapshot`.

It exists because users ask "how much do apps use versus the system", and `mem apps` cannot answer
that: it ranks the top N by RSS, so any sum over it is a sum over a biased sample. Two consequences:

- **It is a second layer, not a slice of the first.** PSS counts shared pages fractionally per
  process and excludes kernel / GPU / vendor memory, so `system + user` does not equal `used`; the
  page says so under the bar instead of pretending.
- **`estimated` is honest.** A process whose `smaps_rollup` cannot be read contributes `VmRSS` and is
  counted in `estimated`, so the sum stays complete while the approximation is visible.

## History sampling (Step 3)

`sample()` runs every 300 s and returns the map the daemon appends to `history/mem.jsonl`, read back
by `sysflow history mem`. The baseline lives in `<data_dir>/state/mem.json` and is written atomically
(temp file + rename), so a killed daemon cannot leave half a baseline behind.

| rule | why |
|---|---|
| instantaneous values on every call (`available_kb`, `swap_used_kb`, `zram_compr_kb`) | a fresh install gets a curve for these three immediately, instead of five minutes of nothing |
| difference series only against a baseline from the **same boot** | the first point after a reboot is a gap, not `0` — `0` would read as "the device was idle" |
| one regressed counter is enough to treat it as a reboot | a negative reclaim count in a curve is worse than a missing point |
| PSI divides stalled µs by the **measured** interval | a phone that slept six hours must not report a burst that never happened |
| swap rates multiply pages by the page size from `auxv` | arm64 ships 4 kB and 16 kB kernels; without a page size the two series are omitted, never guessed |
| a missing `vmstat` / `pgscan_direct` / `allocstall_*` removes only its own series | PSI keeps working on a kernel that renames a counter |

Series names carry no `mem.` prefix, like `battery`'s `level`; the list is protocol §3.22.

## Events (Step 4)

Emitted from inside `sample()`, so they work with the APK closed and never depend on a client. The
hysteresis state lives in `state/mem.json`, which means a daemon restart in the middle of an episode
neither re-announces `high` nor loses the chance to report `recovered`.

| constant | value | note |
|---|---|---|
| `HIGH_PCT` | 300 (3.00 %) | ≈ 6× the S25 long-run mean of 0.52 % — a placeholder |
| `RECOVER_PCT` | 100 (1.00 %) | lower than the trigger on purpose |
| `HIGH_SAMPLES` / `RECOVER_SAMPLES` | 3 / 3 | 15 minutes of sustained pressure at one sample per 300 s |
| `MAX_EVENT_INTERVAL_S` | 900 | longer gaps are recorded but raise nothing |
| `EVENT_WINDOW_S` | 6 h | dedupe window per event family (a second line of defence behind the state machine) |

Three details that are easy to get wrong:

- The dead band between 1 % and 3 % **resets both runs**. "Three consecutive" has to mean three in a
  row; otherwise a noisy curve triggers on any three samples in an afternoon.
- A reboot ends the episode. Reporting `recovered` for pressure that ended in a reboot would be a
  lie, so the upstream state is dropped with the baseline.
- `mem_pressure_high` attaches `last_low_memory_exit` from `dumpsys activity exit-info`, because
  "memory was short" is much more useful when it says who paid for it. That dump is ~2.5 MB on S25,
  so it is read **only when the event fires**, with an 8 s timeout, and a failure just omits the field.

The APK renders these through `EventCopy.kt` (Chinese labels and a `psiDetail` suffix); an
unknown event type still falls back to the producer's own title and message, so a client older
than the module keeps working.

`mem_oom_kill` is deliberately secondary: `oom_kill` counts kernel OOM only, and on S25 it has been
`0` since boot while `exit-info` holds thousands of `LOW_MEMORY` records — lmkd does the killing.

## Per-app ranking (Step 5)

`sysflow mem apps [--limit N] [--refresh]` is a verb, never a `snapshot` item: it opens one entry per
process, so it can only be pulled on demand. Protocol §3.23 has the payload.

| rule | why |
|---|---|
| rank by `VmRSS`, read `smaps_rollup` only for the top `limit` | PSS for a few hundred processes blows the latency budget; RSS is a good ordering key |
| processes below the cut contribute their RSS, `pss_for_top_n_only` says so | the client must be able to label those numbers as approximate |
| aggregate by UID, `procs` counts that app's processes | one app can be several processes (a `:remote` service, a WebView sandbox) |
| kernel threads (no `VmRSS`) are not counted in `skipped` | otherwise hundreds of kthreads would drown the one number that matters |
| the package database is a file, `cmd package` is only the fallback | `/data/system/packages.list` is one read; a binder call per cold ranking is not |
| the UID map has its own ten-minute cache | install/uninstall is slow-moving, and the fallback path is expensive |
| the ranking cache is keyed by `limit` | a different `limit` moves the PSS cut-off, so the numbers are not comparable |

`uid` is Android's short form (`u0a346`) and is the exact inverse of `battery apps`' `resolve_uid`,
including the 10 000 base that ordinary app ids carry. Profiles are matched on `uid % 100000` and
labelled `package (user N)`.

## Device differences worth probing (Step 1 checklist)

- [x] `/proc/pressure/memory`: present and readable.
- [x] zram devices: one; `mm_stat` has 9 columns.
- [x] non-zram swap: none (`/proc/swaps` lists only `zram0`).
- [x] kernel release: `6.6.127-android15-8` → `swappiness` ceiling 200.
- [x] vmstat counter names: all present; `allocstall` is split five ways (see Paths).
- [ ] Does a ROM daemon rewrite `swappiness` / `vfs_cache_pressure` / `dirty_*`? **Inconclusive**: only a
      3-minute run so far (3 identical samples, `130 100 0 0 11584`); the detached attempts died on a
      truncated script (see `Running the probe`). Still needs a 24 h run.
- [x] `dirty_bytes` / `dirty_background_bytes` non-zero on this ROM: yes.
- [x] `/data/system/packages.list` readable by the module; format has 10 columns (use the first two only).
- [x] Kill-record sources: `dmesg` has none; `am_kill` mixes policy kills with memory kills; `exit-info`
      carries `reason=3 (LOW_MEMORY)` per package. Record layout confirmed (see Observed values).
- [x] `dmabuf` / GPU memory: readable, and `GpuTotal` in meminfo equals kgsl `page_alloc`; the 138
      dmabuf buffers are redundant for the composition bar.
- [ ] Cost of one `collect()` and one `sample()` on the device (Step 2).
- [x] Full `/proc/meminfo` key list: 56 keys dumped; the unexplained gap dropped from ≈ 2.8 GB to ≈ 1.8 GiB.
- [x] `MGLRU`: enabled (`0x0003`) — `swappiness` is the weaker lever on this device.
- [x] zRAM writeback: `backing_dev` = `/dev/block/loop37`, enabled.

## Measured costs (filled per step)

| operation | budget | measured |
|---|---|---|
| `collect()` (snapshot section) | < 5 ms | 0.25 ms — host, debug build, fake tree (Step 2). Device pending |
| `snapshot` end-to-end, all modules | — | 4 ms — host, debug, includes process start |
| `sample()` | < 10 ms | 0.35 ms — host, debug, **includes the state-file write** (Step 3). Device pending |
| `mem apps`, hot cache | < 300 ms | 8 ms — host, five-process fake `/proc` (Step 5). Device pending |
| `mem apps`, cold cache | report only | 5 ms — same tree; a device is dominated by the top-N `smaps_rollup` reads |
| history file after 30 days | ~1.7 MB estimate | 8 640 lines at 300 s; estimate unchanged (Step 3) |

## Degradation reasons (filled per step)

| item removed | reason on this device |
|---|---|
| — | — |

## Rollback and safety

- **M3a (telemetry)**: no system state is changed; rollback is unregistering the module in
  `builtin()`.
- **M3b (tunables)**: implemented as `reset mem` (restores the default snapshot captured before the
  first write, i.e. the ROM's own value — 130 for `swappiness` on S25, never a hard-coded 60),
  safe-mode skip (the whole write half is skipped, telemetry keeps working), the drift `contested`
  rule (3 repairs inside 30 min → stop for 6 h + one `mem_setting_contested` event) and boot
  sanitising (a value this kernel cannot accept is dropped one item at a time). `min_free_kbytes` is
  not implemented at all — it is not exposed, so there is no session-only mode to describe.
- **Deferred (Step 9)**: zRAM reconfiguration (`disksize` / `comp_algorithm` / `reset`) needs
  `swapoff`, which pulls swapped pages back into RAM; it is re-evaluated with the process module.

## Host test tree

Built in `core/sysflow-modules/src/mem/tests.rs`: a fake root with `proc/meminfo`,
`proc/pressure/memory`, `proc/vmstat`, `proc/swaps`, `proc/sys/kernel/osrelease`, the `proc/sys/vm/*`
knobs and `sys/block/zram0`. Variants covered: no PSI, unreadable PSI (a directory where the file
should be), malformed PSI, PSI without the `full` line, two zram devices, 7- and 9-column `mm_stat`,
a non-zram swap line, a readable-but-empty swap table, a missing `MemAvailable`, block devices whose
names merely start with `zram` (`zramfs`, `zram`), and an extra meminfo key set (`KernelStack`,
`PageTables`) for the composition probe.

Two tests pin the contract rather than a value: `collect_reports_exactly_the_items_the_schema_lists`
(schema and snapshot can never disagree) and `zram_ratio_is_recomputed_from_the_sums` (a ratio that
was averaged instead of recomputed would read 250 % instead of 399 %).

For sampling the tree also carries `proc/sys/kernel/random/boot_id`, `proc/uptime` and a binary
`proc/self/auxv`, so a reboot (boot-id change), an uptime regression, a counter regression, a missing
`vmstat`, a missing auxv and a 16 kB page size are all reproducible without a device.
