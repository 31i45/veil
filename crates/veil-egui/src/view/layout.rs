//! cosmic-text 排版引擎：块级布局、命中检测、光标坐标、光栅化。
//! 布局单位即物理像素——缩放因子由 shell 在 width/metrics 上乘入，本层无 DPI 概念。
//!
//! 消隐机制（统一）：行尾 `\r` 与非激活 inline 节点的标记段都不参与排版（`\r` 交给
//! cosmic 会渲染成缺字形框；标记消隐即 Reveal）。两类区间统一进消隐集合，由
//! `OffsetMap` 负责 doc↔layout 坐标换算——对外偏移一律为文档坐标。
//! 激活规则 v0：`range.start <= caret <= range.end` 闭区间，逐节点独立、嵌套各自生效。

use cosmic_text::{
    Attrs, Buffer, Color, Cursor, Family, FontSystem, Metrics, Shaping, Style as FontStyle,
    SwashCache, Weight, Wrap,
};
use veil_core::{BlockKind, InlineNode, Range};

use super::style::{style_spans, Style};

/// 行内资源键：占位图与位图缓存之间的寻址键（切片 8/13 共用占位机制）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InlineAsset {
    /// 行内公式：键 = TeX 源码（内容寻址缓存）。
    Math(String),
    /// 文档内图片：键 = 按文档目录解析后的绝对路径（自然尺寸缓存）。
    Image(String),
}

/// 行内资源图（输入）：非激活 Math/Image 节点的渲染结果，供占位排版。
pub struct InlineImg {
    /// 节点区间（doc 绝对坐标，含 `$` / `![` 标记）。
    pub range: Range,
    /// 位图缓存键（绘制时据此取图）。
    pub key: InlineAsset,
    /// 占位宽度（= 显示宽度；图片已按版心宽等比缩放时小于自然宽）。
    pub width: u32,
    pub height: u32,
    /// 图顶到基线的像素（与文本行基线对齐）。图片取 height（底边贴基线）。
    pub baseline: u32,
}

/// 行内资源图（输出）：排版后的块内物理像素位置。
/// `x` 左缘、`y` 图顶（= 行基线 - baseline，可为负，frame.blit 裁剪）。
pub struct PlacedImg {
    pub key: InlineAsset,
    pub x: f32,
    pub y: f32,
    pub width: u32,
    pub height: u32,
}

pub const BASE_FONT_SIZE: f32 = 16.0;
pub const BASE_LINE_HEIGHT: f32 = 22.0;

/// 块级字号倍率（标题）。样式语义见 style.rs 头注。
fn block_scale(kind: BlockKind) -> f32 {
    match kind {
        BlockKind::AtxHeading(1) => 1.6,
        BlockKind::AtxHeading(2) => 1.4,
        BlockKind::AtxHeading(3) => 1.2,
        BlockKind::AtxHeading(_) => 1.1,
        _ => 1.0,
    }
}

/// 替换段：块相对 doc 区间 `[doc_start, doc_end)` → `fill` 文本。
/// `fill` 为空即消隐；非空时整段替换（长度可同可异）。fill 不得含换行——
/// 行基仅在保留段计算。行内公式占位（NBSP）与块级替换（列表/引用/分隔线）共用此结构。
#[derive(Clone, Debug)]
struct Sub {
    doc_start: usize,
    doc_end: usize,
    fill: String,
}

impl Sub {
    fn elide(doc_start: usize, doc_end: usize) -> Self {
        Sub {
            doc_start,
            doc_end,
            fill: String::new(),
        }
    }

    fn fill(doc_start: usize, doc_end: usize, text: impl Into<String>) -> Self {
        Sub {
            doc_start,
            doc_end,
            fill: text.into(),
        }
    }
}

/// 坐标映射段。`sub` 为替换段在已接受替换表中的下标；`None` 为保留源文本段。
#[derive(Clone, Copy)]
struct Seg {
    layout_start: usize,
    doc_start: usize,
    layout_len: usize,
    doc_len: usize,
    sub: Option<usize>,
}

/// doc↔layout 坐标换算段表：doc 升序、互不重叠，layout 侧致密覆盖 `[0, layout_len)`。
/// 普通保留段 ll==dl；替换段（行内公式占位、列表符号、引用标记空格）layout 长度 = fill 长度；
/// 消隐段（fill 为空）不发 seg，仅推进 doc 游标。
struct OffsetMap {
    segs: Vec<Seg>,
    layout_len: usize,
    doc_len: usize,
}

impl OffsetMap {
    /// 由替换段构造，返回 (映射, 已接受替换表)。不变量：替换段两两仅允许包含关系
    /// （占位段覆盖其内消隐段，如多行公式内的 \r）。包含于前段的后来者丢弃；
    /// 其余重叠属调用方违约，防御性跳过不崩。
    fn from_spans(doc_len: usize, spans: &[Sub]) -> (Self, Vec<Sub>) {
        let mut sorted: Vec<Sub> = spans.to_vec();
        // 同起点长者在前：占位段先于其内消隐段入表，保证包含判定命中
        sorted.sort_unstable_by(|a, b| {
            a.doc_start
                .cmp(&b.doc_start)
                .then(b.doc_end.cmp(&a.doc_end))
        });
        let mut accepted: Vec<Sub> = Vec::new();
        for sub in sorted {
            let (s, e) = (sub.doc_start.min(doc_len), sub.doc_end.min(doc_len));
            if s >= e {
                continue;
            }
            match accepted.last() {
                Some(last) if s >= last.doc_start && e <= last.doc_end => continue, // 包含于前段：丢弃
                Some(last) if s < last.doc_end => continue, // 违约重叠：防御跳过
                _ => accepted.push(Sub {
                    doc_start: s,
                    doc_end: e,
                    fill: sub.fill,
                }),
            }
        }
        let mut segs = Vec::new();
        let (mut d, mut l) = (0, 0);
        for (i, sub) in accepted.iter().enumerate() {
            let (s, e) = (sub.doc_start, sub.doc_end);
            if d < s {
                segs.push(Seg {
                    layout_start: l,
                    doc_start: d,
                    layout_len: s - d,
                    doc_len: s - d,
                    sub: None,
                });
                l += s - d;
            }
            let ll = sub.fill.len();
            if ll > 0 {
                segs.push(Seg {
                    layout_start: l,
                    doc_start: s,
                    layout_len: ll,
                    doc_len: e - s,
                    sub: Some(i),
                });
                l += ll;
            }
            d = e;
        }
        if d < doc_len {
            segs.push(Seg {
                layout_start: l,
                doc_start: d,
                layout_len: doc_len - d,
                doc_len: doc_len - d,
                sub: None,
            });
            l += doc_len - d;
        }
        (
            OffsetMap {
                segs,
                layout_len: l,
                doc_len,
            },
            accepted,
        )
    }

    /// doc 偏移 → layout 偏移。落在消隐区间或替换段内时钳到其布局起点（= 图左缘/替换文本左缘）。
    fn doc_to_layout(&self, p: usize) -> usize {
        let i = self.segs.partition_point(|s| s.doc_start <= p);
        match i.checked_sub(1).map(|j| self.segs[j]) {
            Some(seg) if p < seg.doc_start + seg.doc_len => {
                if seg.sub.is_some() {
                    seg.layout_start
                } else {
                    seg.layout_start + (p - seg.doc_start)
                }
            }
            _ => self
                .segs
                .get(i)
                .map(|s| s.layout_start)
                .unwrap_or(self.layout_len),
        }
    }

