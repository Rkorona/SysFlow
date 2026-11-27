# SysFlow Event System

## Ownership boundary

- **Root module is the producer and source of truth.** It captures system state transitions and
  writes a durable journal under `/data/adb/sysflow/events/`.
- **APK is a client.** It requests event pages through the `sysflow event list` CLI, imports them
  into Room, and renders Room `Flow` streams. APK code must not invent events by comparing
  dashboard snapshots or probe kernel state to infer event records.
- **Room is the local read model.** Its `sysflow-events.db` database is a queryable mirror for UI,
  not a replacement for the Root journal.

## Open-world event contract

The event protocol is intentionally not an enum or a module registry. A producer may use any
stable module identifier, category, and event type that follow `[A-Za-z0-9._-]` and the protocol
length limits. Adding a new module or a new event type does not require a Kotlin model, Room entity,
DAO, or APK release change.

Required fields:

| Field | Meaning |
|---|---|
| `module` | Producer/module id, e.g. `cpu`, `battery`, `storage`, `process`, `wakelock` |
| `category` | Open category such as `thermal`, `power`, `pressure`, `lifecycle`, `configuration` |
| `event_type` | Stable machine identifier such as `temperature_high` |
| `severity` | `info`, `notice`, `warning`, `error`, or `critical` |
| `title` | Short user-facing title |
| `message` | Optional explanatory text |
| `metadata` | Any JSON value (object recommended); max 16 KiB |

The module assigns `id` and `timestamp_ms`. Producers should use a stable `dedupe_key` for
repeated transition observations, not for distinct occurrences. Deduplication is scoped by module,
category, event type and key; the dedupe window is configurable. The APK mirrors the optional key
in Room for lossless record storage.

## Machine-checked event vocabulary

The open-world contract above is still the only rule producers must follow. Because "any stable
identifier" is also easy to typo, `tools/check-events.py` keeps the *shipped* producers honest without
turning the protocol into a registry:

- It inventories every literal producer — Rust `NewSystemEvent::new(...)` calls outside `#[cfg(test)]`
  and shell `event emit --module .. --category .. --type ..` lines (continuations joined) — and
  regenerates `docs/events-catalog.md`. That file is generated, not hand-maintained: run the tool and
  commit the result with the code change.
- It validates those literals against the contract: `module` and `category` are 1–64 characters and
  `event_type` is 1–96, all from `[A-Za-z0-9._-]`, and a literal `severity` must be one of `info`,
  `notice`, `warning`, `error`, `critical` (`docs/protocol.md` §3.10; numeric limits mirrored from
  `core/sysflow-core/src/event.rs`).
- Non-literal values (variables, `format!`, shell `$VAR`) are never ignored: they are listed in the
  catalog's dynamic section so a refactor cannot silently drop a producer.
- It proves the APK stays a client: `app/**` must not emit events (`event list` is the only event
  command the APK may call).
- `tools/check-events.py --check` fails when the catalog is stale or a literal is malformed, so CI can
  run it as a drift gate.

This is a lint over *this repository's* producers, not an allow-list: an unknown module, category or
event type from a future producer still works at runtime, Room still stores it, and the APK still
renders it from the producer's own `title` / `message`. A missing Chinese label in `EventCopy.kt` is
reported by `--check` but does not fail (use `--strict-labels` to make it fail); a new event is
expected to arrive before its presentation copy, and unknown events must never be hidden.

To add an event: write the producer with literal identifiers, run `tools/check-events.py`, and commit
`docs/events-catalog.md` together with the code change.

## Emit from any module or service script

The generic CLI is the recommended bridge for shell-based producers and future modules:

```sh
/data/adb/modules/sysflow/bin/sysflow event emit \
  --module battery \
  --category charging \
  --type charging_started \
  --severity notice \
  --title "Charging started" \
  --message "USB power source connected" \
  --metadata '{"source":"usb","voltage_mv":9000}' \
  --dedupe-key battery.charging.started \
  --dedupe-window-s 5
```

Rust module implementations can use `sysflow_core::event::{EventStore, NewSystemEvent}` directly.
The store is rooted at the same `Ctx::data_dir`, so producers in separate processes share the same
journal. Event write failures must be handled as best-effort telemetry: they must never prevent a
safety, thermal, battery, or configuration operation from completing.

## Query and cursor contract

- `sysflow event list --limit 200` returns newest-first records.
- For initial backfill, use `--before-ms` and `--before-id` with the oldest record of the previous
  descending page.
- For incremental sync, use `--after-ms` and `--after-id --ascending` and continue until a short
  page is returned.
