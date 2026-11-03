# ADR-001: rope 选型 —— lapce-xi-rope，mark 语义自封装

## 状态：已采纳

## 背景

veil-core 的 buffer 模块需要：① O(log N) 区间插入/删除；② delta 表示——mark transform（编辑后 mark 跟随）与 undo 逆操作都建立在 delta 之上；③ 行索引（块级增量解析以行为扫描单位）；④ UTF-8 安全边界。

候选（2026-10-24 经 `cargo search` 核实存在性与版本）：

| 候选 | 版本 | delta/Transformer | 维护状态 |
|---|---|---|---|
| xi-rope | 0.3.0 | 有（Delta + Transformer） | 上游已归档，停更 |
| lapce-xi-rope | 0.4.0 | 有（同 xi-rope） | Lapce 编辑器的生产 fork，活跃 |
| crop | 0.4.3 | 无 | 活跃 |
| ropey | 2.0.0-beta.1 | 无 | beta |

反例检验：选 crop/ropey 则 delta 层需自研（mark transform + undo 逆操作两层都要），自研代码量与出错面大于 vendoring xi-rope；选自研 rope 则违反「基础设施手写成本 > 依赖成本」原则（docs/01-define-design.md B4）。

## 决策

1. 依赖 `lapce-xi-rope`。若其停更，退路是 vendor 进仓库自行维护（代码量小、许可兼容），不更换 rope 模型。
2. mark 语义（bias、Range、注解挂载）在 veil-core 内自封装，不直接暴露 xi-rope 类型——rope 实现可替换性由这层隔离保证。
3. mark 存储初版用 `Vec<Mark>`（按创建顺序，**不维护全局有序**——`offset_of`/`remove_mark`/`transform_marks` 均为全量扫描，无消费者依赖顺序；2026-11-03 复核发现同偏移不同 `Bias` 的 mark 在纯插入后会换序，故直接放弃该不变量而非加排序成本）+ 每次编辑全量 delta transform，复杂度 O(#marks)/次击键。**2026-11-03 复核修正**：实际常驻 mark 恒为 2（caret + anchor，移动光标峰值 4），全 workspace 无第二类消费者；原「块边界 mark 约数千个」的估计不成立。升级触发条件首次量化：常驻 mark ≥ 1,000（`create_mark` 内 `debug_assert` 兜底），或 mark 变换占帧预算 > 5%，或 mark 内存 > 1 MB（`Mark` = 24 B）。触发前**不做** sum-tree 预优化，由 INSTANCE §2 的 p95 ≤16ms 红线兜底；回归由 `mark_transform_matches_adr003_across_random_edits` 与 `mark_transform_scales_linearly_with_mark_count` 两条测试守住。
4. 协作位置模型不在本 ADR 范围（v1 冻结、v2 立项）：见 ADR-010。sum-tree 是单机 rope 的内部索引，不用于承担并发收敛语义。

## 后果

- 好：delta/undo/mark-transform 三件套同源，undo 层几乎免费；Lapce 生产使用提供工业验证；自封装层使替换成本可控。
- 坏：依赖一个 fork 而非官方上游；若 Lapce 项目方向变化需执行 vendor 退路（已记录，成本可接受）。

## 替代方案

- crop / ropey：无 delta 机制，需自研 delta 层，否决。
- xi-rope 0.3.0 原版：上游归档停更，否决（同源 fork 更优）。
- 自研 rope：基础设施级工作量，违反依赖决策树，否决。
- Zed text/rope：深嵌 Zed 仓库非独立 crate，且 sum-tree 模型引入即用属过度设计，否决（但 sum-tree 作为 §3 性能退路保留）。
