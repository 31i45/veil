//! 表格只读渲染（切片 9）：GFM 表格 → 网格几何 + 单元格文本光栅。
//!
//! 取舍：
//! - 非活动单元格按 Markdown 行内语义渲染（粗体/斜体/行内代码/删除线/链接色）；
//!   活动单元格（caret 落在 `cell.range` 内）显示含标记的源码，离开即恢复富文本。
//! - 表宽超出版心时按比例收缩列宽（下限 `MIN_COL_WIDTH`），单元格内按字形折行；
//!   触底仍超宽时表格向右溢出，由 frame 裁剪（v1 不做横向滚动）。
//! - 网格点击落点 = 所在单元格内容起点（进入源码态，随后由既有块排版显示源码）。
//! - 已知简化（同数学块）：跨渲染态表格的选区无高亮。

use cosmic_text::{Align, Attrs, Buffer, FontSystem, Metrics, Shaping, Weight, Wrap};
use std::collections::{BTreeMap, HashMap};
use veil_core::{BlockKind, CellAlign, Table, TableCell};

use super::layout::{attrs_of, cell_style_spans, probe_break_violations, LayoutEngine};
use super::linebreak;
use super::style::Style;
use crate::geometry::{BASE_FONT_SIZE, BASE_LINE_HEIGHT};

/// 单元格水平/垂直内边距（基准字号下），随 DPI 缩放。
const CELL_PAD_X: f32 = 8.0;
const CELL_PAD_Y: f32 = 5.0;
/// 列宽收缩下限（基准字号下）：再窄会把文本挤成逐字折行。
const MIN_COL_WIDTH: f32 = 40.0;

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
}

/// 表格网格排版结果（随块缓存；宽度/DPI/内容/活动格变化时重建）。
pub struct TableLayout {
    pub cols: Vec<f32>,
    pub rows: Vec<TableRow>,
    /// 行优先，长度 = rows.len() × cols.len()。
    pub cells: Vec<TableCellLayout>,
    pub width: f32,
    pub height: f32,
    /// 活动单元格 flat 索引（源码态文本）；None = 全部富文本。
    pub active: Option<usize>,
}

impl TableLayout {
    /// 网格内物理坐标 → 该单元格内容起点（文档绝对偏移）。越界钳到首/末行、首/末列。
    pub fn hit(&self, x: f32, y: f32) -> usize {
        if self.cells.is_empty() || self.cols.is_empty() {
            return 0;
        }
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
    text: &'a str,
    styles: Option<Vec<(usize, usize, Style)>>,
    doc_start: usize,
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
        self.layout_table_opts(table, width, color, active, None)
    }

