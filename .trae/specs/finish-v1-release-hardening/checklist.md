# Checklist（验证结果版，2026-11-03）

> 勾选口径：`[x]` = 已在本机取得证据（代码/测试/实测数字/文档）；未取得真机观察的项不写「通过」，按仓库既有铁律标 `needs-user` 并注明自动化证据。

## A. P0 崩溃清零

- [x] `math_source` 单行分支对 `"$$"`（无尾随换行）不再 panic，且有独立回归测试 `math_source_single_line_edge_cases`（`crates/veil-core/src/syntax/mod.rs`）
- [x] `math_source` 对 `"$$\n"`、`"$$$$"`、`"$$x$$"`、`"$$  $$"` 的输入输出与 spec 一致（同一测试四个断言）
- [x] `present` 在帧与目标缓冲长度不等时丢帧返回，不 `copy_from_slice` panic（`can_present` 前置守卫，`crates/veil-ui/src/paint.rs`）
- [x] 存在覆盖「尺寸失配 → 丢帧」的自动化判定用例 `present_requires_matching_lengths`；`present` 是该拷贝的唯一调用点，真实 surface 路径需 GUI（测试注释已说明）
- [x] 手工复验登记为 `needs-user`（不虚标通过）：文末连续输入两个 `$` 不崩溃——逻辑证据 `math_source_single_line_edge_cases`
- [x] 手工复验登记为 `needs-user`（不虚标通过）：最小化后恢复不崩溃且绘制正常——逻辑证据 `present_requires_matching_lengths`

## B. 击键路径归因

- [x] 探针输出 5 段独立数字：`core_apply+reparse` / `index_rebuild` / `dirty_layout` / `rasterize` / `frame_compose`（`app_tests.rs::perf_keystroke_attribution`）
- [x] 探针在 release 下可运行且标注 `#[ignore]`，不进入常规门禁
- [x] 5k / 10.5k / 20k 三档数字已写入 `docs/review/keystroke-attribution.md`
- [x] 归因表给出每段占 p95 的百分比（`index_share` 28.2 / 34.0 / 35.8，优化前 65.5 / 72.9 / 77.0），可直接支撑「改/不改」判断

## C. 索引增量化（条件）

- [x] 判定结果明确记录：**触发**（`index_rebuild` 占 p95 65.5%/72.9%/77.0% ≥ 20%），实施后回落至 28.2%/34.0%/35.8%
- [x] 判定偏差如实记录：`Block` 保留 `start_line/end_line` 供 reparse 扫描（非视图双轨），`Session::blocks()` 零 `offset_of_line`；实测 `blocks_enum` 6941→196µs@10.5k
- [x] `Events` 索引增量 **未触发（数据否决）**：枚举已降至约 4% 占比，剩余成本为任何正确索引都必须做的几何累计；裁决与数字见 tasks.md Task 3.3
- [x] 10.5k 块 p95 **5.20ms** ≤10ms、20k 块 **9.25ms** ≤16ms（release 实测，旧值 13.83 / 24.44ms）
- [x] 测试全绿（867 passed，口径由 852 更新）、clippy 零警告、CommonMark 652 + GFM 23 无回归
- [x] release 产物 5,825,536 B（与改动前一致，增长 0% ≤5%），传递依赖数未增加（合计 141 口径不变）

## D. GFM 删除线边界

- [x] `~~a [b c~~ d](url)~~` 不产生跨越 stop 的 Link，删除线在第一个 `~~` 闭合（`strikethrough_stop_bounds_bracket_pairs`）
- [x] `~~[x](u)~~` 仍在删除线内成链（`strikethrough_still_allows_link_inside`）
- [x] `` ~~a `b~~` c~~ `` 中代码段内的 `~~` 不被当作删除线闭标记（`code_span_wins_over_strikethrough_stop`）
- [x] `scan_bracket_pairs` 有明确上界参数 `limit`，`backtick_runs` 未被上界裁剪且注释写明原因（`layer_limit` + `parse_until`）
- [x] 递归复用 parser 实例导致的索引覆盖已修（`try_strikethrough` 用 `mem::take` 暂存/恢复，`nested_strikethrough_limits_are_per_layer`）
- [x] CommonMark 652 + GFM 23 一致性套件全通过

