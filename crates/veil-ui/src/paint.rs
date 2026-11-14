//! 帧合成：背景/块内容/选区/光标与 preedit 绘制。所有输入为
//! 已排版的几何与缓存，本文件不触发重新排版（除 paint 开头的两遍收敛）。
//! 查找栏与表格网格的绘制片段在 `view::search_view` / `view::table_view`。

use crate::app::App;
use crate::chrome;
use crate::document::block_index::BlockGeom;
use crate::geometry::{Rect, MARGIN};
use crate::i18n::{t, tf, Key};
use crate::view::table::{CellImgAsset, CellMath};
use crate::view::{
    cell_math_spans, ctx_menu, image_is_standalone, inline_details_spans, node_active, outline,
    oversize_notice, quick_open, search, BlockLayout, Frame, ImageCaption, InlineAsset, InlineFold,
    InlineImg, LayoutEngine, LayoutKey, Memo, OverKind, TableKey, UnderKind,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use veil_core::{BlockId, BlockKind, HtmlTag, InlineKind, InlineNode, Range};

/// 专注模式压暗强度：背景色以该 alpha 覆盖非光标块（越大越暗，255 = 全遮）。
const FOCUS_DIM_ALPHA: u8 = 160;

/// 待排版图片：(节点区间, url, 显式宽高, alt)。显式宽高仅来自 HTML `<img width height>`；
/// alt 用于独立成行图片的题注（视图派生，不写回文档）。
type PendingImage = (Range, String, (Option<u32>, Option<u32>), String);

/// 打字机模式锚点：视口高比例（0.42 ≈ 上四成处），光标行中心恒定位于此处。
const TYPEWRITER_ANCHOR: f32 = 0.42;

/// 拖放悬停压暗强度（阶段 1）：内容区背景覆盖，突出居中提示条。
const DROP_DIM_ALPHA: u8 = 120;

/// 链接悬停浮层（N5）：视口边留白 / 锚点上方间距（逻辑像素，乘 scale）。
const LINK_TIP_EDGE: f32 = 8.0;
const LINK_TIP_GAP: f32 = 24.0;

/// 链接浮层可用文本宽（物理像素）：视口宽减去两侧留白与内边距。
pub(crate) fn link_tip_max_text_w(vp: crate::chrome::Viewport, scale: f32) -> f32 {
    let pad = crate::theme::surface::padding(scale);
    (vp.w - 2.0 * (LINK_TIP_EDGE * scale + pad)).max(1.0)
}

fn link_tip_char_w(c: char, scale: f32) -> f32 {
    if c.is_ascii() {
        crate::tokens::typography::UI_SIZE * 0.52 * scale
    } else {
        crate::tokens::typography::UI_SIZE * scale
    }
}

/// 按视口宽截断单行 URL：放不下时保留前缀并追加省略号。
pub(crate) fn truncate_link_tip(text: &str, max_w: f32, scale: f32) -> (String, f32) {
    let w: f32 = text.chars().map(|c| link_tip_char_w(c, scale)).sum();
    if w <= max_w {
        return (text.to_string(), w);
    }
    let ell_w = link_tip_char_w('…', scale);
    let mut out = String::new();
    let mut w = 0.0;
    for c in text.chars() {
        let cw = link_tip_char_w(c, scale);
        if w + cw + ell_w > max_w {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    (out, w + ell_w)
}

/// 链接浮层矩形：锚点上方 24px，水平以锚点为中心，四边夹取在视口内。
/// 上方放不下时改到锚点下方（仍夹取），保证整卡可见且不越界。
pub(crate) fn link_tip_rect(
    w: f32,
    h: f32,
    anchor_x: f32,
    anchor_y: f32,
    vp: crate::chrome::Viewport,
    scale: f32,
) -> Rect {
    let edge = LINK_TIP_EDGE * scale;
    let gap = LINK_TIP_GAP * scale;
    let max_x = (vp.x + vp.w - edge - w).max(vp.x + edge);
    let x = (anchor_x - w / 2.0).clamp(vp.x + edge, max_x);
    let mut y = anchor_y - gap - h;
    if y < vp.y + edge {
        y = (anchor_y + gap).min((vp.y + vp.h - edge - h).max(vp.y + edge));
    }
    Rect { x, y, w, h }
}

/// 模态文本接缝：度量走 engine（不栅格化），绘制走 chrome 标签位图缓存。
struct ModalText<'a> {
    labels: &'a mut crate::chrome::LabelCache,
    engine: &'a mut LayoutEngine,
    scale: f32,
}

impl crate::view::modal::Text for ModalText<'_> {
    fn width(&mut self, s: &str) -> f32 {
        self.engine.measure(s).0
    }

    fn draw(&mut self, frame: &mut Frame, s: &str, color: [u8; 3], x: i32, y: i32) {
        if s.is_empty() {
            return;
        }
        if let Some(l) = self.labels.get_or_insert(self.engine, s, color, self.scale) {
            frame.blit(&l.pixels, l.w, l.h, x, y);
        }
    }

    fn title_width(&mut self, s: &str) -> f32 {
        self.engine.measure_title(s).0
    }

    fn draw_title(&mut self, frame: &mut Frame, s: &str, color: [u8; 3], x: i32, y: i32) {
        if s.is_empty() {
            return;
        }
        if let Some(l) = self
            .labels
            .get_or_insert_title(self.engine, s, color, self.scale)
        {
            frame.blit(&l.pixels, l.w, l.h, x, y);
        }
    }
}

/// present 前置条件：帧像素数与 surface 缓冲长度一致才可整块拷贝。
/// 最小化时 Resized 宽/高为 0，surface 不 resize 而帧被钳成 1×1，长度失配即丢帧。
pub(crate) fn can_present(frame_len: usize, target_len: usize) -> bool {
    frame_len == target_len
}

/// 插入点的焦点表达：聚焦实心；失焦空心框（不依赖 alpha 合成，语义最直接）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaretVisual {
    Solid,
    Hollow,
}

/// 焦点状态 → 插入点视觉（纯函数，便于断言）。
pub(crate) fn caret_visual(focused: bool) -> CaretVisual {
    if focused {
        CaretVisual::Solid
    } else {
        CaretVisual::Hollow
    }
}

/// Callout（内置五种 + 自定义）的强调色；非 Callout 块返回 None。
fn callout_accent(kind: &BlockKind) -> Option<[u8; 3]> {
    match kind {
        BlockKind::Admonition(ty) => Some(ty.color()),
        BlockKind::Callout(c) => Some(c.color),
        _ => None,
    }
}

/// Callout 图标（短文本 + 强调色）；普通引用块返回 None。
fn callout_icon(kind: &BlockKind) -> Option<(&str, [u8; 3])> {
    match kind {
        BlockKind::Admonition(ty) => Some((ty.icon(), ty.color())),
        BlockKind::Callout(c) => Some((c.icon.as_str(), c.color)),
        _ => None,
    }
}

/// 强调色按 `ratio` 混入底色（Callout 浅底色；PDF/HTML 用同一比例保持视觉一致）。
pub(crate) fn tint(accent: [u8; 3], base: [u8; 4], ratio: f32) -> [u8; 4] {
    let r = ratio.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| ((a as f32) * r + (b as f32) * (1.0 - r)).round() as u8;
    [
        mix(accent[0], base[0]),
        mix(accent[1], base[1]),
        mix(accent[2], base[2]),
        255,
    ]
}

/// ▼ 箭头（尖头朝下）：等腰三角形，底在顶部。用 Frame 逐行 fill_rect 画。
fn draw_arrow_down(frame: &mut Frame, cx: i32, cy: i32, size: i32, color: [u8; 3]) {
    let c = [color[0], color[1], color[2], 255];
    let half = size / 2;
    for row in -half..=half {
        let bar_w = (half - row).max(1) as u32;
        frame.fill_rect(cx - bar_w as i32 / 2, cy + row, bar_w, 1, c);
    }
}

/// ▶ 箭头（尖头朝右）：等腰三角形，底在左侧。
fn draw_arrow_right(frame: &mut Frame, cx: i32, cy: i32, size: i32, color: [u8; 3]) {
    let c = [color[0], color[1], color[2], 255];
    let half = size / 2;
    for col in -half..=half {
        let bar_h = (half - col).max(1) as u32;
        frame.fill_rect(cx + col, cy - bar_h as i32 / 2, 1, bar_h, c);
    }
}

/// 行内前置装饰（块与表格共用同一套颜色/圆角规则）：`x/y/w/h` 为目标矩形，
/// 调用方已按像素对齐并外扩 1px（装饰比字形框略大，观感更完整）。
/// 顺序语义：code 底色在最先，mark 其次，kbd 最后（重叠时后者覆盖前者）。
#[allow(clippy::too_many_arguments)] // 内部位图操作参数天然多（同其它绘制原语）
pub(crate) fn paint_under_decor(
    frame: &mut Frame,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    kind: &UnderKind,
    theme: &crate::theme::Theme,
    scale: f32,
) {
    let radius = (2.0 * scale).max(1.0);
    match kind {
        UnderKind::MarkBg => frame.fill_rect(x, y, w, h, [255u8, 247, 153, 180]),
        UnderKind::KbdFrame => {
            // 边框用「外框边框色 + 内缩 1px 底色」叠出（stroke 的透明挖空
            // 在 src-over 下无效，会把整块涂成边框色）。
            frame.fill_rounded_rect(x, y, w.max(2), h.max(2), radius, [160u8, 160, 160, 255]);
            if w > 2 && h > 2 {
                frame.fill_rounded_rect(
                    x + 1,
                    y + 1,
                    w - 2,
                    h - 2,
                    (radius - 1.0).max(0.0),
                    [245u8, 245, 245, 255],
                );
            }
        }
        UnderKind::ImagePlaceholder => {
            frame.fill_rounded_rect(x, y, w.max(2), h.max(2), radius, [190u8, 190, 190, 255]);
            if w > 2 && h > 2 {
                frame.fill_rounded_rect(
                    x + 1,
                    y + 1,
                    w - 2,
                    h - 2,
                    (radius - 1.0).max(0.0),
                    [235u8, 235, 235, 255],
                );
            }
        }
        UnderKind::InlineCodeBg => {
            frame.fill_rounded_rect(
                x,
                y,
                w.max(2),
                h.max(2),
                radius,
                crate::tokens::palette::code_bg(theme),
            );
        }
        UnderKind::StyleBg([r, g, b]) => {
            frame.fill_rect(x, y, w, h, [*r, *g, *b, 255]);
        }
    }
}

/// 行内后置装饰（块与表格共用）：`(x, y_top, w, h)` 为区间在目标坐标的矩形
/// （逐视觉行分段），下划线画在行底往上 2px。
#[allow(clippy::too_many_arguments)] // 内部位图操作参数天然多（同其它绘制原语）
pub(crate) fn paint_over_decor(
    frame: &mut Frame,
    x: i32,
    y_top: i32,
    w: u32,
    h: f32,
    kind: &OverKind,
    theme: &crate::theme::Theme,
    scale: f32,
) {
    match kind {
        OverKind::DottedUnderline => {
            let color = [theme.muted[0], theme.muted[1], theme.muted[2], 200];
            let uy = y_top + h as i32 - 2; // 行底往上 2px，恒 1px 厚
                                           // 点线：x 方向每 `step` 画 `dash` 段（1–2px），随 DPI 缩放。
            let step = (3.0 * scale).round().max(3.0) as i32;
            let dash = (step / 2).max(1);
            let end = x + w as i32;
            let mut cx = x;
            while cx < end {
                let d = dash.min(end - cx) as u32;
                frame.fill_rect(cx, uy, d, 1, color);
                cx += step;
            }
        }
        OverKind::Underline => {
            // 实线下划线：行底往上 1–2px，粗细随 DPI 缩放。
            let th = (scale.round() as i32).max(1);
            let uy = y_top + h as i32 - 1 - th;
            frame.fill_rect(
                x,
                uy,
                w,
                th as u32,
                [theme.fg[0], theme.fg[1], theme.fg[2], 255],
            );
        }
        OverKind::Overline => {
            let th = (scale.round() as i32).max(1);
            frame.fill_rect(
                x,
                y_top,
                w,
                th as u32,
                [theme.fg[0], theme.fg[1], theme.fg[2], 255],
            );
        }
    }
}

/// 代码块头部控件布局（物理像素，视口坐标）：复制按钮 + 语言标签。
/// 锚定代码块首行（围栏行消隐后为空白行），不参与排版，因此不改换行/块高。
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct CodeHeaderLayout {
    /// 复制按钮命中/绘制矩形。
    pub(crate) button: Rect,
    /// 语言标签文本（无语言/未知信息时为 None）。
    pub(crate) label: Option<String>,
    /// 首行行顶与行高（标签垂直居中用）。
    pub(crate) line_top: f32,
    pub(crate) line_h: f32,
}

/// 绘制代码块头部的语言标签与复制按钮（在代码文本之后调用，保证按钮在最上层）。
/// 独立函数而非 `App` 方法：绘制循环借用了 `self.index.blocks`，按字段借用避开整 self 冲突。
fn paint_code_header(
    frame: &mut Frame,
    engine: &mut LayoutEngine,
    labels: &mut crate::chrome::LabelCache,
    h: &CodeHeaderLayout,
    theme: &crate::theme::Theme,
    scale: f32,
    content_x: f32,
) {
    let b = h.button;
    frame.fill_rounded_rect(
        b.x.round() as i32,
        b.y.round() as i32,
        b.w.max(1.0) as u32,
        b.h.max(1.0) as u32,
        (3.0 * scale).max(1.0),
        theme.popup_bg,
    );
    // 1px 描边：四边各一条，避免引入新的绘制原语。
    let border = theme.chrome_border;
    let (bx, by, bw, bh) = (
        b.x.round() as i32,
        b.y.round() as i32,
        b.w.max(1.0) as i32,
        b.h.max(1.0) as i32,
    );
    frame.fill_rect(bx, by, bw as u32, 1, border);
    frame.fill_rect(bx, by + bh - 1, bw as u32, 1, border);
    frame.fill_rect(bx, by, 1, bh as u32, border);
    frame.fill_rect(bx + bw - 1, by, 1, bh as u32, border);
    if let Some(lab) = labels.get_or_insert(engine, t(Key::CodeCopyButton), theme.muted, scale) {
        frame.blit(
            &lab.pixels,
            lab.w,
            lab.h,
            bx + (bw - lab.w as i32) / 2,
            by + (bh - lab.h as i32) / 2,
        );
    }
    // 语言标签右对齐在按钮左侧；未知语言保留 info string 原文。
    if let Some(text) = h.label.as_deref() {
        if let Some(lab) = labels.get_or_insert(engine, text, theme.muted, scale) {
            let gap = (8.0 * scale).round() as i32;
            let x = bx - gap - lab.w as i32;
            if x >= content_x as i32 {
                frame.blit(
                    &lab.pixels,
                    lab.w,
                    lab.h,
                    x,
                    h.line_top.round() as i32 + ((h.line_h - lab.h as f32) / 2.0).round() as i32,
                );
            }
        }
    }
}

impl App {
    /// 代码块头部控件几何：只对 WYSIWYG（非源码/非打印）下的可见围栏块生效。
    /// 位置随 `g.top` / 滚动 / scale 推导，缩放与 DPI 变化后自然稳定。
    pub(crate) fn code_header_layout(&self, g: &BlockGeom) -> Option<CodeHeaderLayout> {
        if self.source_mode() || self.window_state.print.is_some() || self.block_hidden(g) {
            return None;
        }
        let BlockKind::FencedCode(lang) = &g.kind else {
            return None;
        };
        let scale = self.input_state.scale;
        let vp = self.viewport();
        let line_h = self.engine.doc_line_height() * scale;
        let line_top = vp.y + g.top - self.window_state.scroll_y;
        if line_top >= vp.y + vp.h || line_top + line_h <= vp.y {
            return None; // 首行不在视口：不绘制、不可命中
        }
        let pad_x = crate::tokens::doc::CODE_PAD_X * scale;
        let h = (16.0 * scale).round().max(12.0);
        let w = (40.0 * scale).round().max(28.0);
        // 锚定代码卡右上角：卡片宽度跟随最长行（与 code_bg 同源光栅宽度）；
        // 首帧无光栅时回退版心右缘，下一帧即稳定到卡片右缘。
        let base = self.base_key();
        let caret = self.effective_caret();
        let ink_w = self
            .cache
            .raster_key(g.id, &g.kind, g.range, base, caret, self.toc_layout_epoch(g))
            .and_then(|k| self.cache.raster(g.id, k))
            .map(|r| r.1 as f32);
        let right = match ink_w {
            Some(rw) => self.content_x() + rw.max(w + pad_x),
            None => self.content_x() + self.content_width() - pad_x,
        };
        Some(CodeHeaderLayout {
            button: Rect {
                x: right - w,
                y: line_top + (line_h - h) * 0.5,
                w,
                h,
            },
            label: lang
                .as_deref()
                .and_then(crate::view::highlight::language_label),
            line_top,
            line_h,
        })
    }

