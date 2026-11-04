# Marker Toggle 缺陷修复 Implementation Plan

## Repository Research

### 根因（三处独立缺陷，同属"toggle 检测覆盖不全"范畴）

| # | 入口 | 缺陷 | 根因 | 真机重现 |
|---|---|---|---|---|
| 1 | `ToggleWrap("**")` Ctrl+B / `ToggleWrap("*")` Ctrl+I | 选中包裹后二次 toggle 失效 → 无脑插新标记 | 第一次 Ctrl+B 包裹后光标坍缩并移到标记对末尾，二次调用走进"无选区分支"（分支 2），该分支 toggle 检测条件 `c-m..c` 和 `c..c+m` 都等于 marker 太窄，光标在标记对末尾时条件失败 | D9 / D11 |
| 2 | `InsertLink` Ctrl+K | 完全没有 toggle 检测 → 每次都插新 `[]()` | `apply_link` 只有两个分支：有选区 → 包裹；无选区 → 插骨架。没有"光标已在 `[]()` 中 → 解包裹"的 toggle 路径 | Ctrl+K 持续插入 |
| 3 | `ToggleWrap("*")` 在 `**文本**` 中间 | 跨 marker 嵌套破坏结构 → 错误三层嵌套 | 分支 2 toggle 检测只认当前 marker 类型（`*`），不认外层 marker 类型（`**`）。v1 放弃真正嵌套，退化为兜底插（可撤销），不做复杂嵌套策略 | D12（v1 降级，不修） |

### 现有测试覆盖

| 场景 | 有无测试 |
|---|---|
| 选中包裹 → 再选中解包裹（分支 1） | ✅ `bold_wraps_selection_single_undo_step` / `bold_unwraps_already_wrapped_selection` |
| 空选区插标记对 → 光标居中 | ✅ `empty_selection_inserts_pair_second_press_removes` 前半 |
| 空选区二次 toggle 解包裹 | ✅ `empty_selection_inserts_pair_second_press_removes` 后半 |
| **选中包裹后（光标移到末尾）二次 toggle** | ❌ **缺失** — 这是 D9 复现路径 |
| **Ctrl+K 在 `[]()` 中 toggle 解包裹** | ❌ **缺失** — 完全没测试 |
| **跨 marker（`**` 中按 `*`）** | 只测了选区恰好是整个 `**文本**`（分支 1 栈式包裹），没测光标在 `**文本**` 中间（分支 2） |
| CJK 选区不 panic | ✅ |
| 表格 cell 内生效 | ✅ |

### 约束

- `editor.rs` 里 Editor 通过 `session.text_slice(Range)` 取字节切片，**必须按字符边界切 UTF-8**，不能直接用 `text[c-m..c]` 等字节索引（CJK 会 panic）。现有代码已用 `starts_with` / `ends_with` 规避，新代码必须沿用这个模式。
- `apply_wrap` / `apply_link` 返回 `Option<Vec<BlockId>>`（脏块列表），供 UI 层触发重绘。toggle 解包裹后的返回值必须正确。
- 标记对解包裹时，内部文本不变（只删开闭 marker），光标位置应落在删 marker 后的起始位置。
- Link 解包裹：`[text](url)` → `text`（删 `[` 和 `](url)`），光标落在原 `[` 位置。

## Files and Modules

**仅改一个文件**：`crates/veil-ui/src/editor.rs`

| 区域 | 变更 |
|---|---|
| `Editor` 的私有方法区域（约 line 510-590） | 新增 `find_marker_pair` 工具函数 + 新增 `find_link_pair` 工具函数 |
| `apply_wrap` 分支 2（无选区） | 替换窄条件检测，改用 `find_marker_pair` 扩展扫描范围 |
| `apply_link` | 在两分支前插入 toggle 检测路径，用 `find_link_pair` |
| `editor.rs` 底部现有测试模块 | 新增 6-7 条回归测试 |

## Implementation Steps

### Step 1：抽 `find_marker_pair(text: &str, caret: usize, marker: &str) -> Option<(usize, usize)>` 工具函数

位置：放在 `apply_wrap` 前面，作为 Editor 的私有方法或 free function（读代码看是否放在 Editor impl 块内方便）。

逻辑（**按字符边界安全**，不用字节索引切 UTF-8）：