    /// layout 偏移 → doc 偏移（hit 逆映射）。布局坐标致密，必落在某段内。
    /// 替换段内按中点规则落段首/尾（点击公式图或列表符号左右半 → 光标落符号边界）。
    fn layout_to_doc(&self, l: usize) -> usize {
        if l >= self.layout_len {
            return self.doc_len;
        }
        let i = self.segs.partition_point(|s| s.layout_start <= l);
        match i.checked_sub(1).map(|j| self.segs[j]) {
            Some(seg) if l < seg.layout_start + seg.layout_len => {
                if seg.sub.is_some() {
                    if (l - seg.layout_start) * 2 < seg.layout_len {
                        seg.doc_start
                    } else {
                        seg.doc_start + seg.doc_len
                    }
                } else {
                    seg.doc_start + (l - seg.layout_start)
                }
            }
            _ => self.doc_len, // 致密覆盖下不可达，防御
        }
    }
}

/// 单块排版结果。`line_bases[i]` = 第 i 个逻辑行在排版文本（layout 坐标）中的起始字节偏移。
pub struct BlockLayout {
    buffer: Buffer,
    map: OffsetMap,
    line_bases: Vec<usize>,
    /// 排版文本（`\r` 与隐藏标记已剥除）：Reveal 行为的直接观测量。
    layout_text: String,
    pub height: f32,
    /// 行内公式图的块内位置（切片 8）；shell 按 tex 取位图 blit。
    pub imgs: Vec<PlacedImg>,
}

impl BlockLayout {
    pub fn layout_text(&self) -> &str {
        &self.layout_text
    }
}

