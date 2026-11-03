# 表格单元格编辑设计文档

> 状态：**T1~T3 已落地（2026-10-29）**，行列右键菜单 UI 留待 G2 后按真实需求补 · 设计 2026-10-29
> 阶段：阶段 1 最后一项（6.5）
> 前置依赖：0a 超大块保护 ✅ · 0c CommonMark 一致性 ✅ · G1 门禁 ✅
> 架构约束：veil-ui+veil 直接依赖 8/8 已满，零新增 crate

---

## 1. 问题与定位

### 1.1 现状

当前表格是**只读渲染**（`veil-ui/src/view/table.rs` 切片 9）：

- 点击网格 → caret 落到 `TableCell.range.start`（单元格内容起点，文档绝对偏移）
- `table_shows_grid()` 判定：caret 在表格 range 内 → **隐藏网格、显示源码**；离开 → 恢复网格
- 用户被迫切到源码态编辑表格，写完离开才看到网格重新出现

内核侧表格编辑已完备（`crates/veil-core/tests/core.rs::edit_table_cell_reparses_in_place`）：

- 单元格内容编辑走 `Edit::Insert/Delete/Replace`，reparse 只重扫该 Table block（脏区自然闭合，因为 Table 的 block range 不跨空行）
- 结构变更自动处理（删除分隔行 → 段落；插入分隔行 → 表格；列数变化 → 自动补空/截断）

### 1.2 本次范围

| 项 | 做 | 不做 |
|---|---|---|
| 单元格内容编辑 | ✅ 网格态下光标停留在活动单元格，内容直接显示在单元格内 | ❌ 富文本样式工具栏、颜色、合并/拆分单元格 |
| 单元格导航 | ✅ Tab → 下一格（行尾换行到下行首）、Shift+Tab → 上一格、Enter 换行（单元格内软换行，不跨行） | ❌ 方向键在单元格内逐字/逐词移动（v0 光标逻辑已支持，保持不变） |
| 行/列操作 | ✅ 右键行/列 → 删除行、插入行（下）、插入列（右） | ❌ 拖动列宽调整、表格整体右键菜单、对齐方式切换 UI |
| 源码编辑回退 | ✅ 所有操作均可通过源码态完成，不阻断 | — |

### 1.3 设计原则

1. **机制与策略分离**：内核只负责表格 block 的编辑与重解析，视图层负责「网格态下光标在哪个单元格」的派生状态
2. **源码真值不变**：所有单元格编辑最终走 `Session::apply(Edit)`，不存在独立于 Markdown 的表格编辑模式
3. **零架构风险**：不引入新 crate、不改 Session 门面、不新增 Edit 变体
4. **渐进落地**：先做单元格编辑 + Tab 导航，再做行列操作，每步可独立 revert

---

## 2. 架构决策

### 2.1 不新增 Mode

**决策**：`Mode` 枚举维持不变（`Document/Menu/QuickOpen/Search`）。

**理由**：表格单元格编辑是 `Mode::Document` 内的派生状态。活动单元格 = caret 落在 `Table.cells[row][col].range` 区间内。这与「Reveal 态光标落在标记内」完全同构——都是 view 层根据 caret 位置派生的局部行为，不是全局交互模式切换。

**反模式回避**：如果新造 `Mode::Table`，则文档级快捷键（如 Ctrl+S 保存、Ctrl+Z 撤销）需要路由分发，状态机爆炸。

### 2.2 网格显示条件调整

**当前逻辑**（`paint.rs::table_shows_grid`）：

```
caret 在表格 range 内 → false（显示源码）
caret 在表格 range 外 → true（显示网格）
```

**调整后**：

