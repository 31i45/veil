//! 表格视图层（切片 9）：单元格文本 shaping/折行/样式段映射 + 网格帧绘制。
//!
//! 两件事：
//! 1. 把单元格文本按最终列宽排成可光栅的 buffer（`shape_cell`）：本地显式硬行 +
//!    `Wrap::None`，避免 cosmic 的 `Wrap::WordOrGlyph` 在超窄列宽下退化为逐字形断行、
//!    绕过 UAX#14 的 CL/OP 禁则；样式段从剥标记文本坐标映射到展示坐标后交给富文本。
//! 2. 把 `table_strategy::TableLayout` 的几何落成帧像素：活动格底色、表头底色、
//!    单元格位图与网格线。
//!
//! 不负责：列宽分配、行高累积、命中计算与活动格判定（`table_strategy`）。

use cosmic_text::{Align, Attrs, Buffer, Cursor, FontSystem, Metrics, Shaping, Weight, Wrap};
use std::collections::BTreeMap;
use veil_core::{BlockId, BlockKind, CellAlign};

use super::layout::{
    attrs_of, doc_family, probe_break_violations, script_shift_px, LayoutEngine, OverKind,
    UnderKind,
};
use super::linebreak;
use super::style::Style;
use super::table_strategy::TableCellLayout;
use crate::app::App;
use crate::document::block_index::BlockGeom;
use crate::view::Frame;

/// 单元格行内装饰（与正文 `BlockLayout::decorations_under/over` 同语义）：
/// 展示文本区间经 buffer 几何解析为相对单元格内容原点的物理矩形，
/// paint_table 复用 `paint_under_decor` / `paint_over_decor` 同一套绘制规则。
#[derive(Default)]
pub(crate) struct CellDecor {
    pub under: Vec<(f32, f32, f32, f32, UnderKind)>,
    pub over: Vec<(f32, f32, f32, f32, OverKind)>,
    /// Sub/Sup 展示区间 → 物理像素 dy（光栅化时按字形起点施加）。
    pub scripts: Vec<(usize, usize, f32)>,
}

/// 整格是否由同一种富文本样式连续覆盖；是则折行测量可复用最终 shaping 的 attrs。
fn uniform_cell_style(text_len: usize, styles: Option<&[(usize, usize, Style)]>) -> Option<Style> {
    if text_len == 0 {
        return None;
    }
    let styles = styles?;
    let &(_, _, uniform) = styles.first()?;
    let mut cursor = 0usize;
    for &(s, e, st) in styles {
        if s != cursor || e <= s || st != uniform {
            return None;
        }
        cursor = e;
    }
    (cursor == text_len).then_some(uniform)
}