    /// 行内公式图：tex 内容寻址缓存；未命中时渲染并写缓存，失败记入失败表并返回 None
    /// （调用方降级为源码/文本显示，正文与表格单元格共用同一策略与缓存）。
    fn inline_math_image(&mut self, tex: &str) -> Option<&crate::math::MathImage> {
        let key = self.inline_key();
        let dpi_q = self.dpi_q();
        if self.cache.inline_failed(tex, dpi_q) {
            return None;
        }
        if self.cache.inline_math(tex, key).is_none() {
            let scale = self.input_state.scale;
            let fg = self.input_state.theme.fg;
            match self.ports.math.render(tex, false, scale, fg) {
                Ok(img) => {
                    self.cache
                        .inline_math
                        .insert(tex.to_owned(), Memo { key, value: img });
                }
                Err(e) => {
                    eprintln!("行内公式渲染失败（降级为源码显示）: {e}");
                    self.cache.inline_failed.insert(
                        tex.to_owned(),
                        Memo {
                            key: dpi_q,
                            value: (),
                        },
                    );
                    return None;
                }
            }
        }
        self.cache.inline_math(tex, key)
    }

    /// 补齐缺失布局（键命中则零开销）。Reveal 激活已并入布局键，光标移动后
    /// 指纹失配即自动重排，无需另设失效调用（ADR-009 决策四）。
    /// 行内公式（切片 8）：块内非激活 Math 节点渲染出图（tex 内容寻址缓存），
    /// 传占位信息给 layout；失败记 inline_failed 降级源码显示，不每帧重试。
    /// 文档内图片（切片 13）：非激活 Image 节点按文档目录解析相对 URL，经 ImageLoader
    /// 加载自然尺寸位图；宽超版心等比缩放后占位；失败/远程 URL 降级 alt 文本。
    pub(crate) fn ensure_layouts_range(&mut self, indices: std::ops::Range<usize>) {
        let width = self.content_width();
        let caret = self.effective_caret();
        let base = self.base_key();
        let reveal_all = self.source_mode();
        // 脚注编号表：每轮构建一次（O(引用数)），块内查询 O(1)，不逐块回查会话。
        let footnotes = self.footnote_ctx();
        self.engine.set_scale(self.input_state.scale);
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        // TOC 目录条目：仅文档含标记块且需要渲染目录时才扫描标题（编辑/源码态零成本）。
        let mut toc_entries: Option<Vec<(u8, String)>> = None;
        // 先收集待排块与其行内公式/图片（session 借用期），再调渲染器/加载器（&mut self）
        let mut pending = Vec::new();
        for g in blocks {
            if self.block_hidden(g) {
                continue; // 折叠 details 的隐藏子块不排版（展开时再惰性补齐）
            }
            let (id, kind, range) = (g.id, g.kind.clone(), g.range);
            let toc_epoch = self.toc_layout_epoch(g);
            // `[TOC]` 标记：视图态渲染生成目录，源码态/光标在块内时原样显示标记。
            let toc_rendered =
                g.toc_marker && !reveal_all && !(range.start <= caret && caret <= range.end);
            if self
                .cache
                .layout(id, &kind, range, base, caret, toc_epoch)
                .is_some_and(|l| l.toc_rendered == toc_rendered)
            {
                continue;
            }
            let text = self.editor.session().text_slice(range).unwrap_or_default();
            let toc_text = if toc_rendered {
                let entries = toc_entries.get_or_insert_with(|| self.toc_entries());
                Some(crate::view::toc_display_text(entries))
            } else {
                None
            };
            let inlines = self.editor.session().block_inlines(id).unwrap_or_default();
            let mut maths: Vec<(Range, String)> = Vec::new(); // (节点区间, tex)
                                                              // (节点区间, url, 显式宽高)：Markdown 图片无显式尺寸（None, None），
                                                              // HTML `<img width height>` 由解析层白名单收集后带下来。
            let mut images: Vec<PendingImage> = Vec::new();
            let mut stack: Vec<&InlineNode> = inlines.iter().collect();
            while let Some(n) = stack.pop() {
                if !node_active(n.range, caret) {
                    match n.kind {
                        InlineKind::Math => {
                            let tex = self
                                .editor
                                .session()
                                .text_slice(n.content)
                                .unwrap_or_default();
                            maths.push((n.range, tex));
                        }
                        InlineKind::Image => {
                            if let Some(url) = n.url {
                                let url = self.editor.session().text_slice(url).unwrap_or_default();
                                let alt = self
                                    .editor
                                    .session()
                                    .text_slice(n.content)
                                    .unwrap_or_default();
                                images.push((n.range, url, (None, None), alt));
                            }
                        }
                        // 行内/块级 `<img src alt>`：本地 src 走与 Markdown Image 相同的
                        // 加载/缩略图路径；远程地址由解析层存在 `remote_src`（src 为空），
                        // 这里与 Markdown 远程图片走同一套按需加载。
                        InlineKind::HtmlSelfClosed {
                            tag: HtmlTag::Img,
                            ref attrs,
                        } => {
                            if let Some(src) = attrs.src.as_deref().or(attrs.remote_src.as_deref())
                            {
                                images.push((
                                    n.range,
                                    src.to_string(),
                                    (attrs.width, attrs.height),
                                    attrs.alt.clone().unwrap_or_default(),
                                ));
                            }
                        }
                        _ => {}
                    }
                }
                stack.extend(n.children.iter());
            }
            // B3：行内 `<details>` 折叠输入（仅含该标记的块才扫描节点，普通块零开销）。
            let folds: Vec<InlineFold> = if text.contains("<details") {
                inline_details_spans(&inlines, range)
                    .into_iter()
                    .map(|s| InlineFold {
                        body: s.body,
                        expanded: self.ui_state.expanded_inline_details.contains(&(id, s.key)),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            pending.push((
                id,
                kind,
                range,
                text,
                inlines,
                maths,
                images,
                folds,
                toc_text,
                g.toc_marker,
                g.toc_list,
                toc_epoch,
            ));
        }
        let image_base = self.input_state.revs.image_base;
        for (
            id,
            kind,
            range,
            text,
            inlines,
            maths,
            images,
            folds,
            toc_text,
            toc_marker,
            toc_list,
            toc,
        ) in pending
        {
            let mut imgs = Vec::new();
            let container = self.editor.session().block_container(id);
            let img_max_w = self.image_max_width(container, width);
            // B4：行内图片限高 4×正文行高；独占一行改用「窗口可视高度 × 80%」——
            // 行高倍数会把正常截图压成小黑块。宽度上限 = 版心宽 - 容器内边距。
            // 独占判定按「图片所在视觉行」而非整块：段落里图片后另起一行的正文
            // （如 `![a](p.png)\nX`）不应把该图降级为行内小图。
            // 非显式尺寸时仅缩小、不放大。
            let scale = self.input_state.scale;
            let line_h = self.engine.doc_line_height() * scale;
            let inline_max_h = crate::tokens::doc::IMG_INLINE_LINES * line_h;
            for (nrange, tex) in maths {
                // 失败记忆/渲染失败 → 降级源码（Reveal 消隐态）显示，不每帧重试。
                let Some(img) = self.inline_math_image(&tex) else {
                    continue;
                };
                imgs.push(InlineImg {
                    range: nrange,
                    key: InlineAsset::Math(tex),
                    width: img.width,
                    height: img.height,
                    baseline: img.baseline,
                });
            }
            // 图片题注候选（独立成行 + alt 非空）：与是否加载成功无关，
            // 缺失时题注仍显示（布局层把缺失节点降级为通用占位以避免与 alt 重复）。
            let mut captions: Vec<ImageCaption> = Vec::new();
            for (nrange, url, explicit, alt) in images {
                // HTML `width`/`height` 是 CSS 逻辑像素 → 乘 DPI 转布局物理像素。
                let explicit = (
                    explicit
                        .0
                        .map(|v| (v as f32 * scale).round().max(1.0) as u32),
                    explicit
                        .1
                        .map(|v| (v as f32 * scale).round().max(1.0) as u32),
                );
                // 独占判定按「图片所在视觉行」：行内图片不产生题注，避免打断行内排版。
                let standalone = image_is_standalone(&text, range, nrange);
                let alt = alt.trim();
                if standalone && !alt.is_empty() {
                    // 题注单行显示：alt 内部换行折成空格（其余字符不改写）。
                    captions.push(ImageCaption {
                        range: nrange,
                        text: alt.replace(['\n', '\r'], " "),
                    });
                }
                let max_h = if standalone {
                    self.image_max_height()
                } else {
                    inline_max_h
                };
                let key = if let Some(path) = local_image_path(&url, self.doc.path.as_deref()) {
                    let key = path.to_string_lossy().into_owned();
                    if self.cache.image_failed(&key, image_base) {
                        continue;
                    }
                    if self.cache.image(&key, image_base).is_none() {
                        match self.ports.images.load(&path) {
                            Ok(img) => {
                                self.cache.inline_images.insert(
                                    key.clone(),
                                    Memo {
                                        key: image_base,
                                        value: img,
                                    },
                                );
                            }
                            Err(e) => {
                                eprintln!(
                                    "图片加载失败（降级为 alt 文本） {}: {e}",
                                    path.display()
                                );
                                self.cache.image_failed.insert(
                                    key,
                                    Memo {
                                        key: image_base,
                                        value: (),
                                    },
                                );
                                continue;
                            }
                        }
                    }
                    key
                } else if is_remote_image_url(&url) {
                    // 远程图片：默认自动加载；全局阻止/已失败/源码模式 → alt 占位。
                    if self.source_mode()
                        || !self.config.allow_remote_images
                        || self.cache.remote_failed.contains(&url)
                    {
                        continue;
                    }
                    let Some((w, h)) = self.cache.remote_image(&url).map(|i| (i.width, i.height))
                    else {
                        // 本帧占位；首次遇到才发起请求（pending 去重）。
                        self.request_remote_image(&url);
                        continue;
                    };
                    let (dw, dh) =
                        crate::view::image_display_size(w, h, explicit, img_max_w, max_h);
                    imgs.push(InlineImg {
                        range: nrange,
                        key: InlineAsset::Image(url),
                        width: dw,
                        height: dh,
                        baseline: dh, // 底边贴基线
                    });
                    continue;
                } else {
                    continue; // 内联 data URL 或草稿相对路径：降级 alt 文本
                };
                let nat = &self.cache.inline_images[&key].value;
                let (dw, dh) = crate::view::image_display_size(
                    nat.width, nat.height, explicit, img_max_w, max_h,
                );
                imgs.push(InlineImg {
                    range: nrange,
                    key: InlineAsset::Image(key),
                    width: dw,
                    height: dh,
                    baseline: dh, // 底边贴基线
                });
            }
            let l = match toc_text {
                Some(toc) => self.engine.layout_toc(&toc, width),
                None => {
                    let mut l = self.engine.layout_block_with_captions(
                        kind.clone(),
                        &text,
                        range,
                        &inlines,
                        width,
                        caret,
                        &imgs,
                        reveal_all,
                        Some(&footnotes),
                        &folds,
                        // 有序列表显示编号：编号在块索引层按文档顺序派生（BlockKind 不含起始号）。
                        self.index.list_ordinals.get(&id).copied(),
                        toc_list,
                        &captions,
                    );
                    l.toc_source = toc_marker;
                    l
                }
            };
            let reveal = l.reveal_state(&kind, range, caret);
            self.cache.layouts.insert(
                id,
                Memo {
                    key: LayoutKey { base, reveal, toc },
                    value: l,
                },
            );
            // 块高从估算变为实测：标记几何待重算，渲染帧只做一次 O(块数) 累计。
            self.index.geometry_stale = true;
        }
    }

    /// 发起远程图片请求（pending 去重）：全局阻止/已失败/源码模式一律不请求。
    fn request_remote_image(&mut self, url: &str) {
        if self.source_mode()
            || !self.config.allow_remote_images
            || self.cache.remote_failed.contains(url)
        {
            return;
        }
        if self.cache.remote_pending.insert(url.to_string()) {
            self.ports.images.request_remote(url);
        }
    }

    /// 仅补齐视口内（含上下各一屏 overscan）缺失布局（P1b 惰性排版）。
    pub(crate) fn ensure_layouts_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_layouts_range(range);
    }

    /// 测试专用：等价全量排版；不参与非测试构建，避免 dead_code。
    #[cfg(test)]
    pub(crate) fn ensure_layouts_all(&mut self) {
        self.ensure_layouts_range(0..self.index.blocks.len());
    }

    /// 按需排版单块（含其数学渲染/表格网格）：光标、命中、垂直移动等路径使用。
    pub(crate) fn ensure_block(&mut self, id: BlockId) {
        let Some(i) = self.index.blocks.iter().position(|g| g.id == id) else {
            return;
        };
        self.ensure_layouts_range(i..i + 1);
        self.ensure_math_range(i..i + 1);
        self.ensure_tables_range(i..i + 1);
    }

    /// 视口覆盖的块索引区间（含上下各一屏 overscan）：`top` 已由几何重算保证
    /// 单调不减，二分定位首尾，避免逐块线性扫描。
    pub(crate) fn visible_index_range(&self) -> std::ops::Range<usize> {
        let vp = self.viewport();
        let lo = self.window_state.scroll_y - vp.h;
        let hi = self.window_state.scroll_y + 2.0 * vp.h;
        let blocks = &self.index.blocks;
        let start = blocks.partition_point(|g| g.top + g.height < lo);
        let end = blocks.partition_point(|g| g.top < hi);
        start.min(blocks.len())..end.max(start).min(blocks.len())
    }

    /// 光标在块内为源码态不渲染；失败记入 math_failed 不再每帧重试（降级源码显示）。
    /// 源码模式整块显示原文，不需要公式图，也不产图缓存。
    pub(crate) fn ensure_math_range(&mut self, indices: std::ops::Range<usize>) {
        if self.source_mode() {
            return;
        }
        let caret = self.effective_caret();
        let scale = self.input_state.scale;
        let fg = self.input_state.theme.fg;
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind.clone(), g.range);
            if kind != BlockKind::Math
                || (range.start <= caret && caret <= range.end)
                || self.cache.math(id, self.math_key(id)).is_some()
                || self.cache.math_failed(id, self.math_fail_key(id))
            {
                continue;
            }
            let Some(tex) = self.editor.session().math_source(id) else {
                continue;
            };
            match self.ports.math.render(&tex, true, scale, fg) {
                Ok(img) => {
                    self.cache.math_imgs.insert(
                        id,
                        Memo {
                            key: self.math_key(id),
                            value: img,
                        },
                    );
                    self.index.geometry_stale = true;
                }
                Err(e) => {
                    eprintln!("公式渲染失败（降级为源码显示）: {e}");
                    self.cache.math_failed.insert(
                        id,
                        Memo {
                            key: self.math_fail_key(id),
                            value: (),
                        },
                    );
                    self.index.geometry_stale = true;
                }
            }
        }
    }

