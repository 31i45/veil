# Veil v1.0 发布规范化 + v1.1/v2.0 Roadmap - Spec

## Overview
- **Summary**: 规范 Veil v1.0.0 发布产物的位置与一致性（版本号、LICENSE、CHANGELOG、图标），重打一份干净 release，并用 v1.1 / v2.0 详细 roadmap 固化降级项与架构演进路径。
- **Purpose**: 当前虽然门禁六条件全过且已打 `v1.0.0` tag，但发布规范上存在版本号不一致（tag v1.0.0 vs Cargo.toml 0.1.0）、LICENSE/CHANGELOG 缺失、无独立 ico、无一键发布脚本等问题，影响对外可交付性；同时 v1.1/v2.0 的候选项只散落在 walkthrough 台账，缺乏优先级、排期与触发条件的集中管理。
- **Target Users**: Veil 开发者（下一个迭代的执行者）、需要下载使用的最终用户、审阅发布质量的 spec gate。

## Goals
- 发布产物位置规范、版本号自洽、文件元数据可读、附带 LICENSE + CHANGELOG
- 独立 ico 图标文件在仓库可直接获取，icon.rs 作为构建内嵌单一真源不变
- 一键 PowerShell 发布脚本（clean → test → clippy → build → verify → archive）
- 重打 release 并验证产物文件属性、大小、图标均正确
- 产出 v1.1 roadmap（降级项 + 表格可视化 + 菜单键盘导航 + 真机补测）与 v2.0 roadmap（架构批次 + 源编辑模式 + 协作/wgpu 触发条件）

## Non-Goals
- **不**改动 icon.rs 的代码生成机制（继续作为内嵌单一真源）
- **不**新增 installer/MSI/zip 打包脚本（单文件 exe 即分发形态，v1.0 维持简洁）
- **不**实现 v1.1/v2.0 的功能本身（spec 只规划）
- **不**改动已通过的 910 项测试或修复新 bug
- **永**不引入多用户协作、CRDT、OT、网络同步、云端存储（见"永恒设计原则"）
- **暂**不引入 wgpu、插件系统、Mermaid（视 v2.0 触发条件）

## Background & Context
- commit `4847646` 已归档真机证据、走查矩阵 needs-user 清零、门禁六条件全绿
- tag `v1.0.0` 已打在 `main`，但 `Cargo.toml` `version = "0.1.0"` 与 tag 不一致；build.rs 生成的 VERSIONINFO/FileVersion 跟着 0.1.0 走
- release exe 存在（target\release\veil.exe, 5,856,256 B）但构建时间早于门禁收尾（2026-11-05 11:18，比最后一个 commit 4847646 早），且缺符号表外的一切
- 图标：`crates/veil-ui/src/icon.rs` 是像素真源（RGBA + ICO 容器 16/32/48/64 四档），build.rs `include!` 内嵌；仓库根无独立 `.ico` 供 README 引用或直接下载
- 缺失 `LICENSE`、`CHANGELOG.md`，对开源发布不完整
- v1.1/v2.0 候选散落于 `docs/review/3e-walkthrough-results.md` 第 378–441 行，未排序、无排期
- 发布无一键脚本：手动执行 clean/build/test/clippy、手动归档产物，容易出错

### 永恒设计原则（2026-11-05 拍板）
**Veil 是单机单用户纯离线 Markdown 编辑器。永不引入多用户协作、网络同步、云端存储。**
- 差异化锚点：单文件 exe（< 8 MiB）、冷启动 ≤ 300 ms、零外部运行时、零网络依赖
- 架构上：veil-core 永远是纯文本 + 布局内核（零 GUI/异步依赖），veil-ui 永远是 winit + softbuffer 自绘 chrome
- v1/v1.1/v2.0 的 roadmap 均不得包含协作相关条目；任何时候有人提议协作方案，必须先论证"是否偏离单机极致定位"再立项

## Functional Requirements

