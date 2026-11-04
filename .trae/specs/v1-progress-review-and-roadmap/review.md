# Veil v1 进度 Review 与路线图 — Independent Review

## Checkpoints

- [x] CP-R1: 已完成里程碑证据链完整
  - **Type**: `rule`
  - **Covers**: AC-1 (M1-M13)
  - **Evidence**: 13/13 里程碑全覆盖；10 个 commit hash 全部 `git cat-file -t` 返回 `commit`；ADR-011、remediation_plan_v2.md、inline.rs L424/416、mod.rs L222-244 均存在且内容匹配

- [x] CP-R2: 门禁数字与当前实测一致
  - **Type**: `rule`
  - **Covers**: AC-1
  - **Evidence**: `cargo test --workspace` → 875 passed / 0 failed / 12 ignored；`cargo clippy --workspace --all-targets -- -D warnings` → 零警告；`target\release\veil.exe` → 5,825,536 B — 全部与 spec 数字完全一致

- [x] CP-R3: 走查优先级分区有事实依据
  - **Type**: `rubric`
  - **Covers**: AC-2
  - **Scale**: 1-5 = **5**
  - **Anchors**: 每项分区有三维证据（自动化覆盖描述与代码测试名匹配、功能影响判断、发布门槛约束）
  - **Pass Threshold**: >= 4 ✅
  - **Evidence**: 抽查 C10, E3, H2, D1, G7 五项均合理

- [x] CP-R4: 触发条件与非目标边界汇总完整
  - **Type**: `rule`
  - **Covers**: AC-3
  - **Evidence**: 批次 3 共 6 项有 remediation_plan_v2 L612-623 来源；ADR-011 触发条件 4 条与文件 L37-L40 对应；已决策不做项 8 项全部有来源引用

- [x] CP-R5: v1 发布路径无遗漏步骤
  - **Type**: `rule`
  - **Covers**: AC-4
  - **Evidence**: 5 步发布路径逻辑清晰；独立 review 发现分区统计表数字笔误和 tag 命令矛盾，已修复

- [x] CP-R6: C10 headless 探针数据可信
  - **Type**: `rule`
  - **Covers**: AC-2, AC-5
  - **Evidence**: bench_c10.rs 代码合理；实测 106,601 字纯 txt → open+排版 166.84ms；5005 块结构文档 → 527ms；均远低于 2s 红线

- [x] CP-R7: 临时产物不影响门禁
  - **Type**: `rule`
  - **Covers**: 全局约束
  - **Evidence**: `cargo clippy --workspace --all-targets -- -D warnings` 包含 examples 目录零警告；`cargo test --workspace` 875 全绿

- [x] CP-U1: 最终交付文档质量
  - **Type**: `rubric`
  - **Covers**: 文档可操作性
  - **Scale**: 1-5 = **4**（修复后）
  - **Anchors**: spec.md + tasks.md 信息密度极高，用户可直接照单执行
  - **Pass Threshold**: >= 4 ✅

## Review History

### Review R1 — 2026-11-04

