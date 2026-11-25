//! 帧合成：背景/块内容/选区/光标与 preedit 绘制。所有输入为
//! 已排版的几何与缓存，本文件不触发重新排版（除 paint 开头的两遍收敛）。
//! 查找栏与表格网格的绘制片段在 `view::search_view` / `view::table_view`。

use crate::app::{App, WindowState};
use crate::chrome;
use crate::document::block_index::BlockGeom;
use crate::geometry::Rect;
use crate::i18n::{t, Key};
use crate::view::{
    block_details_spans, image_is_standalone, node_active, oversize_notice, Frame, ImageCaption,
    InlineAsset, InlineFold, InlineImg, LayoutEngine, LayoutKey, Memo, OverKind, UnderKind,
};
use softbuffer::Surface;
use std::collections::HashSet;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use veil_core::{BlockId, BlockKind, HtmlTag, InlineKind, InlineNode, Range};
use winit::window::Window;

pub(crate) use crate::paint_table::{
    active_cell_index, table_cell_start, table_delete_column, table_delete_row,
    table_insert_column, table_insert_row, table_set_align,
};
#[cfg(test)]
pub(crate) use crate::paint_table::{
    join_row, parse_aligns, split_row, table_append_row, table_cell_img_seeds,
};

/// 专注模式压暗强度：背景色以该 alpha 覆盖非光标块（越大越暗，255 = 全遮）。
const FOCUS_DIM_ALPHA: u8 = 160;

/// 待排版图片：(节点区间, url, 显式宽高, 对齐)。显式宽高仅来自 HTML
/// `<img width height>`，对齐仅来自 `<img align>`；Markdown 图片两者皆无。
struct PendingImage {
    range: Range,
    url: String,
    explicit: (Option<u32>, Option<u32>),
    align: Option<veil_core::ImageAlign>,
}

/// 拖放悬停压暗强度（阶段 1）：内容区背景覆盖，突出居中提示条。
pub(crate) const DROP_DIM_ALPHA: u8 = 120;

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

/// present 前置条件：帧像素数与 surface 缓冲长度一致才可整块拷贝。
/// 最小化时 Resized 宽/高为 0，surface 不 resize 而帧被钳成 1×1，长度失配即丢帧。
pub(crate) fn can_present(frame_len: usize, target_len: usize) -> bool {
    frame_len == target_len
}

/// 呈现面写入端口：把 softbuffer 的 `resize` / `buffer_mut` 两步显式建模，
/// 使「先 resize 再 buffer_mut」的顺序可在无真实窗口时用假实现验证。
/// 实现方必须保证 `present_pixels` 只会在成功 `resize` 之后被调用。
pub(crate) trait PresentSink {
    /// 设置 surface 缓冲尺寸；失败返回 false（调用方放弃本帧，不得再取 buffer）。
    fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) -> bool;
    /// 取 buffer、校验长度、拷贝并 present。softbuffer 未 resize 时取 buffer 会 panic，
    /// 因此调用方必须先通过 `resize` 完成尺寸同步。
    fn present_pixels(&mut self, pixels: &[u32]) -> bool;
}

/// 真实 softbuffer 呈现面适配器。
pub(crate) struct SoftSurface<'a>(pub(crate) &'a mut Surface<Arc<Window>, Arc<Window>>);

impl PresentSink for SoftSurface<'_> {
    fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) -> bool {
        self.0.resize(width, height).is_ok()
    }

    fn present_pixels(&mut self, pixels: &[u32]) -> bool {
        let Ok(mut target) = self.0.buffer_mut() else {
            return false;
        };
        if !can_present(pixels.len(), target.len()) {
            return false;
        }
        target.copy_from_slice(pixels);
        target.present().is_ok()
    }
}

/// 一帧的呈现前置决策（纯函数）：只在帧尺寸与客户区一致时允许触碰 surface。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceStep {
    /// 帧尺寸为 0，或帧尺寸与当前客户区不一致（窗口已变化）：本帧不呈现。
    Skip,
    /// surface 尚未设置尺寸或尺寸与帧不一致：必须先 resize 再取 buffer。
    Resize { width: u32, height: u32 },
    /// surface 尺寸已与帧一致：可直接取 buffer 呈现。
    Draw,
}

