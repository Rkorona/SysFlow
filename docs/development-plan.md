# SysFlow 统一设备管理项目 · 开发计划

> 项目名：**SysFlow**（二进制名 `sysflow`）。
> 形态：**Root 模块（Rust 原生二进制 + 守护进程 + WebUI）** + **M3 Expressive Android APK（纯 UI，不常驻）**。
> 目标：把 CPU / 电池 / 唤醒锁 / 内存 / 进程 / 储存的管理集中到一个项目里，一套协议、一套配置、一套 UI 框架。

---

## 当前进度

图例：✅ 已完成并验证　🚧 进行中　🟡 已编写，待编译 / 真机验证　⬜ 未开始

| 阶段 | 状态 | 说明 |
|---|---|---|
| 阶段 0 基础与协议 | ✅ | 基础协议、配置原子存储、安全模式与 CLI 骨架已完成；后续迭代已补齐 `event`、`recovery`、`boot`、`daemon`、`info`、`note` 等能力 |
| 阶段 1 CPU + 事件 + 守护 + APK + 救援控制台 | ✅ | CPU 全量调节（核心、调频、GPU/DDR/LLCC/WALT/cpuset/熄屏、自适应四策略）、配置漂移守护与自适应 daemon、开放式事件期刊 + Room 只读镜像、隔离恢复与诊断；APK 功能页与静态救援控制台（`module/webroot/`）均已完成；`cargo test` 工作区通过，`./build-sysflow-module.sh` 产出模块包（当前 v0.2.2）、`:app:assembleDebug` 构建成功；真机回归验收仍在跟踪 |
| 阶段 2 电池 / 唤醒锁 / 历史曲线 | ✅ | **已完成**：历史采样基础设施（daemon 内 `sample()`/`sample_interval()` 调度、`history/<module>.jsonl` 30 天 / 20 000 行、`history` 降采样与真实 `daemon_running`）；电池只读遥测、`service.sh` 事件埋点、`battery apps` 耗电排行、`battery health` 循环计数兜底、APK 电池页（会话趋势 + 24 小时曲线 + 上次充电以来掉电 + 亮屏/熄屏掉电拆分）；唤醒锁两轮全部落地（只读唤醒源表 + 24h 窗口排行 + `wakelock held`；AppOps `WAKE_LOCK` 拦截 + 白名单 + 规则持久化/开机重放 + APK 唤醒锁页 + `wakelock apps` / `wakelock wakeup` / 长持锁看门狗）；熄屏切换并入 daemon。剩余仅为真机回归验收与总览页唤醒锁卡片（纯 UI） |
| 阶段 3 内存 | ✅ | **已实现（Step 0–8，2026-11-25）**，见 `docs/mem-module-plan.md` 与 `docs/modules/mem.md`：`mem.*` 只读遥测（meminfo / PSI / zRAM / 构成项 / MGLRU）、两个可调项（`mem.swappiness`、`mem.vfs_cache_pressure`）、`history mem` 差分序列、压力事件、`mem apps` / `mem breakdown` verb、APK 内存页与导航接入。Step 9（zRAM 重配）暂缓；真机计时、事件噪音阈值与 24 h `swappiness` 观察仍待验 |
| 阶段 4 进程 | 🚧 | **Step 0–5 主机完成（2026-11-25）**：契约冻结、S25 真机探针、只读面（`list`/`get`/前台/保护）、一次性动作（`stop`/`oom`/`renice`）、实验动作（`cpuset`/`freeze`）与协议 §3.25 已落；桌机侧 `cargo fmt` / `clippy -D warnings` / 全量测试通过（process 35 项）。APK 进程页（M3E 列表 + 详情 + 确认弹窗）已写，APK 由用户自行构建；真机写路径与验收待跑 |
| 阶段 5 储存 | ⬜ | |
| 阶段 6 打磨与发布 | ⬜ | |

**已确定的标识**：项目名 / 应用名 SysFlow；模块 id 与二进制 `sysflow`；包名 `io.sysflow`。

**已确定的技术决策**（仓库中没有 `docs/adr/` 目录；决策摘要即本节、第 3 章与 `docs/event-system.md`）：
- **APK 基础**：依赖注入用 **Koin**；**minSdk 29**；root 方案范围 KernelSU / KernelSU Next / APatch / Magisk。模块安装脚本当前限定 **arm64（`aarch64-linux-android`）**。
- **客户端架构**：
  - WebUI 是**静态救援控制台**（`module/webroot/` 三个文件，纯 HTML/CSS/JS，直接调用 KernelSU 注入的 `ksu.exec`），无框架、无 npm、无构建步骤、无 `?demo=1` 预览；功能页（CPU/电池/唤醒锁/内存/自适应/熄屏/进程组/事件）只在 APK 中，理由见 `docs/webui-console.md`；
  - 协议类型由 Rust `sysflow-types` 生成，当前只生成 Kotlin（`--ts` 能力保留但未使用）；
  - APK 采用单 `:app` Gradle 模块 + 包边界隔离、Material 3 Expressive、`libsu` 常驻 shell、Room 只读事件缓存。
- **事件边界**（`docs/event-system.md`）：Root 模块是结构化事件唯一产源（`/data/adb/sysflow/events/`）；APK 使用 Room（`sysflow-events.db`）做前台只读查询缓存；WebUI 不读取也不渲染事件（救援控制台，无事件页）。

**阶段 0 以来的主要演进与调整**：

- 日志用内置轻量大小轮转实现，未引入 `tracing`；错误类型手写，未引入 `thiserror`。
- 新增命令 `boot-confirm`、`info`、`boot status|configure`、`recovery status|restore|clear|clear-corrupt`、`daemon configure|run`、`enforce`（隐藏）、`note` 与开放式 `event emit|list|prune|clear`。
- `snapshot` 增加 `settings`、`defaults`、`errors` 字段：单模块读取失败不影响其余模块返回，同时支持客户端区分静态配置与动态封顶值。
- `Module` trait 扩展 `apply_with_settings()`、`persist_on_mismatch()`、`drift()`、`normalize_default()`、`sanitize_boot()`、`in_snapshot()`；`reset()` 先做模块清理再由 Registry 统一恢复 `defaults_snapshot`。
- 单位约定已定：温度为 0.1 °C 整数，频率 kHz，电压/电流/功率 mV/mA/mW，容量 kB，事件时间戳为毫秒级 Unix 时间，通用 `snapshot`/`history` 时间戳为秒级。
- **守护进程与历史采样拆解**：阶段 1 实现 Rust daemon（模块 `service.sh` 启动独立进程，按 2–120 秒间隔执行配置漂移修复与 CPU 自适应采样）；阶段 2 已把各模块 `sample()` 调度接入该循环并写入 `history/<module>.jsonl`；熄屏检测也已并入 daemon（Rust `screen.rs`），`service.sh` 的 `screen_loop` 与 `screen-state.sh` 已删除；期间修复了两个只有真机才会暴露的检测缺陷：亮度「桶」缓存导致一次瞬时 Off 被永久缓存（亮屏却压频）、以及 `mWakefulnessChanging` 被误当作 `mWakefulness`；唤醒锁先以只读模块落地（内核唤醒源 + 24h 窗口差分在模块状态里滚动，不做逐源历史列）。
- **APK 持久化与界面方案调整**：为支持离线浏览和筛选事件，APK 引入 **Room** 镜像数据库（而原计划用于主题/单位偏好的 DataStore 尚未引入）；UI 放弃早期泛化 `SchemaRenderer`，改为专用的 CPU 核心矩阵、簇双滑块及自适应/熄屏/进程组/诊断/事件二级页。

**下一步**：阶段 0–3 的代码、主机测试与构建均已就绪（模块 v0.2.2、APK debug 已出包），转入真机回归验收：重点验证 CPU 每核心利用率、adaptive 四类策略、daemon 生命周期、电池历史曲线与充电/低电事件、唤醒锁拦截与排行、内存构成条与 PSI 曲线、温控传感器、诊断和隔离恢复入口。熄屏切换已由 daemon 承担（1–5 秒轮询、`dumpsys power` 复核），采样同时记录屏幕状态，APK 电池页可给出亮屏/熄屏掉电速率。内存模块的 Step 9（zRAM 重配）重估结论为**继续暂缓**，不作为 M4 依赖。阶段 4 已进入 Step 0：契约冻结完成（`docs/process-module-plan.md`、协议 §3.25 Draft、`docs/modules/process.md` 矩阵骨架），真机回归验收与 M4 Step 1 探针可并行推进。

协议侧剩下的都是**类型生成**的缺口，不是文档缺口：`core/sysflow-types` 还没有 `BootStatus`，`DaemonStatus` 也只建模了 `running` / `implemented`，而 CLI 的 `daemon status` 还返回 `enabled` 与 `interval_s`（见 `docs/protocol.md` §4）。旧 `procs` / `stop-app` 不迁移，阶段 4 统一为 `process list|stop`（协议 §3.25 Draft）。

---

## 目录

1. 项目目标与非目标
2. 总体架构
3. 技术选型
4. 仓库结构
5. 通信协议规范 v1
6. 模块端设计
7. 功能模块详细设计
8. 守护进程与历史数据
9. 安全与回滚机制
10. APK 设计（M3 Expressive）
11. WebUI 设计
12. 开发阶段与里程碑
13. 测试策略
14. 构建、发布与版本管理
15. 风险与对策
16. 待决策事项

---

## 1. 项目目标与非目标

### 1.1 目标

- **统一**：一个模块、一个 APK，取代分散的多个小工具（并在安装/启动时兼容清理旧版 SoCFlow 遗留状态）。
- **零常驻**：APK 不跑后台服务；所有策略、定时守护、事件采集、历史采样都由模块侧完成，APK 被杀不影响任何已生效的设置。
- **可扩展**：新增一个功能 = 实现一个 Rust trait + 注册；CLI、`snapshot`、通用配置变更事件、WebUI/APK 基础入口自动获得；开放式事件协议允许未来模块自由上报事件而无需修改 APK 数据库结构。
- **安全可恢复**：任何调节都能一键恢复默认，异常开机自动隔离配置并进入安全模式，保留救援文件入口。
- **体验好**：APK 采用 Material 3 Expressive 设计语言；没装 APK 时 KernelSU WebUI 仍可完成完整查看、调节、诊断、恢复与事件回溯。

### 1.2 非目标（v1 不做）

- 不做云同步、账号体系。
- 不做内核级修改（不碰内核模块/补丁），只通过 sysfs / procfs / 系统命令。
- 不做自动"智能调度"（如根据前台应用自动切换性能档位），留到 v2 以后评估。
- 不保证所有机型全部功能可用：靠能力探测，不支持的功能直接隐藏。

---

## 2. 总体架构

```
┌──────────────────────────── 手机 ────────────────────────────┐
│                                                              │
│  APK (Compose, M3 Expressive)        KernelSU WebUI (独立可用) │
│   │  libsu 常驻 shell: sysflow ...       │  ksu.exec sysflow  │
│   │  └─ Room (仅事件只读缓存)             │                  │
│   └──────────────┬──────────────────────┘                    │
│                  ▼                                           │
│        /data/adb/modules/sysflow/bin/sysflow   ← 唯一的"大脑" │
│        ├─ CLI 层：参数解析、JSON 信封                          │
│        ├─ Core：Module trait、注册表、配置、隔离恢复、安全模式   │
│        ├─ EventStore：开放式结构化事件期刊                      │
│        ├─ Modules：cpu / battery（只读） / wakelock / mem / ... │
│        └─ Daemon：漂移修复、自适应、历史采样、熄屏切换监听       │
│                  │                                           │
│                  ▼                                           │
│        /data/adb/sysflow/                                    │
│        ├─ config.json · events/ · state/ · log/              │
│        └─ daemon.pid · guard_interval · history/              │
│                  │                                           │
│                  ▼                                           │
│        sysfs / procfs / devfreq / cpuset / dumpsys / getprop │
└──────────────────────────────────────────────────────────────┘
```

### 2.1 职责划分

| 角色 | 负责 | 不负责 |
|---|---|---|
| 模块 `sysflow` | 读写系统、策略执行、配置漂移守护、自适应与历史采样、事件期刊、配置与隔离回滚 | 任何 UI |
| APK | 展示、交互、风险提示、触发 CLI；使用 Room（`sysflow-events.db`）做事件日志前台只读镜像 | 不直接读写 sysfs、不保存系统配置、不推断/伪造事件、不后台常驻 |
| WebUI | 不装 APK 时的**救援入口**：状态、安全模式、恢复默认、漂移检查、诊断与日志尾部 | 模块调节页、图表、事件页、进程大列表（这些只在 APK） |

### 2.2 核心原则

1. **CLI 是唯一入口**：APK 与 WebUI 都只调用 CLI，配置文件和事件期刊只有 CLI 读写。
2. **set 返回读回值**：写入后重新读取真实值返回，UI 以实际值为准；对动态限频与含离线核心的 cpuset 启用 `persist_on_mismatch`，保留用户静态意图。
3. **能力探测先行**：每个功能必须有 `probe()`，不支持则隐藏或标记不可用原因。
4. **事件与历史由模块记**：APK 不在后台，无法持续采集；系统事件期刊和未来的曲线历史必须由模块侧记录。
5. **每次 su 尽量拿全**：APK 启动及前台轮询通过 `snapshot` 一次性拿回各模块状态、`settings`、`defaults` 与 `errors`。

---

## 3. 技术选型

### 3.1 模块端

| 项 | 选择 | 说明 |
|---|---|---|
| 语言 | Rust | 静态链接、体积小、无运行时依赖 |
| 目标 | `aarch64-linux-android`（当前 `customize.sh` 仅支持 `arm64`） | 用 `cargo-ndk` 构建 |
| 参数解析 | `clap`（derive） | 子命令 + 外部模块子命令路由 |
| 序列化 | `serde` + `serde_json` | 协议、配置、模块状态与事件 |
| 错误处理 | 自定义 `Error` + `ErrorCode`（未引入 `thiserror`） | 映射到协议错误码 |
| 日志 | 内置轻量文件日志，带大小轮转（512KB × 3，未引入 `tracing`） | 区分主日志、守护日志、启动日志与迁移日志 |
| 异步 | 不用 tokio；daemon 用单线程循环 + 睡眠，文件锁防并发写 | 降低体积与功耗 |
| 打包 | KernelSU / Magisk / APatch 通用模块格式 | `module.prop`、`customize.sh`、`service.sh`、`uninstall.sh`、`migrate-legacy*.sh`、`webroot/` |

> 体积优化：`opt-level="z"`、`lto=true`、`codegen-units=1`、`panic="abort"`、`strip=true`。

