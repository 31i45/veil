# 收口 v1 待决策与已知限制（H2 / U1 / B7-3 / I3 / 鼠标断组）Spec

## Why

v1 加固完成后，走查文档里挂着三项「待决策」（H2 / U1 / B7-3）和两项「已知限制」（I3 表格内 Ctrl+I、鼠标点击未接入撤销断组），不收敛就无法定义「发布就绪」。经读码调查：H2 降级、U1/B7-3 维持现状即可；I3 需先验证根因再定口径；鼠标断组是 5 行级一致性修复，值得做。四个触发式项（Events / build_refs / wgpu / line_col）维持触发式、不动。

## What Changes

- **H2 菜单键盘导航：降级预期**（文档）。v1 承诺「Esc 关闭 + 全局快捷键直达」；↑↓/Enter 导航留 v2。
- **U1/B4 撤销粒度：维持现状**（文档）。连续性合并 + 断组为最终语义，不加 200ms 窗口。
- **B7-3 富文本粘贴：维持现状**（文档）。HTML→Markdown 结构转换、不保留视觉样式。
- **鼠标点击断组：实施**（代码）。`on_left_click` 正文命中三分支统一 `break_undo_group`，消除键盘/鼠标语义不一致。
- **I3 表格内 Ctrl+I：先验证后定口径**（测试 + 文档）。editor 级测试证明 ToggleWrap 在 cell 内可改源码；若通过，口径定为「源码已改、网格不渲染行内样式（v2 富文本单元格）」；若失败，按实际根因修复。
- **视图模式口径固化**（文档）。默认模式 = `WritingState::default()`（focus/typewriter 全关、不持久化、启动即回默认）；专注/打字机彼此互斥（toggle 已实现，M1a 有测试）；无源码模式——Reveal 机制替代，v1 不做并写入 README 非目标，v2 评估。
- **发布门槛定义**（文档）。v1.0.0 tag 前置条件与动作清单。

**BREAKING**：无。撤销断组属行为语义微调（更严格的步边界），不破坏 API。

## Impact

- Affected specs：3e 走查口径、v1 发布门禁、撤销语义
- Affected code：
  - `crates/veil-ui/src/interaction.rs`（`on_left_click` 正文命中分支接 `break_undo_group`）
  - `crates/veil-ui/src/editor.rs` 或 `app_tests.rs`（I3 验证测试 + 鼠标断组回归测试）
- Affected docs：`docs/review/3e-walkthrough-results.md`、`docs/review/3e-manual-walkthrough.md`、`.skills/INSTANCE.md`（如有待决策残留）

## ADDED Requirements

### Requirement: 鼠标路径撤销断组

系统 SHALL 在鼠标改变光标/选区时断开撤销分组，与键盘移动行为一致。

#### Scenario: 单击后输入另起一步
- **WHEN** 输入 `ab` → 鼠标单击移动光标 → 输入 `c` → 撤销
- **THEN** 仅撤销 `c`（与键盘方向键断组语义一致）

#### Scenario: 双击/三击选词选行后输入另起一步
- **WHEN** 输入文本 → 双击选词（或三击选行）→ 输入替换 → 撤销
- **THEN** 选区替换单独成步，不与此前的连续输入合并

### Requirement: 表格内行内格式语义明确

系统 SHALL 保证 Ctrl+I 等行内格式意图在表格单元格内可修改源码，并把「网格不渲染行内样式」固化为文档口径。

#### Scenario: 验证测试
- **WHEN** 光标位于表格单元格内容区间，应用 `Intent::ToggleWrap("*")`
- **THEN** 源码对应区间被 `*…*` 包裹，且可单步撤销

#### Scenario: 口径一致
- **WHEN** 查阅 `3e-walkthrough-results.md` 的 I3 记录
- **THEN** 状态由「按用户决定不处理」更新为「根因已定位：编辑有效、渲染不显示；富文本单元格留 v2」，并引用验证测试名

