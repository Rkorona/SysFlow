# 进程模块（M4）开发计划

> 状态：**Step 0–5 主机完成（2026-11-25）**：契约、S25 探针、只读面、动作、实验动作与 APK 页面代码均已落地；APK 由用户构建，真机写路径与验收待跑。
> 修订 2 记录了真机推翻的三个假设（sysconf、renice 增量、freezer 路径）；未验证的行为仍不写进协议正文（§3.25 Draft）。
>
> 关联：`docs/development-plan.md` §7.5（功能设计）与 §12 阶段 4（里程碑）、
> `docs/protocol.md` §3.25（本模块草案）、`docs/modules/process.md`（路径矩阵，Step 1 回填）、
> `docs/mem-module-plan.md`（交付流程与测试计划的模板）。

## 修订记录

**修订 1**（Step 0 初稿，2026-11-25）：

| # | 类型 | 内容 |
|---|---|---|
| 1 | 定稿 | 模块 id `process`；CLI `sysflow process …`；协议 §3.25；不占用 `proc`（与 `/proc` 混淆） |
| 2 | 定稿 | v1 **只做一次性动作**，不持久化 oom / nice / cpuset；不参与 boot / drift / safe-mode / daemon |
| 3 | 定稿 | `in_snapshot = false`（模块 trait 注释早已预留）；列表走 verb，`--no-foreground` 时不做 dumpsys |
| 4 | 定稿 | CPU% 为**单核百分比**（0–100 × 核数），两次 `/proc/<pid>/stat` 差分，默认窗口 500 ms、可调 200–2000 ms |
| 5 | 定稿 | 保护名单：`uid < 10000` 硬保护 + 少量包名 denylist + 内核线程；v1 不可配置 |
| 6 | 定稿 | 受保护目标一律 `invalid_arg` + `detail.protected=true`；包内任一 PID 受保护则整个请求拒绝，不部分执行 |
| 7 | 剪裁 | `chrt` 不进 v1（niche、与 EAS/HAL 调频冲突）；应用级 cpuset 与 freezer 只做**探测到才出现**的实验动作 |
| 8 | 剪裁 | 进程模块不提供进程历史、不按 UID 聚合 PSS（`mem apps` / `mem breakdown` 已覆盖），不迁移 CPU 模块的 `cpu.cpuset.*` |
| 9 | 明确 | Step 9（zRAM 重配）重估结论：**继续暂缓**，不作为 M4 依赖；理由见〈待定问题〉#10 |

**修订 2**（Step 1 真机探针，S25 2026-11-25，`/sdcard/process-probe.json`）：

| # | 类型 | 变化 |
|---|---|---|
| 10 | 实测 | `/proc` 在 su 域**全量可读**：1198 进程、0 不可读、首遍 28 ms；`list` 预算无需压缩 |
| 11 | 纠错 | `sysconf` 在 bionic 上不可用（`_SC_CLK_TCK` 返回 2048、`_SC_PAGESIZE` 返回 1）；时钟改读 `/proc/self/auxv` 的 `AT_CLKTCK`(17) |
| 12 | 纠错 | toybox `renice -n` 是**增量**而不是绝对值；改用 `setpriority(2)` 设绝对值，`renice` 只做回退 |
| 13 | 缺口 | freezer 不在 `/sys/fs/cgroup/uid_*`，v2 统一路径是 `/apps/uid_<uid>/pid_<pid>/`；`am help` 有 `freeze` → Step 4 按这两条实现 |
| 14 | 实测 | cgroup **v1** cpuset 有 13 个组、`tasks` 可写；`/dev/cpuset` 顶层混有普通文件，组发现必须过滤目录 |
| 15 | 实测 | 前台行是 `topResumedActivity=ActivityRecord{… u0 <pkg>/<activity> …}`，解析器已验证；`packages.list` 698 行 / 10 列 |
| 16 | 实测 | `comm` 截断到 15 字符（`.wallpaper.live`）→ 名称优先取 `cmdline`；SELinux `Enforcing`，写路径未验证 |

## 目标

- 让用户看清"**现在谁在占 CPU / 内存**"，并能回答"这个包是谁、几个进程、能不能动"。
- 提供少量对**单个应用**的操作，全部一次性、可读回、有保护边界：强制停止、`oom_score_adj`、`renice`；
  cpuset / freezer 仅在探测到可用机制时以实验功能出现。
- 页面只在进入时采样，离开即停；不新增常驻、不新增后台采样、不把进程列表塞进 `snapshot`。

**成功标准**（Step 6 验收）：

1. 进入进程页 ≤ 1 s 出现首屏（500 ms 采样窗口 + 扫描 + 前台识别）；之后 2 s 轮询不卡顿、不持续占用 CPU。
2. 按 CPU / RSS 排序与 `top`、`dumpsys meminfo` 的前几名**顺序一致**（不要求数值相等，口径不同）。
3. 受保护目标（`uid < 10000`、SystemUI 等）**任何动作都被拒绝**，错误里给出原因；单测覆盖。
4. 每个动作都返回 `before` / `applied` / `verified`，读回不一致时如实标记；误操作不影响开机与配置。
5. 拿不到 `/proc/<pid>` / `dumpsys` / 包名映射时**降级而不是报错**：字段省略、`skipped` 计数、`available:false` + 原因。
6. `snapshot` 与 `caps` 的耗时无可感知变化（模块不参与 snapshot；`probe()` 只做裸文件存在性检查）。

## 不在范围

