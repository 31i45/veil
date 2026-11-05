# Checklist

> 逐项可判定验收。每条都必须能用代码、命令输出或台账证据验证；无证据不得勾选。
> 对应 [spec.md](spec.md) 的 Requirements 与 Excellence Scorecard，执行顺序见 [tasks.md](tasks.md)。

## A. 自动化基建

- [x] 50 条走查用例全部标注 `auto / assisted / manual`，无空等级
- [x] 每条用例写明证据形式（探针日志 / 截图 / 人工记录）
- [x] `tools/walkthrough.ps1` 可按 PID 启动、调宽、滚动、截图、结束进程
- [x] 脚本不依赖前台焦点发送按键；执行前后用户其他窗口状态不受影响（仅短暂置顶捕获，不调用 `SetForegroundWindow`）
- [x] 脚本异常退出后目标进程被终止，无孤儿窗口，日志含失败步骤
- [x] 脚本能区分「脚本失败」与「产品缺陷」，不把脚本错误计入产品缺陷
- [x] 对 `02-混合元素.md` 的 dry-run 留下可复核的截图与探针日志
- [x] 证据命名含文档/宽度/滚动页/时间戳，可按台账路径复跑
- [x] 台账回填模板含「等级 / 命令参数 / 结果 / 证据路径 / 最后验证日期」

## B. v1.0 门禁

- [ ] 走查矩阵无 `needs-user` 悬挂，无未测条目
- [ ] P0/P1 阻断项为 0；不通过项已修复或书面降级（含风险）
- [ ] 遗留未处理项仅剩已决策降级（H2 / U1 / B7-3 / I3 及 P2–P3）
- [x] 冷启动应用内口径 ≤300ms（实测 176ms）
- [x] keystroke p95 <16ms（`bench_typing`，5005 块；实测 7.12ms）
- [x] 10 万字滚动后内存峰值 ≤150MB（实测 WorkingSet 40.76MB）
- [x] release exe ≤15MiB（实测 5,853,696 B）；`veil-ui` 直接依赖 = 4，无新增第三方依赖
- [x] Markdown 字节往返一致率 100%（`roundtrip_identity_without_edits`、`prop_random_edits_roundtrip_and_undo` 全绿；新增单元格源码回归）
- [ ] G1–G9 数据安全场景全部有证据；破坏性用例只在临时目录/副本执行
- [ ] 门禁六条件判定写入台账；未满足项写明阻塞原因
- [ ] 未在门禁前执行 `git tag v1.0.0`

## C. v1.1 收尾

- [x] 窄列宽单元格内中文标点自动换行后行首/行尾无禁则标点（`》` 场景回归：`narrow_cell_wrap_avoids_kinsoku_boundaries`）
- [x] 单元格断行修改不改变 Markdown 源码文本（`narrow_cell_wrapping_keeps_source_bytes` 逐字节一致）
- [x] 单元格内 Tab 跳格、点击定位在断行修复后仍正确（`tab_advances_to_next_cell_start`、`tab_on_last_cell_appends_row_and_moves_to_first_cell`、`tab_on_structure_char_moves_into_next_cell`、`table_grid_caret_drawn_at_active_cell_content_start`）
- [x] C5 表格异常点有枚举清单，逐点修复或书面降级（已修 4 点；行列增删 UI 入口书面降级到 v1.1）
- [x] D12-b 有源码形态证据（`**Chi*n*ese**`，caret=6 时布局文本原为全源码），修复点在 reveal 显隐（`deep_active`）而非 `apply_wrap`
- [x] D12-b 修复后真机截图留证（`docs/review/evidence/c3-nested-emphasis/04-step-x61.png` 显示 `Chi*n*ese`，外层 `**` 已消隐），且 `partial_italic_inside_bold_keeps_surrounding_bold_intact` 仍绿
- [ ] C7 三种图片入口 × 单/多/超大图 × 缩放档位有结论与证据
- [ ] E4/E7/E8、F1–F3、G1–G5/G7–G9、H1/H3–H4 全部有结论与证据
- [ ] 正文 C2 在用户提供原始场景后按同参数复跑；未复现则记录「未复现 + 参数」，不臆测结论

## D. 架构审计

- [ ] 审计覆盖 workspace crate 边界、`veil-core` 零 GUI/零异步、`veil-ui` 三域、`chrome` 三责、函数职责与导出面
- [ ] 每项发现含严重度与证据（文件/行为），无「风格偏好」条目
- [ ] 高严重度项已修复（带回归测试）或书面豁免（理由 + 风险 + 触发条件）
- [ ] 关键边界有测试或文档守卫（如 `veil-core` 无 GUI 依赖、缓存失效入口唯一）
- [ ] `crates/veil-core/**` 未被本计划改动；如改动则另有 spec 与用户批准
- [ ] 未发生无证据的全量重构；每步可回滚
- [ ] 审计结论落 `architecture-audit.md`，并给出「架构达标 / 未达标 + 阻塞项」

## E. 文档与发布

- [ ] README 测试数、体积、依赖数与当期实测一致
- [ ] 台账「覆盖统计」「下一轮优先级」「v1.1 收尾表」与实际状态一致
- [ ] `v1-progress-review-and-roadmap` 档位与触发条件同步
- [ ] 盲测 3–5 人完成三项任务，记录完成时间、卡点与第一句理由
- [ ] 盲测达线：≥3/5 无提示完成，且 ≥2 人给出「快 / 不丢字 / 源码可信」之一
- [ ] 未达线记为「未通过」并进入体验改进，未用自动化替代
- [ ] Release Notes 与红线实测、降级口径一致
- [ ] `git tag v1.0.0` 仅在 B4 六条件 + E2 盲测结论齐备后执行

## F. 范围守卫（防止过度设计）

- [x] 未引入像素级 golden 截图断言
- [x] 未引入 OCR 文本断言
- [x] 未把 GUI 脚本设为 CI 硬门禁
- [x] 未新增第三方依赖
- [x] 未自研排版引擎、未重写架构、未做 v2 功能（协作/CRDT/Mermaid/表格可视化编辑）
- [ ] 所有「通过」均可追溯到测试名、探针日志、截图或人工记录
