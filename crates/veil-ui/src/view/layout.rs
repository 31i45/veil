//! cosmic-text 排版引擎：块级布局、命中检测、光标坐标、光栅化。
//! 布局单位即物理像素——缩放因子由 shell 在 width/metrics 上乘入，本层无 DPI 概念。
//!
//! 消隐机制（统一）：行尾 `\r` 与非激活 inline 节点的标记段都不参与排版（`\r` 交给
//! cosmic 会渲染成缺字形框；标记消隐即 Reveal）。两类区间统一进消隐集合，由
//! `OffsetMap` 负责 doc↔layout 坐标换算——对外偏移一律为文档坐标。
//! 激活规则 v0：`range.start <= caret <= range.end` 闭区间，逐节点独立、嵌套各自生效。

use std::collections::{HashMap, HashSet};

use cosmic_text::{
    Attrs, Buffer, Color, Cursor, Family, FontSystem, Metrics, Shaping, Style as FontStyle,
    SwashCache, Weight, Wrap,
};
use veil_core::{task_span, BlockId, BlockKind, HtmlTag, InlineKind, InlineNode, Range};

use super::highlight::{to_styles, Highlighter};
use super::linebreak;
use super::style::{style_spans, Style};
use crate::geometry::{BASE_FONT_SIZE, BASE_LINE_HEIGHT};
use crate::i18n::{t, tf, Key};
use crate::theme::{DocFontFamily, SyntaxColors, Theme};
use crate::tokens::typography;

/// 行内资源键：占位图与位图缓存之间的寻址键（切片 8/13 共用占位机制）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InlineAsset {
    /// 行内公式：键 = TeX 源码（内容寻址缓存）。
    Math(String),
    /// 文档内图片：键 = 按文档目录解析后的绝对路径（自然尺寸缓存）。
    Image(String),
    /// Mermaid 块位图：键 = block_id.to_string()（通过 BlockId → MermaidImage 寻址）。
    Mermaid(String),
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
    /// 独占视觉行时的水平对齐（`<img align>`）；行内图片为 None（不打断行内流）。
    pub align: Option<veil_core::ImageAlign>,
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

