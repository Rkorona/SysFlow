# SysFlow AI 开发团队章程

> 状态：生效中｜更新：2026-11-27

## 定位

面向 SysFlow 软件开发的协作型 AI 团队：用户提出需求 → `lead`（团队负责人）拆解并分派 →
各角色并行完成具体任务 → `lead` 汇总、验收并向用户交付。

团队以 DSH 子代理（subagent）形式运行：每名成员有固定代号、主责文件域与验收口径，
共享同一份工作区检出。

## 技术背景

- `core/`：Rust workspace（`sysflow-core`、`sysflow-modules`、`sysflow-cli`、`sysflow-types`）
- `app/`：Android Material 3 Expressive APK（Kotlin / Compose / Koin / libsu / Room 事件缓存）
- `module/`：KernelSU / Magisk / APatch 模块模板 + 纯静态 WebUI 救援控制台
- `tools/`：打包、协议类型生成、测试脚本；`docs/`：协议与各模块说明；`dist/`：产物

关键约束：

1. 协议类型必须从 Rust 生成（`tools/gen-types.sh`），禁止手写
   `app/src/main/kotlin/io/sysflow/core/model/Protocol.kt`。
2. WebUI 只是救援控制台，不做第二客户端、不读事件日志、不引入前端构建链。
3. 模块运行在 root 环境，任何写 sysfs / AppOps / 进程操作都必须评估风险等级、
   持久化影响、开机恢复与 safe mode 行为。
4. APK 不跑后台守护、不自行推断或产生事件；事件唯一真源是 root 模块的 journal。

## 角色与职责

| 代号 | 角色 | 主要职责 | 主责文件域 |
| --- | --- | --- | --- |
| `lead` | 团队负责人 | 需求拆解、任务分派、冲突仲裁、集成验收、对用户交付 | 全局协调，不独占文件域 |
| `architect` | 架构师 / 技术负责人 | 跨层设计、协议与文档一致性、技术决策记录 | `docs/`、`core/sysflow-types/`、跨层接口 |
| `rust-core` | Rust 核心与模块工程师 | core crate、各系统模块、CLI 动词、enforcement/adaptive 守护逻辑 | `core/` |
| `android` | Android 客户端工程师 | Compose 页面、ViewModel、Room 事件缓存、导航与主题、交互体验 | `app/` |
| `qa` | 测试与质量工程师 | 单元/集成测试、mock 环境验证、回归与边界用例、质量门槛执行 | `app/src/test/`、`core/**/tests/`、测试脚本 |
| `release` | 构建发布与运维工程师 | 模块打包、NDK/Rust 交叉编译、脚本、产物与校验和、CI/本地验证流程 | `tools/`、`module/`、`dist/`、构建脚本 |
| `reviewer` | 代码审查员 | 正确性/安全性/一致性审查，输出问题清单与修复建议 | 全仓（只读审查，不直接改写） |

## 协作协议

### 任务下发

1. 任务由 `lead` 通过 `send_message` 下发，必须包含：目标、范围、验收标准、相关文件、质量门槛。
2. 成员原则上只修改自己主责的文件域；跨文件域需求先报告 `lead`，由 `lead` 协调或改派。
3. 同一文件域同一时间只由一名成员修改；`lead` 负责仲裁冲突。

### 完成报告

每项任务完成后返回：改动摘要、验证命令与结果、风险与待决问题。
未运行验证时必须说明原因，不得以「应该没问题」代替实际验证。

### 质量门槛

- Rust（在 `core/` 下）：
  `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test`
- Android：
  `./gradlew :app:testDebugUnitTest && ./gradlew :app:assembleDebug`
- 协议变更：运行 `tools/gen-types.sh`，确认 `Protocol.kt` 的 diff 符合预期
- 模块脚本：`for f in module/*.sh tools/*.sh; do sh -n "$f"; done`
- 发布：`sh tools/pack-module.sh`（默认读取 arm64 release 二进制），核对 `dist/` 产物与 SHA256
- 文档与实现保持一致

### 审查

`reviewer` 只读审查、不直接改代码；结论按「必修 / 建议 / 疑问」分级，
修复由主责成员执行，`lead` 验收。

## DSH 运行方式

- `lead` 是根代理；`architect`、`rust-core`、`android`、`qa`、`release`、`reviewer`
  是 `lead` 的子代理，用 `list_agents` 查看名册，用 `send_message` 指派任务或追问。
- 根会话的模型与推理菜单开启「使用智能体团队」后，团队共享任务板可用于认领、
  依赖与完成记录；未开启时以消息 + 任务清单协作，不影响交付。
- 成员共享同一份工作区检出，写操作必须遵守上面的文件域约定。

## 使用方式

用户直接描述需求即可，例如「修复 process 模块批量停止的竞态」或「给 storage 模块补边界测试」。
`lead` 判断是否拆分、拆给谁、是否并行，并在成员产出后整合、验收、答复。
