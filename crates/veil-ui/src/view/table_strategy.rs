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
#[derive(Clone)]
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

/// 占位后的图片段（占位文本坐标），排版后映射为 `TableCellImg`。
struct PendingImg {
    range: Range,
    asset: usize,
    w: u32,
    h: u32,
}

/// 表格单元格内联图片（输入）：已加载的位图 + 自然尺寸（无 URL，失败已在上层过滤）。
/// `alt` 为该图 alt 在单元格可见文本中的区间：图片已加载时替换为无字形占位，
/// 避免 alt 与缩略图重复显示并保留原 inline 位置；`None` 表示无法定位/无 alt。
pub(crate) struct CellImgAsset {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
    pub alt: Option<Range>,
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
    /// 活动单元格源码字形档位：网格内可按点击坐标落点、按光标偏移画光标。
    /// 非活动格为 None（富文本字形与文档偏移不是一一对应）。
    pub(crate) caret_map: Option<CellCaretMap>,
    pub pixels: Vec<u8>,
    pub pixel_w: u32,
    pub pixel_h: u32,
    /// 非活动格内联公式（相对内容原点）；空 = 无公式或活动格（源码态）。
    pub maths: Vec<TableCellMath>,
    /// 非活动格内联图片（相对内容原点的原 inline 占位位置）；空 = 无图或活动格。
    pub(crate) images: Vec<TableCellImg>,
    /// 非活动格行内前置装饰（相对内容原点的物理矩形；与正文同语义）。
    pub(crate) under: Vec<(f32, f32, f32, f32, UnderKind)>,
    /// 非活动格行内后置装饰（abbr 虚线下划线）。
    pub(crate) over: Vec<(f32, f32, f32, f32, OverKind)>,
}

/// 活动单元格源码的字形档位（相对内容原点，升序）。
/// 只对活动格建立：它显示的是含标记的源码纯文本，字形与文档偏移一一对应，
/// 因此可以精确落点、精确画光标；非活动格显示富文本，偏移不可直接映射。
pub(crate) struct CellCaretMap {
    runes: Vec<CellRune>,
    /// 源码字节长度（光标偏移上限）。
    pub len: usize,
    /// 空行/空格的兜底行高。
    line_h: f32,
}

struct CellRune {
    start: usize,
    end: usize,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl CellCaretMap {
    fn from_buffer(buffer: &cosmic_text::Buffer, line_h: f32) -> Self {
        let mut runes = Vec::new();
        let mut len = 0usize;
        for run in buffer.layout_runs() {
            for g in run.glyphs {
                runes.push(CellRune {
                    start: g.start,
                    end: g.end,
                    x: g.x,
                    y: run.line_top,
                    w: g.w,
                    h: run.line_height,
                });
                len = len.max(g.end);
            }
        }
        runes.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
        Self { runes, len, line_h }
    }

    /// 内容坐标 → 字节偏移：同视觉行内按字形中点就近取插入点。
    pub(crate) fn offset_at(&self, x: f32, y: f32) -> Option<usize> {
        let first = self.runes.first()?;
        // y 落在哪一视觉行（越界钳到首/末行）
        let line_y = self
            .runes
            .iter()
            .filter(|r| r.y <= y && y < r.y + r.h)
            .map(|r| r.y)
            .next()
            .unwrap_or_else(|| {
                if y < first.y {
                    first.y
                } else {
                    self.runes.last().map(|r| r.y).unwrap_or(first.y)
                }
            });
        let mut best = None;
        for r in self.runes.iter().filter(|r| r.y == line_y) {
            if x < r.x + r.w * 0.5 {
                return Some(r.start);
            }
            best = Some(r.end);
        }
        best
    }

    /// 字节偏移 → 内容坐标 (x, y, h)。偏移在字形内按比例插值（CJK/拉丁同一口径）。
    pub(crate) fn caret_at(&self, offset: usize) -> Option<(f32, f32, f32)> {
        let first = self.runes.first()?;
        if offset <= first.start {
            return Some((first.x, first.y, first.h));
        }
        let mut prev = None;
        for r in &self.runes {
            if offset <= r.start {
                return Some((r.x, r.y, r.h));
            }
            if offset <= r.end {
                let ratio = (offset - r.start) as f32 / (r.end - r.start).max(1) as f32;
                return Some((r.x + r.w * ratio, r.y, r.h));
            }
            prev = Some(r);
        }
        prev.map(|r| (r.x + r.w, r.y, r.h))
    }

    /// 光标矩形的行高（空单元格兜底）。
    pub(crate) fn line_height(&self) -> f32 {
        self.runes.first().map_or(self.line_h, |r| r.h)
    }
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
    /// 网格内物理坐标 → 文档绝对偏移。活动格按源码字形精确落点；
    /// 其余格（富文本）落到单元格内容起点。越界钳到首/末行、首/末列。
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
        let Some(cell) = self
            .cells
            .get(row * self.cols.len() + col)
        else {
            return 0;
        };
        if let Some(map) = cell.caret_map.as_ref() {
            if let Some(rel) = map.offset_at(x - cell.x, y - cell.y) {
                return cell.doc_start + rel.min(map.len);
            }
        }
        cell.doc_start
    }
}