| 项 | 原因 |
|---|---|
| 守护进程式进程监控 / 自动杀后台 / 自动保活 | 违背"零常驻"与"不做自动智能调度"（`development-plan.md` §1.2）；误杀代价不可控 |
| 进程级历史序列（`history process`） | 每 2 s 数百行、30 天保留会写爆 JSONL；内存趋势 `mem apps` 已覆盖；隐私价值低 |
| 持久化 `oom_score_adj` / `nice` / cpuset 归属 | AMS 在进程状态变化时重置这些值；按 PID 追踪的 drift 必然拉锯，且会在开机时对不存在的 PID 空转 |
| `chrt` / 实时调度策略 | 与 EAS/HAL 的调频调核冲突，误设会饿死 UI 线程；需求弱，v1 不做 |
| 按 UID 聚合的 PSS 排行 / 系统-用户拆分 | 已有 `mem apps` / `mem breakdown`，重复实现只会产生两套口径 |
| 迁移 `cpu.cpuset.*`（进程组核心绑定） | 归属 CPU 模块，已有 schema / persist / drift / 回滚；M4 只做"把某个包临时放进现有组"的实验动作，不动组本身 |
| 杀内核线程 / PID 1 / init / `system_server` | 保护名单硬拒绝 |
| 可配置保护名单 | v1 硬编码；开放配置需要先有"白名单误配导致不可用"的恢复设计 |
| zRAM 重配（内存模块 Step 9） | 重估结论继续暂缓：`swapoff` 风险高、收益与进程模块无关（见待定问题 #10） |
| 跨用户的批量强停 / 全量清理 | AMS 的 per-user 语义复杂，v1 只按目标所属用户操作，`--user` 显式覆盖 |

## 已定决策

| 项 | 决定 | 依据 |
|---|---|---|
| 模块 id | `process` | 与 `docs/development-plan.md` §7.5 的 `process get` 一致；`proc` 易与 `/proc` 混淆 |
| 代码路径 | `core/sysflow-modules/src/process/`；CLI verb 文件 `core/sysflow-cli/src/process*.rs` | 沿用 mem 的"模块目录 + CLI verb 独立文件"结构 |
| 进 `snapshot` | **否**（`in_snapshot() == false`） | `Module` trait 的注释已写明"the process module opts out: it is slow"；APK 进页面再调用 |
| schema | 空 items；`probe()` 只报告 `supported` + `features` | 没有可调项，列表不是 item；`caps` 是能力的唯一入口 |
| 持久化 | 无（`apply` 拒绝一切；`drift` / `sanitize_boot` 保持默认） | 见〈不在范围〉；动作走 verb，失败即返回，不写 `config.json` |
| daemon 参与 | 无 `sample()` / `sample_interval()` | 不做进程历史 |
| CLI 形态 | `sysflow process list\|get\|stop\|oom\|renice\|cpuset\|freeze\|unfreeze\|probe`（末项隐藏） | 与 `mem apps`/`wakelock held` 的 verb 风格一致；在 `run_module()` 的 `external_subcommand` 分支添加 |
| 列表默认 | `--limit 50`（上限 200）、按 CPU 降序、包含系统进程 | 与用户"谁在占 CPU"的第一问题对齐；limit 上限防止一行几百个包拖慢 UI |
| CPU% 口径 | 单核百分比（0–100 × 逻辑核数），可 >100 | 与 `top` 一致；"占整机百分比"会让多核设备上所有值都显得很小 |
| 采样窗口 | 默认 500 ms，范围 200–2000 ms（`--sample-ms`） | 500 ms 对 2 s 轮询足够；窗口可调是给真机上"太慢/太抖"留的旋钮 |
| 时钟 | `CLK_TCK` 取 `/proc/self/auxv` 的 `AT_CLKTCK`(17)，回退 100 | 真机 `sysconf(2)` 返回 2048、`sysconf(30)` 返回 1：bionic 的常量编号与 glibc 不同，不能沿用 `sysconf` |
| renice 实现 | `setpriority(2)` 设绝对值；`renice` 只作回退，且 `-n` 按"目标 − 当前"传增量 | 真机 toybox 0.8.13 的 `renice -n` 是增量，直接传目标值会得到错误结果 |
| freezer 机制 | 先试 `/sys/fs/cgroup/apps/uid_<uid>/pid_<pid>/cgroup.freeze`，再试 `am freeze` / `am unfreeze` | 真机 v2 统一路径是 `/apps/uid_…/pid_…`，且 `am help` 列出 freeze |
| cpuset 组发现 | 只把 `/dev/cpuset/<dir>/tasks` 存在且为**目录**的项当组 | 真机 `/dev/cpuset` 顶层同时有 `cpus` / `tasks` / `cgroup.procs` 等普通文件 |
| 进程身份 | `stat` 的 `starttime`（字段 22）用于跨两次采样识别 PID 复用 | 只比 PID 会在快速重启时把别的进程的 CPU 算进来 |
| RSS | `/proc/<pid>/status` 的 `VmRSS`（kB） | 与 `mem apps` 同源；`statm` 给页数，还要再取页大小，多一步且易错 |
| UID | `/proc/<pid>/status` 的 `Uid:` 行 | 目录 owner 在假根测试树里不可控；`status` 对 fixture 友好，且一次读顺带拿到 RSS |
| 名称 | `cmdline[0]` → `status` 的 `Name:` → `pid N` | 应用进程的 `cmdline[0]` 就是包名；内核线程没有 cmdline，回退 comm |
| 前台识别 | `dumpsys activity activities` 的 `topResumedActivity` → `mResumedActivity`，3 s 超时，结果缓存 5 s | 单一来源、跨版本两种行格式；失败时 `foreground.available=false`，不让整个列表失败 |
| 包名映射 | 复用 `batterystats::read_package_uids()`（`/data/system/packages.list` 优先，`cmd package` 回退），单独 10 min 缓存 | 与 `mem apps` 相同的两级缓存，避免每次打开页面触发 binder 调用 |
| 保护名单 | `uid < 10000` + 包名 denylist + 内核线程；动作前**重新校验** | 见〈保护名单〉；列表行只做展示标记，动作时以最新 `status` 为准 |
| 错误码 | 受保护/目标不存在/参数非法 → `invalid_arg`；`/proc` 整体不可读 → `unsupported`；读失败 → `io` | 复用协议 §2 的固定错误码，`detail` 携带 `protected` / `reason` |
| 审计 | v1 **不发**事件（待定问题 #2 若拍板则在 Step 3 补） | 动作是同步的，返回体已含 before/after；事件留给后台发现的问题 |

