# 内存模块（M3）开发计划

> 状态：**Step 0–8 + 观感补齐已完成（修订 16，2026-11-25）**：只读面、写入面、实时轮询、整机构成条、应用内存归属与分解全部落地；剩余 Step 9（zRAM 重配，暂缓）与真机验收。
> 协议条款（`docs/protocol.md` §3.22 模块、§3.23 `mem apps`、§3.24 `mem breakdown`）与设备矩阵（`docs/modules/mem.md`）已回填，
> 未经验证的行为不写进协议。
>
> 关联：`docs/development-plan.md` §7.4（功能设计）、`docs/protocol.md` §1（id/单位约定）、
> `docs/event-system.md`（事件契约与文案表）、`core/sysflow-modules/src/battery/`（只读模块模板）、
> `core/sysflow-modules/src/wakelock/window.rs`（累计计数器跨重启基线）。

## 修订记录

**修订 2** 相对初版的变化（初版的已定决策全部保留，未改动）：

| # | 类型 | 变化 |
|---|---|---|
| 1 | 语义修正 | 「尖峰捕获」改为「**不漏账**」：累计计数器差分保证区间内总停顿时间不丢，但给出的是区间**平均强度**，不是峰值（见〈采样与历史〉） |
| 2 | 缺口 | 差分一律用**实际经过时间**而不是名义 300 s（设备休眠会拉长间隔）；重启后首个差分点记 **null** 而非 0 |
| 3 | 缺口 | 补 `/proc/vmstat` 的 `pswpin/pswpout`、`allocstall_*`、`workingset_refault_*`；补 meminfo 占用分解；补非 zRAM swap（三星 RAM Plus 等）的处理 |
| 4 | 纠错 | `dirty_ratio` 的「ROM 设了 `dirty_bytes` 则 ratio 无效」不准确：写 ratio 会把 `dirty_bytes` 清零（见〈写入面契约〉） |
| 5 | 缺口 | 补写入面完整契约：`persist` 标志、写入顺序与约束、ROM 守护进程拉锯、开机时序、reset 默认值、安全模式 |
| 6 | 缺口 | `mem apps`：`cmd package list` 是 binder 调用，冷缓存下撑不住 300 ms 目标；拆分 UID→包名缓存，并补 `SwapPss` |
| 7 | 缺口 | 「谁被杀」除 `dmesg` 外增加 `logcat -b events` / ApplicationExitInfo 候选来源（待真机验证） |
| 8 | 缺口 | 事件接入漏了 `EventCopy.kt` 文案表；补文案、严重度、迟滞与阈值初值 |
| 9 | 缺口 | 补测试矩阵、fixture 清单、性能预算、风险与回滚、APK 页面状态机 |
| 10 | 文档一致性 | Step 0 的改文档范围扩大：`development-plan.md` 的 `config.json` 示例（L430）也写了 `vm.swappiness` |
| 11 | 待定项 | 6 个待定项各给出**推荐默认值**，拍板即可开工；新增 3 个待定项 |
| 12 | 排期 | 工作量按新增内容重估：约 10 d → 约 13 d |

**修订 3**（根据 S25 的 `mem probe` 实测，详见 `docs/modules/mem.md`「Observed values」）：

| # | 类型 | 变化 |
|---|---|---|
| 13 | 实测 | 这台设备 `swappiness` 默认是 **130**，不是 60；内核 6.6 上限 200。reset 的默认值必须取 ROM 实际值 |
| 14 | 实测 | ROM 用 **`dirty_bytes`（100 MiB / 25 MiB）**，ratio 文件读出 0。印证了修订 2 的纠错；S25 上 `mem.dirty_*` 默认隐藏，M3b 可考虑改为暴露 bytes 版本（待定项 10） |
| 15 | 实测 | swap 只有一个 `zram0`（12 GiB，`lzo-rle`），**没有非 zRAM swap**；「RAM Plus」分支保留为防御路径，不再是 Step 1 的未知数 |
| 16 | 纠错 | `allocstall_*` 在这台设备上有 5 个后缀，其中 `nomerge` / `nosplit` 不是内存 zone。**求和改为白名单**（`dma` / `dma32` / `normal` / `movable` / `device`），不再按前缀通配 |
| 17 | 实测 | `/data/system/packages.list` 模块可读（695 行、**10 列**）→ 升为 UID→包名的**首选**来源；只取前两列，且按 `uid % 100000` 匹配（分身 / 安全文件夹的 UID 带用户前缀） |
| 18 | 实测 | 杀进程记录：`dmesg` 没有任何杀进程行；`logcat -b events` 的 `am_kill` 混有 `empty #N`（AMS 缓存进程上限，不是内存压力）；**`dumpsys activity exit-info` 的 `reason=3 (LOW_MEMORY)` 才是 lmkd 杀进程的可靠记录**，但一次 2.5 MB，不能周期轮询 |
| 19 | 实测 | `oom_kill` 开机以来为 0，而 `exit-info` 里有数千条 `LOW_MEMORY`：内核 OOM 计数在安卓上几乎不反映压力，`mem_oom_kill` 降为次要事件（待定项 11） |
| 20 | 实测 | `ro.config.low_ram` 未设置（空值）→ 视为非低内存机型，用默认阈值 |
| 21 | 缺口 | 内存构成条有约 2.8 GB（≈ 25 %）不在已有字段里；Step 2 要补更多 meminfo 字段，并确认 dmabuf / GPU 是否可读 |
| 22 | 工具 | 探针增强：dump 全部 meminfo key、`paths` 区块（dmabuf / GPU / MGLRU / zram writeback）、`exit-info` 记录上下文；24 h 观察脚本改用 `nohup setsid`（第一次只跑了 3 分钟） |

**修订 4**（2026-11-25，S25 增强版探针：全量 meminfo、`paths`、`exit-info` 上下文）：