pub struct LayoutEngine {
    font_system: FontSystem,
    swash: SwashCache,
    /// DPI 缩放：metrics 统一乘此值，布局单位保持物理像素。
    scale: f32,
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutEngine {
    pub fn new() -> Self {
        LayoutEngine {
            font_system: FontSystem::new(),
            swash: SwashCache::new(),
            scale: 1.0,
        }
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
    }

    /// 当前 DPI 缩放（表格排版等复用，保持布局单位 = 物理像素）。
    pub(crate) fn scale_factor(&self) -> f32 {
        self.scale
    }

    /// 表格单元格排版需要直接驱动字体系统（跨模块 pub(crate) 访问口）。
    pub(crate) fn font_system(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    /// IME preedit 小串布局（不换行、默认样式、无消隐）。
    pub fn layout_preedit(&mut self, text: &str) -> BlockLayout {
        let metrics = Metrics::new(BASE_FONT_SIZE * self.scale, BASE_LINE_HEIGHT * self.scale);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
        buffer.set_size(None, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let height = buffer
            .layout_runs()
            .map(|r| r.line_top + r.line_height)
            .fold(metrics.line_height, f32::max);
        BlockLayout {
            buffer,
            map: OffsetMap::from_spans(text.len(), &[]).0,
            line_bases: vec![0],
            layout_text: text.to_string(),
            height,
            imgs: Vec::new(),
        }
    }

    /// 排版单块。`text` 为块文本（含内部换行；尾换行可有可无）；`inlines`/`block` 为文档区间；
    /// `caret` 为文档绝对偏移，驱动 Reveal 激活判定（无光光标语义传 `usize::MAX` 全消隐）。
    /// `imgs`：已渲染的非激活行内公式/图片（切片 8/13）——节点整段替换为 NBSP 占位
    /// （宽≈位图宽），排版后定位占位首字形得 PlacedImg（基线对齐）。激活节点（caret
    /// 闭区间内）的 img 忽略。
    /// 已知 v0 简化：高超一行的行内公式/图片可能与同块下一视觉行重叠（块高已扩展
    /// 不溢出版心）；图片顶部钳在行顶（不上侵前一块），公式保留原基线；
    /// 标题内公式/图片不随标题字号放大（尺寸由渲染侧/图片自然尺寸决定）。
    pub fn layout_block(
        &mut self,
        kind: BlockKind,
        text: &str,
        block: Range,
        inlines: &[InlineNode],
        width: f32,
        caret: usize,
        imgs: &[InlineImg],
    ) -> BlockLayout {
        let body = text.strip_suffix('\n').unwrap_or(text);
        let scale = block_scale(kind) * self.scale;
        let metrics = Metrics::new(BASE_FONT_SIZE * scale, BASE_LINE_HEIGHT * scale);
        // NBSP 占位段：NBSP 为 2 字节，个数 = 图宽 / NBSP 宽四舍五入（误差 ≤ 半个 NBSP）。
        let nbsp_w = if imgs.is_empty() {
            1.0
        } else {
            self.measure_nbsp(metrics)
        };
        let mut kept: Vec<&InlineImg> = Vec::new();
        // 块级替换（切片 12）先入表；行内公式占位段随后按起点排序归并
        let mut subs: Vec<Sub> = block_subs(kind, body, block, caret);
        for im in imgs {
            if node_active(im.range, caret) {
                continue; // 激活态显源码，不占位
            }
            let rs = im.range.start.saturating_sub(block.start);
            let re = im.range.end.saturating_sub(block.start).min(body.len());
            if rs >= re {
                continue; // 防御：img 区间须落在本块内（shell 按块收集，不会越界）
            }
            let nbsp = ((im.width as f32 / nbsp_w).round() as usize).max(1);
            subs.push(Sub::fill(rs, re, "\u{A0}".repeat(nbsp)));
            kept.push(im);
        }
        let (layout_text, map, line_bases) = build_layout_text(body, block, inlines, caret, &subs);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let default_attrs = attrs_of(kind, Style::default());
        // style_spans 产出文档坐标，经映射转 layout 坐标；整体落入消隐/占位区的 span 零宽丢弃。
        // 占位 NBSP 区无样式来源，显式补默认样式 span（cosmic 排版文本 = 各 span 顺次拼接，
        // 覆盖必须完备且有序）；指针序即 layout 偏移序。
        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        for (s, e, st) in style_spans(body.len(), block, inlines) {
            let (ls, le) = (map.doc_to_layout(s), map.doc_to_layout(e));
            if ls < le {
                spans.push((&layout_text[ls..le], attrs_of(kind, st)));
            }
        }
        for im in &kept {
            let ls = map.doc_to_layout(im.range.start.saturating_sub(block.start));
            let ll = map.doc_to_layout(im.range.end.saturating_sub(block.start)) - ls;
            spans.push((&layout_text[ls..ls + ll], default_attrs.clone()));
        }
        if spans.is_empty() {
            spans.push(("", default_attrs.clone())); // 空行也需默认样式
        }
        spans.sort_by_key(|(s, _)| s.as_ptr() as usize);
        buffer.set_rich_text(spans, &default_attrs, Shaping::Advanced, None);
        buffer.set_wrap(Wrap::Glyph);
        buffer.set_size(Some(width), None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let mut height = buffer
            .layout_runs()
            .map(|r| r.line_top + r.line_height)
            .fold(metrics.line_height, f32::max);
        // 占位段首字形定位：x = 字形左缘，图顶 = 行基线(line_y) - baseline；块高扩展到图底。
        // 图片底边贴基线，高位图会把图顶推到行顶之上：钳到行顶，避免上侵前一块
        // （同块后续视觉行的重叠属 v0 已知简化，块高已扩展故不影响后续块）。
        let mut placed: Vec<PlacedImg> = Vec::new();
        for im in &kept {
            let ls = map.doc_to_layout(im.range.start.saturating_sub(block.start));
            let line_i = line_bases.partition_point(|&b| b <= ls).saturating_sub(1);
            let index = ls - line_bases.get(line_i).copied().unwrap_or(0);
            let mut at = None;
            for run in buffer.layout_runs() {
                if run.line_i != line_i {
                    continue;
                }
                let run_end = run.glyphs.last().map(|g| g.end).unwrap_or(0);
                if index > run_end && !run.glyphs.is_empty() {
                    continue; // 折行：占位在后续视觉行（同 caret_xy 选 run 规则）
                }
                for g in run.glyphs {
                    if index <= g.start {
                        at = Some((g.x, run.line_y, run.line_top));
                        break;
                    }
                }
                if at.is_some() {
                    break;
                }
            }
            let Some((x, line_y, line_top)) = at else {
                continue;
            }; // 防御：占位段必有 NBSP 字形
            let mut y = line_y - im.baseline as f32; // 可为负（图高出行顶），frame.blit 裁剪
            if matches!(im.key, InlineAsset::Image(_)) {
                y = y.max(line_top);
            }
            height = height.max(y + im.height as f32);
            placed.push(PlacedImg {
                key: im.key.clone(),
                x,
                y,
                width: im.width,
                height: im.height,
            });
        }
        BlockLayout {
            buffer,
            map,
            line_bases,
            layout_text,
            height,
            imgs: placed,
        }
    }

    /// NBSP 占位宽度：同 metrics 下排 8 个 NBSP 求均值。字体缺字形时退化 1px 防除零。
    fn measure_nbsp(&mut self, metrics: Metrics) -> f32 {
        let probe = "\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}";
        let mut b = Buffer::new(&mut self.font_system, metrics);
        b.set_text(probe, &Attrs::new(), Shaping::Advanced, None);
        b.set_size(None, None);
        b.shape_until_scroll(&mut self.font_system, false);
        let w: f32 = b
            .layout_runs()
            .flat_map(|r| r.glyphs.iter())
            .map(|g| g.w)
            .sum();
        (w / 8.0).max(1.0)
    }

    /// 像素点 → 块内字节偏移（文档坐标）。y 越界钳到首/末行（hit 内建行为）。
    pub fn hit(&self, layout: &BlockLayout, x: f32, y: f32) -> usize {
        match layout.buffer.hit(x, y) {
            Some(c) => layout
                .map
                .layout_to_doc(layout.line_bases.get(c.line).copied().unwrap_or(0) + c.index),
            None => 0,
        }
    }

    /// 块内字节偏移（文档坐标）→ 光标矩形 (x, y_top, height)。
    /// 簇内中点按线性比例近似（hit/键盘导航产出恒在 grapheme 边界，该路径仅为防御）。
    /// 折行：同一逻辑行产出多个 run，须跳过字形区间不覆盖 index 的前置视觉行。
    pub fn caret_xy(&self, layout: &BlockLayout, offset: usize) -> (f32, f32, f32) {
        let lo = layout.map.doc_to_layout(offset);
        let line_i = layout
            .line_bases
            .partition_point(|&b| b <= lo)
            .saturating_sub(1);
        let index = lo - layout.line_bases.get(line_i).copied().unwrap_or(0);
        let mut tail = (0.0, 0.0, layout.height); // 同 line_i 最后一个 run 的行尾（兜底）
        for run in layout.buffer.layout_runs() {
            if run.line_i != line_i {
                continue;
            }
            let run_end = run.glyphs.last().map(|g| g.end).unwrap_or(0);
            tail = (
                run.glyphs.last().map(|g| g.x + g.w).unwrap_or(0.0),
                run.line_top,
                run.line_height,
            );
            if index > run_end && !run.glyphs.is_empty() {
                continue; // 折行的后续视觉行才覆盖该 index
            }
            for g in run.glyphs {
                if index <= g.start {
                    return (g.x, run.line_top, run.line_height);
                }
                if index <= g.end {
                    let ratio = (index - g.start) as f32 / (g.end - g.start).max(1) as f32;
                    return (g.x + g.w * ratio, run.line_top, run.line_height);
                }
            }
            // 行尾/空行
            return tail;
        }
        tail
    }

    /// 当前偏移所在视觉行的首尾偏移（块内文档坐标，(start, end)，start <= end）。
    /// Home/End 语义：经 hit 换算，折行按视觉行切分，消隐区间不会成为端点。
    pub fn visual_line_edges(&self, layout: &BlockLayout, offset: usize) -> (usize, usize) {
        let (_, y, _) = self.caret_xy(layout, offset);
        let s = self.hit(layout, 0.0, y + 1.0);
        let e = self.hit(layout, f32::MAX, y + 1.0);
        (s.min(e), s.max(e))
    }

    /// 选区高亮矩形集（块内物理像素，(x, y_top, w, h)；0.19 起同一视觉行在
    /// BiDi/样式切换处可产多条，逐段收齐）。
    /// `start`/`end` 为块相对文档偏移，经 OffsetMap 映射后逐 run 求交；
    /// 换行符本身不产生矩形，空行/纯消隐段无字形故无矩形（v0 已知简化）。
    pub fn selection_rects(
        &self,
        layout: &BlockLayout,
        start: usize,
        end: usize,
    ) -> Vec<(f32, f32, f32, f32)> {
        let (lo0, lo1) = (
            layout.map.doc_to_layout(start),
            layout.map.doc_to_layout(end),
        );
        if lo0 >= lo1 {
            return Vec::new();
        }
        let mut rects = Vec::new();
        for run in layout.buffer.layout_runs() {
            let base = layout.line_bases.get(run.line_i).copied().unwrap_or(0);
            // 行内字节区间（半开）：Cursor 默认 Affinity::Before，比较天然排除右端点起始字形
            let (s, e) = (lo0.saturating_sub(base), lo1.saturating_sub(base));
            if s >= e {
                continue;
            }
            // cosmic-text 0.19：highlight 返回迭代器，BiDi/样式切换处单 run 可产多段，须收齐
            for (x, w) in run.highlight(Cursor::new(run.line_i, s), Cursor::new(run.line_i, e)) {
                if w > 0.0 {
                    rects.push((x, run.line_top, w, run.line_height));
                }
            }
        }
        rects
    }

    /// 光栅化为 RGBA8（straight alpha，src-over 混合到透明底）。`color` 为默认文字色。
    /// 已知简化：彩色 emoji（swash 预乘输出）按 straight 处理，边缘或有轻微杂边。
    pub fn rasterize(&mut self, layout: &BlockLayout, color: [u8; 3]) -> (Vec<u8>, u32, u32) {
        self.rasterize_buffer(&layout.buffer, layout.height, color)
    }

    /// 光栅化任意已排版 buffer（表格单元格复用同一 path）；`height` 为排版高（物理像素）。
    pub(crate) fn rasterize_buffer(
        &mut self,
        buffer: &Buffer,
        height: f32,
        color: [u8; 3],
    ) -> (Vec<u8>, u32, u32) {
        let mut max_w: f32 = 0.0;
        for run in buffer.layout_runs() {
            for g in run.glyphs {
                max_w = max_w.max(g.x + g.w);
            }
        }
        let (w, h) = (
            max_w.ceil().max(1.0) as usize,
            height.ceil().max(1.0) as usize,
        );
        let mut px = vec![0u8; w * h * 4];
        for run in buffer.layout_runs() {
            for g in run.glyphs {
                let base = g.color_opt.map(|c| [c.r(), c.g(), c.b()]).unwrap_or(color);
                let physical = g.physical((0.0, run.line_y), 1.0);
                let fs = &mut self.font_system;
                let key_color = Color::rgb(base[0], base[1], base[2]);
                self.swash
                    .with_pixels(fs, physical.cache_key, key_color, |gx, gy, c| {
                        let (x, y) = (physical.x + gx, physical.y + gy);
                        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                            return;
                        }
                        let i = (y as usize * w + x as usize) * 4;
                        let a = c.a() as u32;
                        let ia = 255 - a;
                        px[i] = ((c.r() as u32 * a + px[i] as u32 * ia) / 255) as u8;
                        px[i + 1] = ((c.g() as u32 * a + px[i + 1] as u32 * ia) / 255) as u8;
                        px[i + 2] = ((c.b() as u32 * a + px[i + 2] as u32 * ia) / 255) as u8;
                        px[i + 3] = (a + px[i + 3] as u32 * ia / 255) as u8;
                    });
            }
        }
        (px, w as u32, h as u32)
    }
}

/// 激活规则 v0（Reveal）：caret 落在节点闭区间内即激活（显示标记）。
/// 闭区间让光标贴标记边缘时即展开，与 Typora 行为对齐。逐节点独立判定，嵌套各自生效。
pub(crate) fn node_active(range: Range, caret: usize) -> bool {
    range.start <= caret && caret <= range.end
}

/// 非激活节点的标记段加入消隐集合（块相对 doc 坐标）：
/// `[range.start, content.start)` + `[content.end, range.end)`。
/// Link 的 content 为可见文本，故 `](url)` 整体消隐；Code/Math 的 children 恒空。
fn collect_elide(nodes: &[InlineNode], block: Range, caret: usize, out: &mut Vec<(usize, usize)>) {
    for n in nodes {
        if !node_active(n.range, caret) {
            let rs = n.range.start.saturating_sub(block.start);
            let cs = n.content.start.saturating_sub(block.start);
            let ce = n.content.end.saturating_sub(block.start);
            let re = n.range.end.saturating_sub(block.start);
            if rs < cs {
                out.push((rs, cs));
            }
            if ce < re {
                out.push((ce, re));
            }
        }
        collect_elide(&n.children, block, caret, out);
    }
}

/// 构造排版文本与坐标映射：行尾 `\r`、非激活节点标记、块级替换统一进替换表。
/// 返回 (排版文本, OffsetMap, 每逻辑行布局起始偏移)。调用方已剥唯一尾 `\n`。
fn build_layout_text(
    body: &str,
    block: Range,
    inlines: &[InlineNode],
    caret: usize,
    subs: &[Sub],
) -> (String, OffsetMap, Vec<usize>) {
    let mut all: Vec<Sub> = Vec::new();
    // 行尾 \r：仅剥「后继为 \n 或文末」的 \r（行内孤 \r 保持原样，与切片 1 行为一致）
    let bytes = body.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\r' && (i + 1 == bytes.len() || bytes[i + 1] == b'\n') {
            all.push(Sub::elide(i, i + 1));
        }
    }
    let mut elide: Vec<(usize, usize)> = Vec::new();
    collect_elide(inlines, block, caret, &mut elide);
    all.extend(elide.into_iter().map(|(s, e)| Sub::elide(s, e)));
    all.extend_from_slice(subs);
    let (map, accepted) = OffsetMap::from_spans(body.len(), &all);
    // 保留段取源文本并记行基；替换段写 fill（调用方保证不含换行）
    let mut layout_text = String::with_capacity(map.layout_len);
    let mut line_bases = vec![0];
    for seg in &map.segs {
        match seg.sub {
            Some(i) => layout_text.push_str(&accepted[i].fill),
            None => {
                let seg_text = &body[seg.doc_start..seg.doc_start + seg.doc_len];
                for (j, b) in seg_text.bytes().enumerate() {
                    if b == b'\n' {
                        line_bases.push(seg.layout_start + j + 1);
                    }
                }
                layout_text.push_str(seg_text);
            }
        }
    }
    (layout_text, map, line_bases)
}

/// 块级替换（切片 12）：无序列表符号 `•`、引用行首 `>` → 空格、分隔线整行消隐。
/// 激活判据沿用块区间闭区间；引用块逐行判定（光标所在行显源码，其余行隐标记）。
fn block_subs(kind: BlockKind, body: &str, block: Range, caret: usize) -> Vec<Sub> {
    match kind {
        BlockKind::ListItem { ordered: false, .. } => {
            if node_active(block, caret) {
                return Vec::new();
            }
            match bullet_pos(body) {
                Some(p) => vec![Sub::fill(p, p + 1, "•")],
                None => Vec::new(),
            }
        }
        BlockKind::BlockQuote => quote_line_subs(body, block, caret),
        BlockKind::ThematicBreak => {
            if node_active(block, caret) {
                Vec::new()
            } else {
                vec![Sub::elide(0, body.len())]
            }
        }
        _ => Vec::new(),
    }
}

/// 无序列表符号位置（块相对字节）：缩进后首个 `-` / `*` / `+`。
fn bullet_pos(body: &str) -> Option<usize> {
    let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
    match body.as_bytes().get(indent) {
        Some(b'-') | Some(b'*') | Some(b'+') => Some(indent),
        _ => None,
    }
}

/// 引用块逐行替换：非激活行的 `>` 换成空格（保留缩进与行距），激活行显源码。
fn quote_line_subs(body: &str, block: Range, caret: usize) -> Vec<Sub> {
    // 块外（含 usize::MAX 无光标）→ 全部行隐标记
    let c = if block.start <= caret && caret <= block.end {
        caret - block.start
    } else {
        usize::MAX
    };
    let mut out = Vec::new();
    let mut off = 0;
    for line in body.split_inclusive('\n') {
        let visible_end = off + line.trim_end_matches('\n').len();
        if !(off <= c && c <= visible_end) {
            if let Some(p) = quote_marker_pos(line) {
                out.push(Sub::fill(off + p, off + p + 1, " "));
            }
        }
        off += line.len();
    }
    out
}

/// 引用行首 `>` 位置（缩进之后）；与内核识别规则一致。
fn quote_marker_pos(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    (line.as_bytes().get(indent) == Some(&b'>')).then_some(indent)
}

fn attrs_of(kind: BlockKind, st: Style) -> Attrs<'static> {
    let a = Attrs::new();
    let a = if st.bold || matches!(kind, BlockKind::AtxHeading(_)) {
        a.weight(Weight::BOLD)
    } else {
        a
    };
    let a = if st.italic {
        a.style(FontStyle::Italic)
    } else {
        a
    };
    let a = if st.mono || matches!(kind, BlockKind::FencedCode) {
        a.family(Family::Monospace)
    } else {
        a
    };
    // st.underline：0.19 Attrs 虽有 text_decoration，下划线渲染随 Reveal 切片另行落地
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use veil_core::Session;