impl LayoutEngine {
    /// 排版单个单元格文本，返回 (buffer, 文本宽, 文本高)。
    /// `wrap_w` 为 None 时求内在宽（用于列宽计算，保留原文）；Some 时由本地显式换行
    /// 生成硬行（禁则感知），再用 `Wrap::None` 排版。`styles` 为剥标记文本坐标，
    /// 活动格/降级路径传 None 走纯文本。
    /// 不用 cosmic 的 `Wrap::WordOrGlyph`：其超窄列宽会退化为逐字形断行，
    /// 绕过 UAX#14 的 CL/OP 禁则（真机复现 `》` 行首违规）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_cell(
        &mut self,
        metrics: Metrics,
        text: &str,
        styles: Option<&[(usize, usize, Style)]>,
        bold: bool,
        wrap_w: Option<f32>,
        align: CellAlign,
        link_color: [u8; 3],
    ) -> (Buffer, f32, f32) {
        let (buffer, w, h, _, _) =
            self.shape_cell_probed(metrics, text, styles, bold, wrap_w, align, link_color, &[]);
        (buffer, w, h)
    }

    /// 同 `shape_cell`，另把 `probes`（占位后的源偏移）映射为（逻辑行号, 行内偏移），
    /// 供公式 NBSP 占位在折行后定位（`wrap_cell_text` 插入的 `\n` 已计入）；
    /// 并产出与正文同语义的行内装饰（code/mark/kbd 底色、abbr 下划线、sub/sup 偏移）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shape_cell_probed(
        &mut self,
        metrics: Metrics,
        text: &str,
        styles: Option<&[(usize, usize, Style)]>,
        bold: bool,
        wrap_w: Option<f32>,
        align: CellAlign,
        link_color: [u8; 3],
        probes: &[usize],
    ) -> (Buffer, f32, f32, Vec<(usize, usize)>, CellDecor) {
        let attrs = {
            let a = if bold {
                Attrs::new().weight(Weight::BOLD)
            } else {
                Attrs::new()
            };
            match doc_family(self.doc_font(), self.doc_face()) {
                Some(family) => a.family(family),
                None => a,
            }
        };
        let doc_font = self.doc_font();
        let doc_face = self.doc_face();
        let mono_family = self.mono_family();
        // 与 rich_spans 相同叠加表头粗体；整格 mono 尤其不能用正文字体测量，
        // 否则列宽按 mono 求出后，折行却会按另一套宽度把 code 拆成 cod/e。
        let wrap_attrs = uniform_cell_style(text.len(), styles).map_or_else(
            || attrs.clone(),
            |mut st| {
                st.bold |= bold;
                attrs_of(
                    BlockKind::Table,
                    st,
                    link_color,
                    doc_font,
                    doc_face,
                    mono_family,
                    metrics.font_size,
                )
            },
        );
        // 折行测量必须与最终 shaping 使用同一套字形属性：混排（粗体/行内代码等）
        // 若按默认比例字宽折行，等宽/粗体渲染会明显更宽，整行墨迹越过列宽并被画到
        // 表格（乃至窗口）之外（issue #11「表格出格了」）。这里按样式段分别测量。
        let wrap_runs: Vec<(usize, usize, Attrs<'static>)> = match styles {
            Some(styles) if !styles.is_empty() => {
                let mut runs = Vec::with_capacity(styles.len() + 1);
                let mut cursor = 0usize;
                for &(s, e, mut st) in styles {
                    let s = s.min(text.len());
                    let e = e.min(text.len());
                    if s > cursor {
                        runs.push((cursor, s, attrs.clone()));
                    }
                    let s = s.max(cursor);
                    if e > s {
                        st.bold |= bold;
                        runs.push((
                            s,
                            e,
                            attrs_of(
                                BlockKind::Table,
                                st,
                                link_color,
                                doc_font,
                                doc_face,
                                mono_family,
                                metrics.font_size,
                            ),
                        ));
                        cursor = e;
                    }
                }
                if cursor < text.len() {
                    runs.push((cursor, text.len(), attrs.clone()));
                }
                runs
            }
            _ => Vec::new(),
        };
        let wrapped = match wrap_w {
            Some(width) if wrap_runs.is_empty() => {
                wrap_cell_text(self.font_system(), metrics, &wrap_attrs, &[], text, width)
            }
            Some(width) => {
                wrap_cell_text(self.font_system(), metrics, &attrs, &wrap_runs, text, width)
            }
            None => WrappedCell::identity(text),
        };
        let fs = self.font_system();
        let mut buffer = Buffer::new(fs, metrics);
        // 展示坐标致密样式段：富文本 attrs 与装饰共用同一份，避免两套口径分叉。
        let dense = match styles {
            Some(styles) if !wrapped.display.is_empty() => display_styles(&wrapped, styles),
            _ => Vec::new(),
        };
        match styles {
            Some(_) if !wrapped.display.is_empty() => {
                let default_style = Style {
                    bold,
                    ..Default::default()
                };
                let default_attrs = attrs_of(
                    BlockKind::Table,
                    default_style,
                    link_color,
                    doc_font,
                    doc_face,
                    mono_family,
                    metrics.font_size,
                );
                let spans = rich_spans(
                    &wrapped.display,
                    &dense,
                    bold,
                    link_color,
                    doc_font,
                    doc_face,
                    mono_family,
                    metrics.font_size,
                );
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
        let offsets = probes
            .iter()
            .map(|&p| {
                let d = wrapped.src_to_display(p);
                let head = wrapped.display.get(..d).unwrap_or("");
                let line_start = head.rfind('\n').map_or(0, |i| i + 1);
                let line_i = head.bytes().filter(|&b| b == b'\n').count();
                (line_i, d - line_start)
            })
            .collect();
        let decor = cell_decor(&buffer, &wrapped.display, &dense, self.scale_factor());
        (buffer, w, h, offsets, decor)
    }
}

/// 展示坐标样式段 → 行内装饰（与正文 `BlockLayout` 同语义）：
/// 前置底色/边框、abbr 下划线、sub/sup 基线偏移，均解析为 buffer 物理矩形/区间。
fn cell_decor(
    buffer: &Buffer,
    display: &str,
    dense: &[(usize, usize, Style)],
    scale: f32,
) -> CellDecor {
    let mut under_ranges: Vec<(usize, usize, UnderKind)> = Vec::new();
    let mut over_ranges: Vec<(usize, usize, OverKind)> = Vec::new();
    let mut scripts: Vec<(usize, usize, f32)> = Vec::new();
    for &(s, e, st) in dense {
        if s >= e {
            continue;
        }
        if st.code_bg {
            under_ranges.push((s, e, UnderKind::InlineCodeBg));
        }
        if st.mark_bg {
            under_ranges.push((s, e, UnderKind::MarkBg));
        }
        if st.kbd_frame {
            under_ranges.push((s, e, UnderKind::KbdFrame));
        }
        if let Some(bg) = st.bg {
            under_ranges.push((s, e, UnderKind::StyleBg(bg)));
        }
        if st.dotted_underline {
            over_ranges.push((s, e, OverKind::DottedUnderline));
        }
        if st.underline {
            over_ranges.push((s, e, OverKind::Underline));
        }
        if st.overline {
            over_ranges.push((s, e, OverKind::Overline));
        }
        if let Some(shift) = st.baseline_shift {
            scripts.push((s, e, script_shift_px(shift, scale)));
        }
    }
    let line_bases = display_line_bases(display);
    let mut decor = CellDecor::default();
    for (s, e, kind) in under_ranges {
        for r in buffer_range_rects(buffer, &line_bases, s, e) {
            decor.under.push((r.0, r.1, r.2, r.3, kind.clone()));
        }
    }
    for (s, e, kind) in over_ranges {
        for r in buffer_range_rects(buffer, &line_bases, s, e) {
            decor.over.push((r.0, r.1, r.2, r.3, kind.clone()));
        }
    }
    decor.scripts = scripts;
    decor
}

/// 展示文本各逻辑行起始字节偏移（与 `LayoutRun::line_i` 对齐）。
fn display_line_bases(text: &str) -> Vec<usize> {
    let mut out = vec![0usize];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            out.push(i + 1);
        }
    }
    out
}

