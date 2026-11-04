# Veil v1 进度 Review 与路线图梳理

## Overview

- **Summary**: 对 Veil v1 Markdown 编辑器的全部已完成工作进行系统性走查，识别走查清单中 50 项 `needs-user` 条目的优先级分区、批次 2 代码优化项的完成度、触发式改进的阈值、以及 v2 立项项的边界，形成准确的当前进度快照与可执行的下一步计划。
- **Purpose**: 在 v1 发布前确立"当前状态是什么、哪些必须真机回填、哪些是 v2 的事"——消除模糊，让发布决策有据可依。
- **Target Users**: Veil 项目开发者（即用户本人）。

## Goals

1. **进度快照**：把已完成里程碑（批次 1-2 全部 5 项 + 崩溃修复 + 决策固化 + 发布加固 + ADR-011）逐条列清，每一项附 commit hash 或代码证据。
2. **走查清单分区**：把 50 项 `needs-user` 按"阻断/高/中/低"四档优先级分类，并标注每一项的自动化覆盖程度（完全覆盖 / 部分覆盖 / 无覆盖）。
3. **触发式项梳理**：批次 3 触发式清单 + ADR-011 四条触发条件，全部列出阈值与事实依据。
4. **v2 立项边界**：把 H2 菜单键盘导航、富文本单元格渲染、窗口过渡动画、源码模式、wgpu GPU 渲染等明确归为 v2 的项列出，附决策来源。
5. **v1 发布门槛路径**：明确从当前状态到 `git tag v1.0.0` 的最短路径——哪些真机验证必须完成、哪些降级、哪些可以进入 v2。

## Non-Goals

- **不执行真机走查**：真机验证由用户亲自执行，本 spec 只做分区与优先级建议。
- **不修复代码缺陷**：本 spec 不包含具体的 bug 修复实现；如需修复，先立项再实施。
- **不改变既有决策**：H2 降级、U1 不加时间窗、B7-3 HTML→Markdown、I3 口径固化、ADR-011 wgpu 不引入——这些决策全部保留，本 spec 只引用。
- **不重新设计架构**：三 crate 分层（veil → veil-ui → veil-core）、零 egui/GL、零 tokio 不变。

## Background & Context

### 已完成里程碑（证据链）

| # | 里程碑 | Commit / 文件证据 | 门禁 |
|---|---|---|---|
| M1 | 批次 1：HTML 属性越界 + reparse 回退条件 | `block.rs` / `mod.rs` / `core.rs` | 875 tests, clippy 0 |
| M2 | 批次 2-1b：inline.rs 括号索引预扫 O(n²)→O(n) | `inline.rs` L424 `scan_bracket_pairs` + L796 `bracket_pairs.get` | CommonMark 652/652 |
| M3 | 批次 2-2：inline.rs 反引号 run 索引 | `inline.rs` L416 `backtick_runs` + L701 查找逻辑 | CommonMark 652/652 |
| M4 | 批次 2-3：table() MAX_TABLE_COLS=128 / MAX_TABLE_ROWS=512 | `mod.rs` L222-244 | 不崩溃，降级截断 |
| M5 | 批次 2-4：history undo_bytes MAX_UNDO_BYTES=16MB | `history.rs` + commit 66c6c35 | 整篇替换×200 ≤ 50MB |
| M6 | 批次 2-5a/2-5b：Syntax.id_index + Session.offset_of_line | commit 37a227d | 派生 API O(1) |
| M7 | 批次 2-5c：build_refs 增量 → 裁决不做 | remediation_plan_v2 L535-537 | first-wins 语义风险 |
| M8 | 批次 3：触发式清单，6 项全部标注 | remediation_plan_v2 L612-623 | Events 已实施、line_col 仅改口径 |
| M9 | 崩溃修复：窗口最小化 0xC0000409 | commit 8812100 + review PASS | release + debug 真机 5min 存活 |
| M10 | 崩溃修复回滚防御层（正确建模哲学） | commit 079d9f8 | remove entry guard + catch_unwind |
| M11 | v1 决策固化：H2 降级 + U1 + B7-3 + I3 | commit 9a25d73 + eb61c8d | 文档 + 代码 |
| M12 | v1 发布加固：present 丢帧 + resize 合并 + 撤销断组 | commit 0da3fa7 + 6240faa | P0 崩溃清零 |
| M13 | ADR-011：wgpu v1 不引入，4 条触发条件 | `docs/adr/ADR-011-wgpu-deferral.md` | 体积 + 依赖约束 |