/// 图片题注输入：仅**独立成行**的图片、且 alt 非空。
/// `text` 只用于视图显示（位于图片下方、占用独立一行），不参与文档字节与编辑映射。
pub struct ImageCaption {
    /// 图片节点区间（doc 绝对坐标），用于激活判定与「是否已出图」比对。
    pub range: Range,
    /// alt 原文（已把内部换行折成空格；其余字符不改写）。
    pub text: String,
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
    tf(
        Key::BlockOversizeNotice,
        &[&format!("{:.1}", full_len as f64 / (1024.0 * 1024.0))],
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

/// 图片显示尺寸（B4）：宽 ≤ `max_w`、高 ≤ `max_h`，等比缩放且绝不放大。
/// 至少 1px；两轴独立取整会有 ≤1px 的比例误差（与既有 `fit_size` 同口径）。
pub(crate) fn fit_size_box(w: u32, h: u32, max_w: f32, max_h: f32) -> (u32, u32) {
    let w = w.max(1) as f32;
    let h = h.max(1) as f32;
    let s = (max_w / w).min(max_h.max(1.0) / h).min(1.0);
    (
        (w * s).round().max(1.0) as u32,
        (h * s).round().max(1.0) as u32,
    )
}

/// 显式尺寸（HTML `<img width height>`）：给定轴直接用给定值，缺失轴按原图比例推算；
/// 与 `fit_size_box` 不同，这里允许放大（作者显式指定即应生效），只把两轴等比夹到
/// `max_w`/`max_h`（调用方传「版心宽 × 4」）——防止离谱属性值把版面撑到不可用。
pub(crate) fn explicit_size(
    w: u32,
    h: u32,
    want_w: Option<u32>,
    want_h: Option<u32>,
    max_w: f32,
    max_h: f32,
) -> (u32, u32) {
    let w = w.max(1) as f32;
    let h = h.max(1) as f32;
    let (dw, dh) = match (want_w, want_h) {
        (Some(ew), Some(eh)) => (ew.max(1) as f32, eh.max(1) as f32),
        (Some(ew), None) => {
            let ew = ew.max(1) as f32;
            (ew, (h * ew / w).round().max(1.0))
        }
        (None, Some(eh)) => {
            let eh = eh.max(1) as f32;
            ((w * eh / h).round().max(1.0), eh)
        }
        (None, None) => (w, h),
    };
    // 只缩到上限，不再往下缩：显式小图应保持显式尺寸。
    let s = (max_w.max(1.0) / dw).min(max_h.max(1.0) / dh).min(1.0);
    (
        (dw * s).round().max(1.0) as u32,
        (dh * s).round().max(1.0) as u32,
    )
}

/// 行内图片的最终显示尺寸：有显式宽/高时走 `explicit_size`（上限 = 版心宽 ×4）；
/// 否则按默认限宽限高（`fit_size_box`，仅缩小不放大）。
pub(crate) fn image_display_size(
    nat_w: u32,
    nat_h: u32,
    explicit: (Option<u32>, Option<u32>),
    max_w: f32,
    max_h: f32,
) -> (u32, u32) {
    match explicit {
        (None, None) => fit_size_box(nat_w, nat_h, max_w, max_h),
        _ => explicit_size(
            nat_w,
            nat_h,
            explicit.0,
            explicit.1,
            max_w * crate::tokens::doc::IMG_EXPLICIT_MAX_RATIO,
            max_w * crate::tokens::doc::IMG_EXPLICIT_MAX_RATIO,
        ),
    }
}

// 独占一行的图片判定与独立图片题注均由内核提供单一真值
// （`veil_core::{image_standalone, standalone_image_caption}`），
// 视图、HTML 导出与 PDF 导出共用同一语义，不再各自实现。

/// 替换段：块相对 doc 区间 `[doc_start, doc_end)` → `fill` 文本。
/// `fill` 为空即消隐；非空时整段替换（长度可同可异）。fill 可含换行（`<br>`、题注），
/// 行基对替换段同样扫描。行内公式占位（NBSP）与块级替换（列表/引用/分隔线）共用此结构。
#[derive(Clone, Debug)]
struct Sub {
    doc_start: usize,
    doc_end: usize,
    fill: String,
    /// 图片题注段：视图派生文本，构建排版文本时回报其 layout 区间用于上色。
    caption: bool,
}

impl Sub {
    fn elide(doc_start: usize, doc_end: usize) -> Self {
        Sub {
            doc_start,
            doc_end,
            fill: String::new(),
            caption: false,
        }
    }

    fn fill(doc_start: usize, doc_end: usize, text: impl Into<String>) -> Self {
        Sub {
            doc_start,
            doc_end,
            fill: text.into(),
            caption: false,
        }
    }

    /// 纯插入：零宽源码区间 → 有宽展示文本（脚注定义的编号前缀 / 块末 `↩`）。
    /// 插入段不消费源码字符，doc 坐标只作锚点。
    fn insert(doc: usize, text: impl Into<String>) -> Self {
        Sub {
            doc_start: doc,
            doc_end: doc,
            fill: text.into(),
            caption: false,
        }
    }

    /// 题注插入：零宽锚点 → 独立一行题注（fill 自带前后换行）。
    fn caption(doc: usize, text: impl Into<String>) -> Self {
        Sub {
            doc_start: doc,
            doc_end: doc,
            fill: text.into(),
            caption: true,
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
                    _ => b
                        .doc_end
                        .cmp(&a.doc_end)
                        .then_with(|| b.fill.len().cmp(&a.fill.len())),
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
                    caption: sub.caption,
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
                    caption: sub.caption,
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
    pub(crate) fn new(ids: &[String], muted: [u8; 3]) -> Self {
        let numbers = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i + 1))
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

/// 块级装饰段：`subs` 之外还记录装饰在 layout 中的宽度，
/// 供 `layout_block` 为「不来自源码」的前缀/后缀补齐样式 span。
#[derive(Default)]
struct BlockDecor {
    subs: Vec<Sub>,
    prefix_len: usize,
    suffix_len: usize,
}

/// 前置装饰（文本之下绘制）类别：mark 底色 / kbd 圆角边框 / 行内代码底色。
/// 装饰区间为块相对 doc 坐标（与 strikes 同源），paint 层经 selection_rects 取物理矩形。
/// 颜色在 paint 层按主题取（mark 浅黄、kbd 灰描边、code 主题 code_bg）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UnderKind {
    /// mark 底色（浅黄柔和色，按主题微调明度）。
    MarkBg,
    /// kbd 圆角边框（按键样式，1px 灰描边 + 浅灰底）。
    KbdFrame,
    /// 缺失/远程 `<img>` 的占位框（浅灰底 + 灰描边）。
    ImagePlaceholder,
    /// 行内代码底色（`` `x` `` / `<code>` / 行内 `<pre>`）：主题 code_bg + 轻微圆角。
    InlineCodeBg,
    /// `style="background-color:..."` 的自定义底色（取值由解析层校验）。
    StyleBg([u8; 3]),
}

/// Sub/Sup 基线偏移物理像素：±4 逻辑像素按 DPI 缩放（正数下移 = sub，负数上移 = sup）。
pub(crate) fn script_shift_px(shift: f32, scale: f32) -> f32 {
    shift * 4.0 * scale
}

/// 后置装饰（文本之上绘制）类别：abbr 虚线下划线。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OverKind {
    /// abbr 虚线下划线（提示可展开缩写）。
    DottedUnderline,
    /// `<u>` / `<ins>` / `text-decoration:underline` 的实线下划线。
    Underline,
    /// `text-decoration:overline` 的上划线。
    Overline,
}

/// 目录展示文本：用 core 的 [`veil_core::generate_toc`] 派生层级/锚点，再转成
/// 视图可见的列表行（`• 标题`，每级 3 空格缩进）。不参与文档字节与编辑映射。
pub(crate) fn toc_display_text(entries: &[(u8, String)]) -> String {
    let raw = veil_core::generate_toc(entries.iter().map(|(l, t)| (*l, t.as_str())));
    let mut out = String::new();
    for line in raw.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let item = line.trim_start();
        let label = item
            .strip_prefix("- [")
            .and_then(|rest| rest.rsplit_once("](#"))
            .map(|(label, _)| label)
            .unwrap_or(item)
            .replace("\\[", "[")
            .replace("\\]", "]");
        out.push_str(&" ".repeat(indent));
        out.push_str("• ");
        out.push_str(&label);
        out.push('\n');
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

/// 单块排版结果。`line_bases[i]` = 第 i 个逻辑行在排版文本（layout 坐标）中的起始字节偏移。
pub struct BlockLayout {
    pub(crate) buffer: Buffer,
    map: OffsetMap,
    line_bases: Vec<usize>,
    /// 排版文本（`\r` 与隐藏标记已剥除）：Reveal 行为的直接观测量。
    layout_text: String,
    pub height: f32,
    /// 行内公式图的块内位置（切片 8）；shell 按 tex 取位图 blit。
    pub imgs: Vec<PlacedImg>,
    /// 删除线区间（块相对 doc 坐标，含标记；paint 层经 selection_rects 取可视矩形）。
    pub strikes: Vec<(usize, usize)>,
    /// 前置装饰（文本光栅之下绘制）：块相对 doc 坐标 + 装饰类别。
    /// 含 mark 底色、kbd 圆角边框；用 selection_rects 取物理矩形。
    pub decorations_under: Vec<(usize, usize, UnderKind)>,
    /// 后置装饰（文本光栅之上绘制）：块相对 doc 坐标 + 装饰类别。
    /// 含 abbr 虚线下划线；strikethrough 沿用 strikes 字段。
    pub decorations_over: Vec<(usize, usize, OverKind)>,
    /// Sub/Sup 基线偏移区间（**layout 坐标**，(start, end, 物理像素 dy)）：
    /// 光栅化时按字形起点命中，正数下移（sub）、负数上移（sup）。
    pub script_ranges: Vec<(usize, usize, f32)>,
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
    /// 本块源码是否为 `[TOC]` 标记（视图层目录替换的适用对象）。
    pub(crate) toc_source: bool,
    /// 当前布局是否为生成的目录（false 时显示源码标记）。
    pub(crate) toc_rendered: bool,
}

impl BlockLayout {
    pub fn layout_text(&self) -> &str {
        &self.layout_text
    }

    /// 布局文本偏移 → 逻辑行下标（越界夹到末行）。生成目录的行与条目一一对应，
    /// 点击行据此定位要跳转的标题。
    pub(crate) fn line_at_layout(&self, off: usize) -> usize {
        let off = off.min(self.layout_text.len());
        self.line_bases
            .partition_point(|&b| b <= off)
            .saturating_sub(1)
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
        // TOC 标记块：视图态渲染目录、编辑态显 `[TOC]` 源码。目录布局没有行内节点，
        // 必须显式把 caret 是否在块内折进指纹，否则缓存命中路径不会随光标进出失效。
        if self.toc_source {
            fnv(&mut h, inside as u64);
        }
        match kind {
            BlockKind::BlockQuote | BlockKind::Admonition(_) | BlockKind::Callout(_) => {
                // 引用/提示块按行：line=0 即块外，天然区分「块外」与「块内任意行」。
                let line = if inside {
                    self.source_newlines.partition_point(|&o| o < rel) as u64 + 1
                } else {
                    0
                };
                fnv(&mut h, line);
            }
            // 块级标记显隐按整块 caret 判定（`block_subs`/`hard_break_subs`）：普通段落
            // 反斜杠硬换行、标题 `#`/Setext 下划线/围栏/列表符号/分隔线/脚注定义前缀。
            // 漏折任一种 → caret 进出块时缓存不变，残留旧态。
            BlockKind::Paragraph
            | BlockKind::ListItem { .. }
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

/// 块级字号相对正文的额外倍率：代码块 14/16，其余 1.0。
fn kind_scale(kind: &BlockKind) -> f32 {
    match kind {
        BlockKind::FencedCode(_)
        | BlockKind::IndentedCode
        | BlockKind::HtmlBlock
        | BlockKind::FrontMatter(_)
        | BlockKind::Mermaid => typography::CODE_SCALE,
        _ => 1.0,
    }
}

/// 按序返回第一个可用族：探测函数由调用方注入，便于无系统字体依赖地测试。
fn pick_family<'a>(candidates: &'a [&'a str], available: impl Fn(&str) -> bool) -> Option<&'a str> {
    candidates.iter().copied().find(|name| available(name))
}

/// 正文字体族 → cosmic family：
/// `system` 不设 family（沿用既有回退链）；`sans`/`serif` 显式指定中文字族；
/// `mono` 用通用等宽族。仅正文/表格文本走这里，UI chrome 不受影响。
pub(super) fn doc_family(font: DocFontFamily) -> Option<Family<'static>> {
    match font {
        DocFontFamily::System => None,
        DocFontFamily::Sans => Some(Family::Name("Microsoft YaHei")),
        DocFontFamily::Serif => Some(Family::Name("SimSun")),
        DocFontFamily::Mono => Some(Family::Monospace),
    }
}

/// 创建字体系统并显式指定 generic family（D8）。
///
/// cosmic-text 的默认族（Open Sans / Noto Sans Mono）在 Windows 上通常不存在，
/// 汉字会走各自回退，造成「`#` 与汉字不同族」。这里按 `typography` 的回退链
/// 探测存在的族后显式设置；候选全部缺失时保持 fontdb 默认。
///
/// 返回同时给出探测到的等宽族名：generic `Monospace` 的回退链会先耗尽等宽候选，
/// 旧 CJK 等宽字体（如 MS Gothic）会截走 Han，因此代码文本改用显式族名（N9）。
fn new_font_system() -> (FontSystem, Option<&'static str>) {
    let mut fs = FontSystem::new();
    let (sans, mono) = {
        let db = fs.db_mut();
        let pick = |candidates: &'static [&'static str]| {
            pick_family(candidates, |name| {
                db.faces()
                    .any(|f| f.families.iter().any(|(fam, _)| fam == name))
            })
        };
        (
            pick(typography::SANS_FAMILIES),
            pick(typography::MONO_FAMILIES),
        )
    };
    let db = fs.db_mut();
    if let Some(family) = sans {
        db.set_sans_serif_family(family);
    }
    if let Some(family) = mono {
        db.set_monospace_family(family);
    }
    (fs, mono)
}
pub struct LayoutEngine {
    font_system: FontSystem,
    swash: SwashCache,
    /// DPI 缩放：metrics 统一乘此值，布局单位保持物理像素。
    scale: f32,
    /// 正文字号基准（物理像素，未乘 scale）；配置读入后可覆盖默认值。
    doc_font_size: f32,
    /// 正文行高基准（物理像素，未乘 scale）；配置读入后可覆盖默认值。
    doc_line_height: f32,
    /// 正文字体族（仅正文/表格文本生效，UI chrome 恒用既有字阶）。
    doc_font: DocFontFamily,
    /// 启动时探测到的等宽族名：代码/行内代码显式指定，避免 generic Monospace 的
    /// 等宽候选链截走 Han（N9）。
    mono_family: Option<&'static str>,
    /// 代码块语法高亮色板（随主题切换更新）。
    syntax: SyntaxColors,
    /// 代码块增量高亮器：语法集惰性加载，逐行状态跨布局复用。
    highlighter: Highlighter,
    /// 链接前景色（随主题切换更新）。
    link_color: [u8; 3],
    /// 次要文本色（图片题注等）：与脚注前缀同源的主题 muted，随主题切换更新。
    muted_color: [u8; 3],
    /// 长代码块后台异步高亮回填代次：任何一个超限 FencedCode 回填颜色后递增；
    /// 递增会自然失效所有 LayoutKey → 触发重排（此时 Highlighter 缓存已命中，
    /// 同步路径瞬间产出颜色）。
    pub highlight_epoch: u64,
    /// 后台高亮器：持有已投递但尚未回填的任务代次，防止重复投递同一块。
    pub pending_background_ticks: std::collections::HashMap<usize, u64>,
    /// 后台 parse 任务投递通道：LayoutEngine 构造时 spawn，每帧 layout_block 命中超限块时
    /// 投递任务；Sender drop 后后台线程自动退出。
    pub(crate) bg_tx: Option<std::sync::mpsc::Sender<crate::view::highlight_bg::BgTask>>,
    /// 后台 parse 结果接收通道：App 层 update 循环每帧 try_recv drain。
    pub(crate) bg_rx: Option<std::sync::mpsc::Receiver<crate::view::highlight_bg::BgResult>>,
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutEngine {
    pub fn new() -> Self {
        let (font_system, mono_family) = new_font_system();
        let (bg_tx, bg_rx) = crate::view::highlight_bg::spawn_background();
        LayoutEngine {
            font_system,
            swash: SwashCache::new(),
            scale: 1.0,
            doc_font_size: BASE_FONT_SIZE,
            doc_line_height: BASE_LINE_HEIGHT,
            doc_font: DocFontFamily::System,
            mono_family,
            syntax: SyntaxColors::default_light(),
            highlighter: Highlighter::default(),
            link_color: Theme::light().link,
            muted_color: Theme::light().muted,
            highlight_epoch: 0,
            pending_background_ticks: std::collections::HashMap::new(),
            bg_tx: Some(bg_tx),
            bg_rx: Some(bg_rx),
        }
    }

    /// 设置正文排版参数（字号/行高为物理像素基准，字体族）。
    ///
    /// 启动读配置后调用一次，必须在任何排版之前完成；派生缓存键只含版心宽/DPI/主题等，
    /// 本参数在会话内固定，故无需额外失效（详见 view::cache 的键说明）。
    pub fn set_doc_typography(&mut self, font_size: f32, line_height: f32, font: DocFontFamily) {
        self.doc_font_size = font_size.max(1.0);
        self.doc_line_height = line_height.max(1.0);
        self.doc_font = font;
    }

    /// 正文字号基准（物理像素，未乘 scale）。
    pub(crate) fn doc_font_size(&self) -> f32 {
        self.doc_font_size
    }

    /// 正文行高基准（物理像素，未乘 scale）。
    pub(crate) fn doc_line_height(&self) -> f32 {
        self.doc_line_height
    }

    /// 当前正文字体族（表格单元格排版复用）。
    pub(crate) fn doc_font(&self) -> DocFontFamily {
        self.doc_font
    }

    /// 启动时探测到的等宽族名（代码块/行内代码排版复用；N9）。
    pub(crate) fn mono_family(&self) -> Option<&'static str> {
        self.mono_family
    }

    /// 每帧排版前 drain 后台 parse 结果：把后台产出的 spans 缓存进 Highlighter，
    /// 递增 highlight_epoch 让 LayoutKey 自然失配，下次 layout_block 命中缓存即同步
    /// 产出颜色。后台 parse 期间块被编辑/删除时，Highlighter::highlight 开头会通过
    /// text 比对兜底丢弃旧 spans —— 这里无条件缓存即可。
    pub(crate) fn drain_background(&mut self) {
        let Some(rx) = &self.bg_rx else { return };
        while let Ok(result) = rx.try_recv() {
            self.highlighter
                .cache_spans(result.key, result.body, result.spans);
            self.pending_background_ticks.remove(&result.key);
            self.highlight_epoch = self.highlight_epoch.saturating_add(1);
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

    /// 更新次要文本色（主题切换时调用）：图片题注与脚注前缀取同一 muted 语义。
    pub fn set_muted_color(&mut self, color: [u8; 3]) {
        self.muted_color = color;
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

    /// 文档内 IME preedit 小串布局（不换行、默认样式、无消隐），正文字阶。
    /// 与正文同套排版规则：kinsoku 胶水 + 中西文间距经 build_layout_text 注入，
    /// OffsetMap 保证组合串内光标偏移（IME 报告的原文档坐标）正确映射。
    pub fn layout_preedit(&mut self, text: &str) -> BlockLayout {
        self.layout_single(
            text,
            self.doc_font_size * self.scale,
            self.doc_line_height * self.scale,
            doc_family(self.doc_font),
            Weight::NORMAL,
        )
    }

    /// UI 单行排版（菜单/状态栏/浮层/模态及其中 IME 组合串）：UI 13px 字阶。
    pub(crate) fn layout_ui(&mut self, text: &str) -> BlockLayout {
        self.layout_single(
            text,
            typography::UI_SIZE * self.scale,
            typography::UI_LINE_HEIGHT * self.scale,
            None,
            Weight::NORMAL,
        )
    }

    /// 模态标题排版：比 UI 正文大一档并加粗，与正文共用同一 shaping/光栅路径。
    pub(crate) fn layout_ui_title(&mut self, text: &str) -> BlockLayout {
        self.layout_single(
            text,
            typography::DIALOG_TITLE_SIZE * self.scale,
            crate::tokens::surface::dialog_title_line_height(self.scale),
            None,
            Weight::SEMIBOLD,
        )
    }

    /// 单行排版：不做块级替换与消隐，只按给定字号/行高成形。
    fn layout_single(
        &mut self,
        text: &str,
        size: f32,
        line_height: f32,
        family: Option<Family<'static>>,
        weight: Weight,
    ) -> BlockLayout {
        let metrics = Metrics::new(size, line_height);
        let block = Range {
            start: 0,
            end: text.len(),
        };
        let (layout_text, map, line_bases, source_newlines, _) =
            build_layout_text(text, block, &[], usize::MAX, &[], true, false);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs = match family {
            Some(family) => Attrs::new().family(family).weight(weight),
            None => Attrs::new().weight(weight),
        };
        buffer.set_text(&layout_text, &attrs, Shaping::Advanced, None);
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
            decorations_under: Vec::new(),
            decorations_over: Vec::new(),
            script_ranges: Vec::new(),
            node_ranges: Vec::new(),
            img_ranges: Vec::new(),
            source_newlines,
            full_len: text.len(),
            truncated: false,
            toc_source: false,
            toc_rendered: false,
        }
    }

    /// 单行文本度量（宽, 高）：仅 shape 不 rasterize，供浮层/模态按内容定宽与折行。
    pub(crate) fn measure(&mut self, text: &str) -> (f32, f32) {
        let layout = self.layout_ui(text);
        let w = layout
            .buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0, f32::max);
        (w, layout.height)
    }

    /// 模态标题度量：与 `layout_ui_title` 同源，保证卡片宽度与上屏宽度一致。
    pub(crate) fn measure_title(&mut self, text: &str) -> (f32, f32) {
        let layout = self.layout_ui_title(text);
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
            kind,
            text,
            block,
            inlines,
            width,
            caret,
            imgs,
            reveal_all,
            None,
            &[],
            None,
            false,
        )
    }

    /// 带脚注编号表的排版入口：编号在全文范围内派生，由调用方每轮构建一次并传引用。
    /// `ordinal` 为有序列表项在文档块序列中的显示编号（见 [`ordered_list_ordinals`]）；
    /// `None` 表示调用方拿不到文档顺序，此时有序列表保留源码编号。
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
        folds: &[InlineFold],
        ordinal: Option<u32>,
        toc_list: bool,
    ) -> BlockLayout {
        self.layout_block_full(
            kind,
            text,
            block,
            inlines,
            width,
            caret,
            imgs,
            reveal_all,
            footnotes,
            folds,
            ordinal,
            toc_list,
            &[],
        )
    }

    /// 带图片题注的排版入口：题注随图片视觉行之后插入一行（视图派生文本，不写回文档）。
    #[allow(clippy::too_many_arguments)] // 内部布局函数参数天然多
    pub(crate) fn layout_block_with_captions(
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
        folds: &[InlineFold],
        ordinal: Option<u32>,
        toc_list: bool,
        captions: &[ImageCaption],
    ) -> BlockLayout {
        self.layout_block_full(
            kind, text, block, inlines, width, caret, imgs, reveal_all, footnotes, folds, ordinal,
            toc_list, captions,
        )
    }

    #[allow(clippy::too_many_arguments)] // 内部布局函数参数天然多
    fn layout_block_full(
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
        folds: &[InlineFold],
        ordinal: Option<u32>,
        toc_list: bool,
        captions: &[ImageCaption],
    ) -> BlockLayout {
        let full = text.strip_suffix('\n').unwrap_or(text);
        // 单块上限（v0）：只排前缀，超出部分不参与成形/光栅，避免内存放大
        let (body, mut truncated) = cap_body(full);
        // 标题/代码块字号按既有比例从正文字号派生。
        let scale = kind_scale(&kind) * block_scale(kind.clone()) * self.scale;
        let metrics = Metrics::new(self.doc_font_size * scale, self.doc_line_height * scale);
        let default_attrs = attrs_of(
            kind.clone(),
            Style::default(),
            self.link_color,
            self.doc_font,
            self.mono_family,
            metrics.font_size,
        );
        // NBSP 占位段：测量字体必须与实际占位 span 一致，尤其 Mermaid 使用等宽族。
        let nbsp_w = if imgs.is_empty() {
            1.0
        } else {
            self.measure_nbsp_with_attrs(metrics, &default_attrs)
        };
        let mut kept: Vec<&InlineImg> = Vec::new();
        // 缺失/远程 `<img>` 的占位文本区间（块相对 doc 坐标），供 paint 画占位框。
        let mut img_placeholders: Vec<(usize, usize)> = Vec::new();
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
            block_subs(kind.clone(), body, block, caret, ordinal, toc_list)
        };
        // 容器内嵌套块标记（REN-SWEEP-002）：引用/提示/列表项里再出现的
        // 列表、任务框、围栏代码、details 标签按块类型递归渲染。
        let mut container_code: Vec<(usize, usize)> = Vec::new();
        if !reveal_all {
            let (nested, code) = container_nested_subs(&kind, body, block, caret);
            subs.extend(nested);
            container_code = code;
        }
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
                // 向上取整：占位 advance 必须 ≥ 图宽，否则图右缘会压住后续文本
                // （CJK 前后行内公式曾因四舍五入向下而重叠）。再夹到整行可容纳的
                // 字形数：满宽图片时 ceil 可能多出一个 NBSP 宽度，导致占位折行 →
                // 每个视觉行都按图高计高，独立图片块高度翻倍（半屏空白）。
                let cap = ((width / nbsp_w).floor() as usize).max(1);
                let nbsp = ((im.width as f32 / nbsp_w).ceil() as usize).max(1).min(cap);
                subs.push(Sub::fill(rs, re, "\u{A0}".repeat(nbsp)));
                kept.push(im);
            }
            // `<br>` 硬换行：void 自闭合标签，整段（<br> 或 <br/>）替换为单个 \n。
            // 与 img 占位同源：br range 在 block 绝对坐标，转块相对后入 subs；
            // OffsetMap::from_spans 按起点排序归并，若与已有 subs 重叠（罕见）则按包含判定丢弃。
            collect_br_subs(inlines, block, &mut subs);
            // `<q>` 双引号：非激活态在内容两端插入可见引号（激活态显源码，不插）。
            collect_q_subs(inlines, block, caret, &mut subs);
            // `<img>` 无可用位图（远程 src 被解析层置空 / 本地缺失 / 加载失败）时，
            // 整段替换为 `[alt]` 文本占位，并登记占位框装饰（paint 画浅灰底）。
            // 带题注的图片改用通用占位：alt 已由下方题注给出，避免重复显示。
            collect_html_img_subs(
                inlines,
                block,
                caret,
                &kept,
                captions,
                &mut subs,
                &mut img_placeholders,
            );
            // 图片题注（仅独立成行的图片、alt 非空）：在图片所在视觉行之后插入独立一行。
            // 视图派生文本，不进文档字节；光标进入图片（显源码）时让位，避免与 alt 重复。
            for cap in captions {
                if node_active(cap.range, caret) {
                    continue;
                }
                let rs = cap.range.start.saturating_sub(block.start);
                let re = cap.range.end.saturating_sub(block.start).min(body.len());
                if rs >= re || !body.is_char_boundary(rs) || !body.is_char_boundary(re) {
                    continue;
                }
                let loaded = kept.iter().any(|im| im.range == cap.range);
                if !loaded && !img_placeholders.iter().any(|&(s, e)| s == rs && e == re) {
                    // Markdown 图片缺失：整节点降级为通用占位，alt 由题注行给出（不重复）。
                    subs.push(Sub::fill(rs, re, t(Key::ImageAltFallback)));
                    img_placeholders.push((rs, re));
                }
                let line_end = body[re..].find('\n').map_or(body.len(), |i| re + i);
                let (anchor, fill) = if line_end < body.len() {
                    (line_end + 1, format!("{}\n", cap.text))
                } else {
                    (body.len(), format!("\n{}", cap.text))
                };
                subs.push(Sub::caption(anchor, fill));
            }
            // 行内 `<details>` 折叠（B3）：收起态的正文整段消隐；标签本身由
            // `collect_elide` 按行内 HTML 规则消隐，故不出现裸标签。
            for f in folds {
                if !f.expanded && f.body.0 < f.body.1 {
                    subs.push(Sub::elide(f.body.0, f.body.1));
                }
            }
        }
        let apply_cjk = !matches!(
            kind,
            BlockKind::FencedCode(_)
                | BlockKind::IndentedCode
                | BlockKind::Math
                | BlockKind::Table
                | BlockKind::Mermaid
                // Details 内是整块源码文本，标签消隐后不应在 CJK↔ASCII 边界补空格，
                // 否则 summary 首字前会多出一个游标空格。
                | BlockKind::Details { .. }
        );
        // 反斜杠硬换行标记：非活动块消隐行尾 `\`；换行语义由源 `\n` 保留（REN-SWEEP-007）。
        if !reveal_all
            && !matches!(
                kind,
                BlockKind::FencedCode(_)
                    | BlockKind::IndentedCode
                    | BlockKind::Math
                    | BlockKind::Table
                    | BlockKind::Mermaid
            )
        {
            subs.extend(hard_break_subs(
                body,
                block,
                caret,
                inlines,
                &container_code,
            ));
        }
        let (layout_text, map, line_bases, source_newlines, caption_layout_spans) =
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
        // style_spans 产出文档坐标，经映射转 layout 坐标；整体落入消隐/占位区的 span 零宽丢弃。
        // 占位 NBSP 区无样式来源，显式补默认样式 span（cosmic 排版文本 = 各 span 顺次拼接，
        // 覆盖必须完备且有序）；指针序即 layout 偏移序。
        //
        // FencedCode 块带语言标记时走语法高亮（inlines 为空，style_spans 仅产出默认段）。
        let style_segments: Vec<(usize, usize, Style)> = match &kind {
            BlockKind::FencedCode(Some(lang)) => {
                // 长代码块（行数/字节数超阈值）首次渲染跳过高亮：先上默认色首帧，
                // 后台 parse 完成后通过 LayoutKey 失效触发重排，此时缓存已命中 → 同步产出颜色。
                let is_long = body.lines().count()
                    >= crate::view::highlight::LONG_CODE_THRESHOLD_LINES
                    || body.len() >= crate::view::highlight::LONG_CODE_THRESHOLD_BYTES;
                let cache_hit = self.highlighter.has(block.start);
                if is_long && !cache_hit {
                    // 投递后台 parse 任务；epoch 用当前 highlight_epoch，后续回填时校验。
                    if let Some(tx) = &self.bg_tx {
                        // 去重：同一 key 在本 epoch 已投递则不重复。
                        let current_epoch = self.highlight_epoch;
                        let should_send = self
                            .pending_background_ticks
                            .get(&block.start)
                            .copied()
                            .map(|e| e < current_epoch)
                            .unwrap_or(true);
                        if should_send {
                            let _ = tx.send(crate::view::highlight_bg::BgTask {
                                key: block.start,
                                lang: lang.to_string(),
                                body: body.to_string(),
                                epoch: current_epoch,
                            });
                            self.pending_background_ticks
                                .insert(block.start, current_epoch);
                        }
                    }
                    // 返回空 spans → to_styles 产出覆盖整个 body 的全默认色段。
                    to_styles(&[], body.len(), &self.syntax, Style::default())
                } else {
                    let tk = self.highlighter.highlight(block.start, lang, body);
                    to_styles(&tk, body.len(), &self.syntax, Style::default())
                }
            }
            _ => style_spans(body.len(), block, inlines),
        };
        // `[n]` 的展示文本并非源码字符，样式表里没有它的颜色来源；
        // 按替换区间切分样式段并置 link（style_spans 会把同式相邻段合并，不能整段改）。
        let style_segments = with_link_spans(style_segments, &footnote_links, body.len());
        // 容器内嵌套围栏代码：整行覆盖为等宽 + 代码底色（覆盖行内标记样式）。
        let style_segments = with_container_code_spans(style_segments, &container_code, body.len());
        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        // 装饰文本（脚注定义编号前缀 / 块末 `↩`）由下方显式 span 提供；基础样式段
        // 必须让出这两段，否则 set_rich_text 会把同一段拼接两次 —— 表现即
        // 定义行在返回箭头前重复出现 `n.`（REN-SWEEP-006）。
        let decor_prefix_end = decor.prefix_len.min(layout_text.len());
        let decor_suffix_start = layout_text.len().saturating_sub(decor.suffix_len);
        for &(s, e, st) in &style_segments {
            let (ls, le) = (map.doc_to_layout(s), map.doc_to_layout(e));
            let ls = ls.max(decor_prefix_end);
            let le = le.min(decor_suffix_start);
            if ls < le {
                spans.push((
                    &layout_text[ls..le],
                    attrs_of(
                        kind.clone(),
                        st,
                        self.link_color,
                        self.doc_font,
                        self.mono_family,
                        metrics.font_size,
                    ),
                ));
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
                spans.push((
                    &layout_text[ls..le],
                    attrs_of(
                        kind.clone(),
                        st,
                        self.link_color,
                        self.doc_font,
                        self.mono_family,
                        metrics.font_size,
                    ),
                ));
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
                spans.push((
                    &layout_text[ls..le],
                    attrs_of(
                        kind.clone(),
                        st,
                        self.link_color,
                        self.doc_font,
                        self.mono_family,
                        metrics.font_size,
                    ),
                ));
            }
        }
        // B4：图片高度参与行高。图高于正文行高时，把占位区间的 attrs 的 line_height
        // 抬到图高（cosmic-text 逐 span metrics），该视觉行整体变高 → 后续行/光标/
        // 命中随之下移，不会与图片重叠。占位区间可能与既有样式 span 重叠，故先按
        // 图片区间切分基础 span（图片 attrs 优先），避免重复 span 被 cosmic 丢弃。
        let mut img_specs: Vec<(usize, usize, Attrs)> = Vec::new();
        for im in &kept {
            let ls = map.doc_to_layout(im.range.start.saturating_sub(block.start));
            let ll = map.doc_to_layout(im.range.end.saturating_sub(block.start)) - ls;
            let img_metrics = if im.height as f32 > metrics.line_height {
                Metrics::new(metrics.font_size, im.height as f32)
            } else {
                metrics
            };
            img_specs.push((ls, ls + ll, default_attrs.clone().metrics(img_metrics)));
        }
        if !img_specs.is_empty() {
            let base = layout_text.as_ptr() as usize;
            let mut out: Vec<(&str, Attrs)> = Vec::new();
            // 每个图片占位区间已被基础 span 覆盖的字节段：拼接式富文本里同一段
            // 只能出现一次，否则占位文本被重复排版（图片块高度翻倍）。
            let mut covered: Vec<Vec<(usize, usize)>> = vec![Vec::new(); img_specs.len()];
            for (txt, at) in spans.drain(..) {
                let s = txt.as_ptr() as usize - base;
                let mut pieces = vec![(s, s + txt.len(), at)];
                for (idx, (is, ie, iat)) in img_specs.iter().enumerate() {
                    let mut next = Vec::new();
                    for (ps, pe, pat) in pieces {
                        if *ie <= ps || *is >= pe {
                            next.push((ps, pe, pat));
                            continue;
                        }
                        if ps < *is {
                            next.push((ps, *is, pat.clone()));
                        }
                        let (os, oe) = (ps.max(*is), pe.min(*ie));
                        if os < oe {
                            next.push((os, oe, iat.clone()));
                            covered[idx].push((os, oe));
                        }
                        if pe > *ie {
                            next.push((*ie, pe, pat));
                        }
                    }
                    pieces = next;
                }
                for (ps, pe, pat) in pieces {
                    out.push((&layout_text[ps..pe], pat));
                }
            }
            // 基础 span 未覆盖的占位字节段补默认样式（仅补空缺，不重复整段）。
            for (idx, (is, ie, at)) in img_specs.iter().enumerate() {
                let mut pos = *is;
                covered[idx].sort_unstable();
                for (cs, ce) in &covered[idx] {
                    if *cs > pos {
                        out.push((&layout_text[pos..*cs], at.clone()));
                    }
                    pos = pos.max(*ce);
                }
                if pos < *ie {
                    out.push((&layout_text[pos..*ie], at.clone()));
                }
            }
            spans = out;
        }
        // 题注：次要文本色（与脚注前缀同源的 muted），字号/行高沿用本块 metrics。
        // 位于 layout 文本末尾/图片行之后，必须显式补样式；换行本身无字形，可留空缺。
        for &(cs, ce) in &caption_layout_spans {
            if cs >= ce || ce > layout_text.len() {
                continue;
            }
            let st = Style {
                color: Some(self.muted_color),
                ..Default::default()
            };
            spans.push((
                &layout_text[cs..ce],
                attrs_of(
                    kind.clone(),
                    st,
                    self.link_color,
                    self.doc_font,
                    self.mono_family,
                    metrics.font_size,
                ),
            ));
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
            self.doc_line_height * self.scale
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
        // 图片与 Mermaid 底边贴基线，高位图会把图顶推到行顶之上：钳到行顶，避免上侵前一块
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
               // 独占视觉行的图片按 `<img align>` 在版心内水平对齐（占位自身不可见，
               // 移动位图不会与文本重叠；行内图片的 align 已在 shell 侧置空）。
            let x = match im.align {
                Some(veil_core::ImageAlign::Center) => ((width - im.width as f32) * 0.5).max(0.0),
                Some(veil_core::ImageAlign::Right) => (width - im.width as f32).max(0.0),
                _ => x,
            };
            let mut y = line_y - im.baseline as f32; // 可为负（图高出行顶），frame.blit 裁剪
            if matches!(im.key, InlineAsset::Image(_) | InlineAsset::Mermaid(_)) {
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
        // Sub/Sup 基线偏移：doc 坐标经 OffsetMap 转 layout 坐标（光栅按字形起点命中）。
        let script_ranges: Vec<(usize, usize, f32)> = style_segments
            .iter()
            .filter_map(|&(s, e, st)| {
                let shift = st.baseline_shift?;
                let (ls, le) = (map.doc_to_layout(s), map.doc_to_layout(e));
                (ls < le).then_some((ls, le, script_shift_px(shift, self.scale)))
            })
            .collect();
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
            // 前置装饰：行内代码底色 / mark 底色 / kbd 圆角边框（按此顺序叠，后者覆盖前者）；
            // 块相对 doc 坐标，paint 层用 selection_rects 取物理矩形。
            decorations_under: style_segments
                .iter()
                .flat_map(|&(s, e, st)| {
                    let mut out: Vec<(usize, usize, UnderKind)> = Vec::new();
                    if st.code_bg {
                        out.push((s, e, UnderKind::InlineCodeBg));
                    }
                    if st.mark_bg.is_some() {
                        out.push((s, e, UnderKind::MarkBg));
                    }
                    if st.kbd_frame {
                        out.push((s, e, UnderKind::KbdFrame));
                    }
                    if let Some(bg) = st.bg {
                        out.push((s, e, UnderKind::StyleBg(bg)));
                    }
                    out
                })
                .chain(
                    img_placeholders
                        .iter()
                        .map(|&(s, e)| (s, e, UnderKind::ImagePlaceholder)),
                )
                .collect(),
            // 后置装饰：abbr 虚线下划线 / 实线下划线 / 上划线；画在文本光栅之上。
            decorations_over: style_segments
                .iter()
                .flat_map(|&(s, e, st)| {
                    let mut out: Vec<(usize, usize, OverKind)> = Vec::new();
                    if st.dotted_underline {
                        out.push((s, e, OverKind::DottedUnderline));
                    }
                    if st.underline {
                        out.push((s, e, OverKind::Underline));
                    }
                    if st.overline {
                        out.push((s, e, OverKind::Overline));
                    }
                    out
                })
                .collect(),
            // Sub/Sup 基线偏移区间（layout 坐标），供光栅化施加 y 偏移。
            script_ranges,
            node_ranges,
            img_ranges,
            source_newlines,
            full_len: full.len(),
            truncated,
            toc_source: false,
            toc_rendered: false,
        }
    }

    /// 生成目录的视图层排版：`text` 为目录展示文本（非文档字节），按正文段落排版。
    /// 不产生任何可编辑映射（doc 坐标按展示文本恒等），也不写回文档。
    pub(crate) fn layout_toc(&mut self, text: &str, width: f32) -> BlockLayout {
        let scale = block_scale(BlockKind::Paragraph) * self.scale;
        let metrics = Metrics::new(self.doc_font_size * scale, self.doc_line_height * scale);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let attrs = attrs_of(
            BlockKind::Paragraph,
            Style::default(),
            self.link_color,
            self.doc_font,
            self.mono_family,
            metrics.font_size,
        );
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_size(Some(width), None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let mut height = buffer
            .layout_runs()
            .map(|r| r.line_top + r.line_height)
            .fold(metrics.line_height, f32::max);
        let max_line_w = buffer
            .layout_runs()
            .flat_map(|r| r.glyphs.iter())
            .map(|g| g.x + g.w)
            .fold(width, f32::max)
            .max(1.0);
        let budget_h = MAX_RASTER_PX as f32 / max_line_w;
        let truncated = height > budget_h;
        if truncated {
            height = budget_h.max(metrics.line_height);
        }
        let line_bases: Vec<usize> = buffer
            .layout_runs()
            .map(|r| r.glyphs.first().map_or(0, |g| g.start))
            .collect();
        let (map, _) = OffsetMap::from_spans(text.len(), &[]);
        BlockLayout {
            buffer,
            map,
            line_bases,
            layout_text: text.to_string(),
            height,
            imgs: Vec::new(),
            strikes: Vec::new(),
            decorations_under: Vec::new(),
            decorations_over: Vec::new(),
            script_ranges: Vec::new(),
            node_ranges: Vec::new(),
            img_ranges: Vec::new(),
            source_newlines: Vec::new(),
            full_len: text.len(),
            truncated,
            toc_source: true,
            toc_rendered: true,
        }
    }

    /// 表格公式占位沿用默认字体口径。
    pub(crate) fn measure_nbsp(&mut self, metrics: Metrics) -> f32 {
        self.measure_nbsp_with_attrs(metrics, &Attrs::new())
    }

    /// NBSP 占位宽度：同 metrics/attrs 下排 8 个 NBSP 求均值，缺字形时退化 1px 防除零。
    fn measure_nbsp_with_attrs(&mut self, metrics: Metrics, attrs: &Attrs) -> f32 {
        let probe = "\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}\u{A0}";
        let mut b = Buffer::new(&mut self.font_system, metrics);
        b.set_text(probe, attrs, Shaping::Advanced, None);
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
        self.rasterize_buffer_scripts(&layout.buffer, layout.height, color, &layout.script_ranges)
    }

    /// 光栅化任意已排版 buffer（表格单元格复用同一 path）。`height` 为排版高（物理像素）。
    /// 悬挂标点 v0 在此生效：满行末字为闭标点时光栅右移半字宽（排版几何不变，
    /// caret/命中/选区仍按未位移坐标——光标落在标点之后恰与位移后墨缘对齐）。
    /// 带 Sub/Sup 基线偏移的光栅化：`scripts` 为 layout 字节区间 → 物理像素 dy
    /// （正数下移 sub，负数上移 sup）。字形起点落入区间即施加偏移。
    pub(crate) fn rasterize_buffer_scripts(
        &mut self,
        buffer: &Buffer,
        height: f32,
        color: [u8; 3],
        scripts: &[(usize, usize, f32)],
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
                let script_dy = scripts
                    .iter()
                    .find(|(s, e, _)| *s <= g.start && g.start < *e)
                    .map_or(0.0, |&(_, _, dy)| dy);
                let baseline = (run.line_y + script_dy).round();
                let physical = g.physical((shift, baseline), 1.0);
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
        '\u{3002}'
            | '\u{3001}'
            | '\u{FF0C}'
            | '\u{FF0E}'
            | '\u{FF1B}'
            | '\u{FF1A}'
            | '\u{FF1F}'
            | '\u{FF01}'
            | '\u{FF09}'
            | '\u{3011}'
            | '\u{300B}'
            | '\u{300D}'
            | '\u{300F}'
            | '\u{201D}'
            | '\u{2019}'
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

/// 行内 `<details>` 折叠区间（B3）：全部为**块相对**字节偏移。
#[derive(Clone, Copy, Debug)]
pub(crate) struct InlineDetailsSpan {
    /// `<details` 开标签的块内相对起点（折叠状态的稳定键）。
    pub key: usize,
    /// summary 可见内容区间（点击切换的命中区）。
    pub summary: Option<(usize, usize)>,
    /// 正文区间（summary 内容之后 → `</details>` 内容末尾）；收起时整段消隐。
    pub body: (usize, usize),
}

/// 从行内节点树收集行内 `<details>` 折叠区间（块相对）。嵌套 details 一并收集。
pub(crate) fn inline_details_spans(nodes: &[InlineNode], block: Range) -> Vec<InlineDetailsSpan> {
    fn walk(nodes: &[InlineNode], block: Range, out: &mut Vec<InlineDetailsSpan>) {
        for n in nodes {
            if let InlineKind::Html {
                tag: HtmlTag::Details,
                ..
            } = &n.kind
            {
                let content_s = n.content.start.saturating_sub(block.start);
                let content_e = n.content.end.saturating_sub(block.start);
                let summary = n.children.iter().find_map(|c| match &c.kind {
                    InlineKind::Html {
                        tag: HtmlTag::Summary,
                        ..
                    } => Some((
                        c.content.start.saturating_sub(block.start),
                        c.content.end.saturating_sub(block.start),
                    )),
                    _ => None,
                });
                let body = match summary {
                    Some((_, ce)) => (ce, content_e),
                    None => (content_s, content_e),
                };
                out.push(InlineDetailsSpan {
                    key: n.range.start.saturating_sub(block.start),
                    summary,
                    body,
                });
            }
            walk(&n.children, block, out);
        }
    }
    let mut out = Vec::new();
    walk(nodes, block, &mut out);
    out
}

/// 块内全部 `<details>` 折叠区间（块相对）：行内节点树 + 容器逐行扫描合并。
///
/// 容器块（引用/提示/列表项）里的 `<details>` 在解析层常退化为 `HtmlLiteral`，
/// 行内节点树收集不到区间；这些块改走逐行扫描。两者按 `key`（`<details` 开标签
/// 的块内偏移）去重，折叠状态的键口径保持唯一。
pub(crate) fn block_details_spans(
    kind: &BlockKind,
    body: &str,
    nodes: &[InlineNode],
    block: Range,
) -> Vec<InlineDetailsSpan> {
    let mut spans = inline_details_spans(nodes, block);
    for s in container_details_spans(kind, body) {
        if !spans.iter().any(|x| x.key == s.key) {
            spans.push(s);
        }
    }
    spans
}

/// 容器内未闭合的 `<details>`（逐行扫描的栈元素）。
struct OpenDetails {
    /// `<details` 开标签的块内偏移（折叠状态键）。
    key: usize,
    /// `<summary>` 标签结束位置（尚未遇到 `</summary>` 时暂存）。
    summary_start: Option<usize>,
    /// 已闭合的 summary 内容区间。
    summary: Option<(usize, usize)>,
}

/// 容器块（引用/提示/自定义 Callout/列表项）内 `<details>` 折叠区间（块相对）。
///
/// 口径与行内版本一致：`key` = `<details` 开标签块内偏移；`summary` 为
/// `<summary>`…`</summary>` 的内容区间；`body` = summary 之后 → `</details>` 之前。
/// 无 `<summary>` 的 details 不产出折叠区间——没有可点击的摘要，收起会让正文
/// 既不可见也无法通过点击恢复（裸标签仍按既有规则消隐）。
fn container_details_spans(kind: &BlockKind, body: &str) -> Vec<InlineDetailsSpan> {
    let quote_like = matches!(
        kind,
        BlockKind::BlockQuote | BlockKind::Callout(_) | BlockKind::Admonition(_)
    );
    let list_like = matches!(kind, BlockKind::ListItem { .. });
    if !quote_like && !list_like {
        return Vec::new();
    }
    let mut stack: Vec<OpenDetails> = Vec::new();
    let mut out = Vec::new();
    let mut in_fence: Option<u8> = None;
    let mut off = 0usize;
    for line in body.split_inclusive('\n') {
        // 内容起点：引用类剥全部 `>` 标记；列表项剥缩进（首行符号不影响标签定位）。
        let content_off = if quote_like {
            let mut s = 0usize;
            while let Some(p) = quote_marker_pos(&line[s..]) {
                s += p + 1;
            }
            s
        } else {
            line.len() - line.trim_start_matches([' ', '\t']).len()
        };
        let content = line.get(content_off..).unwrap_or("");
        // 围栏代码内的 `<details>` 是字面量，不参与折叠（与 `container_nested_subs` 同口径）。
        if let Some(ch) = in_fence {
            if is_fence_close(content, ch) {
                in_fence = None;
            }
        } else if let Some(ch) = fence_open_char(content) {
            in_fence = Some(ch);
        } else {
            scan_details_line(content, off + content_off, &mut stack, &mut out);
        }
        off += line.len();
    }
    out
}

/// 单行标签扫描：维护 `<details>` 嵌套栈，成对闭合且有 summary 时产出折叠区间。
fn scan_details_line(
    line: &str,
    base: usize,
    stack: &mut Vec<OpenDetails>,
    out: &mut Vec<InlineDetailsSpan>,
) {
    let lower = line.to_ascii_lowercase();
    let mut i = 0usize;
    while i < lower.len() {
        let Some(rel) = lower[i..].find('<') else {
            break;
        };
        let start = i + rel;
        let Some(gt) = lower[start..].find('>') else {
            break;
        };
        let end = start + gt + 1;
        let tag = &lower[start..end];
        if tag.starts_with("</details") {
            if let Some(open) = stack.pop() {
                if let Some((ss, se)) = open.summary {
                    out.push(InlineDetailsSpan {
                        key: open.key,
                        summary: Some((ss, se)),
                        body: (se, base + start),
                    });
                }
            }
        } else if tag.starts_with("<details") {
            stack.push(OpenDetails {
                key: base + start,
                summary_start: None,
                summary: None,
            });
        } else if tag.starts_with("</summary") {
            if let Some(o) = stack.last_mut() {
                if let Some(s) = o.summary_start.take() {
                    o.summary = Some((s, base + start));
                }
            }
        } else if tag.starts_with("<summary") {
            if let Some(o) = stack.last_mut() {
                o.summary_start = Some(base + end);
            }
        }
        i = end;
    }
}

/// 单个行内 `<details>` 的排版折叠输入：`expanded == false` 时 body 整段消隐。
#[derive(Clone, Copy, Debug)]
pub struct InlineFold {
    pub body: (usize, usize),
    pub expanded: bool,
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

/// 行内代码 span 的块相对区间（含两端反引号标记）：CJK↔ASCII 自动间距的排除集。
fn collect_code_spans(nodes: &[InlineNode], block: Range, out: &mut Vec<(usize, usize)>) {
    for n in nodes {
        if n.kind == InlineKind::Code {
            let rs = n.range.start.saturating_sub(block.start);
            let re = n.range.end.saturating_sub(block.start);
            if rs < re {
                out.push((rs, re));
            }
        }
        collect_code_spans(&n.children, block, out);
    }
}

/// 缺失/远程 `<img src alt>` 的占位替换：无可用位图时整段标签换成 `[alt]` 可见文本，
/// 并登记占位区间供 paint 画浅灰占位框。可加载的本地图片走 `kept`（NBSP 占位 + 位图 blit）。
fn collect_html_img_subs(
    nodes: &[InlineNode],
    block: Range,
    caret: usize,
    kept: &[&InlineImg],
    captions: &[ImageCaption],
    out: &mut Vec<Sub>,
    placeholders: &mut Vec<(usize, usize)>,
) {
    for n in nodes {
        if let InlineKind::HtmlSelfClosed {
            tag: HtmlTag::Img,
            attrs,
        } = &n.kind
        {
            // 激活态显源码：标签区间已不消隐，勿再替换成占位文本
            let active = node_active(n.range, caret);
            if !active && !kept.iter().any(|im| im.range == n.range) {
                let rs = n.range.start.saturating_sub(block.start);
                let re = n.range.end.saturating_sub(block.start);
                if rs < re {
                    // 带题注的图片：alt 由下方题注给出，占位只提示「图片不可用」。
                    let label = if captions.iter().any(|c| c.range == n.range) {
                        t(Key::ImageAltFallback).to_string()
                    } else {
                        match attrs.alt.as_deref().filter(|a| !a.is_empty()) {
                            Some(alt) => format!("[{alt}]"),
                            None => t(Key::ImageAltFallback).to_string(),
                        }
                    };
                    out.push(Sub::fill(rs, re, label));
                    placeholders.push((rs, re));
                }
            }
        }
        collect_html_img_subs(&n.children, block, caret, kept, captions, out, placeholders);
    }
}

/// `<br>` 硬换行替换：递归找全部 `HtmlSelfClosed(Br)` 节点，整段 range 替换为单个 `\n`。
/// void 自闭合标签（`<br>` / `<br/>`）的 range 即为完整标签区间，无 children / content。
/// 调用方只在 !reveal_all 时调用（源码模式显示原文，不做 HTML 断行替换）。
fn collect_br_subs(nodes: &[InlineNode], block: Range, out: &mut Vec<Sub>) {
    for n in nodes {
        match &n.kind {
            InlineKind::HtmlSelfClosed { tag, .. } if *tag == HtmlTag::Br => {
                let rs = n.range.start.saturating_sub(block.start);
                let re = n.range.end.saturating_sub(block.start);
                if rs < re {
                    out.push(Sub::fill(rs, re, "\n"));
                }
            }
            _ => {}
        }
        collect_br_subs(&n.children, block, out);
    }
}

/// `<q>` 双引号插入：非激活节点的内容两端各插入零宽 `“`/`”`。
/// 与 [block::push_visible] 的表格可见文本口径一致（表格单元格同样补引号）。
fn collect_q_subs(nodes: &[InlineNode], block: Range, caret: usize, out: &mut Vec<Sub>) {
    for n in nodes {
        if let InlineKind::Html {
            tag: HtmlTag::Q, ..
        } = &n.kind
        {
            if !node_active(n.range, caret) {
                let cs = n.content.start.saturating_sub(block.start);
                let ce = n.content.end.saturating_sub(block.start);
                if cs < ce {
                    out.push(Sub::insert(cs, "“"));
                    out.push(Sub::insert(ce, "”"));
                } else {
                    out.push(Sub::insert(cs, "“”"));
                }
            }
        }
        collect_q_subs(&n.children, block, caret, out);
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
            walk(&n.children, block, caret, body, footnotes, subs, links);
        }
    }

    let mut subs = Vec::new();
    let mut links = Vec::new();
    walk(nodes, block, caret, body, footnotes, &mut subs, &mut links);
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
            // 脚注引用 `[^id]` → 上标 `[n]`（与 HTML 导出 `<sup>` 语义一致）。
            // 光标进入引用节点时按 Reveal 契约显源码原文，编号呈现不受影响。
            st.link = true;
            st.size_factor = Some(0.75);
            st.baseline_shift = Some(-1.0);
        }
        match out.last_mut() {
            Some(last) if last.2 == st && last.1 == s => last.1 = e,
            _ => out.push((s, e, st)),
        }
    }
    out
}

/// 容器内嵌套围栏代码（REN-SWEEP-002）：把围栏内容行覆盖为等宽 + 代码底色。
/// 覆盖而非叠加：围栏内容里的 `**`/`` ` `` 等行内标记不应生效（它们属于代码）。
fn with_container_code_spans(
    segments: Vec<(usize, usize, Style)>,
    code: &[(usize, usize)],
    text_len: usize,
) -> Vec<(usize, usize, Style)> {
    if code.is_empty() {
        return segments;
    }
    let mut bounds: Vec<usize> = vec![0, text_len];
    for &(s, e, _) in &segments {
        bounds.push(s);
        bounds.push(e);
    }
    for &(s, e) in code {
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
        let st = if code.iter().any(|&(cs, ce)| cs <= s && e <= ce) {
            Style {
                mono: true,
                code_bg: true,
                ..Default::default()
            }
        } else {
            segments
                .iter()
                .find(|(ss, se, _)| *ss <= s && e <= *se)
                .map(|x| x.2)
                .unwrap_or_default()
        };
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
    if indent > 3 || bytes.get(indent) != Some(&b'[') || bytes.get(indent + 1) != Some(&b'^') {
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
    prev.is_whitespace()
        || next.is_whitespace()
        || prev == '\n'
        || next == '\n'
        || prev == '\r'
        || next == '\r'
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
/// 例外：`elided` 须显式过滤——单字符消隐段（如 Link 的 `[`）与同区间的间距 fill
/// 在 from_spans 中同起点「长者优先」，fill 会胜出并把已消隐的标记重新画出来。
fn build_typo_subs(
    body: &str,
    code_spans: &[(usize, usize)],
    elided: &[(usize, usize)],
) -> Vec<Sub> {
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
        // 行内代码 span（含两端反引号）内永不插间距：内容里的 CJK↔ASCII 边界
        // 必须保持源码原样；而两端反引号标记处于消隐区间，若在此生成 fill，会与
        // elide 同起点竞争并被优先采纳（fill 胜 elide），把反引号重新画出来。
        if code_spans.iter().any(|&(s, e)| (s..e).contains(&j)) {
            continue;
        }
        // 被消隐的字符不参与间距：它不会出现在排版文本里，注入空格只会在
        // from_spans 中与消隐段争同一起点（fill 胜 elide），把标记残片画出来。
        if elided.iter().any(|&(s, e)| s <= j && j < e) {
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
            runs.get(i + 1)
                .is_some_and(|next| next.line_i == run.line_i)
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
        let last = run.glyphs.iter().rev().find_map(|g| {
            run.text
                .get(g.start..g.end)?
                .chars()
                .rev()
                .find(|&c| visible(c))
        });
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
/// 排版文本构建结果：文本、坐标映射、行基、源换行偏移、题注 layout 区间。
type BuildLayoutText = (
    String,
    OffsetMap,
    Vec<usize>,
    Vec<usize>,
    Vec<(usize, usize)>,
);

fn build_layout_text(
    body: &str,
    block: Range,
    inlines: &[InlineNode],
    caret: usize,
    subs: &[Sub],
    apply_cjk: bool,
    reveal_all: bool,
) -> BuildLayoutText {
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
    all.extend(elide.iter().map(|&(s, e)| Sub::elide(s, e)));
    all.extend_from_slice(subs);
    if apply_cjk {
        let mut code_spans = Vec::new();
        collect_code_spans(inlines, block, &mut code_spans);
        // 块级消隐（含硬换行反斜杠）同样不得参与 CJK 自动间距；否则同起点的
        // 间距 fill 会压过 elide，把本应隐藏的标记重新拼回 shaped 文本。
        let mut typo_elide = elide;
        typo_elide.extend(
            subs.iter()
                .filter(|s| s.fill.is_empty() && s.doc_start < s.doc_end)
                .map(|s| (s.doc_start, s.doc_end)),
        );
        all.extend(build_typo_subs(body, &code_spans, &typo_elide));
    }
    let (map, accepted) = OffsetMap::from_spans(body.len(), &all);
    // 保留段取源文本并记行基；替换段写 fill（fill 可含换行，如 `<br>` 与题注）。
    let mut layout_text = String::with_capacity(map.layout_len);
    let mut line_bases = vec![0];
    let mut source_newlines: Vec<usize> = Vec::new();
    let mut caption_spans: Vec<(usize, usize)> = Vec::new();
    for seg in &map.segs {
        match seg.sub {
            Some(i) => {
                // 替换段 fill 可能含换行（br → \n、题注行），也要扫描记行基。
                let sub = &accepted[i];
                let fill = &sub.fill;
                let layout_start = layout_text.len();
                for (j, b) in fill.bytes().enumerate() {
                    if b == b'\n' {
                        line_bases.push(layout_start + j + 1);
                        // 替换段没有源文本坐标，source_newlines 不记（这些换行是虚拟的）。
                    }
                }
                if sub.caption {
                    // 题注 fill 含起/止换行，整段作为一个 span：换行必须随 span 入文本，
                    // 否则题注会与上一行拼在同一视觉行（set_rich_text 只拼接 span 文本）。
                    caption_spans.push((layout_start, layout_start + fill.len()));
                }
                layout_text.push_str(fill);
            }
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
    (layout_text, map, line_bases, source_newlines, caption_spans)
}

/// 先序收集全部行内节点区间（doc 绝对坐标；调用方随即转块相对）：激活指纹按固定顺序折叠。
fn collect_node_ranges(nodes: &[InlineNode], out: &mut Vec<Range>) {
    for n in nodes {
        out.push(n.range);
        collect_node_ranges(&n.children, out);
    }
}

/// 反斜杠硬换行标记：CommonMark 行尾 `\` + 换行即硬换行，`\` 本身不是正文。
/// 非活动块把该反斜杠消隐（换行仍由源 `\n` 保留，语义与命中映射不变）；
/// 活动块/源码模式显原文。偶数个连续反斜杠已被转义为字面 `\`，不处理。
/// `code_spans`（容器内嵌围栏代码的行区间）内的反斜杠是代码内容，同样不处理。
fn hard_break_subs(
    body: &str,
    block: Range,
    caret: usize,
    inlines: &[InlineNode],
    container_code: &[(usize, usize)],
) -> Vec<Sub> {
    if node_active(block, caret) {
        return Vec::new();
    }
    let mut code_spans = container_code.to_vec();
    collect_code_spans(inlines, block, &mut code_spans);
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut line_start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'\n' {
            continue;
        }
        if i > line_start && bytes[i - 1] == b'\\' {
            let mut n = 0usize;
            let mut j = i - 1;
            loop {
                n += 1;
                if j == line_start || bytes[j - 1] != b'\\' {
                    break;
                }
                j -= 1;
            }
            let in_code = code_spans.iter().any(|&(s, e)| j >= s && j < e);
            if n % 2 == 1 && !in_code {
                out.push(Sub::elide(j, j + 1));
            }
        }
        line_start = i + 1;
    }
    out
}

/// 块级替换（切片 12）：任务列表复选框 `☐`/`☑`、无序列表符号 `•`、
/// 引用行首 `>` → 空格、分隔线整行消隐；标题 `#` 与围栏 ` ``` ` 同规则消隐（B7）。
/// 激活判据沿用块区间闭区间；引用块逐行判定（光标所在行显源码，其余行隐标记）。
fn block_subs(
    kind: BlockKind,
    body: &str,
    block: Range,
    caret: usize,
    ordinal: Option<u32>,
    toc_list: bool,
) -> Vec<Sub> {
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
        BlockKind::ListItem {
            ordered: true,
            depth,
            task: None,
        } => {
            // 激活（含块首闭区间）→ 显源码编号，用户可直接改写 Markdown。
            if node_active(block, caret) {
                return Vec::new();
            }
            // 续块（无标记）不编号：其缩进必然大于标记列（与 list_marker_of 同一判据）。
            let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
            if indent > usize::from(depth) * 2 + 1 {
                return Vec::new();
            }
            // 编号由文档块顺序派生（BlockKind 不含起始号）；拿不到顺序的调用方
            //（单块排版/打印）保留源码，避免把后续项错误地重置成 1。
            let Some((start, end, _, delim)) = ordered_marker(body) else {
                return Vec::new();
            };
            // 旧版目录列表：条目文本已自带层级编号，标记替换为等宽空格，
            // 只保留源码缩进层级，不再叠加有序序号。
            if toc_list {
                return vec![Sub::fill(start, end, " ".repeat(end - start))];
            }
            let Some(n) = ordinal else {
                return Vec::new();
            };
            // 编号变宽（9→10）时补前导空格到源码标记宽度，保持内容列不变；
            // 行高只由行数决定，与编号宽度无关（与「标记消隐不改行高」同一约束）。
            let digits = (end - start) - 1;
            let number = n.to_string();
            let mut fill = String::new();
            for _ in number.len()..digits {
                fill.push(' ');
            }
            fill.push_str(&number);
            fill.push(delim as char);
            vec![Sub::fill(start, end, fill)]
        }
        BlockKind::ListItem { ordered: false, .. } => {
            if node_active(block, caret) {
                return Vec::new();
            }
            match bullet_pos(body) {
                Some(p) if toc_list => vec![Sub::fill(p, p + 1, " ")],
                Some(p) => vec![Sub::fill(p, p + 1, "•")],
                None => Vec::new(),
            }
        }
        BlockKind::BlockQuote => quote_subs(body, block, caret),
        BlockKind::Callout(c) => callout_subs(&c, body, block, caret),
        BlockKind::Admonition(ty) => admonition_subs(ty, body, block, caret),
        BlockKind::ThematicBreak => {
            if node_active(block, caret) {
                Vec::new()
            } else {
                vec![Sub::elide(0, body.len())]
            }
        }
        // 标题/围栏：非激活时隐源码标记（与列表/引用/分隔线同一 WYSIWYM 规则）。
        BlockKind::AtxHeading(level) => heading_subs(level, body, block, caret),
        BlockKind::SetextHeading(_) => setext_subs(body, block, caret),
        BlockKind::FencedCode(lang) => fenced_subs(body, block, caret, lang.as_deref()),
        // Mermaid 独立块类型：badge 渲染复用 fenced_subs 的 mermaid 分支逻辑
        BlockKind::Mermaid => fenced_subs(body, block, caret, Some("mermaid")),
        // 折叠块：标签由 inline 消隐机制去掉后，再压掉标签行的换行与块内 Markdown
        // 结构标记，让 summary / 正文以可见文本（段落）形式完整显示。
        BlockKind::Details { .. } => details_subs(body),
        _ => Vec::new(),
    }
}

/// ATX 标题：非激活时消隐缩进(≤3) + `#`×1..=6 + 其后空白。
/// 行内文本与行高不变（只去掉前缀字形），光标进入块即还原源码。
/// H4–H6 同字号，标记区改为缩进空格（2 个半角空格/级）以保留层级可辨；
/// H1–H3 仍有字号差异，保持纯消隐。
fn heading_subs(level: u8, body: &str, block: Range, caret: usize) -> Vec<Sub> {
    if node_active(block, caret) {
        return Vec::new();
    }
    let Some(end) = atx_marker_end(body).filter(|&end| end > 0) else {
        return Vec::new();
    };
    if level >= 4 {
        let indent = usize::from(level - 3) * 2;
        return vec![Sub::fill(0, end, " ".repeat(indent))];
    }
    vec![Sub::elide(0, end)]
}

/// Setext 标题：非激活时只把末行下划线（`===` / `---`）换成等长空格。
/// 不用消隐：末行纯空时排版会丢行高（光标进出会跳），空格占位保持行高与行宽。
/// 内容可跨多行（CommonMark：段落全部行 + 下划线行构成标题），除末行外的
/// 内容行一律保留，不能按「首行之后全换成空格」处理，否则中间行整段不可见。
fn setext_subs(body: &str, block: Range, caret: usize) -> Vec<Sub> {
    if node_active(block, caret) {
        return Vec::new();
    }
    let trimmed = body.trim_end_matches(['\r', '\n']);
    let Some(start) = trimmed.rfind('\n').map(|i| i + 1) else {
        return Vec::new();
    };
    if start >= body.len() {
        return Vec::new();
    }
    let n = body[start..].chars().count().max(1);
    vec![Sub::fill(start, body.len(), " ".repeat(n))]
}

/// `<details>` **头块**渲染（B3 起头块只含开标签行 + summary 行）：
/// 1) 显式消隐 `<details>` / `<summary>` / `</summary>` 裸标签（header 内的
///    `<details>` 未闭合，行内解析退化为 HtmlLiteral，不参与 collect_elide）；
/// 2) 消隐头块内全部换行，让 summary 压成单行（折叠态高度 = 一行；
///    summary 与正文已拆成兄弟块，不再需要逐行清理 Markdown 结构标记）。
///    光标进入头块或右键落光标时不恢复标签行，避免折叠几何被隐藏正文重新撑开。
fn details_subs(body: &str) -> Vec<Sub> {
    let mut out = Vec::new();
    elide_details_tag(body, "<details", &mut out);
    elide_details_tag(body, "<summary", &mut out);
    elide_details_tag(body, "</summary>", &mut out);
    for (i, b) in body.bytes().enumerate() {
        if b == b'\n' {
            out.push(Sub::elide(i, i + 1));
        }
    }
    out
}

/// 消隐 details 头块中的裸标签（`<details ...>` / `<summary ...>` / `</summary>`）。
fn elide_details_tag(body: &str, needle: &str, out: &mut Vec<Sub>) {
    let lower = body.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(i) = lower[from..].find(needle) {
        let start = from + i;
        let Some(rel) = body[start..].find('>') else {
            break;
        };
        let end = start + rel + 1;
        out.push(Sub::elide(start, end));
        from = end;
    }
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
/// mermaid 块特殊处理：开围栏行替换为 `◈ <类型徽标>` 卡片头（调用 `veil_core::detect_type`
/// 识别 body 类型并映射到中文徽标）；空或只有 `%%` 注释的块在头下方插入一行提示。
/// 源码模式下外层不会进入本函数（`reveal_all` 短路），天然隔离。
fn fenced_subs(body: &str, block: Range, caret: usize, lang: Option<&str>) -> Vec<Sub> {
    if node_active(block, caret) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let first_end = body.find('\n').unwrap_or(body.len());
    if first_end > 0 {
        if lang.is_some_and(|l| l.eq_ignore_ascii_case("mermaid")) {
            // 去掉开/闭围栏行，拿到真正的源码 body 供 detect_type 识别
            let last_start = body.rfind('\n').map(|nl| nl + 1).unwrap_or(body.len());
            let closed = last_start > first_end && fence_line(&body[last_start..]);
            let body_mid_full = &body[first_end + 1..];
            let body_mid = if closed {
                // body_mid_full 末尾带闭合围栏行，按它在 body 中的起点切
                let fence_in_mid = last_start - (first_end + 1);
                &body_mid_full[..fence_in_mid]
            } else {
                body_mid_full
            };
            let badge = match veil_core::detect_type(body_mid) {
                Some("flowchart") => Key::MermaidFlowchart,
                Some("sequence") => Key::MermaidSequence,
                Some("class") => Key::MermaidClass,
                Some("state") => Key::MermaidState,
                Some("er") => Key::MermaidEr,
                Some("gantt") => Key::MermaidGantt,
                Some("pie") => Key::MermaidPie,
                _ => Key::MermaidUnknown,
            };
            out.push(Sub::fill(0, first_end, tf(Key::MermaidBadge, &[t(badge)])));
            // 空块提示：所有非空行都是 %% 注释 → 提示用户插入模板
            let is_empty = body_mid
                .lines()
                .map(|l| l.trim())
                .all(|l| l.is_empty() || l.starts_with("%%"));
            if is_empty {
                let insert_pos = first_end + 1;
                if insert_pos <= body.len() {
                    out.push(Sub::insert(insert_pos, t(Key::MermaidEmptyHint)));
                }
            }
        } else {
            out.push(Sub::elide(0, first_end));
        }
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
    (ch == b'`' || ch == b'~') && rest.as_bytes().iter().take_while(|&&b| b == ch).count() >= 3
}

/// 无序列表符号位置（块相对字节）：缩进后首个 `-` / `*` / `+`。
fn bullet_pos(body: &str) -> Option<usize> {
    let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
    match body.as_bytes().get(indent) {
        Some(b'-') | Some(b'*') | Some(b'+') => Some(indent),
        _ => None,
    }
}

/// 有序列表标记（块相对字节）：`(数字起点, 标记终点, 源码起始编号, 分隔符)`。
/// 只认块首行行首的数字 + `.`/`)`；不含尾随空白（由调用方决定是否保留）。
fn ordered_marker(body: &str) -> Option<(usize, usize, u32, u8)> {
    let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
    let bytes = body.as_bytes();
    let digits = bytes[indent..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    // 与解析层一致：1–9 位数字（10 位不再视为列表标记）
    if digits == 0 || digits > 9 {
        return None;
    }
    let delim = *bytes.get(indent + digits)?;
    if delim != b'.' && delim != b')' {
        return None;
    }
    let start = body[indent..indent + digits].parse().ok()?;
    Some((indent, indent + digits + 1, start, delim))
}

/// 列表项首行标记种类：供编号派生区分「新项」与「无标记续块」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListMarker {
    /// 无标记的列表续块（项内内容行）：不参与编号，也不打断父层计数。
    None,
    /// 无序标记（`-` / `*` / `+`）。
    Bullet,
    /// 有序标记，携带源码起始编号。
    Ordered(u32),
}

/// 列表项首行标记识别（块文本 → 标记种类）。
///
/// 用 `depth` 排除续块误判：续块内容缩进 = 项内容列，必然大于标记列
///（`depth * 2`，允许 1 列零头），因此「内容行恰好以 `2024. ` 开头」不会被当作新项。
pub(crate) fn list_marker_of(kind: &BlockKind, body: &str) -> ListMarker {
    let BlockKind::ListItem { ordered, depth, .. } = kind else {
        return ListMarker::None;
    };
    let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
    if indent > usize::from(*depth) * 2 + 1 {
        return ListMarker::None;
    }
    if *ordered {
        ordered_marker(body).map_or(ListMarker::None, |(_, _, n, _)| ListMarker::Ordered(n))
    } else if bullet_pos(body).is_some() {
        ListMarker::Bullet
    } else {
        ListMarker::None
    }
}

/// 列表项可见内容起点（标记之后，块内字节偏移）；非列表项、续块与任务项返回 None。
/// 旧版目录列表识别用：只有「标记之后紧接一个整块链接」的项才算目录条目，
/// 任务列表（`- [ ] …`）不参与。
pub(crate) fn list_content_start(kind: &BlockKind, body: &str) -> Option<usize> {
    let BlockKind::ListItem {
        ordered,
        depth,
        task,
    } = kind
    else {
        return None;
    };
    if task.is_some() {
        return None;
    }
    let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
    if indent > usize::from(*depth) * 2 + 1 {
        return None;
    }
    if *ordered {
        ordered_marker(body).map(|(_, end, _, _)| end)
    } else {
        bullet_pos(body).map(|p| p + 1)
    }
}

/// 有序列表显示编号派生（BlockId → 非激活态显示的编号）。
///
/// `BlockKind::ListItem` 不含源码起始号，排版层又必须按语义递增显示，故编号只能在
/// 文档块序列上派生。算法维护「当前各层列表」栈：
/// - 遇到更浅的层级先弹栈：子列表结束后父层继续递增；
/// - 同层同序型视为同一列表，编号 = 计数器并递增（源码写 `1.` 也显示 2./3.）；
/// - 同层换序型、或列表被非列表块打断 → 新列表，从源码首个编号重新起算；
/// - 无标记续块直接跳过（内容属于当前项，不改计数）。
pub(crate) fn ordered_list_ordinals<'a>(
    items: impl Iterator<Item = (BlockId, &'a BlockKind, ListMarker)>,
) -> HashMap<BlockId, u32> {
    struct Level {
        depth: u8,
        ordered: bool,
        next: u32,
    }
    let mut stack: Vec<Level> = Vec::new();
    let mut out = HashMap::new();
    for (id, kind, marker) in items {
        let BlockKind::ListItem { ordered, depth, .. } = kind else {
            // 非列表块结束全部列表：下一个列表必须重新从源码编号起算。
            stack.clear();
            continue;
        };
        if marker == ListMarker::None {
            continue;
        }
        let d = *depth;
        while stack.last().is_some_and(|l| l.depth > d) {
            stack.pop();
        }
        // 同层换了序型 → 新列表；同层同序型 → 同一列表继续。
        if stack
            .last()
            .is_some_and(|l| l.depth == d && l.ordered != *ordered)
        {
            stack.pop();
        }
        if !stack.last().is_some_and(|l| l.depth == d) {
            let start = match marker {
                ListMarker::Ordered(n) => n,
                _ => 1,
            };
            stack.push(Level {
                depth: d,
                ordered: *ordered,
                next: start,
            });
        }
        let level = stack.last_mut().expect("上方刚保证栈顶存在");
        if *ordered {
            out.insert(id, level.next);
            level.next = level.next.saturating_add(1);
        }
    }
    out
}

/// 有序列表编号代次：任一 (块, 编号) 变化都改变结果。
///
/// 编号是全文派生量——文首插入一项会让后续块的显示编号改变，而块文本与 BlockId
/// 都可以保持不变（增量重解析只平移）。不带此代次时布局缓存会命中陈旧编号
/// （与脚注编号代次同一因果）。按条目异或折叠，与 HashMap 迭代顺序无关。
pub(crate) fn ordered_list_epoch(ordinals: &HashMap<BlockId, u32>) -> u64 {
    let mut acc = 0u64;
    for (id, n) in ordinals {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        h = (h ^ id.0).wrapping_mul(0x0100_0000_01b3);
        h = (h ^ u64::from(*n)).wrapping_mul(0x0100_0000_01b3);
        acc ^= h.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    acc
}

/// 旧版目录列表集合代次：块进出集合会改变「列表标记是否显示」，
/// 与编号同属全文派生量，必须进入布局基础键（否则邻居变更后命中陈旧布局）。
pub(crate) fn toc_list_epoch(items: &HashSet<BlockId>) -> u64 {
    let mut acc = 0u64;
    for id in items {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        h = (h ^ id.0).wrapping_mul(0x0100_0000_01b3);
        acc ^= h.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    acc
}

/// 引用块逐行替换：非激活行的所有层级 `>` 换成空格（保留缩进与行距），
/// 激活行显源码。REN-SWEEP-004：嵌套引用 `> > …` 的每一级标记都要消隐，
/// 级间原有空白自然形成层级缩进（引用条由 paint 按层级补画）。
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
            let mut search = 0usize;
            while let Some(p) = quote_marker_pos(&line[search..]) {
                let abs = search + p;
                out.push(Sub::fill(off + abs, off + abs + 1, " "));
                search = abs + 1;
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

/// 普通引用块：逐行消隐 `>`；首行若形如 `[!XXX]`（未配置的 Callout 类型）也一并
/// 消隐——未知类型回退普通引用块时不得露出裸标记文本。
fn quote_subs(body: &str, block: Range, caret: usize) -> Vec<Sub> {
    let mut out = quote_line_subs(body, block, caret);
    if let Some((start, end)) = admonition_marker(body) {
        if !line_active(body, block, caret, 0) {
            out.push(Sub::fill(start, end, ""));
        }
    }
    out
}

/// 自定义 Callout：沿用引用块的 `>` 消隐，首行 `[!NAME]` 替换为配置标签。
fn callout_subs(c: &veil_core::Callout, body: &str, block: Range, caret: usize) -> Vec<Sub> {
    let mut out = quote_line_subs(body, block, caret);
    if line_active(body, block, caret, 0) {
        return out;
    }
    if let Some((start, end)) = admonition_marker(body) {
        out.push(Sub::fill(start, end, &c.label));
    }
    out
}

/// caret 是否落在块内第 `line_idx` 行（含行尾换行前）。
fn line_active(body: &str, block: Range, caret: usize, line_idx: usize) -> bool {
    if !(block.start <= caret && caret <= block.end) {
        return false;
    }
    let rel = caret - block.start;
    let mut off = 0usize;
    for (i, line) in body.split_inclusive('\n').enumerate() {
        let visible_end = off + line.trim_end_matches('\n').len();
        if i == line_idx {
            return off <= rel && rel <= visible_end;
        }
        off += line.len();
    }
    false
}

/// 容器内嵌套块标记的递归排版（REN-SWEEP-002）。
///
/// 引用/提示/自定义 Callout/列表项在块解析里仍是「一个容器块」，这里按行剥掉
/// 容器前缀后**递归识别嵌套块语法**，把标记替换成对应块类型的视觉形式：
/// - 无序列表 → `•`；任务列表 → `☐`/`☑`（原缩进保留为层级）
/// - 围栏代码 → 消隐围栏行（含换行），内容行登记为等宽代码区间
/// - `<details>`/`<summary>` 裸标签 → 消隐，正文按普通段落行内语法排版
/// - 独立 `<hr>` 行 → 整行消隐（与块级分隔线一致）
/// - 嵌套 `>` → 由 [`quote_line_subs`] 逐级消隐
///
/// 返回 `(替换段, 代码样式区间)`，均为块相对 doc 坐标。激活行（WYSIWYM）显源码。
fn container_nested_subs(
    kind: &BlockKind,
    body: &str,
    block: Range,
    caret: usize,
) -> (Vec<Sub>, Vec<(usize, usize)>) {
    let quote_like = matches!(
        kind,
        BlockKind::BlockQuote | BlockKind::Callout(_) | BlockKind::Admonition(_)
    );
    let list_like = matches!(kind, BlockKind::ListItem { .. });
    if !quote_like && !list_like {
        return (Vec::new(), Vec::new());
    }
    let block_active = node_active(block, caret);
    let mut subs = Vec::new();
    let mut code = Vec::new();
    let mut off = 0usize;
    let mut in_fence: Option<u8> = None;
    for (idx, line) in body.split_inclusive('\n').enumerate() {
        let visible_end = off + line.trim_end_matches('\n').len();
        let active = if quote_like {
            line_active(body, block, caret, idx)
        } else {
            block_active
        };
        // 列表项首行标记由 `block_subs` 处理，避免重复替换。
        if list_like && idx == 0 {
            off += line.len();
            continue;
        }
        // 内容起点：引用类跳过全部 `>` 标记（已被 quote_line_subs 换成空格）；
        // 列表续行同样剥掉嵌套引用标记（列表项没有 quote_line_subs），
        // 缩进只影响视觉层级，不参与后续语法识别。
        let content_off = if quote_like {
            let mut s = 0usize;
            while let Some(p) = quote_marker_pos(&line[s..]) {
                s += p + 1;
            }
            s
        } else {
            let mut s = line.len() - line.trim_start_matches([' ', '\t']).len();
            if in_fence.is_none() {
                while let Some(p) = quote_marker_pos(&line[s..]) {
                    if !active {
                        subs.push(Sub::fill(off + s + p, off + s + p + 1, " "));
                    }
                    s += p + 1;
                }
            }
            s
        };
        let content = line.get(content_off..).unwrap_or("");
        if let Some(ch) = in_fence {
            if is_fence_close(content, ch) {
                in_fence = None;
                if !active {
                    subs.push(Sub::elide(off, off + line.len()));
                }
            } else if !active {
                code.push((off + content_off, visible_end));
            }
        } else if let Some(ch) = fence_open_char(content) {
            in_fence = Some(ch);
            if !active {
                subs.push(Sub::elide(off, off + line.len()));
            }
        } else if !active {
            if is_html_hr_content(content) {
                // 嵌套分隔线：整行消隐（paint 只在块级画分隔线）。
                subs.push(Sub::elide(off, off + line.len()));
            } else {
                elide_nested_details_tags(line, off, &mut subs);
                nested_list_marker_subs(content, off + content_off, &mut subs);
            }
        }
        off += line.len();
    }
    (subs, code)
}

/// 围栏行（≤3 空格缩进）返回围栏字符，用于进入嵌套代码态。
fn fence_open_char(line: &str) -> Option<u8> {
    if !fence_line(line) {
        return None;
    }
    line.trim_start_matches([' ', '\t'])
        .as_bytes()
        .first()
        .copied()
        .filter(|b| *b == b'`' || *b == b'~')
}

/// 嵌套围栏闭合行：同字符、≥3 个、其后只有空白。
fn is_fence_close(line: &str, ch: u8) -> bool {
    let rest = line.trim_start_matches([' ', '\t']);
    let run = rest.as_bytes().iter().take_while(|&&b| b == ch).count();
    run >= 3 && rest[run..].trim().is_empty()
}

/// 独立成行的 `<hr>` / `<hr/>`（大小写不敏感、仅空白围绕）。
fn is_html_hr_content(content: &str) -> bool {
    let lower = content.trim().to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("<hr") else {
        return false;
    };
    let Some(rest) = rest.trim().strip_suffix('>') else {
        return false;
    };
    let rest = rest.trim();
    rest.strip_suffix('/').unwrap_or(rest).trim().is_empty()
}

/// 消隐容器行内的 `<details>` / `</details>` / `<summary>` / `</summary>` 裸标签。
fn elide_nested_details_tags(line: &str, off: usize, out: &mut Vec<Sub>) {
    let lower = line.to_ascii_lowercase();
    for needle in ["</details>", "</summary>", "<details", "<summary"] {
        let mut from = 0usize;
        while let Some(i) = lower[from..].find(needle) {
            let start = from + i;
            let Some(rel) = line[start..].find('>') else {
                break;
            };
            let end = start + rel + 1;
            out.push(Sub::elide(off + start, off + end));
            from = end;
        }
    }
}

/// 容器内嵌套列表标记：任务框 → ☐/☑，无序 → `•`；有序保留源码编号。
fn nested_list_marker_subs(content: &str, base: usize, out: &mut Vec<Sub>) {
    if let Some((start, _, end, done)) = task_span(content) {
        out.push(Sub::fill(
            base + start,
            base + end,
            if done { "☑ " } else { "☐ " },
        ));
        return;
    }
    if let Some(p) = bullet_pos(content) {
        out.push(Sub::fill(base + p, base + p + 1, "•"));
    }
}

/// Admonition 提示块：沿用引用块的逐行 `>` 消隐，并把首行 `[!TYPE]` 标记
/// 整体替换为可读标签（`Note`/`Tip`/…）。首行激活时保留源码，便于编辑标记。
fn admonition_subs(
    ty: veil_core::AdmonitionType,
    body: &str,
    block: Range,
    caret: usize,
) -> Vec<Sub> {
    let mut out = quote_line_subs(body, block, caret);
    // 首行激活（caret 落在首行内）→ 显示源码标记，不做替换
    if line_active(body, block, caret, 0) {
        return out;
    }
    if let Some((start, end)) = admonition_marker(body) {
        out.push(Sub::fill(start, end, admonition_label(ty)));
    }
    out
}

/// 内置 Callout（GFM Admonition）标签：词条键化后与旧 `AdmonitionType::label()` 逐字一致。
fn admonition_label(ty: veil_core::AdmonitionType) -> &'static str {
    use veil_core::AdmonitionType;
    t(match ty {
        AdmonitionType::Note => Key::CalloutNote,
        AdmonitionType::Tip => Key::CalloutTip,
        AdmonitionType::Important => Key::CalloutImportant,
        AdmonitionType::Warning => Key::CalloutWarning,
        AdmonitionType::Caution => Key::CalloutCaution,
    })
}

/// 首行 `> [!TYPE]` 标记的块相对字节区间 `[start, end)`。
/// 返回 `end` = `]` 之后一字节；标签内容不校验（类由调用方给定）。
fn admonition_marker(body: &str) -> Option<(usize, usize)> {
    let line = body.split_inclusive('\n').next()?;
    let b = line.as_bytes();
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    if b.get(indent) != Some(&b'>') {
        return None;
    }
    let mut i = indent + 1;
    while matches!(b.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if b.get(i) != Some(&b'[') || b.get(i + 1) != Some(&b'!') {
        return None;
    }
    let close = line[i..].find(']')? + i;
    Some((i, close + 1))
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
    let mut subs: Vec<Sub> = elide.into_iter().map(|(s, e)| Sub::elide(s, e)).collect();
    collect_q_subs(inlines, cell_range, usize::MAX, &mut subs);
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

/// 单元格内 Markdown 图片 alt 的可见文本区间（与 `cell.text` 同坐标，按 inline 先序；
/// alt 为空的节点给零宽区间）。图片成功加载时视图层据此从显示文本中剔除 alt
/// （只留缩略图），失败时保留原文（REN-SWEEP-008）。
/// 映射不可用（与 `cell_style_spans` 同口径）时返回 None，调用方保留原文。
pub(crate) fn cell_image_alt_spans(
    cell_range: Range,
    text_len: usize,
    inlines: &[InlineNode],
) -> Option<Vec<Range>> {
    let mut elide: Vec<(usize, usize)> = Vec::new();
    collect_elide(inlines, cell_range, usize::MAX, &mut elide);
    let mut subs: Vec<Sub> = elide.into_iter().map(|(s, e)| Sub::elide(s, e)).collect();
    collect_q_subs(inlines, cell_range, usize::MAX, &mut subs);
    let (map, _) = OffsetMap::from_spans(cell_range.len(), &subs);
    if map.layout_len != text_len {
        return None;
    }
    fn walk(nodes: &[InlineNode], cell_range: Range, map: &OffsetMap, out: &mut Vec<Range>) {
        for n in nodes {
            if n.kind == InlineKind::Image {
                let s = map.doc_to_layout(n.content.start.saturating_sub(cell_range.start));
                let e = map.doc_to_layout(n.content.end.saturating_sub(cell_range.start));
                out.push(Range::new(s, e.max(s)));
            }
            walk(&n.children, cell_range, map, out);
        }
    }
    let mut out = Vec::new();
    walk(inlines, cell_range, &map, &mut out);
    Some(out)
}

/// 单元格内 Math 节点的可见文本区间：返回 `(可见起点, 可见终点, TeX 内容区间)`。
/// 与 `cell_style_spans` 共用 `collect_elide` + `OffsetMap`，坐标口径与 `cell.text` 一致
/// （`$x^2$` → `x^2`）。字节映射不可用（如 `\|` 转义改写长度）时返回 None，
/// 调用方降级为纯文本、不画公式图。
pub(crate) fn cell_math_spans(
    cell_range: Range,
    text_len: usize,
    inlines: &[InlineNode],
) -> Option<Vec<(usize, usize, Range)>> {
    let mut elide: Vec<(usize, usize)> = Vec::new();
    collect_elide(inlines, cell_range, usize::MAX, &mut elide);
    let mut subs: Vec<Sub> = elide.into_iter().map(|(s, e)| Sub::elide(s, e)).collect();
    collect_q_subs(inlines, cell_range, usize::MAX, &mut subs);
    let (map, _) = OffsetMap::from_spans(cell_range.len(), &subs);
    if map.layout_len != text_len {
        return None;
    }
    fn walk(
        nodes: &[InlineNode],
        cell_range: Range,
        map: &OffsetMap,
        out: &mut Vec<(usize, usize, Range)>,
    ) {
        for n in nodes {
            if n.kind == InlineKind::Math {
                let s = map.doc_to_layout(n.range.start.saturating_sub(cell_range.start));
                let e = map.doc_to_layout(n.range.end.saturating_sub(cell_range.start));
                if s < e {
                    out.push((s, e, n.content));
                }
            }
            walk(&n.children, cell_range, map, out);
        }
    }
    let mut out = Vec::new();
    walk(inlines, cell_range, &map, &mut out);
    Some(out)
}

pub(super) fn attrs_of(
    kind: BlockKind,
    st: Style,
    link_color: [u8; 3],
    doc_font: DocFontFamily,
    mono_family: Option<&'static str>,
    pt: f32,
) -> Attrs<'static> {
    let a = Attrs::new();
    let a = if st.bold || matches!(kind, BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_)) {
        a.weight(Weight::BOLD)
    } else {
        a
    };
    let a = if st.italic {
        a.style(FontStyle::Italic)
    } else {
        a
    };
    // 代码/行内代码恒用等宽族；其余正文/表格文本按配置的字体族（system 不设）。
    // 显式族名优先：generic Monospace 会先用尽等宽候选，旧 CJK 等宽字体（MS Gothic 等）
    // 会截走 Han 造成与正文不同族；显式族名下 Han 才走脚本回退（N9）。
    let a = if st.mono
        || matches!(
            kind,
            BlockKind::FencedCode(_)
                | BlockKind::IndentedCode
                | BlockKind::HtmlBlock
                | BlockKind::FrontMatter(_)
                | BlockKind::Mermaid
        ) {
        match mono_family {
            Some(name) => a.family(Family::Name(name)),
            None => a.family(Family::Monospace),
        }
    } else if let Some(family) = doc_family(doc_font) {
        a.family(family)
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
    // Sub/Sup 缩字：覆写 span 的 Metrics（font_size × factor），cosmic-text 自动覆写排版字号。
    // line_height 按同比例缩放，保证行内嵌套 Sub/Sup 不破坏行距。
    // 正文字号通常 14pt，line_height ≈ 21pt（1.5×）；这里用 1.5 默认比例保持一致。
    let a = if let Some(factor) = st.size_factor {
        let scaled_font = pt * factor;
        let scaled_lh = pt * 1.5 * factor;
        a.metrics(Metrics::new(scaled_font, scaled_lh))
    } else {
        a
    };
    a
}

#[cfg(test)]
mod typography_tests;

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

    /// 按文档块序号排版（容器/嵌套回归用）：`caret` 为文档绝对偏移。
    fn layout_doc_block(
        text: &str,
        block_idx: usize,
        caret: usize,
        width: f32,
    ) -> (LayoutEngine, BlockLayout) {
        let s = Session::open(text);
        let (id, kind, range) = s.blocks()[block_idx].clone();
        let inlines = s.block_inlines(id).unwrap();
        let mut eng = LayoutEngine::new();
        let block_text = &text[range.start..range.end];
        let l = eng.layout_block(kind, block_text, range, &inlines, width, caret, &[], false);
        (eng, l)
    }

    /// captions 为独立成行图片的题注输入（alt 非空时由 shell 派生）。
    fn layout_with_captions(
        text: &str,
        caret: usize,
        width: f32,
        imgs: &[InlineImg],
        captions: &[ImageCaption],
    ) -> (LayoutEngine, BlockLayout) {
        let s = Session::open(text);
        let (id, kind, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap();
        let mut eng = LayoutEngine::new();
        let block_text = &text[range.start..range.end];
        let l = eng.layout_block_with_captions(
            kind,
            block_text,
            range,
            &inlines,
            width,
            caret,
            imgs,
            false,
            None,
            &[],
            None,
            false,
            captions,
        );
        (eng, l)
    }

    fn caption(range: Range, text: &str) -> ImageCaption {
        ImageCaption {
            range,
            text: text.to_string(),
        }
    }

    /// 整篇逐块排版：按 Session 块顺序派生有序列表编号后排版每个块。
    /// 单块辅助函数拿不到文档顺序，无法验证跨块递增编号。
    fn layout_blocks_with_caret(
        src: &str,
        caret: usize,
        width: f32,
    ) -> (LayoutEngine, Vec<BlockLayout>) {
        let s = Session::open(src);
        let blocks = s.blocks();
        let items = blocks.iter().map(|(id, kind, range)| {
            let marker = match kind {
                BlockKind::ListItem { .. } => list_marker_of(kind, &src[range.start..range.end]),
                _ => ListMarker::None,
            };
            (*id, kind, marker)
        });
        let ordinals = ordered_list_ordinals(items);
        let mut eng = LayoutEngine::new();
        let layouts = blocks
            .iter()
            .map(|(id, kind, range)| {
                let inlines = s.block_inlines(*id).unwrap_or_default();
                eng.layout_block_with_footnotes(
                    kind.clone(),
                    &src[range.start..range.end],
                    *range,
                    &inlines,
                    width,
                    caret,
                    &[],
                    false,
                    None,
                    &[],
                    ordinals.get(id).copied(),
                    false,
                )
            })
            .collect();
        (eng, layouts)
    }

    /// 拼回整篇排版文本（块间以 `\n` 连接），便于断言跨块编号。
    fn block_texts(layouts: &[BlockLayout]) -> String {
        layouts
            .iter()
            .map(|l| l.layout_text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn hit_then_caret_roundtrips_ascii() {
        let (eng, l, ..) = layout_of("hello world", 200.0);
        let (x, y, _) = eng.caret_xy(&l, 5);
        assert_eq!(eng.hit(&l, x + 0.1, y + 1.0), 5, "光标 x 命中应回原偏移");
    }

    #[test]
    fn pick_family_prefers_first_available_and_falls_back() {
        let sans = typography::SANS_FAMILIES;
        assert_eq!(
            pick_family(sans, |n| n == "Microsoft YaHei UI"),
            Some("Microsoft YaHei UI")
        );
        assert_eq!(pick_family(sans, |n| n == "Segoe UI"), Some("Segoe UI"));
        assert_eq!(pick_family(sans, |_| false), None);
    }

    #[test]
    fn ui_text_is_smaller_than_body_preedit() {
        let mut eng = LayoutEngine::new();
        let ui = eng.layout_ui("Aa中");
        let body = eng.layout_preedit("Aa中");
        assert!((ui.height - typography::UI_LINE_HEIGHT).abs() < 0.5);
        assert!((body.height - typography::BODY_LINE_HEIGHT).abs() < 0.5);
        assert!(ui.height < body.height);
    }

    #[test]
    fn code_block_is_smaller_than_paragraph() {
        let mut eng = LayoutEngine::new();
        let text = "let x = 1;";
        let block = Range {
            start: 0,
            end: text.len(),
        };
        let cod = eng.layout_block(
            BlockKind::FencedCode(None),
            text,
            block,
            &[],
            500.0,
            usize::MAX,
            &[],
            false,
        );
        let par = eng.layout_block(
            BlockKind::Paragraph,
            text,
            block,
            &[],
            500.0,
            usize::MAX,
            &[],
            false,
        );
        let expected = typography::BODY_LINE_HEIGHT * typography::CODE_SCALE;
        assert!((cod.height - expected).abs() < 0.5);
        assert!(cod.height < par.height);
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
            let link_attrs = attrs_of(
                kind.clone(),
                style_at(link_at),
                theme.link,
                DocFontFamily::System,
                None,
                12.0,
            );
            assert_eq!(
                link_attrs.color_opt,
                Some(want),
                "链接字符应使用主题链接色: dark={}",
                theme.dark
            );
            let body_attrs = attrs_of(
                kind.clone(),
                style_at(body_at),
                theme.link,
                DocFontFamily::System,
                None,
                12.0,
            );
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
    fn doc_typography_scales_body_but_leaves_ui_untouched() {
        let text = "正文一行";
        let layout = |engine: &mut LayoutEngine| {
            engine
                .layout_block(
                    BlockKind::Paragraph,
                    text,
                    Range::new(0, text.len()),
                    &[],
                    400.0,
                    usize::MAX,
                    &[],
                    false,
                )
                .height
        };
        let mut def = LayoutEngine::new();
        let def_h = layout(&mut def);
        let ui_def = def.measure("菜单");
        let mut big = LayoutEngine::new();
        big.set_doc_typography(18.0, 27.0, DocFontFamily::Serif);
        let big_h = layout(&mut big);
        let ui_big = big.measure("菜单");
        assert!(
            big_h > def_h,
            "18px 正文块应高于默认 16px：{big_h} vs {def_h}"
        );
        assert!(big_h >= 27.0 - 0.5, "块高应使用配置行高：{big_h}");
        assert_eq!(ui_big, ui_def, "UI 文本度量不受正文排版配置影响");
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
    fn list_bullet_hidden_outside_caret_renumbers_ordered() {
        // 无序标记 [0,1) 非激活 → `•`（与 strong 消隐叠加：「- **a**」→「• a」）
        let (_, l) = layout_with_caret("- **a**", usize::MAX, 200.0);
        assert_eq!(l.layout_text(), "• a");
        // 激活（闭区间含块首）→ 列表标记显源码；strong 未激活仍消隐
        let (_, l) = layout_with_caret("- **a**", 0, 200.0);
        assert_eq!(l.layout_text(), "- a");
        // 光标进 strong 内容 → 列表与 inline 标记均显源码
        let (_, l) = layout_with_caret("- **a**", 4, 200.0);
        assert_eq!(l.layout_text(), "- **a**");
        // 单块调用拿不到文档顺序（不传 ordinal）→ 有序列表保留源码编号
        let (_, l) = layout_with_caret("1. a", usize::MAX, 200.0);
        assert_eq!(l.layout_text(), "1. a");
    }

    #[test]
    fn legacy_toc_list_hides_list_markers_but_keeps_indent() {
        // 旧目录列表：条目文本自带层级编号，标记换成等宽空格（不叠加 1./2.）。
        let src = "1. [第一章 介绍](#第一章 - 介绍)\n   1. [1.1 背景](#11- 背景)";
        let s = Session::open(src);
        let blocks = s.blocks();
        let mut eng = LayoutEngine::new();
        let render = |eng: &mut LayoutEngine, caret: usize| {
            blocks
                .iter()
                .map(|(id, kind, range)| {
                    let inlines = s.block_inlines(*id).unwrap_or_default();
                    eng.layout_block_with_footnotes(
                        kind.clone(),
                        &src[range.start..range.end],
                        *range,
                        &inlines,
                        400.0,
                        caret,
                        &[],
                        false,
                        None,
                        &[],
                        Some(1),
                        true,
                    )
                    .layout_text()
                    .to_string()
                })
                .collect::<Vec<_>>()
        };
        // CJK 前会自动补一个中西文间距（既有排版规则），故首项为 4 空格。
        assert_eq!(
            render(&mut eng, usize::MAX),
            ["    第一章 介绍", "      1.1 背景"]
        );
        // 光标进入块 → 显示源码列表标记（与其它列表项一致），离开由调用方重排恢复。
        let first_start = blocks[0].2.start;
        let active = render(&mut eng, first_start);
        assert!(
            active[0].starts_with("1. "),
            "光标进入应显源码标记: {active:?}"
        );
        assert!(active[0].contains("第一章 介绍"));
        assert!(
            !active[0].contains("](#"),
            "链接标记仍按行内规则消隐: {active:?}"
        );
        assert_eq!(active[1], "      1.1 背景");
        // 无序旧目录同样只保留缩进而无 `•`。
        let src = "- [A](#a)\n- [B](#b)";
        let s = Session::open(src);
        let (id, kind, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap_or_default();
        let l = eng.layout_block_with_footnotes(
            kind,
            &src[range.start..range.end],
            range,
            &inlines,
            300.0,
            usize::MAX,
            &[],
            false,
            None,
            &[],
            None,
            true,
        );
        assert_eq!(l.layout_text(), "  A");
    }

    #[test]
    fn list_content_start_matches_marker_end() {
        let cases = [
            (
                BlockKind::ListItem {
                    ordered: true,
                    depth: 0,
                    task: None,
                },
                "1. x",
                Some(2),
            ),
            (
                BlockKind::ListItem {
                    ordered: true,
                    depth: 1,
                    task: None,
                },
                "   1. x",
                Some(5),
            ),
            (
                BlockKind::ListItem {
                    ordered: false,
                    depth: 0,
                    task: None,
                },
                "- x",
                Some(1),
            ),
            (
                BlockKind::ListItem {
                    ordered: false,
                    depth: 0,
                    task: Some(false),
                },
                "- [ ] x",
                None,
            ),
            (BlockKind::Paragraph, "x", None),
        ];
        for (kind, body, want) in cases {
            assert_eq!(list_content_start(&kind, body), want, "{body:?}");
        }
    }

    #[test]
    fn ordered_list_numbering_increments_across_blocks() {
        // 源码每项都写 `1.`（`[TOC]` 旧输出的典型形态）：非激活态按语义递增显示。
        let (_, ls) = layout_blocks_with_caret("1. a\n1. b\n1. c", usize::MAX, 300.0);
        assert_eq!(block_texts(&ls), "1. a\n2. b\n3. c");
    }

    #[test]
    fn ordered_nested_list_restarts_and_parent_continues() {
        // 子列表从 1 起算；子列表结束后父层继续递增（父层源码写 `1.` 仍显示 2.）。
        let src = "1. a\n   1. x\n   1. y\n1. b";
        let (_, ls) = layout_blocks_with_caret(src, usize::MAX, 300.0);
        assert_eq!(block_texts(&ls), "1. a\n   1. x\n   2. y\n2. b");
    }

    #[test]
    fn ordered_list_restarts_from_source_number_after_interruption() {
        // 新列表从源码首个编号起算（3. 开头 → 3./4.）。
        let (_, ls) = layout_blocks_with_caret("3. a\n1. b", usize::MAX, 300.0);
        assert_eq!(block_texts(&ls), "3. a\n4. b");
        // 非列表块打断 → 后续列表重新从源码编号起算。
        let (_, ls) = layout_blocks_with_caret("1. a\n\npara\n\n1. b", usize::MAX, 300.0);
        assert_eq!(block_texts(&ls), "1. a\npara\n1. b");
        // 无序项换序型 → 新列表，不得沿用上一列表的计数。
        let (_, ls) = layout_blocks_with_caret("1. a\n- b\n1. c", usize::MAX, 300.0);
        assert_eq!(block_texts(&ls), "1. a\n• b\n1. c");
    }

    #[test]
    fn ordered_list_active_block_shows_source_number() {
        // caret=7 落在第二块内 → 该块显源码；其余块仍按语义编号。
        let (_, ls) = layout_blocks_with_caret("1. a\n1. b\n1. c", 7, 300.0);
        assert_eq!(block_texts(&ls), "1. a\n1. b\n3. c");
    }

    #[test]
    fn ordered_number_width_growth_keeps_line_height() {
        // 9→10 编号变宽：单行块的行高不得因编号位数变化（必要时前导空格补齐）。
        let src = "1. a\n1. b\n1. c\n1. d\n1. e\n1. f\n1. g\n1. h\n1. i\n1. j";
        let (_, ls) = layout_blocks_with_caret(src, usize::MAX, 300.0);
        assert_eq!(block_texts(&ls).lines().last(), Some("10. j"));
        let h = ls[0].height;
        for l in &ls {
            assert!((l.height - h).abs() < 0.01, "编号宽度变化不得改变行高");
        }
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
    fn admonition_marker_becomes_label_when_inactive() {
        // `> [!NOTE] body`：`>` 隐为空格，`[!NOTE]` 整体替换为可读标签
        let (_, l) = layout_with_caret("> [!NOTE] body", usize::MAX, 400.0);
        assert_eq!(l.layout_text(), "  Note body", "非激活：标记 → 标签");
        assert_eq!(
            l.layout_text().len(),
            "> [!NOTE] body".len() - 3,
            "标签替换为不等长 fill（7B → 4B），映射层按段处理"
        );
    }

    #[test]
    fn admonition_variant_labels_and_alias() {
        for (src, label) in [
            ("> [!TIP] x", "Tip"),
            ("> [!IMPORTANT] x", "Important"),
            ("> [!WARNING] x", "Warning"),
            ("> [!WARN] x", "Warning"),
            ("> [!CAUTION] x", "Caution"),
        ] {
            let (_, l) = layout_with_caret(src, usize::MAX, 600.0);
            assert_eq!(l.layout_text(), format!("  {label} x"), "变体标签: {src}");
        }
    }

    #[test]
    fn admonition_first_line_active_shows_source() {
        // 光标落在首行 → 还原源码（含 `[!TYPE]`），便于编辑标记
        let (_, l) = layout_with_caret("> [!NOTE] body", 5, 400.0);
        assert_eq!(l.layout_text(), "> [!NOTE] body");
    }

    #[test]
    fn admonition_hides_each_line_marker_like_quote() {
        let src = "> [!TIP] a\n> b";
        let (_, l) = layout_with_caret(src, usize::MAX, 400.0);
        assert_eq!(
            l.layout_text(),
            "  Tip a\n  b",
            "两行 `>` 均隐，仅首行标签替换"
        );
        // 光标在第二行 → 仅第二行显源码，首行仍替换标签
        let (_, l) = layout_with_caret(src, src.len() - 1, 400.0);
        assert_eq!(l.layout_text(), "  Tip a\n> b");
        assert!(l.height >= 2.0 * BASE_LINE_HEIGHT, "两行提示块高不塌缩");
    }

    #[test]
    fn admonition_height_stable_across_reveal() {
        let hidden = layout_with_caret("> [!WARNING] careful", usize::MAX, 400.0).1;
        let shown = layout_with_caret("> [!WARNING] careful", 5, 400.0).1;
        assert!(
            (hidden.height - shown.height).abs() < 0.01,
            "显隐切换不得改变行高: {} vs {}",
            hidden.height,
            shown.height
        );
    }

    #[test]
    fn admonition_marker_only_matched_on_first_line() {
        // 第二行出现 `[!NOTE]` 不参与替换（仅首行是标记）
        let src = "> a\n> [!NOTE] b";
        let (_, l) = layout_with_caret(src, usize::MAX, 400.0);
        assert_eq!(l.layout_text(), "  a\n  [!NOTE] b");
    }

    fn layout_with_callouts(
        src: &str,
        caret: usize,
        width: f32,
        registry: &veil_core::CalloutRegistry,
    ) -> BlockLayout {
        let s = Session::open_with_callouts(src, registry);
        let (id, kind, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap();
        let mut eng = LayoutEngine::new();
        let block_text = &src[range.start..range.end];
        eng.layout_block(kind, block_text, range, &inlines, width, caret, &[], false)
    }

    #[test]
    fn custom_callout_uses_configured_label() {
        let reg = veil_core::CalloutRegistry::from_json(
            r##"[{"name":"FIXME","label":"待修复","color":"#c00","icon":"!"}]"##,
        );
        let l = layout_with_callouts("> [!FIXME] body", usize::MAX, 400.0, &reg);
        assert_eq!(l.layout_text(), "  待修复 body", "首行标记 → 配置标签");
        // 光标进入首行 → 还原源码标记
        let l = layout_with_callouts("> [!FIXME] body", 5, 400.0, &reg);
        assert_eq!(l.layout_text(), "> [!FIXME] body");
    }

    #[test]
    fn unknown_callout_marker_hidden_in_plain_quote() {
        // 未配置类型回退普通引用块：`[!XXX]` 不得显示成裸文本
        let (_, l) = layout_with_caret("> [!BOGUS] body", usize::MAX, 400.0);
        assert_eq!(l.layout_text(), "   body");
        assert!(!l.layout_text().contains("[!"), "不得露出裸标记");
    }

    #[test]
    fn highlight_markers_hidden_and_mark_bg_applied() {
        let (_, l) = layout_with_caret("a ==hi== b", usize::MAX, 400.0);
        assert_eq!(l.layout_text(), "a hi b", "非激活：`==` 标记消隐");
        assert!(
            l.decorations_under
                .iter()
                .any(|(_, _, k)| *k == UnderKind::MarkBg),
            "高亮须与 `<mark>` 同一底色装饰"
        );
        // 光标进入节点 → 显示源码
        let (_, l) = layout_with_caret("a ==hi== b", 4, 400.0);
        assert_eq!(l.layout_text(), "a ==hi== b");
    }

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
            align: None,
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
            align: None,
        }
    }

    #[test]
    fn html_alias_styles_match_strong_and_em() {
        let src = "<b>B</b> <strong>S</strong> <i>I</i> <em>E</em>";
        let s = Session::open(src);
        let (id, _kind, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap();
        let spans = style_spans(src.len(), range, &inlines);
        let style_at = |off: usize| {
            spans
                .iter()
                .find(|(s, e, _)| *s <= off && off < *e)
                .map(|(_, _, st)| *st)
                .expect("样式段应致密覆盖")
        };
        assert_eq!(
            style_at(4).bold,
            style_at(15).bold,
            "<b> 与 <strong> 同为粗体"
        );
        assert_eq!(
            style_at(30).italic,
            style_at(41).italic,
            "<i> 与 <em> 同为斜体"
        );
        // 活动/非活动都走同一 style_spans 入口；光栅层权重也一致。
        for (alias, canonical) in [
            ("<b>B</b>", "<strong>B</strong>"),
            ("<i>I</i>", "<em>I</em>"),
        ] {
            let (mut e1, l1) = layout_of(alias, 400.0);
            let (p1, _, _) = e1.rasterize(&l1, [0, 0, 0]);
            let (mut e2, l2) = layout_of(canonical, 400.0);
            let (p2, _, _) = e2.rasterize(&l2, [0, 0, 0]);
            let ink = |p: &[u8]| p.chunks(4).filter(|x| x[3] > 0).count();
            assert_eq!(ink(&p1), ink(&p2), "{alias} 与 {canonical} 光栅墨量一致");
        }
    }

    #[test]
    fn cjk_inline_math_advance_covers_image() {
        let src = "中文 $x$ 后文";
        let imgs = [img(Range::new(7, 10), 40, 10, 8)];
        let (eng, l) = layout_with_imgs(src, usize::MAX, 400.0, &imgs);
        let p = &l.imgs[0];
        let (x_end, _, _) = eng.caret_xy(&l, 10); // 数学节点右缘（后续空格之前）
        assert!(
            x_end + 0.01 >= p.x + p.width as f32,
            "占位 advance 必须覆盖公式图：end={x_end} img_right={}",
            p.x + p.width as f32
        );
        assert!(
            x_end <= p.x + p.width as f32 + 8.0,
            "占位不应明显超过图宽（避免重复占位）：end={x_end} img_right={}",
            p.x + p.width as f32
        );
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
    fn fit_size_box_caps_both_axes_without_enlarging() {
        assert_eq!(fit_size_box(10, 10, 100.0, 100.0), (10, 10), "不得放大");
        assert_eq!(fit_size_box(100, 100, 50.0, 200.0), (50, 50), "宽度上限");
        assert_eq!(fit_size_box(100, 100, 200.0, 44.0), (44, 44), "高度上限");
        // B4 场景：240×160 行内图，2 行限高 44 → 66×44
        assert_eq!(fit_size_box(240, 160, 756.0, 44.0), (66, 44));
        // A：独占一行图片改用视口 80% 限高（600 → 480），宽度不再卡死在小值
        assert_eq!(fit_size_box(800, 600, 756.0, 480.0), (640, 480));
        // A：行内限高从 2 行提到 4 行（行高 22 → 88）
        assert_eq!(fit_size_box(240, 160, 756.0, 88.0), (132, 88));
    }

    #[test]
    fn explicit_size_prefers_author_dimensions() {
        // 显式宽高：原样采用（可放大），不受默认限高约束
        assert_eq!(
            explicit_size(100, 100, Some(300), Some(200), 3024.0, 3024.0),
            (300, 200),
            "宽高都给 → 直接用给定值"
        );
        // 只给宽：高按原图比例（100×50 → 宽 300 则高 150）
        assert_eq!(
            explicit_size(100, 50, Some(300), None, 3024.0, 3024.0),
            (300, 150),
            "只给宽 → 高按比例"
        );
        // 只给高：宽按原图比例（100×50 → 高 200 则宽 400）
        assert_eq!(
            explicit_size(100, 50, None, Some(200), 3024.0, 3024.0),
            (400, 200),
            "只给高 → 宽按比例"
        );
        // 超宽：等比夹到上限（上限 = 版心宽 ×4 = 1600）
        assert_eq!(
            explicit_size(100, 100, Some(5000), Some(1000), 1600.0, 1600.0),
            (1600, 320),
            "超宽 → 等比缩到宽度上限"
        );
        // 超高：等比夹到上限
        assert_eq!(
            explicit_size(100, 100, Some(500), Some(5000), 1600.0, 1600.0),
            (160, 1600),
            "超高 → 等比缩到高度上限"
        );
        // 小图：显式尺寸小于上限时保持原样（不做「不放大」限制）
        assert_eq!(
            explicit_size(1000, 1000, Some(40), Some(40), 1600.0, 1600.0),
            (40, 40),
            "显式小图保持显式尺寸"
        );
    }

    #[test]
    fn image_display_size_uses_explicit_only_when_present() {
        // 无显式尺寸：走默认限宽限高（仅缩小不放大）
        assert_eq!(
            image_display_size(240, 160, (None, None), 756.0, 88.0),
            (132, 88)
        );
        // 有显式尺寸：上限放宽到版心宽 ×4，允许放大
        assert_eq!(
            image_display_size(24, 24, (Some(96), Some(96)), 756.0, 88.0),
            (96, 96)
        );
        // 显式尺寸超极大值：夹到 756 × 4 = 3024
        assert_eq!(
            image_display_size(10, 10, (Some(9999), Some(9999)), 756.0, 88.0),
            (3024, 3024)
        );
    }

    #[test]
    fn tall_image_expands_own_line_and_shifts_following_lines() {
        // B4：100px 高图（> 22px 行高）→ 所在视觉行行高抬到 100，后续行下移，
        // 块高覆盖图片底边，图片下半区域仍可命中节点（不重叠下一块）。
        let imgs = [image(Range::new(0, 13), 100, 100)];
        let (eng, l) = layout_with_imgs(
            "![alt](p.png) bbbbbbbbbbbbbbbbbbbb",
            usize::MAX,
            140.0,
            &imgs,
        );
        let p = &l.imgs[0];
        assert!(p.y.abs() < 0.5, "图应贴视觉行顶: {}", p.y);
        assert!(
            l.height >= p.y + p.height as f32 - 0.5,
            "块高须覆盖图片: {} vs {}",
            l.height,
            p.y + p.height as f32
        );
        let runs: Vec<_> = l.buffer.layout_runs().collect();
        assert!(runs.len() >= 2, "窄宽应折行");
        assert!(
            runs[0].line_height >= 100.0,
            "图所在视觉行行高须容纳图: {}",
            runs[0].line_height
        );
        assert!(
            runs[1].line_top >= runs[0].line_top + 100.0 - 0.5,
            "后续视觉行必须下移: {}",
            runs[1].line_top
        );
        assert_eq!(
            eng.hit(&l, p.x + p.width as f32 * 0.75, p.y + 90.0),
            13,
            "图下半区域（行高内）命中节点尾"
        );
    }

    #[test]
    fn standalone_image_placeholder_is_single_line_without_duplication() {
        // 满宽独立图片：NBSP 占位只能排版一次且单行，块高 = 图片显示高度。
        // 回归：占位 span 被重复拼接时占位文本翻倍 → 折成两行，每行都按图高
        // 计高，独立图片块高度变成 2×图高（下一标题出现半屏空白）。
        let imgs = [image(Range::new(0, 13), 200, 100)];
        let (_, l) = layout_with_imgs("![alt](p.png)", usize::MAX, 200.0, &imgs);
        let runs: Vec<_> = l.buffer.layout_runs().collect();
        assert_eq!(runs.len(), 1, "满宽占位不得折行");
        assert_eq!(l.imgs.len(), 1);
        assert_eq!(runs[0].line_height, 100.0, "视觉行行高 = 图高");
        assert_eq!(l.height, 100.0, "块高只占图片实际显示高度");
        let nbsp = l.layout_text().chars().filter(|c| *c == '\u{a0}').count();
        let glyphs: usize = runs.iter().map(|r| r.glyphs.len()).sum();
        assert_eq!(
            glyphs, nbsp,
            "占位文本不得被重复排版（字形数应等于 NBSP 数）"
        );
    }

    #[test]
    fn mermaid_placeholder_uses_mono_attrs_without_inflating_block_height() {
        let src = "```mermaid\nmindmap\n  root((Veil))\n```\n\n## 后续标题";
        let session = Session::open(src);
        let (id, kind, range) = session.blocks()[0].clone();
        assert!(matches!(kind, BlockKind::Mermaid));
        let mermaid_key = id.0.to_string();
        let inlines = session.block_inlines(id).unwrap();
        let mut eng = LayoutEngine::new();
        let scale = kind_scale(&kind) * block_scale(kind.clone()) * eng.scale;
        let metrics = Metrics::new(eng.doc_font_size * scale, eng.doc_line_height * scale);
        let mermaid_attrs = attrs_of(
            kind.clone(),
            Style::default(),
            eng.link_color,
            eng.doc_font,
            eng.mono_family,
            metrics.font_size,
        );
        let default_w = eng.measure_nbsp_with_attrs(metrics, &Attrs::new());
        let mono_w = eng.measure_nbsp_with_attrs(metrics, &mermaid_attrs);
        assert!(
            mono_w > default_w * 1.25,
            "测试需暴露默认字体与等宽 NBSP 的宽度差: default={default_w}, mono={mono_w}"
        );

        // 旧算法会得到 default_count 个 NBSP：默认字体下单行可容纳，等宽成形后必折行。
        let width = 400.0f32;
        let default_count = (width / mono_w).floor() as usize + 1;
        let image_w = ((default_count - 1) as f32 * default_w + default_w * 0.5).ceil() as u32;
        assert!(image_w > 0 && (image_w as f32) <= width);
        assert_eq!((image_w as f32 / default_w).ceil() as usize, default_count);
        assert!(default_count as f32 * mono_w > width);
        assert!(default_count <= (width / default_w).floor() as usize);

        let image_h = 120u32;
        let imgs = [InlineImg {
            range,
            key: InlineAsset::Mermaid(mermaid_key),
            width: image_w,
            height: image_h,
            baseline: image_h,
            align: None,
        }];
        let block_text = &src[range.start..range.end];
        let l = eng.layout_block(
            kind,
            block_text,
            range,
            &inlines,
            width,
            usize::MAX,
            &imgs,
            false,
        );
        let runs: Vec<_> = l.buffer.layout_runs().collect();
        assert_eq!(runs.len(), 1, "Mermaid NBSP 占位不得折成多个视觉行");
        assert_eq!(l.imgs.len(), 1);
        assert!(
            (l.height - image_h as f32).abs() <= 1.0,
            "块高只能占位图显示高: layout={}, image={image_h}",
            l.height
        );
        // 后续块的 y 由前一块 height 累加；标题起点不得再包含第二条图高视觉行。
        let following_heading_top = l.height;
        assert!(following_heading_top <= image_h as f32 + 1.0);
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
    fn standalone_image_alignment_offsets_within_content_width() {
        // 独立图片按 align 在版心宽内水平偏移；占位本身不可见，移动位图不压文本。
        let text = r#"<img src="p.png" align="center">"#;
        let node = Range::new(0, text.len());
        let mut center = image(node, 40, 10);
        center.align = Some(veil_core::ImageAlign::Center);
        let (_, l) = layout_with_imgs(text, usize::MAX, 400.0, &[center]);
        assert_eq!(l.imgs.len(), 1);
        assert!(
            (l.imgs[0].x - 180.0).abs() < 1.0,
            "center x={}",
            l.imgs[0].x
        );
        let mut right = image(node, 40, 10);
        right.align = Some(veil_core::ImageAlign::Right);
        let (_, l) = layout_with_imgs(text, usize::MAX, 400.0, &[right]);
        assert!((l.imgs[0].x - 360.0).abs() < 1.0, "right x={}", l.imgs[0].x);
        // 无 align（左对齐/行内图片）：沿用占位原位置。
        let (_, l) = layout_with_imgs(text, usize::MAX, 400.0, &[image(node, 40, 10)]);
        assert!(l.imgs[0].x.abs() < 1.0, "left x={}", l.imgs[0].x);
    }

    #[test]
    fn missing_image_falls_back_to_alt_text() {
        // 未提供图（加载失败/远程 URL）：标记消隐，只显 alt
        let (_, l) = layout_of("a ![alt](p.png) b", 400.0);
        assert_eq!(l.layout_text(), "a alt b");
    }

    // ---- 图片题注（Task 5）：独立成行图片下方显示 alt 文本 ----

    #[test]
    fn standalone_image_caption_adds_own_line_and_block_height() {
        // 「![图注](p.png)」逐字节：!=0 [=1 图=2..5 注=5..8 ]=8 (=9 p.png=10..14 )=14
        let text = "![图注](p.png)";
        let node = Range::new(0, 15);
        let imgs = [image(node, 200, 100)];
        let caps = [caption(node, "图注")];
        let (_, l) = layout_with_captions(text, usize::MAX, 400.0, &imgs, &caps);
        let runs: Vec<_> = l.buffer.layout_runs().collect();
        assert_eq!(runs.len(), 2, "图片一行 + 题注一行");
        assert!(l.layout_text().contains("图注"), "题注须进入视图排版");
        assert!(
            runs[1].line_top >= runs[0].line_top + 100.0 - 0.5,
            "题注必须位于图片下方"
        );
        assert!(
            l.height >= runs[1].line_top + runs[1].line_height - 0.5,
            "块高须覆盖题注行: h={}",
            l.height
        );
        assert!(
            l.height >= 100.0 + BASE_LINE_HEIGHT * 0.9,
            "块高须含题注行（图片 100 + 题注行）: {}",
            l.height
        );
    }

    #[test]
    fn standalone_image_without_caption_keeps_single_line() {
        // 空 alt：shell 不传题注，块内仍只有图片一行。
        let node = Range::new(0, 11); // 「![](p.png)」
        let imgs = [image(node, 200, 100)];
        let (_, l) = layout_with_captions("![](p.png)", usize::MAX, 400.0, &imgs, &[]);
        assert_eq!(l.buffer.layout_runs().count(), 1);
        assert_eq!(l.height, 100.0, "无题注时块高 = 图片显示高度");
    }

    #[test]
    fn long_cjk_caption_wraps_within_content_width() {
        let alt = "这是一段很长的中文题注用于验证窄窗口下的换行行为不应溢出也不应覆盖下一块";
        let text = format!("![{alt}](p.png)");
        let node = Range::new(0, text.len());
        let imgs = [image(node, 120, 60)];
        let caps = [caption(node, alt)];
        let width = 160.0;
        let (eng, l) = layout_with_captions(&text, usize::MAX, width, &imgs, &caps);
        let runs: Vec<_> = l.buffer.layout_runs().collect();
        assert!(runs.len() > 2, "长题注应折行: runs={}", runs.len());
        for run in &runs {
            assert!(
                run.line_w <= width + 1.0,
                "题注/占位行不得溢出: line_w={} width={width}",
                run.line_w
            );
            for g in run.glyphs.iter() {
                assert!(
                    g.x + g.w <= width + 1.0,
                    "字形右缘不得超出内容宽: {}",
                    g.x + g.w
                );
            }
        }
        let last = runs.last().unwrap();
        assert!(
            l.height >= last.line_top + last.line_height - 0.5,
            "块高须覆盖末行题注，避免覆盖下一块"
        );
        // 命中题注行只落到图片节点边界，不会产生越界的文档偏移。
        let off = eng.hit(&l, 10.0, last.line_top + 2.0);
        assert!(off <= text.len(), "题注上的命中须钳在块内: {off}");
    }

    #[test]
    fn missing_image_still_renders_caption_with_placeholder() {
        // 图片加载失败（无 InlineImg）：节点降级为通用占位，题注照常可读。
        let text = "![缺失图](p.png)";
        let node = Range::new(0, 18); // 「![缺失图](p.png)」= 18 字节
        let caps = [caption(node, "缺失图")];
        let (_, l) = layout_with_captions(text, usize::MAX, 400.0, &[], &caps);
        assert!(l.layout_text().contains(t(Key::ImageAltFallback)));
        assert!(l.layout_text().contains("缺失图"));
        assert_eq!(l.buffer.layout_runs().count(), 2, "占位一行 + 题注一行");
        assert!(
            l.layout_text().matches("缺失图").count() == 1,
            "alt 不重复显示"
        );
    }

    #[test]
    fn caption_hidden_while_caret_edits_image_source() {
        let text = "![图注](p.png)";
        let node = Range::new(0, 15);
        let imgs = [image(node, 200, 100)];
        let caps = [caption(node, "图注")];
        let (_, l) = layout_with_captions(text, 5, 400.0, &imgs, &caps);
        assert!(
            !l.layout_text().contains('\n'),
            "光标进入图片时显源码、不追加题注行: {:?}",
            l.layout_text()
        );
        assert!(l.layout_text().contains("p.png"), "应显示图片源码");
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
    fn inline_code_hides_both_backticks_and_skips_cjk_spacing() {
        // 纯拉丁：两端反引号正常消隐
        let (_, l) = layout_of("A `plain` B.", 400.0);
        assert_eq!(l.layout_text(), "A plain B.");
        // 末尾为 CJK：闭反引号必须完全消隐（不得多画一个反引号）
        let (_, l) = layout_of("C `中文` D.", 400.0);
        assert_eq!(l.layout_text(), "C 中文 D.", "闭反引号须完全消隐");
        // 代码内 CJK↔ASCII 边界不得插空格
        let (_, l) = layout_of("E `中文abc` F.", 400.0);
        assert_eq!(
            l.layout_text(),
            "E 中文abc F.",
            "代码 span 内不得插 CJK 空格"
        );
        let (_, l) = layout_of("`纯中文`", 400.0);
        assert_eq!(l.layout_text(), "纯中文");
        // 混排整行（§7 真机反例）；WJ 为禁则胶水零宽字符，剥掉后再逐字断言
        let (_, l) = layout_of(
            "行内代码：`中文行内代码` 与 `let 变量 = 1;` 与 `纯 Latin mono`。",
            400.0,
        );
        assert_eq!(
            l.layout_text().replace('\u{2060}', ""),
            "行内代码：中文行内代码 与 let 变量 = 1; 与 纯 Latin mono。"
        );
    }

    #[test]
    fn midline_frontmatter_setext_keeps_every_content_line() {
        // 中置 `---` 块走 Setext：内容可跨多行，除末行下划线外所有行都必须可见
        let text = "正文\n\n---\n标题: 中文标题\n作者: 张三\n描述: 这是一段中文描述\n---\n";
        let s = Session::open(text);
        let (id, kind, range) = s.blocks()[2].clone();
        assert_eq!(kind, BlockKind::SetextHeading(2));
        let inlines = s.block_inlines(id).unwrap();
        let mut eng = LayoutEngine::new();
        let l = eng.layout_block(
            kind,
            &text[range.start..range.end],
            range,
            &inlines,
            600.0,
            usize::MAX,
            &[],
            false,
        );
        let visible = l.layout_text();
        for want in ["中文标题", "张三", "这是一段中文描述"] {
            assert!(visible.contains(want), "可见文本丢行 {want:?}: {visible:?}");
        }
        assert_eq!(
            visible.lines().count(),
            4,
            "三行内容 + 下划线占位: {visible:?}"
        );
        assert!(!visible.contains("---"), "下划线行应消隐: {visible:?}");
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
        l.buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
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
        assert_eq!(hang_shift(&run2, Some(width)), (0.0, None), "非满行不悬挂");
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
        assert_eq!(
            bold,
            vec![(
                0,
                n,
                Style {
                    bold: true,
                    ..Default::default()
                }
            )]
        );

        let (it, n) = cell_spans(doc, 0, 1);
        let it = it.expect("斜体应可映射");
        assert_eq!(
            it,
            vec![(
                0,
                n,
                Style {
                    italic: true,
                    ..Default::default()
                }
            )]
        );

        let (mono, n) = cell_spans(doc, 0, 2);
        let mono = mono.expect("行内代码应可映射");
        // 行内代码：等宽 + 代码底色（与正文同一装饰语义）。
        assert_eq!(
            mono,
            vec![(
                0,
                n,
                Style {
                    mono: true,
                    code_bg: true,
                    ..Default::default()
                }
            )]
        );

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
            &[],
            None,
            false,
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
        // 上标：`[1]` 字形按 sup（负偏移）抬升，且缩到 0.75 倍；与 HTML `<sup>` 一致。
        assert!(
            l.script_ranges.iter().any(|&(_, _, dy)| dy < 0.0),
            "脚注引用应为上标（负基线偏移）: {:?}",
            l.script_ranges
        );
        // 块处于活动态但光标不在引用节点内时，仍保持上标编号（活动/非活动一致）。
        let (_eng2, l_active, _) = layout_footnote_block(src, 0, 0, 400.0, false);
        assert_eq!(l_active.layout_text(), "see [1] now");
        assert!(l_active.script_ranges.iter().any(|&(_, _, dy)| dy < 0.0));
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

    /// DEF-T05-001：源码模式原样显示 `[TOC]` 标记（文档字节也未被生成目录替换）。
    #[test]
    fn toc_marker_shows_raw_in_source_mode() {
        let src = "[TOC]\n\n# Hello\n";
        let (_eng, l, _) = layout_footnote_block(src, 0, usize::MAX, 400.0, true);
        assert_eq!(l.layout_text(), "[TOC]", "源码模式必须显示原始标记");
    }

    #[test]
    fn toc_display_text_lists_headings_with_indent() {
        let entries = vec![
            (1u8, "一".to_string()),
            (2u8, "二 and c".to_string()),
            (1u8, "三".to_string()),
        ];
        assert_eq!(toc_display_text(&entries), "• 一\n   • 二 and c\n• 三");
        assert_eq!(toc_display_text(&[]), "");
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
            let (rs, re) = (node.range.start - range.start, node.range.end - range.start);
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
        let (_eng, l, _) = layout_footnote_block(src, 1, range.start + 2, 400.0, false);
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
    fn hard_break_backslash_hidden_when_inactive_shown_when_active() {
        // REN-SWEEP-007：非活动块只消隐标记，源换行与文档字节/命中坐标不变。
        let src = "甲\\\n乙\n";
        let (eng, l) = layout_of(src, 400.0);
        assert_eq!(l.layout_text(), "甲\n乙", "不得显示反斜杠或遗留自动空格");
        let runs: Vec<_> = l.buffer.layout_runs().collect();
        assert_eq!(runs.len(), 2, "源硬换行必须保留为两个视觉行");
        let shaped = runs.iter().map(|r| r.text).collect::<Vec<_>>().join("\n");
        assert_eq!(shaped, l.layout_text(), "实际 shaped 文本同样不得泄露标记");
        assert_eq!(l.source_newlines, vec![4], "源换行仍参与块内坐标");
        let (x, y, _) = eng.caret_xy(&l, 5);
        assert_eq!(eng.hit(&l, x + 0.1, y + 1.0), 5, "第二行命中仍映射到乙");

        let raw = "a\\\nb\n";
        let (_, active) = layout_with_caret(raw, 1, 400.0);
        assert_eq!(active.layout_text(), "a\\\nb", "光标进入块显示源码");
        let s = Session::open(src);
        assert_eq!(s.serialize(), src, "视图消隐不得改写文档");
        let raw_session = Session::open(raw);
        let (_, kind, range) = raw_session.blocks()[0].clone();
        let inlines = raw_session
            .block_inlines(raw_session.blocks()[0].0)
            .unwrap();
        let source = LayoutEngine::new().layout_block(
            kind.clone(),
            &raw[range.start..range.end],
            range,
            &inlines,
            400.0,
            usize::MAX,
            &[],
            true,
        );
        assert_eq!(source.layout_text(), "a\\\nb", "源码模式保留反斜杠");
        assert_ne!(
            active.reveal_state(&kind, range, usize::MAX),
            active.reveal_state(&kind, range, 1),
            "光标进出普通段落必须使布局缓存失效"
        );
    }

    #[test]
    fn hard_break_backslash_skips_even_runs_and_code_content() {
        // 偶数反斜杠不是 CommonMark 硬换行标记；普通文本两个都保留。
        let (_, literal) = layout_of("a\\\\\nb\n", 400.0);
        assert_eq!(literal.layout_text(), "a\\\\\nb");
        // 行内代码与围栏代码中的反斜杠是代码内容，不得消隐。
        let (_, inline) = layout_of("`a\\\nb`\n", 400.0);
        assert!(
            inline.layout_text().contains("a\\\nb"),
            "{:?}",
            inline.layout_text()
        );
        let (_, fenced) = layout_of("```text\na\\\nb\n```\n", 400.0);
        assert!(
            fenced.layout_text().contains("a\\\nb"),
            "{:?}",
            fenced.layout_text()
        );
    }

    #[test]
    fn footnote_def_number_is_not_duplicated_in_shaped_text() {
        // REN-SWEEP-006：`cell.text/layout_text` 正确不代表字形输入正确 —— 基础样式段
        // 曾整段覆盖含装饰的 layout 文本，set_rich_text 拼接后行尾多出一份 `n.`。
        let src =
            "# 脚注\n\n正文[^note]与[^two]。\n\n[^note]: 第一个脚注。\n[^two]: 第二个脚注。\n";
        let s = Session::open(src);
        let ctx = FootnoteCtx::new(s.footnote_ids(), Theme::light().muted);
        let mut eng = LayoutEngine::new();
        let mut defs = 0usize;
        for (id, kind, range) in s.blocks().iter() {
            if !matches!(kind, BlockKind::FootnoteDef) {
                continue;
            }
            let inlines = s.block_inlines(*id).unwrap_or_default();
            let l = eng.layout_block_with_footnotes(
                kind.clone(),
                &src[range.start..range.end],
                *range,
                &inlines,
                600.0,
                usize::MAX,
                &[],
                false,
                Some(&ctx),
                &[],
                None,
                false,
            );
            defs += 1;
            let shaped: String = l.buffer.layout_runs().map(|r| r.text).collect();
            assert_eq!(
                shaped,
                l.layout_text(),
                "字形输入必须与 layout 文本一致（不得重复编号）"
            );
            assert_eq!(
                shaped.matches("↩").count(),
                1,
                "返回标记只出现一次: {shaped:?}"
            );
        }
        assert_eq!(defs, 2, "夹具应有两个脚注定义块");
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
        let base1 = revs.base(800.0, 1.0, 600.0, false, s1.footnote_epoch());
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
            &[],
            None,
            false,
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
                    toc: 0,
                    highlight_epoch: 0,
                },
                value: l1,
            },
        );
        assert!(
            cache
                .layout(id1, &kind1, range1, base1, usize::MAX, 0)
                .is_some(),
            "代次未变必须命中"
        );

        let s2 = Session::open(after);
        let (id2, kind2, range2) = s2.blocks()[2].clone();
        let ctx2 = FootnoteCtx::new(s2.footnote_ids(), Theme::light().muted);
        let base2 = revs.base(800.0, 1.0, 600.0, false, s2.footnote_epoch());
        assert_ne!(base1, base2, "文首新增未出现的引用 → 全文编号变化");
        assert!(
            cache
                .layout(id2, &kind2, range2, base2, usize::MAX, 0)
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
            &[],
            None,
            false,
        );
        assert_eq!(l2.layout_text(), "1. Y↩", "新引用让文末定义块获得编号");
    }

    // ---- Mermaid v0：detect_type 接入卡片头 + 空块提示 ----

    #[test]
    fn mermaid_badge_flowchart_and_sequence() {
        let flowchart_src = "```mermaid\ngraph TD\nA-->B\n```";
        let (_, fc_layout) = layout_of(flowchart_src, 400.0);
        assert!(
            fc_layout.layout_text().contains("◈ 流程图"),
            "flowchart 应渲染为 ◈ 流程图 卡片头；实际: {}",
            fc_layout.layout_text()
        );
        // 开围栏行本身应被替换掉（没有 ``` 残留）
        assert!(!fc_layout.layout_text().contains("```mermaid"));

        let seq_src = "```mermaid\nsequenceDiagram\n  A->>B: hi\n```";
        let (_, seq_layout) = layout_of(seq_src, 400.0);
        assert!(
            seq_layout.layout_text().contains("◈ 时序图"),
            "sequenceDiagram 应渲染为 ◈ 时序图 卡片头；实际: {}",
            seq_layout.layout_text()
        );
    }

    #[test]
    fn mermaid_badge_other_types_and_fallback() {
        let cases = [
            ("```mermaid\nclassDiagram\nclass A\n```", "◈ 类图"),
            ("```mermaid\nstateDiagram-v2\n[*] --> Idle\n```", "◈ 状态图"),
            ("```mermaid\nerDiagram\nA ||--o{ B : r\n```", "◈ ER图"),
            ("```mermaid\ngantt\ntitle Test\n```", "◈ 甘特图"),
            ("```mermaid\npie\ntitle Pie\nA : 1\n```", "◈ 饼图"),
            ("```mermaid\njourney\ntitle X\n```", "◈ mermaid"),
        ];
        for (src, want) in cases {
            let (_, l) = layout_of(src, 400.0);
            assert!(
                l.layout_text().contains(want),
                "{src:?} 应得到 {want}；实际: {}",
                l.layout_text()
            );
        }
    }

    #[test]
    fn mermaid_source_mode_shows_fences_raw() {
        // 源码模式（reveal_all=true）不做卡片化，保留原始围栏。
        let src = "```mermaid\ngraph TD\nA-->B\n```";
        let s = Session::open(src);
        let (_, kind, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(s.blocks()[0].0).unwrap();
        let mut eng = LayoutEngine::new();
        let block_text = &src[range.start..range.end];
        let l = eng.layout_block(
            kind,
            block_text,
            range,
            &inlines,
            400.0,
            usize::MAX,
            &[],
            true, // reveal_all = 源码模式
        );
        assert!(
            l.layout_text().contains("```mermaid"),
            "源码模式应保留原始围栏；实际: {}",
            l.layout_text()
        );
        assert!(!l.layout_text().contains("◈ 流程图"));
    }

    // ---- N9：代码块/行内代码 CJK 字体族诊断与回归 ----

    /// 常用 CJK 统一表意文字区（测试字符「准备练习环境」均在此）。
    fn is_han(c: char) -> bool {
        matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
    }

    /// 遍历排版字形，返回 (字符, 族名列表, 是否等宽, face id)。
    /// 直接查 fontdb 才能看清实际命中的系统字体（cosmic 的 Attrs 只记录意图）。
    fn glyph_faces(
        eng: &mut LayoutEngine,
        l: &BlockLayout,
    ) -> Vec<(char, Vec<String>, bool, cosmic_text::fontdb::ID)> {
        let mut out = Vec::new();
        for run in l.buffer.layout_runs() {
            for g in run.glyphs.iter() {
                let ch = run.text[g.start..g.end].chars().next().unwrap_or('\u{0}');
                let (families, monospaced) = eng
                    .font_system()
                    .db_mut()
                    .face(g.font_id)
                    .map(|f| {
                        (
                            f.families.iter().map(|(name, _)| name.clone()).collect(),
                            f.monospaced,
                        )
                    })
                    .unwrap_or_default();
                out.push((ch, families, monospaced, g.font_id));
            }
        }
        out
    }

    /// 首个 Han 字形的 (族名列表, face id)；缺 Han 时 panic（调用方文本必含 Han）。
    fn han_face(
        eng: &mut LayoutEngine,
        l: &BlockLayout,
        ctx: &str,
    ) -> (Vec<String>, cosmic_text::fontdb::ID) {
        glyph_faces(eng, l)
            .into_iter()
            .find(|(c, ..)| is_han(*c))
            .map(|(_, fam, _, id)| (fam, id))
            .unwrap_or_else(|| panic!("{ctx}: 未找到 Han 字形"))
    }

    /// 排版指定索引的块并返回块类型（诊断不同块类共用同一字体路径）。
    fn layout_kind_at(
        text: &str,
        idx: usize,
        width: f32,
    ) -> (LayoutEngine, BlockLayout, BlockKind) {
        let s = Session::open(text);
        let (id, kind, range) = s.blocks()[idx].clone();
        let inlines = s.block_inlines(id).unwrap_or_default();
        let mut eng = LayoutEngine::new();
        let l = eng.layout_block(
            kind.clone(),
            &text[range.start..range.end],
            range,
            &inlines,
            width,
            usize::MAX,
            &[],
            false,
        );
        (eng, l, kind)
    }

    /// 墨水覆盖率：alpha>0 像素数 / 非空包围盒面积（同文本同字体下可跨字号比较）。
    fn ink_coverage(px: &[u8], w: u32, h: u32) -> f64 {
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0, 0);
        let mut ink = 0u64;
        for y in 0..h {
            for x in 0..w {
                if px[((y * w + x) * 4 + 3) as usize] > 0 {
                    ink += 1;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        if ink == 0 {
            return 0.0;
        }
        let area = u64::from(max_x - min_x + 1) * u64::from(max_y - min_y + 1);
        ink as f64 / area as f64
    }

    #[test]
    fn codeblock_cjk_glyph_font() {
        let (mut cb_eng, cb) = layout_of("```rust\n// 准备练习环境\n```", 400.0);
        let (mut par_eng, par) = layout_of("准备练习环境", 400.0);
        let (cb_fam, cb_id) = han_face(&mut cb_eng, &cb, "fenced code");
        let (par_fam, par_id) = han_face(&mut par_eng, &par, "paragraph");
        // 修复前基线取证（--nocapture 可见）
        println!("[N9] fenced Han families = {cb_fam:?}, face = {cb_id:?}");
        println!("[N9] paragraph Han families = {par_fam:?}, face = {par_id:?}");
        assert_eq!(cb_fam, par_fam, "代码块 Han 应与正文 Han 同族");
        assert_eq!(cb_id, par_id, "代码块 Han 与正文 Han 应为同一 face");
    }

    #[test]
    fn mermaid_attrs_match_fenced_mono_and_preserve_paragraph_doc_font() {
        let attrs_for = |kind| {
            attrs_of(
                kind,
                Style::default(),
                [0, 0, 0],
                DocFontFamily::Serif,
                Some("Consolas"),
                16.0,
            )
        };
        let mermaid = attrs_for(BlockKind::Mermaid);
        let fenced = attrs_for(BlockKind::FencedCode(None));
        assert_eq!(mermaid.family, fenced.family);
        assert_eq!(mermaid.family, Family::Name("Consolas"));

        let paragraph = attrs_for(BlockKind::Paragraph);
        assert_eq!(paragraph.family, Family::Name("SimSun"));
    }

    #[test]
    fn codeblock_latin_face_is_mono_family() {
        // Latin 仍必须是启动时探测到的等宽族（Consolas），不能被 CJK 修复带偏。
        let (mut eng, l) = layout_of("```rust\nlet x = 1;\n```", 400.0);
        let want = eng.mono_family().expect("测试机应探测到等宽族");
        let latin = glyph_faces(&mut eng, &l)
            .into_iter()
            .find(|(c, ..)| c.is_ascii_alphabetic())
            .expect("代码块应有 Latin 字形");
        println!(
            "[N9] fenced Latin families = {:?}, monospaced = {}, face = {:?}",
            latin.1, latin.2, latin.3
        );
        assert!(
            latin.1.iter().any(|name| name == want),
            "Latin 应命中等宽族 {want}；实际: {:?}",
            latin.1
        );
        assert!(latin.2, "Latin 字形应来自等宽 face");
    }

    #[test]
    fn inline_code_cjk_matches_paragraph() {
        let (mut inline_eng, inline) = layout_of("a `中文` b", 400.0);
        let (mut par_eng, par) = layout_of("中文", 400.0);
        let (inline_fam, inline_id) = han_face(&mut inline_eng, &inline, "inline code");
        let (par_fam, par_id) = han_face(&mut par_eng, &par, "paragraph");
        assert_eq!(inline_fam, par_fam, "行内代码 Han 应与正文同族");
        assert_eq!(inline_id, par_id, "行内代码 Han 与正文 Han 应为同一 face");
    }

    #[test]
    fn code_block_kinds_share_body_han_face() {
        let (mut par_eng, par) = layout_of("准备练习环境", 400.0);
        let (par_fam, par_id) = han_face(&mut par_eng, &par, "paragraph");
        // (源码, 期望块类)；HtmlBlock / FrontMatter 无卡片头，文本原样参与排版。
        let cases = [
            ("    准备练习环境", "indented code"),
            ("<div>\n准备练习环境\n</div>", "html block"),
            ("---\ntitle: 准备练习环境\n---", "front matter"),
        ];
        for (src, ctx) in cases {
            let (mut eng, l, kind) = layout_kind_at(src, 0, 400.0);
            assert!(
                matches!(
                    kind,
                    BlockKind::IndentedCode | BlockKind::HtmlBlock | BlockKind::FrontMatter(_)
                ),
                "{ctx}: 块类型不符，实际 {kind:?}"
            );
            let (fam, id) = han_face(&mut eng, &l, ctx);
            assert_eq!(fam, par_fam, "{ctx}: Han 应与正文同族");
            assert_eq!(id, par_id, "{ctx}: Han 与正文 Han 应为同一 face");
        }
    }

    #[test]
    fn codeblock_cjk_ink_coverage_close_to_paragraph() {
        // 渲染门禁：同文本下代码块 Han 的墨水覆盖率不得显著低于正文。
        // IndentedCode 不带卡片头/边框，且与段落一样只有一行纯 Han，噪声最小。
        let (mut cb_eng, cb) = layout_of("    准备练习环境", 400.0);
        assert!(matches!(
            Session::open("    准备练习环境").blocks()[0].1,
            BlockKind::IndentedCode
        ));
        let (cb_px, cb_w, cb_h) = cb_eng.rasterize(&cb, [0, 0, 0]);
        let cb_cov = ink_coverage(&cb_px, cb_w, cb_h);

        let (mut par_eng, par) = layout_of("准备练习环境", 400.0);
        let (par_px, par_w, par_h) = par_eng.rasterize(&par, [0, 0, 0]);
        let par_cov = ink_coverage(&par_px, par_w, par_h);

        println!("[N9] code coverage = {cb_cov:.4}, paragraph coverage = {par_cov:.4}");
        assert!(par_cov > 0.0, "正文应有墨水像素");
        assert!(
            cb_cov >= par_cov * 0.85,
            "代码块 Han 墨水覆盖率应 ≥ 正文 85%：code={cb_cov:.4} body={par_cov:.4}"
        );
    }

    #[test]
    fn mermaid_empty_block_shows_hint() {
        let empty_src = "```mermaid\n```";
        let (_, l) = layout_of(empty_src, 400.0);
        assert!(
            l.layout_text().contains("空 Mermaid 块 — 点击插入模板"),
            "空 mermaid 块应显示提示；实际: {}",
            l.layout_text()
        );
        // 同样全是 %% 注释也算空块
        let comment_only = "```mermaid\n%% 注释行\n%% 再一行\n```";
        let (_, l2) = layout_of(comment_only, 400.0);
        assert!(
            l2.layout_text().contains("空 Mermaid 块 — 点击插入模板"),
            "只有 %% 注释的块也应显示提示；实际: {}",
            l2.layout_text()
        );
    }

    // ---- Wave 7 Task 3b: inline HTML decorations / br / sub-sup ----

    #[test]
    fn inline_html_kbd_produces_decoration_under() {
        // <kbd>Ctrl</kbd> → decorations_under 应含 KbdFrame
        let (_, l) = layout_of("Press <kbd>Ctrl</kbd> to save", 500.0);
        assert!(
            l.decorations_under
                .iter()
                .any(|(_, _, k)| matches!(k, UnderKind::KbdFrame)),
            "<kbd> 应生成 KbdFrame 前置装饰；实际: {:?}",
            l.decorations_under
        );
        // 光栅化结果应可见「Ctrl」
        assert!(l.layout_text().contains("Ctrl"));
    }

    #[test]
    fn inline_html_pre_renders_mono_not_literal_tags() {
        let text = "正文 <pre>预格式</pre> 结尾";
        let s = Session::open(text);
        let (id, _, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap();
        let segs = crate::view::style::style_spans(range.len(), range, &inlines);
        let (_, l) = layout_of(text, 500.0);
        assert!(
            !l.layout_text().contains("<pre>") && l.layout_text().contains("预格式"),
            "段落内 <pre> 应按行内代码渲染；实际: {:?}",
            l.layout_text()
        );
        assert!(
            segs.iter().any(|(_, _, st)| st.mono),
            "<pre> 应复用 code 的等宽样式；segs: {segs:?}"
        );
    }

    #[test]
    fn inline_html_q_wraps_content_in_quotes_and_semantic_tags_render() {
        // <q> 非激活：内容两端插入中文双引号，标签本身消隐
        let (_, l) = layout_of("他说 <q>你好</q> 结束", 500.0);
        let text = l.layout_text();
        assert!(
            text.contains('“') && text.contains('”') && text.contains("你好"),
            "<q> 应渲染为双引号包裹；实际: {:?}",
            text
        );
        let open = text.find('“').unwrap();
        let close = text.find('”').unwrap();
        assert!(open < close, "引号顺序应为开在前；实际: {text:?}");
        assert!(
            text[open..close].contains("你好"),
            "内容应位于引号之间；实际: {text:?}"
        );
        assert!(!l.layout_text().contains("<q>"), "标签不应泄漏");
        // 语义标签不泄漏源码，内容可见
        for (src, content) in [
            ("<strong>x</strong>", "x"),
            ("<b>x</b>", "x"),
            ("<em>x</em>", "x"),
            ("<u>x</u>", "x"),
            ("<s>x</s>", "x"),
            ("<del>x</del>", "x"),
            ("<ins>x</ins>", "x"),
            ("<cite>x</cite>", "x"),
            ("<var>x</var>", "x"),
            ("<samp>x</samp>", "x"),
            ("<small>x</small>", "x"),
            ("<span>x</span>", "x"),
        ] {
            let (_, l) = layout_of(src, 500.0);
            assert_eq!(l.layout_text(), content, "{src} 应只渲染内容");
        }
    }

    #[test]
    fn inline_html_style_bg_and_underline_decorations() {
        let (_, l) = layout_of(
            "<span style=\"background-color:#ff0;text-decoration:underline\">x</span>",
            500.0,
        );
        assert!(
            l.decorations_under
                .iter()
                .any(|(_, _, k)| matches!(k, UnderKind::StyleBg([255, 255, 0]))),
            "background-color 应生成 StyleBg 装饰；实际: {:?}",
            l.decorations_under
        );
        assert!(
            l.decorations_over
                .iter()
                .any(|(_, _, k)| matches!(k, OverKind::Underline)),
            "text-decoration:underline 应生成实线下划线；实际: {:?}",
            l.decorations_over
        );
    }

    #[test]
    fn inline_html_mark_produces_decoration_under() {
        // <mark>重点</mark> → decorations_under 应含 MarkBg
        let (_, l) = layout_of("这是 <mark>重点</mark> 内容", 500.0);
        assert!(
            l.decorations_under
                .iter()
                .any(|(_, _, k)| matches!(k, UnderKind::MarkBg)),
            "<mark> 应生成 MarkBg 前置装饰；实际: {:?}",
            l.decorations_under
        );
    }

    #[test]
    fn inline_html_br_breaks_line() {
        // foo<br>bar → layout_text 里应有换行符，line_bases ≥ 2
        let text = "foo<br>bar";
        let s = Session::open(text);
        let (id, kind, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap();
        eprintln!("DEBUG kind = {kind:?}, range = {range:?}");
        eprintln!("DEBUG inlines = {inlines:#?}");
        let (_, l) = layout_of(text, 500.0);
        eprintln!("DEBUG layout_text = {:?}", l.layout_text());
        eprintln!("DEBUG line_bases = {:?}", l.line_bases);
        assert!(
            l.layout_text().contains('\n'),
            "<br> 应替换为 \\n；layout_text: {:?}",
            l.layout_text()
        );
        assert!(
            l.line_bases.len() >= 2,
            "<br> 应断成至少 2 行；line_bases: {:?}",
            l.line_bases
        );
    }

    #[test]
    fn inline_html_sub_sup_scaled_down() {
        // x<sup>2</sup>y<sub>1</sub> → style_segments 里 Sub/Sup 段 size_factor ≈ 0.75
        // cosmic-text run 的 line_height 是整行统一值，无法区分 sub/sup；
        // 改为验证 style_segments 里 size_factor 字段（attrs_of 据此乘进 Attrs::size）。
        let text = "x<sup>2</sup>y<sub>1</sub>";
        let s = Session::open(text);
        let (id, _, range) = s.blocks()[0].clone();
        let inlines = s.block_inlines(id).unwrap();
        let segs = crate::view::style::style_spans(range.len(), range, &inlines);
        let has_sup_factor = segs
            .iter()
            .any(|(_, _, st)| st.size_factor.is_some_and(|f| (f - 0.75).abs() < 1e-3));
        assert!(
            has_sup_factor,
            "sup 段应有 size_factor ≈ 0.75；segs: {segs:?}"
        );
        // 同时也验证 layout_text 里 Sub/Sup 内容可见（标签被消隐，只留内容）
        let (_, l) = layout_of(text, 500.0);
        let visible = l.layout_text();
        assert!(
            visible.contains('x')
                && visible.contains('2')
                && visible.contains('y')
                && visible.contains('1'),
            "sup/sub 内容应可见；layout_text: {visible:?}"
        );
        // 缩字：sup/sub 字形的 font_size 必须小于正文 'x'，且登记了下沉/上浮偏移。
        let sizes: Vec<f32> = l
            .buffer
            .layout_runs()
            .flat_map(|r| r.glyphs)
            .map(|g| g.font_size)
            .collect();
        assert!(
            sizes.iter().filter(|&&s| s < 16.0 * 1.0).count() >= 2,
            "sup/sub 应缩字；glyph font_size: {sizes:?}"
        );
        assert!(
            !l.script_ranges.is_empty(),
            "sup/sub 应登记基线偏移区间；实际: {:?}",
            l.script_ranges
        );
    }

    /// 可见文本：剥掉禁则胶水 WJ 与中西文自动间距的空格，便于对整句做精确断言。
    fn visible_text(lt: &str) -> String {
        lt.chars()
            .filter(|c| *c != '\u{2060}' && *c != ' ')
            .collect()
    }

    #[test]
    fn link_markers_elide_atomically_across_cjk_spacing() {
        // 回归：`个[链接]` 的 CJK↔ASCII 间距曾在 `[` 上生成 fill，与单字符消隐段
        // 同起点竞争时 fill 胜出（from_spans「长者优先」），导致 `](url)` 已消隐
        // 但 `[` 残留的半展开态。现要求：非激活时两端标记必须同时消失。
        let src = "以及一个[链接](https://example.com)。\n";
        let full = "以及一个[链接](https://example.com)。";
        // caret 在链接之前：完全折叠
        let (_, l) = layout_with_caret(src, 0, 400.0);
        assert_eq!(visible_text(l.layout_text()), "以及一个链接。");
        // caret 落在链接内（`链` 的字节偏移 13）：整段源码标记完整显示
        let (_, l) = layout_with_caret(src, 13, 400.0);
        assert_eq!(visible_text(l.layout_text()), full);
        // caret 紧贴 `)` 右侧（40 是 `)`，41 即其后）：闭区间激活 → 整段展开，
        // 不得只余 `[`。二者取「全显示」，与全局 node_active 语义一致。
        let (_, l) = layout_with_caret(src, 41, 400.0);
        assert_eq!(visible_text(l.layout_text()), full);
        // caret 在 `。` 之后：离开链接区间 → 完全折叠
        let (_, l) = layout_with_caret(src, 43, 400.0);
        assert_eq!(visible_text(l.layout_text()), "以及一个链接。");
    }

    #[test]
    fn link_markers_elide_in_surrounding_shapes() {
        // 三种周边形态都必须原子折叠：链接在行首、后紧跟中文标点、前面有空格。
        let cases: &[(&str, &str, &str)] = &[
            (
                "[链接](https://example.com)。\n",
                "链接。",
                "[链接](https://example.com)。",
            ),
            (
                "前文[链接](https://example.com)，后文\n",
                "前文链接，后文",
                "前文[链接](https://example.com)，后文",
            ),
            (
                "前文 [链接](https://example.com)后文\n",
                "前文链接后文",
                "前文[链接](https://example.com)后文",
            ),
        ];
        for (src, folded, expanded) in cases {
            let inside = src.find('链').unwrap(); // 链接文本内，节点必然激活
            let (_, l) = layout_with_caret(src, usize::MAX, 400.0);
            assert_eq!(
                visible_text(l.layout_text()),
                *folded,
                "非激活应完全折叠: src={src:?}"
            );
            let (_, l) = layout_with_caret(src, inside, 400.0);
            assert_eq!(
                visible_text(l.layout_text()),
                *expanded,
                "激活应完整显示: src={src:?}"
            );
        }
    }

    #[test]
    fn link_markdown_markers_are_hidden_when_not_active() {
        // 内联链接、引用式链接、隐式方括号——只要 core 解析为 Link 节点，
        // layout 层必须消隐两端 Markdown 标记 `[...]` `(url)`。裸方括号 `[foo]`
        // （无 URL/引用定义）不产生 Link 节点，标记按原文保留——符合 Markdown 规范。
        let cases: &[(&str, &str)] = &[
            ("foo [x](http://x.com) bar\n", "foo x bar"), // 内联
            ("foo [x][ref] bar\n\n[ref]: http://x.com\n", "foo x bar"), // 引用式
            ("foo [x]\n\n[x]: http://x.com\n", "foo x"),  // 隐式方括号
            ("foo [x] bar\n", "foo [x] bar"),             // 裸方括号：不产生 Link，保留标记
        ];
        for (src, want_visible_substr) in cases {
            let (_, l) = layout_with_caret(src, usize::MAX, 400.0);
            let lt = l.layout_text();
            // 只检查「可见子串必须存在」——不管 CJK 自动间距插入的 WORD JOINER
            // （因此避免 contains 含标点的精确匹配）
            assert!(
                lt.contains(want_visible_substr.trim()),
                "src={src:?}: expected visible {want_visible_substr:?} in layout_text {lt:?}"
            );
        }
    }

    // ===== 容器内嵌套块（REN-SWEEP-002 / 004 / 009）=====

    #[test]
    fn nested_quote_markers_elide_all_levels() {
        // REN-SWEEP-004：`> > > x` 的每一级 `>` 都要消隐，级间空白形成层级缩进。
        let src = "> 一级\n> > 二级\n> > > 三级\n";
        let (_, l) = layout_doc_block(src, 0, usize::MAX, 400.0);
        let lt = l.layout_text();
        assert!(!lt.contains('>'), "多层引用标记必须全部消隐: {lt:?}");
        let lines: Vec<&str> = lt.lines().collect();
        assert_eq!(lines.len(), 3);
        let lead = |s: &str| s.len() - s.trim_start_matches(' ').len();
        assert!(lead(lines[0]) < lead(lines[1]), "二级应有缩进: {lt:?}");
        assert!(lead(lines[1]) < lead(lines[2]), "三级应有缩进: {lt:?}");
        assert_eq!(visible_text(lt).replace('\n', ""), "一级二级三级");
    }

    #[test]
    fn nested_quote_active_line_shows_source() {
        // 光标进入某行只还原该行源码，其余层级标记仍消隐。
        let src = "> 一级\n> > 二级\n> > > 三级\n";
        let caret = src.find("二级").unwrap();
        let (_, l) = layout_doc_block(src, 0, caret, 400.0);
        let lines: Vec<&str> = l.layout_text().lines().collect();
        assert!(lines[0].contains(' '), "非活动行仍消隐: {lines:?}");
        assert!(lines[1].contains("> > 二级"), "活动行显源码: {lines:?}");
        assert!(lines[2].contains("   "), "非活动行仍缩进消隐: {lines:?}");
    }

    #[test]
    fn quote_container_renders_nested_list_task_and_quote() {
        // REN-SWEEP-002：引用块内的有序/无序/任务列表与再嵌套引用不得露出裸标记。
        let src = "> 引用中的列表：\n\
                   >\n\
                   > 1. 第一项\n\
                   >    - 子项含 **加粗** 与 `code`\n\
                   >      > 引用中再引用\n\
                   >      >\n\
                   >      > - [ ] 任务\n\
                   >      > - [x] 已完成\n";
        let (_, l) = layout_doc_block(src, 0, usize::MAX, 400.0);
        let lt = l.layout_text();
        assert!(!lt.contains('>'), "引用标记必须全部消隐: {lt:?}");
        assert!(!lt.contains("- "), "无序/任务列表裸标记必须消失: {lt:?}");
        assert!(
            !lt.contains("[ ]") && !lt.contains("[x]"),
            "复选框裸标记: {lt:?}"
        );
        assert!(
            !lt.contains("**") && !lt.contains('`'),
            "行内标记应消隐: {lt:?}"
        );
        assert!(lt.contains('•'), "无序列表应显示圆点: {lt:?}");
        assert!(lt.contains('☐') && lt.contains('☑'), "任务框应显示: {lt:?}");
        assert!(lt.contains("1."), "有序列表编号保留源码编号: {lt:?}");
        assert!(lt.contains("加粗") && !lt.contains("**加粗**"));
    }

    #[test]
    fn list_item_continuation_nested_quote_markers_elided() {
        // REN-SWEEP-002/004：列表项续行里的嵌套引用 `>`（含多层）同样消隐。
        let src = "- 列表中的引用：\n  > 引用段落\n  >\n  > > 二级引用\n";
        let (_, l) = layout_doc_block(src, 0, usize::MAX, 400.0);
        let lt = l.layout_text();
        assert!(!lt.contains('>'), "列表项内引用标记必须消隐: {lt:?}");
        assert!(lt.contains("引用段落") && lt.contains("二级引用"));
    }

    #[test]
    fn list_item_nested_fence_renders_code_lines() {
        // REN-SWEEP-002：列表项续行里的围栏代码按代码行渲染（围栏行消隐）。
        let src = "- 列表中的代码：\n  ```text\n  code in list\n  ```\n";
        let (_, l) = layout_doc_block(src, 0, usize::MAX, 400.0);
        let lt = l.layout_text();
        assert!(!lt.contains("```"), "围栏标记必须消隐: {lt:?}");
        assert!(lt.contains("code in list"), "代码内容应可见: {lt:?}");
        assert!(lt.contains('•'), "列表项圆点应显示: {lt:?}");
    }

    #[test]
    fn list_item_nested_details_tags_elided() {
        // REN-SWEEP-002：列表项续行里的 details/summary 裸标签消隐，正文按行内语法排版。
        let src = "- 列表中的 details：\n  <details>\n  <summary>摘要</summary>\n\n  正文 **加粗**。\n  </details>\n";
        let (_, l) = layout_doc_block(src, 0, usize::MAX, 400.0);
        let lt = l.layout_text();
        for raw in ["<details>", "</details>", "<summary>", "</summary>"] {
            assert!(!lt.contains(raw), "裸标签 {raw} 必须消隐: {lt:?}");
        }
        assert!(!lt.contains("**"), "正文行内标记应消隐: {lt:?}");
        assert!(lt.contains("摘要"), "summary 文本应可见: {lt:?}");
        assert!(lt.contains("加粗"), "正文应可见: {lt:?}");
    }

    #[test]
    fn details_body_paragraph_parses_inline_markdown() {
        // REN-SWEEP-009：details 正文是原生子块，`**加粗**` 必须按行内语法渲染。
        let src = "# T\n\n<details>\n<summary>折叠摘要</summary>\n\n这里是折叠正文，含 **加粗**。\n\n</details>\n";
        let s = Session::open(src);
        let (id, kind, range) = s
            .blocks()
            .into_iter()
            .find(|(_, k, _)| matches!(k, BlockKind::Paragraph))
            .expect("details 正文应拆为 Paragraph 子块");
        let inlines = s.block_inlines(id).unwrap();
        let mut eng = LayoutEngine::new();
        let block_text = &src[range.start..range.end];
        let l = eng.layout_block(
            kind,
            block_text,
            range,
            &inlines,
            400.0,
            usize::MAX,
            &[],
            false,
        );
        let lt = l.layout_text();
        assert!(
            !lt.contains("**"),
            "加粗标记必须消隐（不得显示原文）: {lt:?}"
        );
        assert!(lt.contains("加粗"), "加粗文本应可见: {lt:?}");
    }
}