### FR-1 版本号一致性
workspace `Cargo.toml` 与各 crate `Cargo.toml` 的 `version` 对齐为 `1.0.0`，build.rs 注入的 Windows VERSIONINFO `FILEVERSION`/`PRODUCTVERSION`/`FileVersion`/`ProductVersion` 自动跟随为 `1.0.0.0`，与 git tag `v1.0.0` 一致。

### FR-2 图标分发文件
仓库根 `icons/` 目录包含 `veil.ico`（由 icon.rs 代码生成的 16/32/48/64 四档 BMP-DIB 帧），作为 README 引用与用户直接下载的分发真源；icon.rs 与 build.rs 保持同步（build.rs include! icon.rs 内嵌，icons/veil.ico 仅在发布时由构建生成并拷贝）。

### FR-3 元数据文件
- 仓库根新增 `LICENSE`（MIT — Rust 生态默认）
- 仓库根新增 `CHANGELOG.md`（v1.0.0 版，基于 README 功能表 + walkthrough 门禁记录归纳）
- README.md 顶部补充徽标区（版本、构建、LICENSE）并引用 icons/veil.ico

### FR-4 一键发布脚本
`tools/release.ps1` 完整流程：
1. 前置检查（git status 干净 + tag 存在 + Rust 版本 >= 1.75）
2. `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D warnings`（门禁基线）
3. `cargo build --release -p veil`（clean build）
4. 验证（产物存在、大小在 4–8 MiB、FileVersion == Cargo.toml version、图标非默认 IDI_APPLICATION）
5. 生成独立 `icons/veil.ico` 到仓库根
6. 归档 `release-bundle/` 子目录（veil.exe + veil.pdb + README + LICENSE + CHANGELOG + icons）
7. 打印摘要（版本号、大小、clippy、测试数、图标）

### FR-5 干净 release 构建
release exe 在 task 1（版本号更新）后 clean 重打，确保：
- 产物时间戳晚于所有门禁 commit
- 文件属性 FileVersion/ProductVersion = 1.0.0
- 图标在标题栏/任务栏/Alt+Tab/文件资源管理器四处可见且一致

### FR-6 v1.1 详细 roadmap
输出 `.trae/specs/release-and-roadmap/v1.1-roadmap.md`，覆盖所有已决策降级 v1.1 的条目（硬件依赖 E5/E6/F4、PDF 落盘 E2、菜单键盘导航 H2、表格行列增删、富文本单元格渲染、真机补测 D2/D6），每条含：
- 优先级（P0/P1/P2）
- 触发条件（数据/时间/硬件）
- 验收标准（单测/真机）
- 预估工作量（S/M/L）

### FR-7 v2.0 详细 roadmap
输出 `.trae/specs/release-and-roadmap/v2.0-roadmap.md`，覆盖 README 非目标 + 架构批次（源编辑模式切换、wgpu 后移触发、协作/CRDT、Mermaid、插件/表格可视化），每条含：
- 触发条件（四条 ADR-011 条件 + 探针数据）
- 依赖前置（v1.1 哪些项必须先清）
- 优先级（P1/P2/P3 + 延期标记）

## Non-Functional Requirements
- **NFR-1**: 发布脚本单次执行耗时 ≤ 6 分钟（release 构建 + 测试 + clippy）
- **NFR-2**: 版本号更新不破坏 icon.rs / build.rs 的单一真源机制
- **NFR-3**: 新文件（LICENSE/CHANGELOG/icons/release-bundle）合计增量 ≤ 100 KiB（release-bundle 只含一个 exe，不计）
- **NFR-4**: roadmap 文档可直接作为 v1.1 启动的 spec 输入，每个条目独立、可验收

## Constraints
- **Technical**: Windows SDK rc.exe 可用（build.rs 需要；找不到会降级跳过）；Rust stable >= 1.75
- **Dependencies**: 无新增 crate；所有依赖沿用 workspace 现有
- **Business**: 不 push 远端 tag；产物留在本地 `release-bundle/`

