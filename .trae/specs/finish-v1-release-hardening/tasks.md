# Tasks（执行结果版，2026-11-03）

> 每项独立改动、未提交 git；验收口径：`cargo test --workspace` = 867 passed / 0 failed / 11 ignored；`cargo clippy --workspace --all-targets -- -D warnings` 零警告；一致性套件 CommonMark 652 + GFM 23 全通过；release 产物 5,825,536 B（与改动前一致）。

- [x] Task 1: P0 崩溃清零
  - [x] SubTask 1.1: `math_source` 单行分支长度守卫——`crates/veil-core/src/syntax/mod.rs` 中 `t.len() <= 4` 返回 `Some("")`，不再 `t[2..0]`
    - [x] 测试 `math_source_single_line_edge_cases`：`"$$"`→`Some("")`、`"$$\n"`→`Some("")`、`"$$$$"`→`None`、`"$$x$$"`→`Some("x")`、`"$$  $$"`→`Some("  ")`
  - [x] SubTask 1.2: `present` 长度守卫——新增 `paint::can_present(frame_len, target_len)`，失配丢帧返回
    - [x] 测试 `present_requires_matching_lengths`（纯判定层；失配路径需真实 surface，已在测试注释说明）
  - [x] SubTask 1.3: 最小化路径闭合——确认 `Resized(0,0)` 跳过 surface resize 是对的，修点在 1.2
  - [x] 验收：workspace 全绿；GUI 手工复验移交用户（见 checklist A）

- [x] Task 2: 击键路径分项探针
  - [x] SubTask 2.1: `crates/veil-ui/src/app_tests.rs::perf_keystroke_attribution`（`#[ignore]`，release），5 段真实路径：core / index（`App::refresh_index`）/ dirty_layout（`App::ensure_block`）/ rasterize（`LayoutEngine::rasterize`）/ frame_compose（`App::render` headless）
  - [x] SubTask 2.2: 三档实测写入 `docs/review/keystroke-attribution.md`
  - [x] SubTask 2.3: 归因表完成——优化前 index 占 65.5% / 72.9% / 77.0%；子段拆分显示 `blocks_enum` 占 index 的 68%~70%

- [x] Task 3: 索引增量化（触发并大部分落地，最终以数据收口）
  - [x] SubTask 3.1: 判定——`index_rebuild` 占 p95 65%~77% ≥ 20%，触发
  - [x] SubTask 3.2: `Block` 持字节 `Range`（`syntax/model.rs`），reparse 重建/复用/整体平移全覆盖（`syntax/mod.rs`）；`Session::blocks()` 零 rope 换算。实测 `blocks_enum` 6941→196µs@10.5k
  - [x] SubTask 3.4: `refresh_index` 去掉与 `recompute_geometry` 重复的布局探测 + 同 id/同字节长度块复用 `est_h`；`geom_fill` 1969→607µs@10.5k
  - [x] SubTask 3.3 **不实施（附数据结论）**：`blocks_enum` 已降至 196µs（占合计 p95 约 4%），剩余 index 成本为 `stats`（814µs，文档字数全量重扫，与索引无关）+ `geom_fill`（607µs）+ `recompute`（343µs）；Events 驱动的局部索引只能省掉已可忽略的枚举，无法省去任一块都必须做的几何累计，故不引入其复杂度
  - [x] 验收：10.5k p95 **5.20ms** ≤10ms；20k **9.25ms** ≤16ms；5k **3.37ms**（旧值 13.83 / 24.44 / 7.94ms）；测试/clippy/一致性无回归；体积无增长

- [x] Task 4: GFM 删除线边界一致
  - [x] SubTask 4.1: 新增 `layer_limit`（`syntax/inline.rs`）按主循环跳过规则求本层 stop 上界
  - [x] SubTask 4.2: `scan_bracket_pairs(limit)` 只填 `i < limit`；`backtick_runs` 不裁剪并加注释；发现并处理"递归复用同一 parser 实例导致外层索引被内层覆盖"，在 `try_strikethrough` 用 `mem::take` 暂存/恢复
  - [x] SubTask 4.3: 测试 `strikethrough_stop_bounds_bracket_pairs`、`strikethrough_still_allows_link_inside`、`code_span_wins_over_strikethrough_stop`、`nested_strikethrough_limits_are_per_layer`
  - [x] 验收：CommonMark 652/652、GFM 23/23，0 panic

- [x] Task 5: 撤销分组显式断开
  - [x] SubTask 5.1: `History::break_group`（`history.rs`）+ `Session::break_undo_group`（`session.rs`）
  - [x] SubTask 5.2: 视图接线于 `editor.rs` 的 Intent 分派（仅非编辑意图断组）。**已知限制**：鼠标点击路径复用 `set_caret`，未单点接入，故只覆盖键盘移动/扩选
  - [x] 测试 `break_undo_group_splits_typing`、`typing_without_break_stays_single_step`

- [x] Task 6: `Transaction` 收敛
  - [x] SubTask 6.1: `Transaction { prim: PrimEdit }`；`record`/`try_merge`/`classify`/`tx_byte_size`/`revert` 全部同步
  - [x] SubTask 6.2: 删除空事务与多原语分支，注释写明单原语前提
  - [x] 验收：history 既有测试断言未改、字节预算（a8656d9 回归）仍通过

- [x] Task 7: 架构裁决留痕与文档同步
  - [x] SubTask 7.1: 新建 `docs/adr/ADR-011-wgpu-deferral.md`（四条触发条件、spike 前置、vello/skia/自研 GPU 否决理由；体积与依赖增量明确标注"须 spike 实测"）
  - [x] SubTask 7.2: 批次 3 改为触发式清单并逐项标注（#2/#3/#4 已落地、#5 已解决、#1 不实施附数据、#6 仅文档口径）
  - [x] SubTask 7.3: 批次 2-5c 标注"不做"并给收益/风险理由（>5% 才重开）；README/INSTANCE 增 ADR-011
  - [x] SubTask 7.4: 数字回写——测试 867、体积 5,825,536 B、依赖 141（未新增）、三档 p95 更新

- [x] Task 8: 3e 走查收尾
  - [x] SubTask 8.1: R2 已修（`Resized` 合并到下一帧，测试 `resize_rebuild_is_coalesced_into_one_frame`）；T1 已修（行级缩进改上下文前缀，测试 `tab_whole_line_indent_keeps_paragraph_kind` / `..._nests_list_item`）；I1 已修（CRLF 的 `trailing_newlines` 漏算 `\r\n`）；I3 按用户决定不处理
  - [x] SubTask 8.2: 剩余 50 项转为 `needs-user` 清单 + 分区执行协议（**真机观察代理无法代替**）；预检发现 H2 菜单键盘导航未实现，已入"待决策"表，未擅自改交互
  - [x] SubTask 8.3: 纯人工项已标注 `needs-user`（打印、系统主题/Mica、IME、盲测）
  - [x] SubTask 8.4: `3e-walkthrough-results.md` 与 `3e-manual-walkthrough.md` 已回写（R2/T1/I1 已修、B4 部分改善、H2 预检发现）

# Task Dependencies（实际执行顺序）

- Task 2 → Task 3（先测量：`blocks_enum` 占 index 68%~70%，才决定改 `Block` 形态）
- Task 1 → Task 8（崩溃清零后再做真机走查）
- Task 4 / 5 / 6 / 7 与上述并行，互不阻塞
- Task 7.4 数字回写放在最后统一执行