    /// 仅渲染视口内（含 overscan）的数学块。
    pub(crate) fn ensure_math_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_math_range(range);
    }

    /// 测试专用：全量数学渲染。
    #[cfg(test)]
    pub(crate) fn ensure_math(&mut self) {
        self.ensure_math_range(0..self.index.blocks.len());
    }

    /// 数学块当前是否按渲染图显示：光标在块内（Reveal 同语义）或未渲染时显示源码。
    pub(crate) fn math_shows_image(&self, id: BlockId, range: &veil_core::Range) -> bool {
        if self.source_mode() {
            return false; // 源码模式：全文显原文，不渲染公式图
        }
        let caret = self.effective_caret();
        !(range.start <= caret && caret <= range.end)
            && self.cache.math(id, self.math_key(id)).is_some()
    }

    /// 判定 caret 是否落在指定 Table 的任一单元格内容区间内（不含管道符/分隔行等结构字符）。
    pub(crate) fn caret_in_table_cell(&self, id: BlockId) -> bool {
        let caret = self.effective_caret();
        self.editor
            .session()
            .table(id)
            .map(|t| {
                t.header.iter().any(|c| cell_contains(&c.range, caret))
                    || t.rows
                        .iter()
                        .any(|row| row.iter().any(|c| cell_contains(&c.range, caret)))
            })
            .unwrap_or(false)
    }

    /// 找到 caret 所在的表格与单元格（flat 索引 = header.len() + rows 累积）。
    fn caret_table_and_cell(&self, caret: usize) -> Option<(BlockId, veil_core::Table)> {
        let blocks = &self.index.blocks;
        // 从后往前找最近的包含 caret 的 Table block（先到先得：上方的 Table 先遍历到）
        for g in blocks.iter().rev() {
            if g.kind != BlockKind::Table {
                continue;
            }
            if !(g.range.start <= caret && caret <= g.range.end) {
                continue;
            }
            let tbl = self.editor.session().table(g.id)?;
            return Some((g.id, tbl));
        }
        None
    }

    /// Tab 前进一格：caret 跳到下一个单元格的内容起点。最后一格时回到首格起点。
    /// 返回 true 表示已处理（不应再走 Indent/Dedent）。
    pub(crate) fn tab_next_cell(&mut self) -> bool {
        self.tab_step(1)
    }

    /// Shift+Tab 后退一格：caret 跳到上一个单元格的内容起点。首格时不移动。
    pub(crate) fn tab_prev_cell(&mut self) -> bool {
        self.tab_step(-1)
    }

    fn tab_step(&mut self, dir: i32) -> bool {
        let caret = self.editor.caret();
        let Some((_, tbl)) = self.caret_table_and_cell(caret) else {
            return false;
        };
        let total = tbl.header.len() + tbl.rows.iter().map(|r| r.len()).sum::<usize>();
        if total == 0 {
            return false;
        }
        let cur = active_cell_index(&tbl, caret);
        let Some(cur_idx) = cur else {
            // caret 落在管道符/分隔行等结构字符上：Tab 跳到其后第一个单元格；
            // 其后无格（如分隔行末尾）则按「末格」处理（追加行），Shift+Tab 交给 Indent 反缩进。
            if dir > 0 {
                if let Some(idx) = first_cell_after(&tbl, caret) {
                    self.editor.set_caret(flat_cell_start(&tbl, idx));
                    self.scroll_to_caret();
                    self.request_redraw();
                    return true;
                }
                return self.table_ctrl_enter();
            }
            return false;
        };
        let next = if dir > 0 {
            // 末格 Tab：与 Typora 对齐——追加一行并落首格，而不是绕回表首
            if cur_idx + 1 == total {
                return self.table_ctrl_enter();
            }
            cur_idx + 1
        } else if cur_idx == 0 {
            return true; // 首格 Shift+Tab：不动，吞掉按键
        } else {
            cur_idx - 1
        };
        let target = flat_cell_start(&tbl, next);
        self.editor.set_caret(target);
        self.scroll_to_caret();
        self.request_redraw();
        true
    }

    /// 网格态光标矩形：活动单元格内容左上角（与点击落点、Tab 落点同源）。
    /// 表格显示为网格时，源码排版坐标与所见网格不一致，直接复用会画出「悬空光标」。
    /// `doc_top` 为块的文档坐标（非视口坐标），滚动量在此统一扣除。
    pub(crate) fn table_caret_xy(
        &self,
        id: BlockId,
        doc_top: f32,
        caret: usize,
    ) -> Option<(f32, f32, f32)> {
        let tbl = self.editor.session().table(id)?;
        let idx = active_cell_index(&tbl, caret)?;
        let grid = self.cache.table(id, self.table_key(id))?;
        let cell = grid.cells.get(idx)?;
        let scale = self.input_state.scale;
        let vp = self.viewport();
        Some((
            self.content_x() + grid.x + cell.x,
            vp.y + doc_top - self.window_state.scroll_y + cell.y,
            self.engine.doc_line_height() * scale,
        ))
    }

    /// 表格按需网格排版：caret 在结构字符上（管道/分隔行）或表外 → 排版网格；
    /// caret 在单元格内容区间内 → 同样排版网格（活动单元格高亮）；
    /// 仅 caret 在表内但不落在任何单元格时跳过（源码态）。
    /// 网格缓存与文本布局缓存独立：网格仅供渲染/命中，源码态仍走既有块排版。
    pub(crate) fn ensure_tables_range(&mut self, indices: std::ops::Range<usize>) {
        if self.source_mode() {
            return; // 源码模式：表格显源码，不建网格缓存
        }
        let width = self.content_width();
        let caret = self.effective_caret();
        let fg = self.input_state.theme.fg;
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        let image_base = self.input_state.revs.image_base;
        let doc_path = self.doc.path.clone();
        let mut pending = Vec::new();
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind.clone(), g.range);
            if kind != BlockKind::Table {
                continue;
            }
            let key = self.table_key_at(id, caret);
            if self.cache.table(id, key).is_some() {
                continue;
            }
            let inside_block = range.start <= caret && caret <= range.end;
            if inside_block && key.active == u32::MAX {
                // caret 落在管道符/分隔行等结构字符上 → 源码态，不排版网格
                continue;
            }
            if let Some(t) = self.editor.session().table(id) {
                // 活动格显示源码（含标记）；离开即恢复富文本。
                let active = (key.active != u32::MAX)
                    .then(|| {
                        let idx = key.active as usize;
                        flat_cell_range(&t, idx)
                            .and_then(|r| self.editor.session().text_slice(r))
                            .map(|src| (idx, src))
                    })
                    .flatten();
                // 含 `[^id]` 的单元格按纯文本降级：core 的 cell.text 已剥掉 `[^`/`]`，
                // 这里取源码原文交给网格（不编号、不着色），避免「引用的 id 混进正文」。
                let raw_cells = footnote_cell_sources(self.editor.session(), &t);
                // 非活动格内联公式：先收可见区间与 TeX，渲染放到下面的循环（&mut self）。
                let seeds = table_cell_math_seeds(self.editor.session(), &t, key.active);
                // 非活动格内联图片：直接从 cell.inlines 提取 URL 并加载。
                let seeds_img = table_cell_img_seeds(self.editor.session(), &t, key.active);
                pending.push((id, key, t, active, raw_cells, seeds, seeds_img, g.container));
            }
        }
        for (id, key, t, active, raw_cells, seeds, seeds_img, container) in pending {
            let maths = self.cell_math_assets(seeds);
            let img_assets = self.cell_img_assets(seeds_img, doc_path.as_deref(), image_base);
            let widths = self.ui_state.tables.widths.get(&id).cloned();
            // 容器内表格按容器缩进排版：版心宽扣除缩进，网格整体右移。
            let indent = self.table_indent(container);
            let width =
                (width - indent).max(crate::view::table::MIN_COL_WIDTH * self.input_state.scale);
            let mut l = self.engine.layout_table_opts(
                &t,
                width,
                fg,
                active.as_ref().map(|(idx, src)| (*idx, src.as_str())),
                Some(&raw_cells),
                (!maths.is_empty()).then_some(&maths),
                widths.as_deref(),
                (!img_assets.is_empty()).then_some(&img_assets),
            );
            l.x = indent;
            self.cache.tables.insert(id, Memo { key, value: l });
            self.index.geometry_stale = true;
        }
    }

    /// 表格单元格内联公式：渲染（复用 inline_math 缓存）并整理成排版输入。
    /// 渲染失败/已失败记忆的公式跳过 → 该格降级为可见文本（与正文一致）。
    fn cell_math_assets(
        &mut self,
        seeds: HashMap<usize, Vec<CellMathSeed>>,
    ) -> HashMap<usize, Vec<CellMath>> {
        let mut out = HashMap::new();
        for (flat, list) in seeds {
            let mut items = Vec::with_capacity(list.len());
            for seed in list {
                let Some(img) = self.inline_math_image(&seed.tex) else {
                    continue;
                };
                let (w, h) = (img.width, img.height);
                items.push(CellMath {
                    range: seed.range,
                    tex: seed.tex,
                    w,
                    h,
                });
            }
            if !items.is_empty() {
                out.insert(flat, items);
            }
        }
        out
    }

    /// 表格单元格内联图片：本地路径复用 inline_images，远程 URL 复用远程会话缓存。
    /// 全局阻止 / 源码模式 / 已失败记忆 / 草稿相对路径 → 跳过（降级为 alt 文本）。
    fn cell_img_assets(
        &mut self,
        seeds: HashMap<usize, Vec<String>>,
        doc_path: Option<&Path>,
        image_base: u32,
    ) -> HashMap<usize, Vec<CellImgAsset>> {
        let mut out = HashMap::new();
        for (flat, urls) in seeds {
            let mut items = Vec::with_capacity(urls.len());
            for url in urls {
                if let Some(path) = local_image_path(&url, doc_path) {
                    let key = path.to_string_lossy().into_owned();
                    if self.cache.image_failed(&key, image_base) {
                        continue;
                    }
                    if self.cache.image(&key, image_base).is_none() {
                        match self.ports.images.load(&path) {
                            Ok(img) => {
                                self.cache.inline_images.insert(
                                    key.clone(),
                                    Memo {
                                        key: image_base,
                                        value: img,
                                    },
                                );
                            }
                            Err(e) => {
                                eprintln!(
                                    "表格图片加载失败（降级为 alt 文本） {}: {e}",
                                    path.display()
                                );
                                self.cache.image_failed.insert(
                                    key,
                                    Memo {
                                        key: image_base,
                                        value: (),
                                    },
                                );
                                continue;
                            }
                        }
                    }
                    let nat = self.cache.inline_images.get(&key).unwrap();
                    items.push(CellImgAsset {
                        w: nat.value.width,
                        h: nat.value.height,
                        pixels: nat.value.pixels.clone(),
                    });
                } else if is_remote_image_url(&url) {
                    if self.source_mode()
                        || !self.config.allow_remote_images
                        || self.cache.remote_failed.contains(&url)
                    {
                        continue;
                    }
                    let Some(asset) = self.cache.remote_image(&url).map(|nat| CellImgAsset {
                        w: nat.width,
                        h: nat.height,
                        pixels: nat.pixels.clone(),
                    }) else {
                        self.request_remote_image(&url);
                        continue;
                    };
                    items.push(asset);
                }
            }
            if !items.is_empty() {
                out.insert(flat, items);
            }
        }
        out
    }

    /// 表格网格缓存键：基础键 + 活动单元格（flat 索引；None = 全部富文本）。
    pub(crate) fn table_key(&self, id: BlockId) -> TableKey {
        self.table_key_at(id, self.effective_caret())
    }

    fn table_key_at(&self, id: BlockId, caret: usize) -> TableKey {
        let active = self
            .editor
            .session()
            .table(id)
            .and_then(|t| active_cell_index(&t, caret))
            .map_or(u32::MAX, |i| i as u32);
        TableKey {
            base: self.base_key(),
            active,
            widths: self.ui_state.tables.fingerprint(id),
        }
    }

    /// 仅排版视口内（含 overscan）的表格网格。
    pub(crate) fn ensure_tables_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_tables_range(range);
    }

    /// 测试专用：全量表格排版。
    #[cfg(test)]
    pub(crate) fn ensure_tables(&mut self) {
        self.ensure_tables_range(0..self.index.blocks.len());
    }

    /// 表格当前是否按网格显示：
    /// - caret 在表外 → 显示网格
    /// - caret 在单元格内容区间内 → 显示网格（活动单元格高亮，源码编辑同路径）
    /// - caret 在表内但落在结构字符上（管道/分隔行）→ 源码态
    pub(crate) fn table_shows_grid(&self, id: BlockId, range: &veil_core::Range) -> bool {
        if self.source_mode() {
            return false; // 源码模式：表格显管道与分隔行原文
        }
        let caret = self.effective_caret();
        let inside_block = range.start <= caret && caret <= range.end;
        if inside_block && !self.caret_in_table_cell(id) {
            return false;
        }
        self.cache.table(id, self.table_key(id)).is_some()
    }

    /// 渲染纯化（ADR-009 决策五）：产出完整帧缓冲，只读真值、只填派生缓存。
    /// 无 surface（headless）时仍能运行——`win_size()` 兜底尺寸，供幂等断言与真实像素路径测试。
    /// surface 存在性只影响 present() 侧的外部副作用。
    /// 返回 `&mut Frame` 是为了渲染外测试能继续读像素；产品层 paint() 会在 present 后归还。
    pub(crate) fn render(&mut self) -> &mut Frame {
        let (w, h) = self.win_size();
        // 首次绘制（或索引尚未建立）时补一次；其余时机由编辑/打开/resize 维护
        if self.index.blocks.is_empty() {
            self.refresh_index();
        }
        // B3：光标落在折叠 details 的隐藏区时先展开（键盘/撤销/搜索等任意路径的兜底）。
        self.sync_details_at_caret();
        // 视口驱动惰性排版（P1b）：缓存命中时 ensure 只做键比对；仅当本轮真的
        // 新增了布局/公式/表格（geometry_stale）才需要 O(块数) 重算几何——
        // 编辑路径已在 after_edit 末尾重算过，击键后的首帧直接跳过整篇累计。
        // 估算高度偏小可能让视口底部露出未排版块，故补排后最多再收敛一轮。
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        if self.index.geometry_stale {
            self.recompute_geometry();
            self.ensure_layouts_visible();
            self.ensure_math_visible();
            self.ensure_tables_visible();
            if self.index.geometry_stale {
                self.recompute_geometry();
            }
        }
        let base = self.base_key();
        let caret = self.effective_caret();
        // 帧缓冲跨帧复用：仅尺寸变化才重新分配（每帧只做一次底色填充）
        let mut frame = self.window_state.frame.take().unwrap_or_else(|| {
            Frame::new(
                w,
                h,
                crate::tokens::palette::surface_base(&self.input_state.theme),
            )
        });
        frame.reset(
            w,
            h,
            crate::tokens::palette::surface_base(&self.input_state.theme),
        );
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let vp = self.viewport();
        // 窗口缩放/文档变短后滚动量可能越界：绘制前统一钳位（滚动条与内容同源）。
        // 打印中跳过：滚动即分页断点，末页可越过 max_scroll（页尾留空由帧底色承担）。
        if self.window_state.print.is_none() {
            let max = chrome::max_scroll(vp, self.doc_height());
            self.window_state.scroll_y = self.window_state.scroll_y.clamp(0.0, max);
            self.window_state.goal_y = self.window_state.goal_y.clamp(0.0, max);
        }
        // 打印中不画选区：输出为文档最终形态，不含任何交互态
        let sel = if self.window_state.print.is_none() {
            self.editor.selection()
        } else {
            None
        };
        // 只遍历视口覆盖的块（含 overscan），不再逐块判定后 continue
        let vis = self.visible_index_range();
        // 专注模式压暗需要二次遍历同一区间（主循环按值消费区间）
        let vis_focus = vis.clone();
        let inline_key = self.inline_key();
        let image_base = self.input_state.revs.image_base;
        // 超限块降级提示：内容画完后再画，避免与文本光栅/选区叠加顺序冲突
        let mut oversize: Vec<(i32, f32, usize)> = Vec::new();
        for g in &self.index.blocks[vis] {
            if self.block_hidden(g) {
                continue; // B3：折叠 details 的隐藏子块整组不渲染（非光栅裁剪）
            }
            let (id, kind, range, top) = (g.id, g.kind.clone(), g.range, g.top);
            let sy = (vp.y + top - self.window_state.scroll_y) as i32;
            if (sy as f32) >= vp.y + vp.h || sy + self.block_height(g) as i32 <= vp.y as i32 {
                continue; // 视口外不画（布局已缓存，光栅按需）
            }
            if self.math_shows_image(id, &range) {
                // 数学块渲染态：图整体替换文本光栅（display 公式居中）
                let img = &self.cache.math_imgs[&id].value;
                let x = (self.content_x()
                    + ((self.content_width() - img.width as f32) / 2.0).max(0.0))
                    as i32;
                frame.blit(&img.pixels, img.width, img.height, x, sy);
                continue;
            }
            // 仅网格态才走表格绘制；源码模式（或光标落在结构字符上）必须落到
            // 文本光栅路径，否则表格块只会画空白并跳过原始行。
            if kind == BlockKind::Table
                && self.table_shows_grid(id, &range)
                && self.block_height(g) > 0.0
            {
                // 容器内表格先画引用条（表格分支会提前 continue，装饰 match 不覆盖）。
                self.paint_quote_rails(&mut frame, g, sy);
                self.paint_table(&mut frame, id, sy);
                continue;
            }
            let Some(lk) = self
                .cache
                .raster_key(id, &kind, range, base, caret, self.toc_layout_epoch(g))
            else {
                continue;
            };
            if self.cache.raster(id, lk).is_none() {
                let r = self
                    .engine
                    .rasterize(&self.cache.layouts[&id].value, theme.fg);
                self.cache.rasters.insert(id, Memo { key: lk, value: r });
            }
            let l = &self.cache.layouts[&id].value;
            if matches!(
                kind,
                BlockKind::FencedCode(_)
                    | BlockKind::IndentedCode
                    | BlockKind::HtmlBlock
                    | BlockKind::FrontMatter(_)
            ) {
                let (rw, rh) = {
                    let r = &self.cache.rasters[&id].value;
                    (r.1, r.2)
                };
                let pad_x = (crate::tokens::doc::CODE_PAD_X * scale).round() as i32;
                let pad_y = (crate::tokens::doc::CODE_PAD_Y * scale).round() as i32;
                frame.fill_rect(
                    self.content_x() as i32 - pad_x,
                    sy - pad_y,
                    rw + 2 * pad_x as u32,
                    rh + 2 * pad_y as u32,
                    crate::tokens::palette::code_bg(&theme),
                );
            }
            // Callout（N3b）：强调色 10% 浅底色覆盖整块（含引用缩进区），文字仍后绘。
            if let Some(accent) = callout_accent(&kind) {
                let bh = self.block_height(g) as u32;
                let inset = (crate::tokens::doc::QUOTE_INSET * scale) as i32;
                let x = self.content_x() as i32 - inset;
                let w = (self.content_width() as i32 + inset).max(1) as u32;
                frame.fill_rect(
                    x,
                    sy,
                    w,
                    bh,
                    tint(accent, crate::tokens::palette::surface_base(&theme), 0.10),
                );
            }
            // 查找命中高亮（P3-3）：全部命中用 hover 色，当前命中用选区色；打印不画
            if self.window_state.print.is_none() {
                self.paint_match_highlights(&mut frame, l, &range, sy);
            }
            // 选区高亮：先画底色再 blit 字形，文本保持可读
            if let Some(sel) = sel {
                let (s, e) = (sel.start.max(range.start), sel.end.min(range.end));
                if s < e {
                    for (rx, ry, rw, rh) in
                        self.engine
                            .selection_rects(l, s - range.start, e - range.start)
                    {
                        frame.fill_rect(
                            (self.content_x() + rx) as i32,
                            sy + ry as i32,
                            rw as u32,
                            rh as u32,
                            theme.sel,
                        );
                    }
                }
            }
            // 前置装饰：行内代码底色 / mark 底色 / kbd 圆角边框。在文本光栅之下绘制，
            // 折行 span 按 selection_rects 分段画，每段独立底色/边框。
            // 选区可能覆盖装饰 — 装饰在选区高亮之后、文本 blit 之前，保持可见。
            let cx0 = self.content_x() as i32;
            // 装饰区间已是块相对 doc 坐标（与 strikes 同源），直接交给 selection_rects。
            for &(s, e, ref kind) in &l.decorations_under {
                for (rx, ry, rw2, rh2) in self.engine.selection_rects(l, s, e) {
                    paint_under_decor(
                        &mut frame,
                        cx0 + (rx - 1.0) as i32,
                        sy + (ry - 1.0) as i32,
                        (rw2 + 2.0) as u32,
                        (rh2 + 2.0) as u32,
                        kind,
                        &theme,
                        scale,
                    );
                }
            }
            let (px, rw, rh) = &self.cache.rasters[&id].value;
            // Details 卡片底/边框必须先于文本光栅绘制，否则不透明卡片会把正文整块盖住。
            // 折叠箭头在文本之后补画（见下方块级装饰分支）。
            let details_card = if matches!(&kind, BlockKind::Details { .. }) {
                let expanded = self.ui_state.expanded_details.contains(&id);
                let bh = self.block_height(g) as i32;
                // B2：展开态卡片覆盖紧随其后的子表格，视觉上仍是同一张卡片。
                let extra = if expanded {
                    self.details_children_height(g) as i32
                } else {
                    0
                };
                let cw = self.content_width() as u32;
                let cx = self.content_x() as i32;
                let pad = ((crate::tokens::doc::DETAILS_PAD * scale).max(2.0)) as i32;
                let radius = (3.0 * scale).max(1.0);
                let card_y = sy - pad / 2;
                let card_h = bh + extra + pad;
                let card_x = cx - pad;
                let card_w = cw + 2 * pad as u32;
                let card_bg = if expanded {
                    [250u8, 250, 250, 255]
                } else {
                    [252u8, 252, 252, 255]
                };
                let card_border = [theme.grid[0], theme.grid[1], theme.grid[2], 255];
                frame.fill_rounded_rect(card_x, card_y, card_w, card_h as u32, radius, card_border);
                if card_w > 2 && card_h > 2 {
                    frame.fill_rounded_rect(
                        card_x + 1,
                        card_y + 1,
                        card_w - 2,
                        (card_h - 2) as u32,
                        (radius - 1.0).max(0.0),
                        card_bg,
                    );
                }
                Some((expanded, cx, cw, bh, pad))
            } else {
                None
            };
            // Details 块折叠态：只 blit summary 单行的像素高度，正文区域用卡片底遮罩。
            // 打印态恒展开（skip 折叠钳制），让正文完整渲染。
            let is_collapsed_details = self.window_state.print.is_none()
                && matches!(&kind, BlockKind::Details { .. })
                && !self.ui_state.expanded_details.contains(&id);
            let blit_h = if is_collapsed_details {
                // 折叠高度与几何同源（block_height 已含 DPI 缩放），保证 summary 行不被裁切
                (self.block_height(g).round() as u32).clamp(1, *rh)
            } else {
                *rh
            };
            frame.blit(px, *rw, blit_h, self.content_x() as i32, sy);
            // 删除线（P3-4）：叠加在字形之上；cosmic-text 无原生支持，
            // 按可视行矩形在文本中线画横线（粗细随 DPI 缩放）
            for (s, e) in &l.strikes {
                for (rx, ry, rw2, rh2) in self.engine.selection_rects(l, *s, *e) {
                    let th = (scale as i32).max(1);
                    frame.fill_rect(
                        (self.content_x() + rx) as i32,
                        sy + (ry + rh2 * 0.5) as i32 - th / 2,
                        rw2 as u32,
                        th as u32,
                        [theme.fg[0], theme.fg[1], theme.fg[2], 255],
                    );
                }
            }
            // 后置装饰：abbr 虚线下划线。画在 strikes 之后、imgs 之前。
            for &(s, e, ref kind) in &l.decorations_over {
                for (rx, ry, rw2, rh2) in self.engine.selection_rects(l, s, e) {
                    paint_over_decor(
                        &mut frame,
                        (self.content_x() + rx) as i32,
                        sy + ry as i32,
                        rw2 as u32,
                        rh2,
                        kind,
                        &theme,
                        scale,
                    );
                }
            }
            // 行内公式/图片：位置在排版期已定（PlacedImg），叠在文本光栅之上。
            // 图片按占位显示尺寸最近邻缩放（缓存保留自然尺寸，不生成缩放副本）。
            for p in &l.imgs {
                let dx = (self.content_x() + p.x) as i32;
                let dy = sy + p.y as i32;
                match &p.key {
                    InlineAsset::Math(tex) => {
                        if let Some(img) = self.cache.inline_math(tex, inline_key) {
                            frame.blit(&img.pixels, img.width, img.height, dx, dy);
                        }
                    }
                    InlineAsset::Image(key) => {
                        if let Some(img) = self.cache.image_or_remote(key, image_base) {
                            frame.blit_scaled(
                                &img.pixels,
                                img.width,
                                img.height,
                                p.width,
                                p.height,
                                dx,
                                dy,
                            );
                        }
                    }
                }
            }
            // 块级装饰（切片 12）：引用左竖条；分隔线非激活时画居中横线
            match kind {
                BlockKind::BlockQuote => {
                    let bh = self.block_height(g) as u32;
                    frame.fill_rect(
                        self.content_x() as i32 - (crate::tokens::doc::QUOTE_INSET * scale) as i32,
                        sy,
                        (crate::tokens::doc::QUOTE_RAIL * scale).max(1.0) as u32,
                        bh,
                        theme.grid,
                    );
                }
                BlockKind::Admonition(_) | BlockKind::Callout(_) => {
                    // 提示块：左竖条按类型着色（引用灰条 → 语义/配置色），不改字节。
                    let bh = self.block_height(g) as u32;
                    let rail_color = match callout_accent(&kind) {
                        Some([r, g, b]) => [r, g, b, 255],
                        None => theme.grid,
                    };
                    let rail_x =
                        self.content_x() as i32 - (crate::tokens::doc::QUOTE_INSET * scale) as i32;
                    frame.fill_rect(
                        rail_x,
                        sy,
                        (crate::tokens::doc::QUOTE_RAIL * scale).max(1.0) as u32,
                        bh,
                        rail_color,
                    );
                    // 图标徽标：强调色小方块 + 反色图标字形（三种通道同一图标文本）。
                    if let Some((icon, color)) = callout_icon(&kind) {
                        let sz = (12.0 * scale).max(9.0);
                        let bx = rail_x - sz as i32 - (2.0 * scale) as i32;
                        let radius = (2.5 * scale).max(1.0);
                        frame.fill_rounded_rect(
                            bx,
                            sy,
                            sz as u32,
                            sz as u32,
                            radius,
                            [color[0], color[1], color[2], 255],
                        );
                        let ink = crate::tokens::palette::surface_base(&theme);
                        let ink = [ink[0], ink[1], ink[2]];
                        if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                            &mut self.engine,
                            icon,
                            ink,
                            scale,
                        ) {
                            frame.blit(
                                &lab.pixels,
                                lab.w,
                                lab.h,
                                bx + ((sz - lab.w as f32) / 2.0).round() as i32,
                                sy + ((sz - lab.h as f32) / 2.0).round() as i32,
                            );
                        }
                    }
                }
                BlockKind::Details { .. } => {
                    // 卡片底/边框已在文本光栅之前绘制；这里只补折叠箭头（▶ 收起 / ▼ 展开）。
                    if let Some((expanded, cx, cw, bh, pad)) = details_card {
                        let arrow_size = (6.0 * scale).max(3.0) as i32;
                        let arrow_x = cx + cw as i32 - arrow_size - pad;
                        let arrow_cy = sy + bh / 2;
                        if expanded {
                            draw_arrow_down(&mut frame, arrow_x, arrow_cy, arrow_size, theme.fg);
                        } else {
                            draw_arrow_right(&mut frame, arrow_x, arrow_cy, arrow_size, theme.fg);
                        }
                    }
                }
                BlockKind::FencedCode(_) => {
                    if let Some(h) = self.code_header_layout(g) {
                        let content_x = self.content_x();
                        paint_code_header(
                            &mut frame,
                            &mut self.engine,
                            &mut self.ui_state.chrome.labels,
                            &h,
                            &theme,
                            scale,
                            content_x,
                        );
                    }
                }
                BlockKind::ThematicBreak if !node_active(range, caret) => {
                    let bh = self.block_height(g);
                    frame.fill_rect(
                        self.content_x() as i32,
                        sy + (bh * 0.5) as i32,
                        self.content_width().max(1.0) as u32,
                        1,
                        theme.grid,
                    );
                }
                _ => {}
            }
            if l.truncated {
                oversize.push((sy, l.height, l.full_len));
            }
        }
        // 专注模式（阶段 1）：非光标块整体压暗（背景色半透明覆盖，内容与装饰一并变淡）。
        // 只读视觉，不动真值与派生缓存；当前块保持原样，光标随后绘制仍清晰。打印不压暗。
        if self.window_state.print.is_none() && self.ui_state.writing.focus {
            let focused = self.caret_block_index().map(|i| self.index.blocks[i].id);
            let dim = {
                let bg = crate::tokens::palette::surface_base(&theme);
                [bg[0], bg[1], bg[2], FOCUS_DIM_ALPHA]
            };
            for g in &self.index.blocks[vis_focus] {
                if focused == Some(g.id) {
                    continue;
                }
                let sy = (vp.y + g.top - self.window_state.scroll_y) as i32;
                let h = self.block_height(g).ceil() as u32;
                frame.fill_rect(0, sy, vp.w.ceil() as u32, h.max(1), dim);
            }
        }
        // 超限块降级提示：块高已含提示行，画在文本之后、光标/选区之上
        let notice_x = self.content_x().round() as i32;
        for (sy, bh, full) in oversize {
            let notice = oversize_notice(full);
            let mid = sy as f32 + bh - self.engine.doc_line_height() * scale / 2.0;
            if let Some(l) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &notice,
                theme.muted,
                scale,
            ) {
                frame.blit(
                    &l.pixels,
                    l.w,
                    l.h,
                    notice_x,
                    (mid - l.h as f32 / 2.0).round() as i32,
                );
            }
        }
        // 光标/浮层/chrome 只在屏幕语义下绘制；打印帧 = 纯文档内容（3b）
        if self.window_state.print.is_none() {
            let (line, col) = self.editor.session().line_col(caret).unwrap_or((1, 1));
            let dirty = self.history_live_dirty();
            let external = self.external_state();
            let doc_h = self.doc_height();
            // 查找栏/版本历史打开时文档只读：光标与 preedit 均由浮层承接，不画在正文
            if !self.ui_state.search.open && !self.ui_state.history.open {
                self.paint_caret_and_preedit(&mut frame);
            }
            // 大纲面板（P3-2）：内容之后、chrome 之前（标题区/状态栏覆盖上溢行）
            self.paint_outline(&mut frame);
            // 版本历史（B8a）：左侧快照列表；右侧正文区即只读预览
            self.paint_history(&mut frame);
            // 查找栏（P3-3）：浮在内容/大纲之上、chrome 之前的最后一层内容装饰
            self.paint_search(&mut frame);
            // 快速打开（阶段 1）：浮在查找栏之上
            self.paint_quick(&mut frame);
            // 右键菜单（2f）：浮在全部内容浮层之上、chrome 之前
            self.paint_ctx_menu(&mut frame);
            // 链接悬停提示（N5）：正文之后、chrome 之前的轻量浮层
            self.paint_link_tooltip(&mut frame);
            // 拖放悬停提示（阶段 1）：浮在最上层内容装饰
            self.paint_drop_hint(&mut frame);
            // chrome 最后绘制（不透明覆盖内容；滚动条区保留给状态栏行列）
            let recent = self.recent_labels();
            let flags = self.menu_flags();
            self.ui_state.chrome.paint(
                &mut frame,
                &mut self.engine,
                vp,
                scale,
                &theme,
                self.doc.path.as_deref(),
                dirty,
                external,
                self.persist_state.last_saved_hhmm.as_deref(),
                self.ui_state.writing.words,
                line,
                col,
                doc_h,
                self.window_state.scroll_y,
                &recent,
                flags,
            );
            // L2 命中刻度：滚动条之后绘制，叠在轨道/滑块上（只作位置提示）
            self.paint_search_ticks(&mut frame, vp);
        }
        // 应用内设置模态（优先级高于普通 modal）：覆盖菜单栏与状态栏的最后一层；打印帧不含遮罩
        if self.ui_state.settings.is_some() && self.window_state.print.is_none() {
            self.paint_settings(&mut frame);
        } else if self.ui_state.modal.is_some() && self.window_state.print.is_none() {
            self.paint_modal(&mut frame);
        }
        // 视口驱动缓存回收：paint 帧末尾把 rasters/layouts/tables 从"全部累积"剪到
        // "可见区间 + caret 块 + 块级公式图"量级，防止滚动过程中内存持续增长。
        // 滚动是纯 view-only 操作，不触发 refresh_index → prune_alive，光栅累积永不回收。
        let vp_range = self.visible_index_range();
        let mut keep: HashSet<BlockId> = self.index.blocks[vp_range].iter().map(|g| g.id).collect();
        if let Some(i) = self.caret_block_index() {
            keep.insert(self.index.blocks[i].id);
        }
        keep.extend(self.cache.math_imgs.keys().copied());
        self.cache.prune_viewport(&keep, &base);
        // 记录本帧画出的光标位置：`tick` 据此避免为同一次光标移动再补一帧。
        self.window_state.last_painted_caret = Some(self.editor.caret());
        // 归还缓冲供下一帧复用；返回 &mut Frame 供产品层 present() 与测试断言
        self.window_state.frame = Some(frame);
        self.window_state.frame.as_mut().unwrap()
    }

    /// 呈现分离（ADR-009 决策五）：唯一产生外部副作用的出口。
    /// 从 self.window_state.frame 取缓冲，拷进 surface 并 present；失败属瞬时状态，丢帧不崩溃。
    /// 无 surface（headless）时纯 no-op。
    pub(crate) fn present(&mut self) {
        let Some(frame) = self.window_state.frame.as_ref() else {
            return;
        };
        let Some(surface) = self.window_state.surface.as_mut() else {
            return;
        };
        if let Ok(mut target) = surface.buffer_mut() {
            if !can_present(frame.pixels.len(), target.len()) {
                return;
            }
            target.copy_from_slice(&frame.pixels);
            let _ = target.present();
        }
    }

    /// 帧合成完整路径：render → present。产品层事件循环 Redraw 分支唯一调用。
    pub(crate) fn paint(&mut self) {
        let started = std::time::Instant::now();
        let _ = self.render();
        self.present();
        crate::perf::frame(started.elapsed());
    }

    /// 模态覆盖整窗（含菜单栏/状态栏），布局与绘制共用同一几何。
    pub(crate) fn modal_layout(&mut self) -> crate::view::modal::ModalLayout {
        let (w, h) = self.win_size();
        let vp = Rect {
            x: 0.0,
            y: 0.0,
            w: w as f32,
            h: h as f32,
        };
        let scale = self.input_state.scale;
        // spec 与 labels 同属 ui_state：克隆 spec 以拆开借用
        let spec = self
            .ui_state
            .modal
            .as_ref()
            .map(|a| a.spec.clone())
            .unwrap_or_else(|| {
                crate::view::modal::ModalSpec::new(String::new(), String::new(), Vec::new())
            });
        let mut text = ModalText {
            labels: &mut self.ui_state.chrome.labels,
            engine: &mut self.engine,
            scale,
        };
        crate::view::modal::layout(vp, scale, &spec, &mut text)
    }

    /// 最后一层：遮罩 + 卡片 + 按钮。
    fn paint_modal(&mut self, frame: &mut Frame) {
        let layout = self.modal_layout();
        let Some((spec, state)) = self
            .ui_state
            .modal
            .as_ref()
            .map(|a| (a.spec.clone(), a.state))
        else {
            return;
        };
        let (w, h) = self.win_size();
        let vp = Rect {
            x: 0.0,
            y: 0.0,
            w: w as f32,
            h: h as f32,
        };
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let mut text = ModalText {
            labels: &mut self.ui_state.chrome.labels,
            engine: &mut self.engine,
            scale,
        };
        crate::view::modal::paint(frame, vp, scale, &theme, &spec, &state, &layout, &mut text);
    }

    /// 设置模态布局：全屏视口 + ModalText。
    pub(crate) fn settings_layout(&mut self) -> crate::view::settings::SettingsLayout {
        let (w, h) = self.win_size();
        let vp = Rect {
            x: 0.0,
            y: 0.0,
            w: w as f32,
            h: h as f32,
        };
        let scale = self.input_state.scale;
        let Some(s) = self.ui_state.settings.as_ref() else {
            return crate::view::settings::SettingsLayout {
                card: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                },
                fields: Vec::new(),
                buttons: Vec::new(),
                button_ids: Vec::new(),
                window_geom_clear: None,
                window_geom_value: None,
                scroll: 0,
                shown_rows: 0,
                max_scroll: 0,
            };
        };
        let mut text = ModalText {
            labels: &mut self.ui_state.chrome.labels,
            engine: &mut self.engine,
            scale,
        };
        crate::view::settings::layout(
            vp,
            scale,
            &s.form,
            &mut text,
            &s.buttons,
            s.state.scroll_rows,
        )
    }

    /// 设置模态绘制：遮罩 + 卡片 + 字段 + 按钮。
    fn paint_settings(&mut self, frame: &mut Frame) {
        // 先取 win_size 和 layout（避免与后续的 settings 可变借用冲突）
        let (w, h) = self.win_size();
        let layout = self.settings_layout();
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let vp = Rect {
            x: 0.0,
            y: 0.0,
            w: w as f32,
            h: h as f32,
        };
        let Some(s) = self.ui_state.settings.as_mut() else {
            return;
        };
        // 缓存 layout 到 settings state，便于后续交互（命中判定复用同一几何）
        s.layout = Some(layout.clone());
        let mut text = ModalText {
            labels: &mut self.ui_state.chrome.labels,
            engine: &mut self.engine,
            scale,
        };
        crate::view::settings::paint(
            frame, vp, scale, &theme, &s.form, &s.state, &layout, &s.buttons, &mut text,
        );
    }

    /// 大纲面板（P3-2）：底色 → 条目（含悬停底色）→ 标题区（后画，覆盖上溢行）。
    /// 面板宽已从版心扣除；标签位图复用 chrome 缓存（键含颜色/scale）。
    pub(crate) fn paint_outline(&mut self, frame: &mut Frame) {
        let Some(panel) = self.outline_rect() else {
            return;
        };
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let count = self.ui_state.outline.entries.len();
        // 当前条目由光标每帧派生（不落条目状态）；仅光标变化时把当前条目滚入视野，
        // 否则用户手动滚动大纲会被每帧拉回。
        let caret = self.effective_caret();
        let current = outline::current_index(&self.ui_state.outline.entries, caret);
        let max_scroll = outline::max_scroll(panel, count, scale);
        self.ui_state.outline.scroll = self.ui_state.outline.scroll.clamp(0.0, max_scroll);
        if self.ui_state.outline.last_caret != Some(caret) {
            if let Some(ci) = current {
                if max_scroll > 0.0 {
                    self.ui_state.outline.scroll = outline::ensure_visible(
                        self.ui_state.outline.scroll,
                        ci,
                        count,
                        panel,
                        scale,
                    );
                }
            }
            self.ui_state.outline.last_caret = Some(caret);
        }
        let x = panel.x.round() as i32;
        let y = panel.y.round() as i32;
        let w = panel.w.ceil() as u32;
        frame.fill_surface_card(
            x,
            y,
            w,
            panel.h.ceil() as u32,
            scale,
            theme.dark,
            crate::tokens::palette::surface_popup(&theme),
            theme.chrome_border,
        );
        let fade = self.ui_state.outline.fade.view(self.ui_state.outline.hover);
        for row in outline::visible_rows(panel, count, self.ui_state.outline.scroll, scale) {
            let is_current = current == Some(row.index);
            // 当前条目与悬停同底色：只画一次，避免重复叠加变暗。
            // 悬停高亮走过渡不透明度（S6）；当前条目是派生状态，始终满值。
            if is_current {
                frame.fill_hover(
                    x,
                    row.y.round() as i32,
                    w,
                    row.h.ceil() as u32,
                    scale,
                    theme.hover,
                );
            } else if let Some(a) = fade.alpha_of(row.index) {
                frame.fill_hover(
                    x,
                    row.y.round() as i32,
                    w,
                    row.h.ceil() as u32,
                    scale,
                    [theme.hover[0], theme.hover[1], theme.hover[2], a],
                );
            }
            if is_current {
                // 左内侧 accent 指示条：文字缩进与 x 不因指示条挪动。
                let bar_h = (row.h - 12.0 * scale).max(3.0);
                frame.fill_rounded_rect(
                    (panel.x + 4.0 * scale).round() as i32,
                    (row.y + (row.h - bar_h) / 2.0).round() as i32,
                    (3.0 * scale).max(1.0) as u32,
                    bar_h.ceil() as u32,
                    1.5 * scale,
                    [theme.accent[0], theme.accent[1], theme.accent[2], 255],
                );
            }
            let (level, text) = {
                let e = &self.ui_state.outline.entries[row.index];
                (e.level, e.text.clone())
            };
            let indent = outline::INDENT_STEP * scale * level.saturating_sub(1) as f32;
            let label = outline::fit_label(&text, (row.w - indent).max(0.0), scale);
            let lx = row.x + indent;
            let mid = row.y + row.h / 2.0;
            if let Some(l) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &label,
                theme.text,
                scale,
            ) {
                let ly = mid - l.h as f32 / 2.0;
                frame.blit(&l.pixels, l.w, l.h, lx.round() as i32, ly.round() as i32);
            }
        }
        // 溢出滚动指示：轨道贴面板右内侧，拇指按 scroll/max_scroll 线性映射；
        // 起止均从条目区顶端开始，不进入标题区。
        if count > 0 && max_scroll > 0.0 {
            let top = outline::rows_top(panel, scale);
            let bottom = panel.y + panel.h - 4.0 * scale;
            let track_h = (bottom - top).max(0.0);
            if track_h > 0.0 {
                let track_x = panel.x + panel.w - 6.0 * scale;
                let track_w = (3.0 * scale).max(1.0) as u32;
                let radius = 1.5 * scale;
                frame.fill_rounded_rect(
                    track_x.round() as i32,
                    top.round() as i32,
                    track_w,
                    track_h.ceil() as u32,
                    radius,
                    theme.chrome_border,
                );
                let content_h = count as f32 * outline::ROW_H * scale;
                let thumb_h = (track_h * track_h / content_h)
                    .max(crate::theme::surface::scrollbar_min_thumb(scale))
                    .min(track_h);
                let ty = top + (self.ui_state.outline.scroll / max_scroll) * (track_h - thumb_h);
                frame.fill_rounded_rect(
                    track_x.round() as i32,
                    ty.round() as i32,
                    track_w,
                    thumb_h.ceil() as u32,
                    radius,
                    theme.thumb,
                );
            }
        }
        if count == 0 {
            if let Some(l) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                t(Key::OutlineEmpty),
                theme.muted,
                scale,
            ) {
                let top = outline::rows_top(panel, scale) + outline::PAD * scale;
                frame.blit(
                    &l.pixels,
                    l.w,
                    l.h,
                    (panel.x + outline::PAD * scale).round() as i32,
                    top.round() as i32,
                );
            }
        }
        // 标题区（覆盖滚动到标题区内的行）：顶角与卡片同曲率、底边平直，
        // 内缩 1px 避开卡片圆角边框；分隔线与条目文字起止对齐（不顶到卡片边）
        let head_h = (outline::HEAD_H * scale).ceil() as u32;
        let r = crate::theme::surface::corner_radius(scale);
        let popup_bg = crate::tokens::palette::surface_popup(&theme);
        frame.fill_rounded_rect(
            x + 1,
            y + 1,
            w.saturating_sub(2),
            head_h,
            (r - 1.0).max(0.0),
            popup_bg,
        );
        let sq = r.ceil().max(1.0) as i32;
        let hi = y + head_h as i32;
        frame.fill_rect(x + 1, hi - sq, w.saturating_sub(2), sq as u32, popup_bg);
        frame.fill_rect(
            (panel.x + outline::PAD * scale).round() as i32,
            hi - 1,
            (panel.w - 2.0 * outline::PAD * scale).max(1.0) as u32,
            1,
            theme.chrome_border,
        );
        if let Some(l) = self.ui_state.chrome.labels.get_or_insert(
            &mut self.engine,
            t(Key::OutlineTitle),
            theme.muted,
            scale,
        ) {
            let ly = panel.y + (outline::HEAD_H * scale - l.h as f32) / 2.0;
            frame.blit(
                &l.pixels,
                l.w,
                l.h,
                (panel.x + outline::PAD * scale).round() as i32,
                ly.round() as i32,
            );
        }
    }

    /// 版本历史面板（N1/B8a）：左侧快照列表（时间/大小/当前标记）+ 底部操作按钮。
    /// 右侧正文区由正常渲染管线绘制——预览只是在打开期间把 `self.editor` 换成
    /// 快照文本的只读 Editor，因此这里不再造第二套渲染器。
    pub(crate) fn paint_history(&mut self, frame: &mut Frame) {
        if !self.ui_state.history.open {
            return;
        }
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let l = self.history_layout();
        let panel = l.panel;
        let count = self.ui_state.history.entries.len();
        let rows_h = crate::view::history::rows_height(panel, scale);
        let max_scroll = crate::view::history::max_scroll(panel, count, scale);
        self.ui_state.history.scroll = self.ui_state.history.scroll.clamp(0.0, max_scroll);
        let x = panel.x.round() as i32;
        let y = panel.y.round() as i32;
        let w = panel.w.ceil() as u32;
        let h = panel.h.ceil() as u32;
        frame.fill_surface_card(
            x,
            y,
            w,
            h,
            scale,
            theme.dark,
            crate::tokens::palette::surface_popup(&theme),
            theme.chrome_border,
        );

        // 快照行：选中/悬停底色 + 左侧 accent 指示条 + 时间/大小 + 当前标记
        let top = crate::view::history::first_visible(self.ui_state.history.scroll, scale);
        let slots = crate::view::history::visible_slots(panel, scale);
        let current = self.ui_state.history.current_index();
        for slot in 0..slots {
            let index = top + slot;
            if index >= count {
                break;
            }
            let row =
                crate::view::history::row_rect(panel, index, self.ui_state.history.scroll, scale);
            let selected = self.ui_state.history.selected == Some(index);
            let hovered = self.ui_state.history.hover == Some(index);
            if selected || hovered {
                frame.fill_hover(
                    x,
                    row.y.round() as i32,
                    w,
                    row.h.ceil() as u32,
                    scale,
                    if selected { theme.sel } else { theme.hover },
                );
            }
            if selected {
                let bar_h = (row.h - 14.0 * scale).max(3.0);
                frame.fill_rounded_rect(
                    (panel.x + 4.0 * scale).round() as i32,
                    (row.y + (row.h - bar_h) / 2.0).round() as i32,
                    (3.0 * scale).max(1.0) as u32,
                    bar_h.ceil() as u32,
                    1.5 * scale,
                    [theme.accent[0], theme.accent[1], theme.accent[2], 255],
                );
            }
            let time = self
                .ui_state
                .history
                .labels
                .get(index)
                .cloned()
                .unwrap_or_else(|| crate::app::format_unix_ms_utc(0));
            let bytes = self.ui_state.history.entries[index].bytes;
            let text_w = (row.w - 8.0 * scale).max(0.0);
            let time_label = crate::view::history::fit_label(&time, text_w, scale);
            if let Some(label) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &time_label,
                theme.text,
                scale,
            ) {
                frame.blit(
                    &label.pixels,
                    label.w,
                    label.h,
                    (row.x + 4.0 * scale).round() as i32,
                    (row.y + 7.0 * scale).round() as i32,
                );
            }
            let mut meta = crate::view::history::format_size(bytes);
            if current == Some(index) {
                meta.push_str(t(Key::HistoryCurrentSuffix));
            }
            let meta_label = crate::view::history::fit_label(&meta, text_w, scale);
            let color = if current == Some(index) {
                theme.accent
            } else {
                theme.muted
            };
            if let Some(label) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &meta_label,
                color,
                scale,
            ) {
                let ly = row.y + row.h - label.h as f32 - 7.0 * scale;
                frame.blit(
                    &label.pixels,
                    label.w,
                    label.h,
                    (row.x + 4.0 * scale).round() as i32,
                    ly.round() as i32,
                );
            }
        }

        // 空态/错误态：走 B6 视觉 token（正文色 / danger + muted 辅助说明）
        if count == 0 {
            let (headline, hint, color) = match &self.ui_state.history.error {
                Some(e) => (
                    tf(Key::HistoryErrorHeadline, &[e]),
                    t(Key::HistoryErrorHint).to_string(),
                    theme.danger,
                ),
                None => (
                    t(crate::view::history::EMPTY_TEXT).to_string(),
                    t(crate::view::history::EMPTY_HINT).to_string(),
                    theme.text,
                ),
            };
            let inner_w = (panel.w - 2.0 * crate::view::history::PAD * scale).max(0.0);
            let body_top =
                crate::view::history::rows_top(panel, scale) + crate::view::history::PAD * scale;
            for (i, (text, color)) in [(headline, color), (hint, theme.muted)]
                .into_iter()
                .enumerate()
            {
                let label_text = crate::view::history::fit_label(&text, inner_w, scale);
                if let Some(label) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    &label_text,
                    color,
                    scale,
                ) {
                    frame.blit(
                        &label.pixels,
                        label.w,
                        label.h,
                        (panel.x + crate::view::history::PAD * scale).round() as i32,
                        (body_top + i as f32 * 22.0 * scale).round() as i32,
                    );
                }
            }
        }

        // 滚动指示（仅列表溢出时）
        if max_scroll > 0.0 && rows_h > 0.0 {
            let top_y = crate::view::history::rows_top(panel, scale);
            let track_h = rows_h - 4.0 * scale;
            let track_x = panel.x + panel.w - 6.0 * scale;
            let track_w = (3.0 * scale).max(1.0) as u32;
            let radius = 1.5 * scale;
            frame.fill_rounded_rect(
                track_x.round() as i32,
                top_y.round() as i32,
                track_w,
                track_h.ceil() as u32,
                radius,
                theme.chrome_border,
            );
            let content_h = count as f32 * crate::view::history::ROW_H * scale;
            let thumb_h = (track_h * track_h / content_h)
                .max(crate::theme::surface::scrollbar_min_thumb(scale))
                .min(track_h);
            let ty = top_y + (self.ui_state.history.scroll / max_scroll) * (track_h - thumb_h);
            frame.fill_rounded_rect(
                track_x.round() as i32,
                ty.round() as i32,
                track_w,
                thumb_h.ceil() as u32,
                radius,
                theme.thumb,
            );
        }

        // 标题区（覆盖上溢行）：不透明、底部分隔线与左右内边距对齐
        let head_h = (crate::view::history::HEAD_H * scale).ceil() as u32;
        let r = crate::theme::surface::corner_radius(scale);
        let popup_bg = crate::tokens::palette::surface_popup(&theme);
        frame.fill_rounded_rect(
            x + 1,
            y + 1,
            w.saturating_sub(2),
            head_h,
            (r - 1.0).max(0.0),
            popup_bg,
        );
        let sq = r.ceil().max(1.0) as i32;
        let hi = y + head_h as i32;
        frame.fill_rect(x + 1, hi - sq, w.saturating_sub(2), sq as u32, popup_bg);
        frame.fill_rect(
            (panel.x + crate::view::history::PAD * scale).round() as i32,
            hi - 1,
            (panel.w - 2.0 * crate::view::history::PAD * scale).max(1.0) as u32,
            1,
            theme.chrome_border,
        );
        if let Some(label) = self.ui_state.chrome.labels.get_or_insert(
            &mut self.engine,
            t(Key::HistoryTitle),
            theme.text,
            scale,
        ) {
            let ly = panel.y + (crate::view::history::HEAD_H * scale - label.h as f32) / 2.0;
            frame.blit(
                &label.pixels,
                label.w,
                label.h,
                (panel.x + crate::view::history::PAD * scale).round() as i32,
                ly.round() as i32,
            );
        }

        // 底部操作区（恢复/删除/清空 + 撤销提示）
        let btn_r = crate::theme::surface::corner_radius(scale).min(6.0 * scale);
        for (action, rect) in [
            (crate::view::history::HistoryAction::Restore, l.restore),
            (crate::view::history::HistoryAction::Delete, l.delete),
            (crate::view::history::HistoryAction::Clear, l.clear),
            (crate::view::history::HistoryAction::Close, l.close),
        ] {
            let hovered = self.ui_state.history.hover_action == Some(action);
            let (bg, fg): ([u8; 4], [u8; 3]) = match action {
                crate::view::history::HistoryAction::Restore => (
                    [theme.accent[0], theme.accent[1], theme.accent[2], 255],
                    theme.accent_text,
                ),
                crate::view::history::HistoryAction::Delete
                | crate::view::history::HistoryAction::Clear => {
                    (if hovered { theme.hover } else { popup_bg }, theme.danger)
                }
                crate::view::history::HistoryAction::Close => {
                    (if hovered { theme.hover } else { popup_bg }, theme.muted)
                }
            };
            if hovered || action == crate::view::history::HistoryAction::Restore {
                frame.fill_rounded_rect(
                    rect.x.round() as i32,
                    rect.y.round() as i32,
                    rect.w.ceil() as u32,
                    rect.h.ceil() as u32,
                    btn_r,
                    bg,
                );
            }
            if let Some(label) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                t(action.label()),
                fg,
                scale,
            ) {
                let lx = rect.x + (rect.w - label.w as f32) / 2.0;
                let ly = rect.y + (rect.h - label.h as f32) / 2.0;
                frame.blit(
                    &label.pixels,
                    label.w,
                    label.h,
                    lx.round() as i32,
                    ly.round() as i32,
                );
            }
        }
        if let Some(label) = self.ui_state.chrome.labels.get_or_insert(
            &mut self.engine,
            t(Key::HistoryUndoHint),
            theme.muted,
            scale,
        ) {
            let max_w = (panel.w - 2.0 * crate::view::history::PAD * scale).max(0.0);
            if label.w as f32 <= max_w {
                frame.blit(
                    &label.pixels,
                    label.w,
                    label.h,
                    (panel.x + crate::view::history::PAD * scale).round() as i32,
                    l.footer_note_y.round() as i32,
                );
            }
        }
    }

    /// 查找命中高亮（P3-3）：全部命中用 hover 色、当前命中用选区色，画在字形之下。
    /// 与选区同层语义：仅覆盖按文本布局绘制的内容块（表格网格/公式图 v0 简化）。
    pub(crate) fn paint_match_highlights(
        &self,
        frame: &mut Frame,
        l: &BlockLayout,
        range: &Range,
        sy: i32,
    ) {
        if !self.ui_state.search.open || self.ui_state.search.matches.is_empty() {
            return;
        }
        let x0 = self.content_x();
        // 命中按起点升序：二分定位首个相交者，向后多取一个以覆盖跨块命中
        let from = self
            .ui_state
            .search
            .matches
            .partition_point(|m| m.start < range.start)
            .saturating_sub(1);
        for (i, m) in self.ui_state.search.matches.iter().enumerate().skip(from) {
            if m.start >= range.end {
                break;
            }
            let (s, e) = (m.start.max(range.start), m.end.min(range.end));
            if s >= e {
                continue;
            }
            let color = if i == self.ui_state.search.current {
                self.input_state.theme.sel
            } else {
                self.input_state.theme.hover
            };
            for (rx, ry, rw, rh) in self
                .engine
                .selection_rects(l, s - range.start, e - range.start)
            {
                frame.fill_rect(
                    (x0 + rx) as i32,
                    sy + ry as i32,
                    rw as u32,
                    rh as u32,
                    color,
                );
            }
        }
    }

    /// 快速打开浮层（阶段 1）：面板边框 → 输入行 → 候选行（选中/悬停底色、
    /// 文件名与来源提示）。面板不透明覆盖内容与查找栏；标签复用 chrome 位图缓存。
    pub(crate) fn paint_quick(&mut self, frame: &mut Frame) {
        let Some(l) = self.quick_rect() else {
            return;
        };
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let pad = quick_open::PAD * scale;
        let px = l.panel.x.round() as i32;
        let py = l.panel.y.round() as i32;
        let pw = l.panel.w.ceil() as u32;
        let ph = l.panel.h.ceil() as u32;
        frame.fill_surface_card(
            px,
            py,
            pw,
            ph,
            scale,
            theme.dark,
            crate::tokens::palette::surface_popup(&theme),
            theme.chrome_border,
        );
        // 输入行与候选行的分隔线：左右内缩与输入区文字对齐（不顶到卡片边）
        frame.fill_rect(
            (px as f32 + pad).round() as i32,
            (l.input.y + l.input.h).round() as i32,
            (pw as f32 - 2.0 * pad).max(1.0) as u32,
            1,
            theme.chrome_border,
        );

        let query = self.ui_state.quick.query.clone();
        let top = self.ui_state.quick.top;
        let selected = self.ui_state.quick.selected;
        let hover = self.ui_state.quick.hover;
        let empty = self.ui_state.quick.filtered.is_empty();
        let visible: Vec<(usize, String, String, bool)> = self
            .ui_state
            .quick
            .visible()
            .map(|slot| {
                self.ui_state
                    .quick
                    .filtered
                    .get(slot)
                    .and_then(|&i| self.ui_state.quick.items.get(i))
                    .map(|it| (slot, it.label.clone(), it.hint.clone(), it.recent))
                    .unwrap_or((slot, String::new(), String::new(), false))
            })
            .collect();

        // 输入行：占位提示 / 过滤串（保留尾部，正在输入的一端）+ IME 组合串
        let imid = l.input.y + l.input.h * 0.5;
        let input_y = |h: u32| (imid - h as f32 / 2.0).round() as i32;
        let mut qw = 0.0;
        if query.is_empty() {
            if self.editor.preedit().is_none() {
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    t(Key::QuickOpenPlaceholder),
                    theme.muted,
                    scale,
                ) {
                    qw = lab.w as f32;
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        (l.input.x + pad).round() as i32,
                        input_y(lab.h),
                    );
                }
            }
        } else {
            let shown = search::fit_query(&query, l.input.w - 2.0 * pad, scale);
            if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &shown,
                theme.text,
                scale,
            ) {
                qw = lab.w as f32;
                frame.blit(
                    &lab.pixels,
                    lab.w,
                    lab.h,
                    (l.input.x + pad).round() as i32,
                    input_y(lab.h),
                );
            }
        }
        if let Some(pre) = self.editor.preedit() {
            let text = pre.text.clone();
            let pl = self.engine.layout_ui(&text);
            let (pxs, w, h) = self.engine.rasterize(&pl, theme.fg);
            let x0 = (l.input.x + pad + qw)
                .min((l.input.x + l.input.w - pad - w as f32).max(l.input.x + pad));
            let y0 = imid - h as f32 / 2.0;
            frame.blit(&pxs, w, h, x0.round() as i32, y0.round() as i32);
            frame.fill_rect(
                x0.round() as i32,
                (y0 + h as f32) as i32 - 1,
                w,
                1,
                [theme.fg[0], theme.fg[1], theme.fg[2], 255],
            );
        }
        // 插入光标：无 IME preedit 时画在输入区左端/文本末端（占位提示不参与定位）
        if self.editor.preedit().is_none() {
            let cx = if query.is_empty() {
                l.input.x + pad
            } else {
                l.input.x + pad + qw
            };
            let cw = crate::theme::surface::caret_width(scale).round().max(1.0) as u32;
            let ch = crate::theme::surface::caret_height(scale);
            let cy = (imid - ch / 2.0).round() as i32;
            frame.fill_rect(
                cx.round() as i32,
                cy,
                cw,
                ch as u32,
                [theme.fg[0], theme.fg[1], theme.fg[2], 255],
            );
        }

        // 候选行：底色 → 来源提示（右对齐）→ 文件名（左对齐，避让提示后截断）
        // 选中行是键盘派生状态、始终满值；悬停行走过渡不透明度（S6）。
        let fade = self.ui_state.quick.fade.view(hover);
        for (slot, label, hint, recent) in visible {
            let Some(row) = l.rows.get(slot - top) else {
                continue;
            };
            if slot == selected {
                frame.fill_hover(
                    row.x.round() as i32,
                    row.y.round() as i32,
                    row.w.ceil() as u32,
                    row.h.ceil() as u32,
                    scale,
                    theme.hover,
                );
            } else if let Some(a) = fade.alpha_of(slot) {
                frame.fill_hover(
                    row.x.round() as i32,
                    row.y.round() as i32,
                    row.w.ceil() as u32,
                    row.h.ceil() as u32,
                    scale,
                    [theme.hover[0], theme.hover[1], theme.hover[2], a],
                );
            }
            if label.is_empty() {
                continue;
            }
            let mid = row.y + row.h / 2.0;
            let ty = |h: u32| (mid - h as f32 / 2.0).round() as i32;
            let hint = if recent {
                t(Key::QuickOpenRecent).to_string()
            } else {
                hint
            };
            let hint_shown = quick_open::fit_label(&hint, row.w * 0.45, scale);
            let hint_w = if hint_shown.is_empty() {
                0.0
            } else {
                quick_open::text_width(&hint_shown, scale)
            };
            if hint_w > 0.0 {
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    &hint_shown,
                    theme.muted,
                    scale,
                ) {
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        (row.x + row.w - pad - lab.w as f32).round() as i32,
                        ty(lab.h),
                    );
                }
            }
            let label_w =
                (row.w - 2.0 * pad - hint_w - if hint_w > 0.0 { pad } else { 0.0 }).max(0.0);
            let shown = quick_open::fit_label(&label, label_w, scale);
            if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                &shown,
                theme.text,
                scale,
            ) {
                frame.blit(
                    &lab.pixels,
                    lab.w,
                    lab.h,
                    (row.x + pad).round() as i32,
                    ty(lab.h),
                );
            }
        }
        // 空态提示：占首行，水平/垂直居中；文案区分「无候选」与「查询不命中」。
        if empty {
            if let Some(row) = l.rows.first() {
                let message = quick_open::empty_message(
                    &self.ui_state.quick.query,
                    self.ui_state.quick.items.is_empty(),
                );
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    t(message),
                    theme.muted,
                    scale,
                ) {
                    let x = row.x + (row.w - lab.w as f32) / 2.0;
                    let y = (row.y + row.h / 2.0 - lab.h as f32 / 2.0).round() as i32;
                    frame.blit(&lab.pixels, lab.w, lab.h, x.round() as i32, y);
                }
            }
        }
    }

    /// 链接悬停浮层（N5）：鼠标上方 24px 的单行 URL 卡片，超宽按视口截断。
    /// 纯视觉、不拦截鼠标事件；模态/右键菜单期间不绘制。
    pub(crate) fn paint_link_tooltip(&mut self, frame: &mut Frame) {
        if self.ui_state.modal.is_some() || self.ui_state.ctx.open {
            return;
        }
        let Some((url, ax, ay)) = self.ui_state.hover_link.clone() else {
            return;
        };
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let vp = self.viewport();
        let pad = crate::theme::surface::padding(scale).round() as u32;
        let (text, _) = truncate_link_tip(&url, link_tip_max_text_w(vp, scale), scale);
        let Some(lab) =
            self.ui_state
                .chrome
                .labels
                .get_or_insert(&mut self.engine, &text, theme.text, scale)
        else {
            return;
        };
        let w = lab.w + 2 * pad;
        let h = lab.h + 2 * pad;
        let r = link_tip_rect(w as f32, h as f32, ax, ay, vp, scale);
        let (rx, ry) = (r.x.round() as i32, r.y.round() as i32);
        frame.fill_surface_card(
            rx,
            ry,
            r.w.ceil() as u32,
            r.h.ceil() as u32,
            scale,
            theme.dark,
            crate::tokens::palette::surface_popup(&theme),
            theme.chrome_border,
        );
        frame.blit(&lab.pixels, lab.w, lab.h, rx + pad as i32, ry + pad as i32);
    }

    /// 右键上下文菜单（2f）：面板 → 悬停底色 → 项标签。瞬态浮层：
    /// 只读视觉，不触碰文档真值与缓存；标签位图复用 chrome 缓存。
    pub(crate) fn paint_ctx_menu(&mut self, frame: &mut Frame) {
        if !self.ui_state.ctx.open {
            return;
        }
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let items: Vec<(Key, ctx_menu::CtxAction)> = self.ui_state.ctx.items().to_vec();
        let vp = self.viewport();
        let l = ctx_menu::layout(
            &items,
            self.ui_state.ctx.x,
            self.ui_state.ctx.y,
            vp.x + vp.w,
            vp.y + vp.h,
            scale,
        );
        let px = l.x.round() as i32;
        let py = l.y.round() as i32;
        let pw = l.w.ceil() as u32;
        let ph = l.h.ceil() as u32;
        frame.fill_surface_card(
            px,
            py,
            pw,
            ph,
            scale,
            theme.dark,
            crate::tokens::palette::surface_popup(&theme),
            theme.chrome_border,
        );
        let hover = self.ui_state.ctx.hover;
        let fade = self.ui_state.ctx.fade.view(hover);
        // S3：只有卡片边界线；分组间距由布局提供，不再绘制分隔线。
        for (i, off) in &l.rows {
            let Some((label, _)) = items.get(*i) else {
                continue;
            };
            let iy = l.y + off;
            if let Some(a) = fade.alpha_of(*i) {
                let h = crate::tokens::state::hover_bg(&theme);
                frame.fill_hover(
                    px,
                    iy.round() as i32,
                    pw,
                    l.item_h.ceil() as u32,
                    scale,
                    [h[0], h[1], h[2], a],
                );
            }
            if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                &mut self.engine,
                t(*label),
                theme.text,
                scale,
            ) {
                let y = (iy + l.item_h / 2.0 - lab.h as f32 / 2.0).round() as i32;
                frame.blit(&lab.pixels, lab.w, lab.h, (l.x + l.pad).round() as i32, y);
            }
        }
    }

    /// 拖放落点提示（阶段 1）：内容区压暗 + 居中提示条。
    /// 图片与文档给不同文案；草稿拖图片明确说明「无法插入」而不是静默失败。
    /// 只读视觉：不触碰文档真值与缓存；悬停文件名来自 winit HoveredFile。
    pub(crate) fn paint_drop_hint(&mut self, frame: &mut Frame) {
        let Some(path) = self.input_state.drop_file.clone() else {
            return;
        };
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let vp = self.viewport();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let text = if crate::image::is_image_path(&path) {
            if self.doc.path.is_none() {
                tf(Key::DropImageNeedsDir, &[&name])
            } else {
                tf(Key::DropInsertImage, &[&name])
            }
        } else {
            tf(Key::DropOpenDocument, &[&name])
        };
        frame.fill_rect(
            vp.x.round() as i32,
            vp.y.round() as i32,
            vp.w.ceil() as u32,
            vp.h.ceil() as u32,
            {
                let bg = crate::tokens::palette::surface_base(&theme);
                [bg[0], bg[1], bg[2], DROP_DIM_ALPHA]
            },
        );
        let pad = (12.0 * scale).round() as u32;
        let Some(lab) =
            self.ui_state
                .chrome
                .labels
                .get_or_insert(&mut self.engine, &text, theme.text, scale)
        else {
            return;
        };
        let bw = lab.w + pad * 2;
        let bh = lab.h + pad * 2;
        let bx = (vp.x + (vp.w - bw as f32) / 2.0).round() as i32;
        let by = (vp.y + (vp.h - bh as f32) / 2.0).round() as i32;
        frame.fill_surface_card(
            bx,
            by,
            bw,
            bh,
            scale,
            theme.dark,
            crate::tokens::palette::surface_popup(&theme),
            theme.chrome_border,
        );
        frame.blit(
            &lab.pixels,
            lab.w,
            lab.h,
            bx + pad as i32,
            (by + pad as i32) + ((bh - pad * 2 - lab.h) / 2) as i32,
        );
    }

    pub(crate) fn paint_caret_and_preedit(&mut self, frame: &mut Frame) {
        let (cx, cy, ch) = self.caret_screen();
        let vp = self.viewport();
        if cy + ch <= vp.y || cy >= vp.y + vp.h {
            return; // 光标在视口外：不画（被 chrome 覆盖前已越界）
        }
        let fg = self.input_state.theme.fg;
        let caret_color = [fg[0], fg[1], fg[2], 255];
        if let Some(pre) = self.editor.preedit() {
            // preedit：排版小串 → 位图贴在光标处 + 下划线；光标移至组合串内光标位
            // IME 组合期间强制显示光标（tick 也会保持 cursor_visible = true 兜底）。
            let text = pre.text.clone();
            let l = self.engine.layout_preedit(&text);
            let (px, w, h) = self.engine.rasterize(&l, fg);
            frame.blit(&px, w, h, cx as i32, (cy + ch - h as f32).max(0.0) as i32);
            frame.fill_rect(
                cx as i32,
                (cy + ch) as i32 - 1,
                w,
                1,
                [fg[0], fg[1], fg[2], 255],
            );
            let inner = pre.cursor.map(|(s, _)| s).unwrap_or(0);
            let (ix, _, _) = self.engine.caret_xy(&l, inner);
            frame.fill_rect((cx + ix) as i32, cy as i32, 1, ch as u32, caret_color);
        } else if !self.cursor_visible {
            // 光标闪烁熄灭：跳过绘制（光标高/位置不变，仅视觉消失半周期）。
        } else if caret_visual(self.ui_state.focused) == CaretVisual::Solid {
            frame.fill_rect(cx as i32, cy as i32, 2, ch as u32, caret_color);
        } else {
            // 失焦：同位置同高（ch）的空心框——1 逻辑像素描边（按 scale 换算，至少
            // 1 物理像素），内部留空。仅视觉差异：命中区域/行高/排版均不变。
            let stroke = (self.input_state.scale.round() as i32).max(1);
            let w = (crate::theme::surface::caret_width(self.input_state.scale).round() as i32)
                .max(stroke * 3);
            let h = (ch as i32).max(stroke * 2 + 1);
            let (x, y) = (cx as i32, cy as i32);
            frame.fill_rect(x, y, w as u32, stroke as u32, caret_color);
            frame.fill_rect(x, y + h - stroke, w as u32, stroke as u32, caret_color);
            frame.fill_rect(x, y, stroke as u32, h as u32, caret_color);
            frame.fill_rect(x + w - stroke, y, stroke as u32, h as u32, caret_color);
        }
    }

    pub(crate) fn scroll_to_caret(&mut self) {
        let vp = self.viewport();
        let (_, cy, ch) = self.caret_screen();
        let max = chrome::max_scroll(vp, self.doc_height());
        let mut target = self.window_state.goal_y;
        if self.ui_state.writing.typewriter {
            // 打字机模式（阶段 1）：光标行固定在视口上部锚点，行中心对齐锚点；
            // 文档首尾处仍由下方钳位兜底（无法再居中时贴边）
            let anchor = vp.y + vp.h * TYPEWRITER_ANCHOR;
            target += cy + ch / 2.0 - anchor;
        } else if cy < vp.y {
            target += cy - vp.y - MARGIN * self.input_state.scale;
        } else if cy + ch > vp.y + vp.h {
            // B5：光标行下方至少保留 `BOTTOM_PAD_LINES` 行可点击空白，
            // 换行继续输入时不会贴着状态栏（文档已含同等底部留白）。
            let pad = (crate::tokens::doc::BOTTOM_PAD_LINES * self.engine.doc_line_height())
                .max(MARGIN)
                * self.input_state.scale;
            target += cy + ch - (vp.y + vp.h) + pad;
        }
        self.window_state.goal_y = target.clamp(0.0, max);
        if !self.window_state.smooth {
            self.window_state.scroll_y = self.window_state.goal_y;
        }
    }

    /// 打字机锚点定位（跳转路径）：先把候选滚动位置附近的块补排并重算几何，
    /// 迭代收敛后再落位。
    ///
    /// 大纲/搜索等远距离跳转前，目标块高度可能仍是估算值；若按估算 `top` 直接
    /// 算滚动量，首帧绘制补排后 `top` 会整体漂移，当前行就偏离锚点，要等下一次
    /// 输入调用 `scroll_to_caret` 才纠正（DEF-T11-E1-003）。这里与 `render`
    /// 使用同一套可见区间补排，先收敛几何再算滚动量，保证首帧即锚点。
    pub(crate) fn scroll_to_typewriter_anchor(&mut self) {
        let vp0 = self.viewport();
        let anchor = vp0.y + vp0.h * TYPEWRITER_ANCHOR;
        let mut target = self.window_state.scroll_y;
        for _ in 0..4 {
            self.window_state.scroll_y = target;
            self.ensure_layouts_visible();
            self.ensure_math_visible();
            self.ensure_tables_visible();
            if self.index.geometry_stale {
                self.recompute_geometry();
            }
            let vp = self.viewport();
            let max = chrome::max_scroll(vp, self.doc_height());
            let (_, cy, ch) = self.caret_screen();
            let next = (target + cy + ch / 2.0 - anchor).clamp(0.0, max);
            let converged = (next - target).abs() < 0.5;
            target = next;
            if converged {
                break;
            }
        }
        // 首帧即锚点：直接落位，不依赖平滑滚动动画或后续输入纠正。
        self.window_state.scroll_y = target;
        self.window_state.goal_y = target;
    }

    /// Ctrl+Enter：在 caret 所在表格末尾追加一个空 body 行。返回 true 表示已处理。
    pub(crate) fn table_ctrl_enter(&mut self) -> bool {
        let caret = self.editor.caret();
        let Some((_tid, _tbl)) = self.caret_table_and_cell(caret) else {
            return false;
        };
        let Some(block_range) = self
            .index
            .blocks
            .iter()
            .find(|g| g.kind == BlockKind::Table && g.range.start <= caret && caret <= g.range.end)
            .map(|g| g.range)
        else {
            return false;
        };
        let src = self
            .editor
            .session()
            .text_slice(block_range)
            .unwrap_or_default();
        let new_src = table_append_row(&src);
        if new_src == src {
            return false;
        }
        // 新行在块尾；光标落到新行首个单元格内容处（`|` 后跳过前导空白）。
        // 空单元格（`|  |`）若只按 `+2` 定位会落在管道符上，活动单元格判定失败、Tab 跳格错位。
        let last_line = new_src.trim_end_matches(['\n', '\r']).matches('\n').count();
        let caret_in_block = table_cell_start(&new_src, last_line, 0);
        let Some(dirty_blocks) =
            self.editor
                .replace_table_block(block_range, new_src, caret_in_block)
        else {
            return false;
        };
        self.after_edit(dirty_blocks);
        self.request_redraw();
        true
    }
}