/// 展示文本区间 → 物理矩形集（与正文 `selection_rects` 同一口径：逐视觉行求交，
/// BiDi/样式切换处单行可产多段）。
fn buffer_range_rects(
    buffer: &Buffer,
    line_bases: &[usize],
    start: usize,
    end: usize,
) -> Vec<(f32, f32, f32, f32)> {
    let mut rects = Vec::new();
    for run in buffer.layout_runs() {
        let base = line_bases.get(run.line_i).copied().unwrap_or(0);
        let (s, e) = (start.saturating_sub(base), end.saturating_sub(base));
        if s >= e {
            continue;
        }
        for (x, w) in run.highlight(Cursor::new(run.line_i, s), Cursor::new(run.line_i, e)) {
            if w > 0.0 {
                rects.push((x, run.line_top, w, run.line_height));
            }
        }
    }
    rects
}

/// (逻辑行号, 行内偏移) → (字形左缘 x, 所在视觉行行顶)。占位首字节必落某字形起点；
/// 行尾/空行退回最后一个字形右缘（与正文 `layout_block` 的占位定位规则一致）。
pub(crate) fn glyph_anchor(buffer: &Buffer, line_i: usize, index: usize) -> Option<(f32, f32)> {
    for run in buffer.layout_runs() {
        if run.line_i != line_i {
            continue;
        }
        let run_end = run.glyphs.last().map(|g| g.end).unwrap_or(0);
        if index > run_end && !run.glyphs.is_empty() {
            continue;
        }
        for g in run.glyphs {
            if index <= g.start {
                return Some((g.x, run.line_top));
            }
        }
        if let Some(g) = run.glyphs.last() {
            return Some((g.x + g.w, run.line_top));
        }
    }
    None
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
#[allow(clippy::too_many_arguments)] // 与 attrs_of 同参：字族/字族名/等宽族 + 样式
fn rich_spans<'a>(
    display: &'a str,
    dense: &[(usize, usize, Style)],
    bold: bool,
    link_color: [u8; 3],
    doc_font: crate::theme::DocFontFamily,
    doc_face: Option<&'static str>,
    mono_family: Option<&'static str>,
    pt: f32,
) -> Vec<(&'a str, Attrs<'static>)> {
    let mut out = Vec::with_capacity(dense.len());
    for &(s, e, mut st) in dense {
        if s >= e || e > display.len() {
            continue;
        }
        // 防御：切点必须落在字符边界，否则整体降级纯文本
        if !display.is_char_boundary(s) || !display.is_char_boundary(e) {
            return Vec::new();
        }
        st.bold |= bold;
        out.push((
            &display[s..e],
            attrs_of(
                BlockKind::Table,
                st,
                link_color,
                doc_font,
                doc_face,
                mono_family,
                pt,
            ),
        ));
    }
    out
}