| # | 类型 | 变化 |
|---|---|---|
| 23 | 实测 | 内存构成的缺口从 ≈ 2.8 GB 缩到 ≈ 1.8 GiB：`GpuTotal`(0.39 GiB) + `CmaTotal`(0.61) + `RbinTotal`(0.39) + `DmaHeapPool`(0.02) 全在 meminfo 里 → **构成条不解析 dmabuf 逐 buffer**（138 条，且与 `GpuTotal` 重复） |
| 24 | 实测 | 两个免费自检点：`GpuTotal` 与 `/sys/class/kgsl/kgsl/page_alloc` 完全相等（407 392 kB）；`meminfo zram0` 与 zram `mem_used` 完全相等（1 006 020 kB） |
| 25 | 实测 | zram 带 **writeback**：`backing_dev=/dev/block/loop37`、`writeback_limit_enable=1`（RAM Plus）。页被回写后 `orig_data_size` 不再等于 swap used → **swap 用量只取 `/proc/swaps`**，`ratio` 在 writeback 开启时标注不可比 |
| 26 | 实测 | **MGLRU 已启用**（`lru_gen/enabled=0x0003`，`min_ttl_ms=0`）→ `swappiness` 的作用被稀释且更难预测，UI 不得承诺效果 |
| 27 | 实测 | ROM 已深度调参：`watermark_scale_factor=72`（默认 10）、`page-cluster=0`（默认 3）→ 支持「M3b 只做 `swappiness` + `vfs_cache_pressure`」，`dirty_*` 不做（待定项 10 关闭） |
| 28 | 实测 | `exit-info` 布局确认：匹配行 `process=… reason=3 (LOW_MEMORY)`，上一行 `timestamp=… pid=… packageUid=… user=…`，下一行 `importance=… pss=… rss=… state=…`；`pss=0.00` 不可用，**用 `rss`** → Step 4 可直接解析（待定项 11 关闭） |
| 29 | 实测 | `Committed_AS` 146 GiB vs `CommitLimit` 17 GiB：重度 overcommit，杀进程由 lmkd 主导，内存调优收益上限低 |
| 30 | 纠错 | PSI 会被探针自身推高（探针运行时 `full avg10=3.17 %`，自开机均值仅 0.52 %）→ 事件阈值必须取**安静期**基线，Step 3 要专门测一次不含探针的窗口 |
| 31 | 工具 | 24 h 观察脚本必须**每条命令一行**：行尾 `\` 续行在粘贴中丢过 `done`，脚本报 `unmatched 'while'` 后只写了一行就退出；文档已改为「脚本文件 + `sh -n` 预检 + 前台试跑 + `setsid` 脱离」 |

**修订 5**（2026-11-25，Step 2 主机部分落地）：

| # | 类型 | 变化 |
|---|---|---|
| 32 | 实现 | `collect()` 走 `probe::readings()`，与 `schema` 共用同一份 `Detected` 判定 → **snapshot 的键集合与 schema 恒等**，由一个契约测试钉住；某来源缺失时两边同时消失 |
| 33 | 实现 | 三处口径按协议落定：`mem.dirty` = `Dirty + Writeback`；`mem.swap.*_kb` 取 `/proc/swaps` 的 **Used**（不是容量）；`mem.zram.ratio` = `round(100 × Σorig / Σcompr)`（`295` = 2.95×），`Σcompr = 0` 时不产出该项 |
| 34 | 实现 | `in_snapshot()` 置 `true`；31 项 mem 测试 + `clippy -D warnings` 通过；假根 `collect()` 中位数 0.12 ms、端到端 `snapshot` 4 ms（debug 构建）。**真机计时与逐项对拍仍待补** |

**修订 6**（2026-11-25，Step 3 落地）：

| # | 类型 | 变化 |
|---|---|---|
| 35 | 实现 | `history.rs`：`state/mem.json` 基线（`version` / `boot_id` / `uptime_s` / `ts` / `counters`，临时文件 + rename 原子写），300 s 一个采样点 |
| 36 | 实现 | 重启判定四条：状态版本不符、`boot_id` 变化、uptime 回退、**任一**计数器回退 → 该点只写瞬时值，差值留空（gap），绝不写 0 |
| 37 | 实现 | 增量一律除以**实测间隔**；PSI 百分比 = 停滞 µs ÷ 间隔 µs（钳在 100 %），且换算出 0.01 % 整数 |
| 38 | 实现 | 页大小读 `/proc/self/auxv` 的 `AT_PAGESZ`，不写死 4 kB；取不到就**省略** `swap_in_kb` / `swap_out_kb`，不猜 |
| 39 | 验证 | 短跑 daemon：首点 3 个瞬时值 → 次点 11 个指标（`psi_some_pct=2000` 对应 1 s 停滞 / 5 s 间隔）；41 项 mem 测试 + clippy 通过；`sample()` 宿主 0.35 ms（含状态写）。**真机 300 s 节奏与休眠跨段待验** |

**修订 7**（2026-11-25，Step 4 落地）：

| # | 类型 | 变化 |
|---|---|---|
| 40 | 实现 | 事件在 `sample()` 内产出（无 shell 循环、无客户端推断）：`mem_pressure_high` / `mem_pressure_recovered` / `mem_oom_kill`，category 统一 `pressure` |
| 41 | 纠错 | **去重键必须各自独立**。协议原写 recovered 与 high 共用一把键，那会让 recovered 被 high 的窗口直接吞掉；现在分别是 `mem.pressure.high` / `mem.pressure.recovered` / `mem.oom_kill`，各带 6 h 窗口 |
| 42 | 实现 | 迟滞与连续计数持久化在 `state/mem.json` 的 `pressure` 字段：≥3.00 % 连续 3 点触发、<1.00 % 连续 3 点恢复；死区样本把两个计数都清零；重启结束本回合且**不补发** recovered |
| 43 | 实现 | 间隔 > 3× 名义（900 s）的采样只记录数字、不产生事件；`metadata` 带 `available_kb` / `psi_full_pct` / `psi_full_avg60` / `swap_used_kb`；`high` 额外尝试带 `last_low_memory_exit`（`dumpsys activity exit-info`，仅事件触发时读一次，8 s 超时，失败静默） |
| 44 | 纠错 | 协议原写 metadata 带 `screen`——模块不知道屏幕状态；改由 history 的 `screen` 字段承载，协议已同步 |
| 45 | 验证 | 49 项 mem 测试（死区断链、长间隔抑制、重启不补发、exit-info 解析、去重）+ clippy 通过。**真机上事件的噪音水平与 3 % 阈值是否合适仍待观察** |
| 46 | 实现 | APK 文案：`EventCopy.kt` 增加三条中文标题与详情（`psiDetail` 把 0.01 % 整数换算回百分比并带可用内存与 swap），`:app:compileDebugKotlin` 通过 |

**修订 8**（2026-11-25，Step 5 落地）：

| # | 类型 | 变化 |
|---|---|---|
| 47 | 实现 | `core/sysflow-cli/src/mem_apps.rs`：两段式排名——全量进程按 `VmRSS` 排序，只有前 `limit` 个读 `smaps_rollup` 的 `Pss` / `SwapPss`；按 UID 聚合，`share = pss / total` 保留三位小数 |
| 48 | 实现 | 标签链：`/data/system/packages.list`（首选，纯文件）→ `cmd package list packages -U`（回退）→ AID 表 → `UID <n>`，永不为 null；分身 / 工作资料按 `uid % 100000` 命中并标注 `(user N)`；`uid` 短格式与 `resolve_uid` **互逆**（含 10 000 基准：`10346 → u0a346`） |
| 49 | 实现 | 两级缓存：排行结果 60 s 且**按 `limit` 失效**（截断点变了测量集合就变了）；UID→包名映射单独 10 min，避免 ROM 隐藏 `packages.list` 时每次冷查询都付 `cmd package` 的 binder 开销 |
| 50 | 实现 | 内核线程（无 `VmRSS`）**不计入** `skipped`，否则这个数字会被几百个 kthread 淹没；进程扫描中途退出算 `skipped`，永不导致命令失败 |
| 51 | 验证 | 25 项 CLI 测试（status / smaps / packages 解析、聚合与尾部估算、两级缓存、`/proc` 不可用）；端到端 cold 5 ms / hot 8 ms；全量 311 项 + clippy 通过。**真机耗时与 `dumpsys meminfo` 对拍待验** |

**修订 9**（2026-11-25，Step 6 落地）：

| # | 类型 | 变化 |
|---|---|---|
| 52 | 实现 | 新增 `app/src/main/kotlin/io/sysflow/core/ui/components/Chart.kt`：`Sparkline` 从 `feature/home/BatteryChart.kt` **逐字迁移**，另加泛化的 `LineChart(points, minY, maxY, fill)` 与 `ChartPoint(x, y)` |
| 53 | 实现 | 电池页改为引用新位置（`BatteryChart.kt` 删 73 行、`BatteryScreen.kt` 补 import）；`LevelChart`（带充电影带与掉电统计）**保持原样**，避免视觉回归 |
| 54 | 判断 | `LineChart` 的 x 轴按**采样序号等距**，不按时间戳：daemon 休眠一小时与五分钟间隔在图上无法区分，但每个点本就代表一次采样，等距是诚实的画法；`minY`/`maxY` 可锁死轴范围（百分比轴不该自己缩放） |
| 55 | 验证 | `:app:compileDebugKotlin` 与 `:app:testDebugUnitTest` 通过。**真机渲染待 Step 7 一起看** |

**修订 10**（2026-11-25，2b 构成字段落地）：

| # | 类型 | 变化 |
|---|---|---|
| 56 | 实现 | `probe::items/readings` 新增 8 个只读构成项：`mem.kernel_stack`、`mem.page_tables`、`mem.percpu`、`mem.vmalloc_used`、`mem.gpu`、`mem.vendor.{rbin,cma,dmaheap}`，**逐项可选**（meminfo 没有该键就没有该项） |
| 57 | 实现 | `mem.zram.writeback`（0/1）随 zram 出现：`backing_dev` 存在即为 1。它的作用不是遥测好看，而是告诉客户端 `orig`/`ratio` 是否还能代表全部 swap 流量 |
| 58 | 纠正 | 厂商字段是**池大小**不是已用量（CMA 容量开机即定）。构成条必须把池单独画一段，否则「已用」会被高估——协议与 `mem.md` 都已写明 |
| 59 | 验证 | 52 项 mem 测试（新增 3 项：可选性、取值、writeback 三态）+ 全量 314 项 + clippy 通过；S25 假根端到端 33 项、`writeback=1`，构成覆盖 70.9 %、余 29.1 %（1.82 GiB） |

**修订 11**（2026-11-25，Step 7 落地）：

| # | 类型 | 变化 |
|---|---|---|
| 60 | 实现 | 新增 `app/src/main/kotlin/io/sysflow/feature/mem/`：`MemModels`（协议解析 + 构成分段 + 单位格式化）、`MemViewModel`（快照 + 按需加载 24 h 曲线与排行）、`MemScreen`（hero、构成条、swap/zRAM、PSI 曲线、应用排行、详细读数） |
| 61 | 纠正 | **`Cached` 不能算进「已用」**：页缓存可回收且已包含在 `MemAvailable` 里，误算会让构成条超过 100 %（S25 上 9.3 GB vs used 6.6 GB）。构成条只放 anon / slab / kernel / GPU / vmalloc / percpu，余量才是「其他」 |
| 62 | 实现 | 构成条口径：池按剩余空间**等比例压缩**，保证各段之和恒等于 `used`；池段半透明、图注写明「池是容量不是用量」 |
| 63 | 接线 | `ModuleHubScreen` 的「内存管理」卡接入 `caps.mem`；`ScreenRoute.memDetail` + 返回栈 + 标题/副标题；Koin 注册 `MemViewModel`；`SysFlowClient.memApps()` |
| 64 | 验证 | 6 项 `MemModelsTest`（键解析、构成守恒、池压缩、apps、history、不可用态）+ 既有 58 项 APK 单测通过；`:app:assembleDebug` 产出 APK。**真机可视化待验** |

**修订 12**（2026-11-25，Step 8 写入面落地）：

| # | 类型 | 变化 |
|---|---|---|
| 65 | 实现 | `mem/tunables.rs`：`swappiness`（上限按 `osrelease` 取 100/200）与 `vfs_cache_pressure`（1..1000）；**先整体校验再写**，写后读回，`mismatch` 如实上报 |
| 66 | 实现 | 修复护栏 `Guard`：同一项 30 分钟内修 3 次即停手 6 小时，并只发一次 `mem_setting_contested`（category `settings`，metadata 带 `item`/`live`/`window_s`/`suppress_s`）。**不静默拉锯**是不做 24 h 观察也能确定的行为边界 |
| 67 | 纠正 | 模块不支持时**不再**暴露可调项：之前 `schema()` 无条件挂两个 tunable，会让不支持设备显示半张空页面；现有测试当场抓到了这一点 |
| 68 | 验证 | 8 项新测试（上限随内核、写入读回、越界不半写、只读项拒写、漂移与清除、三次退避 + 事件一次、boot 清洗）；全量 321 项 + clippy 通过；假根端到端：`set --persist` → `check` 漂移 → `enforce` 修复 → `reset` 回 100 |

**修订 13**（2026-11-25，M3b 的 UI 补完）：

| # | 类型 | 变化 |
|---|---|---|
| 69 | 实现 | 内存页新增「可调参数」卡：两个滑块 + 保存/恢复默认。滑块范围取自模块 `schema`（不硬编码），移动滑块**不改任何东西**，点保存才 `set --persist` 并重新读取 |
| 70 | 实现 | 新增只读项 `mem.mglru`（`/sys/kernel/mm/lru_gen/enabled` 非零即 1）。有它 UI 才能说「本机 MGLRU 已启用，swappiness 效果会被稀释」，而不是含糊地提示「可能无效」 |
| 71 | 验证 | Rust 60 项 mem 测试（新增 MGLRU 项测试）+ Kotlin 解析测试通过；`:app:compileDebugKotlin`、`:app:testDebugUnitTest`、`:app:assembleDebug` 通过。**真机点击保存待验** |

**修订 14**（2026-11-25，三处观感补齐）：

| # | 类型 | 变化 |
|---|---|---|
| 72 | 实现 | 内存页加**前台 3 s 轮询**（`repeatOnLifecycle(STARTED)`，离开页面即停）。此前只有进入时读一次，是唯一不轮询的页面；3 s 比 CPU 页的 1.5 s 更省，因为内存是慢变量 |
| 73 | 实现 | 构成条改为**整机口径**：已用各段 + 缓存（可回收，`MemKind.Reclaimable`）+ 空闲，三段之和 = `Total`；另保留「已用口径」的 `segments` 供文案与测试使用。新增 `MemKind` 枚举，颜色按语义而不是按序号 |
| 74 | 实现 | 新增 `sysflow mem breakdown`（协议 §3.24）：**全量**进程 PSS 扫描，按 UID < 10000 / ≥ 10000 分系统与用户，60 s 缓存；页面加「应用内存」卡。读不到 `smaps_rollup` 的回退 `VmRSS` 并计入 `estimated`，不静默丢样本 |
| 75 | 纠错 | 「系统 + 用户」**不能**与「已用」相加：PSS 按比例分摊共享页、且不含内核/GPU/池。卡片下明确写「此口径与上方已用不同，两者不可相加」 |
| 76 | 验证 | CLI 29 项（新增 4 项 breakdown 测试）、Rust 全量 326 项、Kotlin 单测通过；假 `/proc` 端到端：system 530 000 / user 470 000（含工作资料 UID）、缓存命中 true；APK 已重新打包 |

**修订 15**（2026-11-25，消除与进程模块的观感重叠）：

| # | 类型 | 变化 |
|---|---|---|
| 77 | 改名 | 「应用内存」→「**内存归属（系统 / 用户）**」；「应用内存排行」→「**应用内存排行（按 PSS）**」。两者一个回答"归属"、一个回答"谁大"，名字不能都叫「应用内存」 |
| 78 | 交互 | 排行每行可点开**单应用明细**（UID、PSS、已换出、进程数、占比、包名）。**没有**做「查看进程 ›」跳转：进程页尚不存在，死链比没有更糟；进程级列表与停止归进程模块 |
| 79 | 文案 | 排行下方注明「按 UID 汇总，点一行看单应用明细；进程级列表与强制停止属于进程模块」，把边界写在产品里，而不是只写在文档里 |
| 80 | 验证 | `:app:testDebugUnitTest`、`:app:assembleDebug` 通过 |

**修订 16**（2026-11-25，项目负责人复审：对照 S25 探针结论与协议逐项审读代码，修 6 处 Rust、1 处 APK）：

| # | 类型 | 变化 |
|---|---|---|
| 81 | 纠错 | **`Guard` 的 `pending` 会让修不好的值永远不被报告**：守护循环每个 tick 会对每项调两次 `drift`（先找漂移，修完再查残留）。旧逻辑把第二次当成"正在修复"，于是一个写入被拒绝、或被 ROM 在同一 tick 内改回的值，下一个 tick 起一直答 `Repairing`——既不再修、也不累计、也不发 `mem_setting_contested`，还会让 `enforce` 把没修好的项当作 `drift_repaired`。改为**按 tick 计次**：同一 tick（< 2 s）内的重复询问不额外计数，每个发现仍不一致的 tick 算一次尝试，30 分钟内 3 次即停手并报告；值恢复一致后下一次不一致重新算一次观察。`Decision::Repairing` 随之删除 |
| 82 | 纠错 | **`mem_pressure_high` 附带的「最近一次 LOW_MEMORY」不是最近的**：`exit-info` 按包名分组，旧实现取的是文件里**最后一条**，只是"最后列出的那个包"。改为按 `timestamp=`（定宽 `YYYY-MM-DD HH:MM:SS.mmm`，字典序即时间序）取**最新**一条；无时间戳的记录败给有时间戳的。`parse_last_low_memory` 改名 `parse_newest_low_memory`；原测试的断言恰好把错误行为固化了（`com.example.newest` 的时间戳 22:47 比 `com.example.last` 的 23:26 更早），已更正并补乱序用例 |
| 83 | 缺口 | `mem_oom_kill` 事件文案读 `metadata.count`，但生产者没写这个字段，页面永远走兜底文案。现在写入 |
| 84 | 缺口 | APK `EventCopy.kt` 没有 `mem_setting_contested` 的文案（会直接显示英文原文）。补中文标签与说明（「系统反复把 swappiness 改回 100，已暂停重新应用 6 小时」） |
| 85 | 纠错 | 历史差分的**经过时间改用开机时钟**（`/proc/uptime` 的差），仅在拿不到时回退到墙钟。墙钟会在手机校时时跳变，用它做分母会让 PSI 占比和 swap 速率错一个无法被下游发现的倍数 |
| 86 | 缺口 | `with_exit_info` 只在事件确实会被记录（有事件库）时才 shell 出去，避免无事件库的构造（测试、纯采样）也跑一次 `dumpsys` |
| 87 | 纠错 | `mem apps` 的 `cmd package list` 回退没有超时（直接 `spawn` + `wait_with_output`），包管理服务卡住会拖死整个排行。改为复用 `batterystats::read_package_uids()`（8 s 超时），并删掉重复的解析代码 |
| 88 | 验证 | 新增 10 项 Rust 测试（`Guard` 4 项 + 模块级"修不好的值"1 项 + exit-info 乱序 / 无时间戳 + 开机时钟差分 + oom `count`）；全量 `cargo test --workspace`（201 项 mem 所在 crate）、`clippy -D warnings`、新增文件 `rustfmt --check` 通过；假根端到端：`set` → 持久化并记下默认 130 → ROM 改回后 `check` 报漂移、`enforce` 修复 |

**仍未验证 / 需要真机**（Kotlin 在本环境无法编译，这一轮只改了 `EventCopy.kt`，请跑 `:app:assembleDebug`）：`mem_setting_contested` 的实际触发需要 ROM 真的在改写 `swappiness`（S25 上 48 小时观察没有），所以这条路径目前只有单元测试覆盖。

## 目标

- 让用户看懂"这台设备的内存到底紧不紧"：可用量、swap/zRAM、PSI 压力、谁在占。
- 提供少量、可回滚、有明确边界的系统级调节：最终只做 `swappiness` 与 `vfs_cache_pressure`（`dirty_*` 与 `min_free_kbytes` 经实测后放弃，见修订 4 / #27 与〈写入面契约〉）。
- 全部采集发生在 Root 模块；APK 只展示，不常驻；WebUI 不加内存页（WebUI 是救援控制台，
  见 `docs/webui-console.md`，不渲染模块页面）。

**成功标准**（用于 Step 7 之后的整体验收）：

1. 用户打开内存页 3 秒内能回答「现在紧不紧」，并能看到过去 24 h 压力曲线。
2. 打开页面不拖慢系统：`snapshot` 中 mem 段采集 < 5 ms；daemon 每次采样 < 10 ms。
3. 任何一个写入项都能**一键恢复到 ROM 原值**，且重启 / 安全模式下行为可预测。
4. 在拿不到数据的设备上，页面**降级而不是报错**（隐藏区块、说明原因）。

## 不在范围

| 项 | 原因 |
|---|---|
| zRAM 重配（`disksize` / `comp_algorithm` / `reset`） | `swapoff` 会把已用 swap 拉回内存，当场 OOM 或卡死；M3c 再评估 |
| 每进程动作（杀进程 / `oom_score_adj` / freezer） | 归阶段 4 进程模块；本模块只做只读排行 |
| 迁移 `cpu.ddr_boost_freq` / `cpu.llcc_boost_freq` | 已有持久化配置与 drift 检查，迁移是兼容性负债 |
| 把内存 PSI 接进 `cpu.adaptive` | 跨模块耦合会污染 drift / 回滚 / 安全模式边界；等真机数据稳定后再评估。注：`cpu.adaptive` 现有的 PSI 保护读的是 `/proc/pressure/cpu`，与内存 PSI 互不干扰 |
| "一键清理 / 加速" | `drop_caches` 丢的是可回收 page cache，之后还要重新读盘；只作为诊断动作并写明"通常无益" |
| 内核 LMK 参数调优（`minfree`、`ro.lmk.*`） | 完全 ROM 私有，做了就是伪功能 |
| 调整 / 创建 swap 文件（含三星 RAM Plus 的开关） | 属于 zRAM 重配同类风险，且是 ROM 私有机制；只读展示，不碰 |

## 已定决策

| 项 | 决定 | 依据 |
|---|---|---|
| 模块 id | `mem` | `config.json` 示例已用 `mem` 作模块键，阶段 3 也如此命名；不会与 `cpu::mem`（bus boost）混淆，代码路径用 `core/sysflow-modules/src/mem/` |
| item 前缀 | 统一 `mem.*`，含 `mem.zram.*` | 协议 §1 约定 `<module>.<name>`。`Schema::validate_request` 只校验 key 是否在 schema 内、**不校验前缀**，所以现在改最便宜 |
| 大小单位 | kB | 协议 §1；`/proc/meminfo` 原生 kB，zRAM `mm_stat` 为 byte，Rust 侧 ÷1024 |
| PSI 单位 | 0.01 % 整数（`123` = 1.23 %） | 与温度「0.1 °C 整数」的归一化风格一致；整数百分比在低压力下精度不够 |
| 采样节奏 | 300 s | 30 天 × 20 000 行上限下为 8 640 行，符合保留期；60 s 只剩约 14 天（电池现状） |
| 区间统计 | PSI / vmstat **累计计数器差分** | `some/full total`（微秒）与 `pgscan_direct` 等累计值取差，得到区间内**不丢账**的总量；强度语义见〈采样与历史〉 |
| 快照范围 | 廉价遥测进 `snapshot`；per-app 排行走 `mem apps` | 排行是每进程一次 `open`+`read`，塞进 snapshot 会拖慢 APK 每次前台轮询 |
| 事件库接入 | 生产构造函数从 `state_dir.parent()` 派生 `EventStore` | 与 CPU / wakelock 一致（`EventStore::new(state_dir.parent())`），`with_events` 只作测试钩子 |
| 写入面归属 | 走 CPU 那套 `apply` / `drift` / `sanitize_boot` / `persist` | 内存 sysctl 写后能读回，但"读回一致 ≠ 有效果"；不能照抄 battery 的只读捷径 |
| 事件产出位置 | 在 `sample()` 内、可用性检查**之前**做判定并 `emit` | 与 `wakelock` 的 `watchdog` 一致；这样 `/proc/vmstat` 不可读等降级场景不会连带吞掉事件 |
| 基线单写者 | 只有 daemon 的 `sample()` 写 `state/mem.json` | CLI 的 `snapshot` / `get` 只读瞬时值，不读不写基线，避免两个进程互相覆盖 |

> 与现有文档的冲突：`docs/development-plan.md` §7.4 的 item 表仍写作 `vm.swappiness` /
> `zram.disksize`，§「config.json 示例」（约 L430）还有 `"mem": {"vm.swappiness": 60}`，
> 与本计划的 `mem.*` 前缀不一致。Step 0 一并改成 `mem.*`，避免协议、设计、代码三处各说一套。

## Item 草案

### 只读（进 `snapshot`）

缺失字段**省略**而不是报 0（沿用 battery 的做法）；省略表示"这台设备没有 / 读不到"。

| id | 类型 | 单位 | 来源 |
|---|---|---|---|
| `mem.total` / `mem.available` / `mem.free` / `mem.cached` | readonly | kB | `/proc/meminfo`（`MemTotal` / `MemAvailable` / `MemFree` / `Cached`） |
| `mem.anon` / `mem.shmem` / `mem.slab` | readonly | kB | `/proc/meminfo`（`AnonPages` / `Shmem` / `Slab`），用于「谁在占」的构成条 |
| `mem.dirty` | readonly | kB | `/proc/meminfo`（`Dirty` + `Writeback`），配合 `dirty_*` 调节项的效果观察 |
| `mem.swap_total` / `mem.swap_free` | readonly | kB | `/proc/meminfo`（交叉校验 `/proc/swaps`） |
| `mem.swap.zram_kb` / `mem.swap.other_kb` | readonly | kB | `/proc/swaps` 按设备类型拆分（见下「非 zRAM swap」） |
| `mem.psi.some_avg10` / `some_avg60` / `some_avg300` | readonly | 0.01 % | `/proc/pressure/memory` |
| `mem.psi.full_avg10` / `full_avg60` / `full_avg300` | readonly | 0.01 % | 同上 |
| `mem.zram.count` | readonly | 个 | `/sys/block/zram*` 数量 |
| `mem.zram.disksize` / `orig` / `compr` / `mem_used` | readonly | kB | `/sys/block/zram*/{disksize,mm_stat}`，多设备求和 |
| `mem.zram.ratio` | readonly | % 整数 | 聚合后重算：`sum(orig) ÷ sum(compr)`，不对各设备比值求平均 |

**占用分解的口径**：`anon + cached(file) + shmem + slab + 其他 ≈ total - free`，UI 只画前三项加「其余」，
不要把 `Cached` 与 `Shmem` 重复相加（`Cached` 已含 `Shmem`）。`dmabuf` / GPU 显存在新内核里占比可观，
但读取路径随内核与 ROM 变化大，**只在 Step 1 探针里记录是否可读**，不进 v1 schema。

**非 zRAM swap**：部分 ROM（三星的「RAM Plus」等）会在存储上挂 swap 文件，在 `/proc/swaps` 里表现为
`/dev/block/loop*` 或文件路径。v1 行为：`mem.swap_total/free` 照 meminfo 报总量；`zram_kb` / `other_kb`
按 `/proc/swaps` 里设备名是否以 `zram` 开头拆分；`other_kb` 非 0 时 UI 标注「含存储 swap」。
**不碰**这类 swap。具体形态以 Step 1 真机输出为准。

### 累计计数器（只进 `history`，不进 `snapshot`，见待定项 2 的推荐）

| 来源 | 计数器 | 用途 |
|---|---|---|
| `/proc/pressure/memory` | `some total` / `full total`（µs） | 区间平均停顿占比 |
| `/proc/vmstat` | `pgscan_direct`（**精确名**，不含 `pgscan_direct_throttle`）、`allocstall_{dma,dma32,normal,movable,device}`（白名单求和，**不要**通配 `allocstall_*`）、`oom_kill`、`compact_stall` | 直接回收 / 分配停顿 / 内核 OOM |
| `/proc/vmstat` | `pswpin`、`pswpout` | swap 读写速率（页 → kB，页大小取 `getpagesize`，别写死 4 kB） |
| `/proc/vmstat` | `workingset_refault_anon` / `workingset_refault_file`（旧内核为 `workingset_refault`） | 颠簸（thrashing）指标：刚被回收又被读回 |

计数器名在不同内核版本间会改名 / 拆分，**按名字取、取不到就省略该指标**，不得按行号或位置取。

### 可写（M3b）

| id | 类型 | 范围 | 风险 | 备注 |
|---|---|---|---|---|
| `mem.swappiness` | int_range | 0–100（≥ 5.8 内核 0–200） | low | 上限按 `kernel_release` 选择（`registry.rs` 的 `info` 已有该字段）；0 在旧内核仍会 swap，不能宣传成"省电" |
| `mem.vfs_cache_pressure` | int_range | 1–1000 | low | 0 表示不回收 dentry/inode，不推荐，不给 0 |
| `mem.dirty_ratio` / `mem.dirty_background_ratio` | int_range | 1–100，且 `background < ratio` | medium | 见〈写入面契约〉：写入会清零对应的 `*_bytes`，ROM 用了 bytes 时默认隐藏 |
| `mem.min_free_kbytes` | int_range | 探测后定 | high | **不持久化**（`persist=false`），仅实验区；水位线写大了直接 OOM |
| `mem.drop_caches` | action | 写 `3` | low | 不持久化、不参与 drift；UI 放诊断区并注明"通常无益" |

### Verb

- `sysflow mem apps [--limit N] [--refresh]`：RSS 粗排 → Top N 取 `smaps_rollup` 的 PSS 与 `SwapPss`，
  按 UID 聚合，UID→包名复用 `sysflow_core::batterystats::read_package_uids()`，结果缓存 60 s。
  不进 `snapshot`。性能与权限细节见〈`mem apps` 细则〉。
- `sysflow mem probe`（隐藏，仅调试）：输出各数据源的原始读数与判定，Step 1 的产出物，不进协议正文。

### history 序列（v1 定稿，键名不带 `mem.` 前缀，与 battery 的 `level` 一致）

| series | 单位 | 来源 / 推导 |
|---|---|---|
| `available_kb` | kB | meminfo 瞬时值 |
| `swap_used_kb` | kB | `swap_total - swap_free` |
| `zram_compr_kb` | kB | mm_stat 瞬时值 |
| `psi_some_pct` / `psi_full_pct` | 0.01 % | `Δtotal ÷ 实际经过 µs`（区间平均停顿占比） |
| `pgscan_direct` | 页 | 区间差分 |
| `allocstall` | 次 | 区间差分 |
| `swap_in_kb` / `swap_out_kb` | kB | `Δpswpin/pswpout × 页大小` |
| `refault` | 页 | `workingset_refault_*` 区间差分 |
| `oom_kill` | 次 | 区间差分 |
| `screen`（由 daemon 附带） | 0/1 | 协议 §3.9 已有，可拆亮屏 / 息屏压力 |

## 数据来源与降级

| 数据 | 路径 | 缺失/不可用时的行为 |
|---|---|---|
| 内存总量与可用 | `/proc/meminfo` | 模块整体不可用（`unsupported`） |
| 压力 | `/proc/pressure/memory` | PSI 项消失，UI 隐藏压力区块。**两种形态**：文件不存在（未编 `CONFIG_PSI`），或文件在但读取返回错误（运行时 `psi=0` / 默认关闭）——两者都按"不可用"处理，探针记录区别 |
| zRAM | `/sys/block/zram*/` | zRAM 项消失；`mm_stat` 缺失但 `disksize` 存在时只报容量。`mm_stat` 列数随内核版本不同（旧 7 列、新 9 列），只取前三列并**按列容忍多余** |
| swap | `/proc/swaps` | 仅用 meminfo 的 `SwapTotal/SwapFree`，不拆分类型 |
| 回收压力 | `/proc/vmstat` | 差分项消失；单个计数器缺失只省略该项 |
| 进程 RSS / PSS | `/proc/<pid>/status`、`/proc/<pid>/smaps_rollup` | 单进程读失败跳过，不影响整体 |
| 包名 | **首选** `/data/system/packages.list`（S25 实测可读，10 列，只取前两列）；失败回退 `cmd package list packages -U` | 只有 UID 时显示 UID（沿用 `wakelock held` 的 `owner` 规则）。匹配用 `uid % 100000`，非 0 用户在标签里标注用户 id |
| 低内存机型 | `getprop ro.config.low_ram` | 用默认阈值 |
| OOM / LMK 记录 | S25 实测：`dumpsys activity exit-info` 的 `reason=3 (LOW_MEMORY)` 为可靠来源；`dmesg` 无杀进程行；`am_kill` 混有非内存原因（`empty #N`） | 每个来源独立 probe；全部不可用即退化为 PSI 压力代理，**不猜**。`exit-info` 一次 2.5 MB，只在事件触发或用户查看时按需读取，不进周期采样 |

