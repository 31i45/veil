//! cosmic-text 排版引擎：块级布局、命中检测、光标坐标、光栅化。
//! 布局单位即物理像素——缩放因子由 shell 在 width/metrics 上乘入，本层无 DPI 概念。
//!
//! 消隐机制（统一）：行尾 `\r` 与非激活 inline 节点的标记段都不参与排版（`\r` 交给
//! cosmic 会渲染成缺字形框；标记消隐即 Reveal）。两类区间统一进消隐集合，由
//! `OffsetMap` 负责 doc↔layout 坐标换算——对外偏移一律为文档坐标。
//! 激活规则 v0：`range.start <= caret <= range.end` 闭区间，逐节点独立、嵌套各自生效。

use std::collections::{HashMap, HashSet};

use cosmic_text::{
    fontdb, Attrs, Buffer, Color, Cursor, Family, FontSystem, Metrics, Shaping, Style as FontStyle,
    SwashCache, Weight, Wrap,
};
use veil_core::{task_span, BlockId, BlockKind, HtmlTag, InlineKind, InlineNode, Range};

use super::highlight::{to_styles, Highlighter};
use super::linebreak;
use super::style::{style_spans, Style};
use crate::geometry::{BASE_FONT_SIZE, BASE_LINE_HEIGHT};
use crate::i18n::{t, tf, Key};
use crate::theme::{DocFontFamily, DocMargin, SyntaxColors, Theme};
use crate::tokens::typography;
use crate::type_scale;

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

/// 任务列表自绘方框（块内物理像素）：`(x, y)` 为方框左上角，`side` 为边长（正方形）。
/// 几何只由行高推导（`side` = 0.9 × 行高），不取字形宽度、不随正文字族变化；
/// 勾选态只影响绘制内容（填充/对勾），不改变外接矩形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskBox {
    pub x: f32,
    pub y: f32,
    pub side: f32,
    pub done: bool,
}

/// 任务列表标记（块相对 doc 区间 + 勾选态）：排版期用于生成固定宽度占位，
/// 并在成形后换算成 [`TaskBox`] 供绘制层自绘。
#[derive(Clone, Copy, Debug)]
struct TaskMark {
    start: usize,
    end: usize,
    done: bool,
}

/// 任务框侧边（物理像素）：0.9 × 行高，最少 3px。行高来自排版度量（与字族无关），
/// 勾选态不参与计算，保证勾选前后几何完全一致。
pub(crate) fn task_box_side(line_height: f32) -> f32 {
    (line_height * 0.9).round().max(3.0)
}