/// 单元格 range 是否包含 caret（含 start/end，与 block range 判定同口径）。
pub(crate) fn cell_contains(r: &Range, caret: usize) -> bool {
    r.start <= caret && caret <= r.end
}

/// 按 flat 索引（header 先，rows 后）取第 idx 个单元格的 range.start。
fn flat_cell_start(tbl: &veil_core::Table, idx: usize) -> usize {
    if let Some(c) = tbl.header.get(idx) {
        return c.range.start;
    }
    let offset = tbl.header.len();
    let mut remaining = idx - offset;
    for row in &tbl.rows {
        if remaining < row.len() {
            return row[remaining].range.start;
        }
        remaining -= row.len();
    }
    0
}

/// caret 所在单元格的 flat 索引（header 先，rows 后）；不在任何单元格时 None。
fn flat_cell_range(tbl: &veil_core::Table, idx: usize) -> Option<Range> {
    if let Some(c) = tbl.header.get(idx) {
        return Some(c.range);
    }
    let mut remaining = idx.checked_sub(tbl.header.len())?;
    for row in &tbl.rows {
        if remaining < row.len() {
            return Some(row[remaining].range);
        }
        remaining -= row.len();
    }
    None
}

/// 含脚注引用的单元格 → 源码原文（flat 索引 → 文本）。
/// 单元格内的 `[^id]` 按纯文本降级（不编号不着色）；core 的可见文本已剥标记，
/// 因此这里回取源码，保持「原文可见」。空表/无引用时返回空表，调用方零成本。
fn footnote_cell_sources(
    session: &veil_core::Session,
    tbl: &veil_core::Table,
) -> HashMap<usize, String> {
    fn has_ref(nodes: &[InlineNode]) -> bool {
        nodes
            .iter()
            .any(|n| n.kind == InlineKind::FootnoteRef || has_ref(&n.children))
    }
    let mut out = HashMap::new();
    let cols = tbl.aligns.len();
    let mut idx = 0usize;
    for cells in std::iter::once(&tbl.header).chain(tbl.rows.iter()) {
        // 与表格排版的 flat 索引口径一致：按列数递增（缺格也占位）。
        for col in 0..cols {
            if let Some(cell) = cells.get(col) {
                if has_ref(&cell.inlines) {
                    if let Some(src) = session.text_slice(cell.range) {
                        out.insert(idx, src);
                    }
                }
            }
            idx += 1;
        }
    }
    out
}