## 命令面（verb 草案）

> 所有输出走统一信封；下面只画 `data`。字段在 Step 1 探针后可能增删，但**语义**（单核百分比、
> 保护拒绝、不持久化）不再变。JSON 中的 `null` 表示"该进程没有这个属性"（如内核线程的包名）。

### `sysflow process list`

```sh
sysflow process list [--limit N] [--sort cpu|rss|name|pid] [--filter all|user|system]
                     [--query TEXT] [--sample-ms 500] [--no-foreground]
```

```json
{"v":1,"ok":true,"data":{
  "available":true,"generated_at":1795605540,"sample_ms":500,"clock_ticks":100,
  "total":742,"returned":2,"truncated":true,"skipped":2,
  "foreground":{"available":true,"source":"dumpsys","package":"com.android.settings","pid":8123},
  "processes":[
    {"pid":8123,"ppid":701,"uid":10123,"user":0,"app_id":10123,
     "package":"com.android.settings","name":"com.android.settings",
     "cmdline":"com.android.settings","state":"S","threads":42,
     "rss_kb":182344,"cpu_pct":12.5,"cpu_ticks":63,
     "oom_score_adj":0,"cgroup":"/uid_10123/pid_8123",
     "foreground":true,"system":false,"protected":false},
    {"pid":612,"ppid":2,"uid":0,"user":0,"app_id":0,
     "package":null,"name":"kworker/u16:3","cmdline":"",
     "state":"I","threads":1,"rss_kb":0,"cpu_pct":0.0,"cpu_ticks":0,
     "oom_score_adj":0,"cgroup":"/",
     "foreground":false,"system":true,"protected":true}
  ]
}}
```

- **两遍扫描**：第一遍枚举 `/proc/<pid>/` 并读 `stat` / `status` / `cmdline` / `oom_score_adj` /
  `cgroup`；按 `--sample-ms` 等待后，第二遍**只重读 `stat`** 计算差分。PID 在第二遍消失、或
  `starttime` 变化（PID 复用）时，`cpu_pct` / `cpu_ticks` 为 `null`。
- `cpu_pct` 保留一位小数；`cpu_ticks` 是窗口内的时钟滴答增量，客户端可自行按窗口重算。
- `foreground` 只标记**前台包**（该包的进程都标 `true`）；顶层 `foreground.pid` 指 resumed activity
  所属进程。`--no-foreground` 跳过 dumpsys，`foreground.available` 为 `false` 且 `source:"none"`。
- `--filter system` = `uid < 10000`（含内核线程）；`user` = 其余；`query` 对包名 / 名称做大小写
  不敏感的子串匹配。
- 排序在截断**之前**做：`cpu`（降序，缺 `cpu_pct` 的排最后）、`rss`（降序）、`name`（升序、
  大小写不敏感）、`pid`（升序）。`returned` 是实际条数，`truncated` 表示还有更多。
- **不可读的进程计入 `skipped`**，不报错；整个 `/proc` 不可读时 `available:false` + `reason`。
- 结果**不缓存**（列表就是要新鲜）；前台识别单独缓存 5 s。

### `sysflow process get <pid>`

```json
{"v":1,"ok":true,"data":{
  "available":true,"pid":8123,"ppid":701,"uid":10123,"user":0,"app_id":10123,
  "package":"com.android.settings","packages":["com.android.settings"],
  "name":"com.android.settings","cmdline":["com.android.settings"],
  "state":"S","threads":42,"rss_kb":182344,"pss_kb":96400,"swap_pss_kb":0,
  "cpu_pct":12.5,"cpu_ticks":63,"sample_ms":500,
  "oom_score_adj":0,"nice":0,"rt_priority":0,"policy":0,
  "cgroup":"/uid_10123/pid_8123","cpuset":"top-app",
  "start_time_ticks":123456789,
  "foreground":true,"system":false,"protected":false
}}
```

- `get` **只接受 PID**；按包名查进程先走 `list --query <pkg>`，避免"一个包多个进程时返回哪个"的歧义。
- 额外的单进程读取：`smaps_rollup` 的 `Pss` / `SwapPss`（失败则省略，并保留 `rss_kb`）、
  `/proc/<pid>/stat` 的 `nice` / `rt_priority` / `policy`、`/proc/<pid>/cgroup` 的 cpuset 组。
- 进程在读取过程中退出时返回 `invalid_arg`（`detail.gone:true`），而不是半张表。