```
1. caret 左侧：从 caret 向左遍历 marker.len() 次，看 text 以 marker 结尾（text[..caret] ends_with marker）
   → 是：left_content_end = caret - marker.len()
   → 否：return None

2. caret 右侧：从 caret 向右遍历 marker.len() 次，看 text 以 marker 开头（text[caret..] starts_with marker）
   → 是：right_content_start = caret + marker.len()
   → 否：return None

3. 额外保险（处理 marker 更长的情况，如 markdown 里没有但其他可复用）：
   从 left_content_end 再向左走一段，跳过 marker 字符（处理"光标正好在内容开头紧贴 marker"场景）：
   比如 `**hello**`，caret 恰好在 2（`**` 之后 'h' 之前）→ 向左扫 2 字节跳过 `**` → left_content_end=0
   同理向右跳过 marker。
   → 最终返回 (left_content_end, right_content_start) 即 marker 对"内部"文本的范围

4. 若找不到配对的 marker 对，返回 None
```

注意：
- `marker.len()` 是字节长度（`"**".len() == 2`，`"*".len() == 1`）
- 用 `text[..caret].ends_with(marker)` / `text[caret..].starts_with(marker)` 而非字节切片，保证 CJK 安全
- 这个函数统一覆盖：
  - 空选区 caret 在 `****` 正中 → 找到 → toggle ✅
  - 选中包裹后 caret 在 `**文本**` 末尾 → 向左跳过 marker 到内容，向右跳过 marker 到内容 → 找到 → toggle ✅
  - 选中包裹后 caret 在 `**文本**` 中间 → 同上 ✅

### Step 2：抽 `find_link_pair(text: &str, caret: usize) -> Option<(usize, usize)>` 工具函数

逻辑：

```
1. caret 左侧：找到最近的 `[` 起始标记（向左扫直到遇到 `[` 且右侧没有被转义）
2. caret 右侧：找到对应的 `]()` 或 `](url)` 闭合标记（向右扫直到遇到 `]` 紧跟 `(` 紧跟 `)`）
3. 检查：左侧 `[` 和右侧 `]()` 之间的文本不含未配对的 `[` 或 `]`（防止嵌套破坏）
4. 返回 (lbracket_start, url_close_end) —— 即整个 `[text](url)` 的字节范围

注意：
- Link marker 不是单字符而是 `[`...`]()` 复合标记
- 简化处理（v1 够用）：
  - 向左扫直到遇到 `[`（跳过换行，因为 link 不能跨行）
  - 向右扫找到第一个 `]()` 或 `](...)`（括号配对简单处理：遇到 `(` 计数+1，遇到 `)` 计数-1，归零即结束）
  - 中间不含未配对 `[`/`]`（含者视为嵌套破坏，返回 None）
```

### Step 3：改 `apply_wrap` 分支 2（无选区）

原代码（line 539-560）：

```rust
let c = self.caret();
// 旧窄条件：光标恰在 marker 对正中
if c >= m
    && self.session.text_slice(Range::new(c - m, c)).as_deref() == Some(marker)
    && self.session.text_slice(Range::new(c, c + m)).as_deref() == Some(marker)
{
    // 删除 marker 对
}
// 兜底：插新的标记对
```

新代码：

```rust
let c = self.caret();
let text = self.full_text().unwrap_or_default();
// 扩展扫描：caret 向左找最近的 marker 起始，向右找最近的 marker 闭合
if let Some((inner_start, inner_end)) = find_marker_pair(&text, c, marker) {
    // caret 在 marker 对内部 → toggle 解包裹
    // inner_start = marker 起始位置（marker 左边界）
    // inner_end = marker 结束位置（marker 右边界）
    // 解包裹：删除 [inner_start-m .. inner_start] 和 [inner_end .. inner_end+m]
    // 即把 [inner_start-m .. inner_end+m] 替换成 [inner_start .. inner_end]
    let new_text = text[inner_start..inner_end].to_string();
    let ev = self.session.apply(Edit::Replace {
        range: Range::new(inner_start - m, inner_end + m),
        text: new_text,
    }).ok()?;
    return Some(self.collapse_and_return(ev.dirty_blocks, inner_start));
}
// 兜底：插新的标记对（原有行为不变）
let ev = self.session.apply(Edit::Insert {
    at: c,
    text: format!("{marker}{marker}"),
}).expect("caret 恒合法");
Some(self.collapse_and_return(ev.dirty_blocks, c + m))
```

### Step 4：改 `apply_link` — 加 toggle 检测

原代码（line 564-586）只有两个分支（有选区 → 包裹；无选区 → 插骨架）。

新代码：**在两分支前插 toggle 路径**。