```
┌─ 编辑路径 A：单元格内容编辑 ─────────────────────┐
│ caret 在某个 TableCell.range 内                  │
│ → 网格继续显示（活动单元格边框高亮）               │
│ → 文本输入走既有 Session::apply 路径              │
│ → 该 TableLayout 标记为「活动」，每次 reparse 后  │
│   立即重建（因为字符数变化可能改变列宽折行）       │
└──────────────────────────────────────────────────┘
┌─ 编辑路径 B：表格结构编辑 ───────────────────────┐
│ caret 在表格 range 内但不落在任何 cell.range 内  │
│ （即管道符 `|`、分隔行 `|---|`、空单元格边框）    │
│ → 隐藏网格，显示源码（与当前行为一致）             │
└──────────────────────────────────────────────────┘
```

**实现方式**：新增 helper `table_shows_grid()` 的补充判定——不仅检查 caret 是否在 block range 内，还要检查是否落在某个 `TableCell.range` 内。

### 2.3 活动单元格的视觉呈现

**问题**：网格态下单元格的文本是光栅化的位图（`TableCellLayout.pixels`），不可编辑。

**方案**：活动单元格的文本渲染与非活动单元格**同路径但追加光标绘制**。不引入额外文本输入组件。

- 非活动单元格：既有 `pixels` 位图 → 直接 blit（零额外成本）
- 活动单元格：复用同一 `shape_cell` → `rasterize_buffer`，但在 paint 阶段额外绘制：
  1. 活动单元格边框（半透明纯色矩形 + 1px 描边，theme 派生色）
  2. 光标插入符（在 `Buffer::hit(cell_w, caret_offset)` 结果位置）
  3. 选区矩形（如果有）

**为什么不做独立 overlay 组件**：
- 活动单元格内字符变化时，列宽可能因折行变化 → 整个 TableLayout 必须重建
- 独立 overlay 需要维护一份「光栅前 Buffer」与画布坐标的同步，复杂度翻倍
- 复用既有路径，只需在 `paint.rs::paint_table` 循环中加条件分支

### 2.4 Tab 导航（单元格跳转）

**Tab → 下一格**：
1. 当前 TableLayout 的 cells 数组按行优先展开
2. 找到活动单元格在 cells 中的索引
3. 索引 + 1 → 取 `TableCell.range.start` 作为新 caret
4. 若索引 + 1 == cells.len() → 落在首行首列尾部（即 cells[0].range.end，新建行的锚点）

**Shift+Tab → 上一格**：索引 - 1，到 0 时不移动。

**Enter（软换行）**：在单元格内光标位置插入 `\n`。内核 reparse 后 TableCell.range 可能扩展，TableLayout 重建。

**锚点计算伪代码**：

```
fn cell_anchor(&self, table: &Table, row: usize, col: usize) -> usize {
    let (header, rows) = if row == 0 {
        (&table.header, &[])
    } else {
        (&[], &table.rows)
    };
    header.get(col).or_else(|| rows[row-1].get(col))?.range.start
}
```

边界处理见 §4.3。

---

## 3. 实现切片

### 切片 T1：活动单元格视觉 + 网格继续显示

**改动范围**：
- `veil-ui/src/paint.rs`：`table_shows_grid()` 新增「caret 在 cell.range 内」判定；`paint_table` 循环中活动单元格追加光标/边框绘制
- `veil-ui/src/view/table.rs`：新增 helper `is_active_cell()`

**测试**：
- headless：`caret_in_cell_range→grid_shown`（caret = `cell.range.start` → `table_shows_grid` 返回 true）
- headless：`caret_in_cell_border→hidden`（caret 在管道符位置 → 隐藏网格）

**门禁**：workspace 测试全绿；clippy 0 警告；击键 p95 不变（活动单元格重绘只是光栅化后的像素绘制，不在输入路径上）。

### 切片 T2：Tab / Shift+Tab 单元格导航

**改动范围**：
- `veil-ui/src/interaction.rs`：keypress 路由新增 Tab/Shift+Tab 分支
- 需访问当前 TableLayout 的 cells 数组，定位活动 cell 索引

**测试**：
- `tab_advances_to_next_cell`（2×3 表格，Tab × 6 依次到各 cell.range.start）
- `tab_at_last_cell_stays`（边界不越界）
- `shift_tab_goes_back`