### 当前门禁快照（2026-11-04）

| 指标 | 实测 | 红线 | 状态 |
|---|---|---|---|
| workspace 测试 | 875 passed / 0 failed / 12 ignored | ≥ 80 | ✅ |
| clippy | 零警告 | -D warnings | ✅ |
| release 体积 | 5,825,536 B ≈ 5.56 MiB | ≤ 15 MiB | ✅ |
| 三档 p95 | 3.37 / 5.20 / 9.25 ms | ≤ 16 ms | ✅ |
| 内存 | 21.55 MB | ≤ 150 MB | ✅ |
| CommonMark 0.31.2 | 652/652 | 100% | ✅ |
| GFM | 23/23 | 100% | ✅ |
| veil-core GUI/异步依赖 | 0 | 0 | ✅ |
| 传递依赖 | 141 | ≤ 200 | ✅ |

### 走查清单现状

- 原始清单：65 项（A1-A6, B1-B9, C1-C11, D1-D14, E1-E8, F1-F4, G1-G9, H1-H4, I 盲测前置）
- 已完成真机验收：A 区 6/6 + B 区 8/9 = 14 项通过，A2/B4/B5/I1/T1 等缺陷已修复
- 剩余 `needs-user`：C-H 共 50 项（H2 已决策降级为 v2 不承诺）
- 自动化覆盖：42/50 有至少一个对应测试用例；8 项纯人工（C10/E2/E5/E6/F4/G1/G2/H2）

### 已决策不做项

| 项 | 决策来源 | 理由 |
|---|---|---|
| H2 菜单键盘导航 ↑↓/Enter | commit 9a25d73 + walkthrough-results L249 | v1 承诺 Esc 关闭 + 快捷键直达，导航 v2 立项 |
| I3 表格内 Ctrl+I 渲染 | walkthrough-results L140 | 编辑路径有效，渲染层不显示行内样式（shape_cell 纯文本），富文本单元格 v2 |
| wgpu GPU 渲染 | ADR-011 | 体积 ≤15 MiB + 依赖 ≤200 两条硬约束，瓶颈在 CPU |
| build_refs 增量 | remediation_plan_v2 L535-537 | first-wins 全局序语义风险 > 微秒级收益 |
| Events 增量数据扩展 | remediation_plan_v2 L618 | 已实施（索引层优化） |
| line_col 字素列 | remediation_plan_v2 L623 | 纯显示口径，无用户诉求 |
| 200ms 撤销静默窗口 | commit fa584ed + walkthrough-results L47 | 时间窗使撤销步数随停顿变化，语义不确定 |
| vello / skia / 自研 GPU | ADR-011 | 同 wgpu 理由 |

## Functional Requirements

- **FR-1**: 输出一份完整的进度快照文档（spec.md 本身即是产物），列出所有已完成里程碑的 commit / 代码证据。
- **FR-2**: 对 C-H 区 50 项 `needs-user` 按四档优先级（阻断/高/中/低）分类，并标注每项自动化覆盖程度。
- **FR-3**: 把批次 3 触发式清单（6 项）和 ADR-011 触发条件（4 条）汇总为一张"触发条件总表"。
- **FR-4**: 把所有已决策不做 / v2 立项项汇总为一张"非目标边界表"。
- **FR-5**: 制定从当前到 v1.0.0 tag 的最短路径——列出必须真机回填的条目、建议降级的条目、以及明确进入 v2 的条目。
- **FR-6**: 如果任何 needs-user 条目的根因定位显示需要代码修改，在 tasks.md 中立项对应修复任务。

## Non-Functional Requirements

- **NFR-1**: 所有优先级判定必须有事实依据（自动化覆盖程度、是否影响 v1 基本功能、是否阻断发布），不得凭体感。
- **NFR-2**: 非目标边界表中的每项必须有决策来源（commit hash / ADR 文件 / 文档行号），不能只写"不做"。
- **NFR-3**: 产出的优先级分区必须能被用户直接使用——即"哪些项今天必须测完才能 tag，哪些可以明天再说"一目了然。