/// 决定 present 如何触碰 surface。顺序不可交换：
/// 0×0 防护 → 帧/客户区一致性（stale 检查）→ surface 尺寸同步。
/// 客户区为 None（headless）时不阻止；真实窗口的 0×0 客户区视为不可呈现。
pub(crate) fn surface_step(
    frame: (u32, u32),
    client: Option<(u32, u32)>,
    surface_size: Option<(u32, u32)>,
) -> SurfaceStep {
    if frame.0 == 0 || frame.1 == 0 {
        return SurfaceStep::Skip;
    }
    if let Some(client) = client {
        if client.0 == 0 || client.1 == 0 || client != frame {
            return SurfaceStep::Skip;
        }
    }
    match surface_size {
        Some(size) if size == frame => SurfaceStep::Draw,
        _ => SurfaceStep::Resize {
            width: frame.0,
            height: frame.1,
        },
    }
}

/// 呈现一帧：任何 `buffer_mut()` 之前必定先完成 surface 尺寸同步。
///
/// 调用顺序（回归重点）：取帧尺寸并做 0×0 防护 → stale 检查（不碰 surface）
/// → resize（失败即返回）→ 取 buffer/校验/拷贝/present。
/// `surface_size` 由调用方跨帧跟踪（softbuffer 0.4 未暴露 `size()`）。
pub(crate) fn present_frame<S: PresentSink + ?Sized>(
    frame: Option<&Frame>,
    printing: bool,
    client: Option<(u32, u32)>,
    surface: Option<&mut S>,
    surface_size: &mut Option<(u32, u32)>,
) -> bool {
    let Some(frame) = frame else {
        return false;
    };
    // 打印期间的帧是「页帧」语义（尺寸 ≠ 客户区），不得 present 到窗口
    if printing {
        return false;
    }
    let step = surface_step((frame.width, frame.height), client, *surface_size);
    if step == SurfaceStep::Skip {
        return false;
    }
    let Some(surface) = surface else {
        return false;
    };
    if let SurfaceStep::Resize { width, height } = step {
        let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return false;
        };
        if !surface.resize(width, height) {
            // resize 失败：surface 尺寸仍未知/不符，绝不能继续取 buffer。
            return false;
        }
        *surface_size = Some((width.get(), height.get()));
    }
    surface.present_pixels(&frame.pixels)
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

/// 任务列表自绘方框（Task 5）：外接矩形恒为 `(x, y, side, side)`（排版期按行高定死），
/// 描边一律沿内缘绘制，勾选态只叠底色与对勾 —— 方框位置/边长/圆角/描边宽度不因
/// 勾选或正文字族变化，彻底摆脱 `☐`/`☑` 字形宽窄差异。
#[allow(clippy::too_many_arguments)] // 方框绘制参数天然多（同 frame 的位图原语）
fn draw_task_box(
    frame: &mut Frame,
    x: i32,
    y: i32,
    side: i32,
    done: bool,
    theme: &crate::theme::Theme,
    scale: f32,
    bg: [u8; 4],
) {
    let side = side.max(3);
    let stroke = (scale.round() as i32).max(1).min(side / 2);
    let radius = (2.0 * scale).max(1.0).min(side as f32 * 0.35);
    // 描边：外框实色 + 内缩 stroke 的底色/强调色覆盖（src-over 下 stroke 挖空无效）。
    let border = if done { theme.accent } else { theme.muted };
    frame.fill_rounded_rect(
        x,
        y,
        side as u32,
        side as u32,
        radius,
        [border[0], border[1], border[2], 255],
    );
    let inner = side - 2 * stroke;
    if inner > 0 {
        let fill = if done {
            [theme.accent[0], theme.accent[1], theme.accent[2], 255]
        } else {
            // 未勾选内部必须用所在块的底色（Callout 浅底 / Details 卡片底），
            // 否则会在带底色的块里留下页面底色的小方块。
            bg
        };
        frame.fill_rounded_rect(
            x + stroke,
            y + stroke,
            inner as u32,
            inner as u32,
            (radius - stroke as f32).max(0.0),
            fill,
        );
    }
    if done {
        // 对勾：两条斜线（粗细 = stroke），端点按方框比例取，不新增依赖也不用字形。
        let c = [
            theme.accent_text[0],
            theme.accent_text[1],
            theme.accent_text[2],
            255,
        ];
        let (x0, y0) = (x as f32, y as f32);
        let s = side as f32;
        let p = |fx: f32, fy: f32| (x0 + s * fx, y0 + s * fy);
        let (ax, ay) = p(0.27, 0.53);
        let (bx, by) = p(0.44, 0.70);
        let (cx, cy) = p(0.76, 0.33);
        fill_line(frame, ax as i32, ay as i32, bx as i32, by as i32, stroke, c);
        fill_line(frame, bx as i32, by as i32, cx as i32, cy as i32, stroke, c);
    }
}

