# veil-core 批次 1+2 对抗性审查计划

> **目标**：验证批次 1（48163a4）+ 批次 2（3309edb / 6ba0113 / ac2293f / 66c6c35 / 37a227d / 2ab71cb）修复后的 veil-core 是否还有遗漏的地基问题。
> **方法**：2 agent 静态审查已完成（见下文发现），本计划补充实测验证 + 剩余边界穷举。
> **输出**：审查报告（P0/P1/P2 分级问题清单 + 地基完美性结论）+ 修复 commit（如有 P0/P1）。

---

## 0. Repository Research — 已发现的问题（agent 静态审查）

### ✅ 确认正确的改动（4 项）

| 改动 | 审查结论 | agent |
|---|---|---|
| inline.rs 括号索引预扫（scan_bracket_pairs 两遍扫描） | 跳过逻辑与旧 link_close 逐行同源；pair_resolves 预扫 vs 运行时无数据源分歧；所有递归路径覆盖 | Agent-1 |
| table() MAX_TABLE_COLS=128 截断同步 | 截断在 header/body 解析之前发生，列数用同一个 aligns.len() 传递给每一处，完全同步 | Agent-2 |
| reparse `<=` 回退条件 | 6 个反例全覆盖；极端边界穷举无遗漏（bi==0 / bi==len() 空栈 / 编辑点在块内部） | Agent-2 |
| id_index rebuild 中间态 | blocks=new_blocks 整体赋值 → build_refs 不触达 id_index → rebuild_id_index 原子 clear+枚举；无中间态可见 | Agent-2 |

### ❌ 发现的真实 bug（1 项）

| 级别 | 文件 | 问题 | 描述 |
|---|---|---|---|
| **P1** | history.rs L58-87 | **undo_bytes try_merge 路径未更新** | record() 在 try_merge 成功后直接 return，但 try_merge 对栈顶 Transaction 做 in-place 扩展（lp.inserted.push_str / lp.deleted.push_str），undo_bytes 没有相应增加。形式化反例：连续 ins(3,"a") + ins(4,"b") → merge 后栈顶 inserted="ab"（字节=2），undo_bytes 仍是 1。影响：**字节裁剪预算形同虚设**（步数裁剪仍正确）。 |

### ⚠️ 预存边界问题（非批次 2-1b/2-2 回归，旧 link_close 同有）

| 级别 | 文件 | 问题 | 描述 |
|---|---|---|---|
| P2 | inline.rs | bracket_pairs 全局扫描不感知强调 stop 边界 | 如 `~~a [b c~~ d](url) e~~`：全局括号配对让 `[` 拿到 stop 之后的 `]` → 可能跨越强调边界产链。当前 CommonMark 测试未覆盖这个场景（allow_link=true 在 strikethrough 内罕见）。建议独立跟踪，不在本次修复范围内。 |

---

## 1. Files and Modules — 本次计划的改动范围

| 文件 | 改动 | 来源 |
|---|---|---|
| `crates/veil-core/src/history.rs` | **修 P1 bug**：try_merge 成功后 undo_bytes 增加增量；加最小复现测试 | Agent-2 审查发现 |
| `crates/veil-core/src/syntax/inline.rs` | 无改动（静态审查确认无回归） | Agent-1 审查 |
| `crates/veil-core/src/syntax/mod.rs` | 无改动（静态审查确认无回归） | Agent-2 审查 |
| `crates/veil-core/src/syntax/block.rs` | 无改动（静态审查确认无回归） | Agent-2 审查 |
| `crates/veil-core/src/session.rs` | 无改动（纯转发） | Agent-2 审查 |
| `crates/veil-core/tests/core.rs` | **可选**：如有新发现补充集成测试 | 审查补充 |

---

## 2. Implementation Steps（P1 bug 修复 + 实测穷举）

### Step 1 — 实测复现 undo_bytes merge bug（证明存在）

在 history.rs mod tests 末尾加**最小复现测试**：