/// 单元格内联公式种子：剥标记后可见文本区间 + TeX 源码（渲染前的纯数据）。
struct CellMathSeed {
    range: Range,
    tex: String,
}

/// 收集非活动单元格的 Math 节点：可见文本坐标（与 `cell.text` 同口径）+ TeX。
/// 活动格显源码，不参与；无 Math 或映射不可用（字节分歧）时该格不入表。
fn table_cell_math_seeds(
    session: &veil_core::Session,
    tbl: &veil_core::Table,
    active: u32,
) -> HashMap<usize, Vec<CellMathSeed>> {
    fn has_math(nodes: &[InlineNode]) -> bool {
        nodes
            .iter()
            .any(|n| n.kind == InlineKind::Math || has_math(&n.children))
    }
    let mut out = HashMap::new();
    let cols = tbl.aligns.len();
    let mut idx = 0usize;
    for cells in std::iter::once(&tbl.header).chain(tbl.rows.iter()) {
        for col in 0..cols {
            if let Some(cell) = cells.get(col) {
                if active as usize != idx && has_math(&cell.inlines) {
                    if let Some(spans) = cell_math_spans(cell.range, cell.text.len(), &cell.inlines)
                    {
                        let seeds: Vec<CellMathSeed> = spans
                            .into_iter()
                            .filter_map(|(s, e, content)| {
                                let tex = session.text_slice(content)?;
                                Some(CellMathSeed {
                                    range: Range::new(s, e),
                                    tex,
                                })
                            })
                            .collect();
                        if !seeds.is_empty() {
                            out.insert(idx, seeds);
                        }
                    }
                }
            }
            idx += 1;
        }
    }
    out
}

