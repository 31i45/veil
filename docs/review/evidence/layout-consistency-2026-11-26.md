# 屏幕 / PDF 断行一致性证据（Task 7 · Agent G）

> 日期：2026-11-26
> HEAD：`71cb42a`；版本：`3.0.2`
> 范围：只测量、不改行为。未触碰 config / parser / export / UI 代码。
> 可重跑：`cargo test -p veil pdf_layout -- --nocapture`

## 1. 结论摘要

- 屏幕（`veil-ui`：cosmic-text + WJ 胶水）与 PDF（`veil/src/platform/pdf_layout.rs`：自研贪心 + 简化禁则）是两条独立断行路径，本任务把差异变成可测量、可解释的基线。
- 在**同一 em 容量**下，共享禁则（中日文闭/开符号）的断点完全一致：8 项测试全绿，其中 `shared_kinsoku_policy_breaks_agree_at_normalized_capacity` 覆盖纯 CJK、闭符号、开括号三类语料。
- 差异集中在 3 类：**每行容量**（屏幕 40 全角 vs PDF 43 全角）、**半角/破折号禁则表不同**（PDF 多 9 个行首 + 3 个行尾半角符号；屏幕多 `—` `～`）、**分页与表格跨页**（PDF 有页，屏幕连续滚动）。
- 产品宽度下 4 组语料共 **11 行**规范化后内容不同；全部有确定原因，无丢字、无越界、无行首禁则违规。

## 2. 方法

两条测量层，职责分离：

| 层 | 被测对象 | 宽度来源 | 确定性 |
|---|---|---|---|
| 策略层 | PDF `fill_lines` + `is_line_start/end_forbidden`；屏幕禁则表从 `crates/veil-ui/src/view/linebreak.rs` **源码解析** | em 归一（`content_w / 11pt`） | 完全确定（`FakeMetrics`：ASCII 0.55em、CJK 1em） |
| 端到端层 | 真机 `LayoutEngine::layout_block`（cosmic-text + 系统字体） vs `layout_document` | 产品宽度 | 屏幕侧依赖系统字体 advance，属渲染层 |

测试工具：`crates/veil/src/platform/pdf_layout_consistency_tests.rs`（以 `#[path]` 挂在 `pdf_layout::tests` 下，仅测试期编译）。

断点比较口径：去掉 WJ 与空白后的行内容逐行比较；PDF 行首偏移按可见字符累计。这样把「空格归属行尾还是行首」的差异排除，只比较真正的断字位置。

## 3. 语料

| 名称 | 内容 | 目的 |
|---|---|---|
| `cjk` | `汉 × 120` | 纯 CJK 长段落容量 |
| `cjk_punct` | `(甲×42 + "。") × 3` | 闭符号避头 |
| `cjk_open` | `(乙×42 + "（") × 5 + "后"` | 开符号避尾 |
| `halfwidth` | `丁×43 + "..." + "ABC"` | PDF 半角标点避头 |
| `em_dash` | `甲×43 + "—" + 乙×10` | 两表对 `—` 的分歧 |
| `mixed` | `中×20 + " ascii words here " + 文×20` | 中西混排 + 空格断行偏好 |
| `longword` | `x × 200`、`x × 120` | 英文超长词硬断 |
| `table` | 单列表格，单元格 `表 × 4000` | 表格跨页续排、行高固定 |
| `tall` | `汉 × 3000` 单段 | 分页与页边界 |

## 4. 差异清单

### 4.1 禁则表（确定性，逐字符）

`kinoksu_tables_differ_only_in_documented_entries` 实测（候选集 = 可打印 ASCII ∪ 常见中文/全角标点 ∪ 两表成员）：

| 方向 | 条目 | 数量 | 原因 |
|---|---|---:|---|
| 屏幕行首 | 24 | — | GB/T 15834 通行集（`\u3002` 等） |
| PDF 行首 | 31 | — | 自研表 |
| PDF 独有行首 | `! ) , . : ; ? ] }` | 9 | PDF 自研表额外收录半角闭符号；屏幕表只收中文/全角集 |
| 屏幕独有行首 | `—` `～` | 2 | 屏幕用 WJ 胶水强制避头；PDF 表未收录 |
| 屏幕行尾 | 11 | — | 开括号/开引号 |
| PDF 行尾 | 14 | — | 自研表 |
| PDF 独有行尾 | `( [ {` | 3 | 同上，半角开符号 |
| 屏幕独有行尾 | 0 | 0 | — |

后果由两个断言固化：

- `pdf_rejects_halfwidth_punctuation_at_line_start`：`丁×43 + "..."` 时 PDF 首行只留 42 字，把末字与 `.` 一起下移；半角 `.` 绝不出现在 PDF 行首。
- `em_dash_is_glued_on_screen_but_allowed_at_pdf_line_start`：同一 em 容量下，PDF 第二行以 `—` 开头（表未收录），屏幕首行只留 42 字、`—` 永不出现在行首（WJ 把前一字带下去）。

### 4.2 断点与行宽（产品宽度，真实排版）

`product_width_break_difference_summary --nocapture` 实测（屏幕 640px = 40 全角；PDF 481.89pt = 43.8em）：

