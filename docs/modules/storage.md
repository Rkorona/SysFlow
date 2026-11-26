# Storage module: paths, items and device notes

Implementation: `core/sysflow-modules/src/storage/` (`mod.rs`, `capacity.rs`, `device.rs`,
`diskstats.rs`, `tunables.rs`, `history.rs`, `usage.rs`). Protocol: `docs/protocol.md` §3.26. Plan and
decisions: `docs/storage-module-plan.md`.

> **Status: Steps 0–5 implemented on the host (2026-11-26); device probe and acceptance pending.**
> The matrix below is a **checklist**: every `待测` cell is filled by running
> `sysflow storage probe` on the phone. Host behaviour is covered by unit tests against a fake
> `/sys` + `/proc` and a real temporary directory for `statvfs` / directory walks.

## Paths

Probe: `sysflow storage probe --pretty` on the device, saved to `/sdcard/storage-probe.json`.

| feature | path | fallback if missing | observed |
|---|---|---|---|
| mount table | `/proc/mounts` | module `unsupported` | 待测 |
| capacity | `statvfs("/data")` | module `unsupported` | 待测 |
| emulated storage | `/storage/emulated/0` or `/sdcard` | sdcard items omitted | 待测 |
| block devices | `/sys/block/<name>/{size,device/model,queue/*}` | device items omitted; capacity/IO remain | 待测 |
| physical device behind `/data` | `/sys/block/<dm>/slaves/*` | use the mounted node if it has a queue | 待测 |
| scheduler | `/sys/block/<dev>/queue/scheduler` | `scheduler` feature absent | 待测 |
| read-ahead | `/sys/block/<dev>/queue/read_ahead_kb` | `read_ahead` feature absent | 待测 |
| queue depth | `/sys/block/<dev>/queue/nr_requests` | field omitted | 待测 |
| I/O counters | `/proc/diskstats` | `io` feature absent | 待测 |
| health | `/sys/block/<dev>/device/life_time` (or `life_time_estimation`) | `storage.device.health_pct` omitted | 待测 |
| TRIM | `/system/bin/fstrim` (also `/system/xbin`, `/sbin`) | `trim` feature absent | 待测 |
| directory usage | `std::fs` walk from `/data` | `usage` feature absent | 待测 |

## Running the probe (Step 1)

```sh
/data/adb/modules/sysflow/bin/sysflow storage probe --pretty > /sdcard/storage-probe.json
```

Required output (checked off as it lands):

- [ ] `/data` mount line and its device (`dm-*` number), plus whether `/storage/emulated/0` is a
      separate device
- [ ] `statvfs` numbers compared with `df -k /data` in the same minute
- [ ] every `/sys/block` entry: size, model, rotational, scheduler options, read_ahead, nr_requests
- [ ] the physical device behind `/data` (follow `slaves`) and its `/proc/diskstats` line
- [ ] `life_time` / `life_time_estimation` raw text, if present
- [ ] `fstrim --help` first lines and `df -k /data`
- [ ] timing: `storage io --interval-ms 1000`, `storage usage --budget-ms 3000` (elapsed, partial)

## Parsing rules (fixed now)

- **Sectors are 512 bytes** in `/proc/diskstats` even on 4K-native devices. `read_kb = sectors / 2`.
- **Field order** (kernel docs): major, minor, name, reads completed, reads merged, sectors read,
  ms reading, writes completed, writes merged, sectors written, ms writing, in-flight, ms doing I/O,
  weighted ms. SysFlow reads indices 3, 5, 7, 9, 12.
- **Utilisation** = `Δio_ms / Δt`, clamped to 100 %; parallel queues can over-count.
- **Partitions do not shadow the disk**: the data device is resolved through `slaves`, so `sda` is
  used, not `sda1`.
- **Directory sizes are allocated blocks** (`st_blocks × 512`), not apparent size; symlinks are not
  followed and other filesystems are not crossed.
- **`storage partitions` is the mount-level view**, not a walk of `/`: it lists every real mount
  with `statvfs` numbers and skips pseudo filesystems and duplicate views (`/storage/*`, `/mnt/*`).
  A directory walk of `/` would only see the read-only rootfs and would skip the mounts.
- **Root mode** (`--path /`) lists the phone's root: mount points are reported as partition
  used/total (never walked), rootfs files/dirs are walked, and `proc/sys/dev`, `/storage/*`,
  `/mnt/*`, `/apex/*` and symlinks are labelled instead of counted. This is the literal `ls /`
  view with sizes.
- **Two analysis targets.** `/storage/emulated/0` is the user-visible internal storage (DCIM,
  Download, Android…), while `/data` is the whole partition (app, data, media, system…).
  `/storage/emulated/0` is the same blocks as `/data/media/0`, so the two totals must never be
  summed. The APK defaults to the user-visible view and offers `/data` as the diagnostic view.