### `sysflow process stop <pid|package> [--user N]`

```json
{"v":1,"ok":true,"data":{
  "package":"com.example.app","user":0,
  "pids_before":[23123,23124],"pids_after":[],"verified":true,"already_stopped":false
}}
```

- 包目标执行 `am force-stop --user <user> <package>`（`Command::args`，不经 shell，包名先做
  `[A-Za-z0-9._]+` 校验）。PID 目标先解析其包名，再按**包**停止（`am` 没有按 PID 强停的稳定接口）。
- 执行后最多轮询 2 s 确认 `pids_after` 为空；`verified:false` 表示"命令返回成功但进程仍在"。
- 包没有任何进程时**不调用 `am`**，返回 `already_stopped:true`（不产生"清理 job/alarm"的副作用，
  也避免把已停的包再拉一遍 AMS）。
- 多用户：默认用解析到的进程所属用户；无进程且显式给了 `--user` 时按该用户操作。

### `sysflow process oom <pid|package> <adjustment>`

```json
{"v":1,"ok":true,"data":{
  "target":"com.example.app","adjustment":-1000,
  "targets":[{"pid":23123,"before":0,"applied":-1000,"verified":true}]
}}
```

- 范围 `-1000..=1000`；写 `/proc/<pid>/oom_score_adj`，**逐个 PID 读回**；任何一项不一致则
  `verified:false` 并在该行列出 `error`。
- 包目标展开到该包全部进程；**任一 PID 受保护则整个请求拒绝**（不做"部分成功"）。
- 这是 high 风险动作：APK 每次二次确认（实验区开关打开后才可见）；CLI 不额外确认（CLI 本身就是
  显式操作）。不持久化。

### `sysflow process renice <pid|package> <nice>`

```json
{"v":1,"ok":true,"data":{
  "target":"com.example.app","nice":-5,
  "targets":[{"pid":23123,"before":0,"applied":-5,"verified":true}]
}}
```

- 范围 `-20..=19`；优先 `setpriority(2)` 写绝对值，失败时回退 `renice -n (目标 − 当前) -p <pid>`
  （toybox 的 `-n` 是增量）；读回 `/proc/<pid>/stat` 的 nice 字段。
- 包目标同上：展开 + 全量保护检查。`setpriority` 与 `renice` 都不可用时 verb 返回 `unsupported`。

### `sysflow process cpuset <pid|package> <group>`（实验）

```json
{"v":1,"ok":true,"data":{
  "group":"restricted","persisted":false,
  "targets":[{"pid":23123,"before":"top-app","applied":"restricted","verified":true}],
  "note":"ActivityManager may move the process back on its next state change"
}}
```

- 仅当 `/dev/cpuset/<group>/tasks` 存在时提供（可写性在写入前校验，`probe()` 不做 SELinux 断言）；目标组必须是已有组（不创建、不删组）。
- 把该包每个 PID 写入组 `tasks`；读回 `before`/`applied` 用 `/proc/<pid>/cgroup` 的 cpuset 行。
- 一次性、不持久化；AMS 的下一次状态变化可能覆盖，返回体与 UI 都明确说明这一点。

### `sysflow process freeze <pid|package>` / `sysflow process unfreeze <pid|package>`（实验）

```json
{"v":1,"ok":true,"data":{
  "frozen":true,"mechanism":"cgroup-v2",
  "targets":[{"pid":23123,"before":0,"applied":1,"verified":true}]
}}
```

- 机制在 Step 1 探测：优先 cgroup v2 的 `/sys/fs/cgroup/uid_<uid>/pid_<pid>/cgroup.freeze`
  （写 `1` / `0`），其次 `am freeze` / `am unfreeze`（存在才用）。两者都没有则整个动作隐藏。
- high 风险、默认隐藏（实验区）；不持久化，设备重启自然解冻。

### `sysflow process probe`（隐藏）

与 `mem probe` 同定位：输出原始读数与判定，不进协议。至少包含：`/proc` 可读进程数 / 总数、
`CLK_TCK`、页大小、`hidepid`（若可判定）、`dumpsys activity` 原始前台行与解析结果、
`/data/system/packages.list` 可读性（行数）、`/dev/cpuset` 组列表、cgroup v2 freezer 路径是否存在、
`am` / `renice` / `chrt` 的存在性与版本行、`getenforce`、典型进程的 `stat` / `status` 原始字段。
Step 1 的产出物；`docs/modules/process.md` 的矩阵据此回填。

### 保护与错误

```json
{"v":1,"ok":false,"error":{
  "code":"invalid_arg","msg":"uid 1000 (system) is protected",
  "detail":{"protected":true,"reason":"uid < 10000"}
}}
```

- 受保护判定顺序：PID 1/2 → 内核线程 → `uid < 10000` → 包名 denylist → 自身进程（`sysflow` 守护）。
- 列表行照常返回受保护进程（`protected:true`），只是动作拒绝；UI 对动作按钮置灰并显示原因。
- 目标不存在 / 已退出：`invalid_arg`，`detail.gone:true`；参数越界：`invalid_arg`，`detail.range:[min,max]`。

## 字段与口径

### CPU%

- 字段：`/proc/<pid>/stat` 的 `utime`(14) + `stime`(15)，单位是**时钟滴答**；差分 ÷ (`dt` × `CLK_TCK`) × 100。
- `stat` 的 `comm` 在括号内，**可能包含空格和 `)`**；解析从**最后一个 `)`** 之后按空格切分，
  不能按固定列号或第一个 `)` 切。这是最常见的 `/proc` 解析 bug，单测必须覆盖。
