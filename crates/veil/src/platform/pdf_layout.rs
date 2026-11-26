//! Markdown → PDF 版面（Wave 4）：块级排版、按宽度换行、分页、目录、页脚。
//!
//! 设计约束：
//! - 分页与书签**同源**（`pages` 索引即 `outline.page_idx`），禁止二次切分；
//! - 只依赖 `TextMetrics` 与 `MediaProvider` 两个接缝，版面逻辑可纯单测；
//! - 无法渲染的图片/公式一律降级为文本，绝不 panic。

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use veil_core::{
    split_heading_id, BlockId, BlockKind, CellAlign, ImageAlign, InlineKind, InlineNode, Range,
    Session,
};
use veil_ui::i18n::{t, Key};

use crate::platform::pdf::{
    DrawOp, ImageData, OutlineItem, TextMetrics, TextRun, A4_HEIGHT, A4_WIDTH,
};

// =============================================================================
// 版面几何 / 选项 / 输出
// =============================================================================

/// 一页的几何：A4 + 四边等距页边距（pt）。
#[derive(Clone, Copy)]
pub struct PageGeom {
    pub width: f32,
    pub height: f32,
    pub margin_x: f32,
    pub margin_y: f32,
}

impl PageGeom {
    pub fn a4(margin_mm: f32) -> Self {
        let m = (margin_mm.clamp(5.0, 60.0)) * 72.0 / 25.4;
        Self {
            width: A4_WIDTH,
            height: A4_HEIGHT,
            margin_x: m,
            margin_y: m,
        }
    }

    pub fn content_w(&self) -> f32 {
        (self.width - 2.0 * self.margin_x).max(120.0)
    }

    fn top(&self) -> f32 {
        self.margin_y
    }

    fn bottom(&self) -> f32 {
        self.height - self.margin_y
    }

    fn content_h(&self) -> f32 {
        (self.bottom() - self.top()).max(80.0)
    }
}

#[derive(Clone, Copy, Default)]
pub struct LayoutOpts {
    pub toc: bool,
    /// 是否绘制页脚 `n / total`（跟随导出设置，默认关闭以保持 `Default` 语义）。
    pub page_numbers: bool,
}

/// 链接注释目标。`Anchor` 是排版中途的临时态（按页内锚点键），
/// 全部页面就绪后统一解析为 `Internal`；解析不到的锚点不输出动作。
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    /// 外部 URI（仅 http/https/mailto，且长度受限）。
    Uri(String),
    /// 文档内跳转：最终页下标 + 目标左上角（pt，自页面顶部）。
    Internal { page: usize, x: f32, y: f32 },
    /// 尚未解析的锚点键（不会出现在 `DocLayout` 输出中）。
    Anchor(String),
}

/// 链接注释：页面内矩形（pt，y 自页面顶部）+ 目标。
#[derive(Clone, Debug, PartialEq)]
pub struct LinkAnnot {
    pub page: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub target: LinkTarget,
}

/// 版面产物：页面绘制指令 + 图片资源 + 多级书签 + 链接注释。
pub struct DocLayout {
    pub pages: Vec<Vec<DrawOp>>,
    pub images: Vec<ImageData>,
    pub outline: Vec<OutlineItem>,
    pub links: Vec<LinkAnnot>,
}

/// 媒体接缝：图片与公式解码（实现见 `pdf_media`，单测可注入假实现）。
pub trait MediaProvider {
    fn image(&mut self, path: &Path) -> Result<ImageData, String>;
    fn math(&mut self, tex: &str, display: bool) -> Result<ImageData, String>;
}

// =============================================================================
// 样式与行内条目
// =============================================================================

const BODY_SIZE: f32 = 11.0;
const CODE_SIZE: f32 = 9.3;
const TABLE_SIZE: f32 = 8.8;

const COL_TEXT: [u8; 3] = [24, 28, 35];
const COL_MUTED: [u8; 3] = [107, 114, 128];
const COL_LINK: [u8; 3] = [37, 99, 235];
const COL_RULE: [u8; 3] = [209, 213, 219];
const COL_CODE_BG: [u8; 3] = [243, 244, 246];

/// 强调色按比例混入白底（Callout 浅底色；与预览 10% 同比例，PDF 恒浅色页面）。
fn tint_on_white(accent: [u8; 3], ratio: f32) -> [u8; 3] {
    let r = ratio.clamp(0.0, 1.0);
    let mix = |a: u8| ((a as f32) * r + 255.0 * (1.0 - r)).round() as u8;
    [mix(accent[0]), mix(accent[1]), mix(accent[2])]
}
const COL_HEAD_BG: [u8; 3] = [249, 250, 251];

#[derive(Clone, Copy, PartialEq)]
struct Style {
    size: f32,
    bold: bool,
    italic: bool,
    monospace: bool,
    underline: bool,
    strike: bool,
    highlight: Option<[u8; 3]>,
    color: [u8; 3],
}

impl Style {
    fn body() -> Self {
        Self {
            size: BODY_SIZE,
            bold: false,
            italic: false,
            monospace: false,
            underline: false,
            strike: false,
            highlight: None,
            color: COL_TEXT,
        }
    }
}

#[derive(Clone)]
struct BoxItem {
    id: usize,
    w: f32,
    h: f32,
    /// 图顶到基线的距离（pt）。
    baseline: f32,
    /// 独占行图片的水平对齐（`<img align>`）；行内图片为 None。
    align: Option<ImageAlign>,
}

#[derive(Clone)]
enum LItem {
    Ch(char, Style),
    Box(BoxItem),
    /// 链接起点：携带 `Ctx::link_targets` 下标；跨行时每行各成一个矩形。
    LinkOpen(u32),
    LinkClose,
}

fn item_width(it: &LItem, m: &dyn TextMetrics) -> f32 {
    match it {
        LItem::Ch(c, s) => m.width_of_style(*c, s.size, s.monospace),
        LItem::Box(b) => b.w,
        LItem::LinkOpen(_) | LItem::LinkClose => 0.0,
    }
}