    /// 带纯文本覆盖的表格排版。`raw_cells`：flat 索引 → 源码原文，
    /// 用于含脚注引用（`[^id]`）的单元格——core 的 `cell.text` 已剥标记，
    /// 渲染态要求保留原文，且不编号、不着色（纯文本降级）。
    pub(crate) fn layout_table_opts(
        &mut self,
        table: &Table,
        width: f32,
        color: [u8; 3],
        active: Option<(usize, &str)>,
        raw_cells: Option<&HashMap<usize, String>>,
    ) -> TableLayout {
        let scale = self.scale_factor();
        let metrics = Metrics::new(BASE_FONT_SIZE * scale, BASE_LINE_HEIGHT * scale);
        let pad_x = CELL_PAD_X * scale;
        let pad_y = CELL_PAD_Y * scale;
        let ncols = table.aligns.len();
        let link_color = self.link_color();

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
                let (text, styles) = match active_src {
                    Some(src) => (src, None),
                    // 脚注引用单元格：源码原文 + 无样式（不编号、不着色）。
                    None => match (cell, raw) {
                        (Some(_), Some(raw)) => (raw.as_str(), None),
                        (Some(cell), None) => (
                            cell.text.as_str(),
                            cell_style_spans(
                                cell.range,
                                cell.text.len(),
                                &cell.inlines,
                                link_color,
                            ),
                        ),
                        (None, _) => ("", None),
                    },
                };
                row_in.push(CellInput {
                    text,
                    styles,
                    doc_start: cell.map(|x| x.range.start).unwrap_or(0),
                });
                flat += 1;
            }
            inputs.push((row_in, *bold));
        }

        // 第一遍：内在宽度（不折行），列宽 = 该列最宽单元格 + 两侧内边距。
        let mut cols = vec![0.0f32; ncols];
        for (row, bold) in &inputs {
            for (c, cell) in row.iter().enumerate() {
                let (_, cell_w, _) = self.shape_cell(
                    metrics,
                    cell.text,
                    cell.styles.as_deref(),
                    *bold,
                    None,
                    CellAlign::None,
                    link_color,
                );
                cols[c] = cols[c].max(cell_w);
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
        let table_w: f32 = cols.iter().sum();

        // 第二遍：按列宽折行、定行高、光栅化。
        let mut rows = Vec::with_capacity(grid.len());
        let mut cells = Vec::with_capacity(grid.len() * ncols);
        let mut y = 0.0;
        for (row, bold) in &inputs {
            let mut shaped = Vec::with_capacity(ncols);
            let mut row_h = metrics.line_height;
            for (c, col_w) in cols.iter().enumerate() {
                let cell = &row[c];
                let wrap_w = (*col_w - 2.0 * pad_x).max(1.0);
                let (buffer, _, h) = self.shape_cell(
                    metrics,
                    cell.text,
                    cell.styles.as_deref(),
                    *bold,
                    Some(wrap_w),
                    table.aligns[c],
                    link_color,
                );
                row_h = row_h.max(h);
                shaped.push((buffer, h, cell.doc_start));
            }
            rows.push(TableRow {
                top: y,
                height: row_h + 2.0 * pad_y,
            });
            let mut x = 0.0;
            for (c, (buffer, h, doc_start)) in shaped.into_iter().enumerate() {
                let (pixels, pixel_w, pixel_h) = self.rasterize_buffer(&buffer, h, color);
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
            active: active.map(|(idx, _)| idx),
        }
    }

    /// 排版单个单元格文本，返回 (buffer, 文本宽, 文本高)。
    /// `wrap_w` 为 None 时求内在宽（用于列宽计算，保留原文）；Some 时由本地显式换行
    /// 生成硬行（禁则感知），再用 `Wrap::None` 排版。`styles` 为剥标记文本坐标，
    /// 活动格/降级路径传 None 走纯文本。
    /// 不用 cosmic 的 `Wrap::WordOrGlyph`：其超窄列宽会退化为逐字形断行，
    /// 绕过 UAX#14 的 CL/OP 禁则（真机复现 `》` 行首违规）。
    #[allow(clippy::too_many_arguments)]
    fn shape_cell(
        &mut self,
        metrics: Metrics,
        text: &str,
        styles: Option<&[(usize, usize, Style)]>,
        bold: bool,
        wrap_w: Option<f32>,
        align: CellAlign,
        link_color: [u8; 3],
    ) -> (Buffer, f32, f32) {
        let attrs = if bold {
            Attrs::new().weight(Weight::BOLD)
        } else {
            Attrs::new()
        };
        let wrapped = match wrap_w {
            Some(width) => wrap_cell_text(self.font_system(), metrics, &attrs, text, width),
            None => WrappedCell::identity(text),
        };
        let fs = self.font_system();
        let mut buffer = Buffer::new(fs, metrics);
        match styles {
            Some(styles) if !wrapped.display.is_empty() => {
                let default_style = Style {
                    bold,
                    ..Default::default()
                };
                let default_attrs = attrs_of(BlockKind::Table, default_style, link_color);
                let spans = rich_spans(&wrapped, styles, bold, link_color);
                if spans.is_empty() {
                    // 映射不可用（切点非字符边界等）→ 降级纯文本
                    buffer.set_text(&wrapped.display, &default_attrs, Shaping::Advanced, None);
                } else {
                    buffer.set_rich_text(spans, &default_attrs, Shaping::Advanced, None);
                }
            }
            _ => buffer.set_text(&wrapped.display, &attrs, Shaping::Advanced, None),
        }
        let align = match align {
            CellAlign::Center => Some(Align::Center),
            CellAlign::Right => Some(Align::Right),
            CellAlign::Left | CellAlign::None => None, // None → LTR 行左对齐
        };
        for line in buffer.lines.iter_mut() {
            line.set_align(align);
        }
        buffer.set_wrap(Wrap::None);
        buffer.set_size(wrap_w, None);
        buffer.shape_until_scroll(fs, false);
        probe_break_violations(&buffer, true);
        let mut w = 0.0f32;
        let mut h = metrics.line_height;
        for run in buffer.layout_runs() {
            for g in run.glyphs {
                w = w.max(g.x + g.w);
            }
            h = h.max(run.line_top + run.line_height);
        }
        (buffer, w, h)
    }
}

/// 折行结果：展示文本 + 展示坐标 ↔ 源字节偏移的映射段。
struct WrappedCell {
    display: String,
    /// (display_start, display_end, src_start, src_end)：只覆盖非换行字符，按序致密。
    segs: Vec<(usize, usize, usize, usize)>,
}

impl WrappedCell {
    fn identity(text: &str) -> Self {
        let segs = if text.is_empty() {
            Vec::new()
        } else {
            vec![(0, text.len(), 0, text.len())]
        };
        Self {
            display: text.to_owned(),
            segs,
        }
    }

    /// 源偏移 → 展示偏移。插入的 `\n` 不属任何源字符；越过源尾则落到展示尾。
    fn src_to_display(&self, p: usize) -> usize {
        if self.segs.is_empty() {
            return 0;
        }
        let i = self.segs.partition_point(|s| s.2 <= p);
        if i == 0 {
            return 0;
        }
        let (ds, _, ss, se) = self.segs[i - 1];
        if p <= se {
            ds + (p - ss)
        } else if i < self.segs.len() {
            self.segs[i].0
        } else {
            self.display.len()
        }
    }
}

/// 样式段（源坐标）→ 展示坐标，致密覆盖展示文本（插入的 `\n` 用默认样式）。
fn display_styles(
    wrapped: &WrappedCell,
    styles: &[(usize, usize, Style)],
) -> Vec<(usize, usize, Style)> {
    let n = wrapped.display.len();
    let mut out: Vec<(usize, usize, Style)> = Vec::new();
    let mut cursor = 0usize;
    for &(s, e, st) in styles {
        let ds = wrapped.src_to_display(s).min(n);
        let de = wrapped.src_to_display(e).min(n);
        if de <= cursor {
            continue;
        }
        let ds = ds.max(cursor);
        if ds > cursor {
            out.push((cursor, ds, Style::default()));
        }
        match out.last_mut() {
            Some(last) if last.1 == ds && last.2 == st => last.1 = de,
            _ => out.push((ds, de, st)),
        }
        cursor = de;
    }
    if cursor < n {
        out.push((cursor, n, Style::default()));
    }
    out
}

/// 展示坐标样式段 → cosmic `set_rich_text` 输入（表头整行加粗）。
fn rich_spans<'a>(
    wrapped: &'a WrappedCell,
    styles: &[(usize, usize, Style)],
    bold: bool,
    link_color: [u8; 3],
) -> Vec<(&'a str, Attrs<'static>)> {
    let display = wrapped.display.as_str();
    let dense = display_styles(wrapped, styles);
    let mut out = Vec::with_capacity(dense.len());
    for (s, e, mut st) in dense {
        if s >= e || e > display.len() {
            continue;
        }
        // 防御：切点必须落在字符边界，否则整体降级纯文本
        if !display.is_char_boundary(s) || !display.is_char_boundary(e) {
            return Vec::new();
        }
        st.bold |= bold;
        out.push((&display[s..e], attrs_of(BlockKind::Table, st, link_color)));
    }
    out
}

