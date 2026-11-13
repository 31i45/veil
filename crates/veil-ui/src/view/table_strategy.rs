//! 表格策略层（切片 9）：GFM 表格 → 列宽 / 行高 / 命中与活动单元格模型。
//!
//! 取舍：
//! - 非活动单元格按 Markdown 行内语义渲染（粗体/斜体/行内代码/删除线/链接色）；
//!   活动单元格（caret 落在 `cell.range` 内）显示含标记的源码，离开即恢复富文本。
//! - 非活动单元格内的行内公式 `$x^2$`：可见文本替换为 NBSP 占位（宽≈公式图宽），
//!   排版后按占位首字形定位，绘制期由视图层取图 blit（与正文行内公式同一缓存）。
//! - 表宽超出版心时按比例收缩列宽（下限 `MIN_COL_WIDTH`），单元格内按字形折行；
//!   触底仍超宽时表格向右溢出，由 frame 裁剪（v1 不做横向滚动）。
//! - 网格点击落点 = 所在单元格内容起点（进入源码态，随后由既有块排版显示源码）。
//! - 已知简化（同数学块）：跨渲染态表格的选区无高亮。
//! - 已知简化（公式单元格 v0）：含公式的单元格降级为纯文本样式（不做粗体/斜体等
//!   富文本叠加）；公式图不按行高缩放，而是把行高扩到图底（图顶贴所在视觉行行顶）。
//!
//! 边界：本模块只产出几何与随块缓存的数据模型（列宽 / 行高 / 命中 / 活动格），
//! 不写 Frame。单元格文本的 shaping、禁则折行与样式段映射在 `table_view`，
//! 网格线/底色/位图落帧也在 `table_view`。

use cosmic_text::Metrics;
use std::borrow::Cow;
use std::collections::HashMap;
use veil_core::{CellAlign, Range, Table, TableCell};

use super::layout::{cell_style_spans, LayoutEngine, OverKind, UnderKind};
use super::style::Style;
use super::table_view::glyph_anchor;

/// 单元格水平/垂直内边距（基准字号下），随 DPI 缩放。
/// 单一来源：`tokens::doc::TABLE_PAD_X` / `TABLE_PAD_Y`。
const CELL_PAD_X: f32 = crate::tokens::doc::TABLE_PAD_X;
const CELL_PAD_Y: f32 = crate::tokens::doc::TABLE_PAD_Y;
/// 列宽收缩下限（基准字号下）：再窄会把文本挤成逐字折行。
pub(crate) const MIN_COL_WIDTH: f32 = 40.0;

/// 单元格内联公式（输入）：`range` 为剥标记后可见文本的字节区间（`$x^2$` → `x^2`），
/// `w/h` 为渲染图尺寸（占位宽与行高据此计算）。
pub(crate) struct CellMath {
    pub range: Range,
    pub tex: String,
    pub w: u32,
    pub h: u32,
}

/// 单元格内联公式（输出）：相对单元格内容原点的绘制位置与图尺寸。
pub struct TableCellMath {
    pub x: f32,
    pub y: f32,
    pub w: u32,
    pub h: u32,
    pub tex: String,
}

/// 占位后的公式段（占位文本坐标），排版后映射为 `TableCellMath`。
struct PendingMath {
    range: Range,
    w: u32,
    h: u32,
    tex: String,
}

/// 表格单元格内联图片（输入）：已加载的位图 + 自然尺寸（无 URL，失败已在上层过滤）。
pub(crate) struct CellImgAsset {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}

/// 单元格内联图片（输出）：相对单元格内容原点的绘制位置与显示尺寸。
/// `pixel_w/pixel_h` 为位图自然尺寸（绘制时按显示尺寸最近邻缩放）。
pub(crate) struct TableCellImg {
    pub x: f32,
    pub y: f32,
    pub w: u32,
    pub h: u32,
    pub pixel_w: u32,
    pub pixel_h: u32,
    pub pixels: Vec<u8>,
}

/// 表格行几何（网格坐标，原点为表格左上角）。
pub struct TableRow {
    pub top: f32,
    pub height: f32,
}

/// 单元格几何 + 已光栅化的文本（行优先存放）。
pub struct TableCellLayout {
    /// 文本绘制左缘（已含水平内边距）。
    pub x: f32,
    /// 文本绘制上缘（已含垂直内边距）。
    pub y: f32,
    /// 单元格内容宽（= 列宽 - 2×内边距）。
    pub w: f32,
    /// 单元格内容高。
    pub h: f32,
    /// 内容起点（文档绝对偏移）：网格点击落点。
    pub doc_start: usize,
    pub pixels: Vec<u8>,
    pub pixel_w: u32,
    pub pixel_h: u32,
    /// 非活动格内联公式（相对内容原点）；空 = 无公式或活动格（源码态）。
    pub maths: Vec<TableCellMath>,
    /// 非活动格内联图片（相对内容原点，水平并排）；空 = 无图或活动格。
    pub(crate) images: Vec<TableCellImg>,
    /// 非活动格行内前置装饰（相对内容原点的物理矩形；与正文同语义）。
    pub(crate) under: Vec<(f32, f32, f32, f32, UnderKind)>,
    /// 非活动格行内后置装饰（abbr 虚线下划线）。
    pub(crate) over: Vec<(f32, f32, f32, f32, OverKind)>,
}