/// 贪心换行：优先在空格处断行；CJK 任意断行；超长串硬断。
fn fill_lines(items: Vec<LItem>, max_w: f32, m: &dyn TextMetrics) -> Vec<Vec<LItem>> {
    let max_w = max_w.max(24.0);
    let mut lines: Vec<Vec<LItem>> = Vec::new();
    let mut cur: Vec<LItem> = Vec::new();
    let mut w = 0.0f32;
    let mut last_space: Option<(usize, f32)> = None;

    for it in items {
        let iw = item_width(&it, m);
        if w + iw > max_w && !cur.is_empty() {
            match last_space {
                Some((bi, _bw)) if bi > 0 && bi < cur.len() => {
                    let split = kinsoku_break(&cur, bi, None);
                    let rest = cur.split_off(split);
                    lines.push(cur);
                    cur = rest;
                    last_space = None;
                }
                _ => {
                    let next = match &it {
                        LItem::Ch(c, _) => Some(*c),
                        _ => None,
                    };
                    let split = kinsoku_break(&cur, cur.len(), next);
                    if split < cur.len() {
                        let rest = cur.split_off(split);
                        lines.push(cur);
                        cur = rest;
                    } else {
                        lines.push(std::mem::take(&mut cur));
                    }
                    last_space = None;
                }
            }
            while matches!(cur.first(), Some(LItem::Ch(' ', _))) {
                cur.remove(0);
            }
            w = cur.iter().map(|i| item_width(i, m)).sum();
            if matches!(&it, LItem::Ch(' ', _)) {
                continue;
            }
        }
        w += iw;
        cur.push(it);
        if matches!(cur.last(), Some(LItem::Ch(' ', _))) {
            last_space = Some((cur.len(), w));
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// 中文断行禁则用的行首禁排字符（标点/闭符号不得出现在行首）。
fn is_line_start_forbidden(c: char) -> bool {
    matches!(
        c,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '？'
            | '！'
            | '）'
            | '〕'
            | '］'
            | '｝'
            | '〉'
            | '》'
            | '」'
            | '』'
            | '】'
            | '”'
            | '’'
            | '…'
            | '‥'
            | '·'
            | '．'
            | ','
            | '.'
            | ';'
            | ':'
            | '?'
            | '!'
            | ')'
            | ']'
            | '}'
    )
}

/// 行尾禁排字符（开符号/开引号不得留在行尾）。
fn is_line_end_forbidden(c: char) -> bool {
    matches!(
        c,
        '（' | '〔'
            | '［'
            | '｛'
            | '〈'
            | '《'
            | '「'
            | '『'
            | '【'
            | '“'
            | '‘'
            | '('
            | '['
            | '{'
    )
}

/// 直觉断点 `n`（[0,n) 留在本行）→ 禁则修正后的断点。
///
/// 只做最小避头尾：行首禁排字符随前一字符一起下移；行尾禁排字符下移。
/// 修正后至少保留一个 item，避免产生空行；`next` 是尚未入行的下一个字符
/// （硬断时断点正好在它之前）。
fn kinsoku_break(cur: &[LItem], n: usize, next: Option<char>) -> usize {
    if cur.is_empty() {
        return n;
    }
    let mut n = n.clamp(1, cur.len());
    let first_char = cur[n.min(cur.len())..]
        .iter()
        .find_map(|it| match it {
            LItem::Ch(c, _) => Some(*c),
            _ => None,
        })
        .or(next);
    // 避头：下一行首字是闭符号 → 把本行末尾的普通字一起带到下一行。
    // 连续闭符号（如 `。」`）最多再带一个字，避免把前一个闭符号留在行首。
    if let Some(c) = first_char {
        if is_line_start_forbidden(c) {
            let mut moved = 0;
            while n >= 2 && moved < 2 {
                let LItem::Ch(prev, _) = &cur[n - 1] else {
                    break;
                };
                n -= 1;
                moved += 1;
                if !is_line_start_forbidden(*prev) {
                    break;
                }
            }
        }
    }
    // 避尾：本行末字是开符号 → 把它推到下一行。
    if n >= 2 {
        if let LItem::Ch(prev, _) = &cur[n - 1] {
            if is_line_end_forbidden(*prev) {
                n -= 1;
            }
        }
    }
    n
}

// =============================================================================
// 版面上下文
// =============================================================================

struct Head {
    level: u8,
    text: String,
    page: usize,
    top_y: f32,
}

/// 链接目标白名单：仅 `http(s)://`、`mailto:` 与文档内 `#锚点`。
/// 危险 scheme（javascript:/file:/data: 等）、含空白/控制字符或超长 URL
/// 一律不生成可执行动作——保持链接样式，但不输出注释。
fn link_target(url: &str) -> Option<LinkTarget> {
    const MAX_URI: usize = 2048;
    let u = url.trim();
    if u.is_empty() || u.len() > MAX_URI || u.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    if let Some(anchor) = u.strip_prefix('#') {
        return (!anchor.is_empty()).then(|| LinkTarget::Anchor(anchor.to_string()));
    }
    let lower = u.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        Some(LinkTarget::Uri(u.to_string()))
    } else {
        None
    }
}

struct Ctx<'a> {
    metrics: &'a dyn TextMetrics,
    geom: PageGeom,
    pages: Vec<Vec<DrawOp>>,
    images: Vec<ImageData>,
    heads: Vec<Head>,
    page: usize,
    y: f32,
    /// 行内图片高度上限（正文行高倍数，B4）：默认 2 行，独占一行放宽到 6 行，
    /// 表格单元格 1.5 行。与编辑视图同一策略，保证三通道尺寸一致。
    img_max_lines: f32,
    /// 正在排版的带题注独立图片；加载失败时用通用占位，避免 alt 与题注重复。
    captioned_image: Option<Range>,
    /// 已定义脚注 id → 编号（文档首次引用顺序，与 HTML 导出同源）。
    /// 缺定义的引用不进表：引用处保留可读原文，不生成断号。
    footnotes: HashMap<String, usize>,
    /// 链接注释（页下标为当前页；目录插页后统一平移）。
    links: Vec<LinkAnnot>,
    /// 页内锚点：键 → (页下标, 目标 y)。标题 slug 与脚注双向锚点共用。
    anchors: HashMap<String, (usize, f32)>,
    /// 本轮链接目标表（`LItem::LinkOpen` 携带下标）。
    link_targets: Vec<LinkTarget>,
    /// 正在跨越的链接：(目标下标, 本行起点 x)。行尾闭合本行矩形并保留跨行状态。
    open_link: Option<(u32, f32)>,
    /// Mermaid 渲染结果：BlockId → MermaidImage（SVG + 位图）。
    mermaids: &'a std::collections::HashMap<veil_core::BlockId, veil_ui::MermaidImage>,
}

impl<'a> Ctx<'a> {
    fn new(
        metrics: &'a dyn TextMetrics,
        geom: PageGeom,
        mermaids: &'a std::collections::HashMap<veil_core::BlockId, veil_ui::MermaidImage>,
    ) -> Self {
        Self {
            metrics,
            geom,
            pages: Vec::new(),
            images: Vec::new(),
            heads: Vec::new(),
            page: 0,
            y: geom.top(),
            img_max_lines: 2.0,
            captioned_image: None,
            footnotes: HashMap::new(),
            links: Vec::new(),
            anchors: HashMap::new(),
            link_targets: Vec::new(),
            open_link: None,
            mermaids,
        }
    }

    fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.page = self.pages.len() - 1;
        self.y = self.geom.top();
    }

    fn ensure(&mut self, need: f32) {
        if self.y + need > self.geom.bottom() && self.y > self.geom.top() + 1.0 {
            self.new_page();
        }
    }

    fn max_w(&self) -> f32 {
        self.geom.content_w()
    }

    /// 渲染一行（行内条目 → TextRun / Image）。
    fn emit_line(
        &mut self,
        line: Vec<LItem>,
        indent: f32,
        base_size: f32,
        align: CellAlign,
        line_w: f32,
    ) {
        // 基线以上/以下分别取文本与 Box 的最大需求：含行内图片时图顶不得越过
        // 当前 y 压住前一段（尤其是多行题注），文本 descent 也须留在本行高度内。
        let text_h = self.metrics.line_height(base_size);
        let text_above = self.metrics.ascent(base_size);
        let has_text = line.iter().any(|it| matches!(it, LItem::Ch(..)));
        let mut above = if has_text { text_above } else { 0.0 };
        let mut below = if has_text { text_h - text_above } else { 0.0 };
        for it in &line {
            if let LItem::Box(b) = it {
                above = above.max(b.baseline);
                below = below.max(b.h - b.baseline);
            }
        }
        let h = above + below;
        self.ensure(h);
        let line_top = self.y;
        let baseline = self.y + above;

        let mut x = self.geom.margin_x + indent;
        if !matches!(align, CellAlign::None | CellAlign::Left) {
            let wid: f32 = line.iter().map(|i| item_width(i, self.metrics)).sum();
            let slack = (line_w - wid).max(0.0);
            x += if matches!(align, CellAlign::Center) {
                slack / 2.0
            } else {
                slack
            };
        }

        let mut buf = String::new();
        let mut style: Option<Style> = None;
        // 跨行链接：本行从行首重新起算，行尾再闭合（每行一个可点击矩形）。
        if let Some((idx, _)) = self.open_link {
            self.open_link = Some((idx, x));
        }
        for it in line {
            match it {
                LItem::LinkOpen(idx) => {
                    flush_run(
                        &mut self.pages,
                        self.page,
                        &mut buf,
                        &mut style,
                        &mut x,
                        self.metrics,
                        baseline,
                    );
                    self.open_link = Some((idx, x));
                }
                LItem::LinkClose => {
                    flush_run(
                        &mut self.pages,
                        self.page,
                        &mut buf,
                        &mut style,
                        &mut x,
                        self.metrics,
                        baseline,
                    );
                    if let Some((idx, sx)) = self.open_link.take() {
                        self.push_link(idx, sx, x, line_top, h);
                    }
                }
                LItem::Ch(c, st) => {
                    if style != Some(st) {
                        flush_run(
                            &mut self.pages,
                            self.page,
                            &mut buf,
                            &mut style,
                            &mut x,
                            self.metrics,
                            baseline,
                        );
                        style = Some(st);
                    }
                    buf.push(c);
                }
                LItem::Box(b) => {
                    flush_run(
                        &mut self.pages,
                        self.page,
                        &mut buf,
                        &mut style,
                        &mut x,
                        self.metrics,
                        baseline,
                    );
                    // 独占行图片按 align 在可排版宽内偏移；行内图片保持续排位置。
                    let bx = match b.align {
                        Some(ImageAlign::Center) => {
                            self.geom.margin_x + indent + ((line_w - b.w) * 0.5).max(0.0)
                        }
                        Some(ImageAlign::Right) => {
                            self.geom.margin_x + indent + (line_w - b.w).max(0.0)
                        }
                        _ => x,
                    };
                    let top = baseline - b.baseline;
                    self.pages[self.page].push(DrawOp::Image {
                        x: bx,
                        y: top,
                        w: b.w,
                        h: b.h,
                        id: b.id,
                    });
                    x = bx + b.w;
                }
            }
        }
        flush_run(
            &mut self.pages,
            self.page,
            &mut buf,
            &mut style,
            &mut x,
            self.metrics,
            baseline,
        );
        // 行尾仍处于链接内：本行矩形到此为止，跨行状态保留给下一行。
        if let Some((idx, sx)) = self.open_link {
            self.push_link(idx, sx, x, line_top, h);
        }
        self.y += h;
    }

    /// 记录一个链接标记下标（`LItem::LinkOpen` 引用它）。
    fn link_mark(&mut self, target: LinkTarget) -> u32 {
        self.link_targets.push(target);
        (self.link_targets.len() - 1) as u32
    }

    /// 把 `[x0, x1)` × 本行高登记为链接矩形；零宽（空链接/行首尾相接）跳过。
    fn push_link(&mut self, idx: u32, x0: f32, x1: f32, top: f32, h: f32) {
        if x1 <= x0 + 0.1 {
            return;
        }
        let Some(target) = self.link_targets.get(idx as usize).cloned() else {
            return;
        };
        self.links.push(LinkAnnot {
            page: self.page,
            x: x0,
            y: top,
            w: x1 - x0,
            h,
            target,
        });
    }

    fn blank(&mut self, size: f32) {
        self.ensure(self.metrics.line_height(size));
        self.y += self.metrics.line_height(size) * 0.55;
    }

    // —— 块级 ——

    fn heading(&mut self, s: &Session, id: BlockId, level: u8) {
        let (lvl, text) = match s.heading(id) {
            Some((l, t)) => (l, t),
            None => (level, String::new()),
        };
        // 标题尾缀 `{#id}` 是锚点指令，不得进入版面可见文本/书签。
        let (text, _) = split_heading_id(&text);
        let size = heading_size(lvl);
        self.ensure(size * 2.4);
        self.y += size * 0.55;
        self.heads.push(Head {
            level: lvl,
            text: text.trim().to_string(),
            page: self.page,
            top_y: self.y,
        });
        let st = Style {
            size,
            bold: true,
            ..Style::body()
        };
        let items: Vec<LItem> = text.trim().chars().map(|c| LItem::Ch(c, st)).collect();
        for line in fill_lines(items, self.max_w(), self.metrics) {
            self.emit_line(line, 0.0, size, CellAlign::None, self.max_w());
        }
        self.y += size * 0.35;
    }

    fn paragraph(
        &mut self,
        s: &Session,
        id: BlockId,
        start: usize,
        end: usize,
        indent: f32,
        media: &mut Option<&mut dyn MediaProvider>,
    ) {
        let text = slice_text(s, start, end);
        let visible = text.trim_end().len();
        let nodes = s.block_inlines(id).unwrap_or_default();
        // B4：段落含独立图片 → 放宽到 6 行（与编辑视图/HTML 导出同策略）。
        // 独立图片判定与题注均取自内核单一真值，三端语义一致。
        let prev_limit = self.img_max_lines;
        let block = Range::new(start, start + text.len());
        if nodes
            .iter()
            .any(|n| n.is_image() && veil_core::image_standalone(&text, block, n.range))
        {
            self.img_max_lines = 6.0;
        }
        let caption = veil_core::standalone_image_caption(&nodes, &text, start);
        let prev_captioned = self.captioned_image;
        self.captioned_image = caption.as_ref().map(|(range, _)| *range);
        self.rich(
            &text,
            start,
            start,
            start + visible,
            &nodes,
            Style::body(),
            indent,
            media,
        );
        self.captioned_image = prev_captioned;
        self.img_max_lines = prev_limit;
        if let Some((_, caption)) = caption {
            let st = Style {
                size: 9.5,
                color: COL_MUTED,
                ..Style::body()
            };
            let items: Vec<LItem> = caption.chars().map(|c| LItem::Ch(c, st)).collect();
            let avail = (self.max_w() - indent).max(40.0);
            self.y += 2.0;
            for line in fill_lines(items, avail, self.metrics) {
                self.emit_line(line, indent, st.size, CellAlign::None, avail);
            }
        }
        self.y += 5.0;
    }

    /// 行内内容排版：`from..to` 为可见区间（绝对偏移），`base` 为 text 在文档中的起点。
    #[allow(clippy::too_many_arguments)]
    fn rich(
        &mut self,
        text: &str,
        base: usize,
        from: usize,
        to: usize,
        nodes: &[InlineNode],
        style: Style,
        indent: f32,
        media: &mut Option<&mut dyn MediaProvider>,
    ) {
        let mut items: Vec<LItem> = Vec::new();
        self.emit_inlines(text, base, from, to, nodes, style, &mut items, media);
        let avail = (self.max_w() - indent).max(40.0);
        for line in fill_lines(items, avail, self.metrics) {
            self.emit_line(line, indent, style.size, CellAlign::None, avail);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_inlines(
        &mut self,
        text: &str,
        base: usize,
        from: usize,
        to: usize,
        nodes: &[InlineNode],
        style: Style,
        out: &mut Vec<LItem>,
        media: &mut Option<&mut dyn MediaProvider>,
    ) {
        let mut cur = from;
        for node in nodes {
            let ns = node.range.start.max(from);
            let ne = node.range.end.min(to);
            if ns >= ne {
                continue;
            }
            if ns > cur {
                push_chars(out, text, base, cur, ns, style);
            }
            match node.kind {
                InlineKind::Strong => self.emit_inlines(
                    text,
                    base,
                    node.content.start.max(ns),
                    node.content.end.min(ne),
                    &node.children,
                    Style {
                        bold: true,
                        ..style
                    },
                    out,
                    media,
                ),
                InlineKind::Emph => self.emit_inlines(
                    text,
                    base,
                    node.content.start.max(ns),
                    node.content.end.min(ne),
                    &node.children,
                    Style {
                        italic: true,
                        ..style
                    },
                    out,
                    media,
                ),
                InlineKind::Strikethrough => self.emit_inlines(
                    text,
                    base,
                    node.content.start.max(ns),
                    node.content.end.min(ne),
                    &node.children,
                    Style {
                        strike: true,
                        ..style
                    },
                    out,
                    media,
                ),
                // N3c：`==x==` 与 `<mark>` 同一高亮底色。
                InlineKind::Highlight => self.emit_inlines(
                    text,
                    base,
                    node.content.start.max(ns),
                    node.content.end.min(ne),
                    &node.children,
                    Style {
                        highlight: Some([255, 247, 153]),
                        ..style
                    },
                    out,
                    media,
                ),
                InlineKind::Code => {
                    let st = Style {
                        monospace: true,
                        highlight: Some(COL_CODE_BG),
                        ..style
                    };
                    // 代码内容原样：反斜杠与行尾 `\` 不是转义/硬换行标记。
                    push_raw_chars(
                        out,
                        text,
                        base,
                        node.content.start.max(ns),
                        node.content.end.min(ne),
                        st,
                    );
                }
                InlineKind::Link => {
                    // 链接注释：外部 URI / 文档内锚点；不在白名单内则只保留样式。
                    let url = node
                        .url
                        .and_then(|r| {
                            text.get(r.start.saturating_sub(base)..r.end.saturating_sub(base))
                        })
                        .unwrap_or("");
                    let mark = link_target(url).map(|t| self.link_mark(t));
                    if let Some(idx) = mark {
                        out.push(LItem::LinkOpen(idx));
                    }
                    self.emit_inlines(
                        text,
                        base,
                        node.content.start.max(ns),
                        node.content.end.min(ne),
                        &node.children,
                        Style {
                            color: COL_LINK,
                            underline: true,
                            ..style
                        },
                        out,
                        media,
                    );
                    if mark.is_some() {
                        out.push(LItem::LinkClose);
                    }
                }
                InlineKind::Image => {
                    let url = node
                        .url
                        .and_then(|r| {
                            text.get(r.start.saturating_sub(base)..r.end.saturating_sub(base))
                        })
                        .unwrap_or("");
                    let alt = text
                        .get(
                            node.content.start.saturating_sub(base)
                                ..node.content.end.saturating_sub(base),
                        )
                        .unwrap_or("");
                    match self.insert_image(url, media) {
                        Some(id) => out.push(LItem::Box(BoxItem {
                            id,
                            w: self.images[id].pt_w(),
                            h: self.images[id].pt_h(),
                            baseline: self.images[id].pt_h(),
                            align: None,
                        })),
                        None => {
                            let st = Style {
                                italic: true,
                                color: COL_MUTED,
                                ..style
                            };
                            let fallback = if self.captioned_image == Some(node.range) {
                                t(Key::ImageAltFallback)
                            } else {
                                alt
                            };
                            for c in fallback.chars() {
                                out.push(LItem::Ch(c, st));
                            }
                        }
                    }
                }
                InlineKind::Math => {
                    let tex = text
                        .get(
                            node.content.start.saturating_sub(base)
                                ..node.content.end.saturating_sub(base),
                        )
                        .unwrap_or("");
                    match self.insert_math(tex, false, media) {
                        Some(id) => {
                            let im = &self.images[id];
                            out.push(LItem::Box(BoxItem {
                                id,
                                w: im.pt_w(),
                                h: im.pt_h(),
                                baseline: im.baseline_pt(),
                                align: None,
                            }))
                        }
                        None => {
                            let st = Style {
                                color: COL_MUTED,
                                ..style
                            };
                            for c in format!("${tex}$").chars() {
                                out.push(LItem::Ch(c, st));
                            }
                        }
                    }
                }
                InlineKind::FootnoteRef => {
                    let id_txt = text
                        .get(
                            node.content.start.saturating_sub(base)
                                ..node.content.end.saturating_sub(base),
                        )
                        .unwrap_or("");
                    let num = self.footnotes.get(id_txt).copied();
                    match num {
                        Some(num) => {
                            // 首次引用位置 = 文末脚注的返回锚点；引用数字链接到定义。
                            self.anchors
                                .entry(format!("fnref:{id_txt}"))
                                .or_insert((self.page, self.y));
                            let idx = self.link_mark(LinkTarget::Anchor(format!("fn:{id_txt}")));
                            out.push(LItem::LinkOpen(idx));
                            for c in format!("[{num}]").chars() {
                                out.push(LItem::Ch(
                                    c,
                                    Style {
                                        color: COL_LINK,
                                        size: style.size * 0.85,
                                        ..style
                                    },
                                ));
                            }
                            out.push(LItem::LinkClose);
                        }
                        // 引用无定义：保留源码原文（可读），不伪造编号/断链。
                        None => push_chars(
                            out,
                            text,
                            base,
                            node.range.start,
                            node.range.end,
                            Style {
                                color: COL_LINK,
                                size: style.size * 0.85,
                                ..style
                            },
                        ),
                    }
                }
                // Task 3-5：PDF 渲染层按 tag 分派 Html 标签。
                // kbd: 粗体（kbd 外框在 PDF 里简化为粗体+深色字，paint.rs 才画描边）
                // sub/sup: 缩放 0.7（PDF 暂只缩字，偏移留后续）
                // mark: 加 highlight 底色
                // code: 同行内代码样式（等宽+浅底）
                // a: 链接色
                // strong/b/em/i/cite/u/ins/s/del/small/var/q 与预览同语义；
                // style 属性在解析层已净化，此处按白名单语义叠加。
                InlineKind::Html { tag, ref attrs } => {
                    let mut st = match tag {
                        veil_core::HtmlTag::Kbd => Style {
                            bold: true,
                            ..style
                        },
                        veil_core::HtmlTag::Strong | veil_core::HtmlTag::B => Style {
                            bold: true,
                            ..style
                        },
                        veil_core::HtmlTag::Em
                        | veil_core::HtmlTag::I
                        | veil_core::HtmlTag::Cite
                        | veil_core::HtmlTag::Var => Style {
                            italic: true,
                            ..style
                        },
                        veil_core::HtmlTag::U | veil_core::HtmlTag::Ins => Style {
                            underline: true,
                            ..style
                        },
                        veil_core::HtmlTag::S | veil_core::HtmlTag::Del => Style {
                            strike: true,
                            ..style
                        },
                        veil_core::HtmlTag::Small => Style {
                            size: style.size * 0.85,
                            ..style
                        },
                        veil_core::HtmlTag::Sub => Style {
                            size: style.size * 0.7,
                            ..style
                        },
                        veil_core::HtmlTag::Sup => Style {
                            size: style.size * 0.7,
                            ..style
                        },
                        veil_core::HtmlTag::Mark => Style {
                            highlight: Some([255, 247, 153]),
                            ..style
                        },
                        veil_core::HtmlTag::Code => Style {
                            highlight: Some(COL_CODE_BG),
                            ..style
                        },
                        veil_core::HtmlTag::A => Style {
                            color: COL_LINK,
                            underline: true,
                            ..style
                        },
                        _ => style,
                    };
                    if let Some(raw) = attrs.style.as_deref() {
                        let hs = veil_core::HtmlStyle::parse(raw);
                        st.bold |= hs.bold;
                        st.italic |= hs.italic;
                        st.underline |= hs.underline;
                        st.strike |= hs.strike;
                        if let Some(c) = hs.color {
                            st.color = c;
                        }
                        if let Some(bg) = hs.background_color {
                            st.highlight = Some(bg);
                        }
                        if let Some(px) = hs.size_px {
                            // CSS px → PDF pt（96dpi 参考）。
                            st.size = px * 0.75;
                        }
                    }
                    // `<q>` 双引号包裹：与预览/导出语义一致。
                    let is_q = matches!(tag, veil_core::HtmlTag::Q);
                    if is_q {
                        out.push(LItem::Ch('“', st));
                    }
                    self.emit_inlines(
                        text,
                        base,
                        node.content.start.max(ns),
                        node.content.end.min(ne),
                        &node.children,
                        st,
                        out,
                        media,
                    );
                    if is_q {
                        out.push(LItem::Ch('”', st));
                    }
                }
                // 自闭合白名单：br 断行（PDF 里输出一个空格，让 fill_lines 在空格处自然换行；
                // 未来可在 LItem 加 Br variant 做硬断行）、img 复用现有图片路径
                InlineKind::HtmlSelfClosed { tag, ref attrs } => match tag {
                    veil_core::HtmlTag::Br => out.push(LItem::Ch(' ', style)),
                    veil_core::HtmlTag::Img => {
                        // 远程 src 解析层已置空；本地路径复用 insert_image
                        let url = attrs.src.as_deref().unwrap_or("");
                        let alt = attrs.alt.as_deref().unwrap_or("");
                        match self.insert_image(url, media) {
                            Some(id) => out.push(LItem::Box(BoxItem {
                                id,
                                w: self.images[id].pt_w(),
                                h: self.images[id].pt_h(),
                                baseline: self.images[id].pt_h(),
                                align: attrs.align.filter(|_| {
                                    veil_core::image_standalone(
                                        text,
                                        Range::new(base, base + text.len()),
                                        node.range,
                                    )
                                }),
                            })),
                            None => {
                                let st = Style {
                                    italic: true,
                                    color: COL_MUTED,
                                    ..style
                                };
                                let fallback = if self.captioned_image == Some(node.range) {
                                    t(Key::ImageAltFallback)
                                } else {
                                    alt
                                };
                                for c in fallback.chars() {
                                    out.push(LItem::Ch(c, st));
                                }
                            }
                        }
                    }
                    _ => {}
                },
                // 非白名单 HTML 源码原样：PDF 里不画尖括号（escape 留 export.rs 处理），
                // 只输出子节点（HtmlLiteral 无子节点，等价于整段跳过）。
                InlineKind::HtmlLiteral => {}
            }
            cur = cur.max(ne);
        }
        if cur < to {
            push_chars(out, text, base, cur, to, style);
        }
    }

    fn insert_image(
        &mut self,
        url: &str,
        media: &mut Option<&mut dyn MediaProvider>,
    ) -> Option<usize> {
        if url.is_empty() || url.starts_with("http://") || url.starts_with("https://") {
            return None;
        }
        let m = media.as_deref_mut()?;
        let data = m.image(Path::new(url)).ok()?;
        Some(self.push_image(data))
    }

    fn insert_math(
        &mut self,
        tex: &str,
        display: bool,
        media: &mut Option<&mut dyn MediaProvider>,
    ) -> Option<usize> {
        if tex.trim().is_empty() {
            return None;
        }
        let m = media.as_deref_mut()?;
        let data = m.math(tex, display).ok()?;
        Some(self.push_image(data))
    }

    fn push_image(&mut self, mut data: ImageData) -> usize {
        // 版心宽度约束（等比）
        let max_w = self.max_w();
        let w = data.pt_w();
        if w > max_w && w > 0.0 {
            let k = max_w / w;
            data.px_to_pt *= k;
        }
        let h = data.pt_h();
        // B4：限高与编辑视图一致（行高倍数），同时保留整页安全上限。
        let max_h = (self.img_max_lines * self.metrics.line_height(BODY_SIZE))
            .min(self.geom.content_h() * 0.8);
        if h > max_h && h > 0.0 {
            data.px_to_pt *= max_h / h;
        }
        self.images.push(data);
        self.images.len() - 1
    }

    fn push_mermaid_image(&mut self, mut data: ImageData) -> usize {
        let max_w = self.max_w();
        let w = data.pt_w();
        if w > max_w && w > 0.0 {
            data.px_to_pt *= max_w / w;
        }
        let h = data.pt_h();
        // 块前 4pt、块后 8pt 也必须留在同一页内；不套用行内图片的两行限高。
        let max_h = (self.geom.content_h() - 12.0).max(1.0);
        if h > max_h && h > 0.0 {
            data.px_to_pt *= max_h / h;
        }
        self.images.push(data);
        self.images.len() - 1
    }

    fn code(&mut self, s: &Session, id: BlockId, start: usize, end: usize, fenced: bool) {
        let code = if fenced {
            s.fenced_code_source(id).unwrap_or((None, String::new()))
        } else {
            (
                None,
                s.text_slice(veil_core::Range { start, end })
                    .unwrap_or_default()
                    .lines()
                    .map(|l| l.strip_prefix("    ").unwrap_or(l))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        };
        let (lang, body) = code;
        let st = Style {
            size: CODE_SIZE,
            ..Style::body()
        };
        let line_h = self.metrics.line_height(CODE_SIZE);
        let pad = 5.0;
        let bg_w = self.max_w() - 6.0;
        // DEF-T09-E1-005：代码语言标题不得与代码体分页（标题孤行）。
        // 预留「间距 + 标题行 + 首行代码（含底色内边距）」后再落排版位置。
        let label_h = lang
            .as_deref()
            .filter(|l| !l.is_empty())
            .map_or(0.0, |_| self.metrics.line_height(8.0));
        let first_body_h = if body.lines().next().is_some() {
            line_h + pad * 2.0
        } else {
            0.0
        };
        self.ensure(5.0 + label_h + first_body_h);
        self.y += 5.0;
        if let Some(l) = lang.as_deref().filter(|l| !l.is_empty()) {
            let st_label = Style {
                size: 8.0,
                color: COL_MUTED,
                ..Style::body()
            };
            let mut items = Vec::new();
            for c in l.chars() {
                items.push(LItem::Ch(c, st_label));
            }
            if !items.is_empty() {
                for line in fill_lines(items, self.max_w() - 10.0, self.metrics) {
                    self.emit_line(line, 10.0, 8.0, CellAlign::None, self.max_w() - 10.0);
                }
            }
        }

        for raw in body.lines() {
            let mut chars: Vec<LItem> = raw.chars().map(|c| LItem::Ch(c, st)).collect();
            if chars.is_empty() {
                chars.push(LItem::Ch(' ', st));
            }
            let wrapped = fill_lines(chars, bg_w - 2.0 * pad, self.metrics);
            let n = wrapped.len() as f32;
            self.ensure(line_h * n + pad * 2.0);
            let top = self.y;
            self.pages[self.page].push(DrawOp::Rect {
                x: self.geom.margin_x + 3.0,
                y: top,
                w: bg_w,
                h: line_h * n + pad * 2.0,
                color: COL_CODE_BG,
            });
            self.y += pad;
            for line in wrapped {
                self.emit_line(line, 3.0 + pad, CODE_SIZE, CellAlign::None, bg_w);
            }
            self.y += pad;
        }
        self.y += 6.0;
    }

    /// 引用/提示块。`callout` 为 `(标签, 图标, 强调色)`；None = 普通引用块。
    /// 标签与图标来自内核统一类型（内置五种 / 自定义配置），三通道同源。
    fn quote(
        &mut self,
        s: &Session,
        id: BlockId,
        start: usize,
        end: usize,
        callout: Option<(String, String, [u8; 3])>,
        media: &mut Option<&mut dyn MediaProvider>,
    ) {
        let raw = slice_text(s, start, end);
        let text = blank_container_details_tags(&raw);
        let nodes = s.block_inlines(id).unwrap_or_default();
        let rail_color = callout.as_ref().map_or(COL_RULE, |(_, _, c)| *c);
        let start_y = self.y;
        let page = self.page;
        let bg_at = self.pages[page].len();
        let indent = 16.0;
        let mut first = true;
        for ((li, line), raw_line) in text.split('\n').enumerate().zip(raw.split('\n')) {
            // 该行在块文本中的相对起点
            let rel: usize = text.split('\n').take(li).map(|l| l.len() + 1).sum();
            let b = line.as_bytes();
            let mut i = 0;
            // 嵌套引用每行可能有多层 `>` 标记：逐层剥除，绝不把字面 `>` 带进正文。
            loop {
                while i < b.len() && matches!(b[i], b' ' | b'\t') {
                    i += 1;
                }
                if i < b.len() && b[i] == b'>' {
                    i += 1;
                    if i < b.len() && matches!(b[i], b' ' | b'\t') {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            let mut cs = i;
            let mut label: Option<(String, [u8; 3])> = None;
            if first {
                // 首行 `[!TYPE]` 标记一律不进入 PDF 文本：内置/自定义换成标签，
                // 未配置类型（普通引用回退）直接跳过标记，绝不显示裸 `[!XXX]`。
                let rest = &line[cs..];
                if rest.starts_with("[!") {
                    if let Some(close) = rest.find(']') {
                        cs += close + 1;
                        if let Some((name, icon, color)) = &callout {
                            let label_text = if icon.is_empty() {
                                name.clone()
                            } else {
                                format!("{icon} {name}")
                            };
                            label = Some((label_text, *color));
                        }
                    }
                }
            }
            first = false;
            let from = start + rel + cs;
            let to = start + rel + line.trim_end().len();
            if from >= to && label.is_none() {
                // 纯 `<details>`/`</details>` 结构行：消隐且不占行高。
                if raw_line.contains('<') {
                    continue;
                }
                self.blank(BODY_SIZE);
                continue;
            }
            if let Some((name, color)) = label {
                let st = Style {
                    size: 10.0,
                    bold: true,
                    color,
                    ..Style::body()
                };
                let items: Vec<LItem> = name.chars().map(|c| LItem::Ch(c, st)).collect();
                for l in fill_lines(items, self.max_w() - indent, self.metrics) {
                    self.emit_line(l, indent, 10.0, CellAlign::None, self.max_w() - indent);
                }
            }
            if from < to {
                self.rich(
                    text.as_ref(),
                    start,
                    from,
                    to,
                    &nodes,
                    Style::body(),
                    indent,
                    media,
                );
            }
        }
        let h = self.y - start_y;
        if h > 2.0 {
            // 背景先于文字绘制：把整块浅底色插到本块第一个绘制指令之前。
            if callout.is_some() && self.page == page {
                let bg_w = self.max_w() + 4.0;
                self.pages[page].insert(
                    bg_at,
                    DrawOp::Rect {
                        x: self.geom.margin_x,
                        y: start_y,
                        w: bg_w,
                        h: h - 2.0,
                        color: tint_on_white(rail_color, 0.10),
                    },
                );
            }
            self.pages[self.page].push(DrawOp::Rect {
                x: self.geom.margin_x + 1.0,
                y: start_y + 2.0,
                w: 3.0,
                h: h - 4.0,
                color: rail_color,
            });
        }
        self.y += 6.0;
    }

    #[allow(clippy::too_many_arguments)]
    fn list_item(
        &mut self,
        s: &Session,
        id: BlockId,
        start: usize,
        end: usize,
        ordered: bool,
        depth: u8,
        task: Option<bool>,
        media: &mut Option<&mut dyn MediaProvider>,
    ) {
        let raw = slice_text(s, start, end);
        let text = blank_container_details_tags(&raw);
        let visible = text.trim_end().len();
        let (prefix, content_rel) = list_prefix(&text, ordered, task.is_some());
        let nodes = s.block_inlines(id).unwrap_or_default();
        let indent = 16.0 + f32::from(depth) * 14.0;

        let mut items: Vec<LItem> = Vec::new();
        let st = Style::body();
        for c in prefix.chars() {
            items.push(LItem::Ch(c, st));
        }
        // 行内内容（从标记后开始）
        let base = start;
        let from = start + content_rel;
        let to = start + visible;
        if from < to {
            self.emit_inlines(&text, base, from, to, &nodes, st, &mut items, media);
        }
        let avail = (self.max_w() - indent).max(40.0);
        for line in fill_lines(items, avail, self.metrics) {
            self.emit_line(line, indent, BODY_SIZE, CellAlign::None, avail);
        }
        self.y += 2.0;
    }

    fn table(&mut self, s: &Session, id: BlockId, media: &mut Option<&mut dyn MediaProvider>) {
        let Some(t) = s.table(id) else { return };
        // B2：容器内表格按容器缩进（引用层/列表内容列），可用宽同步扣减。
        let container = s.block_container(id);
        let indent =
            f32::from(container.quote_depth) * 14.0 + f32::from(container.list_indent) * 6.0;
        let x0 = self.geom.margin_x + indent;
        let max_w = (self.max_w() - indent).max(40.0);
        let ncols = t.aligns.len().max(1);
        let pad = 4.5;
        let line_h = self.metrics.line_height(TABLE_SIZE);

        // 自然列宽
        let mut nat = vec![48.0f32; ncols];
        for (ci, c) in t.header.iter().enumerate().take(ncols) {
            nat[ci] = nat[ci].max(self.metrics.text_width(&c.text, TABLE_SIZE));
        }
        for row in &t.rows {
            for (ci, c) in row.iter().enumerate().take(ncols) {
                nat[ci] = nat[ci].max(self.metrics.text_width(&c.text, TABLE_SIZE));
            }
        }
        for w in nat.iter_mut() {
            *w += pad * 2.0;
        }
        let total: f32 = nat.iter().sum();
        let k = if total > max_w { max_w / total } else { 1.0 };
        let widths: Vec<f32> = nat.iter().map(|w| (w * k).max(26.0)).collect();

        let mut rows: Vec<&Vec<veil_core::TableCell>> = Vec::new();
        rows.push(&t.header);
        for r in &t.rows {
            rows.push(r);
        }

        for (ri, row) in rows.iter().enumerate() {
            // 单元格按 inlines 排版（粗体/斜体/行内代码/链接与正文同源），
            // 不再截断到 6 行：内容按页拆分完整输出（行可跨页，续页继续）。
            let base_style = Style {
                size: TABLE_SIZE,
                bold: ri == 0,
                ..Style::body()
            };
            let mut cell_lines: Vec<Vec<Vec<LItem>>> = Vec::new();
            let mut max_lines = 1usize;
            for (ci, &cw) in widths.iter().enumerate() {
                let inner_w = (cw - 2.0 * pad).max(12.0);
                let mut items: Vec<LItem> = Vec::new();
                if let Some(cell) = row.get(ci) {
                    let raw = s.text_slice(cell.range).unwrap_or_default();
                    // 单元格源码是文档切片，base = cell.range.start：
                    // `emit_inlines` 的节点区间是文档绝对偏移，两者口径一致。
                    self.emit_inlines(
                        &raw,
                        cell.range.start,
                        cell.range.start,
                        cell.range.end,
                        &cell.inlines,
                        base_style,
                        &mut items,
                        media,
                    );
                }
                let mut lines = fill_lines(items, inner_w, self.metrics);
                if lines.is_empty() {
                    lines.push(Vec::new());
                }
                max_lines = max_lines.max(lines.len());
                cell_lines.push(lines);
            }

            let full_h = line_h * max_lines as f32 + pad * 1.4;
            // 整行能放进一页时，尽量不拆行（避免表格行孤悬）。
            if full_h <= self.geom.content_h()
                && self.y + full_h > self.geom.bottom()
                && self.y > self.geom.top() + 1.0
            {
                self.new_page();
            }
            // 跨页拆行：续页从下一行继续，表头边框在每页重画。
            let mut drawn = 0usize;
            let mut first_chunk = true;
            let mut cell_links: Vec<Option<(u32, f32)>> = vec![None; ncols];
            while drawn < max_lines {
                // 行放不下就换页再画：避免页底只剩几 pt 时 fit 被强制为 1，
                // 使基线越过正文底（文字被裁切或压到页脚）。每页重画边框。
                if self.y + line_h + pad * 1.4 > self.geom.bottom()
                    && self.y > self.geom.top() + 1.0
                {
                    self.new_page();
                }
                let avail = (self.geom.bottom() - self.y - pad * 1.4).max(0.0);
                let fit = ((avail / line_h).floor() as usize)
                    .max(1)
                    .min(max_lines - drawn);
                let row_h = line_h * fit as f32 + pad * 1.4;
                let top = self.y;
                if ri == 0 && first_chunk {
                    self.pages[self.page].push(DrawOp::Rect {
                        x: x0,
                        y: top,
                        w: widths.iter().sum(),
                        h: row_h,
                        color: COL_HEAD_BG,
                    });
                }

                // 单元格文本（保留样式与链接矩形）
                let mut x = x0;
                for ci in 0..ncols {
                    let align = t.aligns.get(ci).copied().unwrap_or_default();
                    let inner = widths[ci] - 2.0 * pad;
                    let mut cy = top + pad * 0.7;
                    for k in 0..fit {
                        let line: &[LItem] = cell_lines[ci]
                            .get(drawn + k)
                            .map(Vec::as_slice)
                            .unwrap_or(&[]);
                        self.emit_cell_line(
                            line,
                            x + pad,
                            inner,
                            cy,
                            line_h,
                            align,
                            &mut cell_links[ci],
                        );
                        cy += line_h;
                    }
                    x += widths[ci];
                }

                // 横线
                let total_w: f32 = widths.iter().sum();
                for yy in [top, top + row_h] {
                    self.pages[self.page].push(DrawOp::Line {
                        x1: x0,
                        y1: yy,
                        x2: x0 + total_w,
                        y2: yy,
                        width: 0.6,
                        color: COL_RULE,
                    });
                }
                // 竖线
                let mut xs = Vec::with_capacity(widths.len() + 1);
                let mut vx = x0;
                xs.push(vx);
                for &cw in &widths {
                    vx += cw;
                    xs.push(vx);
                }
                for vx in xs {
                    self.pages[self.page].push(DrawOp::Line {
                        x1: vx,
                        y1: top,
                        x2: vx,
                        y2: top + row_h,
                        width: 0.6,
                        color: COL_RULE,
                    });
                }
                self.y += row_h;
                drawn += fit;
                first_chunk = false;
                if drawn < max_lines {
                    self.new_page();
                }
            }
        }
        self.y += 8.0;
    }

    /// 在表格单元格内画一行（不推进全局 `y`）。
    ///
    /// 与 [`Ctx::emit_line`] 共用样式 run / 链接矩形语义，但位置由单元格几何
    /// （`x` / `top`）给定；行内在 `LItem` 里携带的粗体、斜体、行内代码底色、
    /// 链接颜色与下划线因此都能落到 PDF。
    #[allow(clippy::too_many_arguments)]
    fn emit_cell_line(
        &mut self,
        line: &[LItem],
        x: f32,
        avail_w: f32,
        top: f32,
        line_h: f32,
        align: CellAlign,
        open: &mut Option<(u32, f32)>,
    ) {
        let wid: f32 = line.iter().map(|i| item_width(i, self.metrics)).sum();
        let slack = (avail_w - wid).max(0.0);
        let mut cx = x + match align {
            CellAlign::Center => slack / 2.0,
            CellAlign::Right => slack,
            _ => 0.0,
        };
        let baseline = top + self.metrics.ascent(TABLE_SIZE);
        let mut buf = String::new();
        let mut style: Option<Style> = None;
        // 跨行链接：本行从行首重新起算，行尾再闭合（每行一个可点击矩形）。
        if let Some((idx, _)) = *open {
            *open = Some((idx, cx));
        }
        for it in line {
            match it {
                LItem::LinkOpen(idx) => {
                    flush_run(
                        &mut self.pages,
                        self.page,
                        &mut buf,
                        &mut style,
                        &mut cx,
                        self.metrics,
                        baseline,
                    );
                    *open = Some((*idx, cx));
                }
                LItem::LinkClose => {
                    flush_run(
                        &mut self.pages,
                        self.page,
                        &mut buf,
                        &mut style,
                        &mut cx,
                        self.metrics,
                        baseline,
                    );
                    if let Some((idx, sx)) = open.take() {
                        self.push_link(idx, sx, cx, top, line_h);
                    }
                }
                LItem::Ch(c, st) => {
                    if style != Some(*st) {
                        flush_run(
                            &mut self.pages,
                            self.page,
                            &mut buf,
                            &mut style,
                            &mut cx,
                            self.metrics,
                            baseline,
                        );
                        style = Some(*st);
                    }
                    buf.push(*c);
                }
                LItem::Box(b) => {
                    flush_run(
                        &mut self.pages,
                        self.page,
                        &mut buf,
                        &mut style,
                        &mut cx,
                        self.metrics,
                        baseline,
                    );
                    self.pages[self.page].push(DrawOp::Image {
                        x: cx,
                        y: baseline - b.baseline,
                        w: b.w,
                        h: b.h,
                        id: b.id,
                    });
                    cx += b.w;
                }
            }
        }
        flush_run(
            &mut self.pages,
            self.page,
            &mut buf,
            &mut style,
            &mut cx,
            self.metrics,
            baseline,
        );
        if let Some((idx, sx)) = *open {
            self.push_link(idx, sx, cx, top, line_h);
        }
    }

    fn block_math(&mut self, s: &Session, id: BlockId, media: &mut Option<&mut dyn MediaProvider>) {
        let tex = s.math_source(id).unwrap_or_default();
        self.y += 4.0;
        if let Some(idx) = self.insert_math(&tex, true, media) {
            let im = &self.images[idx];
            let (w, h) = (im.pt_w(), im.pt_h());
            self.ensure(h + 8.0);
            let x = self.geom.margin_x + (self.max_w() - w).max(0.0) / 2.0;
            let top = self.y;
            self.pages[self.page].push(DrawOp::Image {
                x,
                y: top,
                w,
                h,
                id: idx,
            });
            self.y += h;
        } else {
            let st = Style {
                size: BODY_SIZE,
                color: COL_MUTED,
                ..Style::body()
            };
            for line in tex.lines() {
                let items: Vec<LItem> = line.chars().map(|c| LItem::Ch(c, st)).collect();
                for l in fill_lines(items, self.max_w(), self.metrics) {
                    self.emit_line(l, 0.0, BODY_SIZE, CellAlign::Center, self.max_w());
                }
            }
        }
        self.y += 8.0;
    }

    /// Mermaid 图表块：优先用缓存位图（若 SVG 可用则重新高 DPI 光栅化），
    /// 嵌入 DrawOp::Image；无数据时降级为源码代码块。
    fn mermaid_block(&mut self, s: &Session, id: BlockId, start: usize, end: usize) {
        self.y += 4.0;
        // 优先用 SVG 重新按 300dpi 光栅化（PDF 高 DPI 清晰）；否则用缓存位图。
        let bitmap = if let Some(m) = self.mermaids.get(&id) {
            if let Some(svg) = &m.svg_text {
                crate::platform::mermaid::svg_to_bitmap(svg, 300.0 / 96.0 * 2.0).ok()
            } else {
                None
            }
            .or_else(|| {
                Some(crate::platform::mermaid::MermaidBitmap {
                    data: m.data.clone(),
                    width: m.width,
                    height: m.height,
                    px_to_pt: m.px_to_pt,
                })
            })
        } else {
            None
        };
        if let Some(bm) = bitmap {
            let rgb = premultiplied_rgba_to_rgb_on_white(&bm.data);
            // px_to_pt：从 MermaidBitmap 字段取（resvg 光栅化时已按实际 scale 算好）
            let px_to_pt = bm.px_to_pt;
            let data = ImageData {
                w: bm.width,
                h: bm.height,
                rgb,
                baseline_px: 0.0,
                px_to_pt,
            };
            let idx = self.push_mermaid_image(data);
            let im = &self.images[idx];
            let (w, h) = (im.pt_w(), im.pt_h());
            self.ensure(h + 8.0);
            let x = self.geom.margin_x + (self.max_w() - w).max(0.0) / 2.0;
            let top = self.y;
            self.pages[self.page].push(DrawOp::Image {
                x,
                y: top,
                w,
                h,
                id: idx,
            });
            self.y += h + 8.0;
        } else {
            // 无 Mermaid 数据：降级为源码代码块
            self.code(s, id, start, end, true);
        }
    }

    /// details 头块的 summary 行（B3）：PDF 恒展开，只画标题行，正文由子块排版。
    /// 箭头用 ▼（U+25BC）而非 ▾（U+25BE），保证微软雅黑等系统字体能渲染。
    fn details_summary(&mut self, summary: &str) {
        let st = Style {
            size: 10.0,
            color: COL_TEXT,
            bold: true,
            ..Style::body()
        };
        let mut items = vec![LItem::Ch('▼', st), LItem::Ch(' ', st)];
        items.extend(summary.chars().map(|c| LItem::Ch(c, st)));
        let size = 10.0;
        for l in fill_lines(items, self.max_w(), self.metrics) {
            self.emit_line(l, 0.0, size, CellAlign::None, self.max_w());
        }
        self.y += 4.0;
    }

    fn raw_text(&mut self, s: &Session, start: usize, end: usize, size: f32) {
        let text = slice_text(s, start, end);
        let st = Style {
            size,
            color: COL_MUTED,
            ..Style::body()
        };
        for line in text.lines() {
            let items: Vec<LItem> = line.chars().map(|c| LItem::Ch(c, st)).collect();
            if items.is_empty() {
                self.blank(size);
                continue;
            }
            for l in fill_lines(items, self.max_w(), self.metrics) {
                self.emit_line(l, 0.0, size, CellAlign::None, self.max_w());
            }
        }
        self.y += 4.0;
    }

    fn rule(&mut self) {
        self.ensure(20.0);
        self.y += 6.0;
        let (left, right, y) = (
            self.geom.margin_x,
            self.geom.margin_x + self.max_w(),
            self.y,
        );
        self.pages[self.page].push(DrawOp::Line {
            x1: left,
            y1: y,
            x2: right,
            y2: y,
            width: 0.8,
            color: COL_RULE,
        });
        self.y += 10.0;
    }

    /// 文末脚注区：只输出「被引用且已定义」的条目，编号与引用处一致。
    /// 定义正文剥除 `[^id]:` 前缀并去掉 Markdown 标记（不留裸 `**`/反引号）。
    fn footnote_section(&mut self, s: &Session) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut items: Vec<(usize, String, String)> = Vec::new();
        for (_, kind, range) in s.blocks() {
            if kind != BlockKind::FootnoteDef {
                continue;
            }
            let text = slice_text(s, range.start, range.end);
            let Some((id, body)) = split_footnote_def(&text) else {
                continue;
            };
            let Some(num) = self.footnotes.get(&id).copied() else {
                continue; // 未被引用：不占号也不输出
            };
            if !seen.insert(id.clone()) {
                continue; // 重复定义 first wins
            }
            items.push((num, id, footnote_plain_text(&body)));
        }
        if items.is_empty() {
            return;
        }
        items.sort_by_key(|(num, _, _)| *num);
        self.rule();
        let st = Style {
            size: 9.0,
            color: COL_MUTED,
            ..Style::body()
        };
        for (num, id, body) in items {
            // 定义项锚点：正文引用 `[n]` → 此处；本项编号 → 首次引用位置。
            self.ensure(self.metrics.line_height(9.0));
            self.anchors.insert(format!("fn:{id}"), (self.page, self.y));
            let idx = self.link_mark(LinkTarget::Anchor(format!("fnref:{id}")));
            let mut items: Vec<LItem> = Vec::new();
            items.push(LItem::LinkOpen(idx));
            for c in format!("[{num}]").chars() {
                items.push(LItem::Ch(c, st));
            }
            items.push(LItem::LinkClose);
            for c in format!(" {body}").chars() {
                items.push(LItem::Ch(c, st));
            }
            for line in fill_lines(items, self.max_w().max(24.0), self.metrics) {
                self.emit_line(line, 0.0, 9.0, CellAlign::None, self.max_w());
            }
        }
        self.y += 4.0;
    }

    // —— 目录 / 页脚 ——

    fn insert_toc(&mut self) -> usize {
        let row_h = 19.0;
        let cap = (((self.geom.content_h() - 60.0) / row_h).floor() as usize).max(1);
        let chunks: Vec<Vec<Head>> = self
            .heads
            .chunks(cap)
            .map(|c| {
                c.iter()
                    .map(|h| Head {
                        level: h.level,
                        text: h.text.clone(),
                        page: h.page,
                        top_y: h.top_y,
                    })
                    .collect()
            })
            .collect();
        let toc_n = chunks.len();
        let mut pages: Vec<Vec<DrawOp>> = Vec::new();
        let mut toc_links: Vec<LinkAnnot> = Vec::new();
        for (pi, chunk) in chunks.iter().enumerate() {
            let mut ops: Vec<DrawOp> = Vec::new();
            let mut y = self.geom.top();
            ops.push(DrawOp::Text(TextRun {
                x: self.geom.margin_x,
                y: y + self.metrics.ascent(18.0),
                size: 18.0,
                text: t(Key::PrintTocTitle).into(),
                bold: true,
                italic: false,
                monospace: false,
                underline: false,
                strike: false,
                highlight: None,
                color: COL_TEXT,
            }));
            y += 34.0;
            for h in chunk.iter() {
                let indent = f32::from(h.level.saturating_sub(1)) * 12.0;
                let num = format!("{}", h.page + toc_n + 1);
                let size = if h.level <= 1 { 11.0 } else { 10.0 };
                let num_w = self.metrics.text_width(&num, size);
                let right = self.geom.width - self.geom.margin_x;
                let avail = self.max_w() - indent - num_w - 10.0;
                let mut title = h.text.clone();
                while self.metrics.text_width(&title, size) > avail && !title.is_empty() {
                    title.pop();
                }
                // 整行可点：目录页自身页码 + 最终页序中的标题位置。
                toc_links.push(LinkAnnot {
                    page: pi,
                    x: self.geom.margin_x,
                    y,
                    w: self.geom.content_w(),
                    h: row_h,
                    target: LinkTarget::Internal {
                        page: h.page + toc_n,
                        x: self.geom.margin_x + indent,
                        y: h.top_y,
                    },
                });
                ops.push(DrawOp::Text(TextRun {
                    x: self.geom.margin_x + indent,
                    y: y + self.metrics.ascent(size),
                    size,
                    text: title,
                    bold: h.level <= 1,
                    italic: false,
                    monospace: false,
                    underline: false,
                    strike: false,
                    highlight: None,
                    color: COL_TEXT,
                }));
                ops.push(DrawOp::Text(TextRun {
                    x: right - num_w,
                    y: y + self.metrics.ascent(size),
                    size,
                    text: num,
                    bold: false,
                    italic: false,
                    monospace: false,
                    underline: false,
                    strike: false,
                    highlight: None,
                    color: COL_MUTED,
                }));
                y += row_h;
            }
            pages.push(ops);
        }
        let n = pages.len();
        for (i, p) in pages.into_iter().enumerate() {
            self.pages.insert(i, p);
        }
        // 目录插页后：正文链接与页内锚点的页下标整体后移；
        // 目录行自身的注释已按最终页序生成，直接并入。
        for l in &mut self.links {
            l.page += n;
        }
        for (p, _) in self.anchors.values_mut() {
            *p += n;
        }
        self.links.extend(toc_links);
        n
    }

    fn append_footers(&mut self, enabled: bool) {
        if !enabled {
            return;
        }
        let total = self.pages.len();
        let footer_y = (self.geom.height - self.geom.margin_y * 0.5).min(self.geom.height - 18.0);
        let (margin_x, max_w) = (self.geom.margin_x, self.max_w());
        for (i, page) in self.pages.iter_mut().enumerate() {
            let text = format!("{} / {}", i + 1, total);
            let size = 9.0;
            let w = self.metrics.text_width(&text, size);
            page.push(DrawOp::Text(TextRun {
                x: margin_x + (max_w - w).max(0.0) / 2.0,
                y: footer_y,
                size,
                text,
                bold: false,
                italic: false,
                monospace: false,
                underline: false,
                strike: false,
                highlight: None,
                color: COL_MUTED,
            }));
        }
    }
}

/// tiny-skia 输出预乘 RGBA；DeviceRGB 没有 alpha，写入 PDF 前先合成到白底。
fn premultiplied_rgba_to_rgb_on_white(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.chunks_exact(4) {
        let white = 255 - px[3];
        rgb.push(px[0].saturating_add(white));
        rgb.push(px[1].saturating_add(white));
        rgb.push(px[2].saturating_add(white));
    }
    rgb
}

fn flush_run(
    pages: &mut [Vec<DrawOp>],
    page: usize,
    buf: &mut String,
    style: &mut Option<Style>,
    x: &mut f32,
    m: &dyn TextMetrics,
    baseline: f32,
) {
    if buf.is_empty() {
        *style = None;
        return;
    }
    if let Some(s) = *style {
        let w = m.text_width_styled(buf, s.size, s.monospace);
        pages[page].push(DrawOp::Text(TextRun {
            x: *x,
            y: baseline,
            size: s.size,
            text: std::mem::take(buf),
            bold: s.bold,
            italic: s.italic,
            monospace: s.monospace,
            underline: s.underline,
            strike: s.strike,
            highlight: s.highlight,
            color: s.color,
        }));
        *x += w;
    }
    *style = None;
}

fn push_chars(out: &mut Vec<LItem>, text: &str, base: usize, from: usize, to: usize, style: Style) {
    if from >= to || from < base {
        return;
    }
    // from/to 是文档绝对偏移，text 是本块切片 → 必须先减去 base 再与切片长度比较。
    let (a, b) = (from - base, to - base);
    if b > text.len() || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
        return;
    }
    // CommonMark 反斜杠转义：`\*` 显示字面 `*`；行尾 `\` + 换行是硬换行标记，
    // 反斜杠本身不显示（与编辑器/HTML 一致）。其余反斜杠保持原样。
    let mut it = text[a..b].chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.peek().copied() {
                Some('\n') | Some('\r') => continue,
                Some(n) if n.is_ascii_punctuation() => {
                    it.next();
                    out.push(LItem::Ch(n, style));
                    continue;
                }
                _ => {}
            }
        }
        out.push(LItem::Ch(if c == '\n' || c == '\t' { ' ' } else { c }, style));
    }
}

/// 代码等原文片段：不解释反斜杠转义，仅把换行/制表符折叠为空格。
fn push_raw_chars(
    out: &mut Vec<LItem>,
    text: &str,
    base: usize,
    from: usize,
    to: usize,
    style: Style,
) {
    if from >= to || from < base {
        return;
    }
    let (a, b) = (from - base, to - base);
    if b > text.len() || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
        return;
    }
    for c in text[a..b].chars() {
        out.push(LItem::Ch(
            if c == '\n' || c == '\t' { ' ' } else { c },
            style,
        ));
    }
}

fn slice_text(s: &Session, start: usize, end: usize) -> String {
    s.text_slice(Range::new(start, end)).unwrap_or_default()
}

/// 容器块（引用/提示/列表项）内的 `<details>` 开闭标签是结构标记，不得作为正文进 PDF。
///
/// 屏幕层把这些区间折叠消隐，但容器块不是 Details 头块，标签夹在块文本里
/// （闭标签常被行内解析退化为普通文本）。这里按字节等长替换为空格，
/// 保持块内行内节点的偏移与行结构不变。
fn blank_container_details_tags(text: &str) -> Cow<'_, str> {
    let lower = text.to_ascii_lowercase();
    let mut out: Option<String> = None;
    let mut i = 0;
    while let Some(rel) = lower[i..].find('<') {
        let s = i + rel;
        let Some(gt) = lower[s..].find('>') else {
            break;
        };
        let e = s + gt + 1;
        let tag = &lower[s..e];
        if tag.starts_with("<details") || tag.starts_with("</details") {
            let buf = out.get_or_insert_with(|| text.to_string());
            buf.replace_range(s..e, &" ".repeat(e - s));
        }
        i = e;
    }
    match out {
        Some(v) => Cow::Owned(v),
        None => Cow::Borrowed(text),
    }
}

