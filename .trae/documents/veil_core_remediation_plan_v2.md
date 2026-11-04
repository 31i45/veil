# veil-core 批次 2 详细修复计划

> **批准前严禁动代码**。本计划承接 `.trae/documents/veil_core_remediation_plan.md`（批次 1 已完成），覆盖批次 2 全部 5 个 commit 的代码级修法、测试、验证命令与风险处理。
> 所有行号基于 `git diff` 未提交状态（批次 1 + inline.rs 7 行 skip case）。

## 0. Pre-Commit：批次 1 改动先落盘

当前工作区 dirty：

| 文件 | 改动 | 来源 |
|---|---|---|
| `crates/veil-core/src/syntax/block.rs` | +8 −1 | Commit 1-2 HTML 属性循环边界 |
| `crates/veil-core/src/syntax/mod.rs` | +5 −1 | Commit 1-3 reparse 回退 `<` → `<=` |
| `crates/veil-core/tests/core.rs` | +86 行 | Commit 1-1/1-3/1-4 10 条测试 |
| `crates/veil-core/src/syntax/inline.rs` | +7 行 | **批次 2-1 第二轮修法（表层 skip，不彻底）** |

**操作**：先把 block.rs / mod.rs / core.rs 三个批次 1 文件 commit（inline.rs 留在工作区，等批次 2-1b 彻底修法一起提交）：

```bash
git add crates/veil-core/src/syntax/block.rs crates/veil-core/src/syntax/mod.rs crates/veil-core/tests/core.rs
git commit -m "批次1：修 HTML 属性越界 + reparse 回退条件 + 10 条属性测试"
# 验证（commit 后立刻跑）：
cargo test --workspace                       # 全绿
cargo clippy --workspace --all-targets -- -D warnings  # 零警告
cargo build --release -p veil                # 记录尺寸
```

commit 后 inline.rs 的 7 行 skip case 和 walkthrough 文档仍是未提交状态。**批次 2-1b 开始时先 `git checkout inline.rs` 彻底回滚，再做括号索引预扫方案**。

---

## 批次 2-1b：inline.rs 未配对 `[` O(n²) → 括号索引预扫

### 目标
- 10000 个未配对 `[` 的段落，inline 解析从 170ms 降到 ≤ 10ms（线性）
- CommonMark 0.31.2 全 652 Example 继续 pass（**尤其 Example 559**）
- 不破坏 inner_pair_kills_opener / pair_resolves 语义

### 根因
`link_close()`（inline.rs:733-766）是**每次** `try_link()` / `try_image()` 调用时从当前位置**重新扫到块尾**的 O(n) 栈扫描。对 N 个未配对 `[` 的段落，总复杂度 O(N·n)。

上一轮（当前工作区 inline.rs 7 行改动）在**主循环普通字符分支**加了 skip case，只能防止主循环**再次**从同一位置重试——但 try_link 第一次调用时 link_close 已经把块尾扫完了，所以无效。

### 修法概要
把 `link_close` 的**括号配对**逻辑（栈匹配 + 跳过转义/代码段/HTML）从「每次 try_link 临时做」改成「parse_until 入口处一次性做 O(n) 预扫」，建两个数据结构：

1. **`bracket_pairs: Vec<Option<usize>>`**：对 `self.s` 每个位置 pos，返回与之配对的方括号位置（开→闭、闭→开、孤立→None）
2. **`resolved_count_prefix: Vec<usize>`**：前缀和数组，`resolved_count_prefix[pos]` = `self.s[..pos]` 内「能成链（pair_resolves=true）且非图片开括号（前邻无 `!`）」的 `[` 数量。这样区间 `(a, b)` 内是否存在「淘汰本链接开括号」的内层，O(1) 查询：`resolved_count_prefix[b] - resolved_count_prefix[a+1] > 0`

预扫同时做 pair_resolves 判定（它需要查 refs，而 parse_inlines 总在 build_refs 之后调用，所以 refs 已准备好）。

### 具体改动

#### 1. InlineParser 加字段（inline.rs:400-405）