/// 任务框占位槽宽（物理像素）= 方框侧边 + 间距（0.35 × 行高）。
/// 排版期换算成等宽 NBSP 个数预留横向空间；占位宽度与勾选态无关。
pub(crate) fn task_box_slot(line_height: f32) -> f32 {
    task_box_side(line_height) + (line_height * 0.35).round().max(1.0)
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
/// 标题比值统一由 `type_scale::screen_heading_scale` 派生（纸面基准表 + 屏幕夹取）。
fn block_scale(kind: BlockKind) -> f32 {
    match kind {
        BlockKind::AtxHeading(level) | BlockKind::SetextHeading(level) => {
            type_scale::screen_heading_scale(level)
        }
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

/// 前置装饰的唯一叠加顺序（back → front），与视觉契约 §6.2 逐条对应：
/// `code_bg`(2) → `mark_bg`(3) → `kbd_frame`(4) → `bg`(5)；几何解析不产生绘制。
/// `ImagePlaceholder` 是图片占位几何，永远在最底层（契约未单列，按“几何最底”处置）。
///
/// 绘制层必须按 `rank()` 稳定排序后再画，不得依赖 `decorations_under` 的构造顺序。
#[allow(dead_code)] // 顺序契约的显式声明：绘制层按 rank 排序，本表供测试逐条锁定
pub const STACK_ORDER: &[UnderKind] = &[
    UnderKind::ImagePlaceholder,
    UnderKind::InlineCodeBg,
    UnderKind::MarkBg,
    UnderKind::KbdFrame,
    UnderKind::StyleBg([0, 0, 0]),
];

impl UnderKind {
    /// 叠放层级（越大越靠前）。数值即契约 §6.2 的步骤号；同一 rank 内保持构造顺序。
    pub fn rank(&self) -> u8 {
        match self {
            UnderKind::ImagePlaceholder => 1,
            UnderKind::InlineCodeBg => 2,
            UnderKind::MarkBg => 3,
            UnderKind::KbdFrame => 4,
            UnderKind::StyleBg(_) => 5,
        }
    }
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
    /// 任务列表自绘方框（块内物理像素）：几何由行高推导，勾选前后完全一致。
    pub task_boxes: Vec<TaskBox>,
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
/// `doc_face` 有值时优先使用该已安装字族；否则 `system` 不设 family（沿用既有回退链）、
/// `sans`/`serif` 显式指定中文字族、`mono` 用通用等宽族。
/// 仅正文/表格文本走这里，UI chrome 不受影响。
pub(super) fn doc_family(
    font: DocFontFamily,
    doc_face: Option<&'static str>,
) -> Option<Family<'static>> {
    if let Some(face) = doc_face {
        return Some(Family::Name(face));
    }
    match font {
        DocFontFamily::System => None,
        DocFontFamily::Sans => Some(Family::Name("Microsoft YaHei")),
        DocFontFamily::Serif => Some(Family::Name("SimSun")),
        DocFontFamily::Mono => Some(Family::Monospace),
    }
}

/// 必需字面组（Windows 系统字体目录内的文件名）：组内按顺序取第一个可加载的。
/// 任一组缺失或解析失败（非 Windows、精简系统、字体被裁掉）即判定最小闭集不可用，
/// 回退 [`FontSystem::new()`] 的全量系统字体扫描，保证不缺字。
const ESSENTIAL_FONT_GROUPS: &[&[&str]] = &[
    // 拉丁 UI / 正文常规
    &["segoeui.ttf"],
    // 正文 CJK（微软雅黑；msyh.ttc 内含 YaHei / YaHei UI 两族）
    &["msyh.ttc", "msyhbd.ttc"],
    // 等宽（代码块 / 行内代码）
    &["consola.ttf", "cascadiamono.ttf"],
];

/// 最小闭集候选文件名（`%SystemRoot%\Fonts` 下）：存在即加载，缺失只影响对应样式。
///
/// 粗体/斜体必须显式列出：缺面时 cosmic-text 会取最近的常规面，强调文本静默退回直立，
/// 视觉上等同字体回退——不能为了启动耗时省掉。
const MINIMAL_FONT_FILES: &[&str] = &[
    "segoeui.ttf",      // 拉丁常规
    "segoeuib.ttf",     // 拉丁粗体
    "segoeuii.ttf",     // 拉丁斜体
    "segoeuiz.ttf",     // 拉丁粗斜体
    "msyh.ttc",         // CJK 正文
    "msyhbd.ttc",       // CJK 粗体
    "consola.ttf",      // 等宽常规
    "consolab.ttf",     // 等宽粗体
    "consolai.ttf",     // 等宽斜体
    "consolaz.ttf",     // 等宽粗斜体
    "cascadiamono.ttf", // 等宽后备（Consolas 缺失时）
    "seguiemj.ttf",     // emoji 回退（彩色）
    "simsun.ttc",       // 衬线正文（DocFontFamily::Serif 的显式族名 "SimSun"）
    "simsunb.ttf",      // 生僻汉字兜底（CJK 扩展 B 区，雅黑不覆盖）
];

/// 系统字体目录：`%SystemRoot%\Fonts`（`windir` 兜底，兼容装在非 C 盘的系统）；
/// 两个变量都没有（非 Windows）时返回 `None`。
fn system_font_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map(|root| std::path::PathBuf::from(root).join("Fonts"))
}

/// 排版 locale：优先 `LC_ALL`/`LC_CTYPE`/`LANG`，取不到时用 `zh-CN`。
///
/// cosmic-text 只用它决定 Han 统一回退族；`zh-CN` 对应微软雅黑，正是最小闭集里
/// 实际加载的 CJK 族，其他语言只会落到同一默认分支。
fn font_locale() -> String {
    for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(raw) = std::env::var(key) {
            let tag = raw
                .split(['.', '@'])
                .next()
                .unwrap_or_default()
                .replace('_', "-");
            if !tag.is_empty() && tag != "C" && tag != "POSIX" {
                return tag;
            }
        }
    }
    "zh-CN".to_string()
}

/// 只用最小字体闭集构造 `FontSystem`：不枚举系统字体目录，只 `load_font_file` 上面
/// 列出的几个字面。必需组缺失时返回 `None`，由调用方回退全量扫描。
fn minimal_font_system() -> Option<FontSystem> {
    let dir = system_font_dir()?;
    let mut db = fontdb::Database::new();
    let mut loaded: Vec<&str> = Vec::new();
    for name in MINIMAL_FONT_FILES {
        let path = dir.join(name);
        if path.is_file() && db.load_font_file(&path).is_ok() {
            loaded.push(name);
        }
    }
    let complete = ESSENTIAL_FONT_GROUPS
        .iter()
        .all(|group| group.iter().any(|name| loaded.contains(name)));
    if !complete {
        return None;
    }
    Some(FontSystem::new_with_locale_and_db(font_locale(), db))
}

/// 启动期字体系统构造结果。
struct StartupFonts {
    font_system: FontSystem,
    /// 显式等宽族名（N9：避免 generic Monospace 的候选链截走 Han）。
    mono_family: Option<&'static str>,
    /// 是否只加载了最小闭集（未枚举全部系统字体），供后续按需补全。
    minimal: bool,
}

/// 创建字体系统并显式指定 generic family（D8）。
///
/// 启动路径只加载最小闭集（约几个字体文件），不再全量枚举 `C:\Windows\Fonts`
/// （fontdb 全量扫描约 140 ms）。cosmic-text 的默认族（Open Sans / Noto Sans Mono）
/// 在 Windows 上通常不存在，汉字会走各自回退，造成「`#` 与汉字不同族」——这里按
/// `typography` 的回退链探测已加载的族后显式设置；候选全部缺失时保持 fontdb 默认。
///
/// 返回同时给出探测到的等宽族名：generic `Monospace` 的回退链会先耗尽等宽候选，
/// 旧 CJK 等宽字体（如 MS Gothic）会截走 Han，因此代码文本改用显式族名（N9）。
fn new_font_system() -> StartupFonts {
    // 回退分支：最小闭集不可用（非 Windows / 精简系统 / 字体缺失）时退回原行为——
    // 枚举全部系统字体。宁可启动慢，也不能缺字。
    let (mut fs, minimal) = match minimal_font_system() {
        Some(fs) => (fs, true),
        None => (FontSystem::new(), false),
    };
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
    StartupFonts {
        font_system: fs,
        mono_family: mono,
        minimal,
    }
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
    /// 显式正文字族名（`None` = 沿用 `doc_font` 四档预置）。
    ///
    /// 驻留为 `'static`：`cosmic-text` 的 `Attrs` 借用族名，且布局缓存/导出样式
    /// 都要求跨调用存活。设置项变更频率极低（人工操作），每次至多驻留 64 字节。
    doc_face: Option<&'static str>,
    /// 屏幕版心边距（逻辑像素，未乘 scale）：配置档位的运行时解析值。
    doc_margin_px: f32,
    /// 可读列宽上限（全角当量）：`None` = 全宽档（不封顶）。
    doc_readable_cols: Option<f32>,
    /// 源码模式纯文本行视图：按行排版原文（等宽、不折行、统一行高）。
    /// 仅由 `ensure_layouts_range` 在每次排版前按运行时开关设置，不参与缓存键
    /// （合成行块使用独立 id 空间，切换时自然换键）。
    plain_lines: bool,
    /// 启动时探测到的等宽族名：代码/行内代码显式指定，避免 generic Monospace 的
    /// 等宽候选链截走 Han（N9）。
    mono_family: Option<&'static str>,
    /// 启动期只加载了最小字体闭集：首次需要「全部已安装字族」（设置面板列字体、
    /// 应用配置里的自定义字族）时再补一次全量扫描，不拖慢首帧。
    fonts_minimal: bool,
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

#[path = "layout_engine.rs"]
mod layout_engine;

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

/// 块级替换（切片 12）：任务列表复选框（自绘方框占位）、无序列表符号 `•`、
/// 引用行首 `>` → 空格、分隔线整行消隐；标题 `#` 与围栏 ` ``` ` 同规则消隐（B7）。
/// 激活判据沿用块区间闭区间；引用块逐行判定（光标所在行显源码，其余行隐标记）。
#[allow(clippy::too_many_arguments)] // 包装层比内部多一个容器归属参数
fn block_subs(
    kind: BlockKind,
    body: &str,
    block: Range,
    caret: usize,
    ordinal: Option<u32>,
    toc_list: bool,
    tasks: &mut Vec<TaskMark>,
    quote_depth: u8,
) -> Vec<Sub> {
    let mut out = block_subs_kind(kind.clone(), body, block, caret, ordinal, toc_list, tasks);
    // 容器内提升块（Details 头块及其子块等）：kind 不再是 BlockQuote，但正文仍带
    // 引用前缀。与 BlockQuote 同一规则逐行等长消隐 `>`（保留缩进/行高）。
    if quote_depth > 0
        && !matches!(
            kind,
            BlockKind::BlockQuote | BlockKind::Callout(_) | BlockKind::Admonition(_)
        )
    {
        out.extend(quote_line_subs(body, block, caret));
    }
    out
}

fn block_subs_kind(
    kind: BlockKind,
    body: &str,
    block: Range,
    caret: usize,
    ordinal: Option<u32>,
    toc_list: bool,
    tasks: &mut Vec<TaskMark>,
) -> Vec<Sub> {
    match kind {
        BlockKind::ListItem { task: Some(_), .. } => {
            if node_active(block, caret) {
                return Vec::new();
            }
            // 任务框不再用 `☐`/`☑` 字形（两者字面宽窄不同，勾选/换字族都会改几何）：
            // 标记段登记为 `TaskMark`，由调用方替换成定宽占位，方框在绘制层自绘。
            if let Some((start, _, end, done)) = task_span(body) {
                tasks.push(TaskMark { start, end, done });
            }
            Vec::new()
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
/// - 无序列表 → `•`；任务列表 → 自绘方框占位（原缩进保留为层级）
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
    tasks: &mut Vec<TaskMark>,
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
                nested_list_marker_subs(content, off + content_off, &mut subs, tasks);
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

/// 容器内嵌套列表标记：任务框 → 登记自绘方框，无序 → `•`；有序保留源码编号。
fn nested_list_marker_subs(
    content: &str,
    base: usize,
    out: &mut Vec<Sub>,
    tasks: &mut Vec<TaskMark>,
) {
    if let Some((start, _, end, done)) = task_span(content) {
        tasks.push(TaskMark {
            start: base + start,
            end: base + end,
            done,
        });
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
    doc_face: Option<&'static str>,
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
    } else if let Some(family) = doc_family(doc_font, doc_face) {
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

/// 源码模式纯文本行视图的文本属性：恒等宽、常规字重/字形，忽略块级标题放大与
/// 加粗/斜体标记，只保留语法高亮色与链接色。用于逐行排版原文时保证列对齐一致。
pub(super) fn attrs_of_plain(
    st: Style,
    link_color: [u8; 3],
    mono_family: Option<&'static str>,
    pt: f32,
) -> Attrs<'static> {
    let a = Attrs::new();
    let a = match mono_family {
        Some(name) => a.family(Family::Name(name)),
        None => a.family(Family::Monospace),
    };
    let color = st.color.or_else(|| st.link.then_some(link_color));
    let a = if let Some([r, g, b]) = color {
        a.color(Color::rgb(r, g, b))
    } else {
        a
    };
    if let Some(factor) = st.size_factor {
        a.metrics(Metrics::new(pt * factor, pt * 1.5 * factor))
    } else {
        a
    }
}

#[cfg(test)]
mod typography_tests;

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "layout_inline_tests.rs"]
mod inline_tests;
