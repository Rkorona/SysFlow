# Module WebUI: rescue console

The WebUI shipped inside the module zip is **not** a second client. It is a single-screen console
with one job: keep the module recoverable when the APK cannot be installed, updated or launched, or
when a bad configuration stops the device from behaving.

Entry point: `module/webroot/` (KernelSU / APatch manager's "Open" button).

## Implementation

Plain HTML + CSS + JavaScript, three files, no framework and **no build step**:

| file | size | role |
|---|---|---|
| `module/webroot/index.html` | 2.7 KB | markup |
| `module/webroot/app.js` | 10 KB | CLI bridge + view |
| `module/webroot/app.css` | 4.7 KB | styles (light/dark) |

The KernelSU WebView injects a global `ksu` object; `ksu.exec(command, optionsJson, callbackName)`
invokes a global callback with `(errno, stdout, stderr)`. `app.js` calls that directly instead of
pulling the `kernelsu` npm wrapper, which is why the console needs neither Node nor Vite. The module
build only checks that the three files exist.

## What it does

| action | CLI call |
|---|---|
| version / protocol / module capabilities | `version`, `caps` |
| current state timestamp | `snapshot` |
| safe mode and boot count | `safe-mode status` |
| leave safe mode | `safe-mode clear` |
| restore every module's defaults | `reset --all` |
| read-only drift check | `check` |
| recent module log | `log --tail 200` |

Everything is read or triggered on demand; the console never polls and writes nothing except through
the two explicit recovery buttons (both behind a confirmation or a disabled state).

## What it deliberately does not do

No CPU/battery/adaptive/screen-off/cpuset/event pages, no charts, no settings editor, no `?demo=1`
preview. Those live in the Android APK.

## Why the scope was cut

Full feature parity cost about 7 000 lines of parallel TypeScript against 11 000 lines of Kotlin,
plus four duplicated implementation pairs that had to be kept in sync by hand:

| logic | WebUI | APK |
|---|---|---|
| event copy table | `event-copy.ts` | `EventCopy.kt` |
| battery model | `battery-model.ts` | `BatteryModels.kt` |
| CPU model | `cpu-model.ts` | `CpuModels.kt` |
| client / API | `api.ts` | `SysFlowClient.kt` |

It also put Node and a network-dependent `npm ci` on the module build path, which failed on a
machine without registry access. Cutting parity removed both costs:

| | full React client | rescue console (React) | now (static) |
|---|---|---|---|
| WebUI source | 7 121 lines | ~350 lines | 527 lines total |
| shipped | 352 KB JS + 56 KB CSS | 230 KB JS + 3.7 KB CSS | **17 KB, no bundle** |
| module build needs npm | yes | optional | **no** |
| duplicated models to sync | 4 | 0 | 0 |
| build step | `npm ci && vite build` | same | **none** |

Dropping React was the last piece: a rescue page does not need a component tree, and the 230 KB
bundle was almost entirely the runtime.

## Consequences to keep in mind

- **`docs/protocol.md` remains the contract.** The console only uses documented commands, so it is a
  useful smoke test of the CLI after a module upgrade.
- **Recovery without a GUI** is still possible: `adb shell` + `sysflow safe-mode clear` or
  `sysflow reset --all`. The console is the convenient path, not the only one.
- **Protocol changes must be reflected here only if the console uses them**; feature additions no
  longer require any WebUI work.