- `cpu_pct` 是单核百分比：4 核全忙的进程可以是 400 %。文档、UI、协议三处统一用"单核 %"。
- 第二遍读取只发生在窗口结束时；窗口内新出现的进程（第一遍没有）不参与排序，避免两次集合不一致。

### 进程身份

- 一次采样内用 `(pid, starttime)` 标识进程；跨相机（APK 轮询）只信任 `pid` + `starttime`。
- `state` 取 `stat` 的状态字母（`R/S/D/Z/T/I`…），僵尸进程保留在列表里（`Z`），CPU 为 0。
- 内核线程：`cmdline` 为空、`uid=0`、`ppid=2`；`package:null`，`protected:true`。

### UID 与多用户

- `uid` 是真实 UID；`app_id = uid % 100000`；`user = uid / 100000`（工作资料 / 安全文件夹）。
- 包名映射按 `app_id` 查（与 `mem apps` 相同的规则），标签形如 `包名 (user 10)`。
- 同名包在不同用户下各自有独立进程与 UID；动作默认只作用于解析到的那一个用户。

### 前台应用

- 解析 `dumpsys activity activities`：优先 `topResumedActivity`，否则 `mResumedActivity`；
  取 `ActivityRecord{… u<user> <package>/<activity> …}` 中的 `<package>`。
- 行格式在 Android 版本间有差异（`topResumedActivity=` vs `mResumedActivity:`、是否带 `u0`），
  解析器对两者都容错；两行都没有则 `available:false`，列表仍可用。
- 结果缓存 5 s（2 s 轮询下每 2–3 次列表调用才 dumpsys 一次）。

### 内存

- 列表用 `VmRSS`（与 `mem apps` 的两阶段排序同源），详情才读 `smaps_rollup` 的 `Pss`/`SwapPss`。
- `rss_kb` 可能为 0（内核线程、刚 fork）；缺失时省略而不是写 0（只对 `VmRSS` 行不存在的情况）。

### cgroup / cpuset

- `/proc/<pid>/cgroup` 在 cgroup v2 上是一行 `0::/uid_10123/pid_8123`；v1 上有多行控制器。
- 展示用**统一后的路径**（v2 取 `0::` 行；v1 取 `cpuset` 行），原始多行保留在 `probe` 里。
- cpuset 组名从 cpuset 行末段取（`/top-app` → `top-app`）；读不到则省略。

## 数据来源与降级

| 数据 | 来源 | 缺失 / 不可用时 |
|---|---|---|
| 进程列表 | `list_dir("/proc")` 后逐个 `read_dir` 数字项 | 整个 `/proc` 不可读 → `available:false` + `reason`；单进程消失 → `skipped++` |
| 状态 / PPID / 线程 / CPU 滴答 / starttime | `/proc/<pid>/stat` | 该进程跳过（`skipped++`） |
| UID / RSS / comm | `/proc/<pid>/status` | 该进程跳过；`VmRSS` 行缺失时省略 `rss_kb` |
| 名称 | `/proc/<pid>/cmdline`（NUL 分隔） | 回退 `status` 的 `Name:`，再回退 `pid N` |
| oom_score_adj | `/proc/<pid>/oom_score_adj` | 省略该字段（不猜） |
| cgroup | `/proc/<pid>/cgroup` | 省略 `cgroup` / `cpuset` |
| 包名 | `/data/system/packages.list` → `cmd package list packages -U`（8 s 超时） | `package:null`，标签回退 AID / `UID <n>`；映射缓存 10 min |
| 前台应用 | `dumpsys activity activities`（3 s 超时） | `foreground.available:false`、`source:"none"`；列表正常返回 |
| PSS / SwapPss | `/proc/<pid>/smaps_rollup` | 省略 `pss_kb` / `swap_pss_kb`，保留 `rss_kb` |
| nice / policy | `/proc/<pid>/stat` | 省略 |
| cpuset 写 | `/dev/cpuset/<group>/tasks` | 无该组 → 不报 `cpuset`；写入被拒（SELinux/perm）→ 该动作 `io`/`permission`，不部分执行 |
| freezer | `/sys/fs/cgroup/apps/uid_<uid>/pid_<pid>/cgroup.freeze`，回退 `am freeze` / `am unfreeze` | 两者都不可用 → 不报 `freeze` |
| renice | 主 `setpriority(2)`；回退 `renice -n (目标−当前) -p` | 命令不存在且系统调用失败 → 动作报 `permission` / `io`；机制存在仍报 `renice` |
| force-stop | `am force-stop --user` | `am` 不可用（极罕见）→ `unsupported` |

## 动作契约

所有动作共用同一条路径，顺序不可交换：

1. **解析目标**：PID → 读 `stat` + `status`；包名 → 枚举该包全部 PID。目标为空/已退出 → `invalid_arg`。
2. **保护检查**：PID 1/2、内核线程、`uid < 10000`、包名 denylist、自身进程；**展开后的任一 PID
   命中即整体拒绝**，不写任何东西。保护检查读的是**当前** `status`，不是列表快照。
3. **身份复核**：动作前重读 `starttime` 与 UID，确保不是 PID 复用后的别的进程。
4. **执行**：`oom`/`freeze` 直接写文件（经 `Sysfs::write_str`）；`stop`/`renice` 走
   `Command::args`（不经 shell）；`cpuset` 写 `tasks`。