```rust
#[test]
fn undo_budget_sync_after_merge() {
    let mut h = History::default();
    h.record(ins(3, "a"));        // push_back, undo_bytes = 1
    h.record(ins(4, "b"));        // try_merge 成功 → return, undo_bytes 应 = 2 但实际仍是 1
    
    // 用 tx_byte_size 算出"应有"字节数 vs undo_bytes 实际值
    // 这行必须失败才能证明 bug 存在
}
```

预期：当前测试会 panic —— 证明 bug 真实存在（而不是理论分析）。

### Step 2 — 修复 undo_bytes merge 同步

**方案**：try_merge 前抓 old_top_sz，merge 后算 new_top_sz，差值加到 undo_bytes。

```rust
// history.rs record() 内部：
let group = classify(&tx);
if let (Some(group), Some(last_group)) = (group, self.last) {
    if group == last_group {
        let old_top_sz = Self::tx_byte_size(self.undo.back().unwrap());
        if self.try_merge(&tx, group) {
            let new_top_sz = Self::tx_byte_size(self.undo.back().unwrap());
            self.undo_bytes = self.undo_bytes.saturating_add(new_top_sz - old_top_sz);
            return;   // merge 路径不做步数/字节裁剪（栈长度不变）
        }
    }
}
```

**为什么 merge 后不跑裁剪**：栈长度没变（新 tx 被折入旧 tx），步数裁剪 `undo.len() > MAX_UNDO` 不可能触发；字节裁剪只在栈变长时才需要考虑——merge 让栈总字节略增但通常增量很小（单字符），远达不到 16MB 预算。

### Step 3 — 批次 2-5 额外穷举验证

**id_index rebuild 完整性**：写一条属性测试——反复 Session::open + apply + serialize + open 循环 100 次，每次 blocks() 所有 BlockId 都能通过 table()/inlines()/heading() 找到。验证 reparse 后 id_index 没有残留陈旧条目。

**table() 极限边界**：写一条测试——0 列表格（parse_delim_row 空返回 None）→ table() 返回 None（正确）；128 列 + 512 行 → 全量通过（不截断）；129 列 + 513 行 → 正确截断。

**reparse <= 边界**：已覆盖 6 个反例。额外穷举：空文档 `""` 插入一个字符（bi==0 且 blocks 空）→ 不 panic。

### Step 4 — 最终 4 重验证

```bash
cargo test --workspace                       # 全绿
cargo clippy --workspace --all-targets -- -D warnings  # 零警告
cargo build --release -p veil                # 记录尺寸（应 ≈ 5.8MB）
cargo test -p veil-core --test conformance commonmark   # 652/652
```

### Step 5 — 地基完美性确认

全部通过后输出审查结论：
- **地基完美**：批次 1+2 修复零回归 + P1 bug 已修 + 额外边界穷举无新问题 → 地基完美
- **或**：如有新发现 → 分类分级后处理

---

## 3. Dependencies and Considerations

| 项 | 内容 |
|---|---|
| **零新增 crate** | 继续执行 |
| **每 commit 门禁** | 测试 + clippy + release 尺寸 |
| **单步超限仍保留** | undo_budget_bytes_caps_stable_total 测试依赖这个语义，修 merge 同步不能影响它 |
| **批次 3 不动** | 架构模型（Events/Block 双轨/Transaction 简化等）仍属地基理想形态，走查收官后再立项 |
| **plan mode 约束** | 本计划获批前严禁改代码 |

---

## 4. Risks and Handling

| 风险 | 处理 |
|---|---|
| try_merge 修改后 undo_bytes 增量是 saturating_add 类型安全 | 用 `new_top_sz - old_top_sz`（usize subtraction 可能 underflow if new < old，但 try_merge 只做 push_str 扩展，不会缩小——不会发生） |
| 额外穷举测试挂性能（Session 循环 100 次） | 已有类似属性测试 `reparse_idempotent` 的种子固定模式；这次循环内只做 `open → apply → serialize` 不做 inline 解析；预计 ≤ 100ms |
| release 尺寸略增（history.rs 加几行） | 远低于 5% 预算 |

---

## 5. Validation（Step 4 输出格式）

