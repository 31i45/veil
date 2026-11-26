# 双解析器差分 oracle 证据（2026-11-26）

> 基线：`main` @ `71cb42a`，版本 `3.0.2`，工作区 clean（仅本 Task 文件变更）
> 被测对象：编辑器自研增量语义树（`veil-core::Session`） vs HTML 导出/一致性门禁参照 `pulldown-cmark 0.13.6`
> 性质：**接缝观测**。不替换编辑器解析器，不修改导出路径，不把 `pulldown-cmark` 提升为生产依赖。

## 1. 结论

1. **差异可见、可重跑**：721 篇文档（CommonMark 652 + GFM 23 + 真实文档 46）全链路 **0 panic**；一条命令输出全部统计与分类。
2. **块骨架高度一致**：CommonMark `652/652`、GFM `23/23` 顶层块序列与门禁参照完全一致；真实文档仅 7 篇有骨架差异，且全部可归因（6 篇块切口策略、1 篇 front matter 扩展）。
3. **编辑器扩展与参照不同源**：5 篇真实文档命中编辑器独有语法（front matter、`<details>`、Admonition），另有 callout / 公式 / 脚注 3 个内联样例，均被明确分类为「语法覆盖」，不是缺陷。
4. **渲染差异是序列化差异**：28 篇 canonical HTML 不一致，全部是实体转义、表格 `align` vs `style`、任务列表 `<input>` 属性序、空 `<tbody>`、裸链接扩展等既有门禁已记录的形式差异。
5. **裁决：本 program 不替换编辑器解析器**。oracle 未发现新增「必须修解析器」的差异；既有差异继续由现有一致性门禁与导出层负责。

## 2. 语料（复用，不新造）

| 语料 | 规模 | 位置 | 用途 |
|---|---:|---|---|
| CommonMark 0.31.2 官方规范 | 652 例 | `crates/veil-core/tests/conformance/examples/` | 块骨架 + 文本 + 边界 + 规范期望 HTML |
| GFM 扩展 | 23 例 | `crates/veil-core/tests/conformance/gfm/` | 表格/任务列表/删除线/裸链接 |
| 真实文档 | 46 篇 | `crates/veil-core/tests/conformance/real/` | 真实写作风格（无期望 HTML，只比结构/文本/边界） |
| 内联样例 | 6 组 | `tests/parser_differential.rs` 内联 | CommonMark 基线、GFM 表格、CJK、callout、公式、脚注 |

真实文档目录中的 `index.tsv` 不计入，实际 46 篇。

## 3. 方法

实现：`crates/veil-core/tests/parser_differential.rs`（集成测试，可访问 `dev-dependencies`）。

参照配置与一致性门禁一致：`pulldown-cmark` 启用 `ENABLE_TABLES | ENABLE_STRIKETHROUGH | ENABLE_TASKLISTS | ENABLE_GFM`；块级口径复用 `tests/common/mod.rs`（连续列表项折叠为 List，原始 HTML 块两侧都不计）。未修改 `Cargo.toml`。

四个比较维度：

1. **块级类型**：Veil 顶层块序列 vs 参照顶层块序列（`actual_blocks` vs `pulldown_blocks`）。
2. **文本归一化**：两侧块一一对齐时，按源码字节区间取切片，折叠空白后比较。
3. **边界**：对齐块比较字节区间；文本相同而区间不同记为「边界表示差异」（尾部换行、容器切口等）。
4. **渲染**：`canonical(导出 HTML)` vs `canonical(规范期望 HTML)`（与 Suite B 同口径；真实文档无期望 HTML，不测该维度）。

对齐规则：仅当两侧顶层块数相同才逐块比较；列表按标记签名 `(ordered, marker)` 分组，标记变化起新列表（与 Example 301/302 口径一致）；`LinkRefDef`/`FootnoteDef`/`FrontMatter`/`Details`/`HtmlBlock` 不产 HTML，跳过块序列对齐。