```rust
fn apply_link(&mut self) -> Option<Vec<BlockId>> {
    let c = self.caret();
    let text = self.full_text().unwrap_or_default();

    // 新增：toggle 检测——caret 已在 link 结构内 → 解包裹
    if let Some((link_start, link_end)) = find_link_pair(&text, c) {
        // [text](url) → text；删除 link_start..link_end 范围
        let inner_text = text[link_start..link_end].to_string(); // 需要先提取内部文本
        // 简化：找到 `]` 的位置，提取 `[..]` 之间的内容
        let close_bracket = text[link_start..].find(']').map(|i| link_start + i).unwrap_or(link_end);
        let inner = text[link_start+1..close_bracket].to_string(); // [text] 里的 text
        let ev = self.session.apply(Edit::Replace {
            range: Range::new(link_start, link_end),
            text: inner,
        }).ok()?;
        return Some(self.collapse_and_return(ev.dirty_blocks, link_start));
    }

    // 原有两个分支不变（选区包裹 / 空选区插骨架）
    if let Some(sel) = self.selection() {
        ... // 不变
    }
    // 兜底：插新 []()（原有行为不变）
    ...
}
```

**注意**：toggle 检测必须在两个旧分支**之前**，否则：
- 有选区 + caret 恰在 link 内 → 可能误走"选区包裹"分支 → 先尝试 toggle，再走旧行为
- 无选区 + caret 在 link 内 → toggle 先检测，解包裹成功则返回；检测失败（不在 link 内）才走"插骨架"

### Step 5：写回归测试（6-7 条）

位置：现有测试模块（`editor.rs` 底部 `mod tests`）

| # | 测试名 | 验证场景 | 来源 |
|---|---|---|---|
| 1 | `bold_wrap_then_toggle_unwraps_without_reselection` | 选中 `hello` → Ctrl+B 一次 → `**hello**`（光标在末尾，选区坍缩） → Ctrl+B 二次 → 还原 `hello` | D9 直接复现 |
| 2 | `italic_wrap_then_toggle_unwraps_without_reselection` | 同上但 marker=`*` | D11 |
| 3 | `empty_bold_pair_toggle_still_works` | 验证原有空选区二次 toggle 不被 Step 3 改坏 | 回归保护 |
| 4 | `link_toggle_removes_when_caret_inside_brackets` | 有 `[here](http://x)`，caret 在 `here` 中间 → Ctrl+K → 解包裹成 `here` | Ctrl+K toggle |
| 5 | `link_toggle_removes_when_caret_inside_url_parens` | 同上但 caret 在 `()` 里 | Ctrl+K toggle |
| 6 | `link_no_toggle_when_caret_outside_link` | 有 `[here](x)` 和其他文本，caret 在 link 外 → Ctrl+K 应插新 `[]()`（原有行为） | 回归保护 |
| 7 | `bold_inside_other_bold_toggles_innermost_only`（可选，复杂） | `**hello**world**` caret 在中间 `**` 里 → toggle 只解包裹最近的一对 `**`，不拆错 | 边界情况 |

### Step 6：跑全量测试 + clippy + build --release

