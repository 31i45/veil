# ADR-013: 表格布局保持自研 —— 不引入 comfy-table

## 状态

已采纳（2026-11-17）。依据：`docs/review/component-replacement-spike.md` S4。

## 背景

GFM 表格的列宽、行高、折行、命中与活动单元格模型由 `crates/veil-ui/src/view/table_strategy.rs`（1,706 行）与 `crates/veil-ui/src/view/table_view.rs`（829 行）自研。按「能纯集成覆盖的模块优先用成熟组件」的目标，评估用终端表格库 `comfy-table` 8.0.1 替换。

## 实测

独立空探针 `probe-empty` = 107,008 B；接入 comfy-table（`default-features = false`，关闭 tty/crossterm）后 = 198,656 B。

| 指标 | 结果 |
|---|---|
| exe 增量 | +91,648 B |
| 当前 release 余量 | 11,232 B（9,888,768 / 9,900,000）→ 增量是余量的 8.2 倍 |
| 新增 unique 包 | 1（`comfy-table`）；`unicode-width` / `unicode-segmentation` 已在 workspace 树中 |

能力对照：

| 需求 | Veil 现状 | comfy-table |
|---|---|---|
| 列宽 | cosmic-text 逐字形测量（DPI/字号/字体） | 字符格宽，无像素概念 |
| 折行 | 禁则感知 + shaping + 样式段映射 | 按字符数贪心折行，输出 `String` |
| 单元格富文本 | 粗体/斜体/行内代码/链接色/删除线逐段样式 | 整格 ANSI 前景/背景色 |
| 行内公式 / 图片 | NBSP 占位 + 字形锚点 + 原位 blit | 无 |
| 光标落点 / 命中 | 字形 → 文档字节偏移 + 网格命中 | 无（终端字符串，不接收坐标） |
| 网格绘制 | softbuffer 像素级 1px 线、DPI 对齐、列宽拖拽 | 字符画边框（`+---+`） |

## 决策

保持自研表格布局与绘制，不引入 `comfy-table`。

## 理由

1. **输出契约不同**：comfy-table 产出终端字符网格字符串；Veil 需要像素几何、逐格光栅位图、字形锚点与字节偏移映射。唯一可复用的「边框排布」在 Veil 侧是像素级绘制，复用率约为 0。
2. **会变成双份计算**：必须先用 cosmic-text 量像素宽，再折算字符格喂给 comfy-table，最后从字符网格反推像素位置——增加一层有损换算，而不是减少代码。
3. **体积一票否决**：+91,648 B 对 11,232 B 余量。

## 后果

- 表格的列宽/折行/命中继续由 Veil 维护；新增表格能力（横向滚动、合并单元格等）仍需自研。
- 表格相关的既有测试与 ADR 不变。
- 若未来需要「表格导出为纯文本/Markdown」，可把 comfy-table 限定为导出路径的可选依赖单独立项；渲染主路径不引入。

## 否决的替代方案

| 方案 | 结论 | 理由 |
|---|---|---|
| comfy-table 替换布局 | 否决 | 字符网格 vs 像素几何，复用率≈0，体积超余量 8 倍 |
| comfy-table-inline（fork） | 否决 | 同样是字符串输出，多一层 fork 维护风险 |
| tabled | 否决 | 同为字符串表格库，能力模型与 comfy-table 同类，体积与依赖只会更高 |