/// 对单元格文本做禁则感知的贪婪折行，返回展示文本 + 插入 `\n` 的源偏移。
/// 只处理换行机会与标点归属，不做 shaping/bidi，属于表格展示层局部策略。
fn wrap_cell_text(
    fs: &mut FontSystem,
    metrics: Metrics,
    attrs: &Attrs,
    runs: &[(usize, usize, Attrs<'static>)],
    text: &str,
    width: f32,
) -> WrappedCell {
    // 逐行测量：cosmic-text 对含 `\n` 的多行缓冲区不保证字形偏移是全文口径，
    // 按整段测量再切行会把第二行起的偏移算错（曾导致字节切片越界）。
    let mut display = String::with_capacity(text.len() + 8);
    let mut segs: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut src_base = 0usize;
    let mut line_off = 0usize;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            display.push('\n');
            src_base += 1;
        }
        let line_end = line_off + line.len();
        let line_runs: Vec<(usize, usize, Attrs<'static>)> = runs
            .iter()
            .filter_map(|(s, e, a)| {
                let s = (*s).max(line_off);
                let e = (*e).min(line_end);
                (e > s).then(|| (s - line_off, e - line_off, a.clone()))
            })
            .collect();
        let clusters = measure_clusters_styled(fs, metrics, attrs, line, &line_runs);
        let (_, breaks) = wrap_one_line(line, &clusters, width);
        let mut chunk = 0usize;
        for &brk in &breaks {
            push_display_seg(&mut display, &mut segs, line, src_base, chunk, brk);
            display.push('\n');
            chunk = brk;
        }
        push_display_seg(&mut display, &mut segs, line, src_base, chunk, line.len());
        src_base += line.len();
        line_off = line_end + 1;
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

fn wrap_one_line(text: &str, clusters: &[(usize, usize, f32)], width: f32) -> (String, Vec<usize>) {
    if width <= 1.0 || text.is_empty() {
        return (text.to_owned(), Vec::new());
    }
    if clusters.is_empty() {
        return (text.to_owned(), Vec::new());
    }
    let atomic = short_atomic_tokens(text, clusters, width);
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
        // 绝不在「行首禁则 / 行尾禁则」边界或短原子 token 内断开。
        let mut j = i;
        let mut found = false;
        while j > line_start {
            let brk = clusters[j].0;
            let inside_atomic = atomic.iter().any(|&(s, e)| s < brk && brk < e);
            if !inside_atomic && boundary_allowed(text, &clusters[j - 1], &clusters[j]) {
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

/// 可容纳的 ASCII 单词与反引号 code span 整体折行；超宽 token 不列入，保留字形级兜底。
fn short_atomic_tokens(
    text: &str,
    clusters: &[(usize, usize, f32)],
    width: f32,
) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let open = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            let run = i - open;
            let mut search = i;
            let mut close = None;
            while search < bytes.len() {
                if bytes[search] != b'`' {
                    search += 1;
                    continue;
                }
                let close_start = search;
                while search < bytes.len() && bytes[search] == b'`' {
                    search += 1;
                }
                if search - close_start == run {
                    close = Some(search);
                    break;
                }
            }
            if let Some(end) = close {
                tokens.push((open, end));
                i = end;
                continue;
            }
            i = open;
        }
        if bytes[i].is_ascii_alphanumeric() {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
                i += 1;
            }
            tokens.push((start, i));
        } else {
            i += text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    tokens.retain(|&(start, end)| {
        let token_w: f32 = clusters
            .iter()
            .filter(|c| c.0 < end && c.1 > start)
            .map(|c| c.2)
            .sum();
        token_w <= width
    });
    tokens
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

/// 按样式段分别测量簇宽，返回全文口径的 `(start, end, w)`。
/// 混排单元格（行内代码/粗体）必须按实际字形属性量宽，否则折行宽会小于
/// 最终 shaping 宽度，墨迹会溢出列宽（issue #11）。
fn measure_clusters_styled(
    fs: &mut FontSystem,
    metrics: Metrics,
    fallback: &Attrs,
    text: &str,
    runs: &[(usize, usize, Attrs<'static>)],
) -> Vec<(usize, usize, f32)> {
    if runs.is_empty() {
        return measure_clusters(fs, metrics, fallback, text);
    }
    let mut out: Vec<(usize, usize, f32)> = Vec::new();
    let mut cursor = 0usize;
    for (s, e, attrs) in runs {
        let s = (*s).min(text.len());
        let e = (*e).min(text.len());
        if e <= s {
            continue;
        }
        if s > cursor {
            let mut part = measure_clusters(fs, metrics, fallback, &text[cursor..s]);
            shift_clusters(&mut part, cursor);
            out.extend(part);
        }
        let mut part = measure_clusters(fs, metrics, attrs, &text[s..e]);
        shift_clusters(&mut part, s);
        out.extend(part);
        cursor = e;
    }
    if cursor < text.len() {
        let mut part = measure_clusters(fs, metrics, fallback, &text[cursor..]);
        shift_clusters(&mut part, cursor);
        out.extend(part);
    }
    out.sort_by_key(|c| c.0);
    out
}

fn shift_clusters(clusters: &mut [(usize, usize, f32)], by: usize) {
    for c in clusters {
        c.0 += by;
        c.1 += by;
    }
}

fn boundary_allowed(text: &str, left: &(usize, usize, f32), right: &(usize, usize, f32)) -> bool {
    let prev = text.get(left.0..left.1).and_then(|s| s.chars().next_back());
    let next = text.get(right.0..right.1).and_then(|s| s.chars().next());
    match (prev, next) {
        // 占位内部（NBSP 连续段）不可断开，否则一张公式图会跨视觉行；
        // 占位与前后文本之间仍允许断行（公式可整体折到下一行）。
        (Some('\u{A0}'), Some('\u{A0}')) => false,
        (Some(p), Some(n)) => {
            !linebreak::forbidden_line_end(p) && !linebreak::forbidden_line_start(n)
        }
        _ => true,
    }
}

/// 引用行首 `>` 位置（缩进之后）；用于统计单行引用层级（与 layout 同规则）。
fn nested_quote_marker_pos(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    (line.as_bytes().get(indent) == Some(&b'>')).then_some(indent)
}

/// 宽表横向滚动条的屏幕几何（物理像素）。绘制与拖动共用同一口径。
pub(crate) struct TableScrollbarGeom {
    pub(crate) track_x: f32,
    pub(crate) y: f32,
    pub(crate) track_w: f32,
    pub(crate) bar_h: f32,
    pub(crate) thumb_x: f32,
    pub(crate) thumb_w: f32,
    pub(crate) max_pan: f32,
}

impl App {
    /// 宽表横向滚动条几何；不需要横滚（表宽 ≤ 容器可用宽）或打印态返回 `None`。
    pub(crate) fn table_scrollbar_geom(
        &self,
        t: &crate::view::table::TableLayout,
        top: i32,
    ) -> Option<TableScrollbarGeom> {
        if self.window_state.print.is_some() {
            return None;
        }
        let avail = (self.content_width() - t.x).max(1.0);
        if t.width <= avail + 0.5 {
            return None;
        }
        let scale = self.input_state.scale;
        let bar_h = (crate::tokens::doc::TABLE_SCROLL_H * scale).max(2.0);
        let track_w = avail;
        let max_pan = t.width - avail;
        let min_thumb = (crate::tokens::doc::TABLE_SCROLL_MIN_THUMB * scale)
            .min(track_w)
            .max(2.0);
        let thumb_w = (avail * avail / t.width).max(min_thumb).min(track_w);
        let travel = (track_w - thumb_w).max(0.0);
        let ratio = if max_pan > 0.0 {
            (t.pan / max_pan).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let track_x = self.content_x() + t.x;
        Some(TableScrollbarGeom {
            track_x,
            y: top as f32 + t.height.ceil(),
            track_w,
            bar_h,
            thumb_x: track_x + travel * ratio,
            thumb_w,
            max_pan,
        })
    }

    /// 容器内表格的引用条：按引用层数在表格左侧画竖条（与引用块同色/同宽）。
    pub(crate) fn paint_quote_rails(&self, frame: &mut Frame, g: &BlockGeom, top: i32) {
        if g.container.quote_depth == 0 {
            return;
        }
        let scale = self.input_state.scale;
        let base = self.content_x() + self.table_indent(g.container);
        let bh = self.block_height(g) as u32;
        for level in 0..u32::from(g.container.quote_depth) {
            let x = base - (8.0 + level as f32 * 6.0) * scale;
            frame.fill_rect(
                x as i32,
                top,
                (3.0 * scale).max(1.0) as u32,
                bh,
                self.input_state.theme.grid,
            );
        }
    }

    /// 引用块内嵌引用的次级引用条（REN-SWEEP-004）：标记全消隐后，按源行层级
    /// 为第 2..=N 级补画竖条，使 `> > …` 的层级在视觉上可辨。
    /// 行 y 由版面偏移反查（折行时仍贴合该源行的首个视觉行）。
    pub(crate) fn paint_nested_quote_rails(
        &self,
        frame: &mut Frame,
        g: &BlockGeom,
        l: &crate::view::layout::BlockLayout,
        top: i32,
    ) {
        let Some(text) = self.editor.session().text_slice(g.range) else {
            return;
        };
        let mut lines: Vec<(usize, u8)> = Vec::new();
        let mut off = 0usize;
        for line in text.split_inclusive('\n') {
            let mut s = 0usize;
            let mut depth = 0u8;
            while let Some(p) = nested_quote_marker_pos(&line[s..]) {
                s += p + 1;
                depth = depth.saturating_add(1);
            }
            lines.push((off, depth));
            off += line.len();
        }
        let max = lines.iter().map(|x| x.1).max().unwrap_or(0);
        if max < 2 {
            return;
        }
        let scale = self.input_state.scale;
        let rail_w = (crate::tokens::doc::QUOTE_RAIL * scale).max(1.0) as u32;
        let step = crate::tokens::doc::QUOTE_STEP * scale;
        let inset = crate::tokens::doc::QUOTE_INSET * scale;
        for level in 2..=max {
            let mut i = 0usize;
            while i < lines.len() {
                if lines[i].1 < level {
                    i += 1;
                    continue;
                }
                let start = i;
                while i + 1 < lines.len() && lines[i + 1].1 >= level {
                    i += 1;
                }
                let y0 = self.engine.caret_xy(l, lines[start].0).1;
                let y1 = if i + 1 < lines.len() {
                    self.engine.caret_xy(l, lines[i + 1].0).1
                } else {
                    l.height
                };
                let h = (y1 - y0).max(0.0) as u32;
                if h > 0 {
                    let x = self.content_x() - inset + (level as f32 - 1.0) * step;
                    frame.fill_rect(
                        x as i32,
                        top + y0 as i32,
                        rail_w,
                        h,
                        self.input_state.theme.grid,
                    );
                }
                i += 1;
            }
        }
    }

    /// 绘制表格网格：活动单元格底色 → 表头底色 → 单元格文本 → 网格线（先底后字，线压边）。
    /// 活动单元格由排版键记录的 `active` 判定，光标绘制复用既有 `paint_caret_and_preedit` 路径。
    /// 选区跨网格态表格时不画高亮（同数学块 v0 已知简化）。
    pub(crate) fn paint_table(&self, frame: &mut Frame, id: BlockId, top: i32) {
        let Some(t) = self.cache.table(id, self.table_key(id)) else {
            return;
        };
        // 可读宽表：网格整体左移 pan（pan 与命中/光标同一坐标口径）。
        let x0 = (self.content_x() + t.x - t.pan) as i32;
        let theme = self.input_state.theme;

        // 表格所有墨迹（底色、文本、网格、公式/图片）裁切在「容器可用宽 + 表格矩形」内：
        // 折行/占位仍可能让单元格位图略宽于列宽，不裁切会画到版心与窗口之外。
        let clip_x0 = (self.content_x() + t.x).floor() as i32;
        let clip_x1 = (self.content_x() + self.content_width()).ceil() as i32;
        let clip_h = t.height.ceil().max(1.0) as u32;
        frame.set_clip(clip_x0, top, (clip_x1 - clip_x0).max(0) as u32, clip_h);

        // 活动单元格：排版键已记录 caret 命中的格（源码态显示），画选中色底色
        if let Some(cell_layout) = t.active.and_then(|idx| t.cells.get(idx)) {
            let cx = x0 + cell_layout.x.round() as i32;
            let cy = top + cell_layout.y.round() as i32;
            let cw = cell_layout.w.round().max(1.0) as u32;
            let ch = cell_layout.h.round().max(1.0) as u32;
            frame.fill_rect(cx, cy, cw, ch, theme.sel);
        }

        if let Some(first) = t.rows.first() {
            frame.fill_rect(
                x0,
                top,
                t.width.ceil() as u32,
                first.height.ceil() as u32,
                crate::tokens::palette::table_head_bg(&theme),
            );
        }
        self.paint_table_cells(frame, x0, top, &t.cells);
        let w = t.width.ceil().max(1.0) as u32;
        let h = t.height.ceil().max(1.0) as u32;
        let mut x: f32 = 0.0;
        for cw in &t.cols {
            frame.fill_rect(x0 + x.round() as i32, top, 1, h, theme.grid);
            x += cw;
        }
        let right = t.width.round().max(1.0) as i32;
        frame.fill_rect(x0 + right - 1, top, 1, h, theme.grid);
        frame.fill_rect(x0, top, w, 1, theme.grid);
        for r in &t.rows {
            frame.fill_rect(
                x0,
                top + (r.top + r.height).round() as i32 - 1,
                w,
                1,
                theme.grid,
            );
        }
        frame.clear_clip();
        self.paint_table_scrollbar(frame, id, top, t);
    }

    /// 宽表横向滚动条：轨道 = 容器可用宽，滑块长 = 可见宽/表宽，位置 = pan。
    /// 仅在需要横滚（表宽 > 可用宽）时出现；可拖动滑块或 Shift+滚轮平移。
    fn paint_table_scrollbar(
        &self,
        frame: &mut Frame,
        id: BlockId,
        top: i32,
        t: &crate::view::table::TableLayout,
    ) {
        let Some(g) = self.table_scrollbar_geom(t, top) else {
            return;
        };
        let theme = self.input_state.theme;
        let radius = g.bar_h * 0.5;
        // 轨道/滑块沿用滚动条 thumb 色，仅调整透明度以区分层级（主题化）。
        let track = [theme.thumb[0], theme.thumb[1], theme.thumb[2], 40];
        let active = matches!(self.ui_state.tables.scroll_drag, Some((d, _)) if d == id);
        let alpha = if active { 220 } else { 160 };
        let thumb = [theme.thumb[0], theme.thumb[1], theme.thumb[2], alpha];
        frame.fill_rounded_rect(
            g.track_x.round() as i32,
            g.y.round() as i32,
            g.track_w.round().max(1.0) as u32,
            g.bar_h.round().max(2.0) as u32,
            radius,
            track,
        );
        frame.fill_rounded_rect(
            g.thumb_x.round() as i32,
            g.y.round() as i32,
            g.thumb_w.round().max(2.0) as u32,
            g.bar_h.round().max(2.0) as u32,
            radius,
            thumb,
        );
    }

    /// 画给定单元格的文本位图 + 行内装饰 / 公式 / 图片（网格线由调用方负责）。
    fn paint_table_cells(&self, frame: &mut Frame, x0: i32, top: i32, cells: &[TableCellLayout]) {
        let theme = self.input_state.theme;
        let inline_key = self.inline_key();
        for c in cells {
            let cy = top + c.y.round() as i32;
            if cy + c.pixel_h as i32 <= 0 || cy >= frame.height as i32 {
                continue; // 行在视口外
            }
            let tx = x0 + c.x.round() as i32;
            // 前置装饰（code/mark/kbd 底色）画在文本位图之下，与正文同一套绘制规则。
            // 叠放顺序由 `UnderKind::rank()` 决定（契约 §6.2），稳定排序保留同层构造顺序。
            let mut under: Vec<(f32, f32, f32, f32, &UnderKind)> = c
                .under
                .iter()
                .map(|(rx, ry, rw, rh, kind)| (*rx, *ry, *rw, *rh, kind))
                .collect();
            under.sort_by_key(|(_, _, _, _, kind)| kind.rank());
            for (rx, ry, rw, rh, kind) in under {
                crate::paint::paint_under_decor(
                    frame,
                    tx + (rx - 1.0) as i32,
                    cy + (ry - 1.0) as i32,
                    (rw + 2.0) as u32,
                    (rh + 2.0) as u32,
                    kind,
                    &theme,
                    self.input_state.scale,
                );
            }
            frame.blit(&c.pixels, c.pixel_w, c.pixel_h, tx, cy);
            // 后置装饰（abbr 虚线下划线）画在文本位图之上。
            for &(rx, ry, rw, rh, ref kind) in &c.over {
                crate::paint::paint_over_decor(
                    frame,
                    tx + rx as i32,
                    cy + ry as i32,
                    rw as u32,
                    rh,
                    kind,
                    &theme,
                    self.input_state.scale,
                );
            }
            // 单元格内联公式：叠在文本位图之上、网格线之下（与正文 PlacedImg 同层序）。
            for m in &c.maths {
                if let Some(img) = self.cache.inline_math(&m.tex, inline_key) {
                    frame.blit(
                        &img.pixels,
                        m.w,
                        m.h,
                        x0 + (c.x + m.x).round() as i32,
                        top + (c.y + m.y).round() as i32,
                    );
                }
            }
            // 单元格内联图片：按原 alt 占位位置叠在文本位图之上（与公式同层序）。
            for img in &c.images {
                frame.blit_scaled(
                    &img.pixels,
                    img.pixel_w,
                    img.pixel_h,
                    img.w,
                    img.h,
                    x0 + (c.x + img.x).round() as i32,
                    cy + img.y.round() as i32,
                );
            }
        }
    }

    /// 打印续页表头重绘（R2）：在页帧顶部重画该表的表头行（底色 + 单元格 + 边线）。
    /// 调用方先把整页宽铺成纸色，盖住内容下移后露出的上一行残影。
    pub(crate) fn paint_repeated_table_header(&self, frame: &mut Frame, id: BlockId, top: i32) {
        let Some(t) = self.cache.table(id, self.table_key(id)) else {
            return;
        };
        let Some(first) = t.rows.first() else {
            return;
        };
        let x0 = (self.content_x() + t.x) as i32;
        let theme = self.input_state.theme;
        let w = t.width.ceil().max(1.0) as u32;
        let h = first.height.ceil().max(1.0) as u32;
        let clip_x0 = (self.content_x() + t.x).floor() as i32;
        let clip_x1 = (self.content_x() + self.content_width()).ceil() as i32;
        frame.set_clip(clip_x0, top, (clip_x1 - clip_x0).max(0) as u32, h);
        frame.fill_rect(x0, top, w, h, crate::tokens::palette::table_head_bg(&theme));
        let ncols = t.cols.len().min(t.cells.len());
        self.paint_table_cells(frame, x0, top, &t.cells[..ncols]);
        // 上下边线 + 竖线，与完整网格的分隔线同色同粗。
        frame.fill_rect(x0, top, w, 1, theme.grid);
        frame.fill_rect(x0, top + h as i32 - 1, w, 1, theme.grid);
        let mut x: f32 = 0.0;
        for cw in &t.cols {
            frame.fill_rect(x0 + x.round() as i32, top, 1, h, theme.grid);
            x += cw;
        }
        let right = t.width.round().max(1.0) as i32;
        frame.fill_rect(x0 + right - 1, top, 1, h, theme.grid);
        frame.clear_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{BASE_FONT_SIZE, BASE_LINE_HEIGHT};

    /// 每个视觉行的首/尾可见字符（跳过 WJ 与空白）。
    fn line_edges(buffer: &Buffer) -> Vec<(Option<char>, Option<char>)> {
        let visible = |c: char| c != '\u{2060}' && !c.is_whitespace();
        buffer
            .layout_runs()
            .map(|run| {
                let first = run
                    .glyphs
                    .iter()
                    .find_map(|g| run.text.get(g.start..g.end)?.chars().find(|&c| visible(c)));
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

    fn measured_width(engine: &mut LayoutEngine, metrics: Metrics, text: &str) -> f32 {
        measure_clusters(engine.font_system(), metrics, &Attrs::new(), text)
            .iter()
            .map(|c| c.2)
            .sum()
    }

    #[test]
    fn short_ascii_word_and_inline_code_wrap_atomically() {
        let mut engine = LayoutEngine::new();
        let metrics = Metrics::new(BASE_FONT_SIZE, BASE_LINE_HEIGHT);
        let attrs = Attrs::new();
        for token in ["code", "`code`"] {
            let text = format!("a {token}");
            let token_w: f32 = measure_clusters(engine.font_system(), metrics, &attrs, &text)
                .iter()
                .filter(|c| c.0 >= 2)
                .map(|c| c.2)
                .sum();
            let width = token_w + 1.0;
            let w = wrap_cell_text(engine.font_system(), metrics, &attrs, &[], &text, width);
            assert_eq!(w.display, format!("a \n{token}"), "短 token 不得内部断行");

            let alone = wrap_cell_text(engine.font_system(), metrics, &attrs, &[], token, token_w);
            assert_eq!(alone.display, token, "短 token 独占单元格时必须完整");
        }
    }

    #[test]
    fn overwide_ascii_word_keeps_glyph_fallback_without_overflow() {
        let mut engine = LayoutEngine::new();
        let metrics = Metrics::new(BASE_FONT_SIZE, BASE_LINE_HEIGHT);
        let attrs = Attrs::new();
        let text = "supercalifragilistic";
        let width = measured_width(&mut engine, metrics, text) / 3.0;
        let w = wrap_cell_text(engine.font_system(), metrics, &attrs, &[], text, width);
        assert!(w.display.lines().count() > 1, "超宽英文词仍须逐字兜底");
        assert_eq!(w.display.replace('\n', ""), text);
        for line in w.display.lines() {
            assert!(
                measured_width(&mut engine, metrics, line) <= width + 0.01,
                "兜底折行不得横向溢出: {line:?}"
            );
        }
    }

    #[test]
    fn cjk_cell_wraps_without_horizontal_overflow() {
        let mut engine = LayoutEngine::new();
        let metrics = Metrics::new(BASE_FONT_SIZE, BASE_LINE_HEIGHT);
        let attrs = Attrs::new();
        let text = "中文中文中文中文中文";
        let width = measured_width(&mut engine, metrics, "中") * 2.5;
        let w = wrap_cell_text(engine.font_system(), metrics, &attrs, &[], text, width);
        assert!(w.display.lines().count() > 1, "中文仍须正常折行");
        assert_eq!(w.display.replace('\n', ""), text);
        for line in w.display.lines() {
            assert!(
                measured_width(&mut engine, metrics, line) <= width + 0.01,
                "中文折行不得横向溢出: {line:?}"
            );
        }
    }

    fn check_wrap_mapping(text: &str, width: f32) {
        let mut engine = LayoutEngine::new();
        let metrics = Metrics::new(BASE_FONT_SIZE, BASE_LINE_HEIGHT);
        let attrs = Attrs::new();
        let w = wrap_cell_text(engine.font_system(), metrics, &attrs, &[], text, width);
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
}
