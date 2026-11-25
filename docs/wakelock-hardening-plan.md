# 唤醒锁模块硬化计划（P0–P3 缺陷 + 架构整理）

> 状态：**已完成（2026-11-24）**。Phase 0–6 全部落地，Phase 7 的文档与验证已完成；
> 真机回归项（`appops set … default` 兼容性、`dumpsys power` 字段差异、72 小时浸泡）仍待设备验收，
> 未在本地宣称通过。
>
> 结果：`cargo test --workspace` 253 项通过、`cargo clippy --workspace --all-targets` 无告警、
> `:app:testDebugUnitTest` 57 项通过、`:app:assembleDebug` 成功。模块拆分为
> `mod.rs` / `window.rs` / `debugfs.rs` / `appops.rs` / `policy.rs` / `read.rs` / `state.rs` / `power.rs`。
>
> **真机抽样（Samsung）**：设备 `dumpsys power` 输出暴露了两个本机测不到的解析缺陷，已修复并用真机
> 原文补了回归测试——① 归因段是空格分隔、无 `pkg=`、且 `WorkSource{ chains=WorkChain{(10390), (1041)}}`
> 有嵌套括号，旧的 `find(')')` 会把 `pid`/`pkg` 截断；② 该 ROM 打的是
> `mLastWakeUpReason: 19` + `mLastWakeTime=… (802127 ms ago)`，没有 `mLastWakeupUptimeMs`，所以
> `age_ms` 改为优先读取括号里的现成年龄。此外 `held` 现在会用 `cmd package list packages -U` 把只有
> UID 的锁解析出可读 `owner`（共享 UID 不猜包名）。
>
> 仍待真机确认：`appops set … default` 是否被该 ROM 接受、长持锁事件在长时间运行下的噪音水平。
> 关联：`docs/modules/wakelock.md`、`docs/protocol.md` §3.18–§3.20、`docs/development-plan.md` §7.3。

本计划只做一件事：把 2026-11-24 复核中确认的缺陷全部修掉，并把模块拆到可长期维护的形状。
**不含**与电池页弹层合并、总览页唤醒锁卡片；内核唤醒源屏蔽继续保持不做（理由见
`docs/modules/wakelock.md` 首节）。

## 不在范围

| 项 | 原因 |
|---|---|
| 电池页弹层合并 | 纯 UI 取舍，与缺陷无关 |
| 总览页唤醒锁卡片 | 纯展示层，与缺陷无关 |
| 内核唤醒源屏蔽 | 内核无可移植可写接口，做了就是伪功能 |

## Phase 0 — 基线与回滚点

工作区没有版本控制，先造一个可回滚、可审查的基线。

- [x] 快照 `core/`、`app/`、`docs/` 到工作区外或临时目录，结束时用 `diff -ru` 产出完整改动集
- [x] `cargo test --workspace` 基线（当前：全部通过）
- [x] `./gradlew --offline :app:testDebugUnitTest` 基线
- [x] 记录 `cargo clippy` 现有告警，改动不得新增

**验收**：两条基线命令各跑一次并留存输出。

## Phase 1 — P0：拦截结果读回被前端读错

**问题**：`applied` 按 item id 组织（`{"wakelock.blocked": {"pkg": true}}`），
`WakelockViewModel.kt:114` 却按包名取，永远拿到 `null` 后回退成"成功"，ROM 拒绝被静默吞掉。

**做法**：

- [x] Kotlin 侧新增**可单测的纯函数**（放进 `core/model/WakelockModels.kt`，不留在 ViewModel）：
      `appliedBlocked(result: ApplyResult): Map<String, Boolean>`（解包 item 层）与
      `mismatchNote(result: ApplyResult): String?`
- [x] `WakelockViewModel.setBlocked` 改为三态：已生效 / 系统未接受 / 未确认（字段缺失），
      不再用 `?: blocked` 假装成功
- [x] 批量请求时提示"部分未接受"，而不是整单成功/失败
- [x] 测试：正常、被拒、`applied` 缺键、多包部分成功——4 例 Kotlin 单测

**验收**：伪造一个 `applied` 里 `pkg: false` 的响应，UI 文案必须是"系统未接受"，且开关回到真实状态。

**改动**：`WakelockModels.kt`、`WakelockViewModel.kt`、`WakelockModelsTest.kt`。

## Phase 2 — P1：窗口跨重启失效 + reset 越权

### 2a 窗口（`window.rs`）

**问题**：内核计数器重启归零，`baseline` 却跨重启保留，delta 恒为 0；`summary` 仍报
`ranking_available = true`、`window_s` 约 24h，于是 UI 输出"设备基本处于空闲"这句假话。

- [x] 抽出 `window.rs`：`WindowState`（v2）+ `advance(current, now, uptime)` + `ranked()`
- [x] 重启判定：`uptime_s` 明显小于窗口年龄（含时钟回退）**或**基线整体回退 → 重建基线，
      窗口从重启时刻起算
- [x] `summary` 增 `window_reason`（`fresh`/`reboot`/`rollover`/`baseline`）与 `since_boot_s`
- [x] UI 空状态加"自重启以来"分支，不再无条件说设备空闲
- [x] 测试：重启后立刻有排行、`window_s` = uptime、v1 状态文件可平滑迁移、时钟回退不产生负值

### 2b reset 收窄

**问题**：`release_all()` 把**所有**非保护的 `WAKE_LOCK ignore` 全放掉，包括用户在别处设的；
`reset --all` 会连带撤销第三方策略。

- [x] `Module` trait 增 `reset_with_settings(&self, settings) -> Result<()>`，默认委托 `reset()`
      （与既有 `apply_with_settings` 一致的扩展方式）