/// 已定义脚注 id → 编号（按文档首次引用递增；与 HTML 导出同口径）。
/// 缺定义的引用不编号：引用处保留原文，避免生成指向不存在条目的断号。
fn collect_footnote_numbers(s: &Session) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for (idx, id) in s.footnote_ids().iter().enumerate() {
        if s.footnote_definition(id).is_some() {
            out.insert(id.clone(), s.footnote_number(id).unwrap_or(idx + 1));
        }
    }
    out
}

/// 脚注定义块源码 → (id, 正文)。首行剥 `[^id]:`，续行剥块要求的缩进。
fn split_footnote_def(text: &str) -> Option<(String, String)> {
    let mut lines = text.split_inclusive('\n');
    let first = lines.next()?;
    let rest = first.trim_start().strip_prefix("[^")?;
    let (id, tail) = rest.split_once("]:")?;
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let mut body = tail
        .strip_prefix(' ')
        .unwrap_or(tail)
        .trim_end_matches(['\r', '\n'])
        .to_string();
    for line in lines {
        let n = line.bytes().take(4).take_while(|&b| b == b' ').count();
        body.push(' ');
        body.push_str(line[n..].trim_end_matches(['\r', '\n']));
    }
    Some((id.to_string(), body))
}

/// 脚注定义正文 → 可读纯文本：保留文字/代码/任务标记，丢弃强调/反引号等标记。
fn footnote_plain_text(md: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser};
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut out = String::new();
    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            Event::FootnoteReference(id) => out.push_str(&format!("[^{id}]")),
            Event::TaskListMarker(done) => out.push_str(if done { "[x] " } else { "[ ] " }),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 22.0,
        2 => 18.0,
        3 => 15.5,
        4 => 14.0,
        5 => 13.0,
        _ => 12.5,
    }
}