/// 对单元格文本做禁则感知的贪婪折行，返回展示文本 + 插入 `\n` 的源偏移。
/// 只处理换行机会与标点归属，不做 shaping/bidi，属于表格展示层局部策略。
fn wrap_cell_text(
    fs: &mut FontSystem,
    metrics: Metrics,
    attrs: &Attrs,
    text: &str,
    width: f32,
) -> WrappedCell {
    let mut display = String::with_capacity(text.len() + 8);
    let mut segs: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut src_base = 0usize;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            display.push('\n');
            src_base += 1;
        }
        let (_, breaks) = wrap_one_line(fs, metrics, attrs, line, width);
        let mut chunk = 0usize;
        for &brk in &breaks {
            push_display_seg(&mut display, &mut segs, line, src_base, chunk, brk);
            display.push('\n');
            chunk = brk;
        }
        push_display_seg(&mut display, &mut segs, line, src_base, chunk, line.len());
        src_base += line.len();
    }
    WrappedCell { display, segs }
}

/// 追加一个源区间到展示文本并记录映射（空区间不产生段）。
fn push_display_seg(
    display: &mut String,
    segs: &mut Vec<(usize, usize, usize, usize)>,
    line: &str,
    src_base: usize,
    start: usize,
    end: usize,
) {
    if start >= end {
        return;
    }
    let ds = display.len();
    display.push_str(&line[start..end]);
    segs.push((ds, display.len(), src_base + start, src_base + end));
}