```
审查结论：地基完美 / 地基有缺陷（P0/P1/P2 清单）
commit 清单：
  - <hash> 批次 2-4 补：修 undo_bytes try_merge 同步 + 最小复现测试
  - <hash> 批次 2-3 补：table() 0/128/512 边界测试
  - <hash> 批次 2-5 补：id_index 属性测试
4 重验证结果：
  - workspace: X passed / Y failed
  - clippy: 0 warnings
  - release size: XXXX B (+X.X%)
  - conformance: 652/652
```

---

## 6. 原「不在本次修复范围」三项 —— 现状（2026-11-03 收尾轮）

- P2 bracket_pairs 跨越强调 stop 边界 —— **已修**：`layer_limit` 按主循环跳过规则求本层 stop 上界后传入 `scan_bracket_pairs(limit)`；同时修复递归复用同一 parser 实例导致外层括号索引被内层覆盖的问题（`try_strikethrough` 用 `mem::take` 暂存/恢复）。测试：`strikethrough_stop_bounds_bracket_pairs`、`strikethrough_still_allows_link_inside`、`code_span_wins_over_strikethrough_stop`、`nested_strikethrough_limits_are_per_layer`。
- 批次 3 架构模型 —— **降级为按需触发清单**（见 `veil_core_remediation_plan_v2.md` 批次 3、`INSTANCE.md` 与 ADR-011）：#3/#4 本次落地，#5 已完成，#1 由数据否决（Events 只省可忽略的枚举），#6 仅文档口径。
- 批次 2-5c build_refs 增量 —— **不做**（每击键仅 O(#blocks) 次 kind 比较，收益微秒级；增量化需维护引用定义 first-wins 全局序，属语义风险）。触发条件：探针显示该段 >5% 才重开。

---

## 7. Step 4 落地记录（2026-11-03）

```
审查结论：地基完美（P0 0 项 / P1 1 项已修 / P2 1 项登记为不在本次范围）

commit 清单：
  - 3309edb  批次 2-1b：括号索引预扫，消除未配对 [ 的 O(n²)
  - 6ba0113  批次 2-2：code_span 反引号 run 索引
  - ac2293f  批次 2-3：table() 128×512 硬上限
  - 66c6c35  批次 2-4：undo 字节预算 MAX_UNDO_BYTES=16MB
  - 37a227d  批次 2-5：Syntax.id_index + Session.offset_of_line
  - 2ab71cb  批次 2-5 小修：clippy single_match
  - a8656d9  审查修 P1：undo_bytes try_merge 同步遗漏
  - 947d1dc  v1 发布前改进：veil-egui → veil-ui + Ports struct 收敛

4 重验证结果（2026-11-03 实测）：
  - workspace: 852 passed / 0 failed / 10 ignored
  - clippy: 0 warnings（--workspace --all-targets -D warnings）
  - release size: 5,825,536 B ≈ 5.56 MiB（红线 15 MiB，+0.18% vs 5,815,808 B）
  - conformance: CommonMark 652/652（A/C/D）、GFM 23/23（A/C/D）

复核修正（2026-11-03 代码实测）：
  - ADR-001 §3 原「块边界 mark 约数千个」不成立：常驻 mark 恒为 2（caret + anchor，峰值 4）
  - 批次 3 清单中「Session 缺 offset_of_line」已被 37a227d 修复，应从待办移除
  - 「Events 契约」正确性已由「BlockId 新鲜分配 + alive 集合 prune」覆盖（paint.rs after_edit 每次编辑调用 refresh_index 并以 session.blocks() 为准）；剩余仅为 O(#blocks) 重建的性能问题，非正确性缺陷
  - 「Mark 排序不变量」实测会被破坏：同偏移不同 Bias 的 mark 在纯插入后换序（Vec 顺序 = 创建顺序）。但全 workspace 无消费者依赖顺序（offset_of / remove_mark / transform_marks 均全量扫描），故**删除该不变量**而非加排序成本：create_mark 改为 push（顺带去掉 O(#marks) memmove），并新增随机编辑序列回归测试 + 20k mark 线性度探针；阈值 1,000 由 create_mark 的 debug_assert 兜底
```