/// 列表前缀与内容起点（相对块文本的字节偏移）。
fn list_prefix(text: &str, ordered: bool, task: bool) -> (String, usize) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let mut num = String::new();
    if ordered {
        while i < b.len() && b[i].is_ascii_digit() {
            num.push(b[i] as char);
            i += 1;
        }
        if i < b.len() && (b[i] == b'.' || b[i] == b')') {
            i += 1;
        }
    } else if i < b.len() && matches!(b[i], b'-' | b'*' | b'+') {
        i += 1;
    }
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    // 任务列表复选框
    if task && i + 2 < b.len() && b[i] == b'[' {
        let done = matches!(b[i + 1], b'x' | b'X');
        i += 3;
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        return (if done { "☑ ".into() } else { "☐ ".into() }, i);
    }
    let prefix = if ordered {
        if num.is_empty() {
            "1. ".to_string()
        } else {
            format!("{num}. ")
        }
    } else {
        "• ".to_string()
    };
    (prefix, i)
}

// =============================================================================
// 入口
// =============================================================================

pub fn layout_document(
    session: &Session,
    metrics: &dyn TextMetrics,
    geom: PageGeom,
    opts: LayoutOpts,
    mut media: Option<&mut dyn MediaProvider>,
    mermaids: &std::collections::HashMap<veil_core::BlockId, veil_ui::MermaidImage>,
) -> DocLayout {
    let mut ctx = Ctx::new(metrics, geom, mermaids);
    ctx.footnotes = collect_footnote_numbers(session);
    ctx.new_page();

    for (id, kind, range) in session.blocks() {
        let (start, end) = (range.start, range.end);
        match kind {
            BlockKind::Paragraph => {
                // 纯 TOC 标记是编辑器视图指令（就地生成目录）；PDF/打印与 HTML
                // 一致，不排版字面 `[TOC]`（需要目录页时走 `opts.toc`）。
                if !session
                    .text_slice(range)
                    .is_some_and(|t| veil_core::is_toc_marker(&t))
                {
                    ctx.paragraph(session, id, start, end, 0.0, &mut media);
                }
            }
            BlockKind::AtxHeading(l) | BlockKind::SetextHeading(l) => ctx.heading(session, id, l),
            BlockKind::FencedCode(_) => {
                ctx.code(session, id, start, end, true);
            }
            // Mermaid：优先用缓存位图（若有 SVG 可重新高 DPI 光栅化），无数据降级为源码。
            BlockKind::Mermaid => ctx.mermaid_block(session, id, start, end),
            BlockKind::IndentedCode => ctx.code(session, id, start, end, false),
            BlockKind::Math => ctx.block_math(session, id, &mut media),
            BlockKind::Table => ctx.table(session, id, &mut media),
            BlockKind::ListItem {
                ordered,
                depth,
                task,
            } => ctx.list_item(session, id, start, end, ordered, depth, task, &mut media),
            BlockKind::BlockQuote => ctx.quote(session, id, start, end, None, &mut media),
            BlockKind::Admonition(ty) => ctx.quote(
                session,
                id,
                start,
                end,
                Some((ty.label().to_string(), ty.icon().to_string(), ty.color())),
                &mut media,
            ),
            BlockKind::Callout(c) => ctx.quote(
                session,
                id,
                start,
                end,
                Some((c.label.clone(), c.icon.clone(), c.color)),
                &mut media,
            ),
            BlockKind::ThematicBreak => ctx.rule(),
            BlockKind::LinkRefDef => {}
            // 脚注定义统一在文末按首次引用编号输出，不在此处泄漏源码标记。
            BlockKind::FootnoteDef => {}
            BlockKind::HtmlBlock => ctx.raw_text(session, start, end, 9.0),
            // Front Matter 是元数据：按产品语义隐藏，正文从元数据之后开始。
            BlockKind::FrontMatter(_) => {}
            // B3：Details 头块只含开标签行 + summary 行；PDF 恒按展开态，
            // 只画 summary 标题行，正文由后续子块各自按块型排版。
            BlockKind::Details { summary, .. } => ctx.details_summary(&summary),
        }
    }

    // 文末脚注区（编号与正文引用同源）。
    ctx.footnote_section(session);

    // 去掉末尾的空行留白（若最后一页只有极少量内容，保持原样）
    let toc_pages = if opts.toc && !ctx.heads.is_empty() {
        ctx.insert_toc()
    } else {
        0
    };

    ctx.append_footers(opts.page_numbers);

    let outline = ctx
        .heads
        .iter()
        .map(|h| OutlineItem {
            title: h.text.clone(),
            level: h.level,
            page_idx: h.page + toc_pages,
            top_y: h.top_y,
        })
        .collect();

    // 标题锚点：slug（按标题顺序去重）→（最终页, 标题顶）。登记两份键：
    // 原 slug 与归一化后 slug（作者手写 `#Heading` 也能命中）。
    let titles: Vec<&str> = ctx.heads.iter().map(|h| h.text.as_str()).collect();
    for (h, slug) in ctx
        .heads
        .iter()
        .zip(veil_core::heading_slugs(titles.into_iter()))
    {
        let at = (h.page + toc_pages, h.top_y);
        ctx.anchors
            .entry(veil_core::heading_slug(&slug))
            .or_insert(at);
        ctx.anchors.entry(slug).or_insert(at);
    }
    // 解析文档内锚点：命中则改为 Internal；未命中的锚点丢弃（不输出可执行动作）。
    let margin_x = ctx.geom.margin_x;
    for l in &mut ctx.links {
        let key = match &l.target {
            LinkTarget::Anchor(k) => k.clone(),
            _ => continue,
        };
        let at = ctx
            .anchors
            .get(&key)
            .copied()
            .or_else(|| ctx.anchors.get(&veil_core::heading_slug(&key)).copied());
        if let Some((page, y)) = at {
            l.target = LinkTarget::Internal {
                page,
                x: margin_x,
                y,
            };
        }
    }
    ctx.links
        .retain(|l| !matches!(l.target, LinkTarget::Anchor(_)));

    DocLayout {
        pages: ctx.pages,
        images: ctx.images,
        outline,
        links: ctx.links,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 确定性假度量：ASCII 0.55em、CJK 1.0em、空格 0.28em。
    struct FakeMetrics;

    impl TextMetrics for FakeMetrics {
        fn width_of(&self, c: char, size: f32) -> f32 {
            let em = if c == ' ' {
                0.28
            } else if c.is_ascii() {
                0.55
            } else {
                1.0
            };
            em * size
        }

        fn width_of_monospace(&self, c: char, size: f32) -> Option<f32> {
            c.is_ascii().then_some(0.60 * size)
        }
    }

    fn lay(md: &str, toc: bool) -> DocLayout {
        let session = Session::open(md);
        let metrics = FakeMetrics;
        let geom = PageGeom::a4(20.0);
        let mermaids = std::collections::HashMap::new();
        layout_document(
            &session,
            &metrics,
            geom,
            LayoutOpts {
                toc,
                page_numbers: true,
            },
            None,
            &mermaids,
        )
    }

    fn lay_with(session: &Session, toc: bool) -> DocLayout {
        let metrics = FakeMetrics;
        let geom = PageGeom::a4(20.0);
        let mermaids = std::collections::HashMap::new();
        layout_document(
            session,
            &metrics,
            geom,
            LayoutOpts {
                toc,
                page_numbers: true,
            },
            None,
            &mermaids,
        )
    }

    #[test]
    fn mermaid_alpha_compositing_handles_transparent_semitransparent_and_opaque_pixels() {
        let rgba = [
            0, 0, 0, 0, // 全透明：白底
            128, 64, 32, 128, // 预乘半透明色
            12, 34, 56, 255, // 不透明：原色不变
        ];
        assert_eq!(
            premultiplied_rgba_to_rgb_on_white(&rgba),
            vec![255, 255, 255, 255, 191, 159, 12, 34, 56]
        );
    }

    fn lay_mermaid(source: &str, width: u32, height: u32) -> DocLayout {
        let session = Session::open(&format!("```mermaid\n{source}\n```\n"));
        let id = session
            .blocks()
            .into_iter()
            .find_map(|(id, kind, _)| (kind == BlockKind::Mermaid).then_some(id))
            .expect("Mermaid block");
        let mut mermaids = HashMap::new();
        mermaids.insert(
            id,
            veil_ui::MermaidImage {
                data: vec![0; (width * height * 4) as usize],
                width,
                height,
                svg_text: None,
                px_to_pt: 0.75,
            },
        );
        layout_document(
            &session,
            &FakeMetrics,
            PageGeom::a4(20.0),
            LayoutOpts::default(),
            None,
            &mermaids,
        )
    }

    #[test]
    fn mermaid_flowchart_mindmap_and_sequence_keep_aspect_and_readable_height() {
        let geom = PageGeom::a4(20.0);
        let old_inline_cap = 2.0 * FakeMetrics.line_height(BODY_SIZE);
        for (name, source, width, height) in [
            ("flowchart", "flowchart LR\n  A --> B", 800, 400),
            ("mindmap", "mindmap\n  root((主题))\n    分支", 592, 439),
            (
                "sequence",
                "sequenceDiagram\n  A->>B: 请求\n  B-->>A: 响应",
                500,
                300,
            ),
        ] {
            let doc = lay_mermaid(source, width, height);
            let (x, y, w, h) = doc
                .pages
                .iter()
                .flatten()
                .find_map(|op| match op {
                    DrawOp::Image { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{name}: 应绘制 Mermaid 图片"));
            assert!(h > old_inline_cap + 1.0, "{name}: 仍被两行限高压扁: {h}");
            assert!(
                (w / h - width as f32 / height as f32).abs() < 0.001,
                "{name}: 宽高比改变: {w}×{h}"
            );
            assert!(w <= geom.content_w() + 0.01, "{name}: 越过版心宽");
            assert!(x >= geom.margin_x - 0.01, "{name}: 越过左页边距");
            assert!(y >= geom.top() - 0.01, "{name}: 越过上页边距");
            assert!(
                y + h + 8.0 <= geom.bottom() + 0.01,
                "{name}: 图片及块后间距越过下单页边界"
            );
        }
    }

    #[test]
    fn tall_mermaid_diagram_is_limited_only_by_single_page_safe_height() {
        let geom = PageGeom::a4(20.0);
        let doc = lay_mermaid("sequenceDiagram\n  A->>B: 长图", 400, 1400);
        let (y, w, h) = doc
            .pages
            .iter()
            .flatten()
            .find_map(|op| match op {
                DrawOp::Image { y, w, h, .. } => Some((*y, *w, *h)),
                _ => None,
            })
            .expect("应绘制 Mermaid 图片");
        assert!((h - (geom.content_h() - 12.0)).abs() < 0.01, "高度 {h}");
        assert!((w / h - 400.0 / 1400.0).abs() < 0.001, "限高仍须等比");
        assert!(y + h + 8.0 <= geom.bottom() + 0.01, "不得越出单页");
    }

    /// 完整版 corpus 逐图走「本地 SVG → 高 DPI 位图 → 原生 PDF」；逐图释放位图以限制峰值内存。
    #[cfg(feature = "mermaid-offline")]
    #[test]
    fn offline_mermaid_corpus_pdf_uses_local_images() {
        use crate::platform::pdf::{find_cjk_font, render_pages};
        use veil_ui::MermaidRenderer;

        let font_path = find_cjk_font().expect("Windows CJK font");
        let mut renderer = crate::platform::mermaid_offline::OfflineMermaid::new();

        for &(name, source) in veil_mermaid_offline::corpus::MERMAID_11_12_CORPUS {
            let markdown = format!("# {name}\n\n```mermaid\n{source}\n```\n");
            let session = Session::open(&markdown);
            let id = session
                .blocks()
                .into_iter()
                .find_map(|(id, kind, _)| (kind == BlockKind::Mermaid).then_some(id))
                .expect("Mermaid block");
            let image = renderer
                .render_blocking(name.into(), source.into(), false)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let mut mermaids = HashMap::new();
            mermaids.insert(id, image);

            let doc = layout_document(
                &session,
                &FakeMetrics,
                PageGeom::a4(20.0),
                LayoutOpts::default(),
                None,
                &mermaids,
            );
            assert_eq!(doc.images.len(), 1, "{name}: PDF 必须重新光栅化本地 SVG");
            assert!(!doc.images[0].rgb.is_empty(), "{name}: empty RGB image");
            assert_eq!(
                doc.pages
                    .iter()
                    .flatten()
                    .filter(|op| matches!(op, DrawOp::Image { .. }))
                    .count(),
                1,
                "{name}: Mermaid 必须生成图片绘制指令"
            );
            let bytes = render_pages(
                &doc.pages,
                &doc.images,
                &doc.outline,
                &doc.links,
                Path::new(font_path),
            )
            .unwrap_or_else(|error| panic!("{name}: PDF render: {error}"));
            assert!(bytes.starts_with(b"%PDF-"), "{name}: PDF header");
            assert!(bytes.ends_with(b"%%EOF"), "{name}: PDF trailer");
            assert!(
                String::from_utf8_lossy(&bytes).contains("/Subtype /Image"),
                "{name}: PDF image XObject missing"
            );
        }
    }

    // ===== Task 5：链接注释 =====

    #[test]
    fn external_and_anchor_links_emit_annotations() {
        let doc = lay(
            "[官网](https://example.com/guide)\n\n# 章节一\n\n回到[章节](#章节一)\n",
            false,
        );
        let uris: Vec<&LinkAnnot> = doc
            .links
            .iter()
            .filter(|l| matches!(l.target, LinkTarget::Uri(_)))
            .collect();
        assert_eq!(uris.len(), 1, "外链应生成一条 URI 注释: {:?}", doc.links);
        assert!(matches!(
            &uris[0].target,
            LinkTarget::Uri(u) if u == "https://example.com/guide"
        ));
        assert!(uris[0].w > 0.0 && uris[0].h > 0.0, "矩形须非零");
        let internal: Vec<&LinkAnnot> = doc
            .links
            .iter()
            .filter(|l| matches!(l.target, LinkTarget::Internal { .. }))
            .collect();
        assert_eq!(internal.len(), 1, "内链应生成一条跳转注释: {:?}", doc.links);
        if let LinkTarget::Internal { page, y, .. } = internal[0].target {
            assert_eq!(page, 0);
            let head_y = doc.outline[0].top_y;
            assert!(
                (y - head_y).abs() < 1.0,
                "锚点应指向标题顶部: {y} vs {head_y}"
            );
        }
    }

    #[test]
    fn wrapped_link_gets_one_rect_per_line() {
        let label = "长链接文字".repeat(30);
        let md = format!("[{label}](https://example.com/long)\n");
        let doc = lay(&md, false);
        let rects: Vec<&LinkAnnot> = doc
            .links
            .iter()
            .filter(|l| matches!(l.target, LinkTarget::Uri(_)))
            .collect();
        assert!(rects.len() >= 2, "跨行链接应逐行生成矩形: {}", rects.len());
        for r in rects {
            assert!(r.y >= 0.0 && r.h > 0.0, "矩形坐标须有效: {r:?}");
        }
    }

    #[test]
    fn footnote_ref_and_definition_get_bidirectional_annotations() {
        let doc = lay("正文[^a]\n\n[^a]: 注释\n", false);
        let internal: Vec<&LinkAnnot> = doc
            .links
            .iter()
            .filter(|l| matches!(l.target, LinkTarget::Internal { .. }))
            .collect();
        assert_eq!(
            internal.len(),
            2,
            "引用→定义 与 定义→引用 各一条: {:?}",
            doc.links
        );
        assert!(
            (internal[0].y - internal[1].y).abs() > 1.0,
            "两条注释应位于不同垂直位置"
        );
    }

    #[test]
    fn link_spanning_pages_gets_rect_on_each_page() {
        // 跨页链接：同一 URL 的文字跨过分页，每页各自一个矩形（页下标正确）。
        let label = "跨页链接".repeat(1500);
        let doc = lay(&format!("[{label}](https://example.com/paged)\n"), false);
        let pages: std::collections::HashSet<usize> = doc.links.iter().map(|l| l.page).collect();
        assert!(pages.len() >= 2, "跨页链接应在多页生成矩形: {:?}", pages);
        assert!(doc.links.iter().all(|l| l.page < doc.pages.len()));
    }

    #[test]
    fn dangerous_schemes_and_overlong_urls_emit_no_actions() {
        let doc = lay(
            "[a](javascript:alert(1)) [b](file:///C:/secret.txt) [c](data:text/html,hi)\n",
            false,
        );
        assert!(
            doc.links.is_empty(),
            "危险 scheme 不得生成动作: {:?}",
            doc.links
        );
        let long = format!("[x](https://example.com/{})\n", "x".repeat(2100));
        let doc = lay(&long, false);
        assert!(
            doc.links.is_empty(),
            "超长 URL 不得生成动作: {:?}",
            doc.links
        );
    }

    // ===== N3c / N3b：高亮与 Callout 的 PDF 通道 =====

    #[test]
    fn highlight_emits_mark_background_on_text() {
        let doc = lay("==hi==\n", false);
        let marked: Vec<&str> = doc
            .pages
            .iter()
            .flatten()
            .filter_map(|op| match op {
                DrawOp::Text(t) if t.highlight == Some([255, 247, 153]) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            marked.iter().any(|t| t.contains("hi")),
            "高亮文字须带 <mark> 底色: {marked:?}"
        );
    }

    #[test]
    fn builtin_callout_pdf_has_label_rail_and_tint() {
        let doc = lay("> [!WARNING]\n> 注意\n", false);
        let text: String = doc.pages.iter().map(|p| page_text(p)).collect();
        assert!(text.contains("Warning"), "标签应与预览/HTML 同源: {text}");
        assert!(text.contains("注意"), "{text}");
        assert!(!text.contains("[!WARNING]"), "不得露出标记: {text}");
        let mut rails = 0;
        let mut tint = false;
        for op in doc.pages.iter().flatten() {
            if let DrawOp::Rect { w, color, .. } = op {
                if (*w - 3.0).abs() < 0.01 {
                    rails += 1;
                    assert_eq!(*color, [154, 103, 0], "竖条用统一强调色");
                }
                if *color == tint_on_white([154, 103, 0], 0.10) {
                    tint = true;
                }
            }
        }
        assert!(rails >= 1, "须有强调色竖条");
        assert!(tint, "须有 Callout 浅底色");
    }

    #[test]
    fn custom_callout_pdf_uses_config_label_and_color() {
        let reg = veil_core::CalloutRegistry::from_json(
            r##"[{"name":"FIXME","label":"待修复","color":"#c00","icon":"!"}]"##,
        );
        let session = Session::open_with_callouts("> [!FIXME]\n> 内容\n", &reg);
        let doc = lay_with(&session, false);
        let text: String = doc.pages.iter().map(|p| page_text(p)).collect();
        assert!(text.contains("! 待修复"), "配置标签 + 图标: {text}");
        assert!(text.contains("内容"), "{text}");
        assert!(
            doc.pages.iter().flatten().any(|op| matches!(
                op,
                DrawOp::Rect { w, color, .. } if (*w - 3.0).abs() < 0.01 && *color == [204, 0, 0]
            )),
            "竖条须用配置色"
        );
    }

    #[test]
    fn unknown_callout_pdf_is_plain_quote_without_marker() {
        let doc = lay("> [!BOGUS]\n> body\n", false);
        let text: String = doc.pages.iter().map(|p| page_text(p)).collect();
        assert!(text.contains("body"), "{text}");
        assert!(!text.contains("[!BOGUS]"), "不得露出裸标记: {text}");
        assert!(
            doc.pages.iter().flatten().all(|op| !matches!(
                op,
                DrawOp::Rect { w, color, .. } if (*w - 3.0).abs() < 0.01 && *color != COL_RULE
            )),
            "未知类型竖条应为引用灰"
        );
    }

    fn page_text(page: &[DrawOp]) -> String {
        page.iter()
            .filter_map(|op| match op {
                DrawOp::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn all_text(doc: &DocLayout) -> String {
        doc.pages
            .iter()
            .map(|p| page_text(p))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 纯 TOC 标记是视图指令：PDF/打印版面与 HTML 一致，不输出字面 `[TOC]`，标题照常。
    #[test]
    fn toc_marker_paragraph_is_not_laid_out() {
        for marker in ["[TOC]", "[[TOC]]"] {
            let doc = lay(&format!("{marker}\n\n# 第一章\n"), false);
            let text = all_text(&doc);
            assert!(!text.contains(marker), "不得排版字面 {marker}：{text}");
            assert_eq!(
                text.matches("第一章").count(),
                1,
                "标题应照常排版且仅一次：{text}"
            );
        }
    }

    /// 反斜杠转义与硬换行：`\*` 显示字面星号，行尾 `\` 不显示，正文与标题同理。
    #[test]
    fn backslash_escapes_and_hard_breaks_are_rendered_like_editor() {
        let doc = lay("\\*不是强调\\*\n", false);
        let text = all_text(&doc);
        assert!(text.contains("*不是强调*"), "转义星号应显示为字面：{text}");
        assert!(!text.contains('\\'), "转义反斜杠不得进入版面：{text}");

        let doc = lay("第一行\\\n第二行\n", false);
        let text = all_text(&doc);
        assert!(text.contains("第一行") && text.contains("第二行"), "{text}");
        assert!(!text.contains('\\'), "硬换行反斜杠不得进入版面：{text}");

        // 代码内容不解释转义
        let doc = lay("`\\*x\\*`\n", false);
        let text = all_text(&doc);
        assert!(text.contains("\\*x\\*"), "代码中的反斜杠应原样：{text}");
    }

    /// 标题尾缀 `{#id}` 是锚点指令：版面正文与书签都不得显示。
    #[test]
    fn heading_trailing_id_is_hidden_from_text_and_outline() {
        let doc = lay("## 安装 {#install}\n", false);
        let text = all_text(&doc);
        assert!(text.contains("安装"), "{text}");
        assert!(!text.contains("{#install}"), "{text}");
        assert_eq!(doc.outline.len(), 1);
        assert_eq!(doc.outline[0].title, "安装");
    }

    /// B7：多级中文标题的版面书签必须保留层级与文本；页数/书签数与标题数一致。
    #[test]
    fn multilevel_chinese_headings_produce_nested_outline() {
        let md = "# 第一章 引言\n\n正文一。\n\n## 1.1 背景\n\n正文二。\n\n### 1.1.1 细节\n\n\
                  正文三。\n\n## 1.2 目标\n\n正文四。\n\n# 第二章 实现\n\n正文五。\n";
        let doc = lay(md, false);
        let titles: Vec<&str> = doc.outline.iter().map(|o| o.title.as_str()).collect();
        let levels: Vec<u8> = doc.outline.iter().map(|o| o.level).collect();
        assert_eq!(
            titles,
            vec![
                "第一章 引言",
                "1.1 背景",
                "1.1.1 细节",
                "1.2 目标",
                "第二章 实现"
            ],
            "书签标题必须与标题文本逐字一致（含中文）"
        );
        assert_eq!(levels, vec![1, 2, 3, 2, 1], "书签层级必须与 H1/H2/H3 对应");
        assert!(!doc.pages.is_empty(), "应有可打印页");
        // 每个书签必须落在有效页内，跳转目标才有意义。
        for o in &doc.outline {
            assert!(o.page_idx < doc.pages.len());
            assert!(o.top_y.is_finite() && o.top_y >= 0.0);
        }
        // 页数不因书签多少而膨胀：书签数 = 标题数，页数由内容决定。
        assert_eq!(doc.outline.len(), 5);
    }

    #[test]
    fn markers_are_stripped() {
        let md = "# 标题\n\n**粗体** 与 *斜体* 与 ~~删除~~ 与 `代码` 与 [链接](https://e.com)\n";
        let doc = lay(md, false);
        let text = all_text(&doc);
        for bad in ["#", "**", "~~", "`", "](", "https://e.com"] {
            assert!(!text.contains(bad), "不应出现标记 {bad:?}，实际：{text}");
        }
        for good in ["标题", "粗体", "斜体", "删除", "代码", "链接"] {
            assert!(text.contains(good), "应保留可见文本 {good:?}");
        }
    }

    #[test]
    fn inline_code_uses_monospace_metrics_and_unicode_fallback_width() {
        let doc = lay("A `im中` B\n", false);
        let runs: Vec<&TextRun> = doc
            .pages
            .iter()
            .flatten()
            .filter_map(|op| match op {
                DrawOp::Text(t) => Some(t),
                _ => None,
            })
            .collect();
        let code = runs
            .iter()
            .find(|r| r.text == "im中")
            .expect("inline code 应独立成 TextRun");
        assert!(code.monospace, "inline code 必须携带 monospace 样式");
        let after = runs
            .iter()
            .find(|r| r.text.contains('B'))
            .expect("代码后的正文应绘制");
        assert!(!after.monospace, "普通正文不得变成 monospace");
        let expected_code_w = 2.0 * 0.60 * BODY_SIZE + BODY_SIZE;
        assert!(
            (after.x - code.x - expected_code_w).abs() < 0.001,
            "ASCII 走 0.60em 等宽，中文字符回退为 1.0em：code.x={} after.x={}",
            code.x,
            after.x
        );
    }

    #[test]
    fn semantic_inline_html_and_style_render() {
        let md = "<strong style=\"color:red\">VISUAL</strong> 与 <q>引用</q> 与 <u>下划</u> \
                  与 <s>删除</s> 与 <small>小字</small> 与 \
                  <span style=\"color:red;position:fixed\">红字</span>\n";
        let doc = lay(md, false);
        let text = all_text(&doc);
        for bad in ["<strong", "style=", "position", "<q>", "<small"] {
            assert!(
                !text.contains(bad),
                "PDF 不应含源码/危险声明 {bad:?}：{text}"
            );
        }
        assert!(text.contains("VISUAL"), "实际：{text}");
        assert!(text.contains("“引用”"), "q 应带双引号：{text}");

        let find = |needle: &str| {
            doc.pages
                .iter()
                .flatten()
                .find_map(|op| match op {
                    DrawOp::Text(t) if t.text.contains(needle) => Some(t),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("找不到文本 {needle:?}"))
        };
        let strong = find("VISUAL");
        assert!(strong.bold, "strong 应粗体");
        assert_eq!(strong.color, [255, 0, 0], "style color:red 应生效");
        assert!(find("下划").underline, "u 应下划线");
        assert!(find("删除").strike, "s 应删除线");
        assert!(find("小字").size < BODY_SIZE, "small 应缩小");
        assert_eq!(find("红字").color, [255, 0, 0], "span style 应生效");
    }

    #[test]
    fn heading_sizes_are_monotonic() {
        let md =
            "# 一级\n\n## 二级\n\n### 三级\n\n#### 四级\n\n##### 五级\n\n###### 六级\n\n正文\n";
        let doc = lay(md, false);
        let mut sizes = Vec::new();
        for page in &doc.pages {
            for op in page {
                if let DrawOp::Text(t) = op {
                    if ["一级", "二级", "三级", "四级", "五级", "六级"].contains(&t.text.trim())
                    {
                        sizes.push(t.size);
                    }
                }
            }
        }
        assert_eq!(sizes.len(), 6, "应找到 6 级标题，实际 {sizes:?}");
        for w in sizes.windows(2) {
            assert!(w[0] > w[1], "标题字号应严格递减：{sizes:?}");
        }
        assert!(*sizes.last().unwrap() >= BODY_SIZE);
    }

    #[test]
    fn lines_never_overflow_content_width() {
        let long = "超长段落".repeat(160);
        let md = format!("{}\n\n{long}\n", "CJK 与 ascii 混排 longwordwithoutanyspacesxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".repeat(6));
        let doc = lay(&md, false);
        let geom = PageGeom::a4(20.0);
        let right = geom.margin_x + geom.content_w();
        let m = FakeMetrics;
        for page in &doc.pages {
            for op in page {
                if let DrawOp::Text(t) = op {
                    let w = m.text_width(&t.text, t.size);
                    assert!(
                        t.x + w <= right + 0.5,
                        "文本越界：x={} w={} right={} text={:?}",
                        t.x,
                        w,
                        right,
                        t.text
                    );
                }
            }
        }
    }

    #[test]
    fn outline_pages_match_heading_pages() {
        let mut md = String::from("# 总标题\n\n");
        for i in 0..3 {
            md.push_str(&format!("## 第 {i} 节\n\n"));
            for _ in 0..40 {
                md.push_str("这是填充段落，用来把章节推到后续页面。\n\n");
            }
        }
        let doc = lay(&md, false);
        let s = Session::open(&md);
        eprintln!(
            "DEBUG pages={} blocks={} kinds={:?}",
            doc.pages.len(),
            s.blocks().len(),
            s.blocks()
                .iter()
                .take(6)
                .map(|b| b.1.clone())
                .collect::<Vec<_>>()
        );
        assert!(doc.pages.len() >= 3, "应分多页（实际 {}）", doc.pages.len());
        for item in &doc.outline {
            let page_text = page_text(&doc.pages[item.page_idx]);
            assert!(
                page_text.contains(&item.title),
                "书签《{}》的页码 {} 不含该标题",
                item.title,
                item.page_idx
            );
        }
        assert_eq!(doc.outline.first().unwrap().level, 1);
        assert!(doc.outline.iter().any(|o| o.level == 2));
    }

    #[test]
    fn footer_and_toc_pages() {
        let mut md = String::from("# 第一章\n\n");
        for _ in 0..50 {
            md.push_str("目录测试段落。\n\n");
        }
        md.push_str("## 第二节\n\n更多内容。\n");
        let doc = lay(&md, true);
        assert!(doc.pages.len() >= 2);
        // 首页为目录页
        assert!(page_text(&doc.pages[0]).contains("目录"));
        // 每页都有 n / total 页脚
        let total = doc.pages.len();
        for (i, page) in doc.pages.iter().enumerate() {
            let expected = format!("{} / {total}", i + 1);
            assert!(
                page_text(page).contains(&expected),
                "第 {} 页缺少页脚 {expected}",
                i + 1
            );
        }
        // 目录页码 = 书签实际页 + 1（人类可读页码）
        let first = &doc.outline[0];
        let human = first.page_idx + 1;
        let toc_text = page_text(&doc.pages[0]);
        assert!(
            toc_text.contains("第一章") && toc_text.contains(&human.to_string()),
            "目录页应含标题与页码 {human}：{toc_text}"
        );
    }

    #[test]
    fn block_prefixes_and_rails() {
        let md = "# T\n\n- 项目甲\n- 项目乙\n\n3. 有序项\n\n- [ ] 待办\n- [x] 已完成\n\n> 引用内容\n\n> [!NOTE]\n> 注意事项\n\n```rust\nfn main() {}\n```\n\n| 列A | 列B |\n| --- | --- |\n| 1 | 2 |\n";
        let doc = lay(md, false);
        let text = all_text(&doc);
        assert!(text.contains("• "), "无序列表前缀缺失");
        assert!(text.contains("3. "), "有序列表前缀缺失");
        assert!(text.contains("☐ "), "任务列表未完成前缀缺失");
        assert!(text.contains("☑ "), "任务列表已完成前缀缺失");
        assert!(text.contains("Note"), "Admonition 标签缺失");

        let mut rects = 0;
        let mut rails = 0;
        let mut lines = 0;
        let mut code_bg = false;
        for page in &doc.pages {
            for op in page {
                match op {
                    DrawOp::Rect { w, color, .. } => {
                        rects += 1;
                        if *w <= 4.0 && *w >= 2.0 {
                            rails += 1;
                        }
                        if *color == COL_CODE_BG {
                            code_bg = true;
                        }
                    }
                    DrawOp::Line { .. } => lines += 1,
                    _ => {}
                }
            }
        }
        assert!(rects > 0);
        assert!(rails >= 1, "引用/Admonition 左侧竖条缺失");
        assert!(code_bg, "代码块浅灰底缺失");
        assert!(lines >= 5, "表格网格线不足：{lines}");
    }

    #[test]
    fn quoted_table_lays_out_as_nested_grid() {
        let md = "> | 列A | 列B |\n> | --- | ---: |\n> | 1 | 2 |\n";
        let doc = lay(md, false);
        let text = all_text(&doc);
        assert!(text.contains("列A") && text.contains("列B"), "{text}");
        assert!(!text.contains("|"), "分隔行/管道符不得进入 PDF：{text}");

        let margin_x = PageGeom::a4(20.0).margin_x;
        let cell = doc
            .pages
            .iter()
            .flatten()
            .find_map(|op| match op {
                DrawOp::Text(t) if t.text.trim() == "列A" => Some(t),
                _ => None,
            })
            .expect("应绘制表头单元格");
        assert!(
            cell.x > margin_x,
            "引用内表格应右移：x={} margin={margin_x}",
            cell.x
        );
        assert!(
            doc.pages
                .iter()
                .flatten()
                .any(|op| matches!(op, DrawOp::Line { .. })),
            "嵌套表格应绘制网格线"
        );
    }

    #[test]
    fn table_cell_inlines_preserve_bold_italic_code_and_link() {
        // Task 7.2：表格单元格与正文同源消费 inlines，不再退化为纯文本。
        let md = "| 列 | 值 |\n| --- | --- |\n| **粗体** | *斜体* `代码` [链接](https://example.com) |\n";
        let doc = lay(md, false);
        let runs: Vec<&TextRun> = doc
            .pages
            .iter()
            .flatten()
            .filter_map(|op| match op {
                DrawOp::Text(t) => Some(t),
                _ => None,
            })
            .collect();
        assert!(
            runs.iter().any(|r| r.text.contains("粗体") && r.bold),
            "粗体应保留为 bold TextRun: {:?}",
            runs.iter().map(|r| &r.text).collect::<Vec<_>>()
        );
        assert!(
            runs.iter().any(|r| r.text.contains("斜体") && r.italic),
            "斜体应保留为 italic TextRun"
        );
        assert!(
            runs.iter()
                .any(|r| r.text.contains("代码") && r.highlight == Some(COL_CODE_BG)),
            "行内代码应保留浅灰底色"
        );
        assert!(
            runs.iter().any(|r| r.text.contains("代码") && r.monospace),
            "行内代码应使用真实 monospace 字体资源"
        );
        let link = runs
            .iter()
            .find(|r| r.text.contains("链接"))
            .expect("链接文字应绘制");
        assert!(
            link.underline && link.color == COL_LINK,
            "链接应保留下划线/链接色：underline={} color={:?}",
            link.underline,
            link.color
        );
        assert!(
            doc.links.iter().any(|l| matches!(
                &l.target,
                LinkTarget::Uri(u) if u == "https://example.com"
            )),
            "表格内链接应生成 URI 注释"
        );
    }

    #[test]
    fn table_long_cell_is_not_truncated_and_paginates() {
        // 旧实现把单元格截断到 6 行并追加 `…`；现在必须完整输出并跨页。
        let body = "乙".repeat(5000);
        let md = format!("| 长文 |\n| --- |\n| {body} |\n");
        let doc = lay(&md, false);
        let text = all_text(&doc);
        assert!(!text.contains('…'), "单元格内容不得静默截断");
        assert_eq!(
            text.matches('乙').count(),
            5000,
            "单元格内容必须逐字完整进入 PDF"
        );
        assert!(doc.pages.len() > 1, "超长表格行必须跨页，而不是被截断");
        // 每一页都要有网格线：拆行后每页重画边框。
        for (i, page) in doc.pages.iter().enumerate() {
            assert!(
                page.iter().any(|op| matches!(op, DrawOp::Line { .. })),
                "第 {i} 页缺少表格网格线"
            );
        }
    }

    #[test]
    fn table_long_row_near_page_bottom_stays_inside_content_area() {
        // 先填到接近页底，再放超长单元格：分页时每个 chunk 都必须整体落在
        // [top, bottom] 内，不能再把首行硬塞进只剩几 pt 的页底（基线越界被裁切）。
        let mut md = String::new();
        for _ in 0..20 {
            md.push_str("填充段落。\n\n");
        }
        md.push_str("| 长文 |\n| --- |\n| ");
        md.push_str(&"乙".repeat(3000));
        md.push_str(" |\n");
        let doc = lay(&md, false);
        let geom = PageGeom::a4(20.0);
        let (top, bottom) = (geom.top(), geom.bottom());
        let mut cells = 0usize;
        let mut chars = 0usize;
        for (pi, page) in doc.pages.iter().enumerate() {
            for op in page {
                if let DrawOp::Text(t) = op {
                    if (t.size - TABLE_SIZE).abs() > 0.01 {
                        continue; // 只检查表格单元格，页脚不在正文区
                    }
                    cells += 1;
                    chars += t.text.matches('乙').count();
                    assert!(
                        t.y <= bottom + 0.01,
                        "第 {pi} 页表格基线越过正文底：y={} bottom={bottom}",
                        t.y
                    );
                    assert!(
                        t.y - FakeMetrics.ascent(t.size) >= top - 0.01,
                        "第 {pi} 页表格基线越过正文顶：y={} top={top}",
                        t.y
                    );
                }
            }
        }
        assert!(cells > 0, "应绘制表格单元格");
        assert_eq!(chars, 3000, "跨页后单元格内容必须逐字完整");
    }

    #[test]
    fn fill_lines_applies_chinese_kinsoku_rules() {
        let st = Style::body();
        let items = |text: &str| -> Vec<LItem> { text.chars().map(|c| LItem::Ch(c, st)).collect() };
        // FakeMetrics：CJK 1em @ 11pt → 一行正好 43 个字。
        let line_w = 43.0 * 11.0;
        let rendered = |lines: Vec<Vec<LItem>>| -> Vec<String> {
            lines
                .iter()
                .map(|line| {
                    line.iter()
                        .filter_map(|item| match item {
                            LItem::Ch(c, _) => Some(*c),
                            _ => None,
                        })
                        .collect::<String>()
                })
                .collect()
        };

        // 避头：断点正好落在句号前 → 句号不得出现在下一行行首。
        let text = format!("{}。结尾", "甲".repeat(43));
        let lines = rendered(fill_lines(items(&text), line_w, &FakeMetrics));
        assert!(lines.len() >= 2, "{lines:?}");
        for line in &lines {
            assert!(!line.starts_with('。'), "句号不得出现在行首：{lines:?}");
        }
        assert!(lines[1].starts_with("甲。"), "{lines:?}");

        // 避尾：开括号不得留在行尾。
        let text = format!("{}（后", "乙".repeat(42));
        let lines = rendered(fill_lines(items(&text), line_w, &FakeMetrics));
        assert!(lines.len() >= 2, "{lines:?}");
        for line in &lines {
            assert!(!line.ends_with('（'), "开括号不得留在行尾：{lines:?}");
        }
        assert!(lines[1].starts_with('（'), "{lines:?}");

        // 连续闭符号（`。」`）不得把后一个闭符号孤立到行首。
        let text = format!("{}。」尾", "丙".repeat(43));
        let lines = rendered(fill_lines(items(&text), line_w, &FakeMetrics));
        for line in &lines {
            assert!(
                !line.starts_with(['。', '」']),
                "连续闭符号不得出现在行首：{lines:?}"
            );
        }
    }

    #[test]
    fn degrades_without_media() {
        let md = "文本 ![图片](missing.png) 与 $x^2$\n\n$$\\frac{1}{2}$$\n";
        let doc = lay(md, false);
        let text = all_text(&doc);
        assert!(text.contains("图片"), "图片应降级为 alt 文本");
        assert!(text.contains("$x^2$"), "行内公式应降级为源码");
        assert!(text.contains("\\frac"), "块级公式应降级为源码");
    }

    /// 假媒体：返回固定尺寸图片，公式一律降级。
    struct FakeImageMedia;

    impl MediaProvider for FakeImageMedia {
        fn image(&mut self, _path: &Path) -> Result<ImageData, String> {
            Ok(ImageData {
                w: 240,
                h: 150,
                rgb: vec![0; 240 * 150 * 3],
                baseline_px: 150.0,
                px_to_pt: 1.0,
            })
        }

        fn math(&mut self, _tex: &str, _display: bool) -> Result<ImageData, String> {
            Err("假媒体不支持公式".into())
        }
    }

    fn lay_media(md: &str, media: &mut dyn MediaProvider) -> DocLayout {
        let session = Session::open(md);
        let mermaids = std::collections::HashMap::new();
        layout_document(
            &session,
            &FakeMetrics,
            PageGeom::a4(20.0),
            LayoutOpts::default(),
            Some(media),
            &mermaids,
        )
    }

    /// 假媒体：图片固定尺寸，公式渲染为固定位图（记录调用参数）。
    struct FakeMathMedia {
        calls: Vec<(String, bool)>,
    }

    impl FakeMathMedia {
        fn new() -> Self {
            Self { calls: Vec::new() }
        }
    }

    impl MediaProvider for FakeMathMedia {
        fn image(&mut self, _path: &Path) -> Result<ImageData, String> {
            Ok(ImageData {
                w: 240,
                h: 150,
                rgb: vec![0; 240 * 150 * 3],
                baseline_px: 150.0,
                px_to_pt: 1.0,
            })
        }

        fn math(&mut self, tex: &str, display: bool) -> Result<ImageData, String> {
            self.calls.push((tex.to_string(), display));
            Ok(ImageData {
                w: 40,
                h: 16,
                rgb: vec![255; 40 * 16 * 3],
                baseline_px: 12.0,
                px_to_pt: 1.0,
            })
        }
    }

    /// 回归：`math()` 返回 Ok 时，公式必须以位图（版面 `DrawOp::Image` → PDF
    /// `/Subtype /Image` 图像 XObject）进入导出，不得被静默丢弃，也不得退化为
    /// LaTeX 源码文字。
    #[cfg(windows)]
    #[test]
    fn math_image_becomes_image_xobject_in_pdf() {
        // 正文只含中文：ASCII 公式源码字符只会来自公式，便于在 PDF 中判定文字泄漏。
        let md = "引子 $zq$ 尾声\n\n$$zq_{99}$$\n";
        let mut media = FakeMathMedia::new();
        let doc = lay_media(md, &mut media);

        // —— 版面层 ——
        assert_eq!(
            media.calls,
            vec![("zq".to_string(), false), ("zq_{99}".to_string(), true)],
            "行内与块级公式都应交给 media.math 渲染"
        );
        let images: Vec<&DrawOp> = doc
            .pages
            .iter()
            .flatten()
            .filter(|op| matches!(op, DrawOp::Image { .. }))
            .collect();
        assert_eq!(images.len(), 2, "两个公式都应产出图像指令，不得被静默丢弃");
        for op in &images {
            let DrawOp::Image { w, h, id, .. } = op else {
                unreachable!()
            };
            assert_eq!((*w, *h), (40.0, 16.0), "公式图像应使用 media 返回的尺寸");
            assert_eq!(doc.images[*id].pt_w(), 40.0);
        }
        let text = all_text(&doc);
        assert!(
            !text.contains("zq"),
            "公式不得退化为 LaTeX 源码文字: {text}"
        );

        // —— PDF 层 ——
        let font_path = crate::platform::pdf::find_cjk_font().expect("需要 CJK 字体渲染 PDF");
        let bytes = crate::platform::pdf::render_pages(
            &doc.pages,
            &doc.images,
            &doc.outline,
            &doc.links,
            Path::new(font_path),
        )
        .expect("应产出 PDF");
        let pdf = String::from_utf8_lossy(&bytes);
        assert!(
            pdf.contains("/Subtype /Image"),
            "公式应作为图像 XObject 嵌入"
        );
        assert!(pdf.contains("/Im0 Do"), "内容流应绘制公式位图");
        assert!(
            pdf.matches("/Im1 Do").count() == 1,
            "两个公式位图都应被内容流引用"
        );

        // 用与 render_pages 相同的字符集重建字体，把内容流中的 Tj 十六进制串解码回文字；
        // 公式源码（zq）若被当作文字写出，解码结果里必然出现这些 ASCII 字符。
        let page_chars: Vec<char> = doc
            .pages
            .iter()
            .flatten()
            .filter_map(|op| match op {
                DrawOp::Text(t) => Some(t.text.chars()),
                _ => None,
            })
            .flatten()
            .chain(doc.outline.iter().flat_map(|o| o.title.chars()))
            .collect();
        let font = crate::platform::pdf::FontResource::load_and_subset(
            Path::new(font_path),
            page_chars,
            false,
        )
        .expect("字体子集");
        let decode = |hex: &str| -> String {
            let digits: String = hex.chars().filter(char::is_ascii_hexdigit).collect();
            digits
                .as_bytes()
                .chunks_exact(4)
                .filter_map(|c| std::str::from_utf8(c).ok())
                .filter_map(|s| u16::from_str_radix(s, 16).ok())
                .filter_map(|cid| font.cid_to_char.get(cid as usize).copied())
                .collect()
        };
        let mut shown = String::new();
        for piece in pdf.split(" Tj") {
            let tail = piece.rsplit('\n').next().unwrap_or("");
            let is_cid_hex = tail.starts_with('<')
                && tail.ends_with('>')
                && tail
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() || c == '<' || c == '>');
            if is_cid_hex {
                shown.push_str(&decode(tail));
            }
        }
        assert!(
            shown.contains("引子") && shown.contains("尾声"),
            "PDF 应写出正文文字: {shown}"
        );
        assert!(
            !shown.contains('z') && !shown.contains('q'),
            "公式源码不得作为文字写入 PDF: {shown}"
        );
    }

    #[test]
    fn standalone_images_emit_visible_captions_below_images() {
        let mut media = FakeImageMedia;
        let doc = lay_media(
            "![独立题注](fake.png)\n\n<img src=\"fake.png\" alt=\"HTML 题注\">\n",
            &mut media,
        );
        assert_eq!(
            doc.pages
                .iter()
                .flatten()
                .filter(|op| matches!(op, DrawOp::Image { .. }))
                .count(),
            2,
            "Markdown 与 HTML 独立图片都应绘制"
        );
        for expected in ["独立题注", "HTML 题注"] {
            let caption = doc
                .pages
                .iter()
                .flatten()
                .find_map(|op| match op {
                    DrawOp::Text(t) if t.text == expected => Some(t),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("独立图片应输出可见题注 {expected}"));
            assert_eq!(caption.size, 9.5);
            assert_eq!(caption.color, COL_MUTED);
        }
        let first_image = doc
            .pages
            .iter()
            .flatten()
            .find_map(|op| match op {
                DrawOp::Image { y, h, .. } => Some((*y, *h)),
                _ => None,
            })
            .expect("应有独立图片");
        let first_caption_y = doc
            .pages
            .iter()
            .flatten()
            .find_map(|op| match op {
                DrawOp::Text(t) if t.text == "独立题注" => Some(t.y),
                _ => None,
            })
            .expect("应有首个题注");
        assert!(
            first_caption_y >= first_image.0 + first_image.1,
            "题注应位于图片下方"
        );
    }

    #[test]
    fn html_img_alignment_shifts_draw_x_within_content_width() {
        let mut media = FakeImageMedia;
        let geom = PageGeom::a4(20.0);
        let image_x = |md: &str, media: &mut FakeImageMedia| {
            let doc = lay_media(md, media);
            doc.pages
                .iter()
                .flatten()
                .find_map(|op| match op {
                    DrawOp::Image { x, w, .. } => Some((*x, *w)),
                    _ => None,
                })
                .expect("应绘制图片")
        };
        let (x, w) = image_x("<img src=\"fake.png\" align=\"center\">\n", &mut media);
        let expect = geom.margin_x + (geom.content_w() - w) * 0.5;
        assert!((x - expect).abs() < 0.5, "center x={x} expect={expect}");
        let (x, w) = image_x("<img src=\"fake.png\" align=\"right\">\n", &mut media);
        let expect = geom.margin_x + (geom.content_w() - w);
        assert!((x - expect).abs() < 0.5, "right x={x} expect={expect}");
        // 无 align / 行内图片：保持版心左缘续排。
        let (x, _) = image_x("<img src=\"fake.png\">\n", &mut media);
        assert!((x - geom.margin_x).abs() < 0.5, "left x={x}");
        let (x, _) = image_x(
            "前文 <img src=\"fake.png\" align=\"center\"> 后文\n",
            &mut media,
        );
        assert!(x > geom.margin_x, "行内图片不得跳位：x={x}");
        assert!(x < geom.margin_x + w + 30.0, "行内图片应紧随前文：x={x}");
    }

    #[test]
    fn inline_and_empty_alt_images_do_not_emit_captions() {
        let mut media = FakeImageMedia;
        let doc = lay_media(
            "前文 ![行内替代](fake.png) 后文\n\n![](fake.png)\n\n![   ](fake.png)\n",
            &mut media,
        );
        let text = all_text(&doc);
        assert!(text.contains("前文") && text.contains("后文"), "{text}");
        assert!(!text.contains("行内替代"), "行内图片不得派生题注: {text}");
        assert!(
            doc.pages.iter().flatten().all(|op| !matches!(
                op,
                DrawOp::Text(t) if (t.size - 9.5).abs() < 0.01 && t.color == COL_MUTED
            )),
            "行内与空 alt 图片不得产生独立题注样式: {text}"
        );
    }

    #[test]
    fn long_cjk_caption_wraps_within_content_width() {
        let caption = "长中文题注".repeat(30);
        let mut media = FakeImageMedia;
        let doc = lay_media(&format!("![{caption}](fake.png)\n"), &mut media);
        let runs: Vec<&TextRun> = doc
            .pages
            .iter()
            .flatten()
            .filter_map(|op| match op {
                DrawOp::Text(t) if (t.size - 9.5).abs() < 0.01 && t.color == COL_MUTED => Some(t),
                _ => None,
            })
            .collect();
        assert!(runs.len() >= 2, "长 CJK 题注应换成多行: {runs:?}");
        assert_eq!(
            runs.iter().map(|t| t.text.as_str()).collect::<String>(),
            caption
        );
        let geom = PageGeom::a4(20.0);
        for run in runs {
            let width: f32 = run
                .text
                .chars()
                .map(|c| FakeMetrics.width_of(c, run.size))
                .sum();
            assert!(
                width <= geom.content_w() + 0.01,
                "题注行宽 {width} 不得越出版心 {}",
                geom.content_w()
            );
        }
    }

    #[test]
    fn long_caption_does_not_overlap_following_inline_image() {
        let caption = "长中文题注".repeat(30);
        let mut media = FakeImageMedia;
        let doc = lay_media(
            &format!("![{caption}](fake.png)\n\n后文 ![行内](fake.png) 文本\n"),
            &mut media,
        );
        let last_caption_baseline = doc
            .pages
            .iter()
            .flatten()
            .filter_map(|op| match op {
                DrawOp::Text(t) if (t.size - 9.5).abs() < 0.01 && t.color == COL_MUTED => Some(t.y),
                _ => None,
            })
            .fold(f32::MIN, f32::max);
        let inline_image_y = doc
            .pages
            .iter()
            .flatten()
            .filter_map(|op| match op {
                DrawOp::Image { y, .. } => Some(*y),
                _ => None,
            })
            .next_back()
            .expect("应有后续行内图片");
        assert!(
            inline_image_y >= last_caption_baseline + 2.0,
            "行内图片顶部 {inline_image_y} 不得侵入末行题注基线 {last_caption_baseline} 之上"
        );
    }

    #[test]
    fn missing_standalone_image_uses_placeholder_without_duplicating_caption() {
        let doc = lay(
            "![缺失独立题注](missing.png)\n\n\
             <img src=\"missing.png\" alt=\"缺失 HTML 题注\">\n\n\
             前文 ![行内缺失替代](missing.png) 后文\n",
            false,
        );
        let text = all_text(&doc);
        for caption in ["缺失独立题注", "缺失 HTML 题注"] {
            assert_eq!(text.matches(caption).count(), 1, "{text}");
        }
        assert_eq!(
            text.matches("[图片]").count(),
            2,
            "缺图应使用通用占位: {text}"
        );
        assert_eq!(
            text.matches("行内缺失替代").count(),
            1,
            "行内缺图仍保留 alt: {text}"
        );
    }

    #[test]
    fn standalone_image_is_placed_below_previous_content() {
        let md =
            "| 列A | 列B |\n| --- | --- |\n| 1 | 2 |\n\n![图片](fake.png)\n\n图片之后的段落。\n";
        let session = Session::open(md);
        let metrics = FakeMetrics;
        let mut media = FakeImageMedia;
        let mermaids = std::collections::HashMap::new();
        let doc = layout_document(
            &session,
            &metrics,
            PageGeom::a4(20.0),
            LayoutOpts::default(),
            Some(&mut media),
            &mermaids,
        );
        let ops: Vec<&DrawOp> = doc.pages.iter().flat_map(|p| p.iter()).collect();
        let img_idx = ops
            .iter()
            .position(|op| matches!(op, DrawOp::Image { .. }))
            .expect("应绘制图片");
        let (img_y, img_h) = match ops[img_idx] {
            DrawOp::Image { y, h, .. } => (*y, *h),
            _ => unreachable!(),
        };
        // B4：独占一行图片限高 6×正文行高（11pt × 1.55 × 6 ≈ 102.3pt），与编辑视图一致。
        let cap = 6.0 * metrics.line_height(BODY_SIZE);
        assert!(
            (img_h - cap).abs() < 0.01,
            "图片高度应为 {cap}pt，实际 {img_h}"
        );

        // 图片顶部不得高于表格底边（表格横线的最大 y）。
        let table_bottom = ops[..img_idx]
            .iter()
            .filter_map(|op| match op {
                DrawOp::Line { y1, y2, .. } => Some(y1.max(*y2)),
                _ => None,
            })
            .fold(f32::MIN, f32::max);
        assert!(
            img_y >= table_bottom,
            "独占图片顶部 {img_y} 不应高于表格底边 {table_bottom}"
        );

        // 后续文本应排在图片底部之下（行高按图片高度前进）。
        let next_text_y = ops[img_idx + 1..]
            .iter()
            .find_map(|op| match op {
                DrawOp::Text(t) => Some(t.y),
                _ => None,
            })
            .expect("图片后应有文本");
        assert!(
            next_text_y >= img_y + img_h,
            "后续文本基线 {next_text_y} 应在图片底部 {} 之下",
            img_y + img_h
        );
    }

    /// B3：details 正文子块（Paragraph / BlockQuote / 列表 / 表格…）必须出现在 PDF 里。
    /// 真机曾出现只有 summary 标题行、正文完全丢失的退化。
    #[test]
    fn details_body_blocks_render_in_pdf() {
        let md = "# H\n\n\
                  <details>\n\
                  <summary>S</summary>\n\
                  para inside\n\n\
                  > quote inside\n\
                  </details>\n\n\
                  tail\n";
        let session = Session::open(md);
        let doc = lay_with(&session, false);
        let text = all_text(&doc);
        assert!(
            text.contains("para inside"),
            "details 内段落必须进 PDF: {text}"
        );
        assert!(
            text.contains("quote inside"),
            "details 内引用必须进 PDF: {text}"
        );
        assert!(
            text.contains("tail"),
            "details 后的尾部段落必须进 PDF: {text}"
        );
    }

    /// 真机 b7-outline.md 复刻：details 内正文段落 + 前后空行。
    #[test]
    fn details_from_real_machine_walkthrough_renders_body() {
        let md = "## 2.3 折叠块\n\n\
                  <details>\n\
                  <summary>展开查看中文摘要</summary>\n\n\
                  折叠块内部的中文正文也应出现在 PDF 中。\n\n\
                  </details>\n\n\
                  ## 2.4 图片\n";
        let session = Session::open(md);
        let doc = lay_with(&session, false);
        let text = all_text(&doc);
        assert!(
            text.contains("展开查看中文摘要"),
            "summary 必须进 PDF: {text}"
        );
        assert!(
            text.contains("折叠块内部的中文正文也应出现在 PDF 中。"),
            "details 内正文必须进 PDF: {text}"
        );
        assert!(text.contains("2.4 图片"), "后续块必须进 PDF: {text}");
    }

    /// Task 11 真机抽查：引用/列表容器内的 `<details>` 标签不得作为正文进 PDF。
    /// 容器块不是 Details 头块，标签藏在块文本里；屏幕层由折叠区间消隐，
    /// PDF 层必须同样只输出 summary 与正文。
    #[test]
    fn container_details_tags_do_not_leak_into_pdf() {
        let md = "> <details>\n\
                  > <summary>引用摘要</summary>\n\
                  > 引用正文\n\
                  > </details>\n\n\
                  - 列表项\n\
                  \x20 <details>\n\
                  \x20 <summary>列表摘要</summary>\n\
                  \x20 列表正文\n\
                  \x20 </details>\n\n\
                  尾\n";
        let session = Session::open(md);
        let doc = lay_with(&session, false);
        let text = all_text(&doc);
        assert!(text.contains("引用摘要"), "summary 必须进 PDF: {text}");
        assert!(text.contains("引用正文"), "容器内正文必须进 PDF: {text}");
        assert!(text.contains("列表摘要"), "列表 summary 必须进 PDF: {text}");
        assert!(
            text.contains("列表正文"),
            "列表容器内正文必须进 PDF: {text}"
        );
        assert!(text.contains("尾"), "后续段落必须进 PDF: {text}");
        let lower = text.to_lowercase();
        assert!(
            !lower.contains("<details")
                && !lower.contains("</details")
                && !lower.contains("<summary"),
            "容器内 details 标签不得进 PDF: {text}"
        );
    }

    /// 证据生成（手动）：用真实字体 + 媒体管线把样例渲染为 `target/wave4-sample.pdf`。
    /// 运行：`cargo test -p veil generate_wave4_sample_pdf -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore = "证据生成：需要真实 CJK 字体，非 CI 门禁"]
    fn generate_wave4_sample_pdf() {
        use crate::platform::pdf::{find_cjk_font, render_pages, FontMetrics};
        use crate::platform::pdf_media::PdfMedia;

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .to_path_buf();
        let evidence = root.join(".trae/specs/wave4-pdf-fidelity/evidence");
        let md = std::fs::read_to_string(evidence.join("sample.md")).expect("样例 Markdown");
        let font_path = find_cjk_font().expect("系统 CJK 字体");
        let metrics =
            FontMetrics::load(std::path::Path::new(font_path), md.chars()).expect("字体度量");
        let session = Session::open(&md);
        let mut media = PdfMedia::new(Some(evidence.clone()), Some(crate::math_renderer()));
        let mermaids = std::collections::HashMap::new();
        let doc = layout_document(
            &session,
            &metrics,
            PageGeom::a4(20.0),
            LayoutOpts {
                toc: true,
                page_numbers: true,
            },
            Some(&mut media),
            &mermaids,
        );
        let bytes = render_pages(
            &doc.pages,
            &doc.images,
            &doc.outline,
            &doc.links,
            std::path::Path::new(font_path),
        )
        .expect("渲染 PDF");
        let out = root.join("target/wave4-sample.pdf");
        std::fs::write(&out, &bytes).expect("写样例 PDF");
        eprintln!(
            "sample: pages={} outline={} images={} bytes={}",
            doc.pages.len(),
            doc.outline.len(),
            doc.images.len(),
            bytes.len()
        );
        assert!(doc.pages.len() >= 3, "样例应跨多页");
        assert!(!doc.outline.is_empty(), "样例应有书签");
    }

    // ===== Task 4：PDF 导出缺陷回归 =====

    /// DEF-T09-003：脚注按首次引用递增编号；定义区不得残留 `[^id]:`/`**`/反引号。
    #[test]
    fn footnote_numbers_follow_first_reference_and_defs_have_no_markup() {
        let md = "第一处[^a]，第二处[^b]，再次[^a]。\n\n\
                  [^a]: 注**一**\n\
                  [^b]: 注`二`\n";
        let doc = lay(md, false);
        let text = all_text(&doc);
        assert!(text.contains("[1]"), "首引号应为 1: {text}");
        assert!(text.contains("[2]"), "次引号应为 2: {text}");
        assert!(!text.contains("[^a]"), "定义标记不得外泄: {text}");
        assert!(!text.contains("[^b]"), "定义标记不得外泄: {text}");
        assert!(!text.contains("**"), "强调标记不得外泄: {text}");
        assert!(!text.contains('`'), "代码标记不得外泄: {text}");
        assert!(text.contains("注一"), "定义正文应可读: {text}");
        assert!(text.contains("注二"), "定义正文应可读: {text}");
    }

    /// 无定义的脚注引用保留可读原文，不生成指向空条目的断号。
    #[test]
    fn undefined_footnote_keeps_literal_reference() {
        let doc = lay("引用[^missing] 后文。\n", false);
        let text = all_text(&doc);
        assert!(text.contains("[^missing]"), "应保留原文: {text}");
        assert!(!text.contains("[1]"), "不得伪造编号: {text}");
    }

    /// Front Matter 是元数据：PDF 不输出元数据正文。
    #[test]
    fn front_matter_is_not_rendered_in_pdf() {
        let doc = lay("---\ntitle: 标题\ntags: [a, b]\n---\n正文开始。\n", false);
        let text = all_text(&doc);
        assert!(!text.contains("title:"), "元数据不得输出: {text}");
        assert!(!text.contains("tags:"), "元数据不得输出: {text}");
        assert!(text.contains("正文开始"), "正文须保留: {text}");
    }

    /// DEF-T09-E1-004：嵌套引用不得残留字面 `>`。
    #[test]
    fn nested_blockquote_strips_all_level_markers() {
        let doc = lay("> 外层\n> > 内层文本\n> > > 第三层\n", false);
        let text = all_text(&doc);
        assert!(!text.contains('>'), "不得残留引用标记: {text}");
        assert!(text.contains("内层文本"), "内层正文须保留: {text}");
    }

    #[test]
    fn fenced_code_labels_keep_language_and_omit_unlabelled_blocks() {
        let doc = lay(
            "```rust\nlet rust = 1;\n```\n\n\
             ```rs\nlet alias = 1;\n```\n\n\
             ```not-a-known-language\nlet unknown = 1;\n```\n\n\
             ```\nlet plain = 1;\n```\n",
            false,
        );
        let labels: Vec<&str> = doc
            .pages
            .iter()
            .flatten()
            .filter_map(|op| match op {
                DrawOp::Text(t) if (t.size - 8.0).abs() < 0.01 && t.color == COL_MUTED => {
                    Some(t.text.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(labels, ["rust", "rs", "not-a-known-language"]);
        let text = all_text(&doc);
        for body in [
            "let rust = 1;",
            "let alias = 1;",
            "let unknown = 1;",
            "let plain = 1;",
        ] {
            assert!(text.contains(body), "代码正文须保留 {body}: {text}");
        }
    }

    /// DEF-T09-E1-005：代码语言标题与首行代码不跨页（遍历边界位置）。
    #[test]
    fn code_language_label_never_orphans_from_body() {
        for n in 0..90 {
            let mut md = String::new();
            for i in 0..n {
                md.push_str(&format!("段落 {i} 文本 text text text text\n\n"));
            }
            md.push_str("```rust\nlet a = 1;\nlet b = 2;\n```\n");
            let doc = lay(&md, false);
            let mut label_page = None;
            let mut body_page = None;
            for (pi, page) in doc.pages.iter().enumerate() {
                for op in page {
                    if let DrawOp::Text(t) = op {
                        if t.text == "rust" {
                            label_page = Some(pi);
                        }
                        if t.text.contains("let a = 1;") {
                            body_page = Some(pi);
                        }
                    }
                }
            }
            assert_eq!(label_page, body_page, "n={n}：代码标题与代码体被分页拆开");
        }
    }

    // Task 7：屏幕 / PDF 断行一致性差分工具（只读生产逻辑，不改行为）。
    #[path = "../../pdf_layout_consistency_tests.rs"]
    mod consistency;

    // Task 9：PDF 断行候选组件 spike（dev-dependency unicode-linebreak，不进生产路径）。
    #[path = "../../pdf_linebreak_spike_tests.rs"]
    mod spike;
}