**注意**：`/proc/vmstat` 的 `oom_kill` 只统计**内核** OOM；Android 的 `lmkd` 杀进程不一定计入。
"谁被杀"必须靠上表的日志来源（且要 probe），拿不到就只报压力，不猜。logcat 环形缓冲很小，
只能回溯最近一段，不能当历史库用；因此 v1 事件**不依赖**它，只把它作为 `mem_oom_kill` 事件
`metadata` 里的**可选补充**。

## 采样与历史

- `sample_interval()` = 300 s；`Sample` 指标见上表「history 序列」。
- **语义（修正）**：累计计数器的差分保证区间内**总停顿时间不丢**，所以 5 分钟内哪怕只有 2 秒的
  `full` 停顿也会在 `psi_full_pct` 里留下一个非零点（约 0.67 %）；但它给出的是区间**平均强度**，
  **不是峰值**。UI 与文案不得把它说成"捕获到了尖峰的高度"，只能说"这 5 分钟内累计停顿 X 秒"。
  若之后确实需要峰值，再评估增加一个仅读 PSI 的快采样（例如 15 s 读 `avg10` 取窗口最大值），
  不在 v1 范围。
- **用实际经过时间**：每次采样用 `Instant`/uptime 计算 `dt`，PSI 占比与 swap 速率都除以实际 `dt`，
  不假设恰好 300 s——设备深度休眠时 daemon 可能被推迟，名义间隔会算出虚高的速率。
  `dt` 超过 `3 × interval` 时该点标记为"间隔过长"：照常记录差分总量，但不触发事件。