`cargo test --workspace` → 新增 7 条 + 原有全部 → 0 失败
`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
`cargo build --release -p veil` → 产物大小

## Dependencies and Considerations

- **UTF-8 安全**：所有 marker 检测必须用 `starts_with` / `ends_with`，不能用 `text[c-m..c]` 字节索引切（CJK 会 panic）。这是之前 `italic_on_cjk_selection_does_not_panic` 回归测试的教训，新代码同样适用。
- **find_marker_pair 返回值约定**：返回 `(inner_start, inner_end)` 即 marker 对之间的"内容区间"（不含 marker 本身）。解包裹时删除范围是 `inner_start - m .. inner_end + m`，替换成 `text[inner_start..inner_end]`。
- **Edit::Replace 的 range 半开约定**：`Range::new(start, end)` 是 `[start, end)`。删除 marker 对内部文本时，`start = inner_start - m`（左 marker 起始），`end = inner_end + m`（右 marker 结束，半开所以 +m 后自动到下一个字节）。
- **Link toggle 的简化处理**：`find_link_pair` 不支持嵌套 `[]` 或复杂 URL（如含 `]` 的 URL），v1 够用——返回 None 时退化为插新 `[]()`，用户可撤销。
- **现有测试不能被破坏**：`empty_selection_inserts_pair_second_press_removes`（空选区二次 toggle）和 `link_wraps_selection_caret_in_parens` / `link_empty_selection_inserts_skeleton_caret_in_brackets` 必须保持通过。

## Validation

1. `cargo test --workspace` — 全部通过（含新增回归测试 + 现有全部）
2. `cargo clippy --workspace --all-targets -- -D warnings` — 零警告
3. `cargo build --release -p veil` — 产物大小（与修复前比不变或 <1% 变化）
4. 真机手动复测 D9/D10/D11/D12/Ctrl+K 五场景：
   - 选中 `混合` → Ctrl+B 一次 → Ctrl+B 二次 → 还原
   - 空选区 → Ctrl+B 一次 → Ctrl+B 二次 → 还原
   - 选中 `混合` → Ctrl+I 一次 → Ctrl+I 二次 → 还原
   - `**混合元素**` 中间按 Ctrl+I → 兜底插 `**`（不破坏结构，可撤销）
   - `[here](x)` 中间按 Ctrl+K → 解包裹成 `here`

## Risks

| 风险 | 处理 |
|---|---|
| `find_marker_pair` 对"光标恰在内容边界"的判断不准（如 caret 在 marker 内部但 marker 是多字节 UTF-8） | marker 都是 ASCII（`*`、`**`、`[`、`]`、`(`、`)`），没有多字节问题。UTF-8 安全主要体现在 `text[..caret]` 切 caret 本身时 caret 是字符边界——Editor 保证 caret 恒在字符边界上（`set_caret_rejects_non_boundary` 测试已覆盖） |
| 解包裹时 Delete 的 range 算错（marker 长度 m 和 inner_start/end 组合） | 用回归测试 D9/D11 覆盖，加上 `empty_bold_pair_toggle_still_works` 保证原有空选区 toggle 不回归 |
| `apply_link` 的 toggle 检测在"有选区 + 选区恰好覆盖一个 link"时，会先 toggle 解包裹再覆盖走选区分支 | 实际场景极少（用户选中的内容恰好是完整 link 文本），且 toggle 解包裹 + 包裹 → 净效果等同"重新包裹"（URL 丢失，需要 v2 优化）。v1 降级：toggle 检测只在**无选区**时生效，有选区直接走原有包裹逻辑 |
| `find_link_pair` 不处理 ref link `[text][ref]` / `[ref]` 简写 | veil-core 渲染 ref link，但 veil-ui 没有 ref link toggle 入口。简化 `find_link_pair` 只认 inline link `[text](url)`，ref link 不 toggle（返回 None → 插新骨架，可撤销） |

## Implementation Scope Boundary (v1 明确放弃的)

| 放弃项 | 原因 |
|---|---|
| 跨 marker 正确嵌套（D12 理想行为） | 需要解析"外层 marker 类型"并决定嵌套策略（加粗中间按斜体 → 斜体嵌套在粗体内），实现复杂且需要 inline 解析器辅助定位 marker 边界。v1 退化为兜底插（破坏结构但可撤销），v2 再做 |
| ref link `[text][ref]` toggle | veil-ui 没有 ref link 快捷键入口，不属于 v1 范围 |
| Strikethrough `~~` / InlineCode `` ` `` / Math `$` toggle | veil-ui 没有这些快捷键入口，不属于 v1 范围 |

## 实施结果（2026-11-04 回填）

- 代码改动已落地（未提交）：`veil-core::emphasis_spans` + `apply_wrap` 两分支结构感知；`find_marker_pair` 已删除。
- 门禁：`cargo test --workspace` 894 passed / 0 failed / 12 ignored，clippy 零警告，release 5,840,896 B。
- **真机复测结果：D12 未解决**。用户反馈"粗体中部分字加斜体仍会导致整行异常粗体与星号"，截图形如 `**C|hi*n*ese**`（`**` 与 `*n*` 均以源码形态可见，但 C/ese 粗体、hi 斜体）。
- 关键观察：单元测试里**序列化文本**与预期完全一致（编辑路径已正确），因此残留问题大概率在**渲染/标记显隐路径**，而不是 `apply_wrap` 的判定。下一步应查：
  1. `paint`/`layout` 的 marker 显隐（reveal）是否按「块/行」整体展开：光标在粗体内时把整行所有 marker 都显示，导致看起来"整行异常"；
  2. `Syntax::inlines` 对段落块（可能带 `+-****-` 前缀/列表标记）解析出的 Emphasis 与编辑器判定是否一致；
  3. 实际文档源码是否为本计划验证过的 `**En*gl*ish**` 形态。
- 取舍：本轮不再继续改 `apply_wrap`（避免在错误的层反复打补丁），问题转入 `3e-walkthrough-results.md` 的遗留清单，由用户回头处理。