5. **读回**：`oom_score_adj` / nice / freeze / cpuset 读回实际值；`stop` 轮询 2 s 确认进程消失。
6. **返回**：`before` / `applied` / `verified`；不一致时 `verified:false`，**不回滚**（一次性动作，
   回滚反而会再写一次），但错误里说明实际值。
7. **日志**：每次动作写一行 `sysflow` 日志（目标、before/after、耗时），与 CPU 的 apply 日志同格式。

**误操作与恢复**：

- `stop`：应用下次启动即恢复；无持久化损坏。UI 明确提示"未保存的数据可能丢失"。
- `oom` / `nice`：进程重启后回到 AMS 决定的值；不需要恢复入口。UI 显示 before/after。
- `cpuset` / `freeze`：解冻 / AMS 下一次状态变化即恢复；freeze 提供 `unfreeze` 兄弟命令。
- 任何动作失败都不会写 `config.json`，因此不会影响开机、drift 或安全模式。

## 保护名单

| 规则 | 判定 | 原因文案 |
|---|---|---|
| init / kthreadd | `pid <= 2` | `pid ≤ 2 is the kernel` |
| 内核线程 | `cmdline` 为空且 `uid == 0` | `kernel thread` |
| 平台进程 | `uid < 10000` | `uid < 10000 (platform)` |
| 包名 denylist | 见下表 | `system critical package` |
| 自身 | 进程名为 `sysflow` 或 UID 属于本模块 | `sysflow itself` |

包名 denylist（v1 硬编码，按"停掉会直接破坏可用性"筛选）：

| 包 | 停掉的后果 |
|---|---|
| `com.android.systemui` | 状态栏 / 导航 / 通知全消失，设备基本不可操作 |
| `com.android.phone` | 通话 / 移动网络（通常已被 uid 1001 覆盖，双保险） |
| `com.android.server.telecom` | 来电无法接通 |
| `com.android.bluetooth` | 蓝牙栈（uid 1002 通常已覆盖） |
| `com.android.nfc` | NFC（uid 1027 通常已覆盖） |
| `android` | `system_server` 的包名（uid 1000 已覆盖，双保险） |

> 其余系统应用（设置、启动器、GMS、输入法）**不**在 denylist：停掉后系统会自动重启它们，
> 属于可恢复操作；APK 在 medium 风险动作前统一二次确认。

## 性能预算

| 路径 | 目标 | 说明 |
|---|---|---|
| 扫描 1198 个进程（第一遍） | < 60 ms | 每进程 4 次小文件读；真机实测 28 ms；假根 800 进程列入测试 |
| 第二遍（仅 `stat`） | < 25 ms | 800 次读 |
| dumpsys 前台 | < 400 ms | 3 s 超时；5 s 缓存 |
| `list` 冷路径（默认参数） | < 600 ms | 500 ms 窗口 + 扫描 + 前台（缓存命中时 < 250 ms） |
| `list --no-foreground` | < 250 ms | 给"只要排序"的调用方 |
| `get`（单进程） | < 50 ms | 含一次 `smaps_rollup` |
| `probe()`（`caps` 调用） | < 5 ms | 只查文件存在性，不扫 `/proc` |

APK 每 2 s 轮询时使用 `--sample-ms 300 --limit 80`（页面滚动到底再加载更多），离开页面立即停止。

## 交付顺序

原则：**契约冻结 → 真机探针 → 只读 → 动作 → 实验动作 → UI → 验收**。动作放在只读之后，
因为保护名单、目标解析和前台/包名映射都依赖只读面的实现。

| # | 步骤 | 产出 | 验收 | 依赖 | 粗估 |
|---|---|---|---|---|---|
| 0 | 契约冻结 | 本文件 + 协议 §3.25 草案 + `docs/modules/process.md` 骨架 + `development-plan.md` §7.5 / §12 状态更新 | 评审通过 id / verb / 保护名单 / 字段口径 | — | 0.5 d |
| 1 | 真机探针 | 隐藏 `process probe`（**已在主机实现并通过测试，真机待跑**）；回填路径矩阵：`/proc` 可读性、`CLK_TCK`、dumpsys 前台行格式、包名映射、`/dev/cpuset` 组、freezer 路径、`am`/`renice` 可用性、800 进程计时 | 矩阵中每个"待验证"都有结论；据此裁剪实验动作 | 0 | 1 d |
| 2 | 只读面 | `ProcessModule` 的 `probe` / `caps features` / `list` / `get`；前台识别；保护分类；`in_snapshot=false` | ✅ 主机完成：`model` / `scan` / `foreground` / `packages` + 35 项 process 测试；假根 `list`/`get` 通过；真机排序对拍待跑 | 1 | 2 d |
| 3 | 动作（medium） | `stop` / `renice` / `oom` + 保护全量拒绝 + 读回 + 日志 | ✅ 主机完成：保护/范围/读回假根测试通过；`renice` 走 `setpriority(2)`、toybox 仅回退；真机写路径待验 | 2 | 2 d |
| 4 | 实验动作 | `cpuset` / `freeze` / `unfreeze`，按 Step 1 探测结果决定保留哪些；feature flag 控制 | ✅ 主机完成：cgroup v2 `apps/` 叶子 + `am` 回退两条路径；真机 freezer 叶子与写入待验 | 3 | 1.5 d |
| 5 | APK 进程页 | `feature/process/`（Models / ViewModel / Screen）+ 导航 / Koin 接线 + `SysFlowClient` 动词 + 确认弹窗 | ✅ 主机代码完成（模型单测 + 解析；页面仅进入时 2 s 轮询、保护行降级、实验区默认关闭）；APK 构建由用户执行 | 2、3 | 2.5 d |
| 6 | 真机验收与文档回填 | 平滑度、排序对拍、保护演练、恢复演练；`docs/modules/process.md` 实测值 / 降级原因 / 计时；README 增补 | 成功标准 1–6 逐条通过 | 4、5 | 1 d |

