# 跨表面裁决：远程图片 / Mermaid 暗色 / PDF 表格与中文断行

> 归属：`converge-veil-under-jobs-lens` Phase 1，Task 2.2–2.4（Agent B）。
> 基线：`main` @ `a0ce9f9`（Task 1 之后），版本 3.0.2。
> 原则：能修就修到用户看得见；修不动就写明限制与触发条件，不静默降级。
> 复跑：`cargo test -p veil` → 243 passed / 1 ignored；`cargo test -p veil-mermaid-offline dark_` → 7 passed。

## 1. 远程图片（HTML 导出）——已修复：可见警告

**裁决：修。** 不内联远程图片（离线自包含是产品契约），但不允许静默丢图。

**行为**

- 远程 `src` 仍在导出阶段被删除，导出件不含任何远程请求，与 `README.md` 的“HTML 导出删除远程 `src`”一致。
- 新增：只要导出件里出现过远程图片，正文顶部插入一个可见警告块，中英双语静态文本，
  说明“远程图片未内联，离线打开可能缺失”。不新增 i18n key（导出件脱离应用语言设置）。
- 覆盖两条路径：
  1. Markdown 图片（`![alt](https://…)`）：由 `decide_image_src` 判定为远程并删除；
  2. 行内 HTML 图片（`<img src="https://…">`）：core 把地址放进 `remote_src`、`src` 留空，
     导出时同样留下远程标记，触发同一警告。
- 非位图 data URI（如 `data:image/svg+xml`）仍被删除，但使用不同标记，不会误报为“远程图片”。

**证据**

- 代码：`crates/veil/src/export.rs`
  - `remote_image_warning()`（L435）与 `html_of_doc` 中正文前的警告插入（L424–L430）
  - `REMOTE_IMAGE_MARKER` / `BLOCKED_IMAGE_MARKER`（L1697–L1701）
  - `localize_images` 的 `BlockedRemote` 分支（L1719 起）、`decide_image_src`（L1777）
  - 行内 HTML 远程图片标记（L730）
- 测试（`cargo test -p veil export`，86 passed）：
  - `remote_markdown_image_adds_visible_export_warning`（L3344）
  - `remote_inline_html_image_adds_visible_export_warning`（L3359）
  - `local_image_export_has_no_remote_warning`（L3368，确保无远程图片时不出现警告噪音）
  - 既有 `remote_image_src_is_not_exported` / `export_has_no_remote_resources` 仍通过（无远程地址泄漏）

**用户可见说明位置**

- 导出 HTML 正文顶部的警告块（随导出件分发，读者可见）。
- `README.zh-CN.md` / `README.md` 已知限制中的“远程图片”条目（本次未改，归 Agent A/文档 owner）。

**残留（不在本次范围）**

- PDF / 打印仍降级为 alt 文本或 `[图片]`，已在 README 标注；本轮只处理 HTML 导出。

## 2. Mermaid 暗色对比——接受现状 + 记录已知限制与触发条件

**裁决：接受 + 明确标注。** 不做代码改动：现有的暗色 override 已覆盖主要图型并有自动对比度断言，
剩余项缺少“已验证安全的窄选择器”，硬改会把风险扩散到全部图型。

**现状证据**

- 暗色 `themeCSS`：`crates/veil-mermaid-offline/src/lib.rs` L179–L196（flowchart 边标签、mindmap 全层级、gantt 任务/网格/轴）。
- 自动对比度测试（同一文件）：
  - `dark_flowchart_edge_label_contrast_uses_final_svg_css`（L423）
  - `dark_gantt_task_text_has_readable_contrast`（L608）、`dark_gantt_bars_grid_and_axis_have_readable_contrast`（L627）
  - `dark_c4_stereotype_contrast_and_fixed_font_size`（L714，stereotype 字号固定 12px 的降级）
  - `dark_mindmap_all_label_levels_have_readable_pixels`（L1829）
- 用户可见说明：`README.zh-CN.md` 已知限制“深色 Mermaid 对比度”（L144），列明已覆盖图型与 NOT RUN 范围。
- 本次复跑（当前 HEAD）：`cargo test -p veil-mermaid-offline dark_` → 7 passed，0 failed。

**已知差异**

1. 其余 16 种 corpus 图型（sequence / class / er / pie / state / gitGraph / timeline / sankey /
   xychart / block / packet / architecture / radar / treemap / requirement / quadrant）的暗色观感
   逐图人工目检为 **NOT RUN**：只自动校验几何、可见文字与出图成功。
2. `c4context` 的 `Rel` 连线标签沿用 Mermaid 固定的 `#444444`，暗色背景下可能对比不足；
   未加断言，因为全局 CSS 覆盖会泄漏到其他图型，而以 `aria-roledescription` 收窄的选择器尚未实测。

**触发条件（何时必须重开）**

- 用户在暗色主题下反馈上述某图型文字不可读；
- 升级 Mermaid 版本并改动默认 `themeVariables`；
- Phase 5 无障碍要求逐图型给出对比度证据。

**建议修复点**

- 在 `Session::render` 的 `theme_css` 暗色分支（L179）新增作用域化的 `themeCSS`，
  先实际渲染 c4 SVG 取到 `Rel` 标签的确定 class / role 根，再写窄选择器；
  同时补一条 `contrast_ratio` 断言（复用同文件既有辅助函数）。不要在无断言的情况下扩大全局覆盖。

## 3. PDF 表格 / 中文断行——接受 + 明确文档，移交 Phase 4 spike

**裁决：接受 + 明确文档。** 不改 PDF 代码；此项作为 Phase 4（Task 9）PDF 断行 spike 的输入。

**已知差异**

1. 中文断行是简化禁则（闭符号不出现在行首、开符号不留在行尾），不是完整 UAX #14，
   也不做标点挤压；换行位置可能与渲染模式（cosmic-text）不同。
2. 表格按固定行高估算拆行分页；极端高的单元格可能比浏览器多占几页。
3. 表格单元格保留粗体/斜体/行内代码底色/链接颜色与下划线，超长单元格跨页续排、不再截断。

**触发条件**

- 文档含大段中文标点或中英混排长段落（断行位置差异）；
- 表格含高单元格或长文本（分页页数差异）；
- 物理打印验收（Phase 2 Task 4.4）。

**用户可见说明位置**

- `README.zh-CN.md` L145–L147（“PDF / 打印表格”“PDF / 打印中文断行”）；
- PDF 产物内**不**加警告：此处无数据丢失，差异是排版位置，逐页警告会变成噪音。

**后续归属**

- Phase 3 Task 7（Agent G）：抽出可测试的断行策略层，产出 CJK 长段落/表格/标点禁则的跨表面差异基线。
- Phase 4 Task 9（Agent I）：以 Task 7 语料为基线，先测后换 `icu_segmenter` / 复核 `unicode-linebreak`，
  走六道门（质量、体积 ≤14 MiB、许可证、离线、无 webview、行为契约），产出 go/no-go：
  `docs/review/evidence/spike/pdf-linebreak.md`。
- 在 spike 结论出来之前，README 的“简化禁则”表述即为对外承诺，不得写成“与渲染模式一致”。