/// 表格网格排版结果（随块缓存；宽度/DPI/内容/活动格变化时重建）。
pub struct TableLayout {
    pub cols: Vec<f32>,
    pub rows: Vec<TableRow>,
    /// 行优先，长度 = rows.len() × cols.len()。
    pub cells: Vec<TableCellLayout>,
    pub width: f32,
    pub height: f32,
    /// 容器（引用条/列表项/details）内的水平缩进（物理像素，相对版心原点）。
    pub x: f32,
    /// 活动单元格 flat 索引（源码态文本）；None = 全部富文本。
    pub active: Option<usize>,
}

impl TableLayout {
    /// 网格内物理坐标 → 该单元格内容起点（文档绝对偏移）。越界钳到首/末行、首/末列。
    pub fn hit(&self, x: f32, y: f32) -> usize {
        if self.cells.is_empty() || self.cols.is_empty() {
            return 0;
        }
        // 容器内表格整体右移：命中先把坐标换回网格原点。
        let x = (x - self.x).max(0.0);
        let row = self
            .rows
            .iter()
            .position(|r| y < r.top + r.height)
            .unwrap_or(self.rows.len() - 1);
        let mut col = self.cols.len() - 1;
        let mut acc = 0.0;
        for (i, w) in self.cols.iter().enumerate() {
            if x < acc + w {
                col = i;
                break;
            }
            acc += w;
        }
        self.cells
            .get(row * self.cols.len() + col)
            .map(|c| c.doc_start)
            .unwrap_or(0)
    }
}

/// 单元格排版输入：源码态文本或剥标记文本 + 展示坐标样式段。
struct CellInput<'a> {
    text: Cow<'a, str>,
    styles: Option<Vec<(usize, usize, Style)>>,
    doc_start: usize,
    /// 公式 NBSP 占位段（占位后文本坐标）；空 = 无公式。
    maths: Vec<PendingMath>,
}

/// 把单元格可见文本中的公式区间替换为 NBSP 占位（个数 = 图宽 / NBSP 宽，四舍五入）。
/// 返回 (占位后文本, 占位段)；占位段坐标为占位后文本的字节区间。
fn math_placeholders(text: &str, maths: &[CellMath], nbsp_w: f32) -> (String, Vec<PendingMath>) {
    let mut out = String::with_capacity(text.len() + maths.len() * 8);
    let mut pending = Vec::with_capacity(maths.len());
    let mut cursor = 0usize;
    let mut sorted: Vec<&CellMath> = maths.iter().collect();
    sorted.sort_by_key(|m| (m.range.start, m.range.end));
    for m in sorted {
        let (s, e) = (m.range.start.min(text.len()), m.range.end.min(text.len()));
        if s < cursor || e <= s {
            continue; // 防御：区间须有序且不重叠
        }
        out.push_str(&text[cursor..s]);
        let n = ((m.w as f32 / nbsp_w).round() as usize).max(1);
        let start = out.len();
        let nbsp = "\u{A0}".repeat(n);
        out.push_str(&nbsp);
        pending.push(PendingMath {
            range: Range::new(start, start + nbsp.len()),
            w: m.w,
            h: m.h,
            tex: m.tex.clone(),
        });
        cursor = e;
    }
    out.push_str(&text[cursor..]);
    (out, pending)
}