- **基线存 `state/mem.json`**：PSI total、vmstat 计数、boot id、uptime、上次采样 uptime。
  重启判定复用 wakelock 的 boot id → uptime → 计数回退三级逻辑。
- **重启 / 回退后的首个差分点记 `null`（省略），而不是 0**（修订；初版写作"按 0 计"）。
  理由：0 在图上读作"一切平静"，而此时只是不知道；wakelock 用 0 是因为它画的是累计时长，口径不同。
  同一次回退**不得**触发 `mem_oom_kill` 等事件，只重建基线。
- 屏幕状态由 daemon 写入（`"screen":1|0`），可拆亮屏 / 息屏压力。
- 已知账目：`MAX_LINES = 20 000` 与 30 天保留互相制约，电池 60 s 实测只留约 14 天；
  内存 300 s 为 8 640 行，不受影响；按每行约 200 B 估算，单文件约 1.7 MB。
  是否调大上限见待定项 4。

## 事件

| event_type | category | 严重度 | 触发 | 去重 |
|---|---|---|---|---|
| `mem_pressure_high` | `pressure` | `warning` | 区间 `psi_full_pct` 连续 N 个采样 ≥ 阈值 | 去重键 `mem.pressure.high` + 窗口（参考 `wakelock_long_held`，默认数小时） |
| `mem_pressure_recovered`（可选） | `pressure` | `info` | 连续 M 个采样低于恢复阈值 | 与上同键，仅在已发过 high 之后才发 |
| `mem_oom_kill` | `pressure` | `notice` | `oom_kill` 差分 > 0（内核 OOM）。**S25 上开机以来恒为 0，实际几乎不会触发**，保留是为了别的 ROM | 按开机周期 + 计数去重，避免刷屏 |