    fn layout_of(text: &str, width: f32) -> (LayoutEngine, BlockLayout) {
        // usize::MAX 表示「无光标」：一切可消隐节点均处于非激活态
        layout_with_caret(text, usize::MAX, width)
    }

    /// caret 为文档绝对偏移（激活判定用）。
    fn layout_with_caret(text: &str, caret: usize, width: f32) -> (LayoutEngine, BlockLayout) {
        layout_with_imgs(text, caret, width, &[])
    }

    /// imgs 为已渲染的非激活行内公式（切片 8 占位流程的输入侧）。
    fn layout_with_imgs(
        text: &str,
        caret: usize,
        width: f32,
        imgs: &[InlineImg],
    ) -> (LayoutEngine, BlockLayout) {
        let s = Session::open(text);
        let (id, kind, range) = s.blocks()[0];
        let inlines = s.block_inlines(id).unwrap();
        let mut eng = LayoutEngine::new();
        let block_text = &text[range.start..range.end];
        let l = eng.layout_block(kind, block_text, range, &inlines, width, caret, imgs);
        (eng, l)
    }

    #[test]
    fn hit_then_caret_roundtrips_ascii() {
        let (eng, l, ..) = layout_of("hello world", 200.0);
        let (x, y, _) = eng.caret_xy(&l, 5);
        assert_eq!(eng.hit(&l, x + 0.1, y + 1.0), 5, "光标 x 命中应回原偏移");
    }

