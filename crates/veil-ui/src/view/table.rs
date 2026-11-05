//! 表格只读渲染（切片 9）：GFM 表格 → 网格几何 + 单元格文本光栅。
//!
//! v0 取舍：
//! - 单元格显示 core 侧剥除 inline 标记后的可见文本（表头加粗），不施加强调/链接色。
//! - 表宽超出版心时按比例收缩列宽（下限 `MIN_COL_WIDTH`），单元格内按字形折行；
//!   触底仍超宽时表格向右溢出，由 frame 裁剪（v1 不做横向滚动）。
//! - 网格点击落点 = 所在单元格内容起点（进入源码态，随后由既有块排版显示源码）。
//! - 已知简化（同数学块）：跨渲染态表格的选区无高亮。

use cosmic_text::{Align, Attrs, Buffer, Metrics, Shaping, Weight, Wrap};
use veil_core::{CellAlign, Table, TableCell};

use super::layout::{probe_break_violations, LayoutEngine};
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

/// 表格网格排版结果（随块缓存；宽度/DPI/内容变化时重建）。
pub struct TableLayout {
    pub cols: Vec<f32>,
    pub rows: Vec<TableRow>,
    /// 行优先，长度 = rows.len() × cols.len()。
    pub cells: Vec<TableCellLayout>,
    pub width: f32,
    pub height: f32,
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

impl LayoutEngine {
    /// 表格排版 + 单元格光栅化。`width` 为版心可用宽（物理像素），`color` 为正文色。
    /// 两遍法：先按不折行求各列内在宽，再按最终列宽折行并求行高。
    pub fn layout_table(&mut self, table: &Table, width: f32, color: [u8; 3]) -> TableLayout {
        let scale = self.scale_factor();
        let metrics = Metrics::new(BASE_FONT_SIZE * scale, BASE_LINE_HEIGHT * scale);
        let pad_x = CELL_PAD_X * scale;
        let pad_y = CELL_PAD_Y * scale;
        let ncols = table.aligns.len();

        let grid: Vec<(&[TableCell], bool)> = std::iter::once((table.header.as_slice(), true))
            .chain(table.rows.iter().map(|r| (r.as_slice(), false)))
            .collect();

        // 第一遍：内在宽度（不折行），列宽 = 该列最宽单元格 + 两侧内边距。
        let mut cols = vec![0.0f32; ncols];
        for (row, bold) in &grid {
            for (c, w) in cols.iter_mut().enumerate() {
                let text = row.get(c).map(|x| x.text.as_str()).unwrap_or("");
                let (_, cell_w, _) = self.shape_cell(metrics, text, *bold, None, CellAlign::None);
                *w = w.max(cell_w);
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
        for (row, bold) in &grid {
            let mut shaped = Vec::with_capacity(ncols);
            let mut row_h = metrics.line_height;
            for (c, col_w) in cols.iter().enumerate() {
                let cell = row.get(c);
                let text = cell.map(|x| x.text.as_str()).unwrap_or("");
                let wrap_w = (*col_w - 2.0 * pad_x).max(1.0);
                let (buffer, _, h) =
                    self.shape_cell(metrics, text, *bold, Some(wrap_w), table.aligns[c]);
                row_h = row_h.max(h);
                shaped.push((buffer, h, cell.map(|x| x.range.start).unwrap_or(0)));
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
        }
    }

    /// 排版单个单元格文本，返回 (buffer, 文本宽, 文本高)。
    /// `wrap_w` 为 None 时求内在宽（用于列宽计算）；Some 时按字形折行。
    fn shape_cell(
        &mut self,
        metrics: Metrics,
        text: &str,
        bold: bool,
        wrap_w: Option<f32>,
        align: CellAlign,
    ) -> (Buffer, f32, f32) {
        let fs = self.font_system();
        let mut buffer = Buffer::new(fs, metrics);
        let attrs = if bold {
            Attrs::new().weight(Weight::BOLD)
        } else {
            Attrs::new()
        };
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        let align = match align {
            CellAlign::Center => Some(Align::Center),
            CellAlign::Right => Some(Align::Right),
            CellAlign::Left | CellAlign::None => None, // None → LTR 行左对齐
        };
        for line in buffer.lines.iter_mut() {
            line.set_align(align);
        }
        buffer.set_wrap(if wrap_w.is_some() {
            Wrap::Glyph
        } else {
            Wrap::None
        });
        buffer.set_size(wrap_w, None);
        buffer.shape_until_scroll(fs, false);
        probe_break_violations(&buffer);
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

#[cfg(test)]
mod tests {
    use super::*;
    use veil_core::Session;

    fn layout(doc: &str, width: f32) -> TableLayout {
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        LayoutEngine::new().layout_table(&table, width, [0, 0, 0])
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
        let l = LayoutEngine::new().layout_table(&table, 600.0, [0, 0, 0]);
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
}