- **迟滞**：触发阈值 > 恢复阈值（例如 3.0 % / 1.0 %），避免在阈值附近来回刷事件。
- 阈值初值（**占位，待 Step 3 真机数据定**）：

  | 档位 | 触发（`psi_full_pct`） | N | 恢复 | M |
  |---|---|---|---|---|
  | 默认 | ≥ 3.00 % | 2（≈10 min） | < 1.00 % | 2 |
  | `low_ram` | ≥ 6.00 % | 2 | < 2.00 % | 2 |

- `metadata` 带上当时的 `available_kb` / `psi_full_pct` / `psi_full_avg60` / `swap_used_kb` /
  `screen`，方便事后判断"是内存真紧还是阈值太敏感"。
- **文案表**：协议要求事件文案由 APK 侧表驱动。新增 `event_type` 时必须在
  `app/src/main/kotlin/io/sysflow/feature/home/EventCopy.kt` 补：模块名 `内存`、
  各事件的中文标题与一句话解释。未补也能显示（回退到事件自带的英文 `title`/`message`），
  但不合格，Step 4 的验收包含这一项。
- 事件写入失败只吞掉不影响采样（`let _ = events.emit(..)`），与现有模块一致。

## 写入面契约（M3b 前必须定稿）

### 1. 持久化与 reset