- **Result**: **pass**
- **Reviewer**: Independent read-only agent (general-purpose)
- **Checks Performed**:
  - `git cat-file -t` × 10 确认 commit hash
  - `cargo test --workspace` → 875/0/12
  - `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
  - `(Get-Item target\release\veil.exe).Length` → 5,825,536
  - `cargo run --release -p veil --example bench_c10` → 166.84ms
  - Grep 搜索测试函数名验证 tasks.md 简写与代码匹配
  - Read 验证 remediation_plan_v2 行号和 ADR-011 内容
- **Checkpoint Results**:
  - CP-R1 (`rule`): **pass**
  - CP-R2 (`rule`): **pass**
  - CP-R3 (`rubric`): **pass**; score 5/5
  - CP-R4 (`rule`): **pass**
  - CP-R5 (`rule`): **pass**（修复后）
  - CP-R6 (`rule`): **pass**
  - CP-R7 (`rule`): **pass**
  - CP-U1 (`rubric`): **pass**; score 4/5（修复后）
- **Findings**:
  - **F1**: 分区统计表数字笔误（P2 写 7 实际 6、P3 写 3 实际 4）— 已修复
  - **F2**: Step 4 tag 命令矛盾（lightweight + annotated 同名）— 已修复
- **Overall**: **PASS — merge/tag-ready**

## Gate Decision

**PASS — 交付就绪**

所有 8 个 checkpoint 通过（含 rubric 评分 5/5 和 4/5）。发现的 2 处瑕疵已在 tasks.md 中修复。交付物完整：spec.md（需求）、tasks.md（计划）、review.md（审查）、3e-walkthrough-results.md（走查结果 + 优先级附录）、bench_c10.rs（C10 探针）。

## 后续版本规划快照（2026-11-04）

> 与 `docs/review/3e-walkthrough-results.md` 的「## 未来版本规划候选（2026-11-04 快照）」同一口径。
> **数字时点**：上文 Gate 证据中的 875 passed / 5,825,536 B 为本 spec 交付审查时点数据；此后 commit `4deeca9` 已使基线变为 **894 passed / 12 ignored / 5,840,896 B ≈ 5.57 MiB**（见项目记忆 2026-11-04 快照），两处不矛盾。
> **边界**：上方 Gate Decision（PASS）是 spec 交付审查结论，不等于 v1.0 打 tag 许可；打 tag 仍受走查发布门槛约束。

| 档位 | 内容 | 来源 | 升级 / 解除触发条件 |
|---|---|---|---|
| **v1.0 发布门禁** | P0 C10 内存 ✅ 28.83 MB、窗口最小化崩溃 ✅（`8812100`/`079d9f8`）；D9-b 崩溃已随 `4deeca9` 落地；剩余 needs-user 全部回填；H2/U1/B7-3/I3 决策收口 | 本 spec（review.md Gate Decision、tasks.md）；`3e-walkthrough-results.md`「v1.0.0 发布门槛」 | 走查清单无未标注项且阻断项为 0 → `git tag v1.0.0` |
| **v1.1 走查收尾**（不阻塞 tag） | D12-b（未解决，用户自行处理）、C5 表格（用户判定「比较严重」）、C2 行首禁则、C7 图片细测；E4/E7/E8、F1–F3、G1–G5/G7–G9、H1/H3–H4 补测 | `3e-walkthrough-results.md` 轮次 1「遗留未处理项」 | v1.0 tag 后首个 patch/minor 内清；未清不阻塞 tag，但阻塞 v1.1 收口 |
| **批次 3 内核架构**（触发式，不排期） | 批 3 清单 6 项（tasks.md 现状：1–4 已实施、5 已解决、6 仅文档口径）；内核隐患复核：`build_refs` 增量（已裁决不做）、`bracket_pairs` 强调边界（`~~` stop 已补）；`math_source` 单行守卫已修 | `remediation_plan_v2.md` L612-623、tasks.md「批次 3 触发式清单总表」 | 走查收官且出现新探针数据（击键 p95 回退或块表语义不足）才立项 |
| **v2 候选** | 协作（本地 mark + 远程 CRDT anchored position）、Mermaid、表格可视化编辑器、wgpu 后移、内核隐患复核 | ADR-010（冻结 v1/v2 边界）、ADR-011（触发条件 L37-L40，含 4 条）、`remediation_plan_v2.md` | ADR-011 任一条成立且附 spike 数据；协作待 ADR-010 边界重评 |

### 与既有结论的对应关系

- **Gate Decision（PASS）** 仅覆盖 spec 交付物完整性，不改变 3e 走查台账中 D12-b「未解决」、C5/C2/C7「未解决」与未测区的状态。
- **批次 3** 已是触发式清单，不再作为独立里程碑；本快照只登记现状与触发条件，不产生新的排期承诺。