    #[test]
    fn caret_advances_monotonically() {
        let (eng, l, ..) = layout_of("abcdef", 400.0);
        let mut prev = -1.0;
        for i in 0..=6 {
            let (x, _, _) = eng.caret_xy(&l, i);
            assert!(x >= prev, "光标 x 随偏移单调不减");
            prev = x;
        }
    }

    #[test]
    fn multibyte_hit_stays_on_grapheme_boundary() {
        // 「中a文」：点击「中」右半应落 3（hit 内建 grapheme 切分），不落 1/2
        let (eng, l, ..) = layout_of("中a文", 200.0);
        let (x1, y, _) = eng.caret_xy(&l, 3);
        let (x0, _, _) = eng.caret_xy(&l, 0);
        let mid = (x0 + x1) / 2.0 + 0.1;
        let off = eng.hit(&l, mid, y + 1.0);
        assert!(off == 0 || off == 3, "多字节字符内命中不得切碎: {off}");
    }

    #[test]
    fn hit_second_line_maps_with_base() {
        let (eng, l, ..) = layout_of("ab\ncd", 200.0);
        // 第二行行首命中 → 文档偏移 3（a b \n 之后）
        let (_, y, h) = eng.caret_xy(&l, 3);
        assert!(y > 0.0, "第二行 y 应大于 0");
        assert!(h > 0.0);
        let off = eng.hit(&l, 0.0, y + 1.0);
        assert_eq!(off, 3);
    }

    #[test]
    fn crlf_offsets_stay_doc_coordinates() {
        // 「ab\r\ncd」：第二行「c」文档偏移 4（a b \r \n）
        let (eng, l, ..) = layout_of("ab\r\ncd", 200.0);
        let (_, y, _) = eng.caret_xy(&l, 4);
        assert_eq!(eng.hit(&l, 0.0, y + 1.0), 4);
        // 行尾（\r 处，偏移 2）与第一行末字形同 x
        let (x_cr, _, _) = eng.caret_xy(&l, 2);
        let (x_b, _, _) = eng.caret_xy(&l, 1);
        assert!(x_cr > x_b);
    }

    #[test]
    fn empty_text_still_has_line_height() {
        // 空串（preedit 清空/空行排版路径）也须给出正高度，否则光标矩形高度为 0
        let mut eng = LayoutEngine::new();
        let l = eng.layout_preedit("");
        assert!(l.height >= BASE_LINE_HEIGHT);
    }

    #[test]
    fn rasterize_nonempty_pixels_for_text() {
        let (mut eng, l) = layout_of("hello", 200.0);
        let (px, w, h) = eng.rasterize(&l, [0, 0, 0]);
        assert!(w > 0 && h > 0);
        assert!(px.chunks_exact(4).any(|p| p[3] > 0), "文本应产生非透明像素");
    }

    #[test]
    fn heading_layout_taller_than_paragraph() {
        let (_, h1) = layout_of("# t", 400.0);
        let (_, p) = layout_of("t", 400.0);
        assert!(h1.height > p.height);
    }

    // ---- Reveal（切片 2）：光标不在节点内时消隐标记 ----
    // 偏移均逐字节手工核算：「a **b** c」a=0 ' '=1 *=2 *=3 b=4 *=5 *=6 ' '=7 c=8

    #[test]
    fn reveal_hides_strong_markers_when_caret_outside() {
        let (_, l) = layout_with_caret("a **b** c", 0, 200.0);
        assert_eq!(l.layout_text(), "a b c");
    }

    #[test]
    fn reveal_shows_full_node_when_caret_inside() {
        let (_, l) = layout_with_caret("a **b** c", 4, 200.0);
        assert_eq!(l.layout_text(), "a **b** c");
    }

    #[test]
    fn reveal_boundary_offsets_count_as_inside() {
        // 激活规则为闭区间：caret 贴 range.start(2)/range.end(7) 均激活
        for caret in [2, 7] {
            let (_, l) = layout_with_caret("a **b** c", caret, 200.0);
            assert_eq!(l.layout_text(), "a **b** c", "caret={caret} 应激活");
        }
        let (_, l) = layout_with_caret("a **b** c", 8, 200.0);
        assert_eq!(l.layout_text(), "a b c");
    }

    #[test]
    fn reveal_link_hides_url_and_brackets() {
        // 「x [t](http://a) y」：Link range=[2,14) content=[3,4) → 消隐 [ 与 ](http://a)
        let (_, l) = layout_with_caret("x [t](http://a) y", 0, 300.0);
        assert_eq!(l.layout_text(), "x t y");
        let (_, l) = layout_with_caret("x [t](http://a) y", 3, 300.0);
        assert_eq!(l.layout_text(), "x [t](http://a) y");
    }

    #[test]
    fn reveal_nested_nodes_reveal_independently() {
        // 「**a *b* c**」：caret=3 在外层内容、内层（[4,7)）外 → 仅内层标记消隐
        let (_, l) = layout_with_caret("**a *b* c**", 3, 300.0);
        assert_eq!(l.layout_text(), "**a b c**");
        // caret=5 进内层：两层均激活，全显
        let (_, l) = layout_with_caret("**a *b* c**", 5, 300.0);
        assert_eq!(l.layout_text(), "**a *b* c**");
    }

    #[test]
    fn reveal_composes_with_crlf() {
        // 「ab\r\n**c**」：\r=2 消隐；Strong range=[4,9) content=[6,7) → 标记消隐
        let (_, l) = layout_with_caret("ab\r\n**c**", 0, 200.0);
        assert_eq!(l.layout_text(), "ab\nc");
    }

    #[test]
    fn reveal_hit_roundtrips_visible_offsets() {
        // 消隐态下可见文档偏移的 caret_xy → hit 回环必须保持文档坐标
        let (eng, l) = layout_with_caret("a **b** c", 0, 200.0);
        for doc in [0, 1, 4, 7, 8, 9] {
            let (x, y, _) = eng.caret_xy(&l, doc);
            assert_eq!(
                eng.hit(&l, x + 0.1, y + 1.0),
                doc,
                "可见偏移回环: doc={doc}"
            );
        }
    }

    // ---- 块级替换（切片 12）：列表符号 / 引用标记 / 分隔线 ----

