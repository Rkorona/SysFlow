# 储存模块（M5）开发计划

> 状态：**Step 0–5 主机完成（2026-11-26）**。契约、只读面、可调项、TRIM、历史与 APK 页面代码均已落地；
> 真机探针与验收待跑（`docs/modules/storage.md`〈Device checklist〉）。
>
> 关联：`docs/development-plan.md` §7.6（功能设计）与 §12 阶段 5、`docs/protocol.md` §3.26、
> `docs/modules/storage.md`（路径矩阵 / 实测值 / 降级原因）、`docs/mem-module-plan.md`（模板）。

## 目标

- 回答「还剩多少空间、被谁占了、写得有多快、盘还能写多久」。
- 提供两个低风险、可回滚的队列调节：`storage.scheduler` 与 `storage.read_ahead_kb`，走与 CPU / 内存
  相同的 `apply / drift / sanitize_boot / persist` 路径。
- 提供一次性动作 `storage trim`，不持久化、不参与 drift、不在开机回放。
- 全部采集在 Root 模块；APK 只在页面可见时轮询；`snapshot` 只放廉价值（容量、累计计数器）。

**成功标准**（Step 6）：

1. 进入储存页 1 s 内看到容量与 IO；2 s 轮询不卡。
2. 容量与 `df -k /data` 一致（同一时刻）；IO 速率与 `/proc/diskstats` 手算一致。
3. 两个可调项写入后读回一致；ROM 改回时 drift 能发现，重启后按 `sanitize_boot` 正确处理。
4. TRIM 成功返回字节数，失败有明确错误；目录分析在中端机上 ≤ 预算 + 500 ms 返回，超时标 `partial`。
5. 拿不到 `/proc/mounts` / `statvfs` / 队列节点时**隐藏对应区块**而不是报错。

## 不在范围

| 项 | 原因 |
|---|---|
| `io.discard` / 持续 discard | 与 TRIM 重复且对寿命/延迟影响不可控，默认不做 |
| 分区表 / 文件系统创建、resize、fsck | 破坏性，超出「设备管理」边界 |
| 每设备独立 scheduler 项 | v1 只调 `/data` 背后的物理盘；多盘独立项等有需求再说 |
| 目录分析做成常驻/定时任务 | 违背「零常驻」；只在用户点击时跑，带时间预算 |
| UFS 健康度的厂商私有格式 | 只解析 JEDEC `life_time`；其他格式显示「不支持」而不是猜 |
| 写入量按 TBW 估算剩余寿命 | 需要厂商额定值，公开路径不可靠 |

## 已定决策

| 项 | 决定 | 依据 |
|---|---|---|
| 模块 id | `storage` | 与阶段 5 命名一致；`io` 太泛 |
| item 前缀 | `storage.*`，队列项也放在 `storage.*` | 协议 §1 的 `<module>.<name>` 约定 |
| 容量语义 | `used = total − free`；`free` 是 `f_bfree`；`avail` 单独给出 | Android 的「已用」通常含保留块；把 `avail` 单独暴露避免误读 |
| 容量实现 | `statvfs(3)`（`repr(C)` 声明，不引入 libc crate） | 直接、无子进程；64 位布局在 bionic/glibc 一致 |
| 设备归属 | tunable 作用于 `/data` 背后的**物理盘**（dm → slaves） | dm 节点没有 `queue/scheduler` |
| IO 统计 | `/proc/diskstats` 差分，扇区恒按 512 B | 内核文档口径；利用率 = `Δio_ms / Δt`，钳到 100 |
| 速率时间基 | **实测**间隔（`Instant`），不是名义值 | 设备休眠 / 慢调用会拉长间隔 |
| 可调项 | `storage.scheduler`（enum）/ `storage.read_ahead_kb`（int_enum） | 低风险、写后能读回、ROM 可能重置 |
| read_ahead 选项 | 128/256/512/1024/2048/4096，当前值不在表内时补上 | 保证 `get` 的值永远可 `set` |
| TRIM | verb，只允许 `/data`（或 `/storage/emulated/0`） | 一次性动作；限制路径防误操作 |
| 目录分析 | verb，`--path/--limit/--budget-ms`，按 `st_blocks×512` | 空间问题的答案是「分配大小」不是「表观大小」 |
| 目录分析边界 | 不跟随符号链接、不跨文件系统、时间预算 + 硬上限 | `/data/media` 与 bind mount 不能重复计入 |
| 健康度 | 只解析 JEDEC `life_time` / `life_time_estimation`；解析不了就隐藏 | 厂商格式未验证，猜一个百分比比不显示更糟 |
| 历史 | 300 s；`used_kb/free_kb/used_pct` + IO 差分 | 30 天 × 288 = 8 640 行，符合保留上限 |
| 事件 | `storage_low_space`（≥90 %）/ `storage_space_recovered`（≤85 %） | 迟滞 + dedupe，避免阈值抖动刷屏 |
| 持久化范围 | 只有两个队列项；TRIM / 目录分析不落盘 | 与 CPU / 内存的边界一致 |

