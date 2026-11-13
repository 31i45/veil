//! Markdown → PDF 版面（Wave 4）：块级排版、按宽度换行、分页、目录、页脚。
//!
//! 设计约束：
//! - 分页与书签**同源**（`pages` 索引即 `outline.page_idx`），禁止二次切分；
//! - 只依赖 `TextMetrics` 与 `MediaProvider` 两个接缝，版面逻辑可纯单测；
//! - 无法渲染的图片/公式一律降级为文本，绝不 panic。

use std::path::Path;

use veil_core::{BlockId, BlockKind, CellAlign, HtmlTag, InlineKind, InlineNode, Range, Session};
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

/// 版面产物：页面绘制指令 + 图片资源 + 多级书签。
pub struct DocLayout {
    pub pages: Vec<Vec<DrawOp>>,
    pub images: Vec<ImageData>,
    pub outline: Vec<OutlineItem>,
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
            underline: false,
            strike: false,
            highlight: None,
            color: COL_TEXT,
        }
    }

    fn with_size(size: f32) -> Self {
        Self {
            size,
            ..Self::body()
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
}

#[derive(Clone)]
enum LItem {
    Ch(char, Style),
    Box(BoxItem),
}

fn item_width(it: &LItem, m: &dyn TextMetrics) -> f32 {
    match it {
        LItem::Ch(c, s) => m.width_of(*c, s.size),
        LItem::Box(b) => b.w,
    }
}

/// 段落是否仅含图片（B4）：可见区间被 Image / `<img>` 节点完整覆盖，且其余只剩空白。
/// `base` 为 `text` 在文档中的起点（`from..to` 为绝对偏移）。
fn paragraph_is_image_only(
    nodes: &[InlineNode],
    text: &str,
    base: usize,
    from: usize,
    to: usize,
) -> bool {
    if to <= from {
        return false;
    }
    let mut covered = 0usize;
    let mut count = 0usize;
    for n in nodes {
        let is_img = matches!(n.kind, InlineKind::Image)
            || matches!(
                n.kind,
                InlineKind::HtmlSelfClosed {
                    tag: HtmlTag::Img,
                    ..
                }
            );
        if is_img {
            let s = n.range.start.max(from);
            let e = n.range.end.min(to);
            if s < e {
                count += 1;
                covered += e - s;
            }
        }
    }
    if count == 0 {
        return false;
    }
    let visible = text
        .get(from.saturating_sub(base)..to.saturating_sub(base))
        .unwrap_or("");
    let trimmed = visible.trim();
    !trimmed.is_empty() && covered >= trimmed.len()
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
                    let rest = cur.split_off(bi);
                    lines.push(cur);
                    cur = rest;
                    last_space = None;
                }
                _ => {
                    lines.push(std::mem::take(&mut cur));
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

// =============================================================================
// 版面上下文
// =============================================================================

struct Head {
    level: u8,
    text: String,
    page: usize,
    top_y: f32,
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
}

impl<'a> Ctx<'a> {
    fn new(metrics: &'a dyn TextMetrics, geom: PageGeom) -> Self {
        Self {
            metrics,
            geom,
            pages: Vec::new(),
            images: Vec::new(),
            heads: Vec::new(),
            page: 0,
            y: geom.top(),
            img_max_lines: 2.0,
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
        let mut h = self.metrics.line_height(base_size);
        for it in &line {
            if let LItem::Box(b) = it {
                h = h.max(b.h);
            }
        }
        self.ensure(h);
        // 独占一行的 Box（如块级图片）按块级放置：图顶即当前 y，避免按基线对齐时
        // 顶部被抬高 h - ascent 压住前文。含文本/多个条目的行仍按基线对齐。
        let single_box = match line.as_slice() {
            [LItem::Box(b)] => Some(b.baseline),
            _ => None,
        };
        let baseline = self.y + single_box.unwrap_or_else(|| self.metrics.ascent(base_size));

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
        for it in line {
            match it {
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
                    let top = baseline - b.baseline;
                    self.pages[self.page].push(DrawOp::Image {
                        x,
                        y: top,
                        w: b.w,
                        h: b.h,
                        id: b.id,
                    });
                    x += b.w;
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
        self.y += h;
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
        // B4：段落仅含图片 → 放宽到 6 行（与编辑视图/HTML 导出同策略）。
        let prev_limit = self.img_max_lines;
        if paragraph_is_image_only(&nodes, &text, start, start, start + visible) {
            self.img_max_lines = 6.0;
        }
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
        self.img_max_lines = prev_limit;
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
                        highlight: Some(COL_CODE_BG),
                        ..style
                    };
                    push_chars(
                        out,
                        text,
                        base,
                        node.content.start.max(ns),
                        node.content.end.min(ne),
                        st,
                    );
                }
                InlineKind::Link => self.emit_inlines(
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
                ),
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
                        })),
                        None => {
                            let st = Style {
                                italic: true,
                                color: COL_MUTED,
                                ..style
                            };
                            for c in alt.chars() {
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
                    let num = self.footnote_no(id_txt);
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
                            })),
                            None => {
                                let st = Style {
                                    italic: true,
                                    color: COL_MUTED,
                                    ..style
                                };
                                for c in alt.chars() {
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

    fn footnote_no(&self, _id: &str) -> usize {
        1
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
        self.y += 5.0;
        if let Some(l) = lang.as_deref().filter(|l| !l.is_empty()) {
            let st = Style {
                size: 8.0,
                color: COL_MUTED,
                ..Style::body()
            };
            let mut items = Vec::new();
            for c in l.chars() {
                items.push(LItem::Ch(c, st));
            }
            if !items.is_empty() {
                for line in fill_lines(items, self.max_w() - 10.0, self.metrics) {
                    self.emit_line(line, 10.0, 8.0, CellAlign::None, self.max_w() - 10.0);
                }
            }
        }

        let st = Style {
            size: CODE_SIZE,
            ..Style::body()
        };
        let line_h = self.metrics.line_height(CODE_SIZE);
        let pad = 5.0;
        let bg_w = self.max_w() - 6.0;
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
        let text = slice_text(s, start, end);
        let nodes = s.block_inlines(id).unwrap_or_default();
        let rail_color = callout.as_ref().map_or(COL_RULE, |(_, _, c)| *c);
        let start_y = self.y;
        let page = self.page;
        let bg_at = self.pages[page].len();
        let indent = 16.0;
        let mut first = true;
        for (li, line) in text.split('\n').enumerate() {
            // 该行在块文本中的相对起点
            let rel: usize = text.split('\n').take(li).map(|l| l.len() + 1).sum();
            let b = line.as_bytes();
            let mut i = 0;
            while i < b.len() && b[i] == b' ' {
                i += 1;
            }
            if i < b.len() && b[i] == b'>' {
                i += 1;
                if i < b.len() && b[i] == b' ' {
                    i += 1;
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
                self.rich(&text, start, from, to, &nodes, Style::body(), indent, media);
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
        let text = slice_text(s, start, end);
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

    fn table(&mut self, s: &Session, id: BlockId) {
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
            // 单元格换行（至多 6 行）
            let mut cell_lines: Vec<Vec<String>> = Vec::new();
            let mut max_lines = 1usize;
            for (ci, &cw) in widths.iter().enumerate() {
                let txt = row.get(ci).map(|c| c.text.as_str()).unwrap_or("");
                let inner_w = (cw - 2.0 * pad).max(12.0);
                let items: Vec<LItem> = txt
                    .chars()
                    .map(|c| LItem::Ch(c, Style::with_size(TABLE_SIZE)))
                    .collect();
                let mut lines: Vec<String> = fill_lines(items, inner_w, self.metrics)
                    .into_iter()
                    .map(|l| {
                        l.into_iter()
                            .map(|i| match i {
                                LItem::Ch(c, _) => c,
                                LItem::Box(_) => ' ',
                            })
                            .collect::<String>()
                    })
                    .collect();
                if lines.is_empty() {
                    lines.push(String::new());
                }
                if lines.len() > 6 {
                    lines.truncate(6);
                    if let Some(last) = lines.last_mut() {
                        last.push('…');
                    }
                }
                max_lines = max_lines.max(lines.len());
                cell_lines.push(lines);
            }

            let row_h = line_h * max_lines as f32 + pad * 1.4;
            if self.y + row_h > self.geom.bottom() && self.y > self.geom.top() + 1.0 {
                self.new_page();
            }
            let top = self.y;
            if ri == 0 {
                self.pages[self.page].push(DrawOp::Rect {
                    x: x0,
                    y: top,
                    w: widths.iter().sum(),
                    h: row_h,
                    color: COL_HEAD_BG,
                });
            }

            // 单元格文本
            let mut x = x0;
            for ci in 0..ncols {
                let align = t.aligns.get(ci).copied().unwrap_or_default();
                let mut cy = top + pad * 0.7;
                for text in &cell_lines[ci] {
                    if text.is_empty() {
                        cy += line_h;
                        continue;
                    }
                    let st = Style {
                        size: TABLE_SIZE,
                        bold: ri == 0,
                        ..Style::body()
                    };
                    let mut items: Vec<LItem> = Vec::new();
                    for c in text.chars() {
                        items.push(LItem::Ch(c, st));
                    }
                    // 直接定位（避免 emit_line 推进全局 y）
                    let wid: f32 = items.iter().map(|i| item_width(i, self.metrics)).sum();
                    let inner = widths[ci] - 2.0 * pad;
                    let off = match align {
                        CellAlign::Center => (inner - wid).max(0.0) / 2.0,
                        CellAlign::Right => (inner - wid).max(0.0),
                        _ => 0.0,
                    };
                    let baseline = cy + self.metrics.ascent(TABLE_SIZE);
                    let run = TextRun {
                        x: x + pad + off,
                        y: baseline,
                        size: TABLE_SIZE,
                        text: text.clone(),
                        bold: ri == 0,
                        italic: false,
                        underline: false,
                        strike: false,
                        highlight: None,
                        color: COL_TEXT,
                    };
                    self.pages[self.page].push(DrawOp::Text(run));
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
        }
        self.y += 8.0;
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
        for chunk in chunks.iter() {
            let mut ops: Vec<DrawOp> = Vec::new();
            let mut y = self.geom.top();
            ops.push(DrawOp::Text(TextRun {
                x: self.geom.margin_x,
                y: y + self.metrics.ascent(18.0),
                size: 18.0,
                text: t(Key::PrintTocTitle).into(),
                bold: true,
                italic: false,
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
                ops.push(DrawOp::Text(TextRun {
                    x: self.geom.margin_x + indent,
                    y: y + self.metrics.ascent(size),
                    size,
                    text: title,
                    bold: h.level <= 1,
                    italic: false,
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
                underline: false,
                strike: false,
                highlight: None,
                color: COL_MUTED,
            }));
        }
    }
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
        let w = m.text_width(buf, s.size);
        pages[page].push(DrawOp::Text(TextRun {
            x: *x,
            y: baseline,
            size: s.size,
            text: std::mem::take(buf),
            bold: s.bold,
            italic: s.italic,
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
) -> DocLayout {
    let mut ctx = Ctx::new(metrics, geom);
    ctx.new_page();

    for (id, kind, range) in session.blocks() {
        let (start, end) = (range.start, range.end);
        match kind {
            BlockKind::Paragraph => ctx.paragraph(session, id, start, end, 0.0, &mut media),
            BlockKind::AtxHeading(l) | BlockKind::SetextHeading(l) => ctx.heading(session, id, l),
            BlockKind::FencedCode(_) => ctx.code(session, id, start, end, true),
            BlockKind::IndentedCode => ctx.code(session, id, start, end, false),
            BlockKind::Math => ctx.block_math(session, id, &mut media),
            BlockKind::Table => ctx.table(session, id),
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
            BlockKind::FootnoteDef => ctx.raw_text(session, start, end, 9.0),
            BlockKind::HtmlBlock => ctx.raw_text(session, start, end, 9.0),
            BlockKind::FrontMatter(_) => ctx.raw_text(session, start, end, 9.0),
            // B3：Details 头块只含开标签行 + summary 行；PDF 恒按展开态，
            // 只画 summary 标题行，正文由后续子块各自按块型排版。
            BlockKind::Details { summary, .. } => ctx.details_summary(&summary),
        }
    }

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

    DocLayout {
        pages: ctx.pages,
        images: ctx.images,
        outline,
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
    }

    fn lay(md: &str, toc: bool) -> DocLayout {
        let session = Session::open(md);
        let metrics = FakeMetrics;
        let geom = PageGeom::a4(20.0);
        layout_document(
            &session,
            &metrics,
            geom,
            LayoutOpts {
                toc,
                page_numbers: true,
            },
            None,
        )
    }

    fn lay_with(session: &Session, toc: bool) -> DocLayout {
        let metrics = FakeMetrics;
        let geom = PageGeom::a4(20.0);
        layout_document(
            session,
            &metrics,
            geom,
            LayoutOpts {
                toc,
                page_numbers: true,
            },
            None,
        )
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

    #[test]
    fn standalone_image_is_placed_below_previous_content() {
        let md =
            "| 列A | 列B |\n| --- | --- |\n| 1 | 2 |\n\n![图片](fake.png)\n\n图片之后的段落。\n";
        let session = Session::open(md);
        let metrics = FakeMetrics;
        let mut media = FakeImageMedia;
        let doc = layout_document(
            &session,
            &metrics,
            PageGeom::a4(20.0),
            LayoutOpts::default(),
            Some(&mut media),
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
        assert!(text.contains("para inside"), "details 内段落必须进 PDF: {text}");
        assert!(text.contains("quote inside"), "details 内引用必须进 PDF: {text}");
        assert!(text.contains("tail"), "details 后的尾部段落必须进 PDF: {text}");
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
        assert!(text.contains("展开查看中文摘要"), "summary 必须进 PDF: {text}");
        assert!(
            text.contains("折叠块内部的中文正文也应出现在 PDF 中。"),
            "details 内正文必须进 PDF: {text}"
        );
        assert!(text.contains("2.4 图片"), "后续块必须进 PDF: {text}");
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
        let doc = layout_document(
            &session,
            &metrics,
            PageGeom::a4(20.0),
            LayoutOpts {
                toc: true,
                page_numbers: true,
            },
            Some(&mut media),
        );
        let bytes = render_pages(
            &doc.pages,
            &doc.images,
            &doc.outline,
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
}