/// `<br>` / `<br/>` / `<BR>` / `<BR/>` → `\n`。
///
/// GFM 允许表格单元格内嵌原始 HTML `<br>` 做硬换行；core 的 cell.text 不会剥离它
/// （InlineKind 无 Html 变体），故保留在可见文本里。本函数把四种大小写/闭合变体
/// 统一替换为单个 `\n`，并按替换位置重映射样式段坐标，使后续 `wrap_cell_text`
/// 的硬行拆分与富文本叠加一致。
#[allow(clippy::type_complexity)]
fn preprocess_line_breaks(
    text: &str,
    styles: Option<&[(usize, usize, Style)]>,
) -> (String, Option<Vec<(usize, usize, Style)>>) {
    // 替换计划：按起始位置升序，记录每个替换造成的位移差（Δ = new_len - old_len）。
    // `<br>` → `\n`:  4→1 → Δ=-3
    // `<br/>` → `\n`: 5→1 → Δ=-4
    // 大写变体同长度，Δ 相同。
    let patterns: &[&str] = &["<br/>", "<BR/>", "<br>", "<BR>"]; // 长的先匹配
    let mut changes: Vec<(usize, isize)> = Vec::new();
    for &pat in patterns {
        let mut from = 0usize;
        while let Some(off) = text[from..].find(pat) {
            let abs = from + off;
            // 检查是否与已记录替换重叠（长的先匹配 → 按起始位置升序）
            if changes.last().is_none_or(|&(s, _)| abs >= s) {
                let delta = 1isize - pat.len() as isize;
                changes.push((abs, delta));
            }
            from = abs + pat.len();
            if from >= text.len() {
                break;
            }
        }
    }
    if changes.is_empty() {
        return (text.to_owned(), styles.map(|s| s.to_vec()));
    }
    // 按起始位置升序排序（上面 patterns 迭代顺序可能不是自然升序）
    changes.sort_by_key(|&(s, _)| s);

    // 构建替换后的文本
    let mut out = String::with_capacity(text.len() + changes.len());
    let mut cursor = 0usize;
    for &(start, delta) in &changes {
        out.push_str(&text[cursor..start]);
        out.push('\n');
        cursor = start + (1 - delta as usize); // 跳过被替换的 pattern 长度
    }
    out.push_str(&text[cursor..]);

    // 重映射样式段：对每个 (s, e) 减去所有 start < pos 的累计 Δ
    let remap = |pos: usize| -> usize {
        let mut acc = 0isize;
        for &(s, d) in &changes {
            if s < pos {
                acc += d;
            } else {
                break; // 已按 start 升序
            }
        }
        (pos as isize + acc).max(0) as usize
    };
    let new_styles = styles.map(|list| {
        list.iter()
            .map(|&(s, e, ref st)| (remap(s), remap(e), *st))
            .collect()
    });
    (out, new_styles)
}

impl LayoutEngine {
    /// 表格排版 + 单元格光栅化。`width` 为版心可用宽（物理像素），`color` 为正文色。
    /// `active` = 活动单元格 flat 索引及其源码文本（显示标记，列宽按源码测量）。
    /// 两遍法：先按不折行求各列内在宽，再按最终列宽折行并求行高。
    pub fn layout_table(
        &mut self,
        table: &Table,
        width: f32,
        color: [u8; 3],
        active: Option<(usize, &str)>,
    ) -> TableLayout {
        self.layout_table_opts(table, width, color, active, None, None, None, None)
    }

