//! cosmic-text 排版引擎：块级布局、命中检测、光标坐标、光栅化。
//! 布局单位即物理像素——缩放因子由 shell 在 width/metrics 上乘入，本层无 DPI 概念。
//!
//! 消隐机制（统一）：行尾 `\r` 与非激活 inline 节点的标记段都不参与排版（`\r` 交给
//! cosmic 会渲染成缺字形框；标记消隐即 Reveal）。两类区间统一进消隐集合，由
//! `OffsetMap` 负责 doc↔layout 坐标换算——对外偏移一律为文档坐标。
//! 激活规则 v0：`range.start <= caret <= range.end` 闭区间，逐节点独立、嵌套各自生效。

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use cosmic_text::{
    Attrs, Buffer, Color, Cursor, Family, FontSystem, Metrics, Shaping, Style as FontStyle,
    SwashCache, Weight, Wrap,
};
use veil_core::{task_span, BlockKind, InlineKind, InlineNode, Range};

use super::highlight::{highlight, to_styles};
use super::linebreak;
use super::style::{style_spans, Style};
use crate::geometry::{BASE_FONT_SIZE, BASE_LINE_HEIGHT};
use crate::theme::{SyntaxColors, Theme};

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

/// 块级字号倍率（标题）。样式语义见 style.rs 头注。
fn block_scale(kind: BlockKind) -> f32 {
    match kind {
        BlockKind::AtxHeading(1) | BlockKind::SetextHeading(1) => 1.6,
        BlockKind::AtxHeading(2) | BlockKind::SetextHeading(2) => 1.4,
        BlockKind::AtxHeading(3) => 1.2,
        BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_) => 1.1,
        _ => 1.0,
    }
}

/// 单块排版上限（v0 保护）：超过只排前缀并显式降级，不再为整块成形。
/// 根因：`shape_until_scroll` 会为块内全部字符生成字形，光栅又按「宽 × 块高 × 4B」
/// 分配，1MB 文本即可放大到约 1.9GB。64 KiB / 512 硬行把最坏情况限制在几十 MB，
/// 且正常段落与常规代码块远低于该值，不受影响。
pub(crate) const MAX_BLOCK_BYTES: usize = 64 * 1024;
pub(crate) const MAX_BLOCK_LINES: usize = 512;
/// 光栅面积上限（像素）：极端版心/字体度量下的安全网。
pub(crate) const MAX_RASTER_PX: usize = 24_000_000;

/// 超限提示文案：由绘制层画在内容之后，明确告知「未渲染」而非静默截断。
pub(crate) fn oversize_notice(full_len: usize) -> String {
    format!(
        "该块约 {:.1} MB，超出单块渲染上限，后续内容未渲染；源码完整保留",
        full_len as f64 / (1024.0 * 1024.0)
    )
}