/// 单元格排版输入：源码态文本或剥标记文本 + 展示坐标样式段。
struct CellInput<'a> {
    text: Cow<'a, str>,
    styles: Option<Vec<(usize, usize, Style)>>,
    doc_start: usize,
    /// 源码区间字节数：展示文本与之相等（且无样式/公式/图片）时，
    /// 字形偏移即文档偏移，可建精确落点档位。
    src_len: usize,
    /// 公式 NBSP 占位段（占位后文本坐标）；空 = 无公式。
    maths: Vec<PendingMath>,
    /// 图片 NBSP 占位段（占位后文本坐标）；空 = 无可定位图片。
    images: Vec<PendingImg>,
}

/// 把单元格可见文本中的公式区间替换为 NBSP 占位（个数 = ceil(图宽 / NBSP 宽)），
/// 并同步重映射已生成的图片占位。返回 (占位后文本, 公式占位段)。
fn math_placeholders(
    text: &str,
    maths: &[CellMath],
    nbsp_w: f32,
    images: &mut [PendingImg],
) -> (String, Vec<PendingMath>) {
    let mut out = String::with_capacity(text.len() + maths.len() * 8);
    let mut pending = Vec::with_capacity(maths.len());
    let mut replacements = Vec::with_capacity(maths.len());
    let mut cursor = 0usize;
    let mut sorted: Vec<&CellMath> = maths.iter().collect();
    sorted.sort_by_key(|m| (m.range.start, m.range.end));
    for m in sorted {
        let (s, e) = (m.range.start.min(text.len()), m.range.end.min(text.len()));
        if s < cursor || e <= s {
            continue; // 防御：区间须有序且不重叠
        }
        out.push_str(&text[cursor..s]);
        // 向上取整：占位必须覆盖图宽，避免公式压住后续文本。
        let n = ((m.w as f32 / nbsp_w).ceil() as usize).max(1);
        let start = out.len();
        let nbsp = "\u{A0}".repeat(n);
        out.push_str(&nbsp);
        let target = Range::new(start, start + nbsp.len());
        pending.push(PendingMath {
            range: target,
            w: m.w,
            h: m.h,
            tex: m.tex.clone(),
        });
        replacements.push((m.range, target));
        cursor = e;
    }
    out.push_str(&text[cursor..]);

    for image in images {
        let remap = |pos: usize, is_end: bool| -> usize {
            let mut delta = 0isize;
            for &(source, target) in &replacements {
                if pos < source.start
                    || (is_end && pos == source.start && source.start == source.end)
                {
                    break;
                }
                if pos >= source.end {
                    delta +=
                        (target.end - target.start) as isize - (source.end - source.start) as isize;
                    continue;
                }
                return target.start;
            }
            (pos as isize + delta).max(0) as usize
        };
        image.range = Range::new(
            remap(image.range.start, false),
            remap(image.range.end, true),
        );
    }
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

/// 已成功加载图片的 alt 可见区间原位替换为 NBSP 占位，排版后再把缩略图叠到占位处。
///
/// 样式段与同格数学区间一起重映射，再生成数学占位，避免「含公式就保留重复 alt」
/// 的同根缺陷。返回 None 表示没有可定位区间（保持原文，如加载失败时显示 alt）。
#[allow(clippy::type_complexity)]
fn image_placeholders(
    text: &str,
    styles: Option<&[(usize, usize, Style)]>,
    assets: &[CellImgAsset],
    maths: &[CellMath],
    nbsp_w: f32,
    max_w: f32,
    max_h: f32,
) -> Option<(
    String,
    Option<Vec<(usize, usize, Style)>>,
    Vec<CellMath>,
    Vec<PendingImg>,
)> {
    let mut ordered: Vec<(usize, Range)> = assets
        .iter()
        .enumerate()
        .filter_map(|(i, asset)| asset.alt.map(|range| (i, range)))
        .filter(|(_, r)| {
            r.start <= r.end
                && r.end <= text.len()
                && text.is_char_boundary(r.start)
                && text.is_char_boundary(r.end)
        })
        .collect();
    if ordered.is_empty() {
        return None;
    }
    ordered.sort_by_key(|(i, r)| (r.start, r.end, *i));

    let mut out = String::with_capacity(text.len());
    let mut pending = Vec::with_capacity(ordered.len());
    let mut replacements: Vec<(Range, Range)> = Vec::with_capacity(ordered.len());
    let mut cursor = 0usize;
    for (asset, source) in ordered {
        if source.start < cursor {
            continue; // 防御：重叠区间无法确定唯一 inline 顺序
        }
        out.push_str(&text[cursor..source.start]);
        let (w, h) = super::layout::fit_size_box(assets[asset].w, assets[asset].h, max_w, max_h);
        let n = ((w as f32 / nbsp_w).ceil() as usize).max(1);
        let start = out.len();
        out.push_str(&"\u{A0}".repeat(n));
        let target = Range::new(start, out.len());
        pending.push(PendingImg {
            range: target,
            asset,
            w,
            h,
        });
        replacements.push((source, target));
        cursor = source.end;
    }
    if pending.is_empty() {
        return None;
    }
    out.push_str(&text[cursor..]);

    // 起点按替换后位置映射；终点在零宽 alt 前停下，避免空 alt 占位并入前一样式。
    let remap = |pos: usize, is_end: bool| -> usize {
        let mut delta = 0isize;
        for &(source, target) in &replacements {
            if pos < source.start || (is_end && pos == source.start && source.start == source.end) {
                break;
            }
            if pos >= source.end {
                delta +=
                    (target.end - target.start) as isize - (source.end - source.start) as isize;
                continue;
            }
            return target.start;
        }
        (pos as isize + delta).max(0) as usize
    };
    let new_styles = styles.map(|list| {
        list.iter()
            .filter_map(|&(s, e, ref st)| {
                let (s, e) = (remap(s, false), remap(e, true));
                (s < e).then_some((s, e, *st))
            })
            .collect()
    });
    let new_maths = maths
        .iter()
        .filter_map(|m| {
            let (s, e) = (remap(m.range.start, false), remap(m.range.end, true));
            (s < e).then(|| CellMath {
                range: Range::new(s, e),
                ..m.clone()
            })
        })
        .collect();
    Some((out, new_styles, new_maths, pending))
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
    /// `images`：flat 索引 → 该格内联图片（仅非活动格，已加载自然尺寸位图）。
    /// 可定位的 Markdown 图片按原 alt 位置生成 NBSP 占位，排版后在占位处 blit。
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
        let nbsp_w = if maths.is_some() || images.is_some() {
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
                let mut cell_images = Vec::new();
                let (raw_text, raw_styles, cell_maths) = match active_src {
                    Some(src) => (Cow::Borrowed(src), None, Vec::new()),
                    // 脚注引用单元格：源码原文 + 无样式（不编号、不着色）。
                    None => match (cell, raw) {
                        (Some(_), Some(raw)) => (Cow::Borrowed(raw.as_str()), None, Vec::new()),
                        (Some(cell), None) => {
                            let original_maths = maths
                                .and_then(|m| m.get(&flat))
                                .cloned()
                                .unwrap_or_default();
                            let base_styles = if original_maths.is_empty() {
                                cell_style_spans(
                                    cell.range,
                                    cell.text.len(),
                                    &cell.inlines,
                                    link_color,
                                )
                            } else {
                                None // 含公式格沿用纯文本降级（见模块头注释）。
                            };
                            let assets = images
                                .and_then(|m| m.get(&flat))
                                .map(Vec::as_slice)
                                .unwrap_or(&[]);
                            // 已加载图片在原 alt 位置留白，再同步重映射公式与图片占位。
                            let (visible, styles, visible_maths, mut pending_imgs) =
                                match image_placeholders(
                                    &cell.text,
                                    base_styles.as_deref(),
                                    assets,
                                    &original_maths,
                                    nbsp_w,
                                    width.max(1.0),
                                    crate::tokens::doc::IMG_TABLE_LINES * metrics.line_height,
                                ) {
                                    Some((t, s, m, i)) => (Cow::Owned(t), s, m, i),
                                    None => (
                                        Cow::Borrowed(cell.text.as_str()),
                                        base_styles,
                                        original_maths,
                                        Vec::new(),
                                    ),
                                };
                            let input = if visible_maths.is_empty() {
                                (visible, styles, Vec::new())
                            } else {
                                let (text, pending) = math_placeholders(
                                    &visible,
                                    &visible_maths,
                                    nbsp_w,
                                    &mut pending_imgs,
                                );
                                (Cow::Owned(text), None, pending)
                            };
                            cell_images = pending_imgs;
                            input
                        }
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
                    src_len: cell.map(|x| x.range.len()).unwrap_or(0),
                    maths: cell_maths,
                    images: cell_images,
                });
                flat += 1;
            }
            inputs.push((row_in, *bold));
        }

        // 第一遍：内在宽度（不折行），列宽 = 该列最宽单元格 + 两侧内边距。
        // 可定位图片已计入文本占位；仅无法定位的降级图片沿用尾部宽度。
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
                        let (sum, count) = list
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| !cell.images.iter().any(|p| p.asset == *i))
                            .fold((0.0, 0usize), |(sum, count), (_, img)| {
                                (sum + img.w as f32, count + 1)
                            });
                        if count == 0 {
                            0.0
                        } else {
                            sum + 2.0 * (count - 1) as f32
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
        // 极窄窗口 + 多列：MIN_COL_WIDTH 地板可能仍让总宽超出可用宽。再按比例压一次
        // （绝对下限 0.4×MIN_COL_WIDTH），保证表格不横向溢出、后续块不被压住；
        // 列宽变窄只增加折行，不改变表格语义。
        let hard_min = MIN_COL_WIDTH * scale * 0.4;
        let sum: f32 = cols.iter().sum();
        if sum > width && sum > 0.0 {
            let k = width / sum;
            for w in &mut cols {
                *w = (*w * k).max(hard_min);
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
                let probes: Vec<usize> = cell
                    .maths
                    .iter()
                    .map(|m| m.range.start)
                    .chain(cell.images.iter().map(|i| i.range.start))
                    .collect();
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
                // 活动格是源码纯文本；纯文本格（剥标记后与源码逐字节等长）同样可精确落点。
                // 富文本格的字形偏移与文档偏移不一一对应，保持落格首。
                // 剥标记只会让可见文本变短（标记/转义/`<br>` 都是收缩），
                // 因此「等长 + 无公式/图片占位」即等价于可见文本 == 源码文本，
                // 字形偏移与文档偏移一一对应，可建精确落点档位。
                let plain = cell.maths.is_empty()
                    && cell.images.is_empty()
                    && cell.text.len() == cell.src_len
                    && cell.src_len > 0;
                let caret_map = if plain || matches!(active, Some((idx, _)) if idx == flat) {
                    Some(CellCaretMap::from_buffer(&buffer, metrics.line_height))
                } else {
                    None
                };
                // 公式图顶贴所在视觉行行顶；行高扩到图底，公式不溢出行（不缩图，
                // v0 简化：宁可把行拉高，也不做缩放副本）。
                let mut content_h = h;
                let (math_anchors, img_anchors) = anchors.split_at(cell.maths.len());
                let mut placed = Vec::with_capacity(cell.maths.len());
                for (m, &(line_i, index)) in cell.maths.iter().zip(math_anchors) {
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
                let img_list = images.and_then(|m| m.get(&flat));
                let mut placed_imgs = Vec::with_capacity(cell.images.len());
                if let Some(list) = img_list {
                    // Markdown 图片：按原 alt 占位所在视觉行和字形位置绘制。
                    for (pending, &(line_i, index)) in cell.images.iter().zip(img_anchors) {
                        if let Some((ix, iy)) = glyph_anchor(&buffer, line_i, index) {
                            let img = &list[pending.asset];
                            content_h = content_h.max(iy + pending.h as f32);
                            placed_imgs.push(TableCellImg {
                                x: ix,
                                y: iy,
                                w: pending.w,
                                h: pending.h,
                                pixel_w: img.w,
                                pixel_h: img.h,
                                pixels: img.pixels.clone(),
                            });
                        }
                    }

                    // HTML img 等没有可见文本坐标的资产保留原尾部降级，不删除其派生文本。
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
                    for (i, img) in list.iter().enumerate() {
                        if cell.images.iter().any(|p| p.asset == i) {
                            continue;
                        }
                        let avail_w = (wrap_w - px).max(1.0);
                        let (dw, dh) = super::layout::fit_size_box(
                            img.w,
                            img.h,
                            avail_w,
                            crate::tokens::doc::IMG_TABLE_LINES * metrics.line_height,
                        );
                        content_h = content_h.max(dh as f32);
                        placed_imgs.push(TableCellImg {
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
                }
                row_h = row_h.max(content_h);
                shaped.push((
                    buffer,
                    h,
                    content_h,
                    cell.doc_start,
                    placed,
                    placed_imgs,
                    decor,
                    caret_map,
                ));
                flat += 1;
            }
            rows.push(TableRow {
                top: y,
                height: row_h + 2.0 * pad_y,
            });
            let mut x = 0.0;
            for (c, (buffer, text_h, h, doc_start, maths, imgs, decor, caret_map)) in
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
                    caret_map,
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

    fn loaded_assets(alts: &[Range]) -> Vec<CellImgAsset> {
        alts.iter()
            .map(|&alt| CellImgAsset {
                w: 8,
                h: 8,
                pixels: vec![0; 8 * 8 * 4],
                alt: Some(alt),
            })
            .collect()
    }

    fn has_ink_between(cell: &TableCellLayout, start: f32, end: f32) -> bool {
        let x0 = start.floor().max(0.0) as u32;
        let x1 = end.ceil().max(0.0) as u32;
        if x0 >= x1.min(cell.pixel_w) {
            return false;
        }
        (x0..x1.min(cell.pixel_w)).any(|x| {
            (0..cell.pixel_h).any(|y| {
                let i = ((y * cell.pixel_w + x) * 4 + 3) as usize;
                cell.pixels[i] != 0
            })
        })
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
                let cx = cell.x + 0.5; // 内容左缘：落格首
                let cy = l.rows[row].top + l.rows[row].height / 2.0;
                assert_eq!(l.hit(cx, cy), want, "row={row} col={col}");
            }
        }
        // 越界钳位：右下角外落末行末列的字形末尾
        assert_eq!(l.hit(9999.0, 9999.0), l.cells[3].doc_start + 1);
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
    fn active_cell_hit_and_caret_round_trip() {
        // 活动格显示源码纯文本：点击落点必须精确到字形，光标也要能逐字移动。
        let doc = "| h |\n| --- |\n| **MMMM** |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let src = s.text_slice(table.rows[0][0].range).unwrap();
        let start = table.rows[0][0].range.start;
        let l = LayoutEngine::new().layout_table(&table, 800.0, [0, 0, 0], Some((1, src.as_str())));
        let cell = &l.cells[1];
        let map = cell.caret_map.as_ref().expect("活动格应有字形档位");
        assert_eq!(map.len, src.len());
        assert_eq!(l.hit(cell.x - 4.0, cell.y + 1.0), start, "左侧内边距落格首");
        assert_eq!(
            l.hit(cell.x + cell.w + 40.0, cell.y + 1.0),
            start + src.len(),
            "右侧空白落格尾"
        );
        let mut last_x = -1.0f32;
        for off in 0..=src.len() {
            let (dx, dy, h) = map.caret_at(off).expect("档位应覆盖 [0, len]");
            assert!(h > 0.0);
            assert!(dx >= last_x, "光标 x 应随偏移单调不减：off={off}");
            last_x = dx;
            let got = l.hit(cell.x + dx + 0.25, cell.y + dy + h * 0.5);
            assert_eq!(got, start + off, "偏移 {off} 的落点应回到自身");
        }
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
    fn loaded_cell_image_preserves_prefix_suffix_order() {
        let doc = "| 前缀 ![DUPLICATE_ALT](a.png) 后缀 |\n| --- |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let cell = &table.header[0];
        let alts =
            crate::view::layout::cell_image_alt_spans(cell.range, cell.text.len(), &cell.inlines)
                .expect("alt 区间应可映射");
        assert_eq!(&cell.text[alts[0].start..alts[0].end], "DUPLICATE_ALT");
        let mut images = HashMap::new();
        images.insert(
            0,
            vec![CellImgAsset {
                w: 12,
                h: 8,
                pixels: vec![0; 12 * 8 * 4],
                alt: Some(alts[0]),
            }],
        );
        let laid = LayoutEngine::new().layout_table_opts(
            &table,
            600.0,
            [0, 0, 0],
            None,
            None,
            None,
            None,
            Some(&images),
        );
        let cell = &laid.cells[0];
        let img = &cell.images[0];
        assert!(
            has_ink_between(cell, 0.0, img.x),
            "前缀必须排在图片左侧: x={}",
            img.x
        );
        assert!(
            !has_ink_between(cell, img.x, img.x + img.w as f32),
            "原 alt 位置只能保留无字形的图片占位"
        );
        assert!(
            has_ink_between(cell, img.x + img.w as f32, cell.pixel_w as f32),
            "后缀必须排在图片右侧: img={}..{}, text_w={}",
            img.x,
            img.x + img.w as f32,
            cell.pixel_w
        );
    }

    #[test]
    fn loaded_cell_images_preserve_multiple_inline_positions() {
        let doc = "| 前 ![一](a.png) 中 ![二](b.png) 后 |\n| --- |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let cell = &table.header[0];
        let alts =
            crate::view::layout::cell_image_alt_spans(cell.range, cell.text.len(), &cell.inlines)
                .expect("多图 alt 区间应可映射");
        assert_eq!(alts.len(), 2);
        let mut images = HashMap::new();
        images.insert(
            0,
            vec![
                CellImgAsset {
                    w: 8,
                    h: 8,
                    pixels: vec![0; 8 * 8 * 4],
                    alt: Some(alts[0]),
                },
                CellImgAsset {
                    w: 10,
                    h: 8,
                    pixels: vec![0; 10 * 8 * 4],
                    alt: Some(alts[1]),
                },
            ],
        );
        let laid = LayoutEngine::new().layout_table_opts(
            &table,
            600.0,
            [0, 0, 0],
            None,
            None,
            None,
            None,
            Some(&images),
        );
        let cell = &laid.cells[0];
        assert_eq!(cell.images.len(), 2);
        let first = &cell.images[0];
        let second = &cell.images[1];
        assert!(second.x >= first.x + first.w as f32);
        assert!(has_ink_between(cell, 0.0, first.x), "首图前须有前缀");
        assert!(
            has_ink_between(cell, first.x + first.w as f32, second.x),
            "两图之间必须保留中间文本: first={}..{}, second.x={}",
            first.x,
            first.x + first.w as f32,
            second.x
        );
        assert!(
            has_ink_between(cell, second.x + second.w as f32, cell.pixel_w as f32),
            "第二图后须有后缀"
        );
    }

    #[test]
    fn loaded_cell_image_hides_duplicate_alt_text() {
        // REN-SWEEP-008：图片加载成功后只显示缩略图；alt 不得再以文本重复出现。
        let doc = "| ![小图](a.png) | x |\n| --- | --- |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let cell = &table.header[0];
        assert_eq!(cell.text, "小图", "core 可见文本即 alt");
        let alts =
            crate::view::layout::cell_image_alt_spans(cell.range, cell.text.len(), &cell.inlines)
                .expect("alt 区间应可映射");
        assert_eq!(alts, vec![Range::new(0, "小图".len())]);
        let mut images: HashMap<usize, Vec<CellImgAsset>> = HashMap::new();
        images.insert(
            0,
            vec![CellImgAsset {
                w: 8,
                h: 8,
                pixels: vec![0; 8 * 8 * 4],
                alt: Some(alts[0]),
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
        assert_eq!(
            l.cells[0].images[0].x, 0.0,
            "alt 已剔除，缩略图应贴单元格内容左缘"
        );
        // 未加载（无 asset）时仍显示 alt 文本：缩略图不存在，文本保留。
        let plain = LayoutEngine::new().layout_table(&table, 600.0, [0, 0, 0], None);
        assert!(plain.cells[0].images.is_empty());
        assert!(
            plain.cells[0].pixel_w > l.cells[0].pixel_w,
            "失败回退必须保留 alt 字形"
        );
        assert_eq!(s.serialize(), doc, "显示层去重不得改写文档");
    }

    #[test]
    fn loaded_cell_images_remap_math_nested_multiple_and_empty_alts() {
        // 嵌套 inline 的 alt、多图与空 alt 必须按 visible-text 坐标逐张映射。
        let doc = "| 前 **![嵌](a.png)** 中 ![二](b.png) 后 ![](c.png) 尾 |\n| --- |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let cell = &table.header[0];
        assert_eq!(cell.text, "前 嵌 中 二 后  尾");
        let alts =
            crate::view::layout::cell_image_alt_spans(cell.range, cell.text.len(), &cell.inlines)
                .expect("嵌套图片 alt 应可映射");
        assert_eq!(alts.len(), 3);
        assert_eq!(&cell.text[alts[0].start..alts[0].end], "嵌");
        assert_eq!(&cell.text[alts[1].start..alts[1].end], "二");
        assert_eq!(alts[2].start, alts[2].end, "空 alt 是零宽区间");
        let (placed, _, _, pending) = image_placeholders(
            &cell.text,
            None,
            &loaded_assets(&alts),
            &[],
            4.0,
            600.0,
            36.0,
        )
        .expect("三张成功加载的图片都应原位占位");
        assert_eq!(placed.replace('\u{A0}', ""), "前  中  后  尾");
        assert_eq!(pending.len(), 3);
        assert!(pending[0].range.start < pending[1].range.start);
        assert!(pending[1].range.start < pending[2].range.start);
        let (partial, _, _, pending) = image_placeholders(
            &cell.text,
            None,
            &loaded_assets(&alts[..1]),
            &[],
            4.0,
            600.0,
            36.0,
        )
        .expect("只对成功加载的第一张生成占位");
        assert_eq!(pending.len(), 1);
        assert_eq!(
            partial.replace('\u{A0}', ""),
            "前  中 二 后  尾",
            "未加载第二张的 alt 必须保留"
        );

        // 同格数学不能成为保留重复 alt 的例外：图片原位占位并同步重映射数学。
        let doc = "| 前 $x^2$ 中 ![图](a.png) 后 |\n| --- |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let cell = &table.header[0];
        let (_, maths) = cell_maths(&s, &table, 0);
        let alts =
            crate::view::layout::cell_image_alt_spans(cell.range, cell.text.len(), &cell.inlines)
                .unwrap();
        let (placed, _, remapped, pending) = image_placeholders(
            &cell.text,
            None,
            &loaded_assets(&alts),
            &maths,
            4.0,
            600.0,
            36.0,
        )
        .expect("同格数学的图片 alt 也应原位占位");
        assert_eq!(placed.replace('\u{A0}', ""), "前 x^2 中  后");
        assert_eq!(pending.len(), 1);
        assert!(remapped[0].range.end <= pending[0].range.start);
        assert_eq!(remapped.len(), 1);
        assert_eq!(
            &placed[remapped[0].range.start..remapped[0].range.end],
            "x^2"
        );

        let mut math_map = HashMap::new();
        math_map.insert(0, maths);
        let mut image_map = HashMap::new();
        image_map.insert(
            0,
            vec![CellImgAsset {
                w: 8,
                h: 8,
                pixels: vec![0; 8 * 8 * 4],
                alt: Some(alts[0]),
            }],
        );
        let laid = LayoutEngine::new().layout_table_opts(
            &table,
            600.0,
            [0, 0, 0],
            None,
            None,
            Some(&math_map),
            None,
            Some(&image_map),
        );
        assert_eq!(laid.cells[0].maths.len(), 1, "删 alt 后公式仍须定位");
        assert_eq!(laid.cells[0].images.len(), 1);
        let math = &laid.cells[0].maths[0];
        let image = &laid.cells[0].images[0];
        assert!(
            math.x + math.w as f32 <= image.x,
            "数学在图片前时，二次占位仍须保持相对顺序"
        );
        assert_eq!(s.serialize(), doc);
    }

    #[test]
    fn html_cell_image_does_not_strip_visible_text() {
        let doc = "| 左 <img src=\"a.png\" alt=\"HTMLalt\"> 右 |\n| --- |\n";
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0];
        let table = s.table(id).unwrap();
        let cell = &table.header[0];
        assert_eq!(cell.text, "左 HTMLalt 右");
        let alts =
            crate::view::layout::cell_image_alt_spans(cell.range, cell.text.len(), &cell.inlines);
        assert!(
            alts.is_none(),
            "HTML alt 是派生文本，映射不可用时必须安全保留"
        );
        assert!(image_placeholders(&cell.text, None, &[], &[], 4.0, 600.0, 36.0).is_none());
        let plain = LayoutEngine::new().layout_table(&table, 600.0, [0, 0, 0], None);
        let mut images = HashMap::new();
        images.insert(
            0,
            vec![CellImgAsset {
                w: 8,
                h: 8,
                pixels: vec![0; 8 * 8 * 4],
                alt: None,
            }],
        );
        let loaded = LayoutEngine::new().layout_table_opts(
            &table,
            600.0,
            [0, 0, 0],
            None,
            None,
            None,
            None,
            Some(&images),
        );
        assert_eq!(loaded.cells[0].images.len(), 1);
        assert_eq!(
            loaded.cells[0].pixel_w, plain.cells[0].pixel_w,
            "HTML img 成功加载后也不得删 visible alt 文本"
        );
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
                alt: None,
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
