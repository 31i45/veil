# ADR-010: 协作位置模型 —— v2 立项，v1 冻结

## 状态：推迟（v2）；v1 不实现

## 背景

2026-11-03 架构审计提出的问题：`Buffer` 的 mark 存于有序 `Vec`，每次编辑 O(#marks) 全量 delta transform。若未来 mark 数量爆炸（如每字符一个 mark），是否应换成 sum-tree？若做协作编辑，位置模型应如何设计？

先核对事实（2026-11-03 代码实测）：

| 事实 | 证据 |
|---|---|
| 全 workspace 常驻 mark 恒为 2（caret + anchor），移动光标瞬时峰值 4 | `crates/veil-ui/src/editor.rs` 的 `set_marks`；无第二类 mark 消费者 |
| `Mark` = 24 B；`transform_marks` 为纯算术比较，零分配 | `crates/veil-core/src/range.rs`、`buffer.rs` |
| 块级视图缓存的稳定键是 `BlockId`，不是 mark；reparse 复用 id | `crates/veil-ui/src/view/cache.rs`、`veil-core/src/syntax/mod.rs` |
| v1 非目标明确含协作/CRDT 与任何网络功能 | README、INSTANCE §0 |

由此可判定：单用户场景下 mark 层不需要任何扩容；「mark 爆炸」只在协作或多光标规模下才可能出现，而那是 v2 的范围。

## 决策

1. **v1 冻结**：不改 mark 数据结构，不上 sum-tree，不提前做 block-relative 重构。触发阈值与退路登记在 ADR-001 §3。
2. **v2 协作立项时的位置模型分两类**：
   - **本机光标/选区**：可继续用 mark，或改用 block-relative（`(BlockId, offset_in_block)`）。后者只是优化项——块 id 在本地 reparse 中保留，且视图缓存已以此为准。
   - **远程光标 / 评论锚点 / 批注 / 待办锚点**：必须使用 CRDT anchored position（Yjs `RelativePosition`、Automerge `Cursor` 形态：不存字节偏移，而存「相对某个 CRDT item 的位置 + assoc 方向」，解析时沿冲突可交换的编辑图重算）。
3. **明确否决两种错误解法**：
   - 用 sum-tree 解决协作：sum-tree 是单机 rope 的内部索引（O(log m) 查找/更新），解决不了并发编辑的收敛语义。协作的成本不在数据结构，在语义。
   - 用 block-relative 当跨端锚点：块边界会被并发编辑拆分/合并/移动，「块 id + 块内偏移」在两次并发编辑后即失真。
4. **触发条件**：仅当协作编辑立项时才启动本 ADR。启动前需单独评审 CRDT 选型（自研 / 引入 Yjs 系 / Automerge），并新增红线（锚点解析延迟、离线重连收敛、冲突副本）。

## 后果

- 好：v1 不付协作复杂度预算；mark 层保持「2 个元素」这一可验证事实；避免了「把语义问题误当性能问题」的弯路。
- 坏：v2 若立项，锚点层是全新子系统而非现有 mark 的升级——工作量与验证成本独立于 v1；本 ADR 只给出方向，不给出实现。
- 风险：若将来出现「每字符一个 mark」的真实需求（例如逐字符批注），ADR-001 §3 的阈值会先被触发，届时按那里的退路处理，与本 ADR 无冲突。

## 替代方案

| 方案 | 结论 | 理由 |
|---|---|---|
| 现在就换 sum-tree | 否决 | 收益为 0（#marks=2），成本 1–2k 行高风险树代码 + 验证负担 |
| 现在就改 block-relative | 否决 | 无当前需求；块内 offset 仍会随块重解析漂移，收益不确定 |
| 每字符建 mark 表达稳定位置 | 否决 | 内存 24 B/字符（10 万字 ≈ 2.4 MB）且语义错误——稳定位置属于 block/CRDT 层，不属于 mark 层 |
| 协作时用 block-relative 锚点 | 否决 | 块边界在并发编辑下不稳定（见决策 3） |