    #[test]
    fn list_bullet_hidden_outside_caret_keeps_ordered_source() {
        // 无序标记 [0,1) 非激活 → `•`（与 strong 消隐叠加：「- **a**」→「• a」）
        let (_, l) = layout_with_caret("- **a**", usize::MAX, 200.0);
        assert_eq!(l.layout_text(), "• a");
        // 激活（闭区间含块首）→ 列表标记显源码；strong 未激活仍消隐
        let (_, l) = layout_with_caret("- **a**", 0, 200.0);
        assert_eq!(l.layout_text(), "- a");
        // 光标进 strong 内容 → 列表与 inline 标记均显源码
        let (_, l) = layout_with_caret("- **a**", 4, 200.0);
        assert_eq!(l.layout_text(), "- **a**");
        // 有序列表保留源码编号（v0 不重排编号）
        let (_, l) = layout_with_caret("1. a", usize::MAX, 200.0);
        assert_eq!(l.layout_text(), "1. a");
    }

    #[test]
    fn blockquote_marker_hidden_per_line() {
        // 「> a\n> b」：行 1 [0,4)、行 2 [4,7)
        let (_, l) = layout_with_caret("> a\n> b", usize::MAX, 200.0);
        assert_eq!(l.layout_text(), "  a\n  b", "光标在外：两行均隐 `>`");
        let (_, l) = layout_with_caret("> a\n> b", 6, 200.0);
        assert_eq!(l.layout_text(), "  a\n> b", "光标在行 2：仅该行显源码");
        let (_, l) = layout_with_caret("> a\n> b", 2, 200.0);
        assert_eq!(l.layout_text(), "> a\n  b", "光标在行 1：仅该行显源码");
        assert!(l.height >= 2.0 * BASE_LINE_HEIGHT, "两行引用块高不塌缩");
    }

    #[test]
    fn blockquote_hidden_marker_hit_roundtrips() {
        // 隐标记为等长替换（`>` → 空格）：可见偏移回环仍为文档坐标
        let (eng, l) = layout_with_caret("> a", usize::MAX, 200.0);
        for doc in [0, 1, 2] {
            let (x, y, _) = eng.caret_xy(&l, doc);
            assert_eq!(eng.hit(&l, x + 0.1, y + 1.0), doc, "doc={doc}");
        }
    }

    #[test]
    fn thematic_break_elides_whole_line_inactive() {
        let (_, l) = layout_with_caret("---", usize::MAX, 200.0);
        assert_eq!(l.layout_text(), "");
        assert_eq!(l.height, BASE_LINE_HEIGHT, "消隐后仍占一行高");
        let (_, l) = layout_with_caret("---", 1, 200.0);
        assert_eq!(l.layout_text(), "---");
    }

    // ---- OffsetMap：doc↔layout 坐标换算 ----

    #[test]
    fn offset_map_identity_roundtrip() {
        let (m, _) = OffsetMap::from_spans(5, &[]);
        for p in 0..=5 {
            assert_eq!(m.doc_to_layout(p), p);
            assert_eq!(m.layout_to_doc(p), p);
        }
    }

    #[test]
    fn offset_map_gap_clamps_and_inverts() {
        // "abcdef" 消隐 [2,4)="cd" → 布局 "abef"；消隐区内 doc 钳到其布局起点
        let (m, _) = OffsetMap::from_spans(6, &[Sub::elide(2, 4)]);
        for (doc, want) in [(0, 0), (1, 1), (2, 2), (3, 2), (4, 2), (5, 3), (6, 4)] {
            assert_eq!(m.doc_to_layout(doc), want, "doc={doc}");
        }
        for (l, want) in [(0, 0), (1, 1), (2, 4), (3, 5), (4, 6)] {
            assert_eq!(m.layout_to_doc(l), want, "layout={l}");
        }
    }

    #[test]
    fn offset_map_merges_adjacent_and_tail() {
        let (m, _) = OffsetMap::from_spans(6, &[Sub::elide(2, 3), Sub::elide(3, 5)]);
        assert_eq!(m.doc_to_layout(4), 2);
        assert_eq!(m.doc_to_layout(5), 2);
        assert_eq!(m.layout_to_doc(2), 5);
        // 尾部消隐：布局末尾反映射回文档末尾
        let (tail, _) = OffsetMap::from_spans(4, &[Sub::elide(2, 4)]);
        assert_eq!(tail.doc_to_layout(4), 2);
        assert_eq!(tail.layout_to_doc(2), 4);
    }

    // ---- OffsetMap 替换段（切片 8 行内公式占位 / 切片 12 块级替换） ----
    // 期望均逐字节手工核算（注释内给出 seg 表推导）。

    #[test]
    fn offset_map_substitute_placeholder() {
        // 「aXb」替 [1,2) 为 3 字节：segs=(0,0,1,1) (1,1,3,1) (4,2,1,1)，layout_len=5
        let (m, _) = OffsetMap::from_spans(3, &[Sub::fill(1, 2, "xyz")]);
        assert_eq!(m.layout_len, 5);
        // 占位段内 doc 一律钳到左缘；节点尾（doc=2）落右缘（下一段起点）
        for (doc, want) in [(0, 0), (1, 1), (2, 4), (3, 5)] {
            assert_eq!(m.doc_to_layout(doc), want, "doc={doc}");
        }
        // 占位段内 layout 按中点规则落首/尾（点击图左/右半 → 激活显源码）
        for (l, want) in [(0, 0), (1, 1), (2, 1), (3, 2), (4, 2)] {
            assert_eq!(m.layout_to_doc(l), want, "layout={l}");
        }
    }

    #[test]
    fn offset_map_elide_contained_in_placeholder_is_dropped() {
        // 多行行内公式：节点 [2,8) 占位 3 字节，其内 \r（[4,5)）消隐被占位段覆盖丢弃。
        // segs=(0,0,2,2) (2,2,3,6) (5,8,2,2)，layout_len=7
        let (m, _) = OffsetMap::from_spans(10, &[Sub::fill(2, 8, "xyz"), Sub::elide(4, 5)]);
        assert_eq!(m.layout_len, 7);
        assert_eq!(
            m.doc_to_layout(4),
            2,
            "占位段内钳到左缘（\\r 消隐不得切进占位段）"
        );
        assert_eq!(m.doc_to_layout(8), 5);
        assert_eq!(m.layout_to_doc(3), 2);
        assert_eq!(m.layout_to_doc(4), 8);
        assert_eq!(m.layout_to_doc(5), 8);
        assert_eq!(m.layout_to_doc(6), 9);
    }

    #[test]
    fn offset_map_same_start_placeholder_wins_over_elide() {
        // 同起点时占位段（长）先于消隐段（短）：节点 [0,3) 占位 2 字节，[0,1) 消隐被丢弃。
        // segs=(0,0,2,3) (2,3,2,2)
        let (m, _) = OffsetMap::from_spans(5, &[Sub::elide(0, 1), Sub::fill(0, 3, "xy")]); // 乱序输入亦正确
        assert_eq!(m.layout_len, 4);
        assert_eq!(m.doc_to_layout(1), 0);
        assert_eq!(m.layout_to_doc(1), 3, "(1-0)*2=2 不 <2 → 落节点尾");
        assert_eq!(m.layout_to_doc(2), 3);
    }