### 3.2 APK 端

| 项 | 选择 | 说明 |
|---|---|---|
| 语言 | Kotlin | |
| UI | Jetpack Compose + **Material 3 Expressive** | opt-in `ExperimentalMaterial3ExpressiveApi`；定制硅青品牌色与亮暗色对比度校验 |
| 架构 | MVVM + 单向数据流（UDF） | `StateFlow` + `ViewModel` |
| 依赖注入 | Koin | 已确定采用，装配 Root/Client/Room/ViewModel |
| Root 通信 | `libsu`（topjohnwu） | 复用常驻 root shell，降低每次 `su` 延迟，带 15s 超时 |
| JSON | `kotlinx.serialization` | 与 Rust 生成类型 `Protocol.kt` 一一对应 |
| 图表 | Compose Canvas 自绘 | 电池 24 小时曲线已用自绘 `HistoryChart`（电池页内部组件），暂未引入 Vico；内存曲线阶段 3 复用 |
| 形状动画 | `androidx.graphics:graphics-shapes` / `MaterialShapes` | M3 Expressive 形状、波浪进度与空间动画 |
| 导航 | 状态驱动导航 + `FloatingNavBar` + `FlexibleSubPage` | 支持 `BackHandler`；`NavigationRail` 平板适配留待阶段 6 |
| 本地存储 | **Room**（`sysflow-events.db`，v2 + Schema 导出） | 仅作模块事件日志的本地只读缓存（30 天 / 5,000 条）；UI 偏好 DataStore 暂未引入，不存系统配置 |
| 最低版本 | minSdk 29（Android 10），targetSdk 最新 | 已确定 |

### 3.3 WebUI

- **静态救援控制台**：`module/webroot/{index.html,app.js,app.css}` 三个文件，共 527 行 / 约 17 KB，纯 HTML/CSS/JS。
- 直接调用 KernelSU WebView 注入的全局 `ksu.exec(command, optionsJson, callbackName)`，不经 `kernelsu` npm 包；**无 npm、无 Vite、无构建步骤**，模块打包只校验三个文件存在。
- 直接解析 CLI 的 JSON 信封，因此不需要 TypeScript 类型模型。
- 定位与取舍见 `docs/webui-console.md`：它只在 APK 无法安装/启动或配置导致设备异常时保证模块可救，不做功能对齐。

---

## 4. 仓库结构（Monorepo）

```
sysflow/
├─ README.md
├─ build-sysflow-module.sh        # ARM64 release 二进制 + 静态 WebUI 校验 + 模块 ZIP/SHA256（不含 APK）
├─ build.gradle.kts · settings.gradle.kts · gradle.properties · gradlew(.bat) · gradle/
├─ docs/
│  ├─ development-plan.md         # 本文档
│  ├─ protocol.md                 # 协议规范（第 5 章的正式版）
│  ├─ event-system.md             # 开放式系统事件契约
│  ├─ webui-console.md            # 模块救援控制台说明
│  ├─ mem-module-plan.md          # 内存模块交付计划与逐 Step 记录
│  ├─ wakelock-hardening-plan.md  # 唤醒锁硬化（P0–P3）完成记录
│  └─ modules/                    # sysfs 路径与设备差异记录（cpu.md、battery.md、wakelock.md、mem.md）
├─ core/                          # Rust workspace（当前 version 0.2.2）
│  ├─ Cargo.toml
│  ├─ sysflow-core/               # trait、信封、错误码、配置与隔离恢复、事件期刊、安全模式、Sysfs、Logger、screen、history、batterystats、wakeup_sources
│  │  └─ registry/                # daemon.rs（守护循环/采样调度/熄屏切换）、boot.rs（开机/恢复/安全模式）
│  ├─ sysflow-modules/            # cpu（mod/adaptive/cpuset/gpu/mem/schema/screen/state/walt）+ battery + wakelock + mem + mock
│  ├─ sysflow-cli/                # 二进制入口（clap）+ 各 verb 实现（battery_apps/health、wakelock apps/held/wakeup、mem apps/breakdown/probe）
│  └─ sysflow-types/              # 协议模型 + 类型生成（--kotlin / --ts）
├─ module/                        # 模块打包模板
│  ├─ module.prop
│  ├─ customize.sh                # arm64 校验、放置二进制、PATH shim、停旧 daemon、caps 自检
│  ├─ service.sh                  # 开机编排、事件埋点、apply-boot、daemon start、battery_loop、boot-confirm
│  ├─ uninstall.sh
│  ├─ migrate-legacy.sh           # 清理旧版 SoCFlow msm_performance 与 perfsdkserver 残留
│  ├─ migrate-legacy-doze.sh      # 清理旧版 SoCFlow sleep-governor 遗留进程与电源状态
│  └─ webroot/                    # index.html / app.js / app.css（静态救援控制台，无构建步骤）
├─ app/                           # Android 工程（单 :app + 包边界；第 10.2 节说明拆分规划）
│  ├─ schemas/                    # Room 导出 Schema JSON（v2）
│  └─ src/
│     ├─ main/kotlin/io/sysflow/
│     │  ├─ core/{model,root,event,ui}
│     │  │  └─ ui/{components,theme}   # Expressive 组件、Chart（Sparkline/LineChart）、主题
│     │  ├─ feature/{home,cpu,adaptive,screenoff,cpuset,wakelock,mem,settings,diag}
│     │  └─ ui/{navigation,GuideScreen.kt,SysFlowApp.kt}
│     └─ test/kotlin/io/sysflow/  # CpuLiveTracker、ThemeContrast、CpusetFormat、MemModels、WakelockModels、BatteryDrain、CoreOnlineWriteRace、MirrorOrphanRule、CpuDraftDiscard
├─ tools/
│  ├─ gen-types.sh                # 从 Rust 生成 Kotlin 协议类型
│  ├─ pack-module.sh              # 打包 zip + .sha256
│  └─ tests/                      # migrate-legacy-smoke.sh、migrate-legacy-doze-smoke.sh
├─ dist/                          # sysflow-module-v0.2.2.zip + .sha256
└─ .github/workflows/             # CI（待补全）
```

---

## 5. 通信协议规范 v1

### 5.1 CLI 总览

```text
sysflow version                         # 二进制版本 + 协议版本 + 配置版本
sysflow info                            # 静态 SoC、主板、内核与 Android 版本信息
sysflow caps                            # 各模块能力探测
sysflow snapshot                        # 所有已支持模块的当前 state + settings + defaults + errors
sysflow check                           # 只读漂移检查
sysflow <module> schema                 # 该模块可调项与只读遥测描述
sysflow <module> get [--keys a,b]       # 读取当前值
sysflow <module> set '<json>' [--persist]
sysflow <module> reset                  # 恢复默认
sysflow reset --all                     # 恢复全部支持模块的默认快照
sysflow history <module> --since 24h [--step 5m]
sysflow cpu rebaseline                  # 用 cpuinfo_min_freq 重置 cpu.min_freq 默认值
sysflow battery apps [--limit N] [--refresh]   # per-UID 耗电归因
sysflow battery health [--refresh]      # 循环计数与健康度（带来源）
sysflow wakelock held [--refresh]       # dumpsys power 当前持有的用户态锁
sysflow wakelock apps [--limit N] [--refresh]  # 上次充电以来的 per-app 唤醒时长
sysflow wakelock wakeup [--refresh]     # 上次唤醒原因
sysflow mem apps [--limit N] [--refresh]       # per-UID PSS 排行
sysflow mem breakdown [--refresh]       # 系统 / 用户应用 PSS 拆分
sysflow mem probe                       # 隐藏调试探针（输出可能变化）
sysflow boot status | configure [--enabled true|false] [--delay <0..300>]
sysflow apply-boot [--no-delay]         # 开机应用持久化配置（禁用时仍单独应用 walt_guard）
sysflow boot-confirm                    # 确认本次开机正常，开机计数清零
sysflow safe-mode status|clear
sysflow recovery status|restore|clear|clear-corrupt
sysflow daemon start|stop|status|configure [--enabled true|false] [--interval <2..120>]|run
sysflow enforce                         # 隐藏命令：执行一次持久化漂移修复与自适应推进
sysflow note <message>                  # 追加日志备注（兼容旧版事件注释）
sysflow event emit|list|prune|clear     # 结构化系统事件期刊读写与维护
sysflow log [--tail 200]
```

全局参数：`--pretty`（人读格式）。（原设想的 CLI `--timeout <ms>` 当前由客户端 `RootExecutor` 侧统一以 15,000 ms 超时控制，CLI 侧暂未单设该参数。）

### 5.2 统一信封

```json
{"v":1,"ok":true,"data":{}}
{"v":1,"ok":false,"error":{"code":"unsupported","msg":"…","detail":{}}}
```

错误码（固定集合）：

| code | 含义 | UI 行为 |
|---|---|---|
| `unsupported` | 机型/内核不支持 | 隐藏对应功能（或整模块不可用时进引导页） |
| `permission` | 无 root 或 SELinux 拒绝 | 引导检查 root / 模块状态 |
| `invalid_arg` | 参数不合法 | 回滚 UI 控件到当前值，提示 |
| `busy` | 正在被其他操作占用 | 稍后重试 |
| `io` | 读写失败 | 提示并附日志/诊断入口 |
| `safe_mode` | 安全模式中，调节被禁用 | 顶部横幅提示并提供"退出安全模式" |

### 5.3 `caps`