分类：

- **语法覆盖**：一侧独有的语法（编辑器扩展或参照缺失）。
- **位置/边界**：同内容、不同切口（块合并/拆分、区间表示）。
- **渲染差异**：canonical HTML 或对齐块文本不同。

门禁：`<=` 基线，只允许改善；基线常量与解释在测试文件顶部（`BASELINE_*`）。

## 4. 结果

| 维度 | CommonMark 652 | GFM 23 | 真实文档 46 | 合计 |
|---|---:|---:|---:|---:|
| 块骨架不一致（文件） | 0 | 0 | 7 | **7** |
| 其中：语法覆盖（front matter） | 0 | 0 | 1 | 1 |
| 其中：位置/边界 | 0 | 0 | 6 | 6 |
| 骨架一致但含扩展语义（文件） | 0 | 0 | 4 | **4** |
| 对齐块文本不一致（块） | 1 | 0 | 2 | **3** |
| 渲染不一致（文件，canonical） | 24 | 4 | 0（无期望） | **28** |
| 仅边界表示差异（块，文本相同） | 88 | 0 | 309 | 396 |
| 逐块对齐成功 / 跳过 | 650 / 2 | 23 / 0 | 35 / 0 | 708 / 2 |

- 对齐块总数：CommonMark 750、GFM 36、真实文档 1677；其中文本一致 749 / 36 / 1675。
- CommonMark 跳过的 2 例：Example 109（缩进代码块拆分粒度）、296（列表拆分粒度）——骨架一致但两侧顶层块数不同，不进入逐块比较。
- 真实文档 4 篇「骨架一致但含扩展语义」：`nodejs`（3 个 Details）、`serde`（1 个 Details）、`bat`（8 个 Admonition）、`fd`（2 个 Admonition）。

### 4.1 语法覆盖（5 篇，均为编辑器扩展）

| 文件 | 编辑器块 | 参照侧 | 归属 |
|---|---|---|---|
| `microsoft_vscode-docs__overview_md` | FrontMatter | `---` 被当作 ThematicBreak | 产品扩展，保留 |
| `nodejs_node__README_md` | Details ×3 | HTML 块/段落 | 产品扩展，保留 |
| `serde-rs_serde__README_md` | Details ×1 | HTML 块/段落 | 产品扩展，保留 |
| `sharkdp_bat__README_md` | Admonition ×8 | 普通 BlockQuote | 产品扩展，保留 |
| `sharkdp_fd__README_md` | Admonition ×2 | 普通 BlockQuote | 产品扩展，保留 |

内联样例同样命中 3 组：callout（Admonition vs BlockQuote）、`$$` 公式（Math vs Paragraph）、脚注（FootnoteRef + FootnoteDef vs 普通段落）。

### 4.2 位置/边界（6 篇真实文档 + 396 个边界表示差异）

- 骨架差异 6 篇：`emilk_egui__CHANGELOG`、`emilk_egui__README`、`kubernetes__README`、`rust-lang_rustfmt__README`、`sharkdp_hyperfine__README`、`uber-go_zap__README`。
- 共同模式：列表/表格与相邻段落、分隔线的合并/拆分策略不同（Veil 按块模型合并同类型连续节点，参照按语义边界拆开）。
- 396 个「文本相同、区间不同」全部是区间表示差异（参照段落区间通常不含尾随换行、容器区间切口不同），不是内容差异。
- 该口径与既有 `tests/real_world_check.rs` 的已接受差异同源（阈值 19）。

### 4.3 渲染差异（28 篇，canonical HTML）

| 语料 | 数量 | 代表例 / 原因 |
|---|---:|---|
| CommonMark | 24 | 反斜杠转义（0012/0014）、字符实体（0027/0041）、Setext（0091）、链接引用定义（0209/0210/0211/0218）、代码 span（0343）、强调（0352/0359/0363/0380/0385/0395）、链接/图片（0508/0590）、裸链接扩展（0611/0612）、Raw HTML（0619/0620/0624/0632） |
| GFM | 4 | 表格 `align` vs `style`、空 `<tbody>`（tables-002/008）；任务列表 `<input>` 属性序（task_list_items-001/002） |

