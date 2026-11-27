# Mermaid 22 图型深色观感复核（2026-11-27，F-011）

范围：内嵌 Mermaid 11.12.0 的 `MERMAID_11_12_CORPUS` 全部 22 个图型，深色主题。
证据目录：[`docs/qa/evidence/mermaid-dark-2026-11-27/`](evidence/mermaid-dark-2026-11-27/)（`<name>-dark.svg` + `<name>-dark.png`，另含同名浅色对照与 `index.html` 并排页）。

## 1. 生成方式（可复现）

```powershell
$env:VEIL_MERMAID_EVIDENCE_DIR='D:\VSCode\Veil\docs\qa\evidence\mermaid-dark-2026-11-27'
cargo test -p veil-mermaid-offline mermaid_11_12_corpus_renders_and_recovers
```

- SVG/PNG 全部由测试内真实渲染产生，未手写。
- **深色 PNG 已合成到应用深色底 `#1e1e1e` 后编码**（`DARK_EVIDENCE_BG = [30,30,30]`，
  `Theme::dark().bg`），与编辑器内「Mermaid 位图叠在深色画布上」的真实条件一致；
  浅色 PNG 仍保留透明画布，仅作浅色对照，不用于深色可读性判断。
- 自动门禁：`dark_corpus_renders_differs_and_rasterizes_opaque` 对 22 图逐类断言
  「深色渲染成功、与浅色 SVG 不同、可 resvg 栅格化、非全透明」，并对 fill 规则稳定的
  journey / pie / packet / c4context / radar 追加语境化的 WCAG AA 对比度断言。
  既有断言未删除或放宽。

## 2. 逐图结论

统计：22 图中 **19 张直接通过目检**，**3 张（journey、c4context、radar）修复后通过**，**0 张 NOT RUN**。
无裁切、无整图空白；所有文字在合成后的深底上均可辨认（关键文字实测对比度见 2.1）。

表头说明：「结论」是四项目检（文字可读 / 无深字压深底 / 与浅色不同 / 无裁切与空白）的综合；
任一项异常都会写入「问题」列。深色与浅色不同、非全透明另由自动断言逐类校验。

| 图型 | 结论 | 问题 | 处置 |
|---|---|---|---|
| architecture | 可读 | 无 | — |
| block | 可读 | 无 | — |
| c4context | 已修复 | `Rel` 标签与连线被 Mermaid 写死 `#444444`，实测 **1.71:1**，深底上几乎不可见 | 按 `aria-roledescription="c4"` 精确覆盖 `text[fill="#444444"]` 为白字、`line[stroke="#444444"]` 为 `#8b949e`；本语料中 `#444444` 仅出现在 Rel 元素上，不影响 c4 其它文字 |
| class | 可读 | 无 | — |
| er | 可读 | 无（关系标签 `#ccc` 位于深色半透明底） | — |
| flowchart | 可读 | 无（沿用既有深色边标签 override） | — |
| gantt | 可读 | 无（沿用既有标题/任务/网格 override） | — |
| gitgraph | 可读（轻微限制） | 分支标签 `main` 实测 **3.35:1**（`#2c2c2c` 压药丸 `#797d7d`），低于 AA 4.5、高于 3；提交标签/消息为浅灰，正常 | 未改：`branch-label0/3` 的底色由分支色动态生成，统一改深/浅字会在其它分支色上更差；按轻微限制记录，不宣称「直接可读」 |
| journey | 已修复 | 标题元素 `fill=""`（无效值）被 resvg 按黑色绘制，深底上不可读；任务标签与时间轴本身可读 | 新增 `[aria-roledescription="journey"] text[fill=""]{fill:#ffffff}`，仅命中空 fill 文本 |
| kanban | 可读 | 无（11.12 dark 默认白字/深色列） | — |
| mindmap | 可读 | 无（沿用既有根节点/标签 override） | — |
| packet | 可读 | 无；字段标签 `#e0dfdf` 落在 `#333` 区块上 | 加对比度断言（≥4.5:1） |
| pie | 可读 | 无；图例为浅色 | 加对比度断言（≥4.5:1） |
| quadrant | 可读 | 无 | — |
| radar | 已修复 | 轴标签原为 `color:lightgrey`（`color` 不作用于 SVG 文本 fill，实际继承 `#ccc`），压在半透明浅色网格上观感很差 | 按 `aria-roledescription="radar"` 把 `.radarTitle`/`.radarAxisLabel`/`.radarLegendText` 置白；白字对网格合成色实测 **7.11:1**、对页面底 15.3:1 |
| requirement | 可读 | 无 | — |
| sankey | 可读 | 无 | — |
| sequence | 可读 | 无 | — |
| state | 可读 | 无 | — |
| timeline | 可读 | 无（年份/事件为 `lightgrey`） | — |
| treemap | 可读 | 无 | — |
| xychart | 可读 | 无 | — |

### 2.1 合成后实测对比度（关键文字）

底色统一按 `#1e1e1e`；有实体底色的按实体色计算（WCAG 2.x 相对亮度，取 `#1e1e1e` 为会话画布）。

| 图型 / 文字 | 前景 / 背景 | 对比度 | 判定 |
|---|---|---|---|
| sankey 标签（`A 7` 等） | `#ccc` / `#1e1e1e` | 10.38:1 | 可读 |
| timeline 年份/事件 | `lightgrey` / `#0b0000` · `#4d1037` · `#3f5258` | 13.83 / 9.71 / 5.48:1 | 可读 |
| treemap 叶标签与数值 | inline `lightgrey` / 叶 `#1f2020` | 10.91:1 | 可读（CSS 类默认 black 被 inline 覆盖） |
| xychart 标题/轴标签 | `#ccc` / 图内 `#333` | 7.87:1 | 可读 |
| radar 标题/轴/图例 | `#ffffff` / 网格合成 `#585858`（页面 `#1e1e1e`） | 7.11 / 15.3:1 | 修复后通过 |
| block 块标签 | `#ccc` / `#1e1e1e` | 10.38:1 | 可读 |
| packet 字段标签 | `#e0dfdf` / `#333333` | 9.50:1 | 可读 |
| gitgraph 分支 `main` | `#2c2c2c` / `#797d7d` | **3.35:1** | 轻微限制（不宣称可读） |
| pie 图例 | `hsl(28.57,17.36%,86.27%)` / `#1e1e1e` | 12.91:1 | 可读 |
| quadrant 点标签 | `#e0dfdf` / `#1e1e1e` | 12.54:1 | 可读 |
| requirement `<<satisfies>>` | `#ccc` / `reqLabelBox #484a4a` | 5.56:1 | 可读 |
| architecture 服务标签 | `#ccc` / `#1e1e1e` | 10.38:1 | 可读 |

## 3. 仍未解决 / 明确不修

- **无 NOT RUN 图型**：本轮 22 图全部完成深色目检；GEOMETRY、可见文字、出图与透明度由自动测试持续守护。
- **gitgraph 分支标签**：实测 3.35:1，低于 WCAG AA 4.5:1（高于 3:1），深色下仍能辨认分支名；
  修它需要整体重排分支色板，属于配色设计变更而非安全覆盖，记为本轮已知轻微限制。
- **c4 stereotype 字号固定 12px**：既有已知降级（非深色对比度问题），保持
  `dark_c4_stereotype_contrast_and_fixed_font_size` 的锁定断言，不在本轮改动。
- 目检为人工判断（已完成），后续若 Mermaid 升级导致配色变化，证据目录可重新生成并对比。