- [x] `Registry::reset` 在删除配置**之前**把 `config.modules[id]` 传进去
- [x] 模块：只释放 settings 中记为自己的 `true` 条目；无 settings 时不改系统
- [x] 测试：第三方规则存活、自有规则被释放、受保护应用不动、空配置零 `cmd` 调用

**验收**：先造一条"外部" ignore，再走 reset，它必须还在。

**改动**：`module.rs`、`registry.rs`、`wakelock/mod.rs`、新增 `window.rs`、协议文档。

## Phase 3 — P2：AppOps 还原语义、share 分母、挂载节流

- [x] **3a 释放写 `default` 而不是 `allow`**：`appops.rs` 引入 `OpMode { Ignore, Default }`，
      解除拦截走 `default`，不再给应用打永久显式放行；ROM 不认时靠读回暴露
- [x] **3b share 分母**：先对**全部**增量求和再截断 Top 15；`summary` 增 `measured_ms`
- [x] **3c 挂载闸门**：抽 `debugfs.rs` + `MountGate`（时间注入），
      "挂载成功但表仍不可读"也要记账，避免总览 2s 轮询时反复 fork `mount`

**测试**：`default` 落盘断言、20 个源时 share 之和 ≤ 1 且 `measured_ms` 覆盖全量、闸门三态。

## Phase 4 — P3 与架构拆分

- [x] **4a 逐键持久化**：`registry` 抽纯函数 `persistable(requested, applied)`——对象逐子键
      `satisfied` 才持久化。修掉"一个包被拒 → 整条规则不落盘"
- [x] **4b 混合请求校验**：`validate_rules` 逐条判受保护（`{受保护:false, 其他:true}` 不再整单拒绝）
- [x] **4c 回滚失败可见**：回滚本身失败时在错误信息里列出未回滚的包
- [x] **4d 拆分与单源化**：`wakelock/mod.rs`（600 行）拆为 `mod.rs`（trait 实现）、
      `window.rs`、`debugfs.rs`、`appops.rs`（含 `OpMode`）、`state.rs`；
      新增只读项 `wakelock.protected`，APK 优先用它、旧模块回退内置列表——
      保护名单从此只有一处权威
- [x] **4e 状态文件**：版本迁移 + 长期不出现的源从基线中裁剪，避免无限增长
- [x] **4f 测试补口**：`wakelock held` 的缓存/`--refresh` 分支（当前 0 测试）、
      ViewModel 读回映射、挂载闸门

**验收**：`cargo test --workspace` 全绿、`clippy` 无新增告警、`assembleDebug` 通过。

## Phase 5 — A 类缺口（**可选，待确认**）

- [x] **5a 用户白名单 `wakelock.exempt`**：`{包名: true}`，`scope app`、`risk low`、persist；
      校验阶段拒绝拦截、reset 跳过；挑选器里提供"保护/解除保护"
- [x] **5b 一键恢复**：应用页顶部"全部恢复"（确认弹窗）→ `client.reset("wakelock")`，
      依赖 2b 的收窄语义；含 dry-run 单测

**验收**：受保护应用被用户加进 exempt 后，UI 开关禁用且模块拒绝写入。

## Phase 6 — B 类增值（**可选，待确认**）

- [x] **6a 唤醒次数排行**：窗口同时记录 `wakeup_count`/`event_count` 增量，
      `ranked` 增 `wakeups`（legacy 表 `events` 缺失用 0 表示并注明）；
      UI 支持"按时间/按次数"切换
- [x] **6b 上次唤醒原因**：新 verb `sysflow wakelock wakeup [--refresh]`，
      容错解析 `dumpsys power`，取不到就 `available:false` + 原因；唤醒源页顶部卡片
- [x] **6c 长持锁告警**：模块注入 `EventStore`（`state_dir.parent()`），
      `sample()` 每 300s 检查持锁超阈值（8s 超时、失败静默），同 `tag+uid` 6h 去重；
      `EventCopy.kt` 加文案
- [x] **6d 历史趋势**：`sample()` 增 `wakeups` 指标；新增通用折线图组件；唤醒源页趋势卡
- [x] **6e 从持锁列表直接拦截**：行内动作，受保护项禁用

## Phase 7 — 收尾

- [x] 文档同步：`docs/protocol.md` §3.18–§3.20、`docs/modules/wakelock.md`、
      `docs/development-plan.md` §7.3 与阶段 2 勾选（总览卡片保持未勾）
- [x] 全量测试 + `assembleDebug` + `diff -ru` 审查改动集
- [x] 输出真机待验收清单（`appops set ... default` 兼容性、`dumpsys` 字段差异、事件流）

## 兼容与风险

| 风险 | 处理 |
|---|---|
| 无版本控制 | Phase 0 快照 + 结束时 unified diff |
| 协议新增字段 | 一律"新增可选字段"，旧客户端忽略；`wakelock.protected` 缺失时 APK 回退内置表 |
| ROM 差异 | 所有写操作都以读回为准；`default` 不被接受时体现为 mismatch，不假装成功 |
| 无真机 | 单元测试覆盖逻辑，真机行为列进 Phase 7 验收清单，不宣称已验收 |

## 规模估算

| 阶段 | 主要文件 | 新增/改动测试 |
|---|---|---|
| P0 | 3 | 4 |
| P1 | 5 | 10 |
| P2 | 4 | 8 |
| P3/架构 | 8 | 10 |
| A 类（可选） | 6 | 6 |
| B 类（可选） | 10 | 12 |

核心（P0–P3 + 架构）不改公共协议语义，只做兼容性新增；A/B 类属于功能扩展，可独立交付。