### 切片 T3：行/列操作（右键菜单）

**改动范围**：
- `veil-ui/src/chrome/mod.rs`：表格右键菜单行（`ctx_menu_rows` 新增表内上下文）
- `veil-ui/src/interaction.rs`：右键命中单元格 → 弹出菜单
- 实现：对表格 block range 做多行 `Replace` → `Session::apply`

**列操作的 Markdown 变换（以两列表格为例）**：

```
原：
| a | b |
| :- | -: |
| c | d |

插入一列（在 col 0 后）：
| a |   | b |
| :- | :- | -: |
| c |   | d |

删除列 1：
| b |
| -: |
| d |
```

**核心逻辑**：逐行 split 按 `|` → 增删 → join，同时更新分隔行的 `:-:` 模式。

**行操作更简单**：插入行 = 在末行前 `Insert` 一行 `|   |   |\n`；删除行 = `Replace` 该行为空。

**测试**（headless，core 层）：
- `insert_row_into_table`（2×2 → 3×2，序列化 + block count + table() 行数）
- `delete_row_from_table`
- `insert_column_into_table`（2×2 → 2×3，aligns 长度匹配）
- `delete_column_from_table`
- `row_op_at_header`（在表头行执行插入/删除的边界）
- `column_op_preserves_inline`（删除列后相邻列的 inline 不受影响）

### 切片 T4：右键菜单 UI（chrome 域）

**改动范围**：
- `veil-ui/src/chrome/mod.rs`：新增表格上下文菜单（6 项：插入行/删除行、插入列/删除列、「退出源码态」）
- 命中：右键 → `hit_offset()` 得到 caret → 判定 caret 在 Table block → 弹出上下文菜单

**不做**：独立的右键菜单组件，复用既有菜单框架（`chrome::menu_rows` 单一路由）。

---

## 4. 边界与风险

### 4.1 表格结构变更后的 TableLayout 重建

活动单元格内编辑文本 → `Session::apply` → dirty blocks = [table_id] → 重解析后 Table 结构不变（行数/列数/aligns 没变）但 cell text 变了 → TableLayout 必须重建。

**失效路径**：TableLayout 的 `Memo::key` 是 `base_key()`（block_index 重建标记）。cell 内容编辑 → block_index 不变？不对——cell 内容变更不改变 block 的 top/height/range，block_index 不会重建。但 Table 的派生数据会更新（`Syntax::table()` 在 reparse 后重新计算）。

**修正方案**：把 TableLayout 的 memo key 从 block_index 标记改成「该 Table block 的 syntax_hash」——每次 reparse 后 Table 结构/文本变了 syntax_hash 就变，自然触发重建。

syntax_hash = Block 的 `end - start`（长度） + 分隔行 hash（如果结构变更必然改变分隔行，长度也会变）。最简单可靠的 key：TableLayout key = **range.start → range.end**（每次结构/内容变更，range 或内容必变）。

**实际用**：`BlockIndex::tick()` 的版本号——每次 `refresh_index`（包括 reparse 后的增量重建）版本号自增，TableLayout memo key 引用版本号即可。

### 4.2 单元格内编辑导致分隔行被覆盖

边界 case：活动单元格内光标位置越过本 cell.range.end（比如选中到管道符 `|` 上），Delete 会吃掉 `|` → 表格结构被破坏 → reparse 后退化为段落。

**处理**：不加拦截——这是既有 edit 路径的自然行为。用户删到 `|` 等同于主动拆表格，reparse 后 block kind 从 Table → Paragraph，网格自动隐藏，回到源码态。

### 4.3 空单元格边界

GFM 允许 `| |`（空单元格）。此时 `TableCell.range` 是 `|` 和 `|` 之间的空区间（start == end）。

- 网格命中：空 cell 的 `doc_start = range.start`（管道符之后）
- Tab 导航：正常跳转
- 编辑：空 cell 内 Insert 有效

### 4.4 分隔行编辑