## Assumptions
- 用户接受 MIT License 作为默认开源协议
- v1.1/v2.0 roadmap 中的硬件依赖项（D2 触控板、D6 200% DPI、E6 物理打印机）维持"有设备再测"策略，不阻塞 v1.1 其它项
- wgpu 后移触发条件（ADR-011）在 v2.0 roadmap 中逐条保留，不提前

## Acceptance Criteria

### AC-1: 版本号全链一致
- **Type**: `rule`
- **Given**: `Cargo.toml` workspace version = 1.0.0
- **When**: 执行 `cargo build --release -p veil` 并检查 exe 文件属性
- **Then**: Windows VERSIONINFO FileVersion、ProductVersion、原始文件名均为 `1.0.0.0`；资源管理器显示版本 1.0.0
- **Pass Condition**: `(Get-Item .\target\release\veil.exe).VersionInfo.FileVersion -match "^1\.0\.0"` 为真
- **Evidence**: 脚本自动验证输出 + PowerShell 人工截图

### AC-2: 仓库规范文件齐全
- **Type**: `rule`
- **Given**: 仓库根目录
- **When**: 检查文件列表
- **Then**: 存在 `LICENSE`、`CHANGELOG.md`、`icons/veil.ico`，且 CHANGELOG 至少含 v1.0.0 一节、LICENSE 含 MIT 文本
- **Pass Condition**: 三个文件存在且非空
- **Evidence**: `Test-Path LICENSE; Test-Path CHANGELOG.md; Test-Path icons\veil.ico`

### AC-3: 一键发布脚本通过
- **Type**: `rule`
- **Given**: git status 干净，tag v1.0.0 存在
- **When**: 执行 `powershell -File tools/release.ps1`
- **Then**: 脚本跑完 7 个步骤、无 error、exit code = 0、末尾打印版本号/大小/测试数/图标四项摘要
- **Pass Condition**: 脚本执行时间 ≤ 6 分钟、exit code = 0、release-bundle 目录存在且包含 5 项
- **Evidence**: release.ps1 输出日志

### AC-4: 干净 release 构建产物合格
- **Type**: `rule`
- **Given**: release 构建完成
- **When**: 检查 exe 文件属性
- **Then**: 图标在标题栏/任务栏可见（非默认 IDI_APPLICATION）、大小在 4–8 MiB、无 `.pdb` 外的多余 rlib
- **Pass Condition**: release exe FileVersion == 1.0.0 且 `Get-Item target\release\veil.exe | Measure-Object Length` 4MB–8MB
- **Evidence**: PowerShell 脚本输出

### AC-5: v1.1 roadmap 可执行
- **Type**: `rubric`
- **Dimension**: 条目覆盖度与可验收性
- **Scale**: 1-5
- **Anchors**: 1 = 只列条目标题无细节；3 = 有标题 + 优先级但无验收标准；5 = 每条含优先级/触发条件/验收标准/工作量预估
- **Pass Threshold**: >= 4
- **Evidence**: 人工审阅 roadmap 文档条目

### AC-6: v2.0 roadmap 有触发条件
- **Type**: `rubric`
- **Dimension**: 架构决策与触发条件明确度
- **Scale**: 1-5
- **Anchors**: 1 = 列"想做的"无触发条件；3 = 有候选但无 ADR 链接；5 = 每条关联 ADR + 触发条件 + 依赖前置
- **Pass Threshold**: >= 4
- **Evidence**: 人工审阅 v2.0 roadmap 文档

## Open Questions
- [x] LICENSE 选 MIT？（Rust 生态默认，spec 里定为 MIT）
- [x] CHANGELOG 覆盖到哪个版本？（只写 v1.0.0，历史留在 3e-walkthrough-results.md）
- [x] release-bundle 要不要 zip？（v1.0 维持单文件目录，不做 zip）