    #[test]
    fn offset_map_equal_length_substitute_still_replaces() {
        // 切片 12 引用标记：1 字节 `>` → 1 字节空格（ll==dl 但仍是替换段）。
        // ">a"：segs=(0,0,1,1) (1,1,1,1)，layout_len=2；doc 0 钳左缘、doc 1 起为保留段
        let (m, acc) = OffsetMap::from_spans(2, &[Sub::fill(0, 1, " ")]);
        assert_eq!(m.layout_len, 2);
        assert_eq!(acc[0].fill, " ");
        assert_eq!(m.doc_to_layout(0), 0);
        assert_eq!(m.doc_to_layout(1), 1);
        assert_eq!(m.layout_to_doc(0), 0, "点击替换段左半 → 段首");
        assert_eq!(m.layout_to_doc(1), 1);
    }

    // ---- 行内公式占位（切片 8）：非激活 Math 节点 → n 个 NBSP 占位 + PlacedImg ----
    // 「a $x$ b」逐字节：a=0 ' '=1 $=2 x=3 $=4 ' '=5 b=6，Math range=[2,5) content=[3,4)

    fn img(range: Range, width: u32, height: u32, baseline: u32) -> InlineImg {
        InlineImg {
            range,
            key: InlineAsset::Math("x".into()),
            width,
            height,
            baseline,
        }
    }

    /// 图片占位（切片 13）：键为解析后的路径，底边贴基线（baseline = height）。
    fn image(range: Range, width: u32, height: u32) -> InlineImg {
        InlineImg {
            range,
            key: InlineAsset::Image("p.png".into()),
            width,
            height,
            baseline: height,
        }
    }

    #[test]
    fn inline_math_placeholder_width_matches_image() {
        let imgs = [img(Range::new(2, 5), 40, 10, 8)];
        let (eng, l) = layout_with_imgs("a $x$ b", usize::MAX, 400.0, &imgs);
        assert!(!l.layout_text().contains("$x$"), "非激活节点源码不入排版");
        // 图宽 40px：节点尾光标 x - 节点首光标 x ≈ 40（误差 ≤ 2 个 NBSP 宽，字体度量从宽）
        let (x0, _, _) = eng.caret_xy(&l, 2);
        let (x1, _, _) = eng.caret_xy(&l, 5);
        let w = x1 - x0;
        assert!((w - 40.0).abs() <= 9.0, "占位宽应≈图宽: w={w}");
    }

    #[test]
    fn inline_math_placed_on_text_baseline() {
        let imgs = [img(Range::new(2, 5), 16, 8, 6)];
        let (eng, l) = layout_with_imgs("a $x$ b", usize::MAX, 400.0, &imgs);
        assert_eq!(l.imgs.len(), 1);
        let p = &l.imgs[0];
        assert_eq!(p.width, 16);
        let (x, top, h) = eng.caret_xy(&l, 2);
        assert_eq!(p.x, x, "图左缘 = 节点首光标 x");
        let base = p.y + 6.0; // 图顶 + baseline = 行基线
        assert!(
            base > top && base <= top + h,
            "基线落在本行内: base={base} top={top} h={h}"
        );
    }

    #[test]
    fn tall_inline_image_extends_block_height() {
        let imgs = [img(Range::new(2, 5), 16, 100, 10)];
        let (_, l) = layout_with_imgs("a $x$ b", usize::MAX, 400.0, &imgs);
        let p = &l.imgs[0];
        assert!(
            l.height >= p.y + 100.0,
            "块高须覆盖图底: h={} y={}",
            l.height,
            p.y
        );
        assert!(l.height > BASE_LINE_HEIGHT, "图高超行高，块高必须扩展");
    }

    #[test]
    fn hit_on_image_maps_to_node_edges() {
        let imgs = [img(Range::new(2, 5), 40, 10, 8)];
        let (eng, l) = layout_with_imgs("a $x$ b", usize::MAX, 400.0, &imgs);
        let p = &l.imgs[0];
        let mid_y = p.y + 5.0;
        assert_eq!(eng.hit(&l, p.x + 1.0, mid_y), 2, "图左半 → 节点首");
        assert_eq!(eng.hit(&l, p.x + 39.0, mid_y), 5, "图右半 → 节点尾");
    }

    #[test]
    fn caret_at_node_edge_shows_source_not_image() {
        // 激活规则闭区间：caret 贴节点首/尾即显源码（占位被忽略）
        for caret in [2, 5] {
            let imgs = [img(Range::new(2, 5), 40, 10, 8)];
            let (_, l) = layout_with_imgs("a $x$ b", caret, 400.0, &imgs);
            assert_eq!(l.layout_text(), "a $x$ b", "caret={caret} 应显源码");
            assert!(l.imgs.is_empty());
        }
        let imgs = [img(Range::new(2, 5), 40, 10, 8)];
        let (_, l) = layout_with_imgs("a $x$ b", 0, 400.0, &imgs);
        assert_eq!(l.imgs.len(), 1, "光标在外出图");
    }

    #[test]
    fn narrow_width_wrap_keeps_single_placement() {
        // 「aaaaa $x$ bbbbb」：窄宽强制折行；占位段无论是否跨视觉行，只出一张图，
        // 图上命中恒落节点区间 [6,9)（逐字节：a×5=0..4 ' '=5 $=6 x=7 $=8 ' '=9 b×5=10..14）
        let imgs = [img(Range::new(6, 9), 30, 10, 8)];
        let (eng, l) = layout_with_imgs("aaaaa $x$ bbbbb", usize::MAX, 60.0, &imgs);
        assert!(l.height > BASE_LINE_HEIGHT, "窄宽应折行");
        assert_eq!(l.imgs.len(), 1);
        let p = &l.imgs[0];
        let off = eng.hit(&l, p.x + 15.0, p.y + 5.0);
        assert!((6..=9).contains(&off), "图上命中须落节点区间: {off}");
    }

    #[test]
    fn math_inside_strong_keeps_placeholder() {
        // 「**a $x$**」逐字节：*=0 *=1 a=2 ' '=3 $=4 x=5 $=6 *=7 *=8
        // Strong range=[0,9) content=[2,7)；Math range=[4,7)：强标记消隐与公式占位并存
        let imgs = [img(Range::new(4, 7), 20, 10, 8)];
        let (_, l) = layout_with_imgs("**a $x$**", usize::MAX, 200.0, &imgs);
        assert_eq!(l.imgs.len(), 1);
        assert!(!l.layout_text().contains('$'), "公式源码不入排版");
        assert!(!l.layout_text().contains('*'), "强标记消隐不受占位影响");
    }

    // ---- 文档内图片占位（切片 13）：同公式占位机制，键为解析后的路径 ----
    // 「a ![alt](p.png) b」逐字节：a=0 ' '=1 !=2 [=3 a=4 l=5 t=6 ]=7 (=8 p=9 . . n g=9..14 )=14
    // Image range=[2,15) content=[4,7) url=[9,14)

    #[test]
    fn image_placeholder_replaces_whole_node() {
        let (eng, l) = layout_with_imgs(
            "a ![alt](p.png) b",
            usize::MAX,
            400.0,
            &[image(Range::new(2, 15), 40, 10)],
        );
        assert_eq!(l.imgs.len(), 1);
        assert_eq!(l.imgs[0].key, InlineAsset::Image("p.png".into()));
        assert!(!l.layout_text().contains("alt") && !l.layout_text().contains("p.png"));
        // 图占位左右半 → 节点首/尾（点击进入源码态）
        let p = &l.imgs[0];
        assert_eq!(eng.hit(&l, p.x + 1.0, p.y + 5.0), 2);
        assert_eq!(eng.hit(&l, p.x + 39.0, p.y + 5.0), 15);
    }