- Module, category, and severity filters are optional open-string values.
- `event prune` applies the retention policy. Defaults are 30 days / 5,000 events.
- `event clear --all` deletes the Root journal and returns
  `{"cleared":<count>,"directory":"<resolved events dir>"}`. It is intentionally explicit. Clearing
  Room only deletes the APK cache; it never clears the Root-owned source of truth. A record that
  cannot be deleted fails the command instead of being silently left out of the count, so `cleared: 0`
  always means "the journal was empty", never "the files were still there".
- `event list` also reports `directory`, the path it actually read. When the APK and a terminal
  disagree about the journal, those two strings are the answer: a mismatch means the two processes
  resolved different data directories, not that a delete failed.
- **Deleting "all events" therefore needs both halves.** The APK's 模块动态 page offers a
  `清除全部事件` action (bottom of the list, behind a confirmation dialog) that calls
  `event clear --all` *and* `EventRepository.clearLocalCache()`. Doing only the first leaves the
  list unchanged, because the APK renders its Room mirror rather than the journal.
- The mirror also repairs itself: before each incremental sync it reads the journal's oldest record,
  and if the journal is empty — or its oldest record is newer than the newest mirrored row — the
  mirror is dropped. That is what makes a clear performed *outside* the app (a terminal
  `event clear --all`) empty the list on the next foreground sync. A failed probe is not treated as
  an empty journal, so an older module binary without `event list` cannot wipe the cache.

## Current producers

The Registry emits generic module configuration events when any current or future registered
module persists settings or is reset. Core lifecycle, boot-confirm, daemon and safe-mode actions
also emit events. The module service emits startup, display-transition and battery-transition events (`battery_loop`:
charging started/stopped, fully charged, low battery, battery temperature high). The
CPU adaptive controller emits thermal-protection, battery-saving and PSI guard transitions. The
memory module emits `mem_pressure_high` / `mem_pressure_recovered` / `mem_oom_kill` from inside its
`sample()` and `mem_setting_contested` when a sysctl keeps being reset. The wake-lock module emits
`wakelock_long_held` from its own `sample()`. The process module emits one `process_action` event per
executed `stop` / `oom` / `renice` / `sched` / `cpuset` / `freeze` action (metadata carries the
target, the applied value and whether the read-back verified it). More specific module events should
be added at the code that owns the state transition, rather than in APK refresh or UI code.

## APK / Room behavior

`SystemEventEntity` stores all protocol fields plus `metadataJson`; module/category/type remain
strings, so future event names fit the same schema. Room has time/module/category/severity indexes,
Flow-based recent and filtered queries, exact module/category filters, and a unique event id for
idempotent imports. `EventRepository` serializes import transactions, performs resumable first-install backfill (the
completion marker itself is stored in Room), cursor-based incremental sync, and caps local
retention at 30 days / 5,000 events. Room version 2 migrates the earlier v1 cache without deleting
history and resets the backfill marker so older Root records can be imported.
The overview observes the newest events; the history page supports module/category/severity
filters and paginated local reads.

## Presentation copy (module and event labels)

Producers stay language-neutral: they emit a stable `event_type` identifier plus an English `title`
and `message`. Both clients own a **presentation-only** table that turns `(module, event_type)` into
a Chinese label (`CPU 管理 · 设置已更新`) and a one-line Chinese explanation:

- APK: `app/src/main/kotlin/io/sysflow/feature/home/EventCopy.kt`

The module WebUI is a rescue console and renders no events, so there is a single copy table to
maintain.

The contract is unchanged — this table never filters, rewrites, or drops records. An unknown module
or event type still renders, falling back to the producer's own `title` and `message`, so shipping a
new module needs no UI change to make its events visible. When you add a new `event_type`, add its
label to `EventCopy.kt`; the rescue console renders no events, so there is exactly one table.

## Overview activity layout

The overview renders **one** card for system activity, holding one row per module (hard cap of four
rows: `MODULE_ROW_LIMIT`). Each row is the module name plus up to two recent event lines, so the
section height stays constant no matter how many modules emit events — a growing module count must
never stretch the overview.

The module set is derived from the events themselves, never from a registry, so a new module appears
as soon as it emits its first event and needs no UI change. Rows sort by each module's newest event.

Navigation (APK; the rescue console has no event UI):

1. Tapping a row → that module's event list, filtered to the module.
2. Tapping `查看全部` → `模块动态`: every module that produced events, with its recent count.
3. Tapping a module there → that module's event list. Back returns to `模块动态`, then the overview.

Both screens are derived from the newest 100 records (`HOME_EVENT_LIMIT` in `EventRepository`). A
module whose newest event is older than that window disappears from both lists until it emits again;
the full journal stays one tap away on the history page. The durable fix is a Room `GROUP BY module`
aggregate, which is not implemented yet (it would only change the APK; the CLI has no aggregate
command because no other client needs one).

Identical consecutive records collapse into one row carrying a count, because producers re-emit the
same observation on every tick. The full journal remains available on the history page.