分隔行 `| :-: | --- |` 中 `|` 和 `:` 的位置决定对齐。如果用户把分隔行当成某个 cell（比如把 `---` 中间加上文本），reparse 会把分隔行认成表体行，表格列数自动对齐。

**处理**：不加拦截，源码编辑路径自然处理。

### 4.5 已知限制（v0 接受）

1. 活动单元格内光标逐字移动不走 `Buffer::hit` 路径，只通过源码偏移近似定位——对于等宽字体没问题；对于非等宽字体（如中文 + 英文混排），光标水平位置可能有 1-2px 偏差。**接受**。
2. 单元格内折行文本的光标垂直移动按「源码行」而非「视觉折行」——需要 cosmic-text `Buffer::hit(x, y)` 做视觉行定位才能完美实现。**v1 前做**。
3. 行列操作不处理合并单元格（GFM 不支持合并）。

---

## 5. 改动清单

### veil-core（无改动，现有能力已覆盖）

- `Edit` 枚举：维持 `Insert/Delete/Replace`
- Table parse/reparse：已支持结构变更自动重对齐
- Table.serialize：已支持

### veil-ui

| 文件 | 改动 |
|---|---|
| `src/paint.rs` | `table_shows_grid()` 新增 cell.range 内判定；`paint_table` 活动单元格追加边框/光标 |
| `src/view/table.rs` | 新增 `is_active_cell()` helper；`TableLayout` memo key 改用 block_index version |
| `src/interaction.rs` | Tab/Shift+Tab 路由；右键命中后弹出表格菜单 |
| `src/chrome/mod.rs` | 表格上下文菜单项（插入/删除行列） |

### veil（无改动，平台接缝已满足）

---

## 6. 测试计划

### headless（veil-ui）

| ID | 断言 | 覆盖点 |
|---|---|---|
| T1-1 | caret = cell.range.start → `table_shows_grid` = true | 网格继续显示 |
| T1-2 | caret 在管道符上 → `table_shows_grid` = false | 源码态回退 |
| T1-3 | cell 内容 Insert → grid 重建且 activity 保持 | 活动状态不丢 |
| T2-1 | Tab × 6 遍历 2×3 → 每次 caret = 下一格 range.start | 正向导航 |
| T2-2 | 末格 Tab → caret = 首格 range.end | 边界 |
| T2-3 | Shift+Tab 首格 → caret 不变 | 边界 |
| T3-1 | 插入行 → rows.len() + 1，aligns 不变 | 行操作 |
| T3-2 | 插入列 → aligns.len() + 1，每行 cells + 1 | 列操作 |
| T3-3 | 删除最后一行 → 表格仍 ≥ 1 体行（或退化为仅表头） | 边界 |
| T3-4 | 删除中间列 → 剩余列 inline/对齐不污染 | 数据完整性 |

### core（veil-core）

| ID | 断言 |
|---|---|
| C-1 | row/col 操作后 serialize 往返一致 |
| C-2 | row/col 操作后 Table block 结构正确 |
| C-3 | 删除分隔行 → 表格退化为段落（回归，既有测试） |

### GUI（真人）

| ID | 场景 |
|---|---|
| G-1 | 点击 3×3 表格中间格 → 网格继续显示，光标在格内闪烁 |
| G-2 | 输入 "hello" → 格内显示文本，其他单元格不受影响 |
| G-3 | 连续 Tab 9 次 → 遍历完所有格 |
| G-4 | Tab 到末格后 Tab → 跳到首格尾部 |
| G-5 | 右键列头 → 插入列 → 表格变 4 列，对齐正确 |

---

## 7. 非目标

1. **合并单元格**（GFM 本身不支持）
2. **列宽拖动调整**（Markdown 无此语义，硬保存会破坏源码）
3. **表格整体样式切换**（主题统一处理）
4. **工具栏**（v1 所有编辑通过键盘/右键/菜单完成）
5. **表格属性面板**（对齐/边框线宽等无意义，Markdown 不持久化）