```json
{"v":1,"ok":true,"data":{
  "sysflow_version":"0.2.2","protocol":1,"safe_mode":false,
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

`features` 是各模块自有的自由字符串（不是协议枚举），客户端遇到未知值应忽略。电池的字符串就是
本机存在的 sysfs 字段名，所以它同时回答"这台设备能给出哪些读数"。`cpuset` **不是** caps 特性：
只有存在 `/dev/cpuset` 时，`cpu schema` 里才会出现 `cpu.cpuset.*`。不支持的模块返回
`"supported":false,"features":[],"reason":"…"`；唤醒锁在 debugfs 不可读时**仍然受支持**
（拦截不依赖它），只是失去 `kernel_sources` / `userspace_held` / `history`。

### 5.4 `schema`

```json
{
  "module":"cpu","supported":true,
  "items":[
    {"id":"cpu.online","type":"bool_list","count":8,"locked":[0],
     "risk":"medium","persist":true,"title_key":"cpu.online"},
    {"id":"cpu.max_freq","type":"int_enum","scope":"cluster",
     "options":[1804800,2208000,2419200],"unit":"kHz","risk":"low"},
    {"id":"cpu.governor","type":"enum","scope":"cluster",
     "options":["schedutil","performance"],"risk":"low"}
  ]
}
```

支持的 `type`：

| type | 附加字段 | UI 控件 |
|---|---|---|
| `bool` | | Switch |
| `bool_list` | `count`、`locked[]` | 分组 Chip / 核心矩阵开关 |
| `int_range` | `min`、`max`、`step`、`unit` | Slider / 数值输入 |
| `int_enum` | `options[]`、`unit` | SegmentedButton / ButtonGroup / 下拉 / Slider |
| `enum` | `options[]` | SegmentedButton / Chip 组 / 下拉 |
| `text` | `max_len`（可选） | 文本框（如 WALT `target_loads`） |
| `readonly` | `unit`、`format` | 文本 / 仪表 / 遥测数据 |
| `action` | `confirm`（可选） | 按钮（如 trim、drop_caches） |

通用字段：`id`、`scope`（`global`/`cluster`/`cpu`/`app`）、`risk`（`low`/`medium`/`high`）、`persist`、`depends_on`（依赖其他项）、`title_key`（用于 i18n，UI 缺失时回退到 id）。CPU 的扩展能力（WALT、GPU、DDR/LLCC、cpuset、熄屏、自适应）沿用这套类型与 `cpu.` 前缀，清单见 `docs/modules/cpu.md` 与 `docs/protocol.md`。

### 5.5 `get` / `set` / `reset` 语义

- `get` 返回 `{id: value}` 的平铺字典；`scope` 为 `cluster` 时值为 `{ "0": v, "4": v }`。
- `set` 接受同样结构的**子集**；返回**读回后的实际值**与 `mismatch`：

```json
{"v":1,"ok":true,"data":{
  "applied":{"cpu.online":[1,1,1,1,1,1,0,0]},
  "requested":{"cpu.online":[1,1,1,1,1,1,0,0]},
  "mismatch":[]
}}
```

- `mismatch` 非空时客户端可进一步对照 `requested` 与 `applied` 格式化详细差值提示，并以读回值刷新界面。
- 当自适应引擎或熄屏限频生效，导致实测 `scaling_max_freq` 合理低于静态请求上限时，模块识别为合法动态封顶，不记入 `mismatch`。
- `--persist`：写入 `config.json` 对应模块段，开机由 `apply-boot` 应用。默认跳过 `mismatch` 项，但对 `persist_on_mismatch == true` 的项（如 `cpu.max_freq`、`cpu.min_freq`、`cpu.cpuset.*`），即使因动态封顶或存在离线核心而导致实时读回不完全一致，依然持久化用户的静态配置意图。
- 原子性：同一次 `set` 内多项按依赖顺序执行；遇到不可用簇直接以 `invalid_arg` 拒绝；核心/簇/WALT/GPU/DDR/cpuset 任一步写入失败时尝试回滚该次已写项。

### 5.6 `snapshot`

一次返回所有已支持模块的当前 `get` 结果、已保存静态配置 `settings`、首次默认快照 `defaults` 以及单模块采集 `errors`：

```json
{"v":1,"ok":true,"data":{
  "ts":1767000000,
  "safe_mode":false,
  "modules":{"cpu":{...},"battery":{...},"mem":{...}},
  "settings":{"cpu":{...}},
  "defaults":{"cpu":{...}},
  "errors":{"battery":{"code":"io","msg":"…"}}
}}
```

### 5.7 `history`

```json
{"v":1,"ok":true,"data":{
  "module":"battery","step_s":300,"daemon_running":false,
  "series":{
    "level":[[1767000000,83],[1767000300,82]],
    "temp":[[1767000000,312]]
  }
}}
```

时间戳为 Unix 秒；缺失点用 `null`；`--step` 由 CLI 按桶取最新点降采样（保留峰值而非求均值）；`daemon_running` 反映守护进程的真实存活状态，`points` 是本次返回的桶数。电池以 60 秒、唤醒锁与内存以 300 秒间隔采样，守护进程追加写入 `history/<module>.jsonl`；文件保留 30 天 / 20 000 行，超限时从最旧记录裁剪，断电产生的截断行会被丢弃而不是污染整个文件。详见 `docs/protocol.md` §3.9。

### 5.8 结构化系统事件（开放世界协议）

详见 `docs/event-system.md` 与 `docs/protocol.md` §3.10。事件记录统一由 Root 模块写入 `/data/adb/sysflow/events/`（每条事件独立原子 JSON 文件）：

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

- `module`、`category`、`event_type` 为开放字符串（`[A-Za-z0-9._-]`），新增模块无需修改客户端枚举或 Room Schema；`severity` 固定为 `info | notice | warning | error | critical`。
- 支持 `--dedupe-key` + `--dedupe-window-s` 去重、双向游标分页（`--after-ms`/`--after-id`、`--before-ms`/`--before-id`、`--ascending`）与 30 天 / 5,000 条保留裁剪。

### 5.9 版本兼容

- `protocol` 为整数主版本，不兼容变更才 +1。
- 新增字段只增不删，客户端忽略未知字段。
- 客户端内置 `PROTOCOL_VERSION`，与模块返回的 `version.protocol` **严格相等**才继续；不等时拒绝
  调用并提示"协议版本不匹配：模块 X，应用 Y。请更新模块或应用。"（**没有** `MIN_SYSFLOW_PROTOCOL`
  这种"低了提示更新模块、高了提示更新应用"的分支）。因此协议号 +1 必须模块与 APK 同步发版，
  "只增不删"正是让这件事很少发生的原因。

---

## 6. 模块端设计

### 6.1 Core 抽象

```rust
pub trait Module: Send + Sync {
    fn id(&self) -> &'static str;
    fn probe(&self) -> Caps;                 // 能力探测
    fn schema(&self) -> Schema;
    fn collect(&self, keys: Option<&[String]>) -> Result<Value>;     // get
    fn apply(&self, req: &Map<String, Value>) -> Result<ApplyResult>; // set，返回读回值 + mismatch
    fn apply_with_settings(&self, req: &Map<String, Value>, settings: &Map<String, Value>) -> Result<ApplyResult>;
    fn persist_on_mismatch(&self, item_id: &str) -> bool { false }   // 动态封顶/离线核场景仍持久化请求
    fn drift(&self, settings: &Map<String, Value>) -> Result<Vec<String>>; // 只读漂移检查
    fn normalize_default(&self, current: Map<String, Value>) -> Map<String, Value>;
    fn sanitize_boot(&self, req: &Map<String, Value>) -> Map<String, Value>;
    fn reset(&self) -> Result<()> { Ok(()) } // 额外清理；默认值恢复由 Registry 统一做
    fn in_snapshot(&self) -> bool { true }   // 进程模块返回 false
    fn sample(&self) -> Option<Result<Sample>> { None }   // 仅需历史曲线的模块实现
    fn sample_interval(&self) -> Option<Duration> { None }
}
```

- **Registry**：启动时遍历所有模块 `probe()`，不支持的不进入 snapshot；统一管理 `defaults_snapshot` 捕获、配置合并、隔离恢复、自动配置变更/重置/安全模式/daemon 事件上报。
- **ApplyResult**：`{applied, requested, mismatch}`。
- **Sysfs 工具层（`sysfs.rs`）**：`read_u64 / read_str / write_verified`，统一处理 `EPERM / EBUSY / ENOENT` 与 `SYSFLOW_SYSFS_ROOT` 测试路径重映射；针对三星等 ROM 将 `cpufreq` 节点设为只读 `0444` 的情况，写入失败时临时放宽至 `0644` 写完再还原。

### 6.2 配置文件 `/data/adb/sysflow/config.json` 与数据目录

```json
{
  "version": 1,
  "boot": {"delay_s": 30, "enabled": true},
  "safe": {"boot_count": 0, "threshold": 3, "safe_mode": false},
  "modules": {
    "cpu": {"cpu.online":[1,1,1,1,1,1,0,0]},
    "mem": {"mem.swappiness": 60}
  },
  "defaults_snapshot": {},
  "recovery_backup": null
}
```

- 仅 CLI 写；写入使用 `config.json.lock`（10s 过期自动接管）+ 临时文件 `fsync` + `rename` 保证原子性；解析损坏时自动备份为 `config.json.corrupt`。
- 配置带 `version`，升级时做迁移；安全模式隔离激活时旧 `modules` 自动迁入 `recovery_backup`。
- 其他运行期目录与文件：
  - `state/cpu.json` & `state/cpu.lock`（带 `/proc/<pid>` 存活校验）：WALT 原始值、熄屏前基线、自适应迟滞计数器及 GPU/总线原始值。
  - `state/wakelock.json`：24 h 排行基线（`version` / `boot_id` / 基线计数 / 上次观测），由 300 s `sample()` 推进。
  - `state/mem.json`：内存历史差分基线 + PSI 迟滞状态 + 争用（contested）护栏。
  - `events/`：开放式系统事件单文件期刊与 `.last_prune_ms`。
  - `history/<module>.jsonl`：守护进程写入的时序采样（`battery` 60 s、`wakelock` / `mem` 300 s，30 天 / 20 000 行保留）。
  - 60 秒查询缓存：`state/battery_apps.json`、`state/battery_health.json`、`state/wakelock_apps.json`、`state/wakelock_held.json`、`state/wakelock_wakeup.json`、`state/mem_apps.json`、`state/mem_breakdown.json`；另有 10 分钟的 `state/mem_packages.json`（UID→包名）。
  - `daemon.pid`、`guard_interval`（2–120 s）、`no_enforce`。

### 6.3 模块打包

**`module.prop`**：`id=sysflow`，`versionCode` 与发布版本联动。

**`customize.sh`**：
1. 校验 `$ARCH == arm64`，将 `bin/arm64-v8a/sysflow` 放置为 `$MODPATH/bin/sysflow` 并设置权限（`service.sh`、`uninstall.sh`、`migrate-legacy.sh`、`migrate-legacy-doze.sh` 均设 `0755`）。
2. 初始化 `/data/adb/sysflow/log`（不存在才创建，升级不覆盖用户配置）。
3. 在 `/data/adb/{ksu,ap,magisk}/bin` 下建立指向**最终模块路径**的 `sysflow` 软链（不是 `$MODPATH`：管理器安装期把模块暂存在 `modules_update/`，指向暂存目录的链在重启后会变成悬空链）；`service.sh` 在 `boot_completed` 后会再补一次，因为安装期这些目录可能还没挂载。
4. 停止上一版仍在运行的 daemon（它持有旧二进制的 inode，不停则新功能要等到重启才出现）。
5. 执行 `sysflow caps` 作为安装期自检，输出结果（不阻断安装）。

**`service.sh`**（`late_start`，等待 `sys.boot_completed`）：
1. 发送 `module_service_started` 系统事件并执行 `event prune`（30 天 / 5,000 条）。
2. 执行一次性旧版 SoCFlow 兼容清理：`migrate-legacy.sh`、`migrate-legacy-doze.sh`。
3. 调用 `sysflow apply-boot`（内部按配置的 `delay_s` 延迟、`boot_count+1` 并应用配置；若开机应用已关闭仍可单独恢复 `cpu.walt_guard`）。
4. 若 `boot.enabled == true`、未处于安全/救援模式且开机应用无错误（`"errors":{}`），启动 `sysflow daemon start`（独立后台守护）。
5. 启动 `battery_loop` 监听电池状态转换（60 秒一次，产出充电/低电/过温事件）。熄屏检测由 daemon 负责，不再有 shell 侧循环。
6. 延迟 60 秒后，若开机应用无错则调用 `sysflow boot-confirm`，把 `boot_count` 清零。

> 熄屏切换、漂移修复、自适应推进与历史采样都由 daemon 单进程承担；`service.sh` 只保留开机编排与电池事件循环。`daemon stop` 会先释放仍生效的熄屏配置，避免留下过期的频率上限或离核状态。

**`uninstall.sh`**：先执行旧版清理脚本，再调用 `sysflow daemon stop` 与 `sysflow reset --all` 恢复默认，最后删除 `/data/adb/sysflow/`。

> 尽量避免 `post-fs-data.sh`，因为它阻塞开机，出错会增加变砖风险。

### 6.4 兼容性目标

- KernelSU / KernelSU Next / APatch / Magisk 均可安装；**WebUI 仅在支持的管理器中显示**。
- SELinux：所有写入经 `su` 域执行；daemon 与 `battery_loop` 以 `service.sh` 上下文运行；若遇到 `avc denied`，在日志中记录并在 `caps`/`errors` 中标明 `permission` 或原因。
- 避免依赖特定 ROM 的私有路径；私有路径一律在 `probe()` 里探测。

### 6.5 日志

- 主日志 `/data/adb/sysflow/log/sysflow.log`，单文件 512KB，保留 3 份。
- 每次 `apply` 记录：时间、模块、请求、实际结果、`mismatch`、耗时（ms）。
- 其他专用日志：`daemon.log`、`boot.log`、`legacy-migration.log`。
- `sysflow log --tail N` 供 APK 与 WebUI 的“诊断与日志”页显示与一键复制。

---

## 7. 功能模块详细设计

> 以下路径是常见 Android 内核的参考，**必须在 `probe()` 中验证存在性**，不要硬编码假设。

### 7.1 CPU（M1 · 已实现）

**数据（collect）**
- 每核在线状态：`/sys/devices/system/cpu/cpuN/online`
- 每核/每簇当前频率：`scaling_cur_freq` 与硬件 `cpuinfo_cur_freq`
- 簇划分：`/sys/devices/system/cpu/cpufreq/policyN/{related_cpus,affected_cpus}`
- 当前 governor、每簇独立 `scaling_available_governors`（`cpu.governor_table`）、上下限、硬件范围 `cpuinfo_{min,max}_freq`、每簇独立可用频率表（`cpu.freq_table` 与供熄屏滑块使用的原始表 `cpu.screen_off.freq_table`）
- 负载与频率统计：`stats/time_in_state`（`cpu.time_in_state`）与 `/proc/stat`（`cpu.stat` 与逐核 `cpu.usage_stat`）
- 温度与遥测：`/sys/class/thermal/thermal_zone*/{type,temp}`（`cpu.temp`、`cpu.thermal_sensors`、`cpu.thermal_sensors_all`）、`cooling_device*`（`cpu.cooling_devices`）、GPU busy（`cpu.gpu_busy`）、电池温度（`cpu.battery_temp`）、GPU 详情（`cpu.gpu_info`）

**已实现可调项（完整清单见 `docs/modules/cpu.md` 与 `docs/protocol.md`）**
| 分类 / id | 类型 | 风险 | 说明 |
|---|---|---|---|
| `cpu.online` | bool_list | medium | 核心开关；`cpu0` 与不可切换核心锁定；写后读回验证 |
| `cpu.max_freq` / `cpu.min_freq` | int_enum（cluster） | low | 校验 `min ≤ max`；`persist_on_mismatch` 保留静态上限 |
| `cpu.governor` | enum（cluster） | low | 按簇校验可用 governor |
| `cpu.gpu_min_freq` / `cpu.gpu_max_freq` / `cpu.gpu_governor` / `cpu.gpu_dcvs_boost` / `cpu.gpu_bus_boost` | int_enum / enum / bool | low / medium | Devfreq GPU（保留原始 Hz 字符串读写）与 KGSL DCVS/busmon 联动 |
| `cpu.ddr_boost_freq` / `cpu.llcc_boost_freq` | int_enum | medium | `/sys/devices/system/cpu/bus_dcvs/{DDR,LLCC}/boost_freq` |
| `cpu.walt.*`（12 项）与 `cpu.walt_guard` | int_range / text / bool | medium / high | `adaptive_high_freq`/`adaptive_low_freq` 在限频时自动压零；`walt_guard` 将 7 个节点置 `0444` |
| `cpu.cpuset.<group>` | bool_list | medium | `/dev/cpuset/<group>/cpus`（与在线及 possible 核求交集，持久化含离线核意图） |
| `cpu.screen_off` / `cpu.screen_off.max_freq` / `cpu.screen_off.cores` / `cpu.screen_off_active` | bool / int_range / bool_list | medium / low | 熄屏限频与额外关核（亮屏先恢复上线核心再恢复频率范围） |
| `cpu.adaptive` / `cpu.adaptive.{smart,thermal,battery,psi}` 及 20 项参数 | bool / int_range | medium / low | 智能负载 4 档、温度动态收紧、低电量节能、PSI 性能护栏 |

**注意**
- 关核可能被 thermal/vendor HAL 改回，靠读回值 + `mismatch` 告知。
- 先写 max 再写 min（或反之）要按"扩大范围先扩大、缩小范围先缩小"的顺序避免被内核拒绝；GPU 与熄屏恢复遵循同样原则。
- 持久化与热插拔：切换核心在线后等待 policy 目录就绪（`clusters_settled`）；若某簇未上线则以 `invalid_arg` 显式报错而非静默漏写；`persist_on_mismatch` 确保静态上限与 cpuset 选择不被临时动态状态覆盖。

### 7.2 电池（M2 · 已完成）

> **状态：已完成（2026-11-23）**。实现于 `core/sysflow-modules/src/battery/`，协议 §3.15–§3.17，
> 设备说明 `docs/modules/battery.md`。
> - **只读遥测**：`capacity/status/charging/temp/voltage/current/power/health/health_percent/
>   cycle_count/charge_full/charge_full_design/ac_online/usb_online`，另含无线充电
>   （`wireless_online`）、库仑计（`charge_mah`）、协商模式与电流（`usb_type` /
>   `input_current_limit`）与快充判定（`fast_charging`）；µV/mA/µAh/mW 按量级归一化。
> - **循环计数四级兜底**：sysfs 多节点 → uevent → Samsung EFS（`dumpsys batteryproperties`）→
>   Health HAL（`dumpsys health`）；APK 另取框架值（Android 14 `EXTRA_CYCLE_COUNT`、
>   `BATTERY_PROPERTY_CHARGE_COUNTER`）。模块整体只读：`set` 一律 `invalid_arg`，不参与漂移修复与开机回放。
> - **应用耗电排行**：`sysflow battery apps`（协议 §3.16），解析 `dumpsys batterystats` 的 per-UID
>   归因，解析器位于 `core/sysflow-core/src/batterystats.rs`，供唤醒锁/进程模块复用。
> - **事件**：`service.sh` 的 `battery_loop` 产出充电开始/结束、充满、低电量与过温事件（带去重键）。
> - **历史采样**：`Module::sample()` 60 s，守护进程写入 `history/battery.jsonl`，`sysflow history` 降采样查询。
> - **APK**：`feature/home/BatteryScreen.kt` 提供电量/电气参数/会话内趋势/健康度/应用耗电排行，
>   并基于磁盘历史绘制 24 小时电量曲线与「上次充电以来掉电」。
>
> **现场景**：可调充电项仍未实现（风险 high，默认不开放）；息屏掉电速率拆分**已实现**——daemon 在每个采样行写入 `"screen":1|0`，`history` 把它作为 `screen` 序列返回，APK 据此拆分。

**数据**
- `/sys/class/power_supply/battery/` 下：`capacity`、`status`、`temp`、`voltage_now`、`current_now`、`charge_counter`、`cycle_count`、`charge_full`、`charge_full_design`
- 计算：功率 = V × I；健康度 = `charge_full / charge_full_design`（若可得）
- 充电器类型、USB/AC 在线状态（`power_supply/*/online`）
- 单位差异（µA/mA、µV/mV）在 Rust 侧归一化，协议只输出标准单位（mA、mV、0.1°C）。

**可调项（可选，探测到才有）**
- 充电限流/充电上限（`charge_control_limit`、`input_suspend` 等因机型而异）——**风险 high**，默认不开放，放入"实验功能"。

**历史（sample）· 已实现**
- 每 60s 采样 `level / temp / voltage / current / power / charging` 写入 `history/battery.jsonl`，保留 30 天 / 20 000 行。
- 采样行附带 daemon 观测到的显示状态（`"screen":1|0`，无法判定时省略），`sysflow history` 将其
  作为单独的 `screen` 序列返回。

**派生统计 · 已实现**
- 最近一次充满以来的耗电（APK 由 24 小时历史计算）；充电功率曲线由会话内趋势呈现。
- 亮屏 / 熄屏掉电速率（%/h）由 `screen` 序列拆分得到；升级前写入的历史行没有该字段，因此该拆分
  需要积累若干小时新样本后才会有值。

### 7.3 唤醒锁（M2 · 已完成）

> **状态：两轮均已完成（2026-11-23）**。实现于 `core/sysflow-modules/src/wakelock/`
> （`mod.rs` 模块、`appops.rs` 拦截），解析器在 `core/sysflow-core/src/wakeup_sources.rs`，
> 协议 §3.18–§3.21，设备说明 `docs/modules/wakelock.md`。
> - **读写分离**：读半边（`kernel_sources` / `ranked` / `summary`）只读遥测，进 `snapshot`；
>   写半边（`wakelock.blocked`）通过 AppOps 拦截，risk medium，参与 apply / 漂移修复 / 开机回放。
> - **数据来源**：`/sys/kernel/debug/wakeup_sources`（9 列，毫秒）优先，退化到
>   `/proc/wakelocks`（8 列、无 `event_count`，纳秒）；两者都不可读时模块仍可用（拦截不需要
>   debugfs），只是 `summary.available=false`、内核项为空。
> - **快照项**：`wakelock.kernel_sources`（最多 24 条，活跃优先）、`wakelock.ranked`
>   （24 小时窗口内 `awake_ms` 与占比，最多 15 条）、`wakelock.summary`。
> - **24 小时窗口**：内核计数器是自开机累计值，因此模块在 `state/wakelock.json` 保存基线，
>   由 daemon 每 300 秒的 `sample()` 推进；窗口超过 24 小时滚动到上一次观测值而非清零；计数器回退
>   （期间重启）按 0 计；新装设备在第二次采样前排行是空的。
> - **用户态 wakelock**：`sysflow wakelock held [--refresh]` 解析 `dumpsys power` 的 `Wake Locks:`
>   段，按持有时长倒序，缓存 60 秒；`dumpsys` 失败返回 `io` 而不是空列表。
> - **历史采样**：每 300 秒记录 `known` / `active` / `top_ms` / `wakeups`（只有聚合值，不把每个
>   唤醒源展开成历史列，避免列数爆炸）。
>
> **第二轮（已完成，2026-11-23）**：
> - **AppOps 拦截**：`wakelock.blocked`（`bool`，scope `app`，risk medium，persist）以「包名 → 是否
>   ignore」的映射表达；`apply` 先整体校验再写入，单个应用失败会把已写入的应用回滚到原状态；
>   读回走 `cmd appops query-op WAKE_LOCK ignore`，ROM 不认时体现为 `mismatch`。
> - **白名单**：`android`、SystemUI、电话/Telecom/拨号/短信、`com.android.settings`、`io.sysflow`
>   拒绝拦截（`invalid_arg`）；允许解除；`reset` 跳过这些应用，绝不翻转不是本模块设置的系统默认值。
> - **持久化与开机重放**：`--persist` 写入 `config.modules.wakelock`；`apply-boot` 经 `sanitize_boot`
>   丢弃已卸载或已变成受保护的应用后重放；daemon 的 `drift()` 比对持久化意图与实时 AppOps 状态，
>   不一致即重放。
> - **按需读取**：`wakelock.blocked` 需要一次 binder 调用，**不进入 `snapshot`**，客户端用
>   `get --keys wakelock.blocked` 显式读取。
> - **APK 唤醒锁页**：活跃/已知唤醒源、24 小时排行（占比条）、当前持锁列表（按需刷新）、已拦截
>   应用管理，以及从已安装应用中选择拦截（受保护项禁用并说明原因）。
> - **内核唤醒源屏蔽不做**：内核没有针对「唤醒源名字」的可移植可写接口（`/sys/.../power/wakeup`
>   是设备级唤醒，名字到路径的映射只能靠猜），硬做会变成伪功能。
>
> **第三轮（硬化，2026-11-24）**：修复复核发现的缺陷并补齐能力，模块拆分为
`mod.rs` + `window.rs` + `debugfs.rs` + `appops.rs` + `policy.rs` + `read.rs` + `state.rs` + `power.rs`。
> - **读回解析（P0）**：APK 以前按包名读 `applied`，而协议是按 item id 组织的，于是永远拿到
>   `null` 再回退成"成功"——ROM 拒绝被静默吞掉。现在解包逻辑是 model 层的纯函数，三态（生效 /
>   未接受 / 未确认），并覆盖多包部分成功。
> - **跨重启窗口（P1）**：内核计数器重启归零而基线跨重启保留，导致 delta 恒为 0、排行"死"最多
>   一天，页面还把它解释成"设备空闲"。现在用 boot id → uptime → 计数回退三级判定重启并重建窗口，
>   `summary` 增 `window_reason` / `since_boot_s`，`window_s` 不超过本次开机时长。
> - **reset 越权（P1）**：`reset` 以前释放"所有能看到的 ignore"，会连带撤销用户在别处设的规则。
>   现在 `Module` 新增 `reset_with_settings`（默认委托 `reset`，其它模块零改动），只释放自己持久化
>   的包；无配置时不动系统。
> - **AppOps 还原（P2）**：解除拦截写 `default` 而非 `allow`——后者是永久显式放行，会覆盖 manifest
>   或 OEM 策略，属于"改动应用"而不是"还原"。
> - **其它 P2**：`share` 分母改为全部增量之和（并新增 `measured_ms`）；debugfs "挂载成功但表仍不可读"
>   也纳入小时级节流，避免 2 秒轮询反复 fork `mount`。
> - **P3**：持久化改为逐键（一个包被拒不再丢弃整个请求）；混合请求按条判定受保护；回滚失败会在错误
>   信息里点名；状态文件加版本迁移与 512 源上限。
> - **单源化**：保护名单作为 `wakelock.protected` 发布，APK 优先使用、旧模块回退内置表。
> - **A 类**：`wakelock.exempt` 用户白名单（豁免即释放、boot 清洗、drift 自愈）+ APK 保护名单与
>   全部恢复。
> - **B 类**：24h 排行同时给出唤醒次数（`ranked.wakeups`、`summary.total_wakeups`）与时长切换；
>   `sysflow wakelock wakeup` 上次唤醒原因（ROM 不提供时 `available:false` + note）；长持锁看门狗
>   （30 分钟阈值、6 小时去重、`wakelock_long_held` 事件）；历史写入 `wakeups` 并新增趋势曲线；
>   持锁列表支持一键拦截。
> - **验证**：`cargo test --workspace` 253 项、`clippy` 无告警、APK 单测 57 项、`assembleDebug` 通过。
> - **真机抽样（Samsung）**：`dumpsys power` 的归因段是空格分隔、无 `pkg=` 且带嵌套 `WorkSource`
>   括号（旧解析会截断 `pid`/`pkg`）；`mLastWakeUpReason` 大小写与 `mLastWakeTime=… (N ms ago)`
>   的现成年龄也已适配。`held` 现在会把只带 UID 的锁解析成可读 `owner`。两项均补了真机原文回归测试。
>   仍待真机确认：`appops set … default` 的兼容性与长持锁事件的噪音。

**补充（同轮）**：`sysflow wakelock apps` 提供 per-app 唤醒时间——`batterystats --charged` 的
> `All partial wake locks` 段由 `WakeLockScanner` 流式解析（读到段尾即杀进程，4 000+ 行 dump 不必读完），
> 按 UID 聚合、保留各自最长的 3 个 tag、60 秒缓存；APK 唤醒锁页有「应用唤醒时间」区块。
> 与电池页弹层的合并只是 UI 调整，数据已经具备。

**数据来源（按优先级探测，均已实现）**
1. 内核 `wakeup_sources`：`/sys/kernel/debug/wakeup_sources` 或 `/proc/wakelocks` → `kernel_sources` / `ranked`
2. `dumpsys power`（用户态 wakelock，含包名与 tag）→ `wakelock held` verb（60 s 缓存）
3. `dumpsys batterystats`（统计，较重，按需调用）→ `wakelock apps` verb（流式解析 + 60 s 缓存）

**注意**
- 拦截后要有"通知不到达"的风险提示；提供白名单默认项（IM、闹钟、电话、系统关键服务不允许拦截）。
- **debugfs 已改为按需挂载**：Android 新版本默认不挂 `/sys/kernel/debug`，因此表不可读时模块在
  Android 上执行一次 `mount -t debugfs debugfs /sys/kernel/debug`。`probe()` 保持无副作用
  （`snapshot` 每 2 秒调它），挂载发生在 `collect()` / `sample()` 里；失败按小时节流，"挂载成功但
  表仍不可读"也记在同一条节流上，避免 2 秒轮询反复 fork `mount`。表确实不可读时模块仍**受支持**，
  只是失去 `kernel_sources` / `userspace_held` / `history` 三个特性。

### 7.4 内存（M3 · 已完成 Step 0–8）

> 交付顺序、已定决策与逐 Step 记录见 `docs/mem-module-plan.md`；实测值、降级原因与性能数据见
> `docs/modules/mem.md`；协议条款见 `docs/protocol.md` §3.22–§3.24。实现位于
> `core/sysflow-modules/src/mem/`（`mod.rs` / `probe.rs` / `tunables.rs` / `history.rs`）。
> Step 9（zRAM 重配）暂缓，与阶段 4 一起重估；真机计时与事件噪音阈值仍待验。

**只读遥测**（进 `snapshot`，任何缺失字段都**省略**而不是报 0）

- `/proc/meminfo`：`mem.total/available/free/cached/anon/shmem/slab/dirty/swap_total/swap_free`，
  以及构成项 `mem.kernel_stack/page_tables/percpu/vmalloc_used/gpu/vendor.{rbin,cma,dmaheap}`
  （逐项可选；厂商项是**池大小**，不等于已用）。
- PSI：`/proc/pressure/memory` → `mem.psi.{some,full}_avg{10,60,300}`，单位 0.01 % 整数。
- swap：`/proc/swaps` 的 Used 按设备名拆成 `mem.swap.zram_kb` / `mem.swap.other_kb`。
- zRAM：`/sys/block/zram*/{disksize,mm_stat}` → `mem.zram.count/disksize/orig/compr/mem_used/
  ratio/writeback`；比值由求和后重算，不取各设备平均。
- `mem.mglru`：`/sys/kernel/mm/lru_gen/enabled` 非零即 1，用来诚实地提示 `swappiness` 会被稀释。
- 模块仅在 `/proc/meminfo` 本身不可读时整体 `unsupported`。

**可调项**（两条，`int_range` / `low` / persist，走 CPU 那套 `apply`/`drift`/`sanitize_boot`）

| id | 范围 | 路径 |
|---|---|---|
| `mem.swappiness` | 0..100，5.8+ 内核 0..200（上限读 `osrelease`，不猜） | `/proc/sys/vm/swappiness` |
| `mem.vfs_cache_pressure` | 1..1000（0 表示永不回收 dentry，不提供） | `/proc/sys/vm/vfs_cache_pressure` |

写入前**整体校验**，写后读回，`mismatch` 如实上报。ROM 反复改回时按"30 分钟内修 3 次即停手 6 小时"
处理并发一次 `mem_setting_contested`，而不是每个 tick 拉锯。

**明确不做**（理由见 `docs/mem-module-plan.md`）

- `dirty_ratio` / `dirty_background_ratio`：S25 用 `*_bytes`（100 MiB / 25 MiB），写 ratio 会清零
  ROM 的选择；且 ROM 已自行调过 `watermark_scale_factor` / `page-cluster`。
- `min_free_kbytes`：水位线，写大了正是它想避免的 OOM。
- zRAM 重配（`disksize` / `comp_algorithm` / `reset`）：需 `swapoff`，会把已用 swap 拉回内存。
- `drop_caches` 一键"清理/加速"：丢的是可回收 page cache。

**verb（不进 `snapshot`，均 60 s 缓存）**

- `sysflow mem apps [--limit N]`：`VmRSS` 粗排 → 前 N 读 `smaps_rollup` 的 `Pss`/`SwapPss` → 按 UID
  聚合；`pss_for_top_n_only` 标注估算范围。
- `sysflow mem breakdown`：**全量**进程 PSS，按 `uid < 10 000` 分系统/用户；与"已用"不可相加。
- `sysflow mem probe`：隐藏调试探针，输出各来源原始读数与模块的取舍判定，不进协议。

**历史**（`history mem`，300 秒一个点，键名不带 `mem.` 前缀）

`available_kb` / `swap_used_kb` / `zram_compr_kb` 为瞬时值；`psi_some_pct` / `psi_full_pct` /
`pgscan_direct` / `allocstall` / `swap_in_kb` / `swap_out_kb` / `refault` / `oom_kill` 为同一开机内
基线差分，用**实测经过时间**（`/proc/uptime`）作分母；重启后首个差分点留空而不是写 0。

**事件**（在 `sample()` 内产出，category `pressure`）

`mem_pressure_high`（≥3.00 % 连续 3 点）、`mem_pressure_recovered`（<1.00 % 连续 3 点，仅在 high
之后）、`mem_oom_kill`（内核 OOM 计数增量 > 0）、`mem_setting_contested`（category `settings`）。
阈值是占位常量，等安静期实测数据再定。

**APK**

`feature/mem/`（`MemModels` / `MemViewModel` / `MemScreen` / `MemOverview` / `MemTunables` /
`MemApps` / `MemComponents`）提供可用内存 hero、"整机/已用"两条构成条、swap/zRAM、PSI 24 h 曲线、
PSS 排行与单应用明细、两个滑块（范围取自 `schema`，保存才 `set --persist`）。页面在前台以 3 s
轮询，离开即停。

### 7.5 进程（M4 · Step 0 契约已冻结；cpuset 分组已先行在 CPU 模块实现）

> 交付顺序、字段口径、保护名单与待定问题见 `docs/process-module-plan.md`；设备路径矩阵见
> `docs/modules/process.md`（Step 1 回填）；协议草案见 `docs/protocol.md` §3.25。v1 只读 + 一次性
> 动作（`stop` / `oom` / `renice`），不持久化、不参与 daemon，`snapshot` 不含进程段。

**数据**
- `/proc/<pid>/{stat,status,cmdline,oom_score_adj,cgroup}` 解析：PID、包名、状态、CPU 占用（两次采样差分）、RSS、`oom_score_adj`、线程数。
- 前台应用：`dumpsys activity activities | grep mResumedActivity` 或 `cmd activity`（解析需容错）。

**操作**
- 强制停止：`am force-stop <pkg>`（risk medium）
- 调整 `oom_score_adj`（risk high）
- 调整优先级：`renice`、`chrt`（risk medium）
- cgroup 绑核（cpuset）：按应用限制到小核（risk medium，探测到 cpuset 才有）
- 冻结（若系统支持 cgroup freezer）：作为实验功能

**注意**
- 系统关键进程与 UID<10000 的服务一律不允许操作（硬编码保护名单 + 可配置）。
- CPU 占用需要在 CLI 内做两次读取间隔 ~500ms，因此 `process get` 单独耗时较高，**不放进 `snapshot`**，APK 进入进程页时再单独调用。

### 7.6 储存（M5 · 规划中）

**数据**
- 容量：`/data`、`/sdcard` 使用情况（`statvfs`）
- 块设备：`/sys/block/{sda,mmcblk0,nvme0n1...}`，`queue/scheduler`、`queue/read_ahead_kb`、`queue/nr_requests`
- IO 统计：`/proc/diskstats` 差分 → 读写吞吐、IOPS、利用率
- UFS/eMMC 健康（寿命估算）：路径因厂商不同，探测到再显示，否则隐藏
- 目录占用 Top N（按需、较重，异步任务 + 进度）

**可调项**
| id | 类型 | 风险 |
|---|---|---|
| `io.scheduler` | enum | low |
| `io.read_ahead_kb` | int_enum | low |
| `io.trim` | action（`fstrim -v /data`） | low |
| `io.discard` 相关 | | high，默认不提供 |

**历史**
- 5 分钟采样累计读写量，用于"写入量统计"（对寿命关注）。

---

## 8. 守护进程与历史数据

### 8.1 daemon 行为（已实现）

- 由 `service.sh` 通过 `sysflow daemon start` 拉起独立进程运行 `sysflow daemon run`（日志写至 `log/daemon.log`，PID 记录在 `daemon.pid` 并校检 `/proc/<pid>/cmdline`）。
- 可在客户端设置页通过 `daemon configure` 启停与调节间隔（2–120 秒，默认 10 秒，写入 `guard_interval` 与 `no_enforce`）。
- 每轮循环调用 `enforce()`：检查全模块只读漂移（`drift()`），仅对发生漂移的模块回放经过 `sanitize_boot()` 过滤的持久化配置；当启用 `cpu.adaptive` 时，即使无静态漂移也按间隔推进 CPU 自适应采样。
- **历史采样调度已接入**：`enforce()` 每轮比对各模块 `sample_interval()`，到期的模块调用 `sample()` 并追加到 `history/<module>.jsonl`。采样是尽力而为的：单个模块采样或写盘失败只记日志，不影响漂移修复与自适应推进。
- 模块升级不会自动重启守护进程（运行中的进程持有旧 inode），因此 `customize.sh` 会在安装时停止旧守护，`daemon start` 也会比对 `/proc/<pid>/exe` 与磁盘二进制并在不一致时重启。
- **熄屏切换已并入 daemon**（`core/sysflow-core/src/screen.rs`）：每轮以背光亮度做廉价变化检测，但**亮度原始值变化**（而不仅是 0/非 0 跳变）就会用 `dumpsys power`（3 秒超时）复核，另有 10 秒周期复核与 2 秒最小间隔，随后切换 `cpu.screen_off_active` 并发送 `display.*` 事件；`unknown` 不视为转换。
- **熄屏判定要二次确认**：亮屏立即释放（释放永远安全），灭屏需连续两次框架答案（≥2 秒）才应用，避免唤醒瞬间一次残留 `Dozing` 就把亮屏状态压成熄屏；每次转换写入 `screen transition state=… -> screen_off_active=…` 日志，写入失败会发 `screen_transition_failed` warning 事件；daemon 启动与停止都先释放残留的熄屏配置（`state/cpu.json` 里的 `screen_active` 会被后续任何 apply 重新应用，因此不能跨重启继承）。亮屏 5 秒、熄屏或待确认 1 秒轮询，且不超过守护间隔。
- **采样记录屏幕状态**：`history` 写入时在行内附带 `"screen":1|0`（无法判定时省略），`sysflow history` 将其作为 `screen` 序列返回，客户端据此拆分亮屏/熄屏掉电。

**待补**：
- 低功耗与保护：息屏时降低历史采样频率（如电池息屏 120s）；`nice 19` 下崩溃自拉起；单次采样总耗时上限（如 200ms），超时记日志并跳过。

### 8.2 存储格式（已实现）

- 目录：`/data/adb/sysflow/history/`，每模块一个 `<module>.jsonl`。
- 格式：JSON Lines，每行 `{"ts":<秒>,"v":{<指标>:<值>}}`。先用 JSONL 快速验证，稳定后再评估切定长二进制。
- 保留策略：按追加时裁剪，30 天 / 20 000 行上限，超限从最旧记录删除；断电造成的截断行直接丢弃，避免整文件不可读；`ts` 超前当前时间 5 分钟的记录会被拒绝，防止时钟跳变污染“最新值”。
- `history` 命令只读文件并按桶取最新点降采样，不做重计算；`daemon_running` 读取真实守护状态。

### 8.3 开关（待实现）

- `config.json` 中每个模块 `history.enabled / interval / retention_days`，APK 设置页可调（当前间隔由模块 `sample_interval()` 固定，保留策略为常量；救援控制台无设置页）。
- daemon 未运行时 `history` 返回 `ok` 但 `series` 为空，并带 `daemon_running:false`，UI 提示。

---

## 9. 安全与回滚机制

1. **默认快照**：首次对某项做 `apply` 之前，把当前值写入 `defaults_snapshot`（CPU/GPU 频率会先由 `normalize_default` 归一化为硬件范围，避免把瞬时温控/升频值当默认值）；`reset` 恢复到此快照（而非硬编码值）。
2. **开机保护与配置隔离（Recovery）**：
   - `apply-boot` 开始前 `boot_count += 1`；开机成功（`sys.boot_completed` + `service.sh` 延迟 60 秒窗口且 `apply-boot` 无错误）后调用 `boot-confirm` 清零。
   - `boot_count ≥ threshold(3)` ⇒ 自动进入 `safe_mode`：**将当前 `config.modules` 自动转移到 `config.recovery_backup` 隔离保存并清空激活配置**，跳过持久化调节。
   - APK 设置页展示安全状态、计数及隔离备份模块列表，支持用户审查后“恢复隔离配置”（仅放回配置不自动写 sysfs，需手动确认并退出安全模式）或“丢弃备份”，并支持清理损坏的 `config.json.corrupt`；救援控制台只提供 `safe-mode status` / `safe-mode clear` / `reset --all` 三个入口，不展示备份列表。
   - 若 `apply-boot` 本身出错，`service.sh` 不启动守护进程且不确认本次启动。
3. **风险分级**：
   - low：直接生效；
   - medium：APK 首次提示，之后可"不再提示"（阶段 1 的核心开关/进程组/熄屏已有内置范围与强校验保护；通用实验设置提示随阶段 3+ 完善）；
   - high：每次二次确认，且默认隐藏，需在"实验功能"中手动开启。
4. **保护名单**：关键进程/关键 wakelock/关键分区操作及 `cpu0`/不可下线核心的硬编码禁止列表。
5. **超时与看门狗**：客户端 `RootExecutor` 每次执行设 15s 超时；写 sysfs 若阻塞或权限异常则中止并返回 `io`/`permission`；文件锁带过期/PID 死锁自动接管。
6. **失败恢复**：集群切换、核心开关、GPU/WALT/DDR/cpuset 任一步写入失败时，尝试按逆序写回旧值；`sanitize_boot` 在开机遇到过期配置时逐项丢弃而非整模块报错；WALT `walt_guard` 在普通开机应用关闭时仍可单独恢复。
7. **一键恢复与旧版清理**：`sysflow reset --all`；APK 设置页与 WebUI 均提供入口；`migrate-legacy.sh` 与 `migrate-legacy-doze.sh` 负责一次性清理/还原旧版 SoCFlow 残留状态。
8. **救援入口**：数据目录或模块目录检测到 `disable_tuning` 标记文件时，`apply-boot`、`enforce` 与 `daemon start` 直接跳过（用户可通过 recovery/adb 创建，不依赖 APK）。

---

## 10. APK 设计（M3 Expressive）

### 10.1 设计原则

- **无后台**：不使用前台服务、WorkManager 周期任务、常驻通知。所有实时刷新仅在页面可见时用协程轮询（`repeatOnLifecycle(STARTED)`），页面不可见立即停止；事件日志仅在前台按 10s 节流同步至本地 Room 只读缓存。
- **专用交互优先于泛化渲染**：原设想的 `SchemaRenderer` 在阶段 1 初版后已演化为更贴合 Expressive 交互的专用页面（核心矩阵、簇双滑块、自适应策略、熄屏、进程组、诊断、事件历史）；未来简单模块设置项仍可复用受控组件。
- **状态以读回值为准**：`snapshot` 同时下发静态 `settings` 与实时 `modules`，使静态上限与动态封顶可清晰区分；任何 `set` 后以读回/刷新为准，遇到 `mismatch` 通过 `Snackbar` 与详情卡提示。
- **快速冷启动**：启动校验 root → 校验协议 → 获取 `caps` / `safeStatus` / `snapshot` / `info`，首屏就绪后再异步执行 Room 事件历史回填，不阻塞首帧展示。

### 10.2 工程模块划分

> 当前实现决定：APK 采用单个 `:app` Gradle 模块，用包边界模拟模块划分；构建更快，后续随规模增长拆分为独立 Gradle 模块不需要改业务代码。

```
当前包结构（app/src/main/kotlin/io/sysflow/） ↔ 规划多模块映射：
core/model       ↔ :core-model      # Protocol.kt（生成）、CpuModels、CpusetModels、BatteryModels、BatteryDevice、BatteryDrain、WakelockModels、SystemEvent
core/root        ↔ :core-root       # RootExecutor（libsu 封装）、SysFlowClient（suspend API）
core/event       ↔ :core-event      # Room EventDatabase（sysflow-events.db v2）、EventRepository、SystemEventDao、SystemEventEntity
core/ui          ↔ :core-ui         # M3E 主题（Color/Theme/Type）、Expressive 组件、FlexibleSubPage、Picker、Chart（Sparkline / LineChart）、AppIdentity
feature/home     ↔ :feature-home    # AppViewModel、HomeScreen、ModuleHubScreen、ModuleActivityScreen、EventHistoryScreen、EventCopy/EventUi、BatteryScreen + BatteryChart/BatteryAppsSection/BatteryTopApps/BatteryRankingScreen（暂放 home，后续拆 :feature-battery）
feature/cpu      ↔ :feature-cpu     # CpuScreen、CpuViewModel
feature/adaptive ↔ :feature-cpu     # AdaptiveScreen、AdaptiveViewModel
feature/screenoff↔ :feature-cpu     # ScreenOffScreen、ScreenOffViewModel
feature/cpuset   ↔ :feature-cpu     # CpusetScreen、CpusetViewModel
feature/wakelock ↔ :feature-wakelock# WakelockScreen（+ Sources/Apps/Held/Trend 四个 tab、AppPicker、Components）
feature/mem      ↔ :feature-memory  # MemScreen、MemViewModel、MemModels、MemOverview、MemTunables、MemApps、MemComponents（已接入）
feature/settings ↔ :feature-settings# SettingsScreen、SettingsViewModel
feature/diag     ↔ :feature-settings# DiagnosticsScreen、DiagnosticsViewModel
ui/              ↔ :app             # SysFlowApp、FloatingNavBar、Destination、GuideScreen
（后续规划：:feature-process / :feature-storage）
```

*(注：`feature/home/SettingsPlaceholderScreen.kt` 为早期占位文件，实际已由 `feature/settings/SettingsScreen.kt` 替代，待后续清理。)*

### 10.3 Root 通信层 `SysFlowClient`

原设计接口及已实现扩展能力：

```kotlin
// io.sysflow.core.root.SysFlowClient 的实际方法形状（ok=false 时直接抛 SysFlowException）
class SysFlowClient {
    suspend fun hasRoot(): Boolean
    suspend fun version(): VersionData
    suspend fun verifyProtocol(): VersionData          // 严格相等校验，不等即抛 ProtocolMismatch
    suspend fun caps(): CapsData
    suspend fun snapshot(): SnapshotData
    suspend fun info(): JsonObject
    suspend fun schema(module: String): Schema
    suspend fun get(module: String, keys: List<String> = emptyList()): JsonObject
    suspend fun set(module: String, payload: JsonObject, persist: Boolean): ApplyResult
    suspend fun reset(module: String): JsonObject
    suspend fun resetAll(): JsonObject
    suspend fun cpuRebaseline(): JsonObject
    suspend fun check(): CheckData
    suspend fun bootStatus(): JsonObject
    suspend fun bootConfigure(enabled: Boolean? = null, delaySeconds: Int? = null): JsonObject
    suspend fun daemonStatus(): JsonObject
    suspend fun daemonConfigure(enabled: Boolean? = null, intervalSeconds: Int? = null): JsonObject
    suspend fun safeStatus(): SafeStatus
    suspend fun safeClear(): SafeStatus
    suspend fun recoveryStatus(): JsonObject
    suspend fun recoveryRestore(): JsonObject
    suspend fun recoveryClear(): JsonObject
    suspend fun recoveryClearCorrupt(): JsonObject
    suspend fun logTail(lines: Int = 200): JsonObject
    // 事件
    suspend fun events(...): ModuleEventPage
    suspend fun clearEvents(): JsonObject
    // 按需 verb（均已接入页面）
    suspend fun history(module: String, since: String = "24h", step: String = "1h"): JsonObject
    suspend fun batteryApps(limit: Int = 30, refresh: Boolean = false): JsonObject
    suspend fun batteryHealth(refresh: Boolean = false): JsonObject
    suspend fun wakelockHeld(refresh: Boolean = false): JsonObject
    suspend fun wakelockApps(limit: Int = 20, refresh: Boolean = false): JsonObject
    suspend fun wakelockWakeup(refresh: Boolean = false): JsonObject
    suspend fun memApps(limit: Int = 10, refresh: Boolean = false): JsonObject
    suspend fun memBreakdown(refresh: Boolean = false): JsonObject
    suspend fun raw(command: List<String>): String      // 逃生口，仅供诊断
}
```

- 使用 `libsu` 的 `Shell.cmd(...)`，复用进程级常驻 root shell，避免每次 `su` 的 fork 开销；若未取得 root 会关闭缓存非 root shell 以便下次重试授权。
- 统一解析信封：`ok=false` → 映射为 `SysFlowException`（`Unsupported / Permission / InvalidArg / Busy / Io / SafeMode / ProtocolMismatch / NoBinary`）。
- 所有调用在 `Dispatchers.IO` 执行，带 15s 超时；写操作用 `Mutex` 串行。
- 命令参数一律经单引号转义函数 `quote()` 受控拼接，避免 shell 注入。
- 启动时检测：root 是否授予、`sysflow` 是否存在、协议版本是否兼容 → 对应 `GuideScreen`。

### 10.4 状态管理

```kotlin
// 通用状态模型参考；阶段 1 各具体页面按场景使用专用 UiState（如 CpuUiState、AdaptiveUiState 等）
data class ModuleUiState<T>(
    val loading: Boolean = true,
    val data: T? = null,
    val pending: Set<String> = emptySet(),   // 正在 set 的 item id
    val error: SysFlowError? = null,
    val mismatch: List<String> = emptyList()
)
```

- 每个 feature 一个 `ViewModel`，暴露 `StateFlow<*UiState>`。
- 当前已实现轮询策略（`repeatOnLifecycle(STARTED)`，离开页面即停）：全局 `snapshot` 2s 刷新
  （`SysFlowApp`，`HomeScreen` 与电池页共用该快照流）、`CpuScreen` 1.5s、`AdaptiveScreen` 2s、
  `ScreenOffScreen` 2s、`CpusetScreen` 2.5s、`MemScreen` 3s（内存是慢变量，3s 比 CPU 的 1.5s 更省）；
  `EventRepository` 10s 节流同步。**唤醒锁页不轮询**：它在切换 tab 或点刷新时按需调用
  `wakelock held|apps|wakeup`（各自 60s 缓存），因此没有后台开销也没有节流常量。未来进程页沿用
  可见期轮询（进程 2s）。

### 10.5 导航结构

- **顶层（已实现）**：M3 Expressive 底部悬浮导航栏 `FloatingNavBar`（`HorizontalFloatingToolbar`，随滚动显隐），**3 个主入口**：**总览 / 模块 / 设置**；点击总览卡片或模块中心进入各功能详情页。
- **二级详情页（已实现）**：采用 `FlexibleSubPage`（`LargeFlexibleTopAppBar`）+ `BackHandler` 统一返回栈：
  - 总览 → CPU 管理 / 系统事件
  - 模块 → CPU 管理 → 自适应性能管理 / 熄屏管理 / 进程组
  - 设置 → 自适应性能管理 / 诊断与日志
- **大屏适配（阶段 6 规划）**：平板/大屏引入 `NavigationRail` 与 `WindowSizeClass` 自适应；页面切换进一步精修共享元素与空间动画。

### 10.6 M3 Expressive 应用清单

| 元素 | 用法 | 当前状态 |
|---|---|---|
| `MaterialExpressiveTheme` + `MotionScheme.expressive()` | 全局主题与弹簧动画 | ✅ 已接入 |
| 自定义硅青品牌配色 + 动态取色（opt-in） | 配色与 WCAG 2.1 AA 对比度单测 | ✅ 已实现（固定品牌色，动态取色留为开关） |
| 更大胆的字体层级（Emphasized 字重） | 标题、关键数值（`headline*Emphasized`、`title*Emphasized`） | ✅ 已接入 |
| `MaterialShapes` + 形状变形 | 总览/模块/引导图标（Cookie6/7/9/12、Sunny、Clover4）、开关圆角形变 | ✅ 已接入 |
| `LoadingIndicator`（形变加载） | 取代传统 Circular，用于页面加载、`ApplyDock` 提交中 | ✅ 已接入 |
| `LinearWavyProgressIndicator` / `CircularWavyProgressIndicator` | CPU 在线/利用率与电池电量进度（`WavyMeter`），后续内存、存储进度复用 | ✅ `WavyMeter` 已用于 CPU 与电池 |
| `ButtonGroup` / `SplitButton` / `FilterChip` / `ToggleButton` | Governor 选择、cpuset 核选、预设按钮、"应用 / 应用并持久化" | 🟡 `FilterChip`/`ToggleButton` 已接入；`SplitButton` 待按需精修 |
| `FloatingToolbar` | 顶层 `FloatingNavBar`（已实现），以及未来进程页批量/历史页时间切换 | ✅ 导航已接入 |
| `LargeFlexibleTopAppBar` | 二级详情页标题，滚动折叠（`FlexibleSubPage`） | ✅ 已接入 |
| `SegmentedButton`（Connected Button Group 风格） | 诊断报告/模块日志切换；未来历史时间范围（1h/6h/24h/7d） | ✅ 诊断页已接入 |
| Switch 的 Expressive 形态 | 核心开关、守护开关、熄屏开关与核心下线开关（含图标拇指） | ✅ 已接入 |
| `ModalBottomSheet` / `AlertDialog` | 恢复默认、恢复隔离配置、清除熄屏配置确认（`AlertDialog`）；电池应用耗电排行弹层（`ModalBottomSheet`） | ✅ 均已接入 |
| `Snackbar` | 应用结果、"未完全生效"及诊断提示 | ✅ 已接入 |

> 注意：Expressive API 仍在持续演进，所有通用封装集中在 `core/ui`，以便后续升级 material3 版本时只改一处。

### 10.7 通用组件（`core/ui` / `:core-ui`）

- **已实现组件**：`ExpressiveCard`、`SectionLabel` / `SectionTitle`、`IndexBadge`、`StatusPill` / `InfoPill`（含语义色调 `StatusTone`）、`IconBadge`、`WavyMeter`、`ShapeStat`、`KeyValueRow`、`SegmentedValueList` / `segmentedShape`、`FlexibleSubPage`、`OptionPicker`、`cardPalette`、`onPrimaryContainerSwitchColors`、`AppIdentity`、`Chart.kt`（`Sparkline` + 泛化 `LineChart(points, minY, maxY, fill)` / `ChartPoint`）。
- **规划组件（随阶段 4–6 接入）**：
  - `SchemaRenderer`（用于未来规则型/简单调优页面）；
  - `StatCard`（大号数值 + 单位 + 迷你趋势）；
  - `HistoryChart` 的交互增强（现有 `LineChart` 只画线，无缩放、选点读数与时间范围切换）；
  - `RiskConfirmDialog`（按 `risk` 与"不再提示"偏好决定是否出现）；
  - `ErrorPanel` / `SkeletonBox`（冷启动骨架屏）。

### 10.8 页面清单与内容

**总览（`HomeScreen` · 已实现）**
- 顶部设备信息与状态条：设备型号 / Android 版本 / 内核版本 / 模块版本、`运行状态`/`安全模式` 胶囊（安全模式可点击退出）。
- **01 设备状态**：CPU（`/proc/stat` 实时使用率、温度、均频、在线核数点阵，点击直达 CPU 管理）；电池（电量水波、状态、温度、充电器标签，点击直达电池页）。**内存卡片仍是占位**（值 `—`、状态「等待模块接入」）——内存页本身已从模块中心进入（`MemScreen`），只是这张总览卡片还没接 `snapshot.mem`；储存卡片随阶段 5 接入。
- **02 系统动态**：展示从 Root 事件期刊同步到 Room 的最近 5 条跨模块重要事件；点击“查看全部”进入 **系统事件页（`EventHistoryScreen`）**（支持按模块、类别、等级筛选及分页加载）。
- **03 系统运行状态**：Root 权限状态、Protocol 版本、当前接入模块列表。

**模块中心（`ModuleHubScreen` · 已实现）**
- 集中呈现 CPU 管理（已接入）、电池管理（已接入，探测到电池才可点）、唤醒锁（已接入）、内存管理（已接入，按 `caps.mem` 启用）、储存管理、进程管理（其余标记规划中、不渲染虚假数据）。

**CPU（`CpuScreen` · 已实现）**
- Hero 卡片：在线核心计数、温度胶囊、在线比例波浪条、配置同步状态；
- 三个快捷入口卡片：**自适应性能管理（`AdaptiveScreen`）**、**熄屏管理（`ScreenOffScreen`）**、**进程组（`CpusetScreen`）**；
- **01 核心矩阵**：8 核网格开关，实时渲染逐核利用率水波高度与频率，`cpu0` 锁定；
- **02 调频工作区**：每簇卡片独立展示静态配置 vs 当前生效（标明“动态封顶中”/“熄屏限频中”）、双滑块 `RangeSlider`、Governor `FilterChip`、单簇“恢复默认/应用”，底部悬浮 `ApplyDock` 支持“应用 / 应用并持久化 / 放弃修改 / 重置”。
- **自适应性能管理**：引擎总开关、守护状态横幅、智能负载分档 / 温度动态收紧 / 低电量节能 / PSI 性能护栏参数编辑与合法性校验、CPU/GPU 实时负载、温控传感器列表与重置。
- **熄屏管理**：总开关、各簇熄屏最高频离散滑块（含 24 档回退生成与最右“不限制”）、熄屏额外关核开关（校验保留至少一个常规在线核）、保存与清除配置。
- **进程组（cpuset）**：各进程组可用核心 `ToggleButton` 网格、簇快捷预设、单组及全部恢复默认。

**电池（`BatteryScreen` · 已实现）**
- Hero：大号 Wavy 电量进度、状态胶囊、充电器与温度、剩余 mAh、充满/耗尽粗估时间。
- 电气参数：电压、电流、功率、温度（45°C 琥珀 / 50°C 红）。
- 会话内趋势：电压/电流/功率/温度迷你曲线（仅页面打开期间，内存内 2 分钟窗口）。
- 24 小时电量曲线：读取守护进程写入的 `history/battery.jsonl`，附「上次充电以来掉电」。
- 健康度：健康串与百分比、循环计数（含来源标注）、满容量 vs 设计容量。
- 应用耗电排行：底部弹层展示 `sysflow battery apps` 的 per-UID 归因。
- **未含**：息屏掉电速率拆分（采样缺屏幕状态）；时间范围切换目前固定 24h（阶段 6 再扩展）。

**唤醒锁（`WakelockScreen` · 已实现，独立页面）**
- 与电池页平级，不并入电池页；全页 M3 Expressive：顶部 Connected `ToggleButton` 组分三段，每段以 hero 卡开头，列表为 Connected 分段圆角行（`segmentedShape`）：
  - **唤醒源**：概览卡（活跃/已知、统计窗口，debugfs 不可读时说明只能用拦截）+ 24 小时排行（占比条）。
  - **应用**：`wakelock apps` 的 per-UID 唤醒时间与 AppOps 拦截状态合并成一张表，行内直接开关拦截；
    被拦截但本次没持锁的应用以 `—` 出现在表里，方便找到并解除；受保护应用禁用开关并标注原因。
  - **持锁**：`dumpsys power` 的当前用户态 wakelock（tag / 包名 / 持有时长），按需刷新。
- 拦截提示：可能延迟通知与后台同步；系统关键应用不可拦截。

**内存（`MemScreen` · 已实现）**
- 可用内存 hero + 「整机」与「已用」两条构成条（已用各段之和恒等于 `MemTotal − MemAvailable`；
  缓存单列为「可回收」，池段半透明并注明"池是容量不是用量"）。
- swap / zRAM 区块：用量、压缩比、writeback 提示。
- PSI 24 小时曲线（`history mem`，300 s 一个点），空态区分"daemon 未运行"与"还没采到两个点"。
- 应用内存排行（按 PSS，点一行看单应用明细：UID、PSS、已换出、进程数、占比、包名）。
- 内存归属（系统 / 用户）卡：`mem breakdown` 的全量 PSS 拆分，卡片下明确写"与上方已用口径不同，不可相加"。
- 可调参数卡：`mem.swappiness`、`mem.vfs_cache_pressure` 两个滑块（范围取自模块 `schema`），
  移动滑块不改任何东西，点保存才 `set --persist` 并重新读取；`mem.mglru` 为 1 时提示 swappiness 会被稀释。
- 前台 3 s 轮询（`repeatOnLifecycle(STARTED)`），离开即停。
- **不含**：zRAM 重配与 `drop_caches`（见 §7.4），进程级列表与强制停止（阶段 4）。

**进程（阶段 4 规划）**
- 列表：搜索、排序（CPU/内存/名称）、系统进程过滤。
- 详情：PID、UID、oom_score_adj、线程数、cgroup。
- 操作：强制停止、降/升优先级、限制到小核；批量操作用 FloatingToolbar。
- 进入页面才采样；离开即停止。

**储存（阶段 5 规划）**
- 容量环形/波浪进度；各分区使用率。
- IO 吞吐实时/历史、写入量统计。
- 调度器、read_ahead 设置；TRIM 按钮。
- 目录占用分析（异步，带进度与取消）。

**设置 / 诊断（`SettingsScreen` & `DiagnosticsScreen` · 已实现基础，阶段 6 补全偏好与实验开关）**
- **已实现**：模块行为（开机应用开关、开机延迟 0–300s、持续守护开关、守护间隔 2–120s）、自适应性能管理入口、安全与恢复（安全模式状态、退出安全模式、救援标记说明、隔离配置备份恢复/丢弃、损坏 `config.json.corrupt` 删除）、维护（**诊断与日志页**：静态硬件信息 + 能力探测 + 漂移检查 + CPU 快照报告、最近 50/100/200/500 行日志读取与复制；恢复所有模块默认值）、关于信息。
- **待阶段 6 补齐**：主题偏好（跟随系统/动态取色/种子色）、单位偏好、刷新频率、每模块历史记录开关与保留时长（当前间隔与保留为固定常量）、实验功能总开关（解锁 high 风险项）、第三方开源许可明细。

### 10.9 无后台前提下的可选"入口"

不引入常驻成分的快捷入口，均仅在用户触发时调用 CLI：
- **快捷设置 Tile**：点击切换预设（如性能/省电），触发时短暂 su 调用，结束即退出。
- **App Shortcuts**：长按图标的快捷项（"恢复默认"、"进入电池页"）。
- **桌面小部件**（可选，v2）：数据来自模块写出的 `state/widget.json`，Widget 只读文件，不需要 root，也不需要后台服务（系统按 `updatePeriodMillis` 刷新，最小 30 分钟，或由模块侧触发更新广播——需评估）。

### 10.10 国际化与无障碍

- 目标在阶段 6 完成字符串全面抽离至 `strings.xml`（首发中文 + 英文，schema 的 `title_key` 映射资源并回退 `id`）；当前阶段 1 界面暂以直写中文为主。
- 开关/滑块/核心方块持续补全 `contentDescription` 与语义角色（cpuset 与核心网格已具备语义描述）；颜色对比度满足 WCAG 2.1 AA（已通过 `ThemeContrastTest` 校验）。
- 阶段 6 重点补齐系统字体缩放与平板大屏适配验收。

### 10.11 错误与边界处理

| 场景 | 处理 |
|---|---|
| 没有 root | 全屏引导页（`GuideScreen`），说明需要 root 与模块 |
| 模块未安装 / `sysflow` 不存在 | 引导安装模块（说明确认 `/data/adb/modules/sysflow/bin/sysflow` 存在） |
| 协议版本不兼容 | 提示更新模块或应用，并禁用写操作 |
| `safe_mode` | 只读模式 + 顶部横幅提示（支持一键退出与恢复隔离配置） |
| `unsupported` | 功能入口隐藏或提示不支持原因（如内核/SELinux/机型差异） |
| `set` 部分生效 | Snackbar + 详细请求值/读回值对比提示（并展示于 `CpuScreen`） |
| 命令超时 / 暂态失败 | 15s 超时截断并提示错误；页面轮询容忍单次失败保留上次有效状态 |

---

## 11. WebUI 设计

- **定位**：**模块救援控制台**，不是第二个客户端。它只保证在 APK 无法安装/更新/启动、或某个坏配置
  让设备行为异常时，用户仍能查看状态并自救。完整说明见 `docs/webui-console.md`。
- **能力清单**（全部按需触发，无轮询）：
  - `version` + `caps`（版本、协议、模块能力）
  - `snapshot`（当前状态时间戳）
  - `safe-mode status` / `safe-mode clear`
  - `reset --all`（恢复全部模块默认值，带确认/禁用态）
  - `check`（只读漂移检查）
  - `log --tail 200`（最近日志）
- **技术**：`module/webroot/{index.html,app.js,app.css}` 三个静态文件，共 527 行 / 约 17 KB；纯
  HTML + CSS + JS，直接调用 KernelSU WebView 注入的全局 `ksu.exec(command, optionsJson,
  callbackName)`，不引入 `kernelsu` npm 包，因此**没有 npm、没有 Vite、没有构建步骤**；模块打包
  只校验三个文件存在。
- **明确不做**：CPU / 电池 / 唤醒锁 / 内存 / 自适应 / 熄屏 / 进程组 / 事件页、图表、设置编辑器、
  `?demo=1` 预览。这些只在 APK。
- **为什么砍到这个范围**：与 APK 功能对齐曾需要约 7 000 行并行 TypeScript，外加四对必须手工同步的
  重复实现（事件文案、电池模型、CPU 模型、client/API），还让模块构建依赖联网 `npm ci`。收窄之后
  这些成本全部消失，模块构建只需要 Rust、NDK 和 `zip`。
- **约束**：只用 `docs/protocol.md` 里公开的命令，所以它同时也是模块升级后的 CLI 冒烟测试；功能
  新增不再需要任何 WebUI 改动。

---

## 12. 开发阶段与里程碑

> 每个阶段结束都应是**可独立使用**的版本。时间为单人业余开发的估计，仅供排期参考。

### 阶段 0：基础与协议（约 1–2 周）

**目标**：把骨架立起来，用 mock / CPU 的最小功能验证全链路。

- [x] 初始化 monorepo、Rust workspace
- [x] 定稿 `docs/protocol.md`（信封、错误码、schema 类型、get/set 语义、apply-boot / 安全模式 / 单位约定）✅ JSON 示例已校验
- [x] 实现 `sysflow-core`：信封、错误、`Module` trait、Registry、sysfs 工具层（含 `write_verified` 与只读 `0444` 回退写）✅
- [x] 实现 `config.json` 读写（原子写、文件锁、版本、迁移骨架、损坏文件自动隔离）✅
- [x] 实现 CLI：`version / caps / snapshot / <module> schema|get|set|reset` 以及 `history / apply-boot / boot-confirm / safe-mode / log / reset --all` ✅
- [x] 打包脚本与模块模板（`module.prop / customize.sh / service.sh / uninstall.sh / webroot`）✅
- [x] 用 mock 模块跑通 CLI 单元测试（`SYSFLOW_MOCK=1`；含 8 个端到端用例和各模块内单元测试）✅
- **验收**：模块可安装，`sysflow caps` 输出正确 JSON；主机测试绿。✅ **已通过**（2026-11-22）

### 阶段 1：CPU 模块 + 事件 + 守护 + 隔离恢复 + APK + WebUI（约 2–3 周）

- [x] **CPU 基础与安全硬化**：probe、schema、collect、apply（核心开关、频率、governor）、reset；默认快照归一化为硬件频率范围（CPU/GPU）；修复按集群持久化与 `apply-boot` 过期值容错 ✅
- [x] **CPU 对等能力迁移**：WALT 参数与 `adaptive_*` 联动 / `walt_guard`、cpuset 绑核、GPU devfreq + KGSL、DDR/LLCC boost、熄屏配置、自适应四策略、`time_in_state` / `/proc/stat` 负载读数、温控/冷却遥测 ✅
- [x] **守护进程与熄屏监测**：`sysflow check` 只读漂移检查；`daemon start/stop/status/configure/run` 与 `enforce` 实现周期漂移修复与自适应推进；`service.sh` 负责开机编排，熄屏切换后续并入 daemon ✅
- [x] **开机恢复与旧版清理**：`boot status|configure`、`recovery status|restore|clear|clear-corrupt` 安全模式自动隔离配置、`sysflow info` / `note`、`migrate-legacy.sh` 与 `migrate-legacy-doze.sh` ✅
- [x] **开放式事件系统**：`EventStore` + `sysflow event emit|list|prune|clear` + 系统/CPU/服务自动埋点 + APK Room v2 镜像（`sysflow-events.db`）+ APK 总览“系统动态”与“系统事件”页（救援控制台不渲染事件） ✅
- [x] **协议类型生成与客户端对齐**：Rust `sysflow-types` 生成 Kotlin（`--ts` 能力保留但当前未使用）；APK 搭建完成（M3E 主题与对比度单测、悬浮 3-Tab 导航、Koin、libsu、总览/模块/CPU/自适应/熄屏/进程组/设置/诊断/引导页）；WebUI 已降级为模块救援控制台（状态/安全模式/恢复默认/诊断/日志），不再与 APK 功能对齐 ✅
- [x] **阶段 1 构建**：`cargo test` 工作区通过；`./build-sysflow-module.sh` 产出模块 zip（含 SHA256，当前 v0.2.2）；`:app:assembleDebug` 出包。`sysflow-types` 已补 `DaemonStatus` 与 `HistoryData`，但 `DaemonStatus` 只建模了 `running`/`implemented`，尚无 `BootStatus`。
- [ ] **阶段 1 真机回归验收**：在真机验证 APK 与 WebUI 控制、重启持久化、安全模式与隔离配置恢复。`docs/protocol.md` 重复的 §3.11 已在文档层面修掉（boot/recovery/daemon 并入 §3.13）；仍欠 `BootStatus` 类型与 `DaemonStatus` 的 `enabled`/`interval_s` 字段。

### 阶段 2：电池 + 唤醒锁 + 历史曲线（约 3–4 周）

- [x] 历史采样与存储：daemon 循环内接入 `sample()` / `sample_interval()` 调度、`history/<module>.jsonl` 追加式存储与保留策略（30 天 / 20000 行）
- [x] `history` 命令降采样查询实现（按桶取最新点），`daemon_running` 反映真实守护进程状态
- [x] 将 `service.sh` 的 `screen_loop` 迁入 daemon：Rust `screen.rs` 负责背光/`dumpsys power` 探测，`service.sh` 与 `screen-state.sh` 的熄屏循环已删除
- [x] 采样记录屏幕状态（`screen` 采样上下文），APK 电池页显示亮屏/熄屏掉电速率
- [x] 电池模块（只读部分）：`power_supply` 探测、单位归一化（µV/mA/µAh/mW 按量级）、派生功率与健康度、只读契约（`set` 一律 `invalid_arg`）、`battery/tests.rs` 27 项单测 + `battery_apps`/`battery_health` 9 项 + CLI 端到端验证
- [x] 电池模块（采样部分）：历史采样、事件埋点、派生统计（充满以来耗电、亮屏/熄屏掉电速率）
- [x] 唤醒锁模块（第一轮，只读）：内核唤醒源两格式解析、`wakelock.kernel_sources` / `wakelock.ranked` / `wakelock.summary`、300 秒采样与 24h 窗口滚动、`wakelock held` verb（60 秒缓存）
- [x] 唤醒锁模块（第二轮）：AppOps `WAKE_LOCK` 拦截 + 白名单 + 规则持久化与开机重放 + `drift()` 复核 + APK 唤醒锁页
- [x] `batterystats` per-app 唤醒时间：`WakeLockScanner` 流式解析 `All partial wake locks` + `sysflow wakelock apps`（按 UID 聚合、60 秒缓存）+ APK 唤醒锁页展示（内核唤醒源屏蔽不做，理由见 §7.3）
- [x] APK 电池页：Wavy 进度、会话内趋势、24 小时磁盘曲线与「上次充电以来掉电」、健康度、应用耗电排行；总览电池卡片已接入（`HistoryChart` 为自绘 Canvas；排行榜已从底部抽屉改为独立子页面 `BatteryRankingScreen`，电池页内保留 Top 3 预览）
- [x] APK 唤醒锁页：唤醒源（24h 排行 + 时长/次数切换 + 趋势曲线 + 上次唤醒）、应用（拦截 + 保护名单 + 全部恢复）、持锁（含一键拦截）；**一键恢复**补齐阶段 2 验收项
- [ ] 总览页唤醒锁卡片；设置页加入历史开关与日志导出（WebUI 仅救援控制台，不跟随）
- **验收**：daemon 连续运行 72 小时无崩溃、功耗可接受（自评：采样总 CPU 时间占比 < 1%）；曲线正确；拦截规则开机后仍生效且可一键恢复。

### 阶段 3：内存模块（约 2–3 周 · 已完成 Step 0–8）

- [x] 内存模块只读面：meminfo / PSI / zRAM / 构成项 / MGLRU 读取，全部 `readonly`，进 `snapshot`
- [x] 内存模块写入面：`mem.swappiness` 与 `mem.vfs_cache_pressure`（`int_range` / `low` / persist），
      走 `apply` / `drift` / `sanitize_boot`，含"30 分钟 3 次即停手 6 小时"的争用护栏
- [x] 历史：`available_kb` / `swap_used_kb` / `zram_compr_kb` 瞬时值 + PSI / vmstat 同开机差分（300 s）
- [x] 事件：`mem_pressure_high` / `mem_pressure_recovered` / `mem_oom_kill` / `mem_setting_contested`
- [x] verb：`mem apps`（按 PSS 排行）、`mem breakdown`（系统 / 用户 PSS 拆分）、隐藏 `mem probe`
- [x] APK 内存页（构成条、PSI 曲线、排行、归属、两个滑块）与模块中心 / 导航 / Koin 接线
- [ ] zRAM 重配置（Step 9，暂缓，与阶段 4 一起重估）；`drop_caches` 不做（通常无益）
- [ ] `risk` 确认流程完善（`RiskConfirmDialog`）
- **验收**：参数调节读回一致（已由单测覆盖 **主机部分**）；PSI 曲线可用；**真机计时、事件噪音阈值与
  24 h `swappiness` 观察仍待补**。zRAM 重配置不在本阶段范围。

### 阶段 4：进程管理（约 2–3 周 · Step 0 契约已冻结）

- [x] Step 0：契约冻结（`docs/process-module-plan.md`、协议 §3.25 Draft、`docs/modules/process.md` 骨架、保护名单与字段口径）
- [ ] Step 1：真机探针（隐藏 `process probe`，verb 已实现并主机测试通过）：`/proc` 可读性、CLK_TCK、dumpsys 前台行格式、cpuset / freezer 布局、`am` / `renice` 可用性
- [ ] Step 2：只读面（列表 / 详情 / 前台识别 / 保护分类，`in_snapshot=false`）
- [ ] Step 3：一次性动作：force-stop、renice、`oom_score_adj`（全量保护检查 + 读回）
- [ ] Step 4：实验动作：应用级 cpuset 绑核、freezer（探测到才出现）
- [x] Step 5：APK 进程页（M3E 搜索/排序/过滤/详情/确认；仅进入页面时 2 s 轮询）——主机代码完成，APK 构建由用户执行
- **验收**：列表刷新流畅（2s 周期不卡顿）；保护名单生效；误操作可恢复。**真机项**：写路径（oom/renice/cpuset/freeze 的 SELinux 与 AMS 反应）、freezer 叶子、`am freeze` 回退、2 s 轮询流畅度。

### 阶段 5：储存（约 2 周）

- [ ] 容量、块设备、IO 统计、调度器、read_ahead、TRIM
- [ ] 写入量历史；健康度探测（可用则显示）
- [ ] 目录占用分析（异步任务、可取消）
- [ ] APK 储存页
- **验收**：IO 统计与 `iostat`/`diskstats` 手算一致；TRIM 可用；大目录分析不卡 UI。

### 阶段 6：打磨与发布（约 2–3 周）

- [ ] 预设（省电/均衡/性能）：一组跨模块 `set` 的组合，存于 config，Tile 与 Shortcut 调用
- [ ] 快捷设置 Tile、App Shortcuts、DataStore 偏好设置（主题/单位/刷新率）与实验功能开关
- [ ] 动画与形状变形精修、动态取色开关验收、大屏 `NavigationRail` 适配
- [ ] 国际化（抽离 `strings.xml` 中/英双语与 WebUI 多语言）、无障碍全面检查、清理废弃占位代码
- [ ] 多机型兼容性测试与能力探测修补
- [ ] 补全 GitHub Actions CI、模块支持矩阵与故障排查文档
- [ ] 版本 1.0.0 发布（GitHub Release：模块 zip + APK）
- **验收**：至少 3 款不同厂商/内核设备通过冒烟测试；无 P0/P1 已知问题。

### 后续（v2 候选）

- 桌面小部件（读 `widget.json`）
- 基于规则的自动化（亮屏/充电/温度触发预设）
- 热管理曲线与温控调节（高风险）
- 导入/导出配置、分享预设
- 模块自更新（对接 KernelSU 的 `updateJson`）

---

## 13. 测试策略

### 13.1 Rust 与脚本侧

- **单元测试**：sysfs 读写层用"虚拟根目录"（`SYSFLOW_SYSFS_ROOT` 与 `SYSFLOW_DATA_DIR` 重定向到临时目录）模拟；解析器（proc/stat、thermal、meminfo、diskstats、dumpsys 片段）用样本文本做 fixture。CPU 假树已包含 cpufreq/policy、thermal、WALT、`/dev/cpuset`、GPU devfreq + busmon、DDR/LLCC、`/proc/stat`、`/proc/pressure/cpu` 与电池节点；`event.rs` 覆盖开放模块事件、去重、游标分页与校验。
- **属性测试**：配置迁移、降采样算法（阶段 2）。
- **协议契约测试**：对每个模块的 `schema/get/set` 输出做校验；`tools/gen-types.sh` 在 CI 重新生成并检查 TS/Kotlin 是否与 Rust 模型一致。
- **模拟设备与迁移测试**：构造"不支持/部分支持/路径不同"的假 sysfs 树验证 `probe` 与降级行为；`tools/tests/migrate-legacy-smoke.sh` 与 `migrate-legacy-doze-smoke.sh` 验证旧版 SoCFlow 残留清理与防误伤逻辑。

### 13.2 设备侧

- 冒烟脚本：`adb shell su -c "sysflow snapshot"`、`sysflow check`、`sysflow event list`，校验返回 `ok`。
- 破坏性测试：随机 `set` + 重启，验证 `safe_mode`、`recovery_backup` 隔离与恢复流程。
- 长稳测试（阶段 2）：daemon 72h，监控内存占用、日志大小、CPU 时间。
- 多机型矩阵（记录在 `docs/modules/` 的支持表中）。

### 13.3 APK 与 WebUI 侧

- **APK 现有单测**（`app/src/test/kotlin/io/sysflow/`）：`CpuLiveTrackerTest`（/proc/stat 负载差分、time_in_state 均频、静态与动态上限分离、单簇 diff）、`CpuDraftDiscardTest`、`CoreOnlineWriteRaceTest`、`CpusetFormatTest`、`BatteryDrainTest`、`WakelockModelsTest`、`MemModelsTest`（键解析、构成守恒、池压缩、apps/history、不可用态）、`MirrorOrphanRuleTest`、`ThemeContrastTest`（WCAG 2.1 AA >= 4.5 对比度校验）。
- **APK 待补测试**：
  - `SysFlowClient`：用 Fake Shell 返回预置 JSON，测试信封解析、错误映射、超时、互斥；
  - ViewModel：测状态机（加载 → 成功 → set 中 → mismatch）；
  - Compose UI 测试：核心矩阵/滑块/确认弹窗；截图测试（Paparazzi 或 Roborazzi）覆盖亮/暗色；
  - 假数据模式（`FakeSysFlowClient`）：支持无 root 模拟器开发调试。
- **WebUI**：没有构建步骤，因此也没有前端单测。验证方式是模块升级后在 KernelSU 里打开控制台，逐个按钮确认 `version` / `caps` / `snapshot` / `safe-mode` / `check` / `log` 都返回 `ok`；这也是它作为 CLI 冒烟测试的用处。

### 13.4 兼容性

- Android 版本：至少覆盖 12 / 13 / 14 / 15 及设备实际运行版本（minSdk 29）。
- Root 方案：KernelSU（含 Next）/ Magisk / APatch 至少各一次安装验证。

---

## 14. 构建、发布与版本管理

- **版本号**：模块与 APK 独立 SemVer；`protocol` 单独整数。发布说明列出"最低兼容的对端版本"。
- **本地一键构建脚本**：`./build-sysflow-module.sh` 自动完成 Rust ARM64 release 构建、`tools/pack-module.sh` 打包及 `sha256sum` 校验（缺 `cargo-ndk` / `aarch64-linux-android` target 时会自动安装；不构建 APK），输出 `dist/sysflow-module-<version>.zip` 与 `.sha256`（WebUI 为静态资源，无需构建）。
- **CI（GitHub Actions，阶段 6 落地）**：
  - Rust：`fmt`、`clippy -D warnings`、`test`、`cargo ndk` 交叉编译产物；
  - 类型：`tools/gen-types.sh` 重新生成并检查 `Protocol.kt` 与 Rust 模型一致（没有 TS 目标）；
  - Android：`assembleDebug`、`assembleRelease`、Lint、单元测试；
  - WebUI：只校验 `module/webroot/` 三个静态文件存在（无 tsc、无 vite）；
  - 打包：把二进制与 webroot 注入模块 zip，产出 artifact。
- **发布**：打 tag 触发 Release，附：`sysflow-module-vX.Y.Z.zip`、`sysflow-vX.Y.Z.apk`、校验和、更新日志。
- **更新机制**：`module.prop` 配置 `updateJson`（KernelSU/Magisk 支持在管理器内更新）；APK 启动时仅检查模块与应用的协议兼容，不做联网自动更新。
- **签名**：APK 使用固定签名 key，妥善保管；模块 zip 提供 SHA256。
- **许可**：确定开源许可证（如 GPL-3.0 / Apache-2.0），在仓库与"关于"页标注第三方许可。

---

## 15. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| 不同机型 sysfs 路径差异大 | 功能不可用 | `probe()` 探测 + 隐藏；`docs/modules/` 维护支持矩阵；接受社区提交适配 |
| 调节导致卡死/重启循环 | 用户体验差、可能需救砖 | 开机计数 + `safe_mode` + `recovery_backup` 自动隔离；`disable_tuning` 救援文件；高风险默认隐藏 |
| thermal/vendor HAL 或 WALT 覆盖设置 | 设置"不生效"或限频被绕过 | 读回校验 + `mismatch` 提示；WALT `adaptive_*` 归零与 `0444` 守卫；daemon 周期性复核 `enforce` |
| SELinux 拒绝（尤其读 debugfs/dumpsys） | 数据缺失 | 日志记录 avc；caps/errors 标注原因；多源降级（如背光节点 + `dumpsys power`） |
| `su` 调用延迟/并发冲突 | 体验卡顿或配置损坏 | `libsu` 常驻 shell；`snapshot` 合并；骨架屏/加载态；写 `Mutex` + Rust 文件锁 |
| daemon 耗电或被杀 | 历史断档或守护失效 | 低频（可调 2–120s）、低优先级；阶段 2 自拉起与超时上限；历史带断点标记，UI 容忍缺失 |
| M3 Expressive API 仍在演进 | 升级时编译失败 | 封装在 `core/ui`；锁定版本；升级前单独分支验证 |
| 协议漂移（Rust/Kotlin/TS 三端） | 联调困难 | 单一协议文档 + `sysflow-types` 生成 + 契约测试（需补全 `DaemonStatus`/`BootStatus` 避免手改覆盖） |
| 进程/唤醒锁误拦截关键服务 | 通知丢失、系统异常 | 保护名单 + 默认白名单 + 一键恢复 + 风险提示 |
| 范围过大做不完 | 项目烂尾 | 严格按阶段交付，每阶段可独立发布；先 CPU 后其他 |
| 某些 ROM 禁止 shell 写入关键节点 | 部分功能失效 | 尝试 `0444→0644` 临时放宽写后还原；仍不可写则文档说明并反馈 `permission`/只读状态 |

---

## 16. 待决策事项

1. ~~项目/二进制名称~~ ✅ 已定：项目与应用名 SysFlow，模块 id 与二进制 `sysflow`，包名 `io.sysflow`。
2. **开源许可证** ⬜（如 GPL-3.0 / Apache-2.0，待补入仓库与关于页）。
3. ~~历史存储格式~~ ✅ 已定：事件期刊采用单条独立 JSON 文件；时序曲线先 `history/<module>.jsonl`（30 天 / 20 000 行），稳定后再评估定长二进制。
4. ~~DI 框架~~ ✅ 已定：**Koin**。
5. ~~图表方案~~ ✅ 已定：**自绘 Compose Canvas**，未引入 Vico。电池的 24 h 曲线用 `LevelChart`，通用折线用 `core/ui/components/Chart.kt` 的 `LineChart`（按采样序号等距，不按时间戳）。
6. ~~最低 Android 版本、架构与 root 方案范围~~ ✅ 已定：**minSdk 29**；当前仅 **arm64**；KernelSU / KernelSU Next / APatch / Magisk。
7. **旧版 CPU 模块处理** 🚧 一次性遗留清理脚本（`migrate-legacy.sh`、`migrate-legacy-doze.sh`，含 `tools/tests/` 冒烟测试）已集成；待真机验收后正式归档旧仓库，并在 README 补充旧 `soc_flow` 配置迁移说明。
8. **预设系统的数据结构** ⬜（跨模块 set 组合）放在阶段 6，还是更早设计进 config。
9. ~~WebUI UI 实现方案~~ ✅ 已定：**不做第二客户端**——静态救援控制台（纯 HTML/CSS/JS，无框架、无构建步骤），只覆盖状态/安全模式/恢复/漂移检查/日志；功能页只在 APK。见 `docs/webui-console.md`。
10. **默认语言与翻译策略** 🚧 当前 APK 为中文直写文案（WebUI 控制台为英文静态文案）；阶段 6 抽离 `strings.xml` 实现中英双语。
11. **类型生成器补全** 🚧 已生成：`ErrorCode`、`ItemType`、`Scope`、`Risk`、`ProtocolError`、`Caps`、`CapsData`、`SchemaItem`、`Schema`、`ApplyResult`、`SnapshotData`、`CheckData`、`VersionData`、`SafeStatus`、`HistoryData`、`DaemonStatus`。**两个真实缺口**：`DaemonStatus` 只建模了 `running` / `implemented`，而 CLI 还返回 `enabled` 与 `interval_s`；没有 `BootStatus`，`boot status` 仍按 `JsonObject` 解析。补这两个类型时不要手改 `Protocol.kt`，改 `core/sysflow-types/src/main.rs` 后重跑 `tools/gen-types.sh`。

---

## 附录 A：CPU 模块 schema 示例与全量项分类（阶段 1 已落地）

核心项示例（`options` 在运行时由 `probe()` 按设备填充）：

```json
{
  "module": "cpu",
  "supported": true,
  "items": [
    {"id":"cpu.online","type":"bool_list","scope":"global","count":8,"locked":[0],
     "risk":"medium","persist":true},
    {"id":"cpu.max_freq","type":"int_enum","scope":"cluster","unit":"kHz",
     "options":[],"risk":"low","persist":true},
    {"id":"cpu.min_freq","type":"int_enum","scope":"cluster","unit":"kHz",
     "options":[],"risk":"low","persist":true},
    {"id":"cpu.governor","type":"enum","scope":"cluster",
     "options":[],"risk":"low","persist":true},
    {"id":"cpu.cur_freq","type":"readonly","scope":"cpu","unit":"kHz"},
    {"id":"cpu.temp","type":"readonly","scope":"global","unit":"0.1C"}
  ]
}
```

阶段 1 实际还包含以下同前缀（`cpu.*`）扩展项（完整清单与设备路径见 `docs/modules/cpu.md` 与 `docs/protocol.md`）：
- **拓扑与实时遥测**：`cpu.clusters`、`cpu.cluster_cur_freq`、`cpu.cluster_hw_cur_freq`、`cpu.hw_min_freq`、`cpu.hw_max_freq`、`cpu.freq_table`、`cpu.governor_table`、`cpu.time_in_state`、`cpu.stat`、`cpu.usage_stat`、`cpu.gpu_busy`、`cpu.battery_temp`、`cpu.thermal_sensors`、`cpu.thermal_sensors_all`、`cpu.cooling_devices`、`cpu.gpu_info`
- **GPU 与总线 Boost**：`cpu.gpu_min_freq`、`cpu.gpu_max_freq`、`cpu.gpu_cur_freq`、`cpu.gpu_governor`、`cpu.gpu_dcvs_boost`、`cpu.gpu_bus_boost`、`cpu.ddr_boost_freq`、`cpu.llcc_boost_freq`
- **WALT 调度与写守卫**：`cpu.walt.<key>`（12 项参数，含 `text` 型 `target_loads`）、`cpu.walt_guard`
- **Cpuset 进程组**：`cpu.cpuset.<group>`（如 `top-app`、`foreground`、`system-background`、`background` 等）
- **熄屏配置**：`cpu.screen_off`、`cpu.screen_off.max_freq`、`cpu.screen_off.freq_table`、`cpu.screen_off.cores`、`cpu.screen_off_active`、`cpu.screen_off_saved_online`
- **自适应引擎**：`cpu.adaptive`、`cpu.adaptive.{smart,thermal,battery,psi}`、20 项阈值/上限/迟滞参数、`cpu.adaptive_state`

## 附录 B：开发顺序速查

```text
阶段0 协议+骨架 (✅)
→ 阶段1 CPU全量+事件+守护+恢复+APK+救援控制台 (✅ 构建通过，真机回归验收跟踪中)
→ 阶段2 历史曲线+电池 (✅) + 唤醒锁只读 (✅) + 唤醒锁拦截+排行+上次唤醒 (✅) (真机验收跟踪中)
→ 阶段3 内存 Step 0–8 (✅；Step 9 zRAM 重配暂缓)
→ 阶段4 进程 (⬜) → 阶段5 储存 (⬜) → 阶段6 预设/Tile/i18n/打磨/发布 (⬜)
```