合计约 10.5 d（与 `development-plan.md` 阶段 4 的 2–3 周一致，含真机往返）。
Step 5 的 Models / Screen 骨架可在 Step 2 后提前写（只依赖协议形状，不依赖动作）。

**每步完成定义**：`cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` +
`cargo test --all`（假根用 `SYSFLOW_SYSFS_ROOT`、数据目录用 `SYSFLOW_DATA_DIR`）+ 真机抽样输出 +
更新 `docs/modules/process.md`。CI 当前基线：334 项主机测试通过（1 项 ignored）。

## 测试计划

**Fixture 树**（`SYSFLOW_SYSFS_ROOT` 下）：

```
proc/<pid>/stat         # 含 comm 带空格与 ')'、Z 状态、内核线程、高 CPU 滴答
proc/<pid>/status       # Uid: 四段、VmRSS、Name:、Threads:
proc/<pid>/cmdline      # NUL 分隔；应用包名 / 空（内核线程）
proc/<pid>/oom_score_adj
proc/<pid>/cgroup       # v2 单行 / v1 多控制器两种
dev/cpuset/<group>/tasks
sys/fs/cgroup/uid_<uid>/pid_<pid>/cgroup.freeze
data/system/packages.list
```

| 场景 | 覆盖点 |
|---|---|
| 正常设备 | 应用 + 系统进程 + 内核线程混合；排序 / 过滤 / 截断 |
| comm 含空格 / `)` | `(my proc))` 之类；从最后一个 `)` 解析 |
| 僵尸 / 已退出 | `Z` 保留；读取中途消失 → `skipped++` |
| PID 复用 | 两遍之间 `starttime` 变化 → `cpu_pct:null` |
| 多用户 UID | `uid = 10 10123` → `user:10`、`app_id:10123`、包名标签带用户 |
| `/proc` 整体不可读 | `available:false` + `reason`，命令不报 io 崩溃 |
| 单进程权限拒绝 | 计入 `skipped`，其余正常返回 |
| 包名缺失 | `package:null`，回退 AID / `UID <n>` |
| dumpsys 变体 | `topResumedActivity=` / `mResumedActivity:` / 无匹配 / 超时 |
| 保护名单 | 每条规则的拒绝；包展开中单个 PID 受保护 → 整体拒绝 |
| oom / freeze | 假文件写后读回；越界 / 受保护拒绝；包展开 |
| renice / stop | 参数校验与保护拒绝（命令执行留在真机验收） |
| CPU% 纯函数 | `(delta_ticks, dt, hz)` → 百分比；dt=0 / 负增量（计数器回绕） |

**性能用例**：假根生成 800 个进程，记录 `list` 扫描与 `get` 耗时到 `docs/modules/process.md`；
回归时对比。**真机对拍**：`list --sort rss` 与 `dumpsys meminfo` 的 app 顺序、`list --sort cpu`
与 `top -b -n 1` 的前几名顺序。

## APK 页面状态

| 状态 | 条件 | 页面表现 |
|---|---|---|
| 正常 | 列表可用 | 搜索框 + 排序 chips + 系统/用户过滤 + 行（名称、UID、CPU%、RSS、OOA、前台标记） |
| 无前台信息 | `foreground.available:false` | 列表正常；隐藏"前台"高亮，不显示错误 |
| `/proc` 不可读 | `available:false` | 整页空态 + `reason` + 重试按钮 |
| 进程已退出 | `get` 返回 `gone` | Snackbar + 回到列表 |
| 受保护 | 行 `protected:true` | 动作按钮置灰 + "系统进程不可操作"说明 |
| 动作失败 | `verified:false` / 错误码 | 显示 before/applied 与原因；不假装成功 |
| 实验动作不可用 | `caps.features` 无对应项 | 隐藏按钮，不显示禁用项（避免"为什么不能用"） |
| daemon 未运行 | — | 进程页不依赖 daemon（不读 history），不显示相关提示 |

交互：进入页面立即采样并每 2 s 轮询（`repeatOnLifecycle(STARTED)`），离开即停；下拉刷新用
`--sample-ms 800` 取更稳的值；详情页按 PID 调 `get`；`stop` / `renice` / `oom` 弹确认框
（复用设置页 `ConfirmDialog`，抽到 `core/ui/components` 供两处使用）；`oom` / `freeze` 在
"实验功能"开启后才出现。

## 风险登记

| 风险 | 影响 | 缓解 |
|---|---|---|
| `/proc` 解析踩 comm 括号 | CPU 统计错乱 | 从最后一个 `)` 解析；专门 fixture |
| 2 s 轮询 + 500 ms 采样耗电 | 页面停留时额外耗电 | 窗口降到 300 ms；离开即停；前台识别 5 s 缓存；不做后台轮询 |
| PM 映射触发 binder 卡顿 | 首屏慢 | `packages.list` 优先；UID 映射 10 min 缓存；仅未知 UID 才回退 `cmd package` |
| `dumpsys activity` 版本差异 | 前台识别失败 | 两种行格式 + 无匹配降级；不影响列表 |
| AMS 重置 oom / nice / cpuset | 动作"看起来没生效" | 一次性动作 + 明确说明；UI 显示 `verified` 与"下次状态变化可能重置" |
| 误杀关键进程 | 设备不可用 | 硬保护名单 + 包展开整体拒绝 + medium/high 确认 |
| PID 复用竞态 | 对错误进程执行 | 动作前重读 `starttime` + UID；不一致即拒绝 |
| `renice` / `am` 不存在或行为差异 | 动作失败 | Step 1 探测；`caps.features` 不报即隐藏；返回 `unsupported` |
| cgroup 布局厂商差异 | 实验动作不可用 | 只做探测到才出现的功能；v1 不承诺 |