```rust
struct InlineParser<'a> {
    s: &'a str,
    base: usize,
    refs: &'a RefMap,
    pos: usize,
    /// 一次性括号配对索引（parse_until 入口处建）。未配对位置存 None。
    bracket_pairs: Vec<Option<usize>>,
    /// 位置 i 之前（不含 i）「能成链且非图片开括号」的 `[` 累计数。
    /// O(1) 查询区间 [lo, hi] 内是否有内层成链：prefix[hi+1] - prefix[lo+1] > 0
    resolved_prefix: Vec<usize>,
}
```

构造处（inline.rs 大概 L395 的函数）补初始化：
```rust
fn new(s: &'a str, base: usize, refs: &'a RefMap) -> Self {
    Self {
        s, base, refs, pos: 0,
        bracket_pairs: Vec::new(),
        resolved_prefix: Vec::new(),
    }
}
```

#### 2. 新增 `scan_bracket_pairs()`（inline.rs:410 parse_until 之前）

```rust
/// 预扫 self.s 建括号配对索引 + 内层淘汰判定前缀和。
/// O(n)：栈扫描，跳过转义/代码段/HTML（和 link_close 原跳过逻辑完全同源）。
fn scan_bracket_pairs(&mut self) {
    let b = self.s.as_bytes();
    let n = b.len();
    let mut pairs: Vec<Option<usize>> = vec![None; n];
    let mut open_stack: Vec<usize> = Vec::new();
    let mut pos = 0;

    while pos < n {
        match b[pos] {
            b'\\' if pos + 1 < n => { pos += 2; continue; }
            b'[' => { open_stack.push(pos); pos += 1; }
            b']' => {
                if let Some(o) = open_stack.pop() {
                    pairs[o] = Some(pos);
                    pairs[pos] = Some(o);
                }
                pos += 1;
            }
            b'`' => {
                // 跳过代码段（和 link_close 同源）
                let run = b[pos..].iter().take_while(|&&x| x == b'`').count();
                pos = code_span_close(self.s, pos, run).unwrap_or(pos + run);
            }
            b'<' => {
                // 跳过 HTML 标签内方括号
                pos = html_tag_end(self.s, pos).unwrap_or(pos + 1);
            }
            _ => { pos += 1; }
        }
    }

    // 前缀和：只统计「能成链 + 前邻无 !（非图片开括号）」的 [
    let mut prefix: Vec<usize> = vec![0; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i];
        if b[i] == b'[' {
            if let Some(close_pos) = pairs[i] {
                // 检查前邻是否 `!`（图片开括号 → 不统计，因为内层图片开括号永不淘汰本开括号）
                let prev_is_bang = i >= 1 && b[i - 1] == b'!' 
                    && (i < 2 || b[i - 2] != b'\\');
                if !prev_is_bang && self.pair_resolves(i, close_pos) {
                    prefix[i + 1] += 1;
                }
            }
        }
    }

    self.bracket_pairs = pairs;
    self.resolved_prefix = prefix;
}
```

#### 3. parse_until 入口建索引（inline.rs:416 之后）

```rust
fn parse_until(...) -> Vec<InlineNode> {
    self.scan_bracket_pairs(); // ← 新增：一次性预扫
    let mut toks: Vec<Tok> = Vec::new();
    // ... 其余不变
```

#### 4. try_link 改用索引（inline.rs:667 附近）

**删除** `let text_end = self.link_close(start + 1, false)?;` 替换为：

```rust
let start = self.pos;
let text_end = match self.bracket_pairs.get(start).cloned().flatten() {
    Some(p) => p,
    None => return None, // 本开括号未配对 → 成链失败，按裸文本
};
// 内层淘汰：O(1) 查询 (start+1, text_end) 区间内有无内层成链且非图片开括号的 [
let inner_resolved = self.resolved_prefix[text_end] - self.resolved_prefix[start + 1];
if inner_resolved > 0 {
    return None; // 内层成链淘汰本链接开括号（inner_pair_kills_opener 语义）
}
// 后续分支 1/2/3 完全不变：](...) / ][label] / shortcut
```

#### 5. try_image 改用索引（inline.rs:801+）

同理，把 try_image 内部对 `link_close(start+1, true)` 的调用替换成 `bracket_pairs[start]`。图片开括号 `ours_is_image=true`，内层淘汰恒不触发（inner_pair_kills_opener 第二分支 `!ours_is_image && ...` 为 false），所以无需查 `resolved_prefix`。

#### 6. **删除** `link_close()` 和 `inner_pair_kills_opener()`（inline.rs:733-797）

这两个函数的语义已完全被 `scan_bracket_pairs` + `resolved_prefix` 替代。但保留 `pair_resolves`——它在预扫时被调用，且 try_link 后续分支（判断 `](` / `][label]` / shortcut）**不再**需要它（因为预扫时已经判定过了）。

#### 7. 主循环普通字符分支的 skip case（inline.rs:472-476）——**删掉**

括号索引已经彻底解决了 O(n²)，skip case 不再需要。让主循环正常走，`[` 会先让 try_link/try_image 试一次（O(1) 查索引），失败后正常前进。

### 正确性保证

| 语义 | 原来 link_close 做的 | 新方案等价 |
|---|---|---|
| 括号配对 | 栈扫描 | 预扫 `bracket_pairs` |
| 跳过转义 | `b'\\' if ... => i += 2` | 预扫同源 |
| 跳过代码段 | `code_span_close` | 预扫同源 |
| 跳过 HTML | `html_tag_end` | 预扫同源 |
| 内层成链淘汰 | `inner_pair_kills_opener(o, p, ours_is_image)` | `resolved_prefix` 前缀和 |
| 图片开括号豁免 | `if o >= 1 && b[o-1] == b'!' ...` | 前缀和只统计非图片开括号 |

**CommonMark Example 559 关键路径**：
- `[[*foo* bar]]` + 定义 `[*foo* bar]: /url "title"`
- 预扫时：内层 `[`（位置 1）配对 `]`（位置 12）→ `pair_resolves` 查 shortcut refs 命中 → `would_resolve=true` → 入前缀和
- 外层 `[`（位置 0）配对 `]`（位置 13）→ try_link 查到 `inner_resolved = resolved_prefix[13] - resolved_prefix[1] > 0` → **返回 None（淘汰外层）**
- 主循环前进到位置 1，内层 `[` 再试 → pair_resolves 命中 → **成功** → shortcut 引用 → 输出 `<a href="/url" title="title"><em>foo</em> bar</a>` 包一层 `[...]`
- 期望 HTML：`<p>[<a href="/url" title="title"><em>foo</em> bar</a>]</p>` ✓

### 测试（core.rs 新增）

1. **`inline_unpaired_brackets_perf`**：性能断言。构造 10000 个 `[` 的段落（无配对 `]`），测 inline 解析耗时 ≤ 10ms。
2. **`inline_example_0559_structure`**：直接用 Example 559 输入，断言 inline 节点序列：外层 `[` 是纯文本，内层有一个 Link 节点（url 指向 `/url`，children 含 Emph + text）。**这条是括号索引修法的核心回归保护**。
3. **`inline_nested_link_example_512`**（可选）：如果有嵌套链接的 CommonMark Example，也固化一条。
4. **`inline_paired_regression`**：合法链接/图片/嵌套链接/引用式链接/shortcut/自动链接各一条正常路径测试，确保括号索引没漏掉正常 case。

### 验证命令

```bash
# 1. 先跑 conformance——必须 652/652
cargo test -p veil-core --test conformance commonmark
# 2. 跑 veil-core 全量——尤其新测试
cargo test -p veil-core
# 3. 跑 workspace
cargo test --workspace
# 4. clippy 零警告
cargo clippy --workspace --all-targets -- -D warnings
# 5. release 打包 + 记录尺寸
cargo build --release -p veil
```

### 风险处理

| 风险 | 处理 |
|---|---|
| pair_resolves 在预扫时调用需要 refs 表——parse_inlines 总在 build_refs 之后，安全 | 确认 parse_inlines 调用链：session.apply → reparse 末尾 build_refs → 视图调 block_inlines / heading 等 → 调 parse_inlines（refs 参数直接传）。预扫发生在 parse_until 入口处，此时 refs 已可用。 |
| link_children 递归调用 parse_until 时内层 self.s 更短，要重建 bracket_pairs | 递归深度 ≤ 16（MAX_INLINE_DEPTH），每次重建 O(inner_len)，总复杂度 O(outer_len)——因为每层括号只被建一次。可接受。如果后续要优化，可在 InlineParser 存 base 偏移，一次预扫全段，递归时用偏移查索引。 |
| pair_resolves 预扫时判定的 would_resolve 值，和 try_link 后续分支判定的同一对括号会不会不一致？ | 不会。pair_resolves 只依赖闭合位置之后的文本（`](` / `][` / shortcut）和 refs 表。这两者在预扫与后续判定间不会改变。但**注意**：try_link 后续分支在「目标/title 不合法」时会从 inline 链接回退到 shortcut（Example 0568）——预扫时 pair_resolves 会**直接查 shortcut**（因为 pair_resolves 最后一条就是 shortcut 判定），如果 shortcut 有定义 would_resolve 就为 true。和实际 try_link 行为一致。 |
| 新增 Vec 字段导致 InlineParser 不再能直接构造 | InlineParser 目前在 inline.rs 内部有三处构造点（大概 L395 `new`、L348 `parse_inlines`、link_children L720-725）。都要补新字段初始化（空 Vec）。 |

### Commit 消息

```
批次2-1b：inline.rs 括号索引预扫，彻底消除未配对 [ 的 O(n²)
- parse_until 入口 O(n) 建 bracket_pairs + resolved_prefix 前缀和
- try_link/try_image 改用索引替代 link_close，内层淘汰 O(1)
- 删除 link_close/inner_pair_kills_opener（pair_resolves 保留供分支判定）
- 加 4 条测试：性能断言 + Example 559 结构回归 + 合法链接/图片正常路径
- CommonMark 0.31.2 全 652 Example 继续 pass
```

---

## 批次 2-2：inline.rs 反引号 run O(n²)

### 目标
512KB 含不匹配反引号 run 的段落 → inline 解析 ≤ 10ms

### 根因
`try_code_span()`（inline.rs:569-598）：当开 run 长度为 open，它用 `rest.find('`')` 从当前位置一路扫到 EOF——**每次** parse_until 遇到一个反引号 run，都从那个位置重新扫到块尾。N 个不匹配反引号 → O(N·n)。

但反引号有个**关键不同于括号**的性质：**同一个 run 内所有反引号长度相同**，而且「开 run 匹配不到等长闭 run 时，这个 run 内部任一位置都不能再作为开标记」（CommonMark Example 347）。所以我们根本不需要对每个反引号单独建索引——只要在预扫时**登记所有不同长度的 run 位置**，然后 try_code_span 按开 run 长度「找下一个同长度 run」即可。

### 修法概要
在批次 2-1b 的 `scan_bracket_pairs()` 之后，加一个**反引号 run 位置表** `run_index: Vec<RunEntry>`（也可以合并到同一个预扫函数里，一次遍历建完）。`RunEntry` 存储：起始偏移、run 长度、指向下一个同长度 run 的索引。try_code_span 拿到开 run 长度 open 后，直接从 run_index 里找 `len == open` 的下一个 run 位置，O(k) 查找 k 个候选。

### 具体改动

#### 1. InlineParser 加字段

```rust
/// 反引号 run 索引：每个 run 的 (起始偏移, run 长度)，按位置递增。
/// try_code_span 从此按「相同长度」筛选下一个 run，避免重复 find。
backtick_runs: Vec<(usize, usize)>,
```

#### 2. 预扫时登记 run（可合并到 scan_bracket_pairs 里，同一遍历）

```rust
// 遍历 self.s，遇到反引号时：
// - 如果 run 起始位置不在 bracket_pairs 跳过的范围内（反引号本身就是 bracket_pairs 里的跳过源）
//   记录 (pos, run_len) 进 backtick_runs
// - pos += run_len（已在 bracket_pairs 跳过时处理过）
```

#### 3. try_code_span 改用索引（inline.rs:569-598）

```rust
fn try_code_span(&mut self) -> Option<InlineNode> {
    let rest = &self.s[self.pos..];
    let open = rest.bytes().take_while(|&b| b == b'`').count();
    if open == 0 {
        return None;
    }
    // 从 backtick_runs 里找 pos + open 之后、长度 == open 的下一个 run
    let search_start = self.pos + open;
    let mut found: Option<usize> = None;
    for &(rp, rl) in &self.backtick_runs {
        if rp >= search_start && rl == open {
            found = Some(rp);
            break;
        }
    }
    let close_pos = found?;
    let content_end = close_pos;
    if content_end == search_start {
        return None; // 空内容
    }
    let start = self.pos;
    self.pos = close_pos + open;
    return Some(self.node(
        InlineKind::Code,
        start, start + open, content_end,
        None, Vec::new(),
    ));
}
```

注意：这样写是 O(runs)，runs 通常 ≤ 1000。如果 runs 很多，可以按长度分桶（`Vec<Vec<(usize, usize)>>`）——但当前量级足够，先不分桶。

### 测试
1. **`inline_backtick_unmatched_perf`**：512KB 含 N 个不匹配反引号 run（每个 run 长度随机 1-3），测 inline 解析 ≤ 10ms。
2. **`inline_backtick_matched_regression`**：合法代码段 `` `foo` ``、双反引号 `` `a``b` ``、多段嵌套正常解析。

### Commit 消息

```
批次2-2：inline.rs 反引号 run 索引，消除 try_code_span 的 O(n²)
- 预扫登记所有反引号 run 的位置 + 长度
- try_code_span 按长度直接查索引，不再 find('`') 重扫
- run 数通常 ≤ 1000，O(runs) ≈ O(1)
```

### 验证命令
同上批次 2-1b。

---

## 批次 2-3：table() 列/行硬上限 + 降级

### 目标
31KB 表格 → 私有内存 ≤ 20MB（从 165MB 砍下来）

### 根因
`table()`（mod.rs:191-215）对任意列数/行数都全量解析。GFM 表格 1000 列 × 1000 行 → 100 万个单元格字符串。

### 修法

#### 1. 加常量（mod.rs 顶部或 table() 上方）

```rust
const MAX_TABLE_COLS: usize = 128;   // 视图可渲染上限，超限截断
const MAX_TABLE_ROWS: usize = 512;   // 总行数上限，超限降级
```

#### 2. table() 内加截断（mod.rs:196-214）

```rust
let aligns = parse_delim_row(&delim)?;
let aligns = if aligns.len() > MAX_TABLE_COLS {
    aligns[..MAX_TABLE_COLS].to_vec()
} else { aligns };

// 表头：按 aligns.len() 截断
let header = parse_table_cells(..., aligns.len(), ...);

// 表体：行数截断 + 超限尾部降级
let total_body_rows = b.end_line.saturating_sub(b.start_line + 2);
let capped_rows = total_body_rows.min(MAX_TABLE_ROWS);
let mut rows = Vec::with_capacity(capped_rows);
for line in b.start_line + 2..(b.start_line + 2 + capped_rows) {
    rows.push(parse_table_cells(&line_text(buf, line), ...));
}
if total_body_rows > MAX_TABLE_ROWS {
    // 尾部加一行「截断提示」占位——不额外分配大字符串
    rows.push(vec![ /* 空 cells */ ]);
}
```

**关键**：截断不改变块结构（Table kind 不变），只是派生数据降级——视图侧看到的 table() 单元格少了，但不会 crash。

### 测试（core.rs）
1. **`table_large_col_degrade`**：1000 列表格 → table() 返回 aligns.len() == 128，header/rows 每行列数 ≤ 128。
2. **`table_large_row_degrade`**：2000 行表格 → table() 返回 rows.len() ≤ 513（512 + 1 截断提示），不 panic。
3. **`table_memory_assert`**：6.6KB 真实表格 → table() 调用耗时 ≤ 5ms。

### Commit 消息

```
批次2-3：table() 列/行硬上限 128×512，超限降级不崩溃
- MAX_TABLE_COLS=128，parse_delim_row 后截断 aligns
- MAX_TABLE_ROWS=512，表体循环 bounded；超限加占位行
- 3 条测试覆盖超列/超行/6.6KB 真实表格
```

### 验证命令

```bash
cargo test -p veil-core -- table
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p veil
```

---

## 批次 2-4：history 字节预算淘汰

### 目标
整篇替换 × 200（每步 1MB）→ undo 栈字节稳定 ≤ 50MB（不再 413MB RSS 爆炸）

### 根因
`record()`（history.rs:46-62）只按步数 `MAX_UNDO=1000` 裁剪。每步 Transaction 可能持有整篇文档的 `deleted` 字符串（1MB+）。

### 修法

#### 1. history.rs 加常量 + 累计字段

```rust
const MAX_UNDO_STEPS: usize = 1000;          // 保持不变
const MAX_UNDO_BYTES: usize = 16 * 1024 * 1024;  // 新增：16MB

#[derive(Default)]
pub(crate) struct History {
    undo: VecDeque<Transaction>,
    redo: VecDeque<Transaction>,
    last: Option<Group>,
    /// undo 栈累计字节数（deleted + inserted 之和）
    undo_bytes: usize,
}
```

#### 2. record() 改造（history.rs:57-60 附近）

```rust
self.undo.push_back(tx);
self.undo_bytes += tx_byte_size(&tx);

// 步数裁剪
while self.undo.len() > MAX_UNDO_STEPS {
    if let Some(old) = self.undo.pop_front() {
        self.undo_bytes = self.undo_bytes.saturating_sub(tx_byte_size(&old));
    }
}
// 字节裁剪（**单步自身超 16MB 保留**——保证完整撤销）
while self.undo.len() > 1 && self.undo_bytes > MAX_UNDO_BYTES {
    if let Some(old) = self.undo.pop_front() {
        let sz = tx_byte_size(&old);
        if self.undo_bytes - sz < self.undo_bytes {
            self.undo_bytes -= sz;
        }
        // 如果 pop 后空了，不保留超限单步（超限单步在 while 条件里 self.undo.len() > 1 会留住）
    }
}
```

`tx_byte_size` helper：
```rust
fn tx_byte_size(tx: &Transaction) -> usize {
    tx.prims.iter().map(|p| p.deleted.len() + p.inserted.len()).sum()
}
```

#### 3. pop_undo / push_undo 同步更新 undo_bytes

#### 4. 构造处 + push_back 时都要累加，pop_front 时都要减。

### 测试（history.rs 自身已有单元测试，在同一文件补）
1. **`undo_budget_bytes`**：构造 200 个 Transaction（每个含 1MB deleted），调用 record 200 次 → 断言 `tx_byte_size(所有 undo)` ≤ 50MB，且步数 ≤ 1000。
2. **`undo_max_steps_not_affected`**：连续 1001 次单字符插入 → undo.len() == 1000（保持原有步数裁剪语义）。
3. **`undo_single_oversized_step_kept`**：单个 20MB Transaction → 保留在栈里（单步超限不裁），后续正常编辑入栈后才可能被字节裁掉。

### Commit 消息

```
批次2-4：history 加 MAX_UNDO_BYTES=16MB 字节预算，整篇替换不再 RSS 爆炸
- undo_bytes 累计字段；record 在步数裁剪后加字节裁剪循环
- 单步自身超限保留（len > 1 时才进 while）
- 3 条测试 + 原 MAX_UNDO=1000 步数语义不变
```

### 验证命令

```bash
cargo test -p veil-core -- history
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p veil
```

---

## 批次 2-5：build_refs 增量 + Session 门面 + 派生 API O(1)

### 目标
- `build_refs` 在 reparse 后只重建被修改的 LinkRefDef/BlockQuote → 不再每编辑扫全文
- `Session` 暴露 `offset_of_line(line: usize) -> usize` 门面
- `syntax.inlines / heading / table / math_source` 从 `blocks.iter().find()` O(n) 改成 HashMap 查表 O(1)

### 根因
1. `build_refs()`（mod.rs:56-81）每次被 reparse 末尾调用（mod.rs:382）都扫 blocks 表 + buffer 切片全文
2. Session 缺少 `offset_of_line`，视图只能拷全文取行边界
3. 4 个派生 API（syntax mod.rs:92, 192, session.rs:145, 164, 169, 174, 179）都用 `iter().find(|b| b.id == block)` —— N 块文档每帧 O(N)

### 修法

#### 方案选择（三选一的全部实施，总改动约 150 行，低风险）：

##### 2-5a. BlockId→usize 索引（HashMap）

在 Syntax 结构上加字段：
```rust
pub struct Syntax {
    blocks: Vec<Block>,
    next_id: BlockId,
    refs: RefMap,
    /// BlockId → blocks 内索引（每次 reparse/build_refs 同步更新）
    id_index: std::collections::HashMap<BlockId, usize>,
}
```

在 `build_refs`（或其前置步骤）里同步：
```rust
fn rebuild_id_index(&mut self) {
    self.id_index.clear();
    for (i, b) in self.blocks.iter().enumerate() {
        self.id_index.insert(b.id, i);
    }
}
```

**所有** `blocks.iter().find(|b| b.id == block)` 改成：
```rust
let idx = self.id_index.get(&block)?;
let b = &self.blocks[*idx];
```

在 `reparse()` 重写 blocks 表后（mod.rs:381）、build_refs 之前（mod.rs:382）调用 `rebuild_id_index()`。

##### 2-5b. Session 门面 offset_of_line

session.rs 在现有 API 附近（L100 附近）加：
```rust
/// 行首字节偏移（Buffer 直接转发）。
pub fn offset_of_line(&self, line: usize) -> usize {
    self.buf.offset_of_line(line)
}
```

##### 2-5c. build_refs 增量（最小改动版）—— **不做**（2026-11-03 裁决）

**裁决：不做。** 理由（事实）：每击键仅 O(#blocks) 次 `matches!(kind, LinkRefDef | BlockQuote)`，只有这两类才 slice，10k 块为微秒级；增量化需维护引用定义 first-wins 全局序（前文新增覆盖后文、删除后文定义会「复活」），属语义风险而非零成本重构。**触发条件**：若未来探针显示该段 >5%，再以数据重新立项。以下为原方案，仅作存档。

`build_refs` 改成接受「脏区间」参数——reparse 已知重扫了哪些行，refs 只重建被修改区间内的 LinkRefDef + BlockQuote：

```rust
fn build_refs(&mut self, buf: &Buffer, dirty_lines: Range<usize>) {
    self.refs.clear();
    for b in &self.blocks {
        // 只处理可能受影响的块
        if b.end_line < dirty_lines.start || b.start_line > dirty_lines.end { continue; }
        // 原解析逻辑不变
        ...
    }
}
```

reparse 调用处（mod.rs:382）从 `self.build_refs(buf)` 改成 `self.build_refs(buf, reparse_dirty_lines)`。

**但注意**：如果 reparse 没有「脏区间」信息（它可能重扫了不确定行数），可以先**不改 build_refs 调用**，只改 2-5a + 2-5b。build_refs 增量作为**可选的后续优化**——本次 commit 先做 2-5a + 2-5b，build_refs 增量留待以后。

### 测试（core.rs）
1. **`derive_api_o1`**：40k 块文档（构造循环块），对最后一块调用 `block_inlines / heading / table`（各一次），断言 ≤ 2µs each。
2. **`offset_of_line_roundtrip`**：Session.offset_of_line 和 line_of_offset 互逆，含多字节字符 + CRLF。
3. **`id_index_kept_in_sync`**：reparse 增删块后，所有现存 BlockId 都能在 id_index 查到，且 blocks[id_index[id]].id == id。

### Commit 消息

```
批次2-5：build_refs 增量 + Session.offset_of_line + BlockId→index HashMap
- Syntax 加 id_index: HashMap<BlockId, usize>；reparse 末尾重建
- 4 个派生 API 从 iter().find 改 O(1) 查表
- Session 门面暴露 offset_of_line
- build_refs 暂不改调用（reparse 没有明确脏区间时先保守）
- 3 条测试覆盖 O(1) 查表性能 + offset_of_line 正确性
```

### 验证命令

```bash
cargo test -p veil-core
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p veil
```

---

## 批次 2 总验证

所有 5 个 commit 合并后：

```bash
cargo test --workspace                          # 全绿
cargo clippy --workspace --all-targets -- -D warnings  # 零警告
cargo build --release -p veil                   # 记录尺寸 ≤ 基线 5%
# CommonMark conformance 必须保持 652/652
cargo test -p veil-core --test conformance commonmark
```

### 批次 2 后快照（预期）

| 项 | 批次 1 后 | 批次 2 后（预期） |
|---|---|---|
| workspace 测试 | 831 passed / 0 failed | ≥ 845 (14+ 新测试) |
| clippy 警告 | 0 | 0 |
| release 体积 | 5,822,464 B | ≤ 6,113,587 B (+5%) |
| CommonMark 0.31.2 | 652/652 | 652/652 |
| 10k 未配对 `[` inline | ~170ms | ≤ 10ms |
| 512KB 不匹配反引号 | N/A | ≤ 10ms |
| 31KB 表格 table() | ~165MB 私有 | ≤ 20MB |
| 撤销栈字节稳定 | 爆炸（无上限） | ≤ 16MB（单步超限时） |
| 派生 API O(1) | iter().find O(n) | HashMap O(1) |

---

## 批次 3：按需触发清单（不再作为独立里程碑）

**总纲：批次 3 不再是独立里程碑，改为触发式清单。** 逐项按当前事实标注现状：已实施/已解决的只保留结论、不再排期；标注为待触发的项，仅在对应触发条件满足时才立项。

| # | 项 | 现状 | 说明 |
|---|---|---|---|
| 1 | Events 契约 | **本次实施** | 探针数据触发：索引曾占击键 p95 65%~77%（`docs/review/keystroke-attribution.md`），需扩展 `dirty_blocks` 以支持视图局部更新 |
| 2 | Block 行号→字节 | **已实施** | `Block` 持 `range: Range`，`Session::blocks()` 零 rope 换算；实测 `blocks_enum` 10.5k 块 p95 6941→196µs |
| 3 | undo 断组入口 | **已实施** | `Session::break_undo_group()`（`session.rs:81`） |
| 4 | Transaction 简化 | **已实施** | 收敛为单个 `PrimEdit`（`history.rs:14`） |
| 5 | Mark 排序不变量 | **已解决** | commit a3b7062：删除不变量 + `create_mark` 改 `push` + `MARK_VEC_SCALE_LIMIT=1000` |
| 6 | line_col | **仅改文档口径** | 列按码点计数；CRLF 计一列；emoji ZWJ 序列按码点数，显示列可能小于视觉宽度。不改内核 |

---

## 执行纪律（重申）

1. **Pre-Commit 先落盘批次 1**，inline.rs 彻底回滚后再做 2-1b
2. **每 commit 独立切片**：单 commit 改动单文件或同功能组
3. **每 commit 提交前**：cargo test --workspace + clippy + release 打包 + 记录尺寸
4. **零新增 crate 依赖**（括号索引用手写 Vec，不用 proptest 预扫依赖）
5. **属性测试固定种子**，失败时 seed 进 panic message
6. **批次 1 改动 commit 后**，批次 2-1b 起 inline.rs 先 `git checkout` 回 baseline 再做括号索引
7. **批次 2-1b 括号索引方案是核心**——如果 pair_resolves 在预扫与 try_link 实际判定间出现语义分歧，宁可回退预扫、在 try_link 内部做一次「查索引拿闭合位置 + 运行时判定」的混合方案，也不要牺牲正确性
