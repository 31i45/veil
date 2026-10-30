# 阶段 C 功能补齐计划（编辑能力缺口）

> 状态：**C6 已完成（573 测试全绿）；阶段 C 六片全部完成，等待用户确认后续安排**
> 依据：2026-10-26 对全 crate 的实际检索（非推测），以及 ADR-009 S1~S6、阶段 B+ 完成后「地基收官、回到功能主线」的决定。

## 1. 已核实的能力缺口

全 crate 检索（`crates/**/*.rs`）无任何匹配的实现痕迹：

| 缺口 | 检索证据 | 用户可感知后果 |
|---|---|---|
| Ctrl+A 全选 | 无 `SelectAll` / `select_all` | 无法一键全选，只能拖选或 Shift+点击 |
| 双击选词 / 三击选行 | 无 `DoubleClick` 相关分支 | 鼠标只能点定位、拖动扩选 |
| Tab / Shift+Tab 缩进 | 无 `Key::Tab` 映射 | 列表层级调整只能手工敲空格 |
| Ctrl+N 新建 | 无 `Action::New` / `new_draft` | 「文件」菜单缺少最基本的入口 |
| Ctrl+H 查找替换 | 仅 `Action::Find`（P3-3 只读取） | 找到后只能手工改 |
| 按词移动 / 按词扩选 | 无 `Key::Left if mods.ctrl` 分支 | 长行内只能逐字符移动 |

## 2. 现有约束与可复用面（已核实）

- `Intent` 枚举现有 14 个变体：`InsertText / DeleteBackward / DeleteForward / Newline / MoveLeft / MoveRight / MoveDocStart / MoveDocEnd / SelectLeft / SelectRight / SelectDocStart / SelectDocEnd / Undo / Redo`（`crates/veil-egui/src/editor.rs`）。
- `veil_core::Session` 已暴露本计划所需的全部只读原语：`text_len`、`text_slice`、`line_of_offset`、`line_col`、`prev_grapheme`、`next_grapheme`、`blocks`、`block_inlines`。
- `Session::apply` 只有三种写原语：`Insert / Delete / Replace`；一次 `apply` 即一个撤销事务。**多行缩进可用单次 `Replace` 覆盖整段完成，无需内核改动。**
- 按键路由唯一位置：`input.rs::translate`；动作唯一分派点：`interaction.rs::perform_action`。
- 鼠标点击与拖选在 `app.rs`（`MouseInput` / `CursorMoved`）；查找栏状态在 `view/search.rs`。

**结论：六个缺口全部可在 veil-egui 内完成，veil-core 预计零改动。**

## 3. 切片与顺序

排序原则：先做「零内核改动、最小 diff」的切片验证模板，再逐步进入改变编辑行为与新增 UI 的切片，最复杂的词边界规则放最后。

| 片 | 目标 | 新增 Intent | 涉及层 | 风险 | 验收 |
|---|---|---|---|---|---|
| **C1** | Ctrl+A 全选 | `SelectAll` | editor.rs → input.rs → 单测 | 极低 | 全选后任意输入即整体替换；Ctrl+A 后按 ← 收敛到行首 |
| **C2** | Ctrl+N 新建 | 无（新 `Action::New`） | interaction.rs → doc_state.rs → 单测 | 低 | 新建后：路径空、标题 `veil`、撤销栈空、脏标清除、图片/布局缓存按新会话代次失效、文件监听解除 |
| **C3** | Tab / Shift+Tab 缩进（含多行选区） | `Indent` / `Dedent` | editor.rs → input.rs → 单测 | 中 | 多行选区一次 Tab 对每行加 4 空格，单次撤销可整体回退；空行不缩进；Shift+Tab 不删非空白字符 |
| **C4** | Enter 列表续行 | 无（`Newline` 内策略） | editor.rs（块内策略）→ 单测 | 中 | 在 `- ` / `1. ` / `> ` / `- [ ] ` 行尾回车自动续标记；空列表项回车退化为普通换行（结束列表）；编号项递增 |
| **C5** | Ctrl+H 替换（单次 / 全部） | 无（扩展 `Action`） | view/search.rs → interaction.rs → chrome/paint（浮栏加替换行）→ 单测 | 中高 | 替换可撤销；全部替换单事务；替换后命中缓存重建；查找栏关闭时状态清空 |
| **C6** | 双击选词 / 三击选行 + Ctrl+←/→ 按词移动与扩选 | `WordLeft` / `WordRight` / `SelectWordLeft` / `SelectWordRight` | editor.rs（词边界扫描）→ app.rs（点击计数）→ input.rs → 单测 | 高（CJK 与 ASCII 边界规则） | 双击选中整词、双击空白选中空白串；三击选中整行；Ctrl+←/→ 停在词边界；选中态与 Shift 扩选不冲突 |

每片独立实现、独立 commit（用户确认后再提交）、独立可 `git revert`。

## 4. 每片统一测试策略

1. **Editor 层单测**：Intent 语义（光标/选区落点、文本结果、撤销一次回退）。
2. **路由单测**：`translate` 的键位映射（含「无 Ctrl 时 `a` 仍是文本」这类分流断言）。
3. **交互层单测**：`perform_action` 后的可见状态（标题、脏标、缓存代次）。
4. **性质测试**（若触摸到派生缓存）：复用阶段 B 的 proptest 性质，确认 memo 命中值 == 强制重算值。
5. 每片收尾：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets` 零警告、`cargo test --workspace` 全绿。

## 5. 非目标（本阶段不做）

- 多文档 / 标签页
- 语法高亮主题扩展、表格可视化编辑（仍为 v1 非目标）
- 正则查找 / 区分大小写选项（除非 C5 实现中发现几乎零成本）
- 双向文本（RTL）与复杂字形簇的按词边界（C6 只覆盖 ASCII + CJK）

## 6. 停止点

C1 完成即停，等用户确认后再开始 C2；后续每片同此节奏。