/// 取可排版前缀：先按字节上限截到字符边界，再按硬行上限回退到行首。
/// 返回 (前缀, 是否截断)。优先整行截断，避免把一行切成半个字形。
fn cap_body(body: &str) -> (&str, bool) {
    let mut end = body.len().min(MAX_BLOCK_BYTES);
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    let mut lines = 0usize;
    let mut cut = end;
    for (i, b) in body.as_bytes().iter().enumerate().take(end) {
        if *b == b'\n' {
            lines += 1;
            if lines >= MAX_BLOCK_LINES {
                cut = i; // 不含该换行：硬行数不超过上限
                break;
            }
        }
    }
    end = end.min(cut);
    if end >= body.len() {
        return (body, false);
    }
    // 回退到最近的换行（最多 4 KiB，避免极长行丢掉过多内容）
    if let Some(p) = body[..end].rfind('\n') {
        if end - p <= 4096 {
            end = p;
        }
    }
    (&body[..end], true)
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

    /// 纯插入：零宽源码区间 → 有宽展示文本（脚注定义的编号前缀 / 块末 `↩`）。
    /// 插入段不消费源码字符，doc 坐标只作锚点；fill 同样不得含换行。
    fn insert(doc: usize, text: impl Into<String>) -> Self {
        Sub {
            doc_start: doc,
            doc_end: doc,
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
        // 同起点：零宽插入最先（否则会被同起点的替换段包含判定吃掉），
        // 其次长者在前（占位段先于其内消隐段入表，保证包含判定命中）。
        sorted.sort_unstable_by(|a, b| {
            a.doc_start.cmp(&b.doc_start).then_with(|| {
                match (a.doc_end == a.doc_start, b.doc_end == b.doc_start) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => b.doc_end.cmp(&a.doc_end),
                }
            })
        });
        let mut accepted: Vec<Sub> = Vec::new();
        for sub in sorted {
            let (s, e) = (sub.doc_start.min(doc_len), sub.doc_end.min(doc_len));
            if s > e || (s == e && sub.fill.is_empty()) {
                continue;
            }
            if s == e {
                // 纯插入：不占源码字符，只要求不落在已接受替换段内部（会把替换段劈开）。
                if accepted.last().is_some_and(|last| s < last.doc_end) {
                    continue;
                }
                accepted.push(Sub {
                    doc_start: s,
                    doc_end: s,
                    fill: sub.fill,
                });
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
            // 零宽插入段：doc 锚点落在装饰之前（块末光标停在 `↩` 前，而不是跳到最后）。
            Some(seg) if seg.doc_len == 0 => seg.layout_start,
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

/// 脚注渲染上下文：全文编号表（id → 序号）+ 前缀装饰色。
/// 为什么独立成表：编号是全文派生量（按首次引用顺序），若每个块回查会话就是
/// O(块数 × 全文)；这里每轮排版构建一次，块内查询为 HashMap O(1)。
pub(crate) struct FootnoteCtx {
    numbers: HashMap<String, usize>,
    muted: [u8; 3],
}

impl FootnoteCtx {
    pub(crate) fn new(ids: Vec<String>, muted: [u8; 3]) -> Self {
        let numbers = ids
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id, i + 1))
            .collect();
        Self { numbers, muted }
    }

    /// 未引用/未定义 → None（调用方保留源码原文，不伪造编号）。
    fn number(&self, id: &str) -> Option<usize> {
        self.numbers.get(id).copied()
    }

    fn muted(&self) -> [u8; 3] {
        self.muted
    }
}

/// 脚注代次：`footnote_ids()` 的顺序与编号的确定性哈希。
/// 为什么进缓存键：文首新增一个引用会改变全文编号（含文末定义块的前缀），
/// 但定义块自身文本没变；只按文本缓存会残留旧编号。同一次运行内稳定即可。
pub(crate) fn footnote_epoch(ids: &[String]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ids.len().hash(&mut h);
    for (i, id) in ids.iter().enumerate() {
        id.hash(&mut h);
        (i + 1).hash(&mut h);
    }
    h.finish()
}

/// 块级装饰段：`subs` 之外还记录装饰在 layout 中的宽度，
/// 供 `layout_block` 为「不来自源码」的前缀/后缀补齐样式 span。
#[derive(Default)]
struct BlockDecor {
    subs: Vec<Sub>,
    prefix_len: usize,
    suffix_len: usize,
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
    /// 删除线区间（块相对 doc 坐标，含标记；paint 层经 selection_rects 取可视矩形）。
    pub strikes: Vec<(usize, usize)>,
    /// 全部行内节点区间（**块相对坐标**，先序遍历）：Reveal 激活指纹的输入。
    /// 用相对坐标而非文档绝对坐标，保证块整体平移（增量重解析保留 id）后指纹仍平移不变。
    node_ranges: Vec<Range>,
    /// 行内图片区间（**块相对坐标**，按起点排序）：激活位决定占位/显源码，同属排版输入。
    img_ranges: Vec<Range>,
    /// 源文本换行符的块相对偏移（不含尾 `\n`）：引用块按行激活判定的输入。
    source_newlines: Vec<usize>,
    /// 原块正文字节数（不含尾换行）；大于排版覆盖范围时说明只排了前缀。
    pub full_len: usize,
    /// 是否因超单块上限而只排前缀（绘制层据此画降级提示）。
    pub truncated: bool,
}

impl BlockLayout {
    pub fn layout_text(&self) -> &str {
        &self.layout_text
    }

    /// Reveal 激活指纹：块级激活（引用按行、列表/分隔线按块）与全部行内节点的激活位。
    /// 仅依赖 caret 与块结构（结构随 BlockId 固化），供派生缓存按因果键判定是否重排。
    /// 指纹相等 ⇔ 排版依赖的激活状态相同（64 位 FNV 折叠；碰撞概率忽略不计）。
    pub(crate) fn reveal_state(&self, kind: &BlockKind, block: Range, caret: usize) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        // caret 在块外 → 任何节点都不可能激活。必须显式守卫：块未参与重解析（id 保留）
        // 但整体平移后，旧布局的指纹输入仍会与块内 caret 误配。
        let inside = block.start <= caret && caret <= block.end;
        let rel = if inside { caret - block.start } else { 0 };
        match kind {
            BlockKind::BlockQuote => {
                // 引用按行：line=0 即块外，天然区分「块外」与「块内任意行」。
                let line = if inside {
                    self.source_newlines.partition_point(|&o| o < rel) as u64 + 1
                } else {
                    0
                };
                fnv(&mut h, line);
            }
            // 块级标记显隐按整块 caret 判定（`block_subs`）：标题 `#`/Setext 下划线/围栏
            // ```` ``` ````/列表符号/分隔线/脚注定义前缀。漏折任一种 → caret 进出块时缓存不变，残留旧态。
            BlockKind::ListItem { .. }
            | BlockKind::ThematicBreak
            | BlockKind::AtxHeading(_)
            | BlockKind::SetextHeading(_)
            | BlockKind::FencedCode(_)
            | BlockKind::FootnoteDef => {
                fnv(&mut h, inside as u64);
            }
            _ => {}
        }
        for i in 0..self.node_ranges.len() {
            // 与 collect_elide 同源：深层节点优先（caret 落在后代内则祖先不显示标记）。
            let active = inside && deep_active_flat(&self.node_ranges, i, rel);
            fnv(&mut h, (i as u64) << 1 | active as u64);
        }
        // 图片激活位：与节点分域折叠（前置 u64::MAX 作域分隔），避免与节点索引位混淆。
        if !self.img_ranges.is_empty() {
            fnv(&mut h, u64::MAX);
            for (i, r) in self.img_ranges.iter().enumerate() {
                let active = inside && node_active(*r, rel);
                fnv(&mut h, (i as u64) << 1 | active as u64);
            }
        }
        h
    }
}

/// FNV-1a 折叠：缓存键只需稳定与分散，不需要密码学强度。
fn fnv(h: &mut u64, v: u64) {
    *h = (*h ^ v).wrapping_mul(0x0100_0000_01b3);
}

pub struct LayoutEngine {
    font_system: FontSystem,
    swash: SwashCache,
    /// DPI 缩放：metrics 统一乘此值，布局单位保持物理像素。
    scale: f32,
    /// 代码块语法高亮色板（随主题切换更新）。
    syntax: SyntaxColors,
    /// 链接前景色（随主题切换更新）。
    link_color: [u8; 3],
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
            syntax: SyntaxColors::default_light(),
            link_color: Theme::light().link,
        }
    }

    /// 更新语法高亮色板（主题切换时调用）。
    pub fn set_syntax_colors(&mut self, colors: SyntaxColors) {
        self.syntax = colors;
    }

    /// 更新链接前景色（主题切换时调用）。
    pub fn set_link_color(&mut self, color: [u8; 3]) {
        self.link_color = color;
    }

    /// 当前链接前景色（表格富文本排版复用）。
    pub(crate) fn link_color(&self) -> [u8; 3] {
        self.link_color
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
    /// 与正文同套排版规则：kinsoku 胶水 + 中西文间距经 build_layout_text 注入，
    /// OffsetMap 保证组合串内光标偏移（IME 报告的原文档坐标）正确映射。
    pub fn layout_preedit(&mut self, text: &str) -> BlockLayout {
        let metrics = Metrics::new(BASE_FONT_SIZE * self.scale, BASE_LINE_HEIGHT * self.scale);
        let block = Range {
            start: 0,
            end: text.len(),
        };
        let (layout_text, map, line_bases, source_newlines) =
            build_layout_text(text, block, &[], usize::MAX, &[], true, false);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_text(&layout_text, &Attrs::new(), Shaping::Advanced, None);
        buffer.set_size(None, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let height = buffer
            .layout_runs()
            .map(|r| r.line_top + r.line_height)
            .fold(metrics.line_height, f32::max);
        BlockLayout {
            buffer,
            map,
            line_bases,
            layout_text,
            height,
            imgs: Vec::new(),
            strikes: Vec::new(),
            node_ranges: Vec::new(),
            img_ranges: Vec::new(),
            source_newlines,
            full_len: text.len(),
            truncated: false,
        }
    }

    /// 单行文本度量（宽, 高）：仅 shape 不 rasterize，供浮层/模态按内容定宽与折行。
    pub(crate) fn measure(&mut self, text: &str) -> (f32, f32) {
        let layout = self.layout_preedit(text);
        let w = layout
            .buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0, f32::max);
        (w, layout.height)
    }

    /// 排版单块。`text` 为块文本（含内部换行；尾换行可有可无）；`inlines`/`block` 为文档区间；
    /// `caret` 为文档绝对偏移，驱动 Reveal 激活判定（无光光标语义传 `usize::MAX` 全消隐）。
    /// `imgs`：已渲染的非激活行内公式/图片（切片 8/13）——节点整段替换为 NBSP 占位
    /// （宽≈位图宽），排版后定位占位首字形得 PlacedImg（基线对齐）。激活节点（caret
    /// 闭区间内）的 img 忽略。
    /// `reveal_all`：源码模式——块级标记与行内标记全部显原文、不做图片占位。
    /// 已知 v0 简化：高超一行的行内公式/图片可能与同块下一视觉行重叠（块高已扩展
    /// 不溢出版心）；图片顶部钳在行顶（不上侵前一块），公式保留原基线；
    /// 标题内公式/图片不随标题字号放大（尺寸由渲染侧/图片自然尺寸决定）。
    #[allow(clippy::too_many_arguments)] // 内部布局函数参数天然多
    pub fn layout_block(
        &mut self,
        kind: BlockKind,
        text: &str,
        block: Range,
        inlines: &[InlineNode],
        width: f32,
        caret: usize,
        imgs: &[InlineImg],
        reveal_all: bool,
    ) -> BlockLayout {
        // 无编号表的调用方（打印等）降级：定义标记仍消隐，引用保留 `[^id]` 原文。
        self.layout_block_with_footnotes(
            kind, text, block, inlines, width, caret, imgs, reveal_all, None,
        )
    }

    /// 带脚注编号表的排版入口：编号在全文范围内派生，由调用方每轮构建一次并传引用。
    #[allow(clippy::too_many_arguments)] // 内部布局函数参数天然多
    pub(crate) fn layout_block_with_footnotes(
        &mut self,
        kind: BlockKind,
        text: &str,
        block: Range,
        inlines: &[InlineNode],
        width: f32,
        caret: usize,
        imgs: &[InlineImg],
        reveal_all: bool,
        footnotes: Option<&FootnoteCtx>,
    ) -> BlockLayout {
        let full = text.strip_suffix('\n').unwrap_or(text);
        // 单块上限（v0）：只排前缀，超出部分不参与成形/光栅，避免内存放大
        let (body, mut truncated) = cap_body(full);
        let scale = block_scale(kind.clone()) * self.scale;
        let metrics = Metrics::new(BASE_FONT_SIZE * scale, BASE_LINE_HEIGHT * scale);
        // NBSP 占位段：NBSP 为 2 字节，个数 = 图宽 / NBSP 宽四舍五入（误差 ≤ 半个 NBSP）。
        let nbsp_w = if imgs.is_empty() {
            1.0
        } else {
            self.measure_nbsp(metrics)
        };
        let mut kept: Vec<&InlineImg> = Vec::new();
        // 脚注引用 `[^id]` → `[n]` 的替换区间（块相对 doc 坐标），供 style_spans 之后染链接色。
        let mut footnote_links: Vec<(usize, usize)> = Vec::new();
        // 脚注定义块的编号前缀 / 块末 ↩ 装饰（不来自源码区间，需单独补样式）。
        let is_footnote_def = matches!(kind, BlockKind::FootnoteDef);
        let decor = if !reveal_all && is_footnote_def {
            footnote_def_decor(body, block, caret, footnotes)
        } else {
            BlockDecor::default()
        };
        // 块级替换（切片 12）先入表；行内公式占位段随后按起点排序归并。
        // 源码模式：不做任何块级替换，标记原样显示。
        let mut subs: Vec<Sub> = if reveal_all {
            Vec::new()
        } else if is_footnote_def {
            decor.subs.clone()
        } else {
            block_subs(kind.clone(), body, block, caret)
        };
        if !reveal_all {
            let (ref_subs, links) = footnote_ref_subs(inlines, block, caret, body, footnotes);
            subs.extend(ref_subs);
            footnote_links = links;
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
        }
        let apply_cjk = !matches!(
            kind,
            BlockKind::FencedCode(_)
                | BlockKind::IndentedCode
                | BlockKind::Math
                | BlockKind::Table
        );
        let (layout_text, map, line_bases, source_newlines) =
            build_layout_text(body, block, inlines, caret, &subs, apply_cjk, reveal_all);
        let mut node_ranges = Vec::new();
        collect_node_ranges(inlines, &mut node_ranges);
        // 指纹输入一律存块相对坐标：增量重解析保留 BlockId 而块 range 可能整体平移，
        // 绝对坐标会在命中校验时把新 caret 误判为落在旧区间内（基线 memo 性质测试反例）。
        for r in node_ranges.iter_mut() {
            *r = Range {
                start: r.start.saturating_sub(block.start),
                end: r.end.saturating_sub(block.start),
            };
        }
        // 行内图片同理：激活位决定是否用 NBSP 占位（显源码），也是排版输入。
        let mut img_ranges: Vec<Range> = imgs
            .iter()
            .map(|im| Range {
                start: im.range.start.saturating_sub(block.start),
                end: im.range.end.saturating_sub(block.start),
            })
            .collect();
        img_ranges.sort_by_key(|r| (r.start, r.end));
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let default_attrs = attrs_of(kind.clone(), Style::default(), self.link_color);
        // style_spans 产出文档坐标，经映射转 layout 坐标；整体落入消隐/占位区的 span 零宽丢弃。
        // 占位 NBSP 区无样式来源，显式补默认样式 span（cosmic 排版文本 = 各 span 顺次拼接，
        // 覆盖必须完备且有序）；指针序即 layout 偏移序。
        //
        // FencedCode 块带语言标记时走语法高亮（inlines 为空，style_spans 仅产出默认段）。
        let style_segments: Vec<(usize, usize, Style)> = match &kind {
            BlockKind::FencedCode(Some(lang)) => {
                let tk = highlight(lang, body);
                to_styles(&tk, body.len(), &self.syntax, Style::default())
            }
            _ => style_spans(body.len(), block, inlines),
        };
        // `[n]` 的展示文本并非源码字符，样式表里没有它的颜色来源；
        // 按替换区间切分样式段并置 link（style_spans 会把同式相邻段合并，不能整段改）。
        let style_segments = with_link_spans(style_segments, &footnote_links, body.len());
        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        for &(s, e, st) in &style_segments {
            let (ls, le) = (map.doc_to_layout(s), map.doc_to_layout(e));
            if ls < le {
                spans.push((&layout_text[ls..le], attrs_of(kind.clone(), st, self.link_color)));
            }
        }
        // 装饰 span：前缀 `n. `（muted）与块末 `↩`（链接色）由插入段产生，
        // 必须显式补样式；否则会落进默认 attrs（前缀不高亮、后缀无链接色）。
        if decor.prefix_len > 0 {
            let ls = map.doc_to_layout(0);
            let le = (ls + decor.prefix_len).min(layout_text.len());
            if ls < le {
                let st = Style {
                    color: footnotes.map(FootnoteCtx::muted),
                    ..Default::default()
                };
                spans.push((&layout_text[ls..le], attrs_of(kind.clone(), st, self.link_color)));
            }
        }
        if decor.suffix_len > 0 {
            let ls = map.doc_to_layout(body.len());
            let le = (ls + decor.suffix_len).min(layout_text.len());
            if ls < le {
                let st = Style {
                    link: true,
                    ..Default::default()
                };
                spans.push((&layout_text[ls..le], attrs_of(kind.clone(), st, self.link_color)));
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
        // WordOrGlyph：按 UAX #14 断行机会折行（CJK 逐字可断 + 行首/行尾禁则
        // 由 CL/OP 类天然生效），单个超宽词（长 URL 等）回退逐字形断行。
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_size(Some(width), None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let mut height = buffer
            .layout_runs()
            .map(|r| r.line_top + r.line_height)
            .fold(metrics.line_height, f32::max);
        // 面积上限与排版同源：先按「最宽视觉行 × 块高」预估光栅像素，
        // 超限就截短块高并标记降级。几何与光栅必须一致，否则绘制会在块尾留白。
        let notice_h = if truncated {
            BASE_LINE_HEIGHT * self.scale
        } else {
            0.0
        };
        let max_line_w = buffer
            .layout_runs()
            .flat_map(|r| r.glyphs.iter())
            .map(|g| g.x + g.w)
            .fold(width, f32::max)
            .max(1.0);
        let budget_h = MAX_RASTER_PX as f32 / max_line_w;
        if height + notice_h > budget_h {
            height = (budget_h - notice_h).max(metrics.line_height);
            truncated = true;
        }
        height += notice_h;
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
        probe_break_violations(&buffer, false);
        BlockLayout {
            buffer,
            map,
            line_bases,
            layout_text,
            height,
            imgs: placed,
            strikes: style_segments
                .iter()
                .filter(|(_, _, st)| st.strikethrough)
                .map(|&(s, e, _)| (s, e))
                .collect(),
            node_ranges,
            img_ranges,
            source_newlines,
            full_len: full.len(),
            truncated,
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

    /// 各视觉行行顶（块内物理像素，升序）：打印分页的行级候选断点（3b）。
    pub fn line_tops(&self, layout: &BlockLayout) -> Vec<f32> {
        layout.buffer.layout_runs().map(|r| r.line_top).collect()
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
    /// 超限降级的块只渲染前缀高度：落点统一钳进可视区，避免光标画到下一块上。
    pub fn caret_xy(&self, layout: &BlockLayout, offset: usize) -> (f32, f32, f32) {
        let (x, y, h) = self.caret_xy_raw(layout, offset);
        (x, y.min((layout.height - h).max(0.0)), h)
    }

    fn caret_xy_raw(&self, layout: &BlockLayout, offset: usize) -> (f32, f32, f32) {
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
            if run.line_top >= layout.height {
                continue; // 超出降级可视区（面积上限截短）不画选区
            }
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
    /// 悬挂标点 v0 在此生效：满行末字为闭标点时光栅右移半字宽（排版几何不变，
    /// caret/命中/选区仍按未位移坐标——光标落在标点之后恰与位移后墨缘对齐）。
    pub(crate) fn rasterize_buffer(
        &mut self,
        buffer: &Buffer,
        height: f32,
        color: [u8; 3],
    ) -> (Vec<u8>, u32, u32) {
        let width_opt = buffer.size().0;
        let mut max_w: f32 = 0.0;
        for run in buffer.layout_runs() {
            let (dx, hi) = hang_shift(&run, width_opt);
            for (i, g) in run.glyphs.iter().enumerate() {
                let shift = if Some(i) == hi { dx } else { 0.0 };
                max_w = max_w.max(g.x + shift + g.w);
            }
        }
        let (w, h) = (
            max_w.ceil().max(1.0) as usize,
            height.ceil().max(1.0) as usize,
        );
        // 光栅面积安全网：单块上限之下仍可能因版心极窄/行极密而偏大，
        // 超出部分直接裁掉（像素越界本就有边界检查），绝不为病态输入分配巨缓冲。
        let h = if w * h > MAX_RASTER_PX {
            (MAX_RASTER_PX / w).max(1)
        } else {
            h
        };
        let mut px = vec![0u8; w * h * 4];
        for run in buffer.layout_runs() {
            let (dx, hi) = hang_shift(&run, width_opt);
            for (i, g) in run.glyphs.iter().enumerate() {
                let shift = if Some(i) == hi { dx } else { 0.0 };
                let base = g.color_opt.map(|c| [c.r(), c.g(), c.b()]).unwrap_or(color);
                let physical = g.physical((shift, run.line_y), 1.0);
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

/// 行尾可悬挂标点（闭标点，墨形偏全角框左半）：满行时光栅右移半字宽，
/// 墨缘对齐版心右缘、标点凸出（悬挂式 v0）。居中墨形的省略号/破折号不在此列。
fn hangable_end(ch: char) -> bool {
    matches!(
        ch,
        '\u{3002}' | '\u{3001}' | '\u{FF0C}' | '\u{FF0E}' | '\u{FF1B}' | '\u{FF1A}'
            | '\u{FF1F}' | '\u{FF01}' | '\u{FF09}' | '\u{3011}' | '\u{300B}' | '\u{300D}'
            | '\u{300F}' | '\u{201D}' | '\u{2019}'
    )
}

/// 满行末字悬挂判定：返回 (右移量, 目标字形下标)。
/// 仅当行基本排满（行宽与版心差 ≤ 0.25 字号）、最右字形恰为单个可悬挂闭标点、
/// 且其 advance 右缘即行宽时生效；RTL 行与无定宽 buffer（preedit）不悬挂。
fn hang_shift(run: &cosmic_text::LayoutRun, width_opt: Option<f32>) -> (f32, Option<usize>) {
    let Some(width) = width_opt else {
        return (0.0, None);
    };
    if run.rtl {
        return (0.0, None);
    }
    let Some((i, g)) = run
        .glyphs
        .iter()
        .enumerate()
        .max_by(|a, b| (a.1.x + a.1.w).total_cmp(&(b.1.x + b.1.w)))
    else {
        return (0.0, None);
    };
    let tol = g.font_size * 0.25;
    if (width - run.line_w).abs() > tol || (g.x + g.w - run.line_w).abs() > tol {
        return (0.0, None); // 未排满/超宽词/末字形右缘异常：不悬挂
    }
    let cluster = &run.text[g.start..g.end];
    let mut it = cluster.chars();
    let (Some(c), None) = (it.next(), it.next()) else {
        return (0.0, None);
    };
    if !hangable_end(c) {
        return (0.0, None);
    }
    (g.w * 0.5, Some(i))
}

/// 激活规则 v0（Reveal）：caret 落在节点闭区间内即激活（显示标记）。
/// 闭区间让光标贴标记边缘时即展开，与 Typora 行为对齐。嵌套由 `collect_elide`
/// 的「深层优先」规则裁决（见 `deep_active`），本函数只回答「区间是否覆盖 caret」。
pub(crate) fn node_active(range: Range, caret: usize) -> bool {
    range.start <= caret && caret <= range.end
}

/// 深层节点优先：caret 落在本节点内、但不落在任何后代节点内时，本节点才显示标记。
/// 与 Typora 对齐：光标在 `**a *b* c**` 的 `b` 上只显示内层 `*b*`，外层 `**` 保持消隐；
/// 否则祖先与后代同时展开会出现「整行星号」的观感（D12-b）。
fn deep_active(node: &InlineNode, caret: usize) -> bool {
    node_active(node.range, caret) && !descendant_covers(&node.children, caret)
}

fn descendant_covers(nodes: &[InlineNode], caret: usize) -> bool {
    nodes
        .iter()
        .any(|n| node_active(n.range, caret) || descendant_covers(&n.children, caret))
}

/// 扁平（先序）区间列表上的同义判定：后代在 preorder 中紧随其后且起点落在本区间内。
/// 用于缓存指纹——`node_ranges` 只存区间不存树，但先序顺序足以还原父子关系。
fn deep_active_flat(ranges: &[Range], i: usize, caret: usize) -> bool {
    let r = ranges[i];
    if !node_active(r, caret) {
        return false;
    }
    for d in &ranges[i + 1..] {
        if d.start >= r.end {
            break; // 子树结束：后续为同级或更外层节点
        }
        if node_active(*d, caret) {
            return false;
        }
    }
    true
}

/// 非激活节点的标记段加入消隐集合（块相对 doc 坐标）：
/// `[range.start, content.start)` + `[content.end, range.end)`。
/// Link 的 content 为可见文本，故 `](url)` 整体消隐；Code/Math 的 children 恒空。
fn collect_elide(nodes: &[InlineNode], block: Range, caret: usize, out: &mut Vec<(usize, usize)>) {
    for n in nodes {
        if !deep_active(n, caret) {
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

/// 脚注引用替换：非激活 `[^id]` → `[n]`，同时返回替换区间供样式层染链接色。
/// `footnotes` 缺省或 id 无编号时**保留源码原文**：宁可露出 `[^id]`，
/// 也不把未定义引用伪装成已编号（不背叛源码）。
fn footnote_ref_subs(
    nodes: &[InlineNode],
    block: Range,
    caret: usize,
    body: &str,
    footnotes: Option<&FootnoteCtx>,
) -> (Vec<Sub>, Vec<(usize, usize)>) {
    fn walk(
        nodes: &[InlineNode],
        block: Range,
        caret: usize,
        body: &str,
        footnotes: Option<&FootnoteCtx>,
        subs: &mut Vec<Sub>,
        links: &mut Vec<(usize, usize)>,
    ) {
        for n in nodes {
            if n.kind == InlineKind::FootnoteRef && !node_active(n.range, caret) {
                let rs = n.range.start.saturating_sub(block.start);
                let re = n.range.end.saturating_sub(block.start).min(body.len());
                let cs = n.content.start.saturating_sub(block.start);
                let ce = n.content.end.saturating_sub(block.start).min(body.len());
                // 截断块（cap_body）可能切掉引用尾部：越界即保持原样，不猜。
                if rs < re && rs < body.len() && cs < ce && ce <= body.len() {
                    match footnotes.and_then(|f| f.number(&body[cs..ce])) {
                        Some(num) => {
                            subs.push(Sub::fill(rs, re, format!("[{num}]")));
                            links.push((rs, re));
                        }
                        None => subs.push(Sub::fill(rs, re, body[rs..re].to_string())),
                    }
                }
            }
            walk(
                &n.children,
                block,
                caret,
                body,
                footnotes,
                subs,
                links,
            );
        }
    }

    let mut subs = Vec::new();
    let mut links = Vec::new();
    walk(
        nodes, block, caret, body, footnotes, &mut subs, &mut links,
    );
    (subs, links)
}

/// 在样式段上叠加「链接色区间」（脚注引用替换段）。
/// `style_spans` 会把同式相邻段合并（引用两侧都是默认样式），因此必须按
/// 链接区间边界重新切分，再逐段求并，不能直接改整段。
fn with_link_spans(
    segments: Vec<(usize, usize, Style)>,
    links: &[(usize, usize)],
    text_len: usize,
) -> Vec<(usize, usize, Style)> {
    if links.is_empty() {
        return segments;
    }
    let mut bounds: Vec<usize> = vec![0, text_len];
    for &(s, e, _) in &segments {
        bounds.push(s);
        bounds.push(e);
    }
    for &(s, e) in links {
        bounds.push(s.min(text_len));
        bounds.push(e.min(text_len));
    }
    bounds.sort_unstable();
    bounds.dedup();
    let mut out: Vec<(usize, usize, Style)> = Vec::new();
    for w in bounds.windows(2) {
        let (s, e) = (w[0], w[1]);
        if s >= e {
            continue;
        }
        let mut st = segments
            .iter()
            .find(|(ss, se, _)| *ss <= s && e <= *se)
            .map(|x| x.2)
            .unwrap_or_default();
        if links.iter().any(|&(fs, fe)| fs <= s && e <= fe) {
            st.link = true;
        }
        match out.last_mut() {
            Some(last) if last.2 == st && last.1 == s => last.1 = e,
            _ => out.push((s, e, st)),
        }
    }
    out
}

/// 解析脚注定义首行前缀 `[^id]: `（块相对）→ (前缀结束偏移, id 区间)。
/// 与 core 的 `parse_footnote_def` 同口径（缩进 ≤3、id 允许字母数字/-/_、`]:`），
/// 但只做「标记消隐 + 编号」所需的定位，不重新解释块语义。
fn footnote_def_prefix(body: &str) -> Option<(usize, std::ops::Range<usize>)> {
    let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
    let bytes = body.as_bytes();
    if indent > 3 || bytes.get(indent) != Some(&b'[') || bytes.get(indent + 1) != Some(&b'^')
    {
        return None;
    }
    let id_start = indent + 2;
    let mut i = id_start;
    loop {
        let ch = body.get(i..)?.chars().next()?;
        if ch == ']' {
            break;
        }
        if !(ch.is_alphanumeric() || ch == '-' || ch == '_') {
            return None;
        }
        i += ch.len_utf8();
    }
    if i == id_start || body.as_bytes().get(i + 1) != Some(&b':') {
        return None;
    }
    let line_end = body.find('\n').unwrap_or(body.len());
    let mut prefix_end = (i + 2).min(line_end);
    // 标记后的一个空格一并消隐（GFM 惯例），与 content 区间是否含空格无关。
    if body.as_bytes().get(prefix_end) == Some(&b' ') && prefix_end < line_end {
        prefix_end += 1;
    }
    Some((prefix_end, id_start..i))
}

/// 脚注定义块装饰：消隐 `[^id]:`（含一个空格），有编号时以 `n. ` 插入到内容前，
/// 块末追加 `↩`。光标进入块 → 沿用既有 reveal 规则，返回空装饰（源码原样）。
fn footnote_def_decor(
    body: &str,
    block: Range,
    caret: usize,
    footnotes: Option<&FootnoteCtx>,
) -> BlockDecor {
    if node_active(block, caret) {
        return BlockDecor::default();
    }
    let Some((prefix_end, id_range)) = footnote_def_prefix(body) else {
        return BlockDecor::default();
    };
    let mut decor = BlockDecor {
        subs: vec![Sub::elide(0, prefix_end)],
        prefix_len: 0,
        // 未编号（定义未被引用）也保留返回标记：它是「定义块」的视觉标识，不依赖编号。
        suffix_len: "↩".len(),
    };
    if let Some(n) = footnotes.and_then(|f| f.number(&body[id_range])) {
        let text = format!("{n}. ");
        decor.prefix_len = text.len();
        decor.subs.push(Sub::insert(prefix_end, text));
    }
    decor.subs.push(Sub::insert(body.len(), "↩"));
    decor
}

/// CJK 字符判定：覆盖中日韩统一表意文字、韩文音节、平假名/片假名、
/// 全角标点/字母（Halfwidth & Fullwidth Forms）及 CJK 符号与标点。
/// v0 近似（不覆盖 Extension B-F 等生僻区；命中不到只是不加间距，不会崩）。
fn is_cjk(ch: char) -> bool {
    let cp = ch as u32;
    (0x4E00..=0x9FFF).contains(&cp)
        || (0x3400..=0x4DBF).contains(&cp)
        || (0xAC00..=0xD7AF).contains(&cp)
        || (0x3040..=0x309F).contains(&cp)
        || (0x30A0..=0x30FF).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0x3000..=0x303F).contains(&cp)
        || (0xFF00..=0xFFEF).contains(&cp)
}

/// 边界跳过：已存在空白/换行/全角空格 → 不再加间距。
fn cjk_boundary_skip(prev: char, next: char) -> bool {
    prev.is_whitespace() || next.is_whitespace()
        || prev == '\n' || next == '\n'
        || prev == '\r' || next == '\r'
        || (prev as u32) == 0x3000
        || (next as u32) == 0x3000
}

/// kinsoku 胶水字符 = 左粘 ∪ 右粘（判定与禁则表同源于 `view::linebreak`）。
/// 已知简化：fallback 字体链中 WJ 有约 1.3px 的零墨 advance（非 tofu），
/// 每个粘连点带来发丝级缝隙；`word_joiner_adds_no_advance` 守住 tofu 退化。
fn is_glue_punct(ch: char) -> bool {
    linebreak::glue_before(ch) || linebreak::glue_after(ch)
}

/// 中文排版替换表 v0 = kinsoku 胶水段 + 中西文自动间距。
/// 两趟产出互不重叠（间距在胶水字符两侧一律不注入），from_spans 可直接归并。
/// 被包含在 elide/占位区的 sub 会被 from_spans 丢弃，
/// 无需显式过滤（代码块、图片占位、行内公式标记都已在前一步进替换表）。
fn build_typo_subs(body: &str) -> Vec<Sub> {
    let mut subs = Vec::new();
    // 第一趟：kinsoku 胶水段。左粘字符前插 WJ（文首/硬行首无需，与换行粘连本就无断点）、
    // 右粘字符后插 WJ（行尾同理）；WJ×/×WJ 禁断使 run 内不断、与相邻字不断。
    for g in linebreak::glue_spans(body) {
        subs.push(Sub::fill(g.start, g.end, g.fill));
    }
    // 第二趟：CJK↔ASCII 边界间距——把「另一侧的首字符」替换为「空格+原字符」，
    // 让 OffsetMap 自动吸收新增字节。胶水字符两侧跳过（省略号/破折号是中文标点，
    // 不加间距；也避免与第一趟的 sub 撞区间）。
    let mut it = body.char_indices().peekable();
    while let Some((_i, ch)) = it.next() {
        let Some(&(j, next)) = it.peek() else { break };
        if is_glue_punct(ch) || is_glue_punct(next) {
            continue;
        }
        if cjk_boundary_skip(ch, next) {
            continue;
        }
        if is_cjk(ch) == is_cjk(next) {
            continue;
        }
        let nl = next.len_utf8();
        subs.push(Sub::fill(j, j + nl, format!(" {next}")));
    }
    subs
}

/// 真机断行探针：仅在 `VEIL_BREAK_PROBE` 设置时输出行首/行尾禁则违规。
/// 走 stderr（`2> probe.txt` 收集），默认路径只读一次环境变量，无逐块开销。
/// 用途：C2 归因——区分「胶水集合不全」与「断行器绕过断点」。
/// `include_hard=true` 用于表格单元格：单元格折行由本地显式换行实现，
/// 所有视觉行边界都应检查（仍排除首行首、末行尾）。
pub(crate) fn probe_break_violations(buffer: &Buffer, include_hard: bool) {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var_os("VEIL_BREAK_PROBE").is_some()) {
        return;
    }
    let runs: Vec<_> = buffer.layout_runs().collect();
    const WJ: char = '\u{2060}';
    let run_head = |run: &cosmic_text::LayoutRun| -> Option<char> {
        run.glyphs
            .first()
            .and_then(|g| run.text.get(g.start..g.end))
            .and_then(|s| s.chars().next())
    };
    let run_tail = |run: &cosmic_text::LayoutRun| -> Option<char> {
        run.glyphs
            .last()
            .and_then(|g| run.text.get(g.start..g.end))
            .and_then(|s| s.chars().last())
    };
    let visible = |c: char| c != WJ && !c.is_whitespace() && c != '\n' && c != '\r';
    // 行内图片/公式用 NBSP 占位（唯一 NBSP 来源，见上方 Sub::fill）。
    // 占位符本身不可见，但它是视觉行首/行尾的合法对象：其后的标点不属于「行首禁则」，
    // 否则会把 "…行内公式$E=mc^2$，块级公式…" 的逗号误报成行首（真机已确认视觉正确）。
    let starts_with_placeholder = |run: &cosmic_text::LayoutRun| {
        run.glyphs
            .first()
            .and_then(|g| run.text.get(g.start..g.end))
            .is_some_and(|s| s.starts_with('\u{A0}'))
    };
    let ends_with_placeholder = |run: &cosmic_text::LayoutRun| {
        run.glyphs
            .last()
            .and_then(|g| run.text.get(g.start..g.end))
            .is_some_and(|s| s.ends_with('\u{A0}'))
    };
    for (i, run) in runs.iter().enumerate() {
        let wrapped_start = if include_hard {
            i > 0
        } else {
            i > 0 && runs[i - 1].line_i == run.line_i
        };
        let wrapped_end = if include_hard {
            i + 1 < runs.len()
        } else {
            runs.get(i + 1).is_some_and(|next| next.line_i == run.line_i)
        };
        // 胶水保护的两种形态：
        // 1) 本 run 以 WJ 开头→断行点在 WJ 之前（cosmic 正确遵守 WJ 规则）
        // 2) 前一 run 以 WJ 结尾→断行点在 WJ 之后（cosmic 违反 WJ，但 WJ 已阻止
        //    在禁则字符之前断行）
        // 两种情况都不应判为禁则违规。
        let prev_tail_is_wj = i > 0 && run_tail(&runs[i - 1]) == Some(WJ);
        let next_head_is_wj = runs
            .get(i + 1)
            .is_some_and(|next| run_head(next) == Some(WJ));
        let skip_first = run_head(run) == Some(WJ)
            || prev_tail_is_wj
            || starts_with_placeholder(run)
            || (i > 0 && ends_with_placeholder(&runs[i - 1]));
        let skip_last = run_tail(run) == Some(WJ) || next_head_is_wj;
        let first = run
            .glyphs
            .iter()
            .find_map(|g| run.text.get(g.start..g.end)?.chars().find(|&c| visible(c)));
        let last = run
            .glyphs
            .iter()
            .rev()
            .find_map(|g| run.text.get(g.start..g.end)?.chars().rev().find(|&c| visible(c)));
        if let Some(c) = first {
            if wrapped_start && !skip_first && linebreak::forbidden_line_start(c) {
                eprintln!(
                    "[break-probe] 行首禁则违规 line={} char={c:?} text={:?}",
                    run.line_i, run.text
                );
            }
        }
        if let Some(c) = last {
            if wrapped_end && !skip_last && linebreak::forbidden_line_end(c) {
                eprintln!(
                    "[break-probe] 行尾禁则违规 line={} char={c:?} text={:?}",
                    run.line_i, run.text
                );
            }
        }
    }
}

/// 构造排版文本与坐标映射：行尾 `\r`、非激活节点标记、块级替换统一进替换表。
/// 返回 (排版文本, OffsetMap, 每逻辑行布局起始偏移, 源换行符的块相对偏移)。
/// 调用方已剥唯一尾 `\n`。
fn build_layout_text(
    body: &str,
    block: Range,
    inlines: &[InlineNode],
    caret: usize,
    subs: &[Sub],
    apply_cjk: bool,
    reveal_all: bool,
) -> (String, OffsetMap, Vec<usize>, Vec<usize>) {
    let mut all: Vec<Sub> = Vec::new();
    // 行尾 \r：仅剥「后继为 \n 或文末」的 \r（行内孤 \r 保持原样，与切片 1 行为一致）
    let bytes = body.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'\r' && (i + 1 == bytes.len() || bytes[i + 1] == b'\n') {
            all.push(Sub::elide(i, i + 1));
        }
    }
    let mut elide: Vec<(usize, usize)> = Vec::new();
    if !reveal_all {
        collect_elide(inlines, block, caret, &mut elide);
    }
    all.extend(elide.into_iter().map(|(s, e)| Sub::elide(s, e)));
    all.extend_from_slice(subs);
    if apply_cjk {
        all.extend(build_typo_subs(body));
    }
    let (map, accepted) = OffsetMap::from_spans(body.len(), &all);
    // 保留段取源文本并记行基；替换段写 fill（调用方保证不含换行）
    let mut layout_text = String::with_capacity(map.layout_len);
    let mut line_bases = vec![0];
    let mut source_newlines: Vec<usize> = Vec::new();
    for seg in &map.segs {
        match seg.sub {
            Some(i) => layout_text.push_str(&accepted[i].fill),
            None => {
                let seg_text = &body[seg.doc_start..seg.doc_start + seg.doc_len];
                for (j, b) in seg_text.bytes().enumerate() {
                    if b == b'\n' {
                        line_bases.push(seg.layout_start + j + 1);
                        source_newlines.push(seg.doc_start + j);
                    }
                }
                layout_text.push_str(seg_text);
            }
        }
    }
    (layout_text, map, line_bases, source_newlines)
}

/// 先序收集全部行内节点区间（doc 绝对坐标；调用方随即转块相对）：激活指纹按固定顺序折叠。
fn collect_node_ranges(nodes: &[InlineNode], out: &mut Vec<Range>) {
    for n in nodes {
        out.push(n.range);
        collect_node_ranges(&n.children, out);
    }
}

/// 块级替换（切片 12）：任务列表复选框 `☐`/`☑`、无序列表符号 `•`、
/// 引用行首 `>` → 空格、分隔线整行消隐；标题 `#` 与围栏 ` ``` ` 同规则消隐（B7）。
/// 激活判据沿用块区间闭区间；引用块逐行判定（光标所在行显源码，其余行隐标记）。
fn block_subs(kind: BlockKind, body: &str, block: Range, caret: usize) -> Vec<Sub> {
    match kind {
        BlockKind::ListItem {
            task: Some(done), ..
        } => {
            if node_active(block, caret) {
                return Vec::new();
            }
            // 列表标记 + `[ ]`（含尾空格）整体折叠为 ☐/☑（与 `•` 字形替换同机制）
            match task_span(body) {
                Some((start, _, end, _)) => {
                    vec![Sub::fill(start, end, if done { "☑ " } else { "☐ " })]
                }
                None => Vec::new(),
            }
        }
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
        // 标题/围栏：非激活时隐源码标记（与列表/引用/分隔线同一 WYSIWYM 规则）。
        BlockKind::AtxHeading(_) => heading_subs(body, block, caret),
        BlockKind::SetextHeading(_) => setext_subs(body, block, caret),
        BlockKind::FencedCode(_) => fenced_subs(body, block, caret),
        _ => Vec::new(),
    }
}

/// ATX 标题：非激活时消隐缩进(≤3) + `#`×1..=6 + 其后空白。
/// 行内文本与行高不变（只去掉前缀字形），光标进入块即还原源码。
fn heading_subs(body: &str, block: Range, caret: usize) -> Vec<Sub> {
    if node_active(block, caret) {
        return Vec::new();
    }
    atx_marker_end(body)
        .filter(|&end| end > 0)
        .map(|end| vec![Sub::elide(0, end)])
        .unwrap_or_default()
}

/// Setext 标题：非激活时把下划线行（`===` / `---`）换成等长空格。
/// 不用消隐：末行纯空时排版会丢行高（光标进出会跳），空格占位保持行高与行宽。
fn setext_subs(body: &str, block: Range, caret: usize) -> Vec<Sub> {
    if node_active(block, caret) {
        return Vec::new();
    }
    body.find('\n')
        .map(|nl| nl + 1)
        .filter(|&start| start < body.len())
        .map(|start| {
            let n = body[start..].chars().count().max(1);
            vec![Sub::fill(start, body.len(), " ".repeat(n))]
        })
        .unwrap_or_default()
}

/// ATX 标记结束位置（块相对）：CommonMark 允许 ≤3 空格缩进、`#` 后接空白或行尾。
fn atx_marker_end(body: &str) -> Option<usize> {
    let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
    if indent > 3 {
        return None;
    }
    let bytes = body.as_bytes();
    let hashes = bytes[indent..].iter().take_while(|&&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let after = indent + hashes;
    if after < bytes.len() && !matches!(bytes[after], b' ' | b'\t') {
        return None;
    }
    let mut end = after;
    while end < bytes.len() && matches!(bytes[end], b' ' | b'\t') {
        end += 1;
    }
    Some(end)
}

/// 围栏代码：非激活时消隐开围栏行（含语言标记）与闭合围栏行，保留换行（行高不变）。
/// 未闭合围栏只有开围栏行需要消隐。
fn fenced_subs(body: &str, block: Range, caret: usize) -> Vec<Sub> {
    if node_active(block, caret) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let first_end = body.find('\n').unwrap_or(body.len());
    if first_end > 0 {
        out.push(Sub::elide(0, first_end));
    }
    if let Some(nl) = body.rfind('\n') {
        let last_start = nl + 1;
        if last_start > first_end && fence_line(&body[last_start..]) {
            // 闭合围栏在末行：消隐会让纯空末行丢行高，用等长空格占位
            let n = body[last_start..].chars().count().max(1);
            out.push(Sub::fill(last_start, body.len(), " ".repeat(n)));
        }
    }
    out
}

/// 是否围栏行：≤3 空格缩进后至少 3 个同种 ``` 或 ~~~。
fn fence_line(line: &str) -> bool {
    let rest = line.trim_start_matches([' ', '\t']);
    if line.len() - rest.len() > 3 {
        return false;
    }
    let Some(&ch) = rest.as_bytes().first() else {
        return false;
    };
    (ch == b'`' || ch == b'~')
        && rest.as_bytes().iter().take_while(|&&b| b == ch).count() >= 3
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

/// 单元格富文本样式段：剥标记坐标 → 可见文本坐标，致密覆盖 `[0, text_len)`。
/// `text_len` 与 `OffsetMap` 结果不一致（如单元格内 `\|` 转义改写字节）时返回 None，
/// 调用方降级为纯文本。
pub(crate) fn cell_style_spans(
    cell_range: Range,
    text_len: usize,
    inlines: &[InlineNode],
    _link_color: [u8; 3],
) -> Option<Vec<(usize, usize, Style)>> {
    let mut elide: Vec<(usize, usize)> = Vec::new();
    collect_elide(inlines, cell_range, usize::MAX, &mut elide);
    let subs: Vec<Sub> = elide.into_iter().map(|(s, e)| Sub::elide(s, e)).collect();
    let (map, _) = OffsetMap::from_spans(cell_range.len(), &subs);
    if map.layout_len != text_len {
        return None;
    }
    let mut out: Vec<(usize, usize, Style)> = Vec::new();
    let mut cursor = 0usize;
    for (s, e, st) in style_spans(cell_range.len(), cell_range, inlines) {
        let ls = map.doc_to_layout(s).min(text_len);
        let le = map.doc_to_layout(e).min(text_len);
        if le <= cursor {
            continue;
        }
        let ls = ls.max(cursor);
        if ls > cursor {
            out.push((cursor, ls, Style::default()));
        }
        match out.last_mut() {
            Some(last) if last.1 == ls && last.2 == st => last.1 = le,
            _ => out.push((ls, le, st)),
        }
        cursor = le;
    }
    if cursor < text_len {
        out.push((cursor, text_len, Style::default()));
    }
    Some(out)
}

pub(super) fn attrs_of(kind: BlockKind, st: Style, link_color: [u8; 3]) -> Attrs<'static> {
    let a = Attrs::new();
    let a = if st.bold
        || matches!(
            kind,
            BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_)
        )
    {
        a.weight(Weight::BOLD)
    } else {
        a
    };
    let a = if st.italic {
        a.style(FontStyle::Italic)
    } else {
        a
    };
    let a = if st.mono
        || matches!(
            kind,
            BlockKind::FencedCode(_) | BlockKind::IndentedCode | BlockKind::HtmlBlock
        )
    {
        a.family(Family::Monospace)
    } else {
        a
    };
    // 显式语法高亮色优先；链接在无色时才回落到主题链接色。
    // 语法高亮只作用于代码块（style_spans 不产出），二者不会同时命中。
    let color = st.color.or_else(|| st.link.then_some(link_color));
    let a = if let Some([r, g, b]) = color {
        a.color(Color::rgb(r, g, b))
    } else {
        a
    };
    a
}

#[cfg(test)]
mod tests {
    use super::linebreak::{LINE_END_FORBIDDEN, LINE_START_FORBIDDEN};
    use super::*;
    use crate::theme::Theme;
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
        let (id, kind, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap();
        let mut eng = LayoutEngine::new();
        let block_text = &text[range.start..range.end];
        let l = eng.layout_block(kind, block_text, range, &inlines, width, caret, imgs, false);
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
    fn nested_emphasis_reveals_only_innermost_markers() {
        // D12-b 真机形态：`**Chi*n*ese**` 光标在 `n` 上时，`**` 不得同时显形
        let (_, l) = layout_with_caret("**Chi*n*ese**", 6, 400.0);
        assert_eq!(l.layout_text(), "Chi*n*ese");
        // 光标在粗体正文（内层之外）→ 只显外层标记
        let (_, l) = layout_with_caret("**Chi*n*ese**", 3, 400.0);
        assert_eq!(l.layout_text(), "**Chinese**");
        // 光标在块外 → 全部消隐
        let (_, l) = layout_with_caret("**Chi*n*ese**", usize::MAX, 400.0);
        assert_eq!(l.layout_text(), "Chinese");
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
    fn link_span_attrs_follow_theme_link_color() {
        let src = "see [官网](https://example.com) now";
        let s = Session::open(src);
        let (id, kind, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap();
        let spans = style_spans(range.len(), range, &inlines);
        let style_at = |at: usize| {
            spans
                .iter()
                .find(|(s, e, _)| *s <= at && at < *e)
                .map(|(_, _, st)| *st)
                .expect("style_spans 必须致密覆盖")
        };
        let link_at = src.find('官').unwrap();
        let body_at = 0; // 「see」中的 s
        assert!(style_at(link_at).link && !style_at(body_at).link);
        for theme in [Theme::light(), Theme::dark()] {
            let want = Color::rgb(theme.link[0], theme.link[1], theme.link[2]);
            let link_attrs = attrs_of(kind.clone(), style_at(link_at), theme.link);
            assert_eq!(
                link_attrs.color_opt,
                Some(want),
                "链接字符应使用主题链接色: dark={}",
                theme.dark
            );
            let body_attrs = attrs_of(kind.clone(), style_at(body_at), theme.link);
            assert_eq!(body_attrs.color_opt, None, "正文不得使用链接色");
            assert_ne!(theme.link, theme.fg, "链接色与正文前景色必须可区分");
        }
        // 活动链接显示源码，样式标记不变（仍由 link 色上色）
        let (_, shown) = layout_with_caret(src, link_at, 400.0);
        assert!(shown.layout_text().contains("https://example.com"));
    }

    #[test]
    fn engine_link_color_defaults_light_and_follows_setter() {
        let mut eng = LayoutEngine::new();
        assert_eq!(eng.link_color, Theme::light().link);
        eng.set_link_color(Theme::dark().link);
        assert_eq!(eng.link_color, Theme::dark().link);
    }

    #[test]
    fn reveal_nested_nodes_reveal_independently() {
        // 「**a *b* c**」：caret=3 在外层内容、内层（[4,7)）外 → 仅外层显标记
        let (_, l) = layout_with_caret("**a *b* c**", 3, 300.0);
        assert_eq!(l.layout_text(), "**a b c**");
        // caret=5 进内层：深层优先，只显内层 `*b*`，外层 `**` 保持消隐（D12-b）
        let (_, l) = layout_with_caret("**a *b* c**", 5, 300.0);
        assert_eq!(l.layout_text(), "a *b* c");
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

    // ---- B7：标题/围栏源码标记与列表/引用同一显隐规则（非激活消隐） ----

    #[test]
    fn atx_heading_marker_hidden_when_inactive_and_height_stable() {
        let (_, hidden) = layout_with_caret("## title", usize::MAX, 400.0);
        assert_eq!(hidden.layout_text(), "title", "非激活：`## ` 消隐");
        let (_, shown) = layout_with_caret("## title", 3, 400.0);
        assert_eq!(shown.layout_text(), "## title", "光标进入块：还原源码");
        assert!(
            (hidden.height - shown.height).abs() < 0.01,
            "显隐切换不得改变行高: {} vs {}",
            hidden.height,
            shown.height
        );
        // 缩进 ≤3 的标题同样消隐；`#` 后无空白不算标题标记（内核不会给出该 kind）
        let (_, l) = layout_with_caret("   ###  x", usize::MAX, 400.0);
        assert_eq!(l.layout_text(), "x");
    }

    #[test]
    fn setext_heading_underline_hidden_when_inactive() {
        let (_, hidden) = layout_with_caret("Title\n===", usize::MAX, 400.0);
        assert_eq!(
            hidden.layout_text().trim_end(),
            "Title",
            "下划线行字符替换为空格（保留行高）"
        );
        let (_, shown) = layout_with_caret("Title\n===", 0, 400.0);
        assert_eq!(shown.layout_text(), "Title\n===");
        assert!(
            (hidden.height - shown.height).abs() < 0.01,
            "下划线消隐后空行仍占高"
        );
    }

    #[test]
    fn fenced_code_fences_hidden_when_inactive_revealed_inside() {
        let src = "```rust\nlet x = 1;\n```";
        let (_, hidden) = layout_with_caret(src, usize::MAX, 400.0);
        assert_eq!(
            hidden.layout_text().trim_end(),
            "\nlet x = 1;",
            "围栏行不显示、内容保留（闭合行空格占位保行高）"
        );
        assert!(!hidden.layout_text().contains("```"));
        // 光标在代码内容内 → 整块激活，围栏还原
        let (_, shown) = layout_with_caret(src, 10, 400.0);
        assert_eq!(shown.layout_text(), src);
        assert!(
            (hidden.height - shown.height).abs() < 0.01,
            "围栏消隐不得塌缩行数: {} vs {}",
            hidden.height,
            shown.height
        );
        // 未闭合围栏：只隐开围栏行，末行内容不能被误删
        let (_, open) = layout_with_caret("```\na\nb", usize::MAX, 400.0);
        assert_eq!(open.layout_text(), "\na\nb");
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

    // ---- 代码块语法高亮（P3-1） ----

    #[test]
    fn fenced_code_with_lang_layouts_without_panic() {
        let src = "```rust\nlet x = 1;\nfn f() {}\n```";
        let (_eng, l) = layout_of(src, 400.0);
        assert!(l.height > BASE_LINE_HEIGHT * 2.0, "代码块应有多行高度");
    }

    #[test]
    fn fenced_code_without_lang_uses_default_style() {
        let src = "```\nlet x = 1;\n```";
        let (_eng, l) = layout_of(src, 400.0);
        assert!(l.height > BASE_LINE_HEIGHT);
    }

    // ---- 删除线（P3-4） ----

    #[test]
    fn strikethrough_span_recorded_whole_node() {
        // 样式恒按整节点（含标记）记录；paint 层经 OffsetMap 映射到可视文本
        let (_eng, l) = layout_of("a ~~del~~ c", 400.0);
        assert_eq!(l.strikes, vec![(2, 9)]);
    }

    #[test]
    fn strikethrough_markers_hidden_when_inactive() {
        // 非激活（无光标）时标记消隐：删除线矩形宽 ≈ 可视文本 "del" 宽
        let (eng, l) = layout_of("a ~~del~~ c", 400.0);
        let rects = eng.selection_rects(&l, 2, 9);
        assert_eq!(rects.len(), 1, "单行删除线");
        let (sx, _, sw, _) = rects[0];
        let (x_del, _, _) = eng.caret_xy(&l, 4);
        let (x_end, _, _) = eng.caret_xy(&l, 7);
        assert!((sx - x_del).abs() < 1.0, "矩形起点 = 可视文本起点");
        assert!((sw - (x_end - x_del)).abs() < 1.0, "矩形宽 = 可视文本宽");
    }

    #[test]
    fn strikethrough_absent_without_syntax() {
        let (_eng, l) = layout_of("a ~ b", 400.0);
        assert!(l.strikes.is_empty());
    }

    // ---- 任务列表复选框（P3-5） ----

    #[test]
    fn task_item_checkbox_substitution() {
        // 非激活：`- [ ] foo` → `☐ foo`；勾选 → `☑`
        let (_eng, l) = layout_of("- [ ] foo", 400.0);
        assert_eq!(l.layout_text(), "☐ foo");
        let (_eng, l) = layout_of("- [x] bar", 400.0);
        assert_eq!(l.layout_text(), "☑ bar");
        // 有序列表任务同规则
        let (_eng, l) = layout_of("1. [x] done", 400.0);
        assert_eq!(l.layout_text(), "☑ done");
    }

    #[test]
    fn task_item_active_shows_source() {
        // 光标进入块内：显源码 `- [ ] foo`（与列表符号/引用的激活行为一致）
        let (_eng, l) = layout_with_caret("- [ ] foo", 4, 400.0);
        assert_eq!(l.layout_text(), "- [ ] foo");
    }

    #[test]
    fn plain_list_item_unaffected_by_task_rules() {
        let (_eng, l) = layout_of("- [ ]a", 400.0); // 后无空白：非任务 → `• [ ]a`
        assert_eq!(l.layout_text(), "• [ ]a");
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

    // ---- 中西文自动间距（v0 切片） ----

    #[test]
    fn cjk_to_ascii_adds_space() {
        // 「中文text」→ 「中文 text」
        let (_, l) = layout_of("中文text", 400.0);
        assert_eq!(l.layout_text(), "中文 text");
    }

    #[test]
    fn ascii_to_cjk_adds_space() {
        // 「text中文」→ 「text 中文」
        let (_, l) = layout_of("text中文", 400.0);
        assert_eq!(l.layout_text(), "text 中文");
    }

    #[test]
    fn both_boundaries_add_spaces() {
        // 「中text文」→ 「中 text 文」
        let (_, l) = layout_of("中text文", 400.0);
        assert_eq!(l.layout_text(), "中 text 文");
    }

    #[test]
    fn no_double_space_when_already_whitespace() {
        // 已有空白时不再加（边界跳过）
        let (_, l) = layout_of("中 text 文", 400.0);
        assert_eq!(l.layout_text(), "中 text 文");
    }

    #[test]
    fn fenced_code_skips_cjk_spacing() {
        // 代码块里的 CJK↔ASCII 不插入空格（layout_text 含围栏消隐痕迹，但不含「中 text」）
        let (_, l) = layout_of("```\n中text\n```\n", 400.0);
        assert!(
            !l.layout_text().contains("中 text"),
            "代码块不应加 CJK 间距: {:?}",
            l.layout_text()
        );
    }

    #[test]
    fn hit_roundtrip_works_with_cjk_spacing() {
        // 插入空格后 OffsetMap 仍正确：doc 偏移 3（中结束、t 开始）→ layout 应落到空格之后
        let (eng, l) = layout_of("中t", 400.0);
        // doc 文本：中=0..3, t=3..4；layout_text 应为 "中 t"
        let doc_t = 3; // 文档中 t 的位置
        let (x_before, _, _) = eng.caret_xy(&l, 0); // 中之前
        let (x_after, _, _) = eng.caret_xy(&l, doc_t); // t 之前（layout 里空格之后）
        assert!(x_after > x_before, "CJK 间距后光标 x 应递增");
        // hit 逆映射：layout 末尾 → 文档末尾
        let (x_last, y, h) = eng.caret_xy(&l, 4);
        assert_eq!(eng.hit(&l, x_last + 0.5, y + h / 2.0), 4);
    }

    #[test]
    fn is_cjk_classifies_correctly() {
        assert!(is_cjk('中'));
        assert!(is_cjk('あ'));
        assert!(is_cjk('한'));
        assert!(!is_cjk('A'));
        assert!(!is_cjk('1'));
        assert!(!is_cjk('.'));
    }

    /// 每条视觉行的（首簇字符, 尾簇字符）：禁则测试的观测量。
    fn visual_line_edges(l: &BlockLayout) -> Vec<(char, char)> {
        l.buffer
            .layout_runs()
            .filter_map(|r| {
                let f = r.glyphs.first()?;
                let g = r.glyphs.last()?;
                Some((
                    r.text[f.start..f.end].chars().next()?,
                    r.text[g.start..g.end].chars().next()?,
                ))
            })
            .collect()
    }

    /// 排版单行文本的实测行宽（禁则/悬挂测试的版心宽度来源）。
    fn single_line_width(eng: &mut LayoutEngine, text: &str) -> f32 {
        let block = Range {
            start: 0,
            end: text.len(),
        };
        let l = eng.layout_block(
            BlockKind::Paragraph,
            text,
            block,
            &[],
            4000.0,
            usize::MAX,
            &[],
            false,
        );
        l.buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0, f32::max)
    }

    fn layout_paragraph(eng: &mut LayoutEngine, text: &str, width: f32) -> BlockLayout {
        let block = Range {
            start: 0,
            end: text.len(),
        };
        eng.layout_block(
            BlockKind::Paragraph,
            text,
            block,
            &[],
            width,
            usize::MAX,
            &[],
            false,
        )
    }

    #[test]
    fn kinsoku_forbidden_chars_never_start_a_visual_line() {
        let mut eng = LayoutEngine::new();
        let prefix: String = "中".repeat(10);
        let w10 = single_line_width(&mut eng, &prefix);
        for &p in LINE_START_FORBIDDEN {
            let text = format!("{prefix}{p}{}{}", "中".repeat(4), "文".repeat(8));
            for delta in [-4.0, 0.0, 4.0] {
                // 版心恰好容纳 10 字：无禁则时第 11 位标点必顶到下一行行首
                let l = layout_paragraph(&mut eng, &text, w10 + 1.0 + delta);
                let edges = visual_line_edges(&l);
                assert!(edges.len() >= 2, "{p:?} 处应折行: {edges:?}");
                for (fc, _) in &edges {
                    assert!(
                        !LINE_START_FORBIDDEN.contains(fc),
                        "行首禁则违反: {fc:?} 出现在行首（测试标点 {p:?}，宽 {delta:+.1}）"
                    );
                }
                for (_, lc) in &edges {
                    assert!(
                        !LINE_END_FORBIDDEN.contains(lc),
                        "行尾禁则违反: {lc:?} 出现在行尾（测试标点 {p:?}，宽 {delta:+.1}）"
                    );
                }
            }
        }
    }

    #[test]
    fn kinsoku_open_brackets_never_end_a_visual_line() {
        let mut eng = LayoutEngine::new();
        let w10 = single_line_width(&mut eng, &"中".repeat(10));
        // 括号在第 11 位（自然折行点）与第 10 位（无禁则时恰好滞留行尾）都要守住
        for prefix_len in [10usize, 9] {
            let prefix: String = "中".repeat(prefix_len);
            for &p in LINE_END_FORBIDDEN {
                let text = format!("{prefix}{p}{}{}", "中".repeat(4), "文".repeat(8));
                let l = layout_paragraph(&mut eng, &text, w10 + 1.0);
                let edges = visual_line_edges(&l);
                assert!(edges.len() >= 2, "{p:?} 处应折行: {edges:?}");
                for (_, lc) in &edges {
                    assert!(
                        !LINE_END_FORBIDDEN.contains(lc),
                        "行尾禁则违反: {lc:?} 出现在行尾（括号 {p:?}，prefix={prefix_len}）"
                    );
                }
                for (fc, _) in &edges {
                    assert!(
                        !LINE_START_FORBIDDEN.contains(fc),
                        "行首禁则违反: {fc:?} 出现在行首（括号 {p:?}，prefix={prefix_len}）"
                    );
                }
            }
        }
    }

    #[test]
    fn kinsoku_glue_keeps_ellipsis_attached_to_previous_char() {
        // 版心满到省略号前一字：…… 必须整体随前字下沉，不得出现在行首，
        // 也不得内部断开（WJ × WJ 链）
        let mut eng = LayoutEngine::new();
        let w10 = single_line_width(&mut eng, &"中".repeat(10));
        let text = format!("{}……{}", "中".repeat(10), "文".repeat(6));
        let l = layout_paragraph(&mut eng, &text, w10 + 1.0);
        let edges = visual_line_edges(&l);
        assert!(edges.len() >= 2, "应折行: {edges:?}");
        assert!(
            edges.iter().all(|(fc, _)| *fc != '…'),
            "…… 不得在行首（内部断开必致后半在行首）: {edges:?}"
        );
    }

    #[test]
    fn hanging_punctuation_shifts_full_line_end_glyph() {
        let mut eng = LayoutEngine::new();
        let w10 = single_line_width(&mut eng, &"中".repeat(10));
        let width = w10 + 1.0;
        // 9 字 + 句号恰满一行：末字应悬挂
        let full = format!("{}。{}", "中".repeat(9), "文".repeat(6));
        let l = layout_paragraph(&mut eng, &full, width);
        let runs: Vec<_> = l.buffer.layout_runs().collect();
        assert!(runs.len() >= 2, "满行句号文本应折行");
        let (dx, hi) = hang_shift(&runs[0], Some(width));
        assert!(dx > 0.0 && hi.is_some(), "满行末字句号应悬挂: dx={dx}");
        // 非满行（行尾同句号但行宽差很多）不悬挂
        let short = format!("{}。{}", "中".repeat(5), "文".repeat(2));
        let l2 = layout_paragraph(&mut eng, &short, width);
        let run2 = l2.buffer.layout_runs().next().unwrap();
        assert_eq!(
            hang_shift(&run2, Some(width)),
            (0.0, None),
            "非满行不悬挂"
        );
        // 末字非闭标点不悬挂
        let plain = "中".repeat(12);
        let l3 = layout_paragraph(&mut eng, &plain, width);
        let run3 = l3.buffer.layout_runs().next().unwrap();
        assert_eq!(
            hang_shift(&run3, Some(width)),
            (0.0, None),
            "末字非闭标点不悬挂"
        );
    }

    #[test]
    fn hanging_punctuation_widens_raster_beyond_text_edge() {
        let mut eng = LayoutEngine::new();
        let w10 = single_line_width(&mut eng, &"中".repeat(10));
        let hung = format!("{}。{}", "中".repeat(9), "文".repeat(6));
        let l = layout_paragraph(&mut eng, &hung, w10 + 1.0);
        let (_px, w_hung, _h) = eng.rasterize(&l, [0, 0, 0]);
        assert!(
            w_hung as f32 > w10,
            "悬挂后位图应宽于 10 字版心: w_hung={w_hung} w10={w10}"
        );
    }

    #[test]
    fn preedit_applies_body_typography_rules() {
        let mut eng = LayoutEngine::new();
        // 中西文间距：与正文同规则注入
        let l = eng.layout_preedit("ni好a");
        assert_eq!(l.layout_text(), "ni 好 a");
        // 组合串内光标（IME 报告原串字节偏移）经 OffsetMap 映射后单调
        let (x_hao, _, _) = eng.caret_xy(&l, 2); // 「好」前
        let (x_a, _, _) = eng.caret_xy(&l, 5); // 「a」前
        assert!(x_a > x_hao, "preedit 光标映射应单调: {x_hao} -> {x_a}");
        // kinsoku 胶水：省略号 run 注入 WJ
        let l2 = eng.layout_preedit("中……");
        assert!(
            l2.layout_text().contains('\u{2060}'),
            "胶水段应注入 WJ: {:?}",
            l2.layout_text()
        );
    }

    #[test]
    fn word_joiner_adds_no_advance() {
        // tofu 守卫：fallback 字体链中 WJ 必须近零宽（实测 1.29px 零墨字形）。
        // 若某天字体链退化把 WJ 渲染成缺字形盒（advance ≥ 半字宽），此测试先红。
        let mut eng = LayoutEngine::new();
        let plain = single_line_width(&mut eng, "中A");
        let glued = single_line_width(&mut eng, "中\u{2060}A");
        assert!(
            (glued - plain).abs() < 2.0,
            "WJ advance 不可见级（<2px）: plain={plain} glued={glued}"
        );
    }

    // ---- 表格单元格富文本（B3）：剥标记坐标 → 可见文本坐标 ----

    type CellSpans = Option<Vec<(usize, usize, Style)>>;

    /// 打开单行表格，取第 row 行第 col 个单元格的样式段与可见文本长。
    fn cell_spans(doc: &str, row: usize, col: usize) -> (CellSpans, usize) {
        let s = Session::open(doc);
        let (id, _, _) = s.blocks()[0].clone();
        let t = s.table(id).unwrap();
        let cell = if row == 0 {
            &t.header[col]
        } else {
            &t.rows[row - 1][col]
        };
        let spans = cell_style_spans(cell.range, cell.text.len(), &cell.inlines, [0, 0, 0]);
        (spans, cell.text.len())
    }

    fn at(spans: &[(usize, usize, Style)], p: usize) -> Style {
        spans
            .iter()
            .find(|(s, e, _)| *s <= p && p < *e)
            .map(|x| x.2)
            .unwrap_or_default()
    }

    #[test]
    fn cell_spans_map_markers_to_visible_text() {
        let doc = "| **粗** | *斜* | `code` | [t](http://a) |\n| --- | --- | --- | --- |\n";
        let (bold, n) = cell_spans(doc, 0, 0);
        let bold = bold.expect("粗体应可映射");
        assert_eq!(n, "粗".len());
        assert_eq!(bold, vec![(0, n, Style { bold: true, ..Default::default() })]);

        let (it, n) = cell_spans(doc, 0, 1);
        let it = it.expect("斜体应可映射");
        assert_eq!(it, vec![(0, n, Style { italic: true, ..Default::default() })]);

        let (mono, n) = cell_spans(doc, 0, 2);
        let mono = mono.expect("行内代码应可映射");
        assert_eq!(mono, vec![(0, n, Style { mono: true, ..Default::default() })]);

        let (link, n) = cell_spans(doc, 0, 3);
        let link = link.expect("链接应可映射");
        assert_eq!(n, 1);
        assert!(link.iter().all(|(s, e, _)| (*s, *e) == (0, 1)) && at(&link, 0).link);
    }

    #[test]
    fn cell_spans_cover_visible_text_densely() {
        let docs = [
            "| 中文 **粗** 混排 |\n| --- |\n",
            "| a *b* `c` [d](u) |\n| --- |\n",
            "|  |\n| --- |\n",
        ];
        for doc in docs {
            let (spans, n) = cell_spans(doc, 0, 0);
            let spans = spans.expect("应可映射");
            let mut pos = 0;
            for &(s, e, _) in &spans {
                assert_eq!(s, pos, "致密覆盖不得有缝: {doc:?}");
                assert!(s < e);
                pos = e;
            }
            assert_eq!(pos, n, "覆盖须恰好等于可见文本长: {doc:?}");
        }
    }

    #[test]
    fn cell_spans_none_when_bytes_diverge() {
        // `\|` 在 core 侧被解转义为单个 `|`，与剥标记长度不一致 → 降级信号
        let (spans, n) = cell_spans("| a\\|b |\n| --- |\n", 0, 0);
        assert!(spans.is_none(), "转义改写字节时应返回 None");
        assert_eq!(n, "a|b".len());
    }

    // ---- 脚注（渲染态）：引用 `[^id]` → `[n]`，定义块前缀/返回标记 ----

    /// 打开文档，取第 `idx` 个块，带全文编号表排版（caret 为文档绝对偏移）。
    fn layout_footnote_block(
        text: &str,
        idx: usize,
        caret: usize,
        width: f32,
        reveal_all: bool,
    ) -> (LayoutEngine, BlockLayout, Range) {
        let s = Session::open(text);
        let (id, kind, range) = s.blocks()[idx].clone();
        let inlines = s.block_inlines(id).unwrap_or_default();
        let ctx = FootnoteCtx::new(s.footnote_ids(), Theme::light().muted);
        let mut eng = LayoutEngine::new();
        let l = eng.layout_block_with_footnotes(
            kind,
            &text[range.start..range.end],
            range,
            &inlines,
            width,
            caret,
            &[],
            reveal_all,
            Some(&ctx),
        );
        (eng, l, range)
    }

    fn first_footnote_ref(nodes: &[InlineNode]) -> Option<&InlineNode> {
        for n in nodes {
            if n.kind == InlineKind::FootnoteRef {
                return Some(n);
            }
            if let Some(f) = first_footnote_ref(&n.children) {
                return Some(f);
            }
        }
        None
    }

    #[test]
    fn footnote_ref_renders_number_with_link_color() {
        let src = "see [^a] now\n\n[^a]: note\n";
        let (_eng, l, _) = layout_footnote_block(src, 0, usize::MAX, 400.0, false);
        assert_eq!(
            l.layout_text(),
            "see [1] now",
            "引用应渲染为 [n]，不暴露 ^ 与 id 原文"
        );
        let link = Theme::light().link;
        let want = Color::rgb(link[0], link[1], link[2]);
        let colored = l
            .buffer
            .layout_runs()
            .flat_map(|r| r.glyphs.iter())
            .filter(|g| g.color_opt == Some(want))
            .count();
        assert!(colored >= 3, "`[1]` 字形应使用链接色: colored={colored}");
    }

    #[test]
    fn footnote_ref_without_number_keeps_source() {
        let src = "see [^a] now\n\n[^a]: note\n";
        let s = Session::open(src);
        let (id, kind, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap();
        let mut eng = LayoutEngine::new();
        // 无编号表（打印等降级路径）：不伪造编号，保留 `[^id]`。
        let l = eng.layout_block(
            kind,
            &src[range.start..range.end],
            range,
            &inlines,
            400.0,
            usize::MAX,
            &[],
            false,
        );
        assert_eq!(l.layout_text(), "see [^a] now");
        let (x, y, _) = eng.caret_xy(&l, 4);
        let off = eng.hit(&l, x + 0.5, y + 1.0);
        assert!((4..=8).contains(&off), "原文降级态点击不得串位: {off}");
    }

    #[test]
    fn footnote_ref_source_mode_shows_raw() {
        let src = "see [^a] now\n\n[^a]: note\n";
        let (_eng, l, _) = layout_footnote_block(src, 0, usize::MAX, 400.0, true);
        assert_eq!(l.layout_text(), "see [^a] now", "源码模式不受替换影响");
    }

    #[test]
    fn footnote_ref_hit_maps_into_source_range_after_edits() {
        // 引用前后插入/删除字符后重新排版：点击 `[1]` 任一位置都必须落在源码区间内。
        for src in [
            "a[^n]b\n\n[^n]: x\n",
            "aX[^n]b\n\n[^n]: x\n",
            "a[^n]bY\n\n[^n]: x\n",
        ] {
            let (eng, l, range) = layout_footnote_block(src, 0, usize::MAX, 400.0, false);
            let s = Session::open(src);
            let (id, _, _) = s.blocks()[0].clone();
            let inlines = s.block_inlines(id).unwrap();
            let node = first_footnote_ref(&inlines).expect("应有脚注引用节点");
            let (rs, re) = (
                node.range.start - range.start,
                node.range.end - range.start,
            );
            let (x0, y, _) = eng.caret_xy(&l, rs);
            let (x1, _, _) = eng.caret_xy(&l, re);
            assert!(x1 > x0, "替换段应有宽度: {src:?}");
            for t in [0.25, 0.75] {
                let off = eng.hit(&l, x0 + (x1 - x0) * t, y + 1.0);
                assert!(
                    (rs..=re).contains(&off),
                    "{src:?} 点击 `[1]` 应落在源码引用区间 [{rs},{re}]: {off}"
                );
            }
        }
    }

    #[test]
    fn footnote_def_hides_marker_and_appends_return_mark() {
        let src = "body[^a]\n\n[^a]: note\n";
        let (eng, l, _range) = layout_footnote_block(src, 1, usize::MAX, 400.0, false);
        assert_eq!(l.layout_text(), "1. note↩");
        // 点击 `1. ` 前缀 → 内容起点（`[^a]: ` 长 6）。
        let (x0, y, _) = eng.caret_xy(&l, 0);
        assert_eq!(eng.hit(&l, x0 + 1.0, y + 1.0), 6, "前缀应映射到内容起点");
        // 点击块末 `↩` → 块末。
        let body_len = "[^a]: note".len();
        let (xs, ys, _) = eng.caret_xy(&l, body_len);
        assert_eq!(
            eng.hit(&l, xs + 1.0, ys + 1.0),
            body_len,
            "返回标记应映射到块末"
        );
    }

    #[test]
    fn footnote_def_without_reference_hides_marker_without_number() {
        let src = "no refs\n\n[^b]: unused\n";
        let (_eng, l, _) = layout_footnote_block(src, 1, usize::MAX, 400.0, false);
        assert_eq!(l.layout_text(), "unused↩", "未编号定义不显示编号前缀");
    }

    #[test]
    fn footnote_def_reveals_source_when_caret_inside() {
        let src = "body[^a]\n\n[^a]: note\n";
        let s = Session::open(src);
        let (_, _, range) = s.blocks()[1].clone();
        let (_eng, l, _) =
            layout_footnote_block(src, 1, range.start + 2, 400.0, false);
        assert_eq!(l.layout_text(), "[^a]: note", "光标进入沿用 reveal 显源码");
    }

    #[test]
    fn footnote_def_number_prefix_survives_typo_spacing() {
        // `[^a]:中文`：中西文间距段与编号插入段同起点，插入段必须排在前，否则被吞。
        let src = "see [^a]\n\n[^a]:中文\n";
        let (_eng, l, _) = layout_footnote_block(src, 1, usize::MAX, 400.0, false);
        let t = l.layout_text();
        assert!(t.starts_with("1. "), "编号前缀不得被间距段挤掉: {t:?}");
        assert!(t.ends_with('↩'));
    }

    #[test]
    fn offset_map_insertion_at_end_keeps_suffix_out_of_caret() {
        let (m, _) = OffsetMap::from_spans(3, &[Sub::insert(3, "↩")]);
        assert_eq!(m.layout_len, 3 + "↩".len());
        assert_eq!(m.doc_to_layout(3), 3, "块末光标停在装饰之前");
        assert_eq!(m.layout_to_doc(3), 3, "点击装饰 → 块末");
        assert_eq!(m.layout_to_doc(m.layout_len), 3);
    }

    #[test]
    fn offset_map_insertion_after_elided_prefix_maps_to_content_start() {
        // "[^a]: x"：前缀 [0,6) 消隐，`1. ` 于 6 处插入；内容 "x" 仍在 6..7。
        let (m, _) = OffsetMap::from_spans(7, &[Sub::elide(0, 6), Sub::insert(6, "1. ")]);
        assert_eq!(m.layout_len, "1. x".len());
        assert_eq!(m.doc_to_layout(0), 0);
        assert_eq!(m.doc_to_layout(6), 3);
        assert_eq!(m.doc_to_layout(7), 4);
        assert_eq!(m.layout_to_doc(0), 6, "前缀任一点击 → 内容起点");
        assert_eq!(m.layout_to_doc(2), 6);
        assert_eq!(m.layout_to_doc(3), 6);
    }

    #[test]
    fn footnote_epoch_invalidates_tail_definition_layout() {
        use crate::view::cache::{Cache, LayoutKey, Memo, Revs};

        let before = "see [^x]\n\n[^x]: X\n\n[^y]: Y\n";
        let after = "[^y] see [^x]\n\n[^x]: X\n\n[^y]: Y\n";
        let revs = Revs::default();
        let mut eng = LayoutEngine::new();

        let s1 = Session::open(before);
        let (id1, kind1, range1) = s1.blocks()[2].clone();
        assert_eq!(kind1, BlockKind::FootnoteDef);
        let ctx1 = FootnoteCtx::new(s1.footnote_ids(), Theme::light().muted);
        let base1 = revs.base(800.0, 1.0, false, footnote_epoch(&s1.footnote_ids()));
        let inlines1 = s1.block_inlines(id1).unwrap_or_default();
        let l1 = eng.layout_block_with_footnotes(
            kind1.clone(),
            &before[range1.start..range1.end],
            range1,
            &inlines1,
            800.0,
            usize::MAX,
            &[],
            false,
            Some(&ctx1),
        );
        assert_eq!(l1.layout_text(), "Y↩", "未被引用：仅隐藏标记");
        let reveal1 = l1.reveal_state(&kind1, range1, usize::MAX);
        let mut cache = Cache::default();
        cache.layouts.insert(
            id1,
            Memo {
                key: LayoutKey {
                    base: base1,
                    reveal: reveal1,
                },
                value: l1,
            },
        );
        assert!(
            cache.layout(id1, &kind1, range1, base1, usize::MAX).is_some(),
            "代次未变必须命中"
        );

        let s2 = Session::open(after);
        let (id2, kind2, range2) = s2.blocks()[2].clone();
        let ctx2 = FootnoteCtx::new(s2.footnote_ids(), Theme::light().muted);
        let base2 = revs.base(800.0, 1.0, false, footnote_epoch(&s2.footnote_ids()));
        assert_ne!(base1, base2, "文首新增未出现的引用 → 全文编号变化");
        assert!(
            cache
                .layout(id2, &kind2, range2, base2, usize::MAX)
                .is_none(),
            "旧布局不得以新代次命中"
        );
        let inlines2 = s2.block_inlines(id2).unwrap_or_default();
        let l2 = eng.layout_block_with_footnotes(
            kind2,
            &after[range2.start..range2.end],
            range2,
            &inlines2,
            800.0,
            usize::MAX,
            &[],
            false,
            Some(&ctx2),
        );
        assert_eq!(l2.layout_text(), "1. Y↩", "新引用让文末定义块获得编号");
    }
}