| 项 | `persist` | 进 drift | reset 行为 |
|---|---|---|---|
| `mem.swappiness` / `vfs_cache_pressure` / `dirty_*` | `true` | 是 | 恢复**首次接管前 ROM 的原值**（`normalize_default` 在首次写入前抓取并存为默认快照） |
| `mem.min_free_kbytes` | `false` | 否 | 只在本次开机生效；重启即回 ROM 值；reset 时若读回值与默认不同则写回 |
| `mem.drop_caches` | `false`（action） | 否 | 无状态，无需恢复 |

**S25 实测**：`swappiness` 的 ROM 默认值是 **130**（不是 60），`dirty_*` 的 ratio 文件读出 0 而 bytes 文件非 0。
这两条正是上面「默认快照取 ROM 原值」与「§2 `*_bytes` 与 `*_ratio` 互斥」要防的情形。

`normalize_default` 要做的事：抓默认值时若读到的已是本模块上次写入的值（例如 daemon 重启后 reset），
不能把它当 ROM 原值存下来——默认快照一旦落盘就**不再被覆盖**。

### 2. 写入顺序与约束

- `dirty_background_ratio` 必须小于 `dirty_ratio`；否则内核会把后台阈值按 `ratio / 2` 处理，
  读回值与请求不符。`validate` 阶段就拒绝 `bg >= ratio`，错误码 `invalid_arg`。
- 同一次请求同时改两者时的写入顺序：**调高**时先写 `dirty_ratio` 再写 background；
  **调低**时先写 background 再写 `dirty_ratio`，保证任何中间态都满足 `bg < ratio`。
- **`*_bytes` 与 `*_ratio` 互斥**：内核里写 `dirty_ratio` 会把 `dirty_bytes` 清零（background 同理）。
  所以 probe 到 ROM 把 `dirty_bytes` / `dirty_background_bytes` 设为非 0 时，
  不是"ratio 无效"，而是**写 ratio 会改掉 ROM 的选择**。处理：此时 `mem.dirty_*` 项默认隐藏，
  不自动覆盖；若以后要开放，需二次确认并在 reset 中把 bytes 值一并还原。
- `swappiness` 上限按 `kernel_release` 判定；解析失败时按保守上限 100。

### 3. 与 ROM 守护进程的拉锯

部分 ROM 的内存管理守护进程（例如三星的 `mmd`，需 Step 1 实测确认是否存在及行为）会在运行时
**周期性改写** `swappiness` 等参数。若 daemon 的 drift 自愈发现一次就改回，两边会无限拉锯，
既耗电又让 UI 状态抖动。约束：

- 同一 key 的自愈有**频率上限**（例如 10 分钟内最多 1 次）；连续 K 次被改回则标记
  `contested`，停止自愈，向 UI 报告「该参数被系统改写，设置可能不稳定」。
- `contested` 状态写进 `get` 返回的附加字段，不新增 error code。
- 这一条的实际形态取决于 Step 1 的观察（在 S25 上连续读 `swappiness` 24 h，看是否变化）。

### 4. 开机时序

`apply-boot` 写入时，ROM 的 init / vendor 脚本可能**随后**再改写同一 sysctl。
因此开机应用后必须在一段延迟后（例如 boot-confirm 之后）再做一次读回校验，
由 drift 路径兜底，而不是假设一次写入永久有效。具体延迟交给 Step 8 的真机重启回放定。

### 5. 安全模式与 `sanitize_boot`

- 安全模式下写入面**整体跳过**，只读遥测照常。
- `sanitize_boot`：按当前 `kernel_release` 丢弃越界值（内核升级后 `swappiness` 上限变化等），
  只丢越界的那一项，不影响其他持久化项。

## `mem apps` 细则

- **流程**：枚举 `/proc/<pid>` → 读 `/proc/<pid>/status` 取 `VmRSS` / `Uid` 粗排 → Top N（默认 N=30，
  上限 100）读 `smaps_rollup` 取 `Pss` 与 `SwapPss` → 按 UID 聚合 → 映射包名。
- **字段**：每条 `uid`、`label`（包名或 aid 名，复用 `aid_label`）、`pss_kb`、`swap_pss_kb`、
  `procs`（进程数）。`swap_pss_kb` 能直接回答「这个 App 有多少被压进了 zRAM」。