## 只读项

| id | 单位 | 来源 |
|---|---|---|
| `storage.data.total_kb` / `used_kb` / `free_kb` / `avail_kb` | kB | `statvfs("/data")` |
| `storage.data.used_pct` | % | `used / total`，整数 |
| `storage.sdcard.*` | kB / % | 仅当 `/storage/emulated/0` 与 `/data` 不是同一设备 |
| `storage.device.name` / `size_kb` / `model` / `rotational` | — / kB / — / — | `/sys/block/<dev>/{size,device/model,queue/rotational}` |
| `storage.device.health_pct` | % | `device/life_time` 的 JEDEC 编码（剩余寿命） |
| `storage.io.read_kb` / `write_kb` | kB | `/proc/diskstats` 扇区累计 ÷ 2 |
| `storage.io.read_ios` / `write_ios` | 次 | 同上，完成请求数 |

## Verb

### `storage io [--interval-ms 1000]`

两次读取 `/proc/diskstats`，返回 `read_kb` / `write_kb`（区间量）、`read_iops` / `write_iops`、
`util_pct`、`total_*`（自开机累计）。间隔钳在 300–5000 ms。APK 每 2 s 调一次。

### `storage usage [--path /data] [--limit 20] [--budget-ms 3000]`

顶层条目递归求和、按分配字节排序。**有界**：超时返回 `partial:true` 与 `visited`；跳符号链接、
不跨文件系统；无权限条目计入 `denied`。`limit` 钳 1–100。

### `storage trim [--path /data]`

`fstrim -v <path>` 一次；只接受 `/data` 或 `/storage/emulated/0`；返回 `trimmed_bytes` / `output` /
`ms`；ROM 没有 `fstrim` 时 `unsupported`。不持久化。

### `storage probe`（隐藏）

原始数据：挂载表、`statvfs`、全部块设备（scheduler 选项 / read_ahead / nr_requests / 健康度）、
`/proc/diskstats` 行、`fstrim` 帮助、`df -k /data`。Step 1 的产出物，不进协议。

## 写入面契约

1. **先校验后写**：值必须在 `schema` 的选项内，否则 `invalid_arg` 且一个字节都不写。
2. **写后读回**：`applied` 是内核的答案；`mismatch` 列出被拒项。
3. **失败回滚**：scheduler 写成功、read_ahead 失败时，把 scheduler 写回旧值（反向顺序）。
4. **开机清洗**：`sanitize_boot` 丢掉当前内核不接受的 scheduler / read_ahead，而不是整模块失败。
5. **reset**：恢复 `defaults_snapshot`（首次写前捕获的 ROM 原值），由 Registry 统一执行。
6. **drift**：默认 `collect(keys)` 比对；ROM 反复改回时沿用与内存模块相同的退避策略（当前实现为默认
   drift 路径，未引入模块私有 `contested` 事件）。

## 数据来源与降级

| 数据 | 来源 | 缺失时 |
|---|---|---|
| 挂载表 | `/proc/mounts` | 模块 `unsupported` |
| 容量 | `statvfs("/data")` | `unsupported`（`statvfs` 失败） |
| 块设备 | `/sys/block/<name>` | 只保留容量与 IO，隐藏设备/队列项 |
| scheduler 选项 | `queue/scheduler` | 不报 `scheduler` feature |
| read_ahead | `queue/read_ahead_kb` | 不报 `read_ahead` feature |
| IO | `/proc/diskstats` | 不报 `io` feature，`storage io` 返回 `unsupported` |
| 健康度 | `device/life_time` | 隐藏 `storage.device.health_pct` |
| TRIM | `/system/bin/fstrim` 等 | 不报 `trim` feature，verb `unsupported` |
| 目录分析 | `std::fs` 遍历 | 无权限条目计入 `denied`，超时 `partial` |

## 交付顺序

| # | 步骤 | 产出 | 验收 | 状态 |
|---|---|---|---|---|
| 0 | 契约冻结 | 协议 §3.26 + 本文件 + `docs/modules/storage.md` 骨架 | 评审 id / 单位 / verb / 写入面 | ✅ 2026-11-26 |
| 1 | 真机探针 | 隐藏 `storage probe`；回填挂载/设备/队列/健康度/计时 | 每个「待测」有结论 | ⏳ 待真机 |
| 2 | 只读面 | 容量、设备、IO 累计、健康度进 `snapshot` | 假根测试；`df` 对拍 | ✅ 主机 |
| 3 | 可调项 | scheduler / read_ahead 的 apply / drift / boot / reset | 假根写读回、越界拒绝、回滚 | ✅ 主机 |
| 4 | 历史与事件 | 300 s 差分 + 90/85 % 事件 | 重启首点无负值；事件迟滞 | ✅ 主机 |
| 5 | APK + TRIM + 目录分析 | `feature/storage/`（M3E）+ 三个 verb + 页面 | 构建通过；轮询离开即停；分析可取消 | ✅ 主机 |
| 6 | 真机验收与回填 | 计时、对拍、TRIM 实测、页面验收 | 成功标准 1–5 | ⏳ 待真机 |