    #[test]
    fn missing_image_falls_back_to_alt_text() {
        // 未提供图（加载失败/远程 URL）：标记消隐，只显 alt
        let (_, l) = layout_of("a ![alt](p.png) b", 400.0);
        assert_eq!(l.layout_text(), "a alt b");
    }

    #[test]
    fn image_activated_by_caret_shows_source() {
        for caret in [2, 4, 15] {
            let (_, l) = layout_with_imgs(
                "a ![alt](p.png) b",
                caret,
                400.0,
                &[image(Range::new(2, 15), 40, 10)],
            );
            assert_eq!(l.layout_text(), "a ![alt](p.png) b", "caret={caret}");
            assert!(l.imgs.is_empty());
        }
    }

    #[test]
    fn tall_image_top_clamped_to_line_top_and_extends_block() {
        let (_, l) = layout_with_imgs(
            "a ![alt](p.png) b",
            usize::MAX,
            400.0,
            &[image(Range::new(2, 15), 40, 100)],
        );
        let p = &l.imgs[0];
        assert!(p.y >= 0.0, "图片顶部不得上侵前一块: y={}", p.y);
        assert!(
            l.height >= p.y + 100.0,
            "块高须覆盖图底: h={} y={}",
            l.height,
            p.y
        );
        assert!(l.height > BASE_LINE_HEIGHT);
    }

    // ---- 选区高亮（切片 3）：doc 区间 → 各视觉行 x 矩形（物理像素） ----

    #[test]
    fn selection_rects_single_line_matches_caret_span() {
        let (eng, l) = layout_of("hello", 200.0);
        let rects = eng.selection_rects(&l, 0, 5);
        assert_eq!(rects.len(), 1);
        let (x, y, w, h) = rects[0];
        let (x0, _, ch) = eng.caret_xy(&l, 0);
        let (x1, _, _) = eng.caret_xy(&l, 5);
        assert_eq!((x, y, h), (x0, 0.0, ch));
        assert_eq!(w, x1 - x0, "高亮宽度 = 两端光标 x 之差");
    }

    #[test]
    fn selection_rects_span_newline_per_line() {
        // 「ab\ncd」选 [1,4)：行1 高亮 "b"，行2 高亮 "c"，行间换行符不产生矩形
        let (eng, l) = layout_of("ab\ncd", 200.0);
        let rects = eng.selection_rects(&l, 1, 4);
        assert_eq!(rects.len(), 2);
        let (x_b, y1, _) = eng.caret_xy(&l, 1);
        assert_eq!(rects[0].0, x_b);
        assert_eq!(rects[0].1, y1);
        let (x_c, y2, h2) = eng.caret_xy(&l, 3);
        let (x_d, _, _) = eng.caret_xy(&l, 4);
        assert_eq!((rects[1].0, rects[1].1, rects[1].3), (x_c, y2, h2));
        assert_eq!(rects[1].2, x_d - x_c);
    }

    #[test]
    fn selection_rects_collapsed_is_empty() {
        let (eng, l) = layout_of("hello", 200.0);
        assert!(eng.selection_rects(&l, 2, 2).is_empty());
    }

    #[test]
    fn selection_rects_respect_reveal_elision() {
        // caret=0 时「**b**」标记消隐：选 doc [4,5)="b"，高亮须落在可见 "b" 的 x 上
        let (eng, l) = layout_with_caret("a **b** c", 0, 200.0);
        let rects = eng.selection_rects(&l, 4, 5);
        assert_eq!(rects.len(), 1);
        let (x0, _, _) = eng.caret_xy(&l, 4);
        let (x1, _, _) = eng.caret_xy(&l, 5);
        assert_eq!(rects[0].0, x0);
        assert_eq!(rects[0].2, x1 - x0);
    }

    // ---- 视觉行首尾（切片 4）：Home/End 落在当前视觉行两端 ----

    #[test]
    fn caret_xy_on_wrapped_line_uses_its_own_run() {
        // 回归：折行块内靠后偏移的光标曾恒落第一视觉行（line_bases 仅记硬换行，
        // 循环在首个 line_i 匹配的 run 即返回）。文档尾光标必须落在更靠下的视觉行。
        let (eng, l) = layout_of("aaaa aaaa aaaa", 30.0);
        let (_, y0, _) = eng.caret_xy(&l, 0);
        let (_, y1, _) = eng.caret_xy(&l, 14);
        assert!(y1 > y0, "折行后文档尾光标 y 应大于首行: y0={y0} y1={y1}");
    }

    #[test]
    fn visual_line_edges_single_line() {
        let (eng, l) = layout_of("hello", 200.0);
        assert_eq!(eng.visual_line_edges(&l, 2), (0, 5));
        assert_eq!(eng.visual_line_edges(&l, 0), (0, 5));
        assert_eq!(eng.visual_line_edges(&l, 5), (0, 5));
    }

    #[test]
    fn visual_line_edges_hard_break_lines() {
        // 「ab\ncd」逐字节：a=0 b=1 \n=2 c=3 d=4，文档长 5
        let (eng, l) = layout_of("ab\ncd", 200.0);
        assert_eq!(eng.visual_line_edges(&l, 1), (0, 2), "第一行");
        assert_eq!(eng.visual_line_edges(&l, 2), (0, 2), "第一行行尾（\\n 处）");
        assert_eq!(eng.visual_line_edges(&l, 3), (3, 5), "第二行行首");
        assert_eq!(eng.visual_line_edges(&l, 4), (3, 5), "第二行");
    }

    #[test]
    fn visual_line_edges_wrap_stays_within_visual_line() {
        // 窄宽强制折行（切片 3 已证 ≥2 视觉行）：末偏移在末视觉行，行尾即文档尾
        let (eng, l) = layout_of("aaaa aaaa aaaa", 30.0);
        let (s, e) = eng.visual_line_edges(&l, 14);
        assert!(s > 0, "折行后末视觉行起点必非文档起点: s={s}");
        assert!(s <= 14);
        assert_eq!(e, 14, "末视觉行行尾 = 文档尾");
        // 与 hit 直接换算一致（shell 接线的参照行为）
        let (_, y, _) = eng.caret_xy(&l, 14);
        assert_eq!(eng.hit(&l, 0.0, y + 1.0), s);
        assert_eq!(eng.hit(&l, f32::MAX, y + 1.0), e);
    }

    #[test]
    fn visual_line_edges_respect_elision() {
        // caret=0 时「**b**」标记消隐，layout_text="a b c"；全文长 9，单行
        let (eng, l) = layout_with_caret("a **b** c", 0, 200.0);
        assert_eq!(eng.visual_line_edges(&l, 8), (0, 9));
    }

    #[test]
    fn selection_rects_wrap_gives_rect_per_visual_line() {
        // 窄宽度强制折行：全选时每条视觉行一个矩形，y 递增、宽高为正
        let (eng, l) = layout_of("aaaa aaaa aaaa", 30.0);
        let rects = eng.selection_rects(&l, 0, 14);
        assert!(rects.len() >= 2, "折行后应至少两行高亮: {:?}", rects);
        let mut prev_y = -1.0;
        for &(_, y, w, h) in &rects {
            assert!(w > 0.0 && h > 0.0);
            assert!(y > prev_y, "视觉行 y 严格递增");
            prev_y = y;
        }
    }
}