- **为什么 PSS**：RSS 会把共享页重复计入各进程，聚合到 UID 后严重虚高；PSS 按共享者均摊。
  但 PSS 只对 Top N 取，其余用 RSS 估，输出里要标注 `pss_for_top_n_only`。
- **性能预算**：目标 < 300 ms 指**热缓存路径**（UID 映射已缓存）。冷路径里
  `cmd package list packages -U` 是 binder 调用，`read_package_uids` 的超时是 8 s，
  不可能保证 300 ms。因此：
  - 排行结果缓存 60 s（沿用 `battery_apps` 的 `state/*.json` 缓存方式）；
  - **UID→包名映射单独缓存，TTL 10 分钟**，并在发现未知 UID 时才触发刷新；
  - `/data/system/packages.list` 已在 S25 验证可读（695 行，毫秒级），作为首选；`cmd package` 只做回退。
  - 该文件按 appId 记录（如 `10346`）；分身 / 安全文件夹里的进程 UID 带用户前缀（如 `9510346`），必须用 `uid % 100000` 匹配，并在标签里标注用户 id。
- **权限**：`/proc/<pid>/smaps_rollup` 需要 root；SELinux 在部分 ROM 上会拒绝对特定域进程的读取，
  单进程读失败即跳过并计入 `skipped`，不报错。
- **一致性**：采样期间进程会退出，读取中途 ENOENT 属正常，同样跳过。
- **并发**：沿用 battery 的做法，子进程调用加超时，避免 wedge 住调用方。
- **验收对拍**：与 `dumpsys meminfo` 的前几名比较**顺序**（不要求数值相等，二者口径不同）。

## 交付顺序

原则：**契约冻结 → 真机探针 → 只读 → 历史 → 事件 → 排行 → UI → 写入面**，
zRAM 写入留到最后甚至不做。

| # | 步骤 | 产出 | 验收 | 依赖 | 粗估 | 状态 |
|---|---|---|---|---|---|---|
| 0 | 契约冻结 | 协议 §3.22 / §3.23 草案 + `docs/modules/mem.md` 路径矩阵骨架（值留空）+ 把 `development-plan.md` §7.4 与 `config.json` 示例里的 `vm.*` / `zram.*` 改成 `mem.*` + 定稿 history 序列表 | 评审通过 id / 单位 / 节奏 / 序列名 | — | 0.5 d | ✅ 已完成（契约冻结后由 Step 1 实测回填，`development-plan.md` §7.4 已改为已实现口径） |
| 1 | 探针 | `mem schema` 的 supported + 隐藏 `mem probe` 输出原始读数与判定；**连续观察** `swappiness` 等是否被 ROM 改写；确认 `packages.list`、`logcat -b events`、`dmesg`、非 zRAM swap 形态 | 真机输出据此裁剪清单；矩阵里每个「待验证」都有结论 | 0 | 1.5 d | ✅ 探针已在 S25 跑通两轮（基础版 + 增强版），宿主 26 项测试 + clippy 通过；**24 h 观察仍待补**（脚本已改为单行命令版，见 `docs/modules/mem.md`） |
| 2 | 只读遥测 | `MemModule` 的 `probe`/`schema`/`collect`，全 `Readonly`，进 `snapshot` | `snapshot` 出现 mem 段，采集 < 5 ms；缺失字段省略 | 1 | 1.5 d | ✅ 主机完成（31 项 mem 测试 + clippy；假根 `collect()` 中位 0.12 ms、端到端 `snapshot` 4 ms/debug），**真机计时与逐项对拍待补** |
| 3 | 历史采样 | `state/mem.json` 基线 + PSI / vmstat 差分（用实际 `dt`、重启首点 null） | `history mem` 有 series，daemon 重启无负值或假峰；休眠后无虚高速率 | 2 | 1.5 d | ✅ 主机完成（41 项 mem 测试 + clippy；短跑 daemon 验证：首点仅 3 个瞬时值，次点 11 个指标，`state/mem.json` 落盘），**真机 300 s 节奏待验** |
| 4 | 压力事件 | `mem_pressure_high` / `mem_oom_kill`（+ 可选 recovered）+ `EventCopy.kt` 文案 | 单测覆盖去重、迟滞与重启，真机抽样；文案表已补 | 3 | 1 d | ✅ 主机完成（49 项 mem 测试 + clippy；事件在 `sample()` 内产出，含死区断链 / 长间隔抑制 / 重启不补发 / exit-info 解析），**真机噪音水平待验** |
| 5 | `mem apps` | 独立 verb + 60 s 排行缓存 + 10 min UID 缓存 | 与 `dumpsys meminfo` 前几名顺序一致；热缓存 < 300 ms | 2 | 2 d | ✅ 主机完成（25 项 CLI 测试；假 `/proc` cold 5 ms / hot 8 ms，排行缓存按 `limit` 失效、UID 映射单独 10 min），**真机耗时与对拍待验** |
| 6 | APK 图表抽取 | `Sparkline` / 曲线绘制上移 `core/ui/components/` 并泛化成 series 版 | 电池页行为零变化，可单独 review | — | 0.5 d | ✅ 完成（`core/ui/components/Chart.kt`：`Sparkline` 逐字迁移 + 新增 `LineChart`/`ChartPoint`；`:app:compileDebugKotlin` 与 `:app:testDebugUnitTest` 通过） |
| 7 | APK 内存页 | 可用内存 hero、构成条、swap/zRAM 条、PSI 24 h 曲线、排行页、首页入口卡 | 真机可视化对拍；各降级状态可见（见下） | 5、6 | 2 d | ✅ 主机完成（`feature/mem/` 三个文件 + 导航/注入接线；`:app:compileDebugKotlin`、`:app:testDebugUnitTest`、`:app:assembleDebug` 通过），**真机可视化待验** |
| 8 | 写入面（M3b） | 三个 sysctl 的完整 apply / drift / boot / safe-mode，按〈写入面契约〉 | 重启回放、drift 自愈且不拉锯、安全模式跳过、reset 回 ROM 原值 | 7 | 3 d | ✅ 主机完成（只做 `swappiness` + `vfs_cache_pressure`；写入前整体校验、读回、contested 退避、boot 清洗；59 项 mem 测试 + 全量 321 项 + clippy；假根端到端验证 set→check→enforce→reset），**真机待验** |
| 9 | M3c | zRAM 重配、每进程动作 | **暂缓**：与阶段 4 进程模块一起重估 | — | — | ⏸ |

合计（0–8）约 13.5 d。**并行**：Step 6 与 Step 1–5 无冲突（只动 `app/` 与 `core/ui/`），可以先做。
Step 0 之后协议细节随 Step 2 回填，不提前写死未验证行为。

**每步完成定义**：`cargo fmt` + `clippy -D warnings` + `cargo test`（假根用 `SYSFLOW_SYSFS_ROOT`，
`/proc` 也在同一根下——`Sysfs::resolve` 统一拼接根目录，CPU 的 PSI 读取就是这样测的）+ 真机抽样输出 +
更新 `docs/modules/mem.md` 的实测值与降级原因。

## 测试计划

**Fixture 清单**（放在假根的 `proc/`、`sys/` 下）：

| 场景 | 要覆盖的点 |
|---|---|
| 正常设备 | meminfo、PSI、vmstat、单 zram、swaps |
| 无 PSI | 文件不存在 / 读取报错，两种形态 |
| 多 zram | `zram0` + `zram1`，聚合与 ratio 重算 |
| `mm_stat` 变体 | 7 列 / 9 列 / 缺失 |
| 非 zRAM swap | swaps 里混入 loop 设备 |
| 缺 `MemAvailable` | 老内核 |
| vmstat 改名 | `workingset_refault` 与 `_file` / `_anon` |

**差分与基线用例**（`mem/tests.rs`）：计数器正常递增；计数器回退（重启）；boot id 变化；
uptime 回退；时钟跳变；`dt` 过长；首次运行无基线；基线文件损坏（降级为重建，不 panic）。