### Requirement: 决策固化

系统 SHALL 把 H2 / U1 / B7-3 三项决策固化为走查文档最终口径，三处引用（results 表 / results 附表 / manual 预期）互相一致，文档中不再存在「待决策」悬挂。

#### Scenario: H2 降级一致
- **WHEN** 查阅两走查文件的 H2 条目
- **THEN** 口径一致：v1 承诺「Esc 关闭 + 快捷键直达」，键盘导航（↑↓/Enter/选中高亮）标 v2 立项

#### Scenario: U1 / B7-3 决策一致
- **WHEN** 查阅 B4 / B7 行
- **THEN** B4 为「已修并决策：不加 200ms 静默窗口（与 fa584ed 一致）」；B7-3 为「已确认：结构转换、不保留视觉样式」

### Requirement: 发布门槛

系统 SHALL 在走查结果文档写明 v1.0.0 可检验前置：①本 spec 全部收口；②50 项 `needs-user` 由用户全部回填；③门槛满足后动作 = `git tag v1.0.0` + Release Notes 起草，走查未完成前不执行。

### Requirement: 视图模式口径

系统 SHALL 在文档中明确三态视图口径，且与代码事实一致。

#### Scenario: 默认模式
- **WHEN** 应用启动（无任何持久化视图状态）
- **THEN** 进入普通所见即所得编辑模式（focus=false、typewriter=false）；README/INSTANCE 中的描述与 `WritingState::default()` 一致

#### Scenario: 互斥关系
- **WHEN** 开启专注（或打字机）时另一个处于开启态
- **THEN** 另一个被强制关闭（既有 toggle 实现，M1a 测试锁定）；文档表述为「两个视图开关彼此互斥，全关即默认模式」，不表述为「三种模式」

#### Scenario: 源码模式不存在
- **WHEN** 查阅 README 非目标段
- **THEN** 含「源码模式（v1 不做）」条目：Reveal 机制（光标进入展开标记）为替代；v2 再评估文件内切换形态

## MODIFIED Requirements

### Requirement: 撤销分组语义

原描述：键盘移动/扩选断组，鼠标点击路径未接入（已知限制）。

新描述：键盘移动/扩选与鼠标单击/双击/三击均断组；`try_merge` 位置连续性为底层兜底，`break_undo_group` 为显式语义保证；两路径行为一致并有回归测试。

### Requirement: I3 处置

原描述：I3 表格内 Ctrl+I 疑似无效——按用户决定不处理。

新描述：根因定位为渲染层（`shape_cell` 纯文本、单元格不解析行内样式）；编辑路径经测试证明有效；处置 = 文档口径固化 + 富文本单元格留 v2，不改 v1 渲染代码。

## REMOVED Requirements

### Requirement: H2 菜单键盘导航（v1 范围）

**Reason**：菜单功能全部由全局快捷键直达，键盘用户无阻断；实现需新增选中项状态机，属新功能，发布前引入交互变更风险大于收益。
**Migration**：预期降级为「Esc 关闭、快捷键直达」；完整键盘导航留 v2 立项。

### Requirement: 表格单元格富文本渲染（v1 范围）

**Reason**：`shape_cell` 改 span 化涉及对齐、折行、命中映射全链路重写，属新功能；编辑语义已可用（源码被正确修改）。
**Migration**：v1 口径「单元格显示源码文本」；v2 立项做行内样式渲染。

### Requirement: 源码编辑模式（v1 范围）

**Reason**：是第二个完整编辑视图（等宽全文渲染、共享 Session/撤销栈、切换状态机、图片/公式/表格降级形态、测试矩阵翻倍），属数百行中型功能；Reveal 机制（光标进入展开标记）已覆盖查看/修改源码标记的核心诉求。
**Migration**：写入 README 非目标段；v2 评估文件内切换形态（参考 Typora）。