**回滚**：Step 0–2 只读，回滚 = 从 `builtin()` 移除一行注册；Step 3–4 的动作无一写
`config.json`，回滚 = 移除 verb 分支 + 模块注册，已执行的 `stop`/`oom`/`freeze` 由系统自行恢复。

## 待定问题（含推荐默认值）

> 与内存模块的规矩一致：每项给推荐，拍板后 Step 1 即可按此实现；未拍板不影响只读面。

1. **是否持久化 oom / nice / cpuset？** **推荐：不持久化**。AMS 随进程状态重置，持久化必然产生
   "开机对不存在 PID 空转 + drift 假报"；真有需求时另立 M4b，以"包级意图 + 进程出现时应用"的设计
   单独评审。
2. **动作是否写审计事件？** **推荐：Step 3 写一条** `process_action`（category `action`、severity
   `info`、metadata `{action,target,before,after,verified}`）。成本低，能给"谁在什么时候停了什么"留痕；
   若嫌噪音，也可只对 `stop` 与 `oom` 发。
3. **列表默认 `limit` / 排序？** **推荐：50 / CPU 降序**；200 上限。APK 首屏 80 条 + 滚动加载。
4. **内核线程默认显示？** **推荐：显示**（它们是系统进程的一部分，且按 CPU 排序时通常不在前列）；
   `--filter user` 可一键只看应用。额外加 `--kernel` 过滤会多一个几乎没人用的开关。
5. **`chrt` 是否进 v1？** **推荐：不进**。与 EAS/HAL 冲突，收益低；真机验证成本高。
6. **应用级 cpuset / freezer 的默认可见性？** **推荐：实验区、默认隐藏**，且仅在 Step 1 探测到
   机制时注册；没有探测结果就不写实现。
7. **保护名单是否可配置？** **推荐：v1 不可配置**；若确需，M4b 以 `process.protected`（文本列表）
   走标准 `set --persist`，但要先设计"全部放行导致 SystemUI 被停"的恢复路径。
8. **`stop` 的目标是包还是 PID？** **推荐：对外只承诺包**；PID 输入先解析成包，再按包强停
   （`am` 无稳定按 PID 接口）。这与用户心智"停掉这个应用"一致。
9. **列表是否需要 60 s 缓存？** **推荐：不缓存**。列表是"现在"的快照，缓存会让 2 s 轮询看到旧数据；
   只缓存前台识别与包名映射。
10. **内存 Step 9（zRAM 重配）是否随 M4 一起做？** **推荐：继续暂缓**。它属于写入面高风险改动，
    与进程模块无依赖；等 M4 验收后单独评估，若做则单独立项（swapoff 会当场影响系统）。
11. **CPU% 是否提供"整机百分比"？** **推荐：不提供**。单核口径与 `top` 一致；整机百分比在
    大小核设备上会产生"同一负载、不同数值"的困惑，客户端要时自行除以核数。
12. **`process get` 是否支持包名？** **推荐：只支持 PID**；包名先用 `list --query` 定位，避免
    "多进程包返回哪一个"的歧义。

## 参考实现

- 只读扫描、跳过与缓存：`core/sysflow-cli/src/mem_apps.rs`（`/proc` 枚举、`skipped`、60 s 缓存）。
- UID → 包名：`core/sysflow-core/src/batterystats.rs::read_package_uids`（`packages.list` 优先、
  `cmd package` 回退、8 s 超时、`uid % 100000` 匹配）。
- `dumpsys` 调用与超时：`core/sysflow-core/src/dumpsys.rs`（`run_with_timeout`）。
- 命令执行封装：`core/sysflow-core/src/command.rs`（子进程 watchdog；`stop` / `renice` 复用）。
- verb 路由：`core/sysflow-cli/src/main.rs` 的 `run_module()` + `external_subcommand` 分支
  （`"apps" if module == "mem"` 同款写法）。
- cpuset 组读写与格式：`core/sysflow-modules/src/cpu/cpuset.rs`（组发现、`parse_cpu_list`、
  `tasks` 写法的边界）。
- 模块注册：`core/sysflow-modules/src/lib.rs::builtin()`。
- 缓存：`core/sysflow-core/src/cache.rs`（`read_fresh` / `write` / TTL）。
- 事件（若采用待定 #2）：`core/sysflow-core/src/event.rs` 的 `NewSystemEvent`，
  参考 `core/sysflow-modules/src/wakelock/mod.rs` 的 `watchdog`。
- APK：`app/src/main/kotlin/io/sysflow/feature/mem/`（列表 + 详情 + 降级）、
  `core/ui/components/FlexibleSubPage.kt`、`feature/settings/SettingsScreen.kt` 的 `ConfirmDialog`。
- 设备矩阵与探针文档：`docs/modules/mem.md`（路径矩阵 / 实测值 / 降级原因的写法）。