**事件用例**：连续 N 次触发才发；窗口内去重；迟滞；重启后不补发；`oom_kill` 回退不触发。

**写入面用例**（M3b）：越界拒绝；`bg >= ratio` 拒绝；调高 / 调低的写入顺序；`dirty_bytes` 非 0 时隐藏；
`persist=false` 项不落盘；drift 自愈频率上限与 `contested`；安全模式跳过；reset 回默认；
默认快照不被二次覆盖。

**性能**：Step 2 / 3 / 5 各自记录实测耗时到 `docs/modules/mem.md`，回归时对比。

## APK 页面状态

| 状态 | 条件 | 页面表现 |
|---|---|---|
| 正常 | 数据齐全 | 全部区块 |
| 无 PSI | `mem.psi.*` 缺失 | 隐藏压力区块与曲线，保留可用 / swap / zRAM；说明"内核未开压力统计" |
| 无 zRAM | `mem.zram.*` 缺失 | 隐藏 zRAM 条；仍显示 swap 总量 |
| 无历史 | daemon 未运行 / 序列为空 | 曲线区显示空态并解释（沿用 `daemon_running` 字段） |
| 数据过期 | 历史最新点过旧 | 曲线标注"采样已停止"，不拿旧数据当现状 |
| 无权限 / 不支持 | `unsupported` / `permission` | 按协议 §2 的 UI 行为表处理 |

写入区（M3b）只在 `supported` 且非安全模式时出现；`contested` 项显示警示而非隐藏。

## 风险登记

| 风险 | 影响 | 缓解 |
|---|---|---|
| ROM 守护进程改写 sysctl | 设置不稳、drift 拉锯、耗电 | 〈写入面契约〉§3 的频率上限与 `contested`；Step 1 先观察 |
| 内核计数器改名 / 拆分 | 差分项静默消失 | 按名取、缺失省略、fixture 覆盖多个命名变体 |
| `mem apps` 冷路径过慢 | 排行页卡顿 | 双层缓存；首选 `packages.list` |
| 误把平均强度当峰值 | 误导用户 | 文案与文档统一口径（见〈采样与历史〉） |
| `min_free_kbytes` 写错 | 设备 OOM / 假死 | `persist=false`、默认隐藏、二次确认、范围由探测给出，重启即复原 |
| 与阶段 4 进程模块重叠 | 重复造轮子 | `mem apps` 只读，输出 schema 预留可被进程模块复用的字段 |

**回滚**：只读阶段（Step 0–7）无系统状态改动，回滚 = 删模块注册一行；写入阶段（Step 8）回滚 =
`reset mem` + 安全模式跳过，二者在 Step 8 验收里必须各演练一次。

## 待定问题（含推荐默认值）

> 每项给出推荐；你拍板后 Step 0 就能冻结契约。

1. **zRAM 多设备**：`zram0` 之外可能还有 `zram1`。**推荐：v1 聚合**（`disksize`/`orig`/`compr` 求和、
   比值重算），另给 `mem.zram.count`；单设备细节留到 M3b。
2. **累计计数器的暴露面**：`_total` / `pgscan_direct` 这类累计值对 UI 无意义，只对差分有用。
   **推荐：只留在 `history` 内**，`snapshot` 不暴露；调试需要时走隐藏的 `mem probe`。
3. **事件阈值**：`full` 占比超过多少、连续几个采样算"高压力"；`low_ram` 机型是否放宽。
   **推荐：先用〈事件〉里的占位值上线，Step 3 的真机数据到手后再改**，文档里标注"占位"。
4. **history 上限**：是否顺手把 `MAX_LINES` 调大（或按模块可配），让电池也能真正留 30 天。
   **推荐：单独提一个小改动，与内存模块不同批**（`MAX_LINES` 是全局常量，改它要同时改协议 §3.9
   的描述与 `history.rs` 的测试），避免内存模块评审被无关改动拖住。
5. **`drop_caches` 是否进 UI**：**推荐：进诊断区、标注"通常无益"，不做主按钮。**
6. ~~**协议编号**~~ ✅ 已处理：`protocol.md` 的两个 §3.11 已合并——「CPU extras」保留 §3.11，
   原「boot, recovery and daemon」并入 §3.13「boot, recovery, daemon and safe mode」，全仓对
   `§3.11` 的引用已核对。`BootStatus` 类型**仍未补**（见 `docs/protocol.md` §4 的已知缺口）。
7. ~~**（新）事件是否发 `recovered`**~~ ✅ 已定并实现：v1 发，但去重键**各自独立**（`mem.pressure.high` /
   `mem.pressure.recovered` / `mem.oom_kill`）——共用一个键会让 recovered 被 high 的窗口吞掉（修订 7 / #41）。
8. **（新）非 zRAM swap 的 UI 呈现**：**推荐：只读标注「含存储 swap」**，不单独做一条；
   形态以 Step 1 为准，若设备上没有就整段不做。
9. **（新）`mem apps` 的默认 N 与排序键**：**推荐：N=30，按 PSS 降序**；
   是否额外提供按 `SwapPss` 排序，等 Step 7 页面设计时再定。
10. ~~**（新）S25 上的 dirty 调节项**~~ → **已定（修订 4 / #27）**：M3b **不暴露任何 `dirty_*`**。ROM 用
    bytes（100 MiB / 25 MiB），且已把 `watermark_scale_factor` 调到 72、`page-cluster` 调到 0，
    写 vm sysctl 与 ROM 调参拉锯的风险大于收益；只做 `swappiness` 与 `vfs_cache_pressure`。
11. ~~**（新）杀进程信号**~~ → **布局已确认（修订 4 / #28）**：保留 `mem_oom_kill` 但降为次要；
    `mem_pressure_high` 触发时读一次 `exit-info`，按「`process=… reason=3 (LOW_MEMORY)` 行 + 上一行
    `timestamp/pid/packageUid/user` + 下一行 `rss`」解析后写入事件 `metadata`。一次 2.5 MB，
    不进周期采样。
12. ~~**（新）内存构成条的口径**~~ → **已定（修订 4 / #23）**：构成条只用 `/proc/meminfo`，字段为
    `AnonPages` / `Slab`（=`SReclaimable`+`SUnreclaim`）/ `KernelStack` / `PageTables` / `Percpu` /
    `GpuTotal` / `RbinTotal` / `CmaTotal` / `DmaHeapPool` / `VmallocUsed`，余量归「其他」。
    **不解析 dmabuf 逐 buffer。**
13. **（新）`mem_pressure_high` 的阈值来源**：探针自身会把 `full avg10` 推到 3 % 以上（修订 4 / #30）。
    **推荐：Step 3 先跑一段不含探针的安静窗口（≥ 6 h），用其 p95 定阈值，再用占位值上线。**

## 参考实现

- 模块骨架与只读遥测：`core/sysflow-modules/src/battery/`（`node()` 探测、缺失字段省略而不是报 0）。
- 累计计数器基线：`core/sysflow-modules/src/wakelock/window.rs`（重启三级判定；注意本计划在
  "重启后首点"上与其有意不同：记 null 而非 0）。
- 在 `sample()` 内发事件：`core/sysflow-modules/src/wakelock/mod.rs` 的 `watchdog`
  （`wakelock_long_held`，含去重键与窗口）。
- 按需 verb + 缓存：`core/sysflow-cli/src/battery_apps.rs`、`wakelock_apps.rs`（60 s 缓存、`--refresh`）。
- CLI 路由：`core/sysflow-cli/src/main.rs` 的 `external_subcommand` + `"apps" if module == …` 分支。
- 模块注册：`core/sysflow-modules/src/lib.rs` 的 `builtin()`（只读模块仿 `BatteryModule::new`，
  带状态与事件的仿 `WakelockModule::new(.., state_dir)`）。
- 采样循环与 `screen` 附带：`core/sysflow-core/src/registry/daemon.rs` 的 `sample_due_modules`。
- 图标/曲线：`app/src/main/kotlin/io/sysflow/feature/home/BatteryChart.kt`（Step 6 抽取来源）。
- 事件文案表：`app/src/main/kotlin/io/sysflow/feature/home/EventCopy.kt`。