## Constraints

- **Technical**: 门禁数字以 2026-11-04 实测为准（875/0/12, clippy 0, 5.56 MiB），任何新代码修改必须保持门禁全绿。
- **Business**: v1 发布门槛（walkthrough-results L257-261）明确要求"剩余 needs-user 全部回填"才能 tag。
- **Dependencies**: 批次 2 全部 5 项 + 批次 3 全部 6 项已完成或裁决，本 spec 不引入新依赖或新 crate。

## Assumptions

- 用户本人将执行真机走查（spec 只做分区建议）。
- 如果某项 needs-user 经真机发现是阻断性缺陷，立项修复后再回填。
- 现有自动化测试覆盖有效——有测试覆盖的条目真机通过概率更高。

## Acceptance Criteria

### AC-1: 进度快照完整性
- **Type**: `rule`
- **Given**: 已完成的 13 个里程碑（M1-M13）
- **When**: 查阅 spec.md 的 Background & Context 章节
- **Then**: 每个里程碑都有可验证的代码证据或 commit hash；无遗漏
- **Pass Condition**: 13/13 里程碑有证据；门禁数字与当前实测一致
- **Evidence**: 本 spec "Background & Context §已完成里程碑" 表格 + 2026-11-04 门禁实测

### AC-2: 走查优先级分区合理
- **Type**: `rubric`
- **Dimension**: 优先级分区的可操作性与事实依据充分性
- **Scale**: 1-5
- **Anchors**: 1 = 分区无依据，全凭体感；3 = 部分分区有依据，但边界模糊；5 = 每项分区有明确依据（是否影响基本功能 + 自动化覆盖程度 + 发布门槛约束）
- **Pass Threshold**: >= 4
- **Evidence**: tasks.md 中的 C-H 区优先级分区表

### AC-3: 触发条件与非目标边界汇总
- **Type**: `rule`
- **Given**: 批次 3 触发式清单 + ADR-011 四条触发条件 + 已决策不做项
- **When**: 查阅 spec.md 的"已决策不做项"和 tasks.md 的汇总表
- **Then**: 所有触发条件有阈值，所有非目标项有决策来源
- **Pass Condition**: 6/6 批次 3 项 + 4/4 ADR-011 条件全部列出；8/8 已决策不做项全部有来源
- **Evidence**: 本 spec Background & Context §已决策不做项 + tasks.md 触发条件总表

### AC-4: v1 发布路径清晰
- **Type**: `rule`
- **Given**: 当前状态与 v1 发布门槛
- **When**: 查阅 tasks.md 的"v1 发布路径"章节
- **Then**: 明确列出"必须真机回填才能 tag"的条目、"可降级"的条目、"进入 v2"的条目
- **Pass Condition**: 三类条目无交叉覆盖；用户可以直接照单执行
- **Evidence**: tasks.md v1 发布路径章节

### AC-5: 代码缺陷立项准确
- **Type**: `rule`
- **Given**: 走查清单中 needs-user 条目的根因定位
- **When**: 对任何需要代码修复的条目
- **Then**: tasks.md 中有对应修复任务，且任务引用正确的根因文件/行号
- **Pass Condition**: 如有缺陷需修复，每个修复任务附根因定位证据；如无缺陷，pass condition 自动满足
- **Evidence**: tasks.md 修复任务列表（如果有）

## Open Questions

- [ ] C 区（排版中日韩细节 11 项）和 D 区（交互 14 项）在 v1 发布中的权重——是否全部需要真机通过才能 tag？
- [ ] C10（10 万字文档打开 ≤2s / 内存 ≤150MB）有门禁红线但无自动化覆盖——是否可以用 headless 探针数据替代真机？
- [ ] F4（Win11 Mica / Win10 不崩）纯平台实现无单测——是否需要真机验证才能发布？
- [ ] E5（页码范围打印）和 E6（物理打印机抽验）是硬件依赖项——无物理打印机的话如何处理？
- [ ] G1/G2（任务管理器杀进程恢复 + 恢复后清快照）是进程级场景——是否需要真机验证？