| 语料 | PDF 行数 | 屏幕行数 | 内容不同的行 | 首行容量 | 原因 |
|---|---:|---:|---:|---|---|
| `cjk` | 3 | 3 | 3 | 43 vs 40 | 容量差 |
| `cjk_punct` | 3 | 4 | 4 | 43 vs 40 | 容量差 + 屏幕 WJ 把 `。` 与前字绑定 |
| `mixed` | 2 | 2 | 2 | — | PDF 优先空格（词边界）；屏幕在 CJK 边界断 |
| `longword` | 2 | 2 | 2 | 79 vs 78 | 字宽度量不同（PDF `FakeMetrics` 0.55em/字 vs 微软雅黑真实 advance） |

合计 **11 行差异**；四组语料均满足：不丢字、不重字、两侧行首/行尾不违反各自禁则表。

### 4.3 分页与表格

| 事实 | 实测 | 归属 |
|---|---|---|
| 正文行距恒为 `11pt × 1.55 = 17.05pt` | `pdf_pagination_uses_fixed_line_height_and_stays_inside_margins` 通过 | PDF 固定行高 |
| 屏幕行高 22px / 16px = 1.375em | 由 `BASE_LINE_HEIGHT / BASE_FONT_SIZE` 得出 | 屏幕更紧 12.7% |
| 长段落按行分页，基线不越正文区 | 3000 字 → 多页，每页行距一致 | 通过 |
| 超长单元格跨页续排、每页重绘边框，续页不重复表头文字 | 4000 字单元格 → 多页，文本页均 > 0 条且未截断 | 通过（PDF 已知行为） |
| 屏幕无分页 | 连续滚动，单一 `BlockLayout` | 媒介差异 |

## 5. 原因分类与裁决

| # | 差异 | 原因 | 裁决 |
|---|---|---|---|
| 1 | 每行 40 vs 43 全角 | 屏幕正文 16px、版心 ≤ 640px；PDF 正文 11pt、A4 去 20mm 页边距 | **接受**：纸面字号更小是既有产品选择 |
| 2 | 行距 1.375em vs 1.55em | 屏幕 `BODY_LINE_HEIGHT=22`；PDF `line_height = size × 1.55` | **接受**：纸面行距更松 |
| 3 | 半角标点表差 9 + 3 | PDF 自研表额外收录半角闭/开符号；屏幕依赖 cosmic 的 UAX #14 | **接受，交 Task 9 统一** |
| 4 | `—` `～` 屏幕独占 | 屏幕用 WJ 强制避头；PDF 表未收录 | **交 Task 9**：统一到 `view::linebreak` 单一真值 |
| 5 | 空格断行偏好不同 | PDF 贪心 `last_space` 优先；cosmic `Wrap::WordOrGlyph` | **接受**：两侧语义都成立 |
| 6 | 英文长词断点差 1 字 | PDF 用 `FontMetrics` 字宽表；屏幕用真实 shaping advance | **接受**：渲染层差异，不做数值门禁 |
| 7 | 分页/表格跨页 | 纸面独有 | **接受**：媒介差异；跨页行为已有契约测试 |
| 8 | 未做标点挤压/悬挂 | 屏幕有 `hang_shift` 半字悬挂；PDF 无 | **接受并披露**：README 已注明非完整 UAX #14 |

## 6. 交给 Task 9（PDF 断行 spike）的基线

1. **行为契约**：替换断行器前后必须保持 `fill_lines` / `kinsoku_break` 的对外语义（行内容、行数、无丢字、禁则合规）。本文件 8 项测试可直接作为回归门槛。
2. **单一真值**：若引入 `icu_segmenter` / `unicode-linebreak`，应让 PDF 复用 `view::linebreak` 的禁则表与胶水决策，消除 4.1 的双表；本文件的表差分测试届时应从「断言差异存在」改为「断言无差异」。
3. **待裁决**：ASCII 半角标点是否纳入统一表（当前 PDF 比屏幕更严 9 + 3 项）。
4. **未覆盖**：屏幕侧半角 `.` 在 UAX 语境下的行为未做断言（IS 类依赖前向数字语境）；Task 9 需要完整 UAX 表才能下结论，不要把本文件的 PDF 侧结论外推到屏幕。
5. **门禁口径**：换实现后仍需通过 `cargo test -p veil pdf_layout`（当前 52 passed / 1 ignored）与体积 ≤ 14 MiB 检查。

## 7. 复现与局限

```powershell
cargo test -p veil pdf_layout -- --nocapture
# 只看差分摘要
cargo test -p veil pdf_layout::tests::consistency -- --nocapture
```

局限：

- 屏幕侧需要系统字体才能成形；本机与 CI（`windows-latest`）均具备微软雅黑。断点数值随字体版本可能微小漂移，因此端到端层只断言不变量，不断言具体断点。
- PDF 侧测试使用 `FakeMetrics`（确定性），真实导出用 `FontMetrics`（从字体 cmap 读字宽），字宽略有差异；该差异属渲染层，本任务不收敛。
- 绝对宽度不可直接比：屏幕用 px、PDF 用 pt，比较一律在 em 归一容量下进行。
- 只覆盖正文段落与表格单元格；列表/引用/代码块沿用同一 `fill_lines`，未另测。