## 测试计划

- **容量**：`statvfs` 在真实临时目录上可读（`total > 0`、`avail ≤ free ≤ total`）；挂载表解析含
  `\040` 转义、同一设备不回显 sdcard。
- **设备**：`[mq-deadline] kyber none` 解析；dm → slaves → sda；loop/ram/zram 过滤；life_time
  `0x01 0x02 → 80 %`、`0x0b → 0 %`、未知格式 → `None`。
- **IO**：diskstats 行解析；分区不遮蔽整盘；速率/利用率；计数器回退饱和为 0。
- **可调项**：写读回；越界拒绝不落盘；sanitize 丢弃非法值；回滚路径。
- **历史**：首点无差分；用实测间隔；重启跳过差分；低空间迟滞与持久化。
- **目录分析**：排序与大小；零预算立即 `partial`；文件路径报 `invalid_arg`；limit 钳制。
- **模块**：probe/schema/collect/apply/sanitize；缺失 `/data` 时 `unsupported`。

## APK 页面状态

| 状态 | 条件 | 表现 |
|---|---|---|
| 正常 | 容量 + IO 可用 | 容量 hero、IO 实时卡、24 h 曲线、设备与队列项、TRIM、目录分析 |
| 无 IO | `io` feature 缺失 | 隐藏 IO 卡与曲线，保留容量与队列项 |
| 无队列项 | `scheduler` / `read_ahead` 缺失 | 隐藏对应选择器 |
| 无健康度 | `health_pct` 缺失 | 隐藏健康度行 |
| 分析超时 | `partial:true` | 结果标注「部分结果」+ 已访问条目数 |
| 无权限 | `permission` | 按协议 §2 处理；目录分析显示 `denied` 数 |
| daemon 未运行 | `history` 空 + `daemon_running:false` | 曲线空态并解释 |

交互：进入页面后 2 s 轮询 `snapshot` 与 `storage io`（`--interval-ms 1000`）；离开即停；
scheduler / read_ahead 点选即 `set --persist` 并读回；TRIM 二次确认；目录分析按钮触发后显示
加载与「可随时返回」，结果按大小排序。

## 风险登记

| 风险 | 影响 | 缓解 |
|---|---|---|
| `statvfs` 结构体 ABI 不一致 | 容量读数错乱 | 64 位布局与 bionic/glibc 一致；真机与 `df` 对拍；异常值有测试 |
| `/data` 是 dm 节点 | 写到没有 scheduler 的节点 | 沿 `slaves` 解析物理盘；解析失败则 `unsupported` |
| ROM 改写 scheduler / read_ahead | 设置不稳 | drift 修复 + 开机清洗；一次性动作不承诺持久 |
| 目录分析过慢 | 页面卡住 | 时间预算 + 硬上限 + `partial`；仅点击触发 |
| 健康度格式误判 | 误导寿命 | 仅接受 JEDEC 编码；其他格式隐藏 |
| TRIM 被误用 | 误清其他挂载 | 只允许 `/data` 与 emulated storage |
| `/data/media` 双计 | 目录分析偏大 | 不跨文件系统、不跟随符号链接 |

## 待定问题（含推荐）

1. **是否提供每设备独立的 scheduler 项？** 推荐：v1 不做，只调 `/data` 物理盘；有需求再扩展 `storage.scheduler.<dev>`。
2. **目录分析默认预算？** 推荐：3000 ms；APK 展示「部分结果」而不是把预算拉到 30 s。
3. **是否把目录分析做成可取消的后台任务？** 推荐：v1 用预算+超时（阻塞 ≤3 s），取消/进度需要流式协议，复杂度高。
4. **健康度是否展示为「剩余 %」？** 推荐：是，并标注「厂商 JEDEC 估算」；原始值放 probe。
5. **低空间阈值是否可配？** 推荐：v1 固定 90/85 %；设置项留到阶段 6。
6. **`storage.sdcard` 是否值得单独展示？** 推荐：仅当确为不同设备时展示，否则只显示 `/data` 一张卡。

## 参考实现

- 只读块 + 可调项：`core/sysflow-modules/src/mem/`（detect / tunables / history）。
- enum 选项与 apply 回滚：`core/sysflow-modules/src/cpu/schema.rs`、`cpu/mod.rs`。
- 历史基线：`core/sysflow-modules/src/mem/history.rs`（重启/差分规则）。
- 事件：`core/sysflow-modules/src/mem/history.rs`（`NewSystemEvent` + dedupe）。
- CLI verb 路由：`core/sysflow-cli/src/main.rs` 的 `external_subcommand` 分支。
- APK 页面：`app/src/main/kotlin/io/sysflow/feature/mem/`（hero / 卡片 / 曲线 / 权限降级）。