## E. 撤销分组

- [x] `Session` 暴露 `break_undo_group`，视图仅在 `editor.rs` 的 Intent 分派一处调用（`editor.rs:356`，非编辑意图断组）；已知限制：鼠标点击路径未接入
- [x] `ab` → 断组 → `c` → 一次撤销只退 `c`（`break_undo_group_splits_typing`）
- [x] 无光标移动的连续输入仍一次撤销全部回退（`typing_without_break_stays_single_step`）
- [x] `undo_bytes` 预算与既有合并测试语义不变（`undo_bytes_sync_after_merge` 等 a8656d9 回归用例全通过）

## F. Transaction 收敛

- [x] `Transaction { prim: PrimEdit }` 不再包含 `Vec<PrimEdit>`；`history.rs` 无「恒为 1」的防御性注释残留
- [x] `record` / `try_merge` / `pop_undo` / `pop_redo` / `push_undo` / `push_redo` / `tx_byte_size` 全部按单原语同步，无死分支
- [x] 既有 history 测试断言未被修改而全部通过

## G. 架构裁决留痕

- [x] `docs/adr/ADR-011-wgpu-deferral.md` 存在，含「不引入 wgpu」结论、四条触发条件、`spikes/` → ADR 立项 → 单 commit 的前置顺序
- [x] ADR-011 中无未实测的体积/依赖数字作为结论；所有增量标注「须 spike 实测」，且写明优化前/后的索引占比
- [x] 批次 3 在 `veil_core_remediation_plan_v2.md` 与 `INSTANCE.md` 中改为「按需触发清单」；#1 以数据否决、#2 已落地、#3/#4 已落地、#5 已解决、#6 仅文档口径
- [x] 批次 2-5c 在 `veil_core_remediation_plan_v2.md` 标注「**不做**」并附收益（微秒级）/风险（first-wins 全局序）理由与 >5% 重开阈值
- [x] `README.md` 非目标段落与 ADR 列表、`INSTANCE.md` 决策清单均已含 ADR-011
- [x] `line_col` 的「按码点计数、CRLF 计一列、emoji ZWJ 按码点」口径已写入 `veil_core_remediation_plan.md` 与 `veil_core_remediation_plan_v2.md`
- [x] `.trae/documents/adversarial_review_plan.md` 原「不在本次修复范围」三项已同步为现状（P2 已修、批次 3 触发式、2-5c 不做）

## H. 3e 走查收尾

- [x] R2 / T1 / I1 三项各有明确结论（均已修并附测试名），I3 保持用户指定不处理
- [x] 剩余 50 项逐区给出状态（C 11 / D 14 / E 8 / F 4 / G 9 / H 4），50/50 均有状态；加上已走查项与 I3，清单无未标注条目
- [x] 纯人工项（打印、系统主题/Mica、IME、盲测）标注 `needs-user` 并给出分区执行协议，等待用户回填
- [x] `docs/review/3e-walkthrough-results.md` 与 `3e-manual-walkthrough.md` 已回写（R2/T1/I1、B4 部分改善、H2 预检发现「菜单键盘导航未实现」并入待决策）

## I. 全局门禁

- [x] `cargo test --workspace` 全绿：867 passed / 0 failed / 11 ignored
- [x] `cargo clippy --workspace --all-targets -- -D warnings` 零警告
- [x] `cargo build --release` 成功，`target\release\veil.exe` = 5,825,536 B ≈ 5.56 MiB ≤15MiB
- [x] `veil-core` 的 GUI/异步依赖仍为 0（红线未变）
- [x] 无编辑往返字节级一致（CommonMark 一致性套件 + 往返测试覆盖）
- [x] 无跨任务混合提交：本轮改动**全部未提交**，工作区按任务边界可分别暂存；已知未纳管资产（`.trae/documents/*`、`.trae/specs/`、`docs/review/keystroke-attribution.md`）需在用户决定提交时一并纳入
