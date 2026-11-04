# veil-core 地基修复计划

> **批准前严禁动代码**。本计划覆盖对抗性审查四视角（正确性 / 增量解析 / 架构哲学 / 健壮性）定位出的全部 P0/P1 问题，按优先级分批，每批独立 commit + 测试 + clippy + release 打包复验。

## 基线快照（修复前）

| 项 | 值 |
|---|---|
| `cargo test --workspace` | 829 passed / 0 failed / 10 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 警告 |
| `target\release\veil.exe` | 5,822,464 B |
| veil-core 源文件行数 | buffer 254 / history 262 / range 44 / session 249 / block 2658 / inline 1347 / mod 863 / model 124 |
| veil-core 测试文件行数 | core 1155 / conformance 259 / gfm 202 / pathological 96 / real_world 132 / common 531 |

## 交付目标（修复后判定）

1. **0 条可达 panic**（任意合法输入下 Session 门面永不 panic；已知两个 panic 修掉并加属性测试证明）。
2. **增量 == 全量**（属性测试：对任意随机编辑序列，`Session::open(serialize())` 的块结构 + inline + math + table == 连续 apply 后的派生结果）。
3. **性能预算不变**：p95 编辑 ≤16ms（10k 块长文连续 typing）；私有工作集 ≤150MB（含表格与撤销栈）；release 体积变化 ≤5%。
4. **新增测试量**：≥40 条（每条有明确的反例来源）；veil-core 单元测试总数从 829 中 core 子集的 200+ 提升到 ≥280。
5. **全量测试 + clippy + release 打包** 与修复前口径一致。

---

## 批次 1：阻断级崩溃与正确性（3 个 P0，4 个 commit）

### Commit 1-1：math_source 长度守卫
- **文件**：`crates/veil-core/src/syntax/mod.rs:174-178`
- **现状**：单行数学块 `t[2..t.len()-2]` 在 `t.len() == 4`（即 trim 后只含 `$$`）时下溢 panic；`math_marker` 已保证 `t.len() > 4` 才回 Some(true)，但 trim 后若首尾空白把内容吃光（`"  $$  "` 单行 trim 后仍为 `"$$"`），仍会走到该分支。
- **修法**：
  ```rust
  if b.end_line == b.start_line + 1 {
      let t = text.trim();
      if t.len() > 4 && t.starts_with("$$") && t.ends_with("$$") {
          return Some(t[2..t.len() - 2].to_string());
      }
      return None; // 单行形态不成立，回退（view 侧 math_source 已用 Option）
  }
  ```
- **测试**（`core.rs`）：
  - `math_source_single_line_empty_markers`: `Session::open("  $$  \n")` → 单行 Math 块，`math_source(id) == None`。
  - `math_source_single_line_short`: 任何 trim 后长度 ≤4 的单行数学 → 返回 None 而非 panic。

### Commit 1-2：HTML 属性循环边界
- **文件**：`crates/veil-core/src/syntax/block.rs:1254-1300`（`html_start_blank_line` 属性循环）
- **现状**：`while i < ab.len() { ... ab[i] ... }` 循环跳空白后若 `i >= ab.len()` 仍执行 `ab[i]`（属性名判断），索引越界。最小反例：`"<span "`（末尾空格，无闭合）→ panic。
- **修法**：循环体内**每次访问 `ab[i]` 前**做 `i >= ab.len()` 退出；同时允许「整行只含空白」合法（`trim().is_empty()`）——这是 CommonMark 类型 6/7 HTML 块的正常结尾。
- **测试**（`core.rs`，在 block inline tests 组）：
  - `html_type6_only_spaces`: `"<span \n\n"`（第二块起的块级标签只含空白属性名）→ 不 panic、解析为段落（因为不是合法类型 6）。
  - `html_type7_trailing_spaces`: `"<span \n"`（无 `>` 闭合）→ 不 panic、块级标签仍按非 block 处理。
  - 现有 `html_block_type6` 类用例必须继续通过。

### Commit 1-3：reparse 回退条件修正 + 属性测试（P0 结构正确性）
- **文件**：`crates/veil-core/src/syntax/mod.rs:259-288`
- **现状**：回退条件 `edit_start_line < blocks[bi].start_line` 漏掉「编辑起点正好是块首行」这一族。编辑落在 `ListItem` / `BlockQuote` / 普通段落的首行时，前一块可能向下生长合并进来（删列表标记、删引用标记、加首行缩进），当前逻辑**不回退、直接从编辑点开始重扫**，看到的是编辑后的新段落，于是把「本该合并/降级的旧前块」和「新段落」拆成两块。
- **修法**：把回退条件改为：
  ```rust
  let mut bi = self.blocks.iter().position(|b| edit_start_line < b.end_line).unwrap_or(self.blocks.len());
  if bi == self.blocks.len() {
      bi = bi.saturating_sub(1);
  } else if edit_start_line <= self.blocks[bi].start_line && bi > 0 {
      // 新条件：编辑点落在（或紧邻）当前块首行时也回退一块
      bi -= 1;
  }
  ```
  列表区域回走逻辑（`bi` 前后同为 ListItem + 间隔全空行）保持不变，但**也要对新的 `bi` 值重新判定**（`while` 循环起始条件不变，会自动回退到列表区域起点）。
