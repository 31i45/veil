# v1 记录整理（记忆 / 走查进度 / 未处理问题 / 版本规划）Spec

## Why

真机走查轮次 1 结束后，结论散落在四处：`docs/review/3e-walkthrough-results.md`、`docs/review/3e-manual-walkthrough.md`、`.trae/specs/v1-progress-review-and-roadmap/*` 与项目记忆 `project_memory.md`。同一项存在重复甚至互相矛盾的表述（D12 一度标「已修复」，真机复测后又退回「未解决」），接手者无法快速判断当前真实状态与下一步。需要一次**只动文档**的整理，形成单一可信口径，支撑 v1 收尾与后续版本规划。

## What Changes

- 统一走查状态口径为四类：**已通过 / 已修复 / 未解决 / 未测**；每项只保留一个权威状态与证据（测试名或 commit hash），删除重复表里的过期结论
- 补齐未处理问题清单：D12-b（粗体部分加斜渲染异常）、C5（表格交互）、C2（行首禁则）、C7（图片细测）、内核隐患；每项写清「现象 / 已知根因或嫌疑 / 下一步排查 / 归属版本」
- 版本规划归位为四档：**v1.0 发布门禁 / v1.1 走查收尾 / 批次 3 内核架构 / v2 候选**，每档标注来源文档与触发条件，与 spec review 的 Gate Decision 对齐
- 同步项目记忆：进度快照（测试数、release 体积、commit 链）、lessons、已知未处理问题与文档一致
- **不修改任何代码**，不改任何交互行为；工作区仅文档与记忆变更

## Impact

- Affected specs: `v1-progress-review-and-roadmap`
- Affected code: 无（仅 `docs/review/*.md`、`.trae/specs/*.md`、项目记忆 `project_memory.md`）

## ADDED Requirements

### Requirement: 单一可信的走查状态台账

项目文档 SHALL 为每个走查项（C/D/E/F/G/H 区、P0/P1/P2/P3 分区）提供唯一状态与证据，不得在多个表里给出互斥结论。

#### Scenario: 查询任一项的当前状态

- **WHEN** 任何人查看某一走查项（如 D12、C5、E3）
- **THEN** 在 `3e-walkthrough-results.md` 中能定位到唯一状态、现象描述、证据（测试名或 commit）与下一步

#### Scenario: 旧结论不再误导

- **WHEN** 接手者阅读 D12
- **THEN** 看到的是「D12-b 未解决」及「编辑路径已由单测证明正确、嫌疑在渲染/标记显隐」，而不是「已修复」

### Requirement: 未处理问题清单可执行

每个未处理项 SHALL 包含：现象、已知根因或嫌疑、下一步排查方向、归属（用户自行处理 / v1.1 / 批次 3 / v2），且不得把未验证的推断写成结论。

#### Scenario: 接手 D12-b

- **WHEN** 接手者按清单开始排查
- **THEN** 明确先验证 `session.serialize()`（编辑路径）再查 reveal / 块级 inline 解析，并知道不要继续在 `apply_wrap` 上加补丁

### Requirement: 版本规划可追溯

四档规划 SHALL 标注来源（spec 文档 / ADR / `remediation_plan_v2.md`）与升级触发条件，避免同一事项在不同档位重复立项。

#### Scenario: 判断一项该进哪一档

- **WHEN** 评估 C5 表格或批次 3 的 Events 契约
- **THEN** 能按清单直接判定归属档位与前置条件

## MODIFIED Requirements

### Requirement: 项目记忆快照

`project_memory.md` SHALL 记录与实际基线一致的进度快照（`cargo test --workspace` 通过数 / ignored 数、release 可执行文件字节数、最新 commit 链），并包含本次整理的 lessons 与已知未处理问题索引。

**Migration**: 以 `2026-11-04` 快照替换旧快照，保留历史 commit 链但删除被本快照覆盖的过期数字。

## REMOVED Requirements

### Requirement: 「D12 已修复」的旧结论

**Reason**: 真机复测（2026-11-04）仍复现整行异常粗体 + 源码星号，旧结论与事实不符。

**Migration**: 改为遗留项 `D12-b（未解决）`；编辑路径正确性作为子结论保留在「本轮已修复项」中并注明「仅编辑路径」。