    /// 带纯文本覆盖与公式占位的表格排版。`raw_cells`：flat 索引 → 源码原文，
    /// 用于含脚注引用（`[^id]`）的单元格——core 的 `cell.text` 已剥标记，
    /// 渲染态要求保留原文，且不编号、不着色（纯文本降级）。
    /// `maths`：flat 索引 → 该格内联公式（仅非活动格），可见文本区间在排版前
    /// 替换为 NBSP 占位（宽≈图宽），排版后按占位首字形定位供视图层 blit。
    /// `images`：flat 索引 → 该格内联图片（仅非活动格，已加载自然尺寸位图），
    /// 排版后水平并排置于文本右侧。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn layout_table_opts(
        &mut self,
        table: &Table,
        width: f32,
        color: [u8; 3],
        active: Option<(usize, &str)>,
        raw_cells: Option<&HashMap<usize, String>>,
        maths: Option<&HashMap<usize, Vec<CellMath>>>,
        widths: Option<&[f32]>,
        images: Option<&HashMap<usize, Vec<CellImgAsset>>>,
    ) -> TableLayout {
        let scale = self.scale_factor();
        let metrics = Metrics::new(self.doc_font_size() * scale, self.doc_line_height() * scale);
        let pad_x = CELL_PAD_X * scale;
        let pad_y = CELL_PAD_Y * scale;
        let ncols = table.aligns.len();
        let link_color = self.link_color();
        let nbsp_w = if maths.is_some() {
            self.measure_nbsp(metrics)
        } else {
            1.0
        };

        let grid: Vec<(&[TableCell], bool)> = std::iter::once((table.header.as_slice(), true))
            .chain(table.rows.iter().map(|r| (r.as_slice(), false)))
            .collect();

        // 单元格排版输入（两遍法共用）：活动格取源码纯文本，其余取剥标记文本 + 样式段。
        let mut inputs: Vec<(Vec<CellInput<'_>>, bool)> = Vec::with_capacity(grid.len());
        let mut flat = 0usize;
        for (row, bold) in &grid {
            let mut row_in = Vec::with_capacity(ncols);
            for c in 0..ncols {
                let cell = row.get(c);
                let active_src = active.and_then(|(idx, src)| (idx == flat).then_some(src));
                let raw = raw_cells.and_then(|m| m.get(&flat));
                let (raw_text, raw_styles, cell_maths) = match active_src {
                    Some(src) => (Cow::Borrowed(src), None, Vec::new()),
                    // 脚注引用单元格：源码原文 + 无样式（不编号、不着色）。
                    None => match (cell, raw) {
                        (Some(_), Some(raw)) => (Cow::Borrowed(raw.as_str()), None, Vec::new()),
                        (Some(cell), None) => match maths.and_then(|m| m.get(&flat)) {
                            Some(list) if !list.is_empty() => {
                                // 含公式：占位 + 纯文本降级（v0 已知简化，见模块头注释）。
                                let (text, pending) = math_placeholders(&cell.text, list, nbsp_w);
                                (Cow::Owned(text), None, pending)
                            }
                            _ => (
                                Cow::Borrowed(cell.text.as_str()),
                                cell_style_spans(
                                    cell.range,
                                    cell.text.len(),
                                    &cell.inlines,
                                    link_color,
                                ),
                                Vec::new(),
                            ),
                        },
                        (None, _) => (Cow::Borrowed(""), None, Vec::new()),
                    },
                };
                // `<br>` → `\n` 预处理（活动格源码态不做，避免用户看见被改写）。
                let (text, styles) = if active_src.is_some() {
                    (raw_text, raw_styles)
                } else {
                    let (t, s) = preprocess_line_breaks(raw_text.as_ref(), raw_styles.as_deref());
                    (Cow::Owned(t), s)
                };
                row_in.push(CellInput {
                    text,
                    styles,
                    doc_start: cell.map(|x| x.range.start).unwrap_or(0),
                    maths: cell_maths,
                });
                flat += 1;
            }
            inputs.push((row_in, *bold));
        }

        // 第一遍：内在宽度（不折行），列宽 = 该列最宽单元格 + 两侧内边距。
        // 单元格图片总宽也算入内在宽（多图水平并排 + 2px 间距）。
        let mut cols = vec![0.0f32; ncols];
        let mut flat = 0usize;
        for (row, bold) in &inputs {
            for (c, cell) in row.iter().enumerate() {
                let (_, cell_w, _) = self.shape_cell(
                    metrics,
                    cell.text.as_ref(),
                    cell.styles.as_deref(),
                    *bold,
                    None,
                    CellAlign::None,
                    link_color,
                );
                let img_w = images
                    .and_then(|m| m.get(&flat))
                    .map(|list| {
                        if list.is_empty() {
                            0.0
                        } else {
                            list.iter().map(|img| img.w as f32).sum::<f32>()
                                + 2.0 * (list.len() - 1) as f32 // 图片间 2px 间距
                        }
                    })
                    .unwrap_or(0.0);
                cols[c] = cols[c].max(cell_w.max(img_w));
                flat += 1;
            }
        }
        for w in &mut cols {
            *w += 2.0 * pad_x;
        }
        let total: f32 = cols.iter().sum();
        if total > width && total > 0.0 {
            let k = width / total;
            for w in &mut cols {
                *w = (*w * k).max(MIN_COL_WIDTH * scale);
            }
        }
        // 用户拖动列宽（B2，纯视图态）：长度匹配即覆盖内在宽；
        // 总宽超出可用宽时按比例压回（无横向滚动，避免裁切）。
        if let Some(ov) = widths {
            if ov.len() == ncols {
                for (w, o) in cols.iter_mut().zip(ov) {
                    *w = o.max(MIN_COL_WIDTH * scale);
                }
                let t: f32 = cols.iter().sum();
                if t > width && t > 0.0 {
                    let k = width / t;
                    for w in &mut cols {
                        *w *= k;
                    }
                }
            }
        }
        let table_w: f32 = cols.iter().sum();

        // 第二遍：按列宽折行、定行高、光栅化。
        let mut rows = Vec::with_capacity(grid.len());
        let mut cells = Vec::with_capacity(grid.len() * ncols);
        let mut y = 0.0;
        let mut flat = 0usize;
        for (row, bold) in &inputs {
            let mut shaped = Vec::with_capacity(ncols);
            let mut row_h = metrics.line_height;
            for (c, col_w) in cols.iter().enumerate() {
                let cell = &row[c];
                let wrap_w = (*col_w - 2.0 * pad_x).max(1.0);
                let probes: Vec<usize> = cell.maths.iter().map(|m| m.range.start).collect();
                let (buffer, _, h, anchors, decor) = self.shape_cell_probed(
                    metrics,
                    cell.text.as_ref(),
                    cell.styles.as_deref(),
                    *bold,
                    Some(wrap_w),
                    table.aligns[c],
                    link_color,
                    &probes,
                );
                // 公式图顶贴所在视觉行行顶；行高扩到图底，公式不溢出行（不缩图，
                // v0 简化：宁可把行拉高，也不做缩放副本）。
                let mut content_h = h;
                let mut placed = Vec::with_capacity(cell.maths.len());
                for (m, &(line_i, index)) in cell.maths.iter().zip(&anchors) {
                    if let Some((mx, my)) = glyph_anchor(&buffer, line_i, index) {
                        content_h = content_h.max(my + m.h as f32);
                        placed.push(TableCellMath {
                            x: mx,
                            y: my,
                            w: m.w,
                            h: m.h,
                            tex: m.tex.clone(),
                        });
                    }
                }
                // 图片（水平并排置于文本右侧）。
                // 先求 buffer 内文本的最大行宽作为图片起始 x。
                let img_list = images.and_then(|m| m.get(&flat));
                let placed_imgs = if let Some(list) = img_list {
                    if list.is_empty() {
                        Vec::new()
                    } else {
                        let mut max_text_w = 0.0f32;
                        for run in buffer.layout_runs() {
                            for g in run.glyphs {
                                max_text_w = max_text_w.max(g.x + g.w);
                            }
                        }
                        let mut px = if max_text_w > 0.0 {
                            max_text_w + 2.0
                        } else {
                            0.0
                        };
                        let mut out = Vec::with_capacity(list.len());
                        for img in list {
                            // B4：单元格内图片限高 1.5×行高，且不超出单元格剩余宽度；
                            // 等比缩放、只缩不放，位图仍按自然尺寸缓存。
                            let avail_w = (wrap_w - px).max(1.0);
                            let (dw, dh) = super::layout::fit_size_box(
                                img.w,
                                img.h,
                                avail_w,
                                crate::tokens::doc::IMG_TABLE_LINES * metrics.line_height,
                            );
                            content_h = content_h.max(dh as f32);
                            out.push(TableCellImg {
                                x: px,
                                y: 0.0,
                                w: dw,
                                h: dh,
                                pixel_w: img.w,
                                pixel_h: img.h,
                                pixels: img.pixels.clone(),
                            });
                            px += dw as f32 + 2.0;
                        }
                        out
                    }
                } else {
                    Vec::new()
                };
                row_h = row_h.max(content_h);
                shaped.push((
                    buffer,
                    h,
                    content_h,
                    cell.doc_start,
                    placed,
                    placed_imgs,
                    decor,
                ));
                flat += 1;
            }
            rows.push(TableRow {
                top: y,
                height: row_h + 2.0 * pad_y,
            });
            let mut x = 0.0;
            for (c, (buffer, text_h, h, doc_start, maths, imgs, decor)) in
                shaped.into_iter().enumerate()
            {
                // 与正文同一光栅化入口：sub/sup 按字形起点施加基线偏移。
                let (pixels, pixel_w, pixel_h) =
                    self.rasterize_buffer_scripts(&buffer, text_h, color, &decor.scripts);
                let col_w = cols[c];
                cells.push(TableCellLayout {
                    x: x + pad_x,
                    y: y + pad_y,
                    w: (col_w - 2.0 * pad_x).max(1.0),
                    h,
                    doc_start,
                    pixels,
                    pixel_w,
                    pixel_h,
                    maths,
                    images: imgs,
                    under: decor.under,
                    over: decor.over,
                });
                x += col_w;
            }
            y += rows.last().map(|r| r.height).unwrap_or(0.0);
        }

        TableLayout {
            cols,
            rows,
            cells,
            width: table_w,
            height: y,
            x: 0.0,
            active: active.map(|(idx, _)| idx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veil_core::Session;

    fn layout(doc: &str, width: f32) -> TableLayout {
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        LayoutEngine::new().layout_table(&table, width, [0, 0, 0], None)
    }

    #[test]
    fn column_widths_override_honored() {
        // B2：用户拖动后的列宽覆盖优先于内在宽（总宽不超版心时原样采纳）
        let s = Session::open("| a | wide-content |\n| --- | --- |\n| c | d |\n");
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let natural = LayoutEngine::new().layout_table(&table, 600.0, [0, 0, 0], None);
        let ov = [120.0f32, 260.0f32];
        let l = LayoutEngine::new().layout_table_opts(
            &table,
            600.0,
            [0, 0, 0],
            None,
            None,
            None,
            Some(&ov),
            None,
        );
        assert_ne!(natural.cols, l.cols, "覆盖必须改变列宽");
        assert!((l.cols[0] - 120.0).abs() < 0.5 && (l.cols[1] - 260.0).abs() < 0.5);
        assert!(
            (l.width - 380.0).abs() < 1.0,
            "表宽 = 列宽之和: {}",
            l.width
        );
    }

    #[test]
    fn column_widths_override_clamps_and_len_mismatch_ignored() {
        let s = Session::open("| a | b |\n| --- | --- |\n| c | d |\n");
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        // 长度不匹配：忽略覆盖，退回内在宽
        let ignored = LayoutEngine::new().layout_table_opts(
            &table,
            600.0,
            [0, 0, 0],
            None,
            None,
            None,
            Some(&[100.0]),
            None,
        );
        let natural = LayoutEngine::new().layout_table(&table, 600.0, [0, 0, 0], None);
        assert_eq!(ignored.cols, natural.cols);
        // 超窄覆盖被下限托住，且总宽不超版心
        let tiny = LayoutEngine::new().layout_table_opts(
            &table,
            600.0,
            [0, 0, 0],
            None,
            None,
            None,
            Some(&[1.0, 1.0]),
            None,
        );
        assert!(
            tiny.cols.iter().all(|w| *w >= MIN_COL_WIDTH - 0.5),
            "{:?}",
            tiny.cols
        );
        assert!(tiny.width <= 600.0 + 1.0);
    }

    #[test]
    fn columns_follow_content_width() {
        let l = layout("| a | wide-content |\n| --- | --- |\n| c | d |\n", 600.0);
        assert_eq!(l.cols.len(), 2);
        assert_eq!(l.rows.len(), 2);
        assert_eq!(l.cells.len(), 4);
        assert!(l.cols[1] > l.cols[0], "宽内容列应更宽: {:?}", l.cols);
        assert!(l.width > 0.0 && l.height > 0.0);
    }

    #[test]
    fn narrow_width_wraps_and_grows_table() {
        let doc = "| head | note |\n| --- | --- |\n| x | 这是一段很长很长的说明文字 |\n";
        let wide = layout(doc, 800.0);
        let narrow = layout(doc, 120.0);
        assert!(
            narrow.height > wide.height,
            "窄版心须折行增高: {} vs {}",
            narrow.height,
            wide.height
        );
    }

    #[test]
    fn hit_maps_to_cell_content_start() {
        let doc = "| a | b |\n| --- | --- |\n| c | d |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let l = LayoutEngine::new().layout_table(&table, 600.0, [0, 0, 0], None);
        for row in 0..2 {
            for col in 0..2 {
                let cell = &l.cells[row * 2 + col];
                let want = if row == 0 {
                    table.header[col].range.start
                } else {
                    table.rows[0][col].range.start
                };
                assert_eq!(cell.doc_start, want);
                let cx = cell.x + cell.w / 2.0;
                let cy = l.rows[row].top + l.rows[row].height / 2.0;
                assert_eq!(l.hit(cx, cy), want, "row={row} col={col}");
            }
        }
        // 越界钳位：右下角外仍落末行末列
        assert_eq!(l.hit(9999.0, 9999.0), l.cells[3].doc_start);
    }

    #[test]
    fn alignment_offsets_glyphs_within_cell() {
        // 两列同宽（表头同文本）：右对齐的 "x" 光栅含左侧留白，宽于左对齐同行
        let l = layout("| wide | wide |\n| --: | :-- |\n| x | x |\n", 600.0);
        assert_eq!(l.cols[0], l.cols[1]);
        assert!(
            l.cells[2].pixel_w > l.cells[3].pixel_w,
            "右对齐应产生左侧留白"
        );
    }

    #[test]
    fn header_only_table_has_geometry() {
        let l = layout("| a | b |\n| --- | --- |\n", 600.0);
        assert_eq!(l.rows.len(), 1);
        assert_eq!(l.cells.len(), 2);
        assert!(l.height > 0.0);
    }

    #[test]
    fn empty_cell_keeps_row_height() {
        let l = layout("| a | b |\n| --- | --- |\n|  |  |\n", 600.0);
        assert_eq!(l.rows.len(), 2);
        assert!(l.rows[1].height > 0.0);
        assert_eq!(
            l.cells[2].pixels.len() as u32,
            l.cells[2].pixel_w * l.cells[2].pixel_h * 4
        );
    }

    #[test]
    fn narrow_cell_wrapping_keeps_source_bytes() {
        let doc = "| 列一 | 列二 | 列三 |\n| --- | --- | --- |\n| 中文中文中文中文中文中文中文中文》后文 | 中文中文中文中文中文中文中文中文》后文 | 中文中文中文中文中文中文中文中文》后文 |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        LayoutEngine::new().layout_table(&table, 180.0, [0, 0, 0], None);
        assert_eq!(s.serialize(), doc, "展示层折行不得改写 Markdown 源码");
    }

    // ---- 富文本映射与样式（B3） ----

    #[test]
    fn rich_cells_render_styles_distinctly() {
        let doc = "| h | h | h | h | h |\n| --- | --- | --- | --- | --- |\n\
                   | **MMMM** | MMMM | `MMMM` | *MMMM* | [MMMM](http://a) |\n";
        let l = layout(doc, 1200.0);
        // flat：header 0..4，正文 bold=5 plain=6 mono=7 italic=8 link=9
        let (bold, plain, mono, italic, link) = (5, 6, 7, 8, 9);
        assert!(
            l.cells[bold].pixel_w > l.cells[plain].pixel_w,
            "粗体应宽于同文本普通格: {} vs {}",
            l.cells[bold].pixel_w,
            l.cells[plain].pixel_w
        );
        assert_ne!(
            l.cells[mono].pixel_w, l.cells[plain].pixel_w,
            "等宽字体宽度应不同"
        );
        assert_ne!(
            l.cells[italic].pixels, l.cells[plain].pixels,
            "斜体光栅应不同"
        );
        let lc = crate::theme::Theme::light().link;
        let has_link = |c: &TableCellLayout| {
            c.pixels
                .chunks_exact(4)
                .any(|p| p[3] == 255 && p[0] == lc[0] && p[1] == lc[1] && p[2] == lc[2])
        };
        assert!(has_link(&l.cells[link]), "链接格应出现主题链接色像素");
        assert!(!has_link(&l.cells[plain]), "普通格不得出现链接色");
    }

    #[test]
    fn active_cell_shows_source_markup() {
        let doc = "| h |\n| --- |\n| **MMMM** |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let src = s.text_slice(table.rows[0][0].range).unwrap();
        assert_eq!(src, "**MMMM**");
        let plain = LayoutEngine::new().layout_table(&table, 800.0, [0, 0, 0], None);
        assert_eq!(plain.active, None);
        let active =
            LayoutEngine::new().layout_table(&table, 800.0, [0, 0, 0], Some((1, src.as_str())));
        assert_eq!(active.active, Some(1));
        assert!(
            active.cells[1].pixel_w > plain.cells[1].pixel_w + 8,
            "活动格应显示含标记源码: active={} plain={}",
            active.cells[1].pixel_w,
            plain.cells[1].pixel_w
        );
    }

    #[test]
    fn footnote_ref_cell_degrades_to_raw_text() {
        // 单元格内 `[^id]`：按纯文本降级（保留原文、不编号、不着色），
        // 且不得阻塞折行/光栅（含禁则 probe）。
        let doc = "| a [^n] b | c |\n| --- | --- |\n\n[^n]: x\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let raw = s.text_slice(table.header[0].range).unwrap();
        assert_eq!(raw, "a [^n] b", "源码区间应保留脚注标记");
        let mut raw_cells = HashMap::new();
        raw_cells.insert(0usize, raw);
        let narrow = LayoutEngine::new().layout_table_opts(
            &table,
            90.0,
            [0, 0, 0],
            None,
            Some(&raw_cells),
            None,
            None,
            None,
        );
        assert_eq!(narrow.cells.len(), 2);
        assert!(narrow.height > 0.0);
        assert!(!narrow.cells[0].pixels.is_empty(), "降级格仍须正常光栅");
        // 无覆盖时不 panic：富文本降级路径（core 已剥标记）保持可排版。
        let plain = LayoutEngine::new().layout_table(&table, 90.0, [0, 0, 0], None);
        assert_eq!(plain.cells.len(), 2);
    }

    // ---- 单元格行内公式 / 图片 alt（D10 收尾） ----

    /// 取某格 Math 的可见文本区间与 TeX，构造排版输入。
    fn cell_maths(
        s: &Session,
        table: &Table,
        flat: usize,
    ) -> (Vec<(usize, usize, veil_core::Range)>, Vec<CellMath>) {
        let cell = if flat < table.header.len() {
            &table.header[flat]
        } else {
            &table.rows[0][flat - table.header.len()]
        };
        let spans =
            crate::view::layout::cell_math_spans(cell.range, cell.text.len(), &cell.inlines)
                .expect("公式格应可映射");
        let items = spans
            .iter()
            .map(|&(vs, ve, content)| CellMath {
                range: Range::new(vs, ve),
                tex: s.text_slice(content).unwrap(),
                w: 40,
                h: 18,
            })
            .collect();
        (spans, items)
    }

    #[test]
    fn math_cell_places_bitmap_and_active_shows_source() {
        let doc = "| $x^2$ |\n| --- |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        assert_eq!(table.header[0].text, "x^2", "可见文本应剥掉 $ 标记");
        let (spans, items) = cell_maths(&s, &table, 0);
        assert_eq!(spans.len(), 1);
        assert_eq!(&table.header[0].text[spans[0].0..spans[0].1], "x^2");
        assert_eq!(items[0].tex, "x^2", "TeX 内容不含 $ 标记");
        // 放大图宽，确保占位对列宽的影响远大于文本宽度的测量噪声（NBSP 量化）。
        let mut items = items;
        items[0].w = 120;

        let mut maths = HashMap::new();
        maths.insert(0usize, items);
        let with_math = LayoutEngine::new().layout_table_opts(
            &table,
            600.0,
            [0, 0, 0],
            None,
            None,
            Some(&maths),
            None,
            None,
        );
        let plain = LayoutEngine::new().layout_table(&table, 600.0, [0, 0, 0], None);
        assert!(plain.cells[0].maths.is_empty(), "无公式输入时不得画图");
        assert_eq!(with_math.cells[0].maths.len(), 1);
        let m = &with_math.cells[0].maths[0];
        assert_eq!((m.w, m.h, m.tex.as_str()), (120, 18, "x^2"));
        assert!(
            m.x.abs() < 1.0 && m.y >= 0.0,
            "占位应贴内容左缘: ({}, {})",
            m.x,
            m.y
        );
        // 占位宽计入列宽：120px 图明显宽于 "x^2" 文本
        assert!(
            with_math.cols[0] > plain.cols[0] + 40.0,
            "公式占位应计入列宽: {} vs {}",
            with_math.cols[0],
            plain.cols[0]
        );
        assert!(
            with_math.rows[0].height >= m.h as f32,
            "行高须容下公式图: {} < {}",
            with_math.rows[0].height,
            m.h
        );

        // 活动格：完整源码（含 $ 与 ^），不画公式图
        let src = s.text_slice(table.header[0].range).unwrap();
        let active =
            LayoutEngine::new().layout_table(&table, 600.0, [0, 0, 0], Some((0, src.as_str())));
        assert!(active.cells[0].maths.is_empty(), "活动格不得画公式图");
        assert!(
            active.cells[0].pixel_w > plain.cells[0].pixel_w,
            "活动格应包含 $/^ 标记宽度: active={} plain={}",
            active.cells[0].pixel_w,
            plain.cells[0].pixel_w
        );
    }

    #[test]
    fn math_cell_wraps_in_narrow_columns() {
        let doc = "| 中文中文中文中文中文中文 $x^2$ |\n| --- |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let (_, items) = cell_maths(&s, &table, 0);
        let mut maths = HashMap::new();
        maths.insert(0usize, items);
        let wide = LayoutEngine::new().layout_table_opts(
            &table,
            600.0,
            [0, 0, 0],
            None,
            None,
            Some(&maths),
            None,
            None,
        );
        let narrow = LayoutEngine::new().layout_table_opts(
            &table,
            120.0,
            [0, 0, 0],
            None,
            None,
            Some(&maths),
            None,
            None,
        );
        assert_eq!(narrow.cells[0].maths.len(), 1, "折行后公式仍须定位");
        assert!(narrow.height > wide.height, "窄版心须折行增高");
        assert!(
            narrow.cells[0].maths[0].y > 0.0,
            "折到第二行的公式应落在行顶 > 0: {}",
            narrow.cells[0].maths[0].y
        );
        assert_eq!(wide.cells[0].maths[0].y, 0.0, "宽版心公式在第一行");
        // 占位是 NBSP，折行不得改写 Markdown 源码
        assert_eq!(s.serialize(), doc);
    }

    #[test]
    fn image_cell_renders_alt_without_markers() {
        // core 的 cell.text 对 Image 已是 alt（切片 13）；网格只画 alt，不加载外部资源。
        let doc = "| ![图](a.png) | 图 |\n| --- | --- |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        assert_eq!(table.header[0].text, "图");
        assert!(
            !table.header[0].text.contains("![") && !table.header[0].text.contains("]("),
            "可见文本不得露出 Markdown 图片标记: {:?}",
            table.header[0].text
        );
        let l = LayoutEngine::new().layout_table(&table, 800.0, [0, 0, 0], None);
        assert_eq!(
            l.cells[0].pixel_w, l.cells[1].pixel_w,
            "图片格与纯 alt 文本格的光栅宽度应一致"
        );
        assert!(l.cells[0].maths.is_empty());
    }

    #[test]
    fn table_image_height_capped_to_one_and_half_lines() {
        // B4：单元格图片限高 1.5×行高，等比缩放；位图缓存保持自然尺寸。
        use std::collections::HashMap;
        let doc = "| a | b |\n| --- | --- |\n| c | d |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let mut images: HashMap<usize, Vec<CellImgAsset>> = HashMap::new();
        images.insert(
            0,
            vec![CellImgAsset {
                w: 100,
                h: 200,
                pixels: vec![0; 100 * 200 * 4],
            }],
        );
        let l = LayoutEngine::new().layout_table_opts(
            &table,
            600.0,
            [0, 0, 0],
            None,
            None,
            None,
            None,
            Some(&images),
        );
        let img = &l.cells[0].images[0];
        let cap = crate::tokens::doc::IMG_TABLE_LINES * crate::geometry::BASE_LINE_HEIGHT;
        assert!(
            (img.h as f32) <= cap + 0.5,
            "单元格图片高度须 ≤ 1.5 行: {} > {cap}",
            img.h
        );
        assert_eq!((img.pixel_w, img.pixel_h), (100, 200), "位图保持自然尺寸");
        assert!(l.cells[0].h >= img.h as f32, "行高须容纳图片");
    }
}