/// 斜线段（Bresenham，每步画 `thickness` 见方的小块）：用于任务框对勾。
fn fill_line(
    frame: &mut Frame,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    thickness: i32,
    color: [u8; 4],
) {
    let t = thickness.max(1) as u32;
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y) = (x0, y0);
    let mut err = dx + dy;
    loop {
        frame.fill_rect(x, y, t, t, color);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
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
        UnderKind::MarkBg => frame.fill_rect(x, y, w, h, theme.mark_bg),
        UnderKind::KbdFrame => {
            // 边框用「外框边框色 + 内缩 1px 底色」叠出（stroke 的透明挖空
            // 在 src-over 下无效，会把整块涂成边框色）。颜色必须取自主题：
            // 深色下若用浅色底，浅色字坐在近白块上会形成眩光、整段不可读。
            let border = theme.kbd_border;
            frame.fill_rounded_rect(
                x,
                y,
                w.max(2),
                h.max(2),
                radius,
                [border[0], border[1], border[2], 255],
            );
            if w > 2 && h > 2 {
                frame.fill_rounded_rect(
                    x + 1,
                    y + 1,
                    w - 2,
                    h - 2,
                    (radius - 1.0).max(0.0),
                    theme.kbd_bg,
                );
            }
        }
        UnderKind::ImagePlaceholder => {
            // 占位块也必须是主题色：深色下浅灰块等同于一块亮斑。
            frame.fill_rounded_rect(x, y, w.max(2), h.max(2), radius, theme.chrome_border);
            if w > 2 && h > 2 {
                frame.fill_rounded_rect(
                    x + 1,
                    y + 1,
                    w - 2,
                    h - 2,
                    (radius - 1.0).max(0.0),
                    theme.code_bg,
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
    /// 刚复制过该块：按钮标签就地显示「已复制」。
    pub(crate) copied: bool,
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
        crate::tokens::palette::surface_popup(theme),
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
    let copy_text = if h.copied {
        t(Key::CodeCopiedButton)
    } else {
        t(Key::CodeCopyButton)
    };
    if let Some(lab) = labels.get_or_insert(engine, copy_text, theme.muted, scale) {
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

/// 非活动态 Front Matter 的折叠条：左侧 ▸、灰色「Front Matter」与右侧细分隔线。
/// 整条都是可点击区域（点击由 `hit_offset` 落光标，光标进入即展开源码）。
#[allow(clippy::too_many_arguments)] // 绘制原语参数天然多（同其它位图操作）
fn paint_frontmatter_collapsed(
    frame: &mut Frame,
    engine: &mut LayoutEngine,
    labels: &mut crate::chrome::LabelCache,
    theme: &crate::theme::Theme,
    scale: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    let cy = (y + h / 2.0).round() as i32;
    let ax = x.round() as i32 + (6.0 * scale).round() as i32;
    let arrow = (7.0 * scale).round().max(5.0) as i32;
    draw_arrow_right(frame, ax, cy, arrow, theme.muted);
    let tx = ax + arrow + (8.0 * scale).round() as i32;
    if let Some(lab) =
        labels.get_or_insert(engine, t(Key::FrontMatterCollapsed), theme.muted, scale)
    {
        frame.blit(&lab.pixels, lab.w, lab.h, tx, cy - lab.h as i32 / 2);
        let lx = tx + lab.w as i32 + (8.0 * scale).round() as i32;
        let lw = (x + w).round() as i32 - lx;
        if lw > 0 {
            frame.fill_rect(
                lx,
                cy,
                lw as u32,
                1,
                [theme.muted[0], theme.muted[1], theme.muted[2], 64],
            );
        }
    }
}

/// 空文档首行引导：仅当文档没有任何内容时画一行弱化提示，不参与命中与排版。
fn paint_empty_hint(
    frame: &mut Frame,
    engine: &mut LayoutEngine,
    labels: &mut crate::chrome::LabelCache,
    theme: &crate::theme::Theme,
    scale: f32,
    x: f32,
    y: f32,
) {
    if let Some(lab) = labels.get_or_insert(engine, t(Key::DocEmptyHint), theme.muted, scale) {
        frame.blit(
            &lab.pixels,
            lab.w,
            lab.h,
            x.round() as i32,
            y.round() as i32,
        );
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
            .raster_key(
                g.id,
                &g.kind,
                g.range,
                base,
                caret,
                self.toc_layout_epoch(g),
            )
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
            copied: self.window_state.code_copied == Some(g.id),
            line_top,
            line_h,
        })
    }

    /// 行内公式图：tex 内容寻址缓存；未命中时渲染并写缓存，失败记入失败表并返回 None
    /// （调用方降级为源码/文本显示，正文与表格单元格共用同一策略与缓存）。
    pub(crate) fn inline_math_image(&mut self, tex: &str) -> Option<&crate::math::MathImage> {
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
        // 纯文本行视图：等宽、不折行、统一行高（合成行块已在 ensure_layouts_visible 建好）。
        self.engine.set_plain_lines(self.source_plain_active());
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
                                images.push(PendingImage {
                                    range: n.range,
                                    url,
                                    explicit: (None, None),
                                    align: None,
                                });
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
                                images.push(PendingImage {
                                    range: n.range,
                                    url: src.to_string(),
                                    explicit: (attrs.width, attrs.height),
                                    align: attrs.align,
                                });
                            }
                        }
                        _ => {}
                    }
                }
                stack.extend(n.children.iter());
            }
            // B3：行内/容器内 `<details>` 折叠输入（仅含该标记的块才扫描，普通块零开销）。
            // 容器块（引用/列表）里的标签常退化为 HtmlLiteral，行内节点树收集不到，
            // 由 `block_details_spans` 的逐行扫描分支补齐；两者按开标签偏移去重。
            let folds: Vec<InlineFold> = if text.contains("<details") {
                let body = text.strip_suffix('\n').unwrap_or(&text);
                block_details_spans(&kind, body, &inlines, range)
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
                    align: None,
                });
            }
            // 图片题注候选（独立图片 + alt 非空）：与 HTML/PDF 导出共用内核判定
            // （`veil_core::standalone_image_caption`），三端题注语义一致。
            // 缺失图片时题注仍显示（布局层把缺失节点降级为通用占位以避免与 alt 重复）。
            let mut captions: Vec<ImageCaption> = Vec::new();
            if kind == BlockKind::Paragraph {
                if let Some((nr, caption)) =
                    veil_core::standalone_image_caption(&inlines, &text, range.start)
                {
                    captions.push(ImageCaption {
                        range: nr,
                        text: caption,
                    });
                }
            }
            for PendingImage {
                range: nrange,
                url,
                explicit,
                align,
            } in images
            {
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
                // 对齐只对独占视觉行的图片生效；行内图片保持基线内联，不跳位。
                let align = standalone.then_some(align).flatten();
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
                                self.cache.insert_inline_image(
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
                        align,
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
                    align,
                });
            }
            // Mermaid 块位图占位：cache 里有该 block_id 的 MermaidImage → 构造 InlineImg
            // 覆盖整个块（含围栏），LayoutEngine 用 NBSP 占位 + paint 按 key blit。
            // 位图是「CSS 尺寸 × 光栅倍率」的副本（`px_to_pt` 记录倍率），不能直接把
            // 像素当逻辑尺寸：宽图会越出版心并与相邻块重叠。这里还原 CSS 尺寸、乘以
            // 界面 DPI，再复用图片的限宽/限高口径。
            if matches!(kind, BlockKind::Mermaid) {
                if let Some(bm) = self.cache.mermaid_imgs.get(&id) {
                    let (dw, dh) = mermaid_display_size(
                        bm.width,
                        bm.height,
                        bm.px_to_pt,
                        scale,
                        img_max_w,
                        self.image_max_height(),
                    );
                    imgs.push(InlineImg {
                        range,
                        key: InlineAsset::Mermaid(id.0.to_string()),
                        width: dw,
                        height: dh,
                        baseline: dh,
                        align: None,
                    });
                }
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
                        Some(footnotes.as_ref()),
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
                    key: LayoutKey {
                        base,
                        reveal,
                        toc,
                        highlight_epoch: self.engine.highlight_epoch,
                    },
                    value: l,
                },
            );
            // 块高从估算变为实测：标记几何待重算，渲染帧只做一次 O(块数) 累计。
            self.index.geometry_stale = true;
        }
    }

    /// 发起远程图片请求（pending 去重）：全局阻止/已失败/源码模式一律不请求。
    pub(crate) fn request_remote_image(&mut self, url: &str) {
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
        // 先 drain 后台 parse 结果：回填 Highlighter 缓存、递增 highlight_epoch，
        // 让 LayoutKey 自然失配 → 重排时命中缓存同步产出颜色。
        self.engine.drain_background();
        // 源码模式纯文本行视图：先把块索引切换为（或恢复出）逐行几何，
        // 后续视口区间/几何累计/排版全部基于同一模型。
        self.sync_source_plain_index();
        let range = self.visible_index_range();
        self.ensure_layouts_range(range);
    }

    /// 测试专用：等价全量排版；不参与非测试构建，避免 dead_code。
    #[cfg(test)]
    pub(crate) fn ensure_layouts_all(&mut self) {
        self.sync_source_plain_index();
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
        let Some(blocks) = self.index.blocks.get(indices.clone()) else {
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
        // REN-SWEEP-001：光标进入/离开 Front Matter 时切换「隐藏 ↔ 显源码」。
        self.sync_frontmatter_at_caret();
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
        // 空文档首行引导：正文没有任何内容时补一句可开始写作的提示（打印不出现）。
        if self.window_state.print.is_none() && self.editor.session().text_len() == 0 {
            let hint_x = self.content_x();
            let hint_y = vp.y + self.engine.doc_margin_px() * scale;
            paint_empty_hint(
                &mut frame,
                &mut self.engine,
                &mut self.ui_state.chrome.labels,
                &theme,
                scale,
                hint_x,
                hint_y,
            );
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
            if self.frontmatter_hidden(g, caret) {
                // REN-SWEEP-001：非活动态 Front Matter 不绘制正文，但保留一条
                // 可点击折叠条（打印态高度仍为 0，也不绘制）。
                if self.window_state.print.is_none() {
                    let sy = (vp.y + g.top - self.window_state.scroll_y).round();
                    let fh = self.block_height_with(g, base, caret);
                    let fx = self.content_x();
                    let fw = self.content_width();
                    paint_frontmatter_collapsed(
                        &mut frame,
                        &mut self.engine,
                        &mut self.ui_state.chrome.labels,
                        &theme,
                        scale,
                        fx,
                        sy,
                        fw,
                        fh,
                    );
                }
                continue;
            }
            let (id, kind, range, top) = (g.id, g.kind.clone(), g.range, g.top);
            let sy = (vp.y + top - self.window_state.scroll_y) as i32;
            if (sy as f32) >= vp.y + vp.h
                || sy + self.block_height_with(g, base, caret) as i32 <= vp.y as i32
            {
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
                && self.block_height_with(g, base, caret) > 0.0
            {
                // 容器内表格先画引用条（表格分支会提前 continue，装饰 match 不覆盖）。
                self.paint_quote_rails(&mut frame, g, sy);
                self.paint_table(&mut frame, id, sy);
                continue;
            }
            let Some(lk) =
                self.cache
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
                    | BlockKind::Mermaid
            ) {
                let (rw, rh) = {
                    let r = &self.cache.rasters[&id].value;
                    (r.1, r.2)
                };
                let pad_x = (crate::tokens::doc::CODE_PAD_X * scale).round() as i32;
                let pad_y = (crate::tokens::doc::CODE_PAD_Y * scale).round() as i32;
                // Mermaid 块若已有位图渲染态，文本光栅只含 NBSP 占位，高度 = 位图高度。
                // 必须用 block_height_with 取整块高度（含位图撑开的部分），否则代码底色只盖占位。
                let bg_h = if matches!(kind, BlockKind::Mermaid) {
                    self.block_height_with(g, base, caret)
                } else {
                    rh as f32
                };
                frame.fill_rect(
                    self.content_x() as i32 - pad_x,
                    sy - pad_y,
                    rw + 2 * pad_x as u32,
                    (bg_h + 2.0 * pad_y as f32).max(1.0) as u32,
                    crate::tokens::palette::code_bg(&theme),
                );
            }
            // Callout（N3b）：强调色 10% 浅底色覆盖整块（含引用缩进区），文字仍后绘。
            if let Some(accent) = callout_accent(&kind) {
                let bh = self.block_height_with(g, base, caret) as u32;
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
                let bh = self.block_height_with(g, base, caret) as i32;
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
                let card_bg = crate::tokens::palette::details_card_bg(&theme, expanded);
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
                (self.block_height_with(g, base, caret).round() as u32).clamp(1, *rh)
            } else {
                *rh
            };
            // 任务列表自绘方框（Task 5）：几何在排版期由行高定死（勾选前后同一矩形），
            // 勾选态只叠底色与对勾，不再依赖 `☐`/`☑` 字形，故换字族也不变。
            // 未勾选方框内部用所在块底色（Callout 浅底 / Details 卡片底），避免页面底色露出。
            let task_box_bg = if let Some(accent) = callout_accent(&kind) {
                tint(accent, crate::tokens::palette::surface_base(&theme), 0.10)
            } else if details_card.is_some() {
                crate::tokens::palette::details_card_bg(
                    &theme,
                    self.ui_state.expanded_details.contains(&id),
                )
            } else {
                crate::tokens::palette::surface_base(&theme)
            };
            for b in &l.task_boxes {
                draw_task_box(
                    &mut frame,
                    (self.content_x() + b.x) as i32,
                    sy + b.y as i32,
                    b.side.round() as i32,
                    b.done,
                    &theme,
                    scale,
                    task_box_bg,
                );
            }
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
                    InlineAsset::Mermaid(block_id_str) => {
                        // key = block_id.to_string()，转回 BlockId 查 mermaid_imgs
                        if let Ok(bid) = block_id_str.parse::<u64>() {
                            let block_id = veil_core::BlockId(bid);
                            if let Some(img) = self.cache.mermaid_imgs.get(&block_id) {
                                frame.blit_scaled(
                                    &img.data, img.width, img.height, p.width, p.height, dx, dy,
                                );
                            }
                        }
                    }
                }
            }
            // 块级装饰（切片 12）：引用左竖条；分隔线非激活时画居中横线
            match kind {
                BlockKind::BlockQuote => {
                    let bh = self.block_height_with(g, base, caret) as u32;
                    frame.fill_rect(
                        self.content_x() as i32 - (crate::tokens::doc::QUOTE_INSET * scale) as i32,
                        sy,
                        (crate::tokens::doc::QUOTE_RAIL * scale).max(1.0) as u32,
                        bh,
                        theme.grid,
                    );
                    // REN-SWEEP-004：多层引用按层级补画次级引用条。
                    self.paint_nested_quote_rails(&mut frame, g, l, sy);
                }
                BlockKind::Admonition(_) | BlockKind::Callout(_) => {
                    // 提示块：左竖条按类型着色（引用灰条 → 语义/配置色），不改字节。
                    let bh = self.block_height_with(g, base, caret) as u32;
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
                    let bh = self.block_height_with(g, base, caret);
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
                let h = self.block_height_with(g, base, caret).ceil() as u32;
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
        if self.ui_state.image_preview.is_some() && self.window_state.print.is_none() {
            crate::view::image_preview::paint(self, &mut frame);
        } else if self.ui_state.settings.is_some() && self.window_state.print.is_none() {
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
    /// 从 self.window_state.frame 取缓冲，拷进 surface 并 present；返回是否成功上屏。
    /// 无 surface（headless）、打印页帧、尺寸仍未对齐时返回 false。
    ///
    /// surface 与客户区尺寸不一致时按当前帧尺寸重建后再拷：softbuffer 只对自身缓冲
    /// 尺寸 BitBlt、不会拉伸，沿用旧尺寸会把右下留在未擦除状态（启动黑块）。
    pub(crate) fn present(&mut self) -> bool {
        let WindowState {
            frame,
            print,
            window,
            surface,
            surface_size,
            ..
        } = &mut self.window_state;
        let client = window.as_ref().map(|win| {
            let size = win.inner_size();
            (size.width, size.height)
        });
        let mut sink = surface.as_mut().map(SoftSurface);
        present_frame(
            frame.as_ref(),
            print.is_some(),
            client,
            sink.as_mut().map(|s| s as &mut dyn PresentSink),
            surface_size,
        )
    }

    /// 帧合成完整路径：render → present。产品层事件循环 Redraw 分支唯一调用。
    pub(crate) fn paint(&mut self) {
        let started = std::time::Instant::now();
        let _ = self.render();
        let _ = self.present();
        crate::perf::frame(started.elapsed());
    }
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

/// Mermaid 位图的逻辑显示尺寸。
///
/// 位图是「SVG 的 CSS 尺寸 × 光栅倍率」的副本，`px_to_pt = 72 / (96 * scale)`
/// 记录了倍率；直接把位图像素当逻辑尺寸会让宽图（≥2000px）越出版心。
/// 这里先还原 CSS 像素，再乘界面 DPI 得到逻辑像素，最后复用图片的限宽/限高
/// 口径（`image_display_size` → `fit_size_box`，只缩小不放大）。
fn mermaid_display_size(
    bmp_w: u32,
    bmp_h: u32,
    px_to_pt: f32,
    ui_scale: f32,
    max_w: f32,
    max_h: f32,
) -> (u32, u32) {
    // px_to_pt = 72 / (96 * raster) → CSS px = 位图 px * px_to_pt * 96 / 72。
    // 非法/缺失换算比时按默认光栅倍率 2.0 还原（与 `svg_to_bitmap` 的 clamp 下限一致）。
    let css_per_bmp = if px_to_pt.is_finite() && px_to_pt > 0.0 {
        px_to_pt * (96.0 / 72.0)
    } else {
        0.5
    };
    let ui = if ui_scale.is_finite() && ui_scale > 0.0 {
        ui_scale
    } else {
        1.0
    };
    let nat_w = (bmp_w as f32 * css_per_bmp * ui).round().max(1.0) as u32;
    let nat_h = (bmp_h as f32 * css_per_bmp * ui).round().max(1.0) as u32;
    crate::view::image_display_size(nat_w, nat_h, (None, None), max_w, max_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "| a | b |\n| --- | --- |\n| c | d |\n";
    const TABLE_2X2_ALIGNED: &str = "| a | b |\n| :-- | --: |\n| c | d |\n| e | f |\n";

    #[test]
    fn table_html_image_seed_never_strips_visible_alt() {
        let doc =
            "| 左 <img src=\"a.png\" alt=\"HTMLalt\"> 右 | ![MDalt](b.png) |\n| --- | --- |\n";
        let session = veil_core::Session::open(doc);
        let (id, _, _) = session.blocks()[0];
        let table = session.table(id).unwrap();
        let seeds = table_cell_img_seeds(&session, &table, u32::MAX);
        assert!(seeds[&0][0].alt.is_none(), "HTML alt 不得成为删除区间");
        assert!(seeds[&1][0].alt.is_some(), "Markdown alt 才可按区间去重");
        assert_eq!(table.header[0].text, "左 HTMLalt 右");
    }

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

    #[test]
    fn row_split_matches_kernel_escaping() {
        // `\|` 与 code span 内的 `|` 都不是列分隔符
        let cells = split_row("| a \\| b | `x|y` | c |");
        assert_eq!(cells, vec!["a \\| b", "`x|y`", "c"]);
        // 写路径整行往返不得改写单元格原文
        assert_eq!(join_row(&cells), "| a \\| b | `x|y` | c |");
    }

    #[test]
    fn table_column_edits_preserve_escaped_cells() {
        let src = "| a \\| b | `x|y` |\n| --- | --- |\n| c \\| d | e |\n";
        let inserted = table_insert_column(src, 1);
        assert_eq!(
            inserted,
            "| a \\| b |  | `x|y` |\n| --- | --- | --- |\n| c \\| d |  | e |\n"
        );
        // 再删除同一列回到原文（列数、转义与内容逐字节一致）
        assert_eq!(table_delete_column(&inserted, 1), src);
        // 追加空行按 header 的 2 列补齐，不因转义竖线多出列
        assert_eq!(table_append_row(src), format!("{src}|  |  |\n"));
    }

    #[test]
    fn table_align_keeps_escaped_cells_and_cell_start() {
        let src = "| a \\| b | c |\n| --- | --- |\n";
        assert_eq!(
            table_set_align(src, 1, veil_core::CellAlign::Center).unwrap(),
            "| a \\| b | c |\n| --- | :-: |\n"
        );
        // 第二列起点按转义口径计算，不被 `\|` 提前截断
        assert_eq!(table_cell_start(src, 0, 1), 11);
    }

    #[test]
    fn mermaid_display_size_restores_css_size_and_clamps_to_content_width() {
        // 2400×1200 位图 @2x → CSS 1200×600；界面 DPI 1.0、版心 640 → 限宽到 640。
        let (w, h) = mermaid_display_size(2400, 1200, 0.375, 1.0, 640.0, 10000.0);
        assert_eq!(w, 640, "宽图必须压到版心宽：{w}");
        assert_eq!(h, 320, "等比缩放后高度：{h}");
        // 小图不被放大：600×300 位图 @2x → CSS 300×150。
        let (w, h) = mermaid_display_size(600, 300, 0.375, 1.0, 640.0, 10000.0);
        assert_eq!((w, h), (300, 150), "小图保持 CSS 尺寸，不放大");
        // 界面 DPI 2.0 时按逻辑像素放大一倍（仍不超版心）。
        let (w, _) = mermaid_display_size(600, 300, 0.375, 2.0, 2000.0, 10000.0);
        assert_eq!(w, 600, "高 DPI 下 CSS 尺寸按界面倍率换算");
        // 非法换算比回退到默认 2x 光栅，不退化为 0。
        let (w, h) = mermaid_display_size(400, 200, f32::NAN, 1.0, 640.0, 10000.0);
        assert_eq!((w, h), (200, 100));
        // 限高同样生效（视口 80%）。
        let (_, h) = mermaid_display_size(800, 8000, 0.375, 1.0, 640.0, 400.0);
        assert_eq!(h, 400);
    }
}