/// 收集非活动单元格的图片 URL（递归扫描 inlines，活动格跳过）。
fn table_cell_img_seeds(
    session: &veil_core::Session,
    tbl: &veil_core::Table,
    active: u32,
) -> HashMap<usize, Vec<String>> {
    fn collect_urls(nodes: &[InlineNode], out: &mut Vec<String>, session: &veil_core::Session) {
        for n in nodes {
            match &n.kind {
                InlineKind::Image => {
                    if let Some(url_range) = n.url {
                        if let Some(url) = session.text_slice(url_range) {
                            out.push(url);
                        }
                    }
                }
                // 单元格内 HTML `<img>`：本地 src 与 Markdown Image 同路径；
                // 远程地址在 `remote_src`（src 为空），走同一套远程加载。
                InlineKind::HtmlSelfClosed {
                    tag: HtmlTag::Img,
                    attrs,
                } => {
                    if let Some(src) = attrs.src.as_deref().or(attrs.remote_src.as_deref()) {
                        out.push(src.to_string());
                    }
                }
                _ => {}
            }
            collect_urls(&n.children, out, session);
        }
    }
    let mut out: HashMap<usize, Vec<String>> = HashMap::new();
    let cols = tbl.aligns.len();
    let mut idx = 0usize;
    for cells in std::iter::once(&tbl.header).chain(tbl.rows.iter()) {
        for col in 0..cols {
            if let Some(cell) = cells.get(col) {
                if active as usize != idx {
                    let mut urls = Vec::new();
                    collect_urls(&cell.inlines, &mut urls, session);
                    if !urls.is_empty() {
                        out.insert(idx, urls);
                    }
                }
            }
            idx += 1;
        }
    }
    out
}

