# Tasks（执行结果版，2026-11-04）

> 代码改动两切片（鼠标断组 + I3 验证测试），其余文档固化。验收：workspace **869 passed / 0 failed / 12 ignored**；clippy 零警告；release 5,825,536 B（与改动前一致）。

- [x] Task 1: 鼠标点击断组（代码）
  - [x] SubTask 1.1: `editor.rs:110` 新增 `pub(crate) fn break_undo_group` 门面；`interaction.rs:966` 在 `try_toggle_task` 之后、`click_count` match 之前统一调用（chrome/菜单/查找栏/滚动条/大纲/任务方框分支均提前 return，不受影响）
  - [x] SubTask 1.2: 回归测试 `mouse_click_breaks_undo_group`（`app_tests.rs`，走真实 `on_left_click` 路径）：输入 "ab" → 单击文首 → 输入 "c" → 第一次 undo 回 "ab"、第二次回空
  - [x] SubTask 1.3: `3e-walkthrough-results.md` B4 行已删除「鼠标点击未接入」已知限制表述，改为断组已补齐并附测试名

- [x] Task 2: I3 根因验证与口径（测试 + 文档）
  - [x] SubTask 2.1: `toggle_wrap_inside_table_cell_edits_source`（`editor.rs`）：cell 内选区 `ToggleWrap("*")` → 源码 `*c*`、单步撤销还原；另覆盖空选区插标记对居中。**证实预判：编辑路径完全有效，无真实缺陷**
  - [x] SubTask 2.2: `3e-walkthrough-results.md` I3 更新为「根因已定位：编辑有效、渲染层错觉（`shape_cell` 纯文本、cell 不解析行内样式）；v1 口径固化、富文本单元格留 v2」，附测试名
  - [x] SubTask 2.3: `3e-manual-walkthrough.md` I3 注记同步

- [x] Task 3: H2 / U1 / B7-3 决策固化（文档）
  - [x] SubTask 3.1: results H2 两处（预检发现表 + H 区表）改「已决策（降级）」：v1 承诺 Esc 关闭 + 快捷键直达，↑↓/Enter 导航留 v2
  - [x] SubTask 3.2: manual H2 预期同步；末尾「待决策项」段替换为「已决策（2026-11-04）」四条汇总（H2 降级 / U1 不加时间窗 / B7-3 维持结构转换 / I3 根因口径）
  - [x] SubTask 3.3: results B4 改「✅ 已修并决策：不加 200ms 静默窗口（与 fa584ed 去 500ms 窗口一致）」；B7 改「✅ 4 通过」，B7-3 标「已确认」
  - [x] SubTask 3.4: `.skills/INSTANCE.md` 全文无待决策残留（无需改）；grep 确认走查两文件无「待决策/待定口径」悬挂（仅 1 处历史轮次叙述保留）

- [x] Task 4: 视图模式口径（文档）
  - [x] SubTask 4.1: README 非目标段新增「源码编辑模式（v1 不做；以 Reveal 机制替代，v2 再评估）」（README.md:39）
  - [x] SubTask 4.2: README 功能第 10 条扩写：「两个视图开关彼此互斥、全关即默认模式、不持久化，启动即回默认」（README.md:28）

- [x] Task 5: 发布门槛与提交
  - [x] SubTask 5.1: `3e-walkthrough-results.md` 末尾新增「v1.0.0 发布门槛（2026-11-04）」段：决策收口（✅）+ needs-user 全部回填 + 门槛满足后 `git tag v1.0.0` + Release Notes；明确「条件 2 完成前不执行」
  - [x] SubTask 5.2: `cargo test --workspace` = 869 passed / 0 failed / 12 ignored；`cargo clippy --workspace --all-targets -- -D warnings` 零警告；release 5,825,536 B（无增长）
  - [x] SubTask 5.3: 两 commit：代码切片（interaction.rs + editor.rs + app_tests.rs）、文档切片（走查两文件 + README + spec 三件套）

# Task Dependencies（实际执行顺序）

- Task 1+2 代码先行（一个代理），Task 1.3/2.2/2.3/3/4/5.1 文档随后（一个代理）
- Task 5.2/5.3 最后统一执行