- **测试（关键 — 先写后改）**：
  1. 在 `syntax/mod.rs` 加私有属性测试 `reparse_idempotent`：任意随机编辑序列（1–30 行、含块首行起点），连续 apply 后的块结构 == `Session::open(serialize())` 结果。用 `proptest` 或手写 6 个最小反例直接 assert（测试框架不增依赖）。
  2. 在 `core.rs` 固化 6 个反例（对抗性审查 agent 给的）：
     - 删列表标记：`"[r]\n- [ ] t\nb    c\n"` 删 `5..10` → 应合并为 Paragraph。
     - 给段落首行加缩进：`"- a\n\nb\n"` 在 5 插 `"  "` → 应合并为 ListItem。
     - 删引用标记首行：`"> - z\n<!-- c -->\n[r]: /u\n"` 删 `7..8` → BlockQuote 与下一段合并。
     - 围栏代码降级（删开标记首行 ` ``` ` 的一个 backtick）。
     - Setext 下划线行被正文覆盖。
     - 全文首行起点编辑。
  3. 现有 7 个 reparse 边界测试**全部保持通过**（历史上钉过的路径不变）。

### Commit 1-4：veil-egui 侧联动回归 + 集成验证
- 说明：Commit 1-1 到 1-3 内核行为变化（reparse 回退条件变宽 → 块结构会变），veil-egui 现有单元测试可能因块序列不同而失败。该 commit 只改**断言 / 快照 / 偏移常量**，不改逻辑；若发现 veil-egui 自身逻辑依赖「错误的块结构」，则另立修复。
- **验证**：`cargo test --workspace` 全绿；`cargo clippy --workspace --all-targets -- -D warnings` 零警告；release 打包尺寸记录；手动启动 veil.exe 走查 B8/B9/C1 回归。

---

## 批次 2：健壮性与性能悬崖（5 个 P1，5 个 commit）

所有 O(n²) 和内存超预算问题均有实测数据，修法以「单遍算法 / 预算上限 / 降级」为原则，**不引入新依赖**。

### Commit 2-1：inline.rs 未配对 `[` 的 O(n²)
- **位置**：`crates/veil-core/src/syntax/inline.rs:655-726`
- **现状**：未闭合 `[` 从当前位置重新扫描到块尾，再回溯找配对。实测 40KB 段落 1043ms。
- **修法**：单遍栈扫描。维护「可能未闭合的 `[` 起始位置栈」；遇到 `]` 时弹栈内最后一个 `[` 配对；扫描结束栈里剩余的视为孤立，统一按裸文本返回。保持 CommonMark 语义（若栈底的 `[` 已被其它 `[` 嵌住，仍按最内层匹配）。
- **测试**：`inline_unpaired_brackets_perf`（性能断言：40KB 含 N 个未配对 `[` 的段落，inline 解析 ≤ 5ms）；`inline_paired_regression`（合法链接 `[a](b)`、图片 `![a](b)`、嵌套链接 `[[a](b)](c)` 全部不变）。

### Commit 2-2：inline.rs 反引号 run 无匹配 O(n²)
- **位置**：`crates/veil-core/src/syntax/inline.rs:562-590`
- **现状**：反引号匹配从首字符重新扫到 EOF。实测 2MB 退化 926ms。
- **修法**：单遍 run 索引——扫描时为每个 run（含起始反引号数）登记位置，按 run 数从近到远匹配，无需「从头再扫」。run 数通常 ≤ 1000，开销 O(runs)。
- **测试**：`inline_backtick_unmatched_perf`（512KB 含不匹配反引号 run，inline ≤ 10ms）；`inline_backtick_matched_regression`（合法代码 span 与双反引号代码全部不变）。

### Commit 2-3：table() 加列/单元上限 + 视口切片
- **位置**：`crates/veil-core/src/syntax/mod.rs:191-209`
- **现状**：31KB 表格 → 私有内存 165MB（超 150MB 预算）。
- **修法**：
  - 列数硬上限 `MAX_TABLE_COLS = 128`（CommonMark 无上限，但视图可渲染 ≤128 列；超限截断并返回 warning 标志）；
  - 总行数硬上限 `MAX_TABLE_ROWS = 512`；超限返回降级表（仅头部 + 末尾 + 截断提示文本），**不崩溃**；
  - 视图侧若只读渲染（统计/大纲），可传 `Viewport{start_row, end_row}` 让派生 API 按视口切片——这步是可选的，内核先提供裸上限即可解决内存问题。
- **测试**：`table_large_col_degrade`（1000 列 → 返回前 128 列 + warning）；`table_large_row_degrade`（2000 行 → 返回前 512 行 + 末尾提示）；`table_memory_assert`（6.6KB 表格 ≤ 22ms + 私有 ≤ 5MB）。

### Commit 2-4：history 字节预算淘汰
- **位置**：`crates/veil-core/src/history.rs`
- **现状**：仅按步数限 1000，每步可能含整篇文档（1MB×200 = +413MB RSS）。
- **修法**：把 `MAX_UNDO = 1000` 扩展为：
  ```rust
  const MAX_UNDO_STEPS: usize = 1000;
  const MAX_UNDO_BYTES: usize = 16 * 1024 * 1024; // 16MB，约 5000 步普通编辑
  ```
  `record` 在 `push_back` 后先 pop_front 裁剪步数，再累计字节、裁剪最旧步直到 ≤16MB。若单步自身超 16MB（整篇替换），保留该步（保证完整撤销）。
- **测试**：`undo_budget_bytes`（1MB 全文替换 ×200 → 内存稳定 ≤50MB，步数 ≤ 实际可达的最大步数）；`undo_max_steps_not_affected`（连续单字符插入仍保留 1000 步）。

### Commit 2-5：build_refs 增量 + Session.offset_of_line + id→index 映射
- **位置**：`syntax/mod.rs:379`（`reparse` 末尾全量 `build_refs`）、`syntax/mod.rs:92/163/192/221`（派生 API 线性 `iter().find`）、`session.rs`（缺 offset_of_line）。
- **现状**：每次编辑 `build_refs` 扫全文；`blocks()` 每次全量枚举；派生 API 每帧 O(n) 找 id。
- **修法（三选一，按影响面最小原则）**：
  1. ~~`build_refs` 增「脏区」参数——reparse 已知重扫区间，refs 只重建被修改行的 LinkRefDef，其余沿用旧值。~~ **不做**（2026-11-03 裁决）：每击键仅 O(#blocks) 次 `matches!(kind, LinkRefDef | BlockQuote)`，只有这两类才 slice，10k 块为微秒级；增量化需维护引用定义 first-wins 全局序（前文新增覆盖后文、删除后文定义会「复活」），属语义风险而非零成本重构。触发条件：若未来探针显示该段 >5%，再以数据重新立项。
  2. `syntax.blocks` 维护 `BlockId -> usize` 的 HashMap（每次 reparse 同步更新），派生 API 改为 O(1) 查表。
  3. `Session` 门面暴露 `offset_of_line(line: usize) -> usize`，内部直接转发 rope。
- **测试**：`refs_incremental_unchanged_blocks`（改非 LinkRefDef 段落，未定义链接的 inline 节点不变）；`derive_api_o1`（40k 块末块 `block_inlines` ≤ 2µs）；`offset_of_line_roundtrip`（与 `line_of_offset` 互逆，含多字节 + CRLF）。

---

## 批次 3：按需触发清单（不再作为独立里程碑）

**总纲：批次 3 不再是独立里程碑，改为触发式清单。** 逐项按当前事实标注现状：已实施/已解决的只保留结论、不再排期；标注为待触发的项，仅在对应触发条件满足时才立项。

这些项**不影响正确性/不崩溃**，属「地基的理想形态」。下表「处置（2026-11-03）」列是当前事实，不再是待办排期：

| 项 | 位置 | 原现状 | 理想形态 | 处置（2026-11-03） |
|---|---|---|---|---|
| Events 契约 | `session.rs:29` | `dirty_blocks: Vec<BlockId>` 未表达删除块 / 块合并拆分 | 增 `removed: Vec<BlockId>` + `replaced: Vec<(BlockId,Vec<BlockId>)>`，或用块表代次 `generation` | **本次实施**（探针数据触发：索引曾占击键 p95 65%~77%），用于视图局部更新 |
| Block 行号→字节双轨 | `syntax/model.rs:86-92` + `session::blocks` | Block 存 `start_line/end_line`，门面换字节 Range | Block 持字节 Range（reparse 平移时维护），行号按需派生 | **已实施**：`Block` 持 `range: Range`，`Session::blocks()` 零 rope 换算；实测 `blocks_enum` 10.5k 块 p95 6941→196µs |
| Session 缺 offset_of_line | `session.rs` | 视图为取行边界只能 `text_slice(0,len)` 拷全文 | 门面薄封装 rope | **已实施** |
| undo 策略写死 | `history.rs:46-62,119-132` | 分组规则内核硬编码，无断组入口 | `Session::break_undo_group()` + `apply` 接受 `break_group` 选项 | **已实施**（`Session::break_undo_group`） |
| Transaction 恒单元素 | `history.rs:20-24` | Vec<PrimEdit> 恒为 1，合并只读 `[0]` | 收为 `PrimEdit` 单结构体；或承认多原语事务的价值并让视图能利用 | **已实施**（收敛为单个 `PrimEdit`） |
| Mark 排序不变量失效 | `buffer.rs:10,57-60` | transform_marks 后 marks 可能不再有序 | 要么每次变换后 `marks.sort_by_key(|m| (m.offset, m.id))`（O(n log n)，n≤100 无影响），要么让 create_mark 用线性插入替代 partition_point | **已解决**（commit a3b7062：删不变量 + `create_mark` 改 `push` + `MARK_VEC_SCALE_LIMIT=1000`） |
| line_col 语义与 VS Code 不一致 | `session.rs:107-116` | 列按码点、CRLF 计一列、组合字符/ZWJ 给误导值 | 改按字素计数 + 视图统一按字节偏移列位置（编辑内核返回字节偏移即够，列数是视图层概念） | **仅改文档口径**：列按码点计数；CRLF 计一列；emoji ZWJ 序列按码点数，显示列可能小于视觉宽度。不改内核 |

---

## 执行纪律（必须遵守）

1. **单批次内独立 commit**；批次 1 和 2 之间必须跑一次完整的 veil-egui 回归（Commit 1-4 已经做了）。
2. **每 commit 提交前**：`cargo test --workspace` 全绿 + `cargo clippy --workspace --all-targets -- -D warnings` 零警告 + release 打包 + 记录 `target\release\veil.exe` 尺寸。
3. **新增测试必须能独立复现缺陷**：每条新测试在**修复前**单独跑应失败（证明它钉住了 bug）；**修复后**跑应通过。
4. **属性测试**（批次 1-3）必须带随机种子可复现：固定 `rand::rng(seed)`，失败时 seed 写进 panic message。
5. **走查期间不推进修复**（用户既定规则）；批次 1-2 是地基类问题，允许在走查期间独立修（因为崩溃/结构缺陷是阻断所有后续工作的前提），但批次 3 必须等走查收官后。
6. **零新增 crate 依赖**（proptest 用手写 rand 实现，或用 `std::rand` 等价品，不引入第三方 crate）。

---

## 风险与降级策略

| 风险 | 概率 | 降级 |
|---|---|---|
| Commit 1-3 回退条件变宽导致「该拆的块被合并」 | 中 | 属性测试会暴露；若发现合法反例，只把回退条件缩窄到当前已知的 6 类（ListItem / BlockQuote / Paragraph 首行），而非无限放宽 |
| 批次 2 算法替换导致性能退化（最坏情况） | 低 | 性能断言每条 ≤10ms；若超过，先加缓存（如 inline.rs 给已解析 run 存位置）再回退到旧实现 |
| table() 列/行上限导致合法大表格被截断 | 中 | 超限后**返回降级数据但仍暴露完整行数给视图**（用 `rows.len() == MAX_TABLE_ROWS` + `table.capped = true`），视图显示「表格过大，仅显示前 N 行」并允许滚动加载 |
| 字节预算淘汰导致长撤销链被意外截断 | 低 | 单步超预算不裁剪；并在 `History::record` 的裁剪路径加 debug_assert 保证步数 ≥ 1 |
| veil-egui 大量用例因块结构变化而失败 | 中 | Commit 1-4 单独拎出；若涉及逻辑修改（非常量/断言），则列为独立修复 |
| build_refs 增量导致定义位置变化后 inline 不刷新 | 低 | refs 变化时脏块**强制**包含所有含链接的段落（不管 reparse 是否触及），或用 generation 版本号让依赖段落知道 refs 变了 |

---

## 验证矩阵

| 阶段 | 命令 | 期望 |
|---|---|---|
| 每 commit 结束 | `cargo test -p veil-core` | 新增测试独立复现缺陷并通过；原有 200+ 测试零回归 |
| 每批次结束 | `cargo test --workspace` | 829 + 新增 ≥40 全绿 |
| 每批次结束 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 警告 |
| 批次 1 + 2 完成后 | `cargo build --release -p veil` + 记录尺寸 | 变化 ≤ 5%（5.82MB ± 0.3MB） |
| 批次 1 完成后 | 手动启动 veil.exe | 不崩溃、B8/B9/C1 正确 |
| 批次 1-2 完成后 | 冷启动计时（main→首帧） | ≤ 190ms（比修复前略有下降，因为 reparse/build_refs 做了增量） |
| 批次 1-2 完成后 | 10k 块长文连打 30 秒 | p95 ≤ 16ms |