/// caret 所在单元格的 flat 索引（header 先，rows 后）；不在任何单元格时 None。
pub(crate) fn active_cell_index(tbl: &veil_core::Table, caret: usize) -> Option<usize> {
    let mut idx = 0usize;
    for c in tbl.header.iter() {
        if cell_contains(&c.range, caret) {
            return Some(idx);
        }
        idx += 1;
    }
    for row in tbl.rows.iter() {
        for c in row.iter() {
            if cell_contains(&c.range, caret) {
                return Some(idx);
            }
            idx += 1;
        }
    }
    None
}

/// caret 之后（不含所在格）第一个单元格的 flat 索引：结构字符上的 Tab 落点。
fn first_cell_after(tbl: &veil_core::Table, caret: usize) -> Option<usize> {
    let total = tbl.header.len() + tbl.rows.iter().map(|r| r.len()).sum::<usize>();
    (0..total).find(|&i| flat_cell_start(tbl, i) > caret)
}

// ---- GFM 表格行/列 Markdown 变换（T3：纯字符串，零核心依赖） ----
#[allow(dead_code)]
fn split_row(row: &str) -> Vec<String> {
    // 去掉首尾空白和首/尾管道符
    let trimmed = row.trim();
    let trimmed = trimmed.trim_start_matches('|').trim_end_matches('|');
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed.split('|').map(|s| s.trim().to_string()).collect()
}

#[allow(dead_code)]
fn join_row(cells: &[String]) -> String {
    let parts: Vec<String> = cells.iter().map(|s| format!(" {} ", s)).collect();
    format!("|{}|", parts.join("|"))
}

#[allow(dead_code)]
fn sep_cell_for(align: veil_core::CellAlign) -> String {
    match align {
        veil_core::CellAlign::None => "---".into(),
        veil_core::CellAlign::Left => ":--".into(),
        veil_core::CellAlign::Center => ":-:".into(),
        veil_core::CellAlign::Right => "--:".into(),
    }
}

#[allow(dead_code)]
fn parse_aligns(sep_line: &str) -> Vec<veil_core::CellAlign> {
    let cells = split_row(sep_line);
    let mut out = Vec::with_capacity(cells.len());
    for c in cells {
        let l = c.starts_with(':');
        let r = c.ends_with(':');
        out.push(match (l, r) {
            (true, true) => veil_core::CellAlign::Center,
            (true, false) => veil_core::CellAlign::Left,
            (false, true) => veil_core::CellAlign::Right,
            (false, false) => veil_core::CellAlign::None,
        });
    }
    out
}

pub(crate) fn table_append_row(src: &str) -> String {
    let lines: Vec<&str> = src.trim_end().split('\n').collect();
    if lines.len() < 3 {
        return src.to_string();
    }
    let ncols = split_row(lines[0]).len();
    let empty = vec!["".to_string(); ncols];
    let empty_row = join_row(&empty);
    let mut new_lines = lines.clone();
    new_lines.push(empty_row.as_str());
    format!("{}\n", new_lines.join("\n"))
}

