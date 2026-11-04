# Checklist（验证结果版，2026-11-04）

## A. 鼠标断组

- [x] `on_left_click` 正文命中处统一调用 `break_undo_group`（`interaction.rs:966`，`try_toggle_task` 之后、click_count match 之前，chrome 各分支均提前 return）
- [x] 回归测试 `mouse_click_breaks_undo_group`：单击移动后输入单独成步（走真实 `on_left_click` 路径）
- [x] 走查文档 B4 行「鼠标点击未接入」限制表述已删除并改为已补齐

## B. I3 根因

- [x] editor 级测试 `toggle_wrap_inside_table_cell_edits_source`：cell 内 ToggleWrap 修改源码且单步可撤销（含空选区插标记对）
- [x] `3e-walkthrough-results.md` I3 状态为「根因已定位 + 编辑有效/渲染不显示 + v2 富文本单元格」，附测试名
- [x] 测试证实预判（编辑有效），无真实缺陷，未做代码修复——符合「若失败才修」的分支

## C. H2 / U1 / B7-3 决策固化

- [x] results 与 manual 的 H2 口径一致：Esc 关闭 + 快捷键直达、导航 v2 立项
- [x] B4 行含「不加 200ms 静默窗口」及理由（与 fa584ed 去 500ms 窗口一致）
- [x] B7 行无「待定口径」，B7-3 标「已确认」，整行「✅ 4 通过」
- [x] manual「待决策项」段替换为「已决策（2026-11-04）」四条汇总
- [x] grep `待决策|待定口径` 在走查两文件中无未闭合悬挂（仅 1 处历史轮次叙述「当时按用户决定不处理」，属原始记录）
- [x] `.skills/INSTANCE.md` 无 H2/U1/B7-3/I3 待决策残留（全文核查，无需改动）

## D. 视图模式口径

- [x] README 非目标段含「源码编辑模式（v1 不做；Reveal 替代，v2 评估）」（README.md:39）
- [x] README 写明：默认模式 = focus/typewriter 全关、不持久化、启动即回默认；两开关彼此互斥（README.md:28）
- [x] 表述与 `WritingState::default()`（app.rs:218）、toggle 互斥实现（interaction.rs:1112-1130）、M1a 测试一致

## E. 发布门槛

- [x] results 末尾存在「v1.0.0 发布门槛（2026-11-04）」段：三项前置 + tag/Release Notes 动作
- [x] 门槛段明确「条件 2（needs-user 全部回填）完成前不打 tag、不起草 Release Notes」

## F. 门禁与边界

- [x] `cargo test --workspace` = **869 passed / 0 failed / 12 ignored**（867 + 2 新增）
- [x] `cargo clippy --workspace --all-targets -- -D warnings` 零警告
- [x] release 产物 5,825,536 B，与改动前一致（无增长）
- [x] 四个触发式项（Events / build_refs / wgpu / line_col）未被实施，触发条件文档未改动
- [x] 50 项 `needs-user` 状态未被改动（仅 H2 行按决策转为「已决策（降级）」，H 区其余 3 项仍为 needs-user）
- [x] 两个独立可回滚 commit：代码切片 / 文档切片