这些差异与现有一致性门禁一致：Suite B canonical 基线 628/652（即 24 例差异）；GFM 侧 `gfm_canonical` 把表格对齐、`<input>` 属性序、空 `<tbody>` 三类序列化差异抹平后为 23/23。

### 4.4 对齐块文本差异（3 个块）

| 位置 | 现象 | 原因 |
|---|---|---|
| CommonMark 0210 | 编辑器段落 `[foo]: /url "title" ok` vs 参照 `"title" ok` | 多行链接引用定义：编辑器未把首行识别为定义 |
| `rust-lang_rust__README` | 编辑器 ListItem 区间不含列表末尾链接定义，参照 List 区间包含 | `LinkRefDef` 在编辑器是独立块，参照折进 List 容器 |
| `tokio-rs_tokio__README` | 同上 | 同上 |

均为链接引用定义的归属/边界差异，不影响编辑器可见渲染（编辑器按源码行展示定义）。

## 5. 内联样例

| 样例 | 编辑器 | 参照 | 分类 |
|---|---|---|---|
| CommonMark 标题 + 强调 + 链接 | AtxHeading + Paragraph | 同 | Match（文本差异 0） |
| GFM 表格（CJK 单元格） | Table，单元格 `["名称","值","中文","1"]` | Table | Match |
| CJK 全角标点段 | 2 × Paragraph | 同 | Match（文本差异 0） |
| Callout `> [!NOTE]` | Admonition(Note) | BlockQuote | 语法覆盖 |
| 公式 `$$…$$` | Math | Paragraph | 语法覆盖 |
| 脚注 `[^a]` + 定义 | FootnoteRef + FootnoteDef | 2 × Paragraph | 语法覆盖 |

## 6. 需修差异与后续归属

- **需修解析器差异：0**。本 oracle 没有发现必须修改编辑器解析器的新缺陷。
- 渲染精确性 → 既有 `tests/conformance.rs` Suite B 与 HTML 导出（Agent B 区），oracle 不重复修。
- 真实文档块切口差异 → `tests/real_world_check.rs`（阈值 19）继续作为策略差异接受。
- 编辑器扩展（front matter / details / admonition / callout / 公式 / 脚注）→ 产品差异化能力，保留；导出/PDF 的表现差异归 Task 7（断行）与 Task 9（PDF spike）评估。
- 链接引用定义归属（0210、rust/tokio README）→ 记录为已知语法覆盖边界；本 program 不修，避免为差分而改解析器行为。

## 7. 局限与风险

- 逐块比较要求两侧顶层块数相等；2 篇 CommonMark 跳过（109/296），差异只在拆分粒度。
- 文本比较用源码切片归一化，不是渲染后可见文本；行内差异不在本 oracle（由 Suite C/D 覆盖）。
- 真实文档没有规范期望 HTML，渲染维度不适用。
- 语料是仓库内快照；`conformance/real/` 固定 46 篇，不受仓库文档增长影响。
- 门禁按「只允许改善」锁定当前计数；若新增扩展语法或语料升级，需同步更新基线并在本文件追加说明。

## 8. 复现

```powershell
cargo test -p veil-core parser_differential -- --nocapture
```

- 输出：`CORPUS commonmark|gfm|real` 统计、`TOTAL` 汇总、`[render]/[text]/[type]/[semantic]` 明细、`SAMPLE DIFFERENTIAL` 样例表。
- 环境：Windows，`rustc 1.99.1`，debug profile；本机实测 1.04s（编译缓存后）。
- 基线常量：`crates/veil-core/tests/parser_differential.rs` 顶部 `BASELINE_*`。