/// 在 body 行前/后插入一个空行（body_index 0-based；越界按尾部处理）。
/// 列数取自表头，分隔行原样保留，故列数与对齐方式不变。
pub(crate) fn table_insert_row(src: &str, body_index: usize, above: bool) -> String {
    let lines: Vec<&str> = src.trim_end().split('\n').collect();
    if lines.len() < 2 {
        return src.to_string();
    }
    let ncols = split_row(lines[0]).len().max(1);
    let empty = join_row(&vec![String::new(); ncols]);
    let body_count = lines.len().saturating_sub(2);
    let at = if body_index >= body_count {
        lines.len()
    } else if above {
        2 + body_index
    } else {
        3 + body_index
    };
    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len() + 1);
    new_lines.extend_from_slice(&lines[..at]);
    new_lines.push(&empty);
    new_lines.extend_from_slice(&lines[at..]);
    format!("{}\n", new_lines.join("\n"))
}

/// 生成源码中第 line_index 行（0=header、1=分隔行、2+=body）第 col 列单元格
/// 内容起点（`|` 后跳过前导空白）；行/列越界返回 0。
pub(crate) fn table_cell_start(src: &str, line_index: usize, col: usize) -> usize {
    let lines: Vec<&str> = src.split('\n').collect();
    let Some(line) = lines.get(line_index) else {
        return 0;
    };
    let line_off: usize = lines[..line_index].iter().map(|l| l.len() + 1).sum();
    let mut i = line.len() - line.trim_start().len();
    if line[i..].starts_with('|') {
        i += 1;
    }
    let mut cur = 0usize;
    loop {
        while i < line.len() && line.as_bytes()[i] == b' ' {
            i += 1;
        }
        if cur == col {
            return line_off + i;
        }
        match line[i..].find('|') {
            Some(k) => {
                i += k + 1;
                cur += 1;
            }
            None => return 0,
        }
    }
}

/// 删除指定 body 行（body_index 0-based；0 = 第一行 body，不是 header）。
/// 保留表格至少 1 行 body（只有 header + 分隔行时退化为仅 header 行）。
pub(crate) fn table_delete_row(src: &str, body_index: usize) -> String {
    let lines: Vec<&str> = src.trim_end().split('\n').collect();
    if lines.len() < 3 {
        return src.to_string();
    }
    // body 行在 lines[2..]
    if body_index + 2 >= lines.len() {
        return src.to_string();
    }
    if lines.len() <= 3 {
        // 只剩 header + 分隔行 + 1 body → 删除 body，留 header + 分隔行
        let keep: Vec<&str> = lines[..2].to_vec();
        return format!("{}\n", keep.join("\n"));
    }
    let mut new_lines: Vec<&str> = lines.clone();
    new_lines.remove(body_index + 2);
    format!("{}\n", new_lines.join("\n"))
}

/// 在指定列 index 前插入空列（col_index 0-based；包含 header 和 separator）。
pub(crate) fn table_insert_column(src: &str, col_index: usize) -> String {
    let lines: Vec<&str> = src.trim_end().split('\n').collect();
    if lines.len() < 3 {
        return src.to_string();
    }
    let ncols = split_row(lines[0]).len();
    let col_index = col_index.min(ncols);
    let mut new_lines = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let cells = split_row(line);
        let mut new_cells = cells.clone();
        // 处理列数不足的行（表格结构异常时）
        if new_cells.len() < ncols {
            new_cells.resize(ncols, String::new());
        }
        let insert = if i == 1 {
            // 分隔行：生成 None 对齐
            sep_cell_for(veil_core::CellAlign::None)
        } else {
            String::new()
        };
        new_cells.insert(col_index, insert);
        new_lines.push(join_row(&new_cells));
    }
    format!("{}\n", new_lines.join("\n"))
}

/// 删除指定列（col_index 0-based）。
/// 保留至少 1 列（只有 header 时删除后仍有 1 列）。
pub(crate) fn table_delete_column(src: &str, col_index: usize) -> String {
    let lines: Vec<&str> = src.trim_end().split('\n').collect();
    if lines.len() < 3 {
        return src.to_string();
    }
    let ncols = split_row(lines[0]).len();
    if ncols <= 1 || col_index > ncols {
        return src.to_string();
    }
    let mut new_lines = Vec::with_capacity(lines.len());
    for line in lines.iter() {
        let cells = split_row(line);
        let mut new_cells = cells.clone();
        if new_cells.len() > col_index {
            new_cells.remove(col_index);
        }
        if new_cells.is_empty() {
            new_cells.push(String::new());
        }
        new_lines.push(join_row(&new_cells));
    }
    format!("{}\n", new_lines.join("\n"))
}

/// 分隔行单元格（含两侧空白）的字节区间：剥除行首尾空白与可选外层竖线后按 `|` 切分。
fn sep_cell_spans(line: &str) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let (mut s, mut e) = (0, b.len());
    while s < e && b[s].is_ascii_whitespace() {
        s += 1;
    }
    while e > s && b[e - 1].is_ascii_whitespace() {
        e -= 1;
    }
    if s < e && b[s] == b'|' {
        s += 1;
    }
    if e > s && b[e - 1] == b'|' {
        e -= 1;
    }
    let mut out = Vec::new();
    let mut seg = s;
    let mut i = s;
    while i < e {
        if b[i] == b'|' {
            out.push((seg, i));
            i += 1;
            seg = i;
        } else {
            i += 1;
        }
    }
    out.push((seg, e));
    out
}

/// 改写分隔行第 `col` 列的对齐标记（左/中/右/无），其余字节逐字保留，返回新源码。
/// 标记宽度沿用该列原有分隔符宽度（不足 3 补到 3），避免列宽跳动；
/// `col` 越界或第 2 行不是合法分隔格时返回 None。已是目标对齐时输出与输入相等。
pub(crate) fn table_set_align(
    src: &str,
    col: usize,
    align: veil_core::CellAlign,
) -> Option<String> {
    // 第 2 行（分隔行）的字节区间；`\r` 归入行尾不参与改写。
    let mut line_start = 0usize;
    let mut sep = None;
    for (i, line) in src.split_inclusive('\n').enumerate() {
        if i == 1 {
            sep = Some((
                line_start,
                line_start + line.trim_end_matches(['\n', '\r']).len(),
            ));
            break;
        }
        line_start += line.len();
    }
    let (ls, le) = sep?;
    let line = &src[ls..le];
    let (cs, ce) = *sep_cell_spans(line).get(col)?;
    let content = &line[cs..ce];
    let trimmed = content.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|b| b == b'-' || b == b':') {
        return None;
    }
    let start = cs + (content.len() - content.trim_start().len());
    let width = trimmed.len().max(3);
    let marker = match align {
        veil_core::CellAlign::None => "-".repeat(width),
        veil_core::CellAlign::Left => format!(":{}", "-".repeat(width - 1)),
        veil_core::CellAlign::Center => format!(":{}:", "-".repeat(width - 2)),
        veil_core::CellAlign::Right => format!("{}:", "-".repeat(width - 1)),
    };
    let mut out = String::with_capacity(src.len() + 2);
    out.push_str(&src[..ls + start]);
    out.push_str(&marker);
    out.push_str(&src[ls + start + trimmed.len()..]);
    Some(out)
}

/// 是否为可加载的远程图片地址：trim 后大小写不敏感匹配 `http://` / `https://`。
/// 与 `local_image_path` 的 scheme 判定互补：凡带其它 scheme（data:/file:）都不远程加载。
pub(crate) fn is_remote_image_url(url: &str) -> bool {
    let url = url.trim();
    let b = url.as_bytes();
    (b.len() >= 8 && b[..8].eq_ignore_ascii_case(b"https://"))
        || (b.len() >= 7 && b[..7].eq_ignore_ascii_case(b"http://"))
}

/// 图片 URL → 本地路径（切片 13）。相对 URL 相对文档所在目录；草稿（无路径）的
/// 相对 URL 无法解析；`http:`/`https:`/`data:`/`file:` 等带 scheme 的 URL v0 不加载。
/// Windows 盘符（`C:`）只 1 个字母，不误判为 scheme。
pub(crate) fn local_image_path(url: &str, doc: Option<&Path>) -> Option<PathBuf> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if let Some((scheme, _)) = url.split_once(':') {
        if scheme.len() >= 2 && scheme.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
    }
    // 百分号编码的中文/空格路径（`%E4%B8%AD` / `%20`）按 UTF-8 解码后再解析；
    // 与 HTML 导出的资产路径口径一致（veil_core::percent_decode_path）。
    let decoded = veil_core::percent_decode_path(url);
    let p = Path::new(&decoded);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    Some(doc?.parent()?.join(p))
}

/// 图片显示尺寸（仅宽度上限；测试与历史调用口径）。生产路径统一走
/// `view::fit_size_box`（同时限宽限高，B4）。
#[cfg(test)]
pub(crate) fn fit_size(w: u32, h: u32, max_width: f32) -> (u32, u32) {
    crate::view::fit_size_box(w, h, max_width, f32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "| a | b |\n| --- | --- |\n| c | d |\n";
    const TABLE_2X2_ALIGNED: &str = "| a | b |\n| :-- | --: |\n| c | d |\n| e | f |\n";

    #[test]
    fn caret_visual_marks_focus_state() {
        assert_eq!(caret_visual(true), CaretVisual::Solid, "聚焦：实心插入点");
        assert_eq!(caret_visual(false), CaretVisual::Hollow, "失焦：空心框");
    }

    #[test]
    fn ui_state_focus_defaults_true_and_flips() {
        let mut ui = crate::app::UIState::default();
        assert!(ui.focused, "启动默认聚焦");
        ui.focused = false;
        assert!(!ui.focused, "失焦写入 false");
        ui.focused = true;
        assert!(ui.focused, "重新聚焦写回 true");
    }

    #[test]
    fn append_row_adds_empty_row_with_same_columns() {
        assert_eq!(table_append_row(TABLE), format!("{TABLE}|  |  |\n"));
        // 不足 header + 分隔行时不改动
        assert_eq!(table_append_row("| a |\n"), "| a |\n");
    }

    #[test]
    fn insert_row_above_and_below_keep_alignment() {
        assert_eq!(
            table_insert_row(TABLE_2X2_ALIGNED, 1, true),
            "| a | b |\n| :-- | --: |\n| c | d |\n|  |  |\n| e | f |\n"
        );
        assert_eq!(
            table_insert_row(TABLE_2X2_ALIGNED, 1, false),
            "| a | b |\n| :-- | --: |\n| c | d |\n| e | f |\n|  |  |\n"
        );
        // 分隔行与列数不变
        assert_eq!(
            table_insert_row(TABLE_2X2_ALIGNED, 0, true).lines().nth(1),
            Some("| :-- | --: |")
        );
    }

    #[test]
    fn insert_row_out_of_range_appends_to_tail() {
        assert_eq!(
            table_insert_row(TABLE_2X2_ALIGNED, 99, true),
            format!("{TABLE_2X2_ALIGNED}|  |  |\n")
        );
        assert_eq!(
            table_insert_row(TABLE_2X2_ALIGNED, 99, false),
            format!("{TABLE_2X2_ALIGNED}|  |  |\n")
        );
    }

    #[test]
    fn delete_row_and_last_body_row() {
        assert_eq!(
            table_delete_row(TABLE_2X2_ALIGNED, 0),
            "| a | b |\n| :-- | --: |\n| e | f |\n"
        );
        // 仅剩 1 行 body → 删成只有 header + 分隔行
        assert_eq!(table_delete_row(TABLE, 0), "| a | b |\n| --- | --- |\n");
        // 越界不改动
        assert_eq!(table_delete_row(TABLE, 9), TABLE);
    }

    #[test]
    fn insert_column_keeps_aligns_and_column_count() {
        let out = table_insert_column(TABLE_2X2_ALIGNED, 1);
        assert_eq!(
            out,
            "| a |  | b |\n| :-- | --- | --: |\n| c |  | d |\n| e |  | f |\n"
        );
        assert_eq!(
            parse_aligns(out.lines().nth(1).unwrap()),
            vec![
                veil_core::CellAlign::Left,
                veil_core::CellAlign::None,
                veil_core::CellAlign::Right
            ],
            "插入列不得改动既有列对齐"
        );
        // 追加到末尾
        assert_eq!(
            table_insert_column(TABLE, 9),
            "| a | b |  |\n| --- | --- | --- |\n| c | d |  |\n"
        );
    }

    #[test]
    fn delete_column_and_single_column_boundary() {
        assert_eq!(
            table_delete_column(TABLE_2X2_ALIGNED, 0),
            "| b |\n| --: |\n| d |\n| f |\n"
        );
        assert_eq!(
            table_delete_column(TABLE_2X2_ALIGNED, 1),
            "| a |\n| :-- |\n| c |\n| e |\n"
        );
        // 单列不删除
        assert_eq!(
            table_delete_column("| a |\n| --- |\n| b |\n", 0),
            "| a |\n| --- |\n| b |\n"
        );
    }

    #[test]
    fn set_align_round_trips_and_leaves_other_bytes_untouched() {
        let src = "| a | b |\n| --- | --- |\n| c | d |\n";
        let left = table_set_align(src, 1, veil_core::CellAlign::Left).unwrap();
        assert_eq!(left, "| a | b |\n| --- | :-- |\n| c | d |\n");
        let center = table_set_align(&left, 1, veil_core::CellAlign::Center).unwrap();
        assert_eq!(center, "| a | b |\n| --- | :-: |\n| c | d |\n");
        let right = table_set_align(&center, 1, veil_core::CellAlign::Right).unwrap();
        assert_eq!(right, "| a | b |\n| --- | --: |\n| c | d |\n");
        assert_eq!(
            table_set_align(&right, 1, veil_core::CellAlign::None).unwrap(),
            src,
            "回到无对齐时与原文逐字节一致"
        );
        // 幂等：同值再设一次输出相等
        assert_eq!(
            table_set_align(&right, 1, veil_core::CellAlign::Right),
            Some(right)
        );
    }

    #[test]
    fn set_align_keeps_crlf_and_wide_cell_padding() {
        let src = "| a | b |\r\n| :---: | ---- |\r\n| c | d |\r\n";
        let out = table_set_align(src, 1, veil_core::CellAlign::Center).unwrap();
        assert_eq!(out, "| a | b |\r\n| :---: | :--: |\r\n| c | d |\r\n");
        // 未选中列的 `:---:` 与该列宽度原样保留（总字节长度不变）
        assert_eq!(out.len(), src.len());
        assert_eq!(
            table_set_align(&out, 1, veil_core::CellAlign::None).unwrap(),
            src
        );
        // 窄于 3 的分隔格补齐到 3
        assert_eq!(
            table_set_align("| a |\n| - |\n| b |\n", 0, veil_core::CellAlign::Center).unwrap(),
            "| a |\n| :-: |\n| b |\n"
        );
        // 无外层竖线的单列表格
        assert_eq!(
            table_set_align("a\n---\nb\n", 0, veil_core::CellAlign::Right).unwrap(),
            "a\n--:\nb\n"
        );
    }

    #[test]
    fn set_align_out_of_range_and_non_delimiter_rows() {
        // 单列表格
        assert_eq!(
            table_set_align("| a |\n| --- |\n| b |\n", 0, veil_core::CellAlign::Left).unwrap(),
            "| a |\n| :-- |\n| b |\n"
        );
        // col 越界 / 无分隔行 / 第 2 行不是分隔格
        assert_eq!(table_set_align(TABLE, 2, veil_core::CellAlign::Left), None);
        assert_eq!(
            table_set_align("| a |\n", 0, veil_core::CellAlign::Left),
            None
        );
        assert_eq!(
            table_set_align("| a |\n| x |\n| b |\n", 0, veil_core::CellAlign::Left),
            None
        );
    }

    #[test]
    fn cell_start_skips_pipe_and_spaces() {
        assert_eq!(table_cell_start(TABLE, 0, 0), 2);
        assert_eq!(table_cell_start(TABLE, 0, 1), 6);
        assert_eq!(table_cell_start(TABLE, 1, 0), 12);
        assert_eq!(table_cell_start(TABLE, 2, 0), 26);
        assert_eq!(table_cell_start(TABLE, 2, 1), 30);
        // 无首管道符的行
        assert_eq!(table_cell_start("a | b\n--- | ---\n", 0, 0), 0);
        assert_eq!(table_cell_start("a | b\n--- | ---\n", 0, 1), 4);
        // 越界
        assert_eq!(table_cell_start(TABLE, 9, 0), 0);
        assert_eq!(table_cell_start(TABLE, 0, 9), 0);
    }

    #[test]
    fn cell_start_of_inserted_empty_row_matches_caret_convention() {
        let src = format!("{TABLE}|  |  |\n");
        // 空单元格内容起点 = 闭合管道符位置（与原 Ctrl+Enter 落点一致）
        assert_eq!(table_cell_start(&src, 3, 0), 37);
        assert_eq!(table_cell_start(&src, 3, 1), 40);
    }
}