- **JEDEC health**: `0x01..0x0a` = 10 %…100 % of rated endurance used, `0x0b` = exceeded. Any other
  format hides `storage.device.health_pct` instead of guessing.

## Units

| quantity | unit |
|---|---|
| capacity / read-ahead / I/O volumes | kB |
| percentages (`used_pct`, `util_pct`, `health_pct`) | integer % |
| durations (`interval_ms`, `elapsed_ms`, `ms`) | ms |
| counters (`*_ios`) | completed requests |

## Tunables

| id | type | values | note |
|---|---|---|---|
| `storage.scheduler` | enum / low / persist | the kernel's own list | written to the physical device behind `/data` |
| `storage.read_ahead_kb` | int_enum / low / persist | 128, 256, 512, 1024, 2048, 4096 (+ current if outside) | 128 is the usual Android default |

Both go through `apply` → read back → `mismatch`, and `sanitize_boot` drops values this kernel no
longer accepts. There is no module-private contested-event guard yet (the default drift path is
used).

## History and events

`storage` samples every **300 s** into `history/storage.jsonl`: `used_kb`, `free_kb`, `used_pct`
always; `read_kb`, `write_kb`, `io_util_pct` only with a same-boot baseline. Events:
`storage_low_space` at ≥ 90 % used (warning) and `storage_space_recovered` at ≤ 85 % (info), both
deduped for 6 hours.

## Measured costs

Filled per step; do not guess.

| path | target | measured |
|---|---|---|
| `snapshot` storage section | < 5 ms | 待测 |
| `storage io --interval-ms 1000` | ~1 s + < 20 ms | 待测 |
| `storage usage --budget-ms 3000` | ≤ 3.5 s | 待测 |
| `storage trim` | device dependent | 待测 |

## Degradation reasons

| condition | symptom in output | first seen |
|---|---|---|
| `/proc/mounts` missing or no `/data` | module `unsupported`, no storage section | 待测 |
| `statvfs` fails | module `unsupported` | 待测 |
| dm node without slaves | queue tunables hidden, capacity/IO still shown | 待测 |
| no `/proc/diskstats` | `io` feature absent | 待测 |
| no `life_time` in JEDEC format | health item omitted | 待测 |
| no `fstrim` binary | trim verb `unsupported` | 待测 |
| scan budget exhausted | `partial:true` + `visited` | 待测 |

## Rollback and safety

- Read-only steps change nothing; rollback is removing the module registration.
- The two queue tunables are persisted only with `--persist`; `reset` restores the values captured
  before the first write. Reboot re-applies through `apply-boot` / daemon drift.
- `trim` and `usage` never write configuration; they cannot affect boot, drift or safe mode.
- `trim` is restricted to `/data` and `/storage/emulated/0`.

## Device checklist (Step 6)

1. **Probe** and send `storage-probe.json` (mounts, devices, queue options, diskstats, health,
   `fstrim`, `df`).
2. **Capacity** vs `df -k /data` at the same moment (total/used/avail within rounding).
3. **I/O** vs a manual `cat /proc/diskstats` pair over the same interval.
4. **Scheduler / read_ahead**: change both, read back, reboot, confirm replay; then `reset`.
5. **TRIM**: run it, confirm `trimmed_bytes > 0` and the device stays responsive.
6. **Partitions**: run `storage partitions`; compare `/data` with `df -k /data`, and confirm
   `/storage` / `/mnt` rows are absent (they would double-count).
7. **Root scan**: `sysflow storage usage --path /`; compare the mount rows (`/data`, `/system`,
   `/cache`…) with `df -k` and check `pseudo`/`view`/`symlink` entries are labelled, not counted.
8. **Directory analysis**: run with the default budget on both `/storage/emulated/0` and `/data`;
   report elapsed, `partial`, denied counts, and whether the top entries look right (compare with
   `du -x -d1 /storage/emulated/0` and `du -x -d1 /data`). They must not be added together.
7. **Low-space event**: hard to force on a healthy device; check the event path by temporarily
   lowering the threshold in a debug build if needed.
8. **APK**: page loads, 2 s polling stops when backgrounded, TRIM confirmation works, partial
   analysis is labelled.

## Host test tree

Unit tests live beside each module (`capacity.rs`, `device.rs`, `diskstats.rs`, `tunables.rs`,
`history.rs`, `usage.rs`, `mod.rs`). Fake roots hold `proc/mounts`, `proc/diskstats`,
`proc/sys/kernel/random/boot_id`, `proc/uptime`, `sys/block/<dm>/slaves/<disk>`,
`sys/block/<disk>/queue/*`, `sys/block/<disk>/device/{model,life_time}` and `system/bin/fstrim`.
`statvfs` and directory walks use real temporary directories via `SYSFLOW_STORAGE_DATA_PATH`.