fn wrap_one_line(
    fs: &mut FontSystem,
    metrics: Metrics,
    attrs: &Attrs,
    text: &str,
    width: f32,
) -> (String, Vec<usize>) {
    if width <= 1.0 || text.is_empty() {
        return (text.to_owned(), Vec::new());
    }
    let clusters = measure_clusters(fs, metrics, attrs, text);
    if clusters.is_empty() {
        return (text.to_owned(), Vec::new());
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut breaks = Vec::new();
    let mut last_byte = 0usize;
    let mut line_start = 0usize;
    let mut line_w = 0.0f32;
    for i in 0..clusters.len() {
        let cluster_w = clusters[i].2;
        if line_w + cluster_w <= width || i == line_start {
            line_w += cluster_w;
            continue;
        }
        // 从当前溢出点向前找最近的允许断点；找不到就允许悬挂，
        // 绝不在「行首禁则 / 行尾禁则」边界断开。
        let mut j = i;
        let mut found = false;
        while j > line_start {
            if boundary_allowed(text, &clusters[j - 1], &clusters[j]) {
                found = true;
                break;
            }
            j -= 1;
        }
        if !found {
            line_w += cluster_w;
            continue;
        }
        let brk = clusters[j].0;
        if brk > last_byte {
            out.push_str(&text[last_byte..brk]);
            out.push('\n');
            breaks.push(brk);
            last_byte = brk;
        }
        line_start = j;
        line_w = clusters[j..=i].iter().map(|c| c.2).sum();
    }
    out.push_str(&text[last_byte..]);
    (out, breaks)
}

/// 按字形簇测量：(起始字节, 结束字节, 宽度)，按起始字节排序。
fn measure_clusters(
    fs: &mut FontSystem,
    metrics: Metrics,
    attrs: &Attrs,
    text: &str,
) -> Vec<(usize, usize, f32)> {
    let mut buffer = Buffer::new(fs, metrics);
    buffer.set_text(text, attrs, Shaping::Advanced, None);
    buffer.set_wrap(Wrap::None);
    buffer.set_size(None, None);
    buffer.shape_until_scroll(fs, false);
    let mut map: BTreeMap<usize, (usize, f32)> = BTreeMap::new();
    for run in buffer.layout_runs() {
        for g in run.glyphs {
            let entry = map.entry(g.start).or_insert((g.end, 0.0));
            entry.0 = entry.0.max(g.end);
            entry.1 += g.w;
        }
    }
    map.into_iter()
        .map(|(start, (end, w))| (start, end, w))
        .collect()
}

fn boundary_allowed(text: &str, left: &(usize, usize, f32), right: &(usize, usize, f32)) -> bool {
    let prev = text.get(left.0..left.1).and_then(|s| s.chars().next_back());
    let next = text.get(right.0..right.1).and_then(|s| s.chars().next());
    match (prev, next) {
        (Some(p), Some(n)) => {
            !linebreak::forbidden_line_end(p) && !linebreak::forbidden_line_start(n)
        }
        _ => true,
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

    /// 每个视觉行的首/尾可见字符（跳过 WJ 与空白）。
    fn line_edges(buffer: &Buffer) -> Vec<(Option<char>, Option<char>)> {
        let visible = |c: char| c != '\u{2060}' && !c.is_whitespace();
        buffer
            .layout_runs()
            .map(|run| {
                let first = run.glyphs.iter().find_map(|g| {
                    run.text
                        .get(g.start..g.end)?
                        .chars()
                        .find(|&c| visible(c))
                });
                let last = run.glyphs.iter().rev().find_map(|g| {
                    run.text
                        .get(g.start..g.end)?
                        .chars()
                        .rev()
                        .find(|&c| visible(c))
                });
                (first, last)
            })
            .collect()
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

    #[test]
    fn narrow_cell_wrap_avoids_kinsoku_boundaries() {
        let mut engine = LayoutEngine::new();
        let metrics = Metrics::new(BASE_FONT_SIZE, BASE_LINE_HEIGHT);
        // 同时覆盖 UAX 承担的 》 与需要胶水的 ……，逐档收窄版心
        let text = "中文中文中文中文中文中文中文中文中文中文中文中文》后文……结束";
        for width in (24..=160).step_by(2) {
            let (buffer, _, _) = engine.shape_cell(
                metrics,
                text,
                None,
                false,
                Some(width as f32),
                CellAlign::None,
                [0, 0, 0],
            );
            for (first, last) in line_edges(&buffer) {
                if let Some(c) = first {
                    assert!(
                        !linebreak::forbidden_line_start(c),
                        "宽度 {width}px：行首出现禁则标点 {c:?}"
                    );
                }
                if let Some(c) = last {
                    assert!(
                        !linebreak::forbidden_line_end(c),
                        "宽度 {width}px：行尾出现禁则标点 {c:?}"
                    );
                }
            }
        }
    }

    // ---- 富文本映射与样式（B3） ----

    fn check_wrap_mapping(text: &str, width: f32) {
        let mut engine = LayoutEngine::new();
        let metrics = Metrics::new(BASE_FONT_SIZE, BASE_LINE_HEIGHT);
        let attrs = Attrs::new();
        let w = wrap_cell_text(engine.font_system(), metrics, &attrs, text, width);
        let mut covered = vec![false; w.display.len()];
        let (mut prev_d, mut prev_s) = (0usize, 0usize);
        for &(ds, de, ss, se) in &w.segs {
            assert!(ds >= prev_d && ss >= prev_s, "映射段须按序: {text:?}");
            assert_eq!(&w.display[ds..de], &text[ss..se], "映射段逐字节还原源区间");
            covered[ds..de].fill(true);
            prev_d = de;
            prev_s = se;
        }
        for (i, ch) in w.display.char_indices() {
            if ch != '\n' {
                assert!(covered[i], "非换行字符未被映射覆盖: {text:?} @ {i}");
            }
        }
        // 插入的换行之外，展示文本与源文本一致（两侧都剥离换行比较）
        let strip = |s: &str| s.replace('\n', "");
        assert_eq!(strip(&w.display), strip(text));
    }

    #[test]
    fn wrap_mapping_preserves_source_bytes() {
        for text in [
            "普通文本",
            "中文中文中文中文中文中文中文中文》后文",
            "第一行\n第二行…》结束",
            "中文中文中文中文中文中文中文中文中文中文》后文……结束",
        ] {
            for width in [12.0, 30.0, 60.0, 400.0] {
                check_wrap_mapping(text, width);
            }
        }
        // 不折行用恒等映射
        let w = WrappedCell::identity("中文");
        assert_eq!(w.display, "中文");
        assert_eq!(w.segs, vec![(0, 6, 0, 6)]);
        assert!(WrappedCell::identity("").segs.is_empty());
    }

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
        let narrow = LayoutEngine::new()
            .layout_table_opts(&table, 90.0, [0, 0, 0], None, Some(&raw_cells));
        assert_eq!(narrow.cells.len(), 2);
        assert!(narrow.height > 0.0);
        assert!(!narrow.cells[0].pixels.is_empty(), "降级格仍须正常光栅");
        // 无覆盖时不 panic：富文本降级路径（core 已剥标记）保持可排版。
        let plain = LayoutEngine::new().layout_table(&table, 90.0, [0, 0, 0], None);
        assert_eq!(plain.cells.len(), 2);
    }
}
