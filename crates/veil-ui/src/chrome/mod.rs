//! 自绘 chrome：菜单栏、弹出菜单、状态栏与滚动条（切片 15）。
//!
//! 零 GUI 依赖：几何、命中、滚动换算、状态文本全部为纯函数，headless 可测；
//! 绘制只依赖 `Frame` 与 `LayoutEngine`（标签经 preedit 排版 + 光栅化）。
//! 策略（何时开菜单、动作如何执行）在 shell；本层只回答「画在哪/点到谁」。
//!
//! 三责拆分：本文件 = 几何布局 + 菜单模型 + 命中矩形（模型层）；
//! paint.rs = 菜单栏/状态栏/滚动条绘制；cache.rs = 标签位图缓存。

mod cache;
mod paint;

use std::path::Path;

use crate::document::ExternalState;
use crate::geometry::BASE_FONT_SIZE;
use crate::theme::ThemeMode;

pub(crate) use cache::LabelCache;

/// chrome 基准高度/宽度（乘 DPI scale 使用）。
pub const MENU_H: f32 = 30.0;
pub const STATUS_H: f32 = 26.0;
pub const SCROLL_W: f32 = 12.0;
/// 滚动条滑块最小高度（基准值）。
pub const THUMB_MIN_H: f32 = 24.0;

/// 内容视口：菜单栏以下、状态栏以上。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn viewport(win_w: u32, win_h: u32, scale: f32) -> Viewport {
    let menu_h = MENU_H * scale;
    let status_h = STATUS_H * scale;
    Viewport {
        x: 0.0,
        y: menu_h,
        w: win_w.max(1) as f32,
        h: (win_h as f32 - menu_h - status_h).max(1.0),
    }
}

/// 菜单动作 + 分隔线哨兵（不参与动作路由、不进命中区、不绘制文字）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    /// 分隔线：无动作，仅在布局和绘制层识别。
    Separator,
    /// 新建空白草稿（Ctrl+N）。
    New,
    Open,
    /// 快速打开浮层（Ctrl+P）。
    QuickOpen,
    Save,
    SaveAs,
    Export,
    /// 直接导出 PDF（Ctrl+Shift+E）：无打印对话框，写同目录 `<stem>.pdf`。
    ExportPdf,
    /// 打印 / 导出 PDF（3b）：系统打印对话框，PDF 走「Microsoft Print to PDF」。
    Print,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    /// 全选（Ctrl+A）。
    SelectAll,
    /// 切换大纲面板（P3-2）。
    ToggleOutline,
    /// 打开文档内查找栏（P3-3）。
    Find,
    /// 打开查找替换栏（Ctrl+H）。
    Replace,
    /// 插入图片（2f）：系统对话框选图，插入相对路径链接。
    InsertImage,
    /// 行内格式（3c-1）：加粗/斜体切换、插入链接。
    ToggleBold,
    ToggleItalic,
    InsertLink,
    ThemeSystem,
    ThemeLight,
    ThemeDark,
    /// 切换专注模式（阶段 1）。
    ToggleFocus,
    /// 切换打字机模式（阶段 1）。
    ToggleTypewriter,
    /// 切换源码模式（Ctrl+/）：全文按 Markdown 源码排版。
    ToggleSourceMode,
    /// 视图缩放（3c-2）：放大/缩小一档、还原 100%。
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// 全屏切换（3c-3，F11）：无边框全屏。
    ToggleFullscreen,
    Quit,
    /// 打开最近文件（0b）：下标为最近列表位置（0 = 最近一次）。
    OpenRecent(usize),
}

pub struct MenuDef {
    pub title: &'static str,
    /// (标签, 快捷键提示, 动作)
    pub items: &'static [(&'static str, &'static str, MenuAction)],
}

pub const MENUS: [MenuDef; 3] = [
    MenuDef {
        title: "文件",
        items: &[
            ("新建", "Ctrl+N", MenuAction::New),
            ("打开…", "Ctrl+O", MenuAction::Open),
            ("快速打开…", "Ctrl+P", MenuAction::QuickOpen),
            ("", "", MenuAction::Separator),
            ("保存", "Ctrl+S", MenuAction::Save),
            ("另存为…", "Ctrl+Shift+S", MenuAction::SaveAs),
            ("", "", MenuAction::Separator),
            ("导出 HTML", "Ctrl+E", MenuAction::Export),
            ("导出 PDF", "Ctrl+Shift+E", MenuAction::ExportPdf),
            ("打印…", "Ctrl+Shift+P", MenuAction::Print),
            ("", "", MenuAction::Separator),
            ("退出", "", MenuAction::Quit),
        ],
    },
    MenuDef {
        title: "编辑",
        items: &[
            ("撤销", "Ctrl+Z", MenuAction::Undo),
            ("重做", "Ctrl+Shift+Z", MenuAction::Redo),
            ("", "", MenuAction::Separator),
            ("剪切", "Ctrl+X", MenuAction::Cut),
            ("复制", "Ctrl+C", MenuAction::Copy),
            ("粘贴", "Ctrl+V", MenuAction::Paste),
            ("全选", "Ctrl+A", MenuAction::SelectAll),
            ("", "", MenuAction::Separator),
            ("加粗", "Ctrl+B", MenuAction::ToggleBold),
            ("斜体", "Ctrl+I", MenuAction::ToggleItalic),
            ("链接", "Ctrl+K", MenuAction::InsertLink),
            ("", "", MenuAction::Separator),
            ("查找…", "Ctrl+F", MenuAction::Find),
            ("替换…", "Ctrl+H", MenuAction::Replace),
            ("", "", MenuAction::Separator),
            ("插入图片…", "Ctrl+Shift+I", MenuAction::InsertImage),
        ],
    },
    MenuDef {
        title: "视图",
        items: &[
            ("大纲面板", "Ctrl+Shift+O", MenuAction::ToggleOutline),
            ("", "", MenuAction::Separator),
            ("放大", "Ctrl+=", MenuAction::ZoomIn),
            ("缩小", "Ctrl+-", MenuAction::ZoomOut),
            ("还原缩放", "Ctrl+0", MenuAction::ZoomReset),
            ("", "", MenuAction::Separator),
            ("全屏", "F11", MenuAction::ToggleFullscreen),
            ("", "", MenuAction::Separator),
            ("跟随系统", "", MenuAction::ThemeSystem),
            ("浅色", "", MenuAction::ThemeLight),
            ("深色", "", MenuAction::ThemeDark),
            ("", "", MenuAction::Separator),
            ("源码模式", "Ctrl+/", MenuAction::ToggleSourceMode),
            ("专注模式", "", MenuAction::ToggleFocus),
            ("打字机模式", "", MenuAction::ToggleTypewriter),
        ],
    },
];

/// chrome 专用近似文本宽度：CJK 全角按字号、ASCII 按半字号。
/// 只服务菜单栏/弹窗的矩形与命中区，不参与文档排版；文字绘制走 LayoutEngine
/// 实测光栅，二者允许数像素差异（命中框略宽无碍）。这样命中判定不依赖上一帧绘制状态。
const ASCII_ADVANCE_RATIO: f32 = 0.52;

fn text_width(text: &str, scale: f32) -> f32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                BASE_FONT_SIZE * ASCII_ADVANCE_RATIO * scale
            } else {
                BASE_FONT_SIZE * scale
            }
        })
        .sum()
}

/// 菜单标题矩形：(菜单下标, x, 宽)。标题间无间隙，命中按矩形判定。
pub fn menu_bar_rects(scale: f32) -> Vec<(usize, f32, f32)> {
    let pad = 14.0 * scale;
    let mut x = 8.0 * scale;
    let mut out = Vec::with_capacity(MENUS.len());
    for (i, m) in MENUS.iter().enumerate() {
        let w = text_width(m.title, scale) + 2.0 * pad;
        out.push((i, x, w));
        x += w;
    }
    out
}

/// 菜单栏命中：仅菜单栏高度内有效。
pub fn hit_menu_bar(x: f32, y: f32, scale: f32) -> Option<usize> {
    if y < 0.0 || y >= MENU_H * scale {
        return None;
    }
    menu_bar_rects(scale)
        .into_iter()
        .find(|&(_, rx, rw)| x >= rx && x < rx + rw)
        .map(|(i, _, _)| i)
}

/// 菜单项行（0b 动态化）：静态项 + 文件菜单末尾插入的最近文件（在「退出」之前）。
/// 返回 (标签, 快捷键提示, 动作)；最近文件标签由调用方按 `path_label` 生成
/// （`文件名 · 父目录`，同名文件可区分）。
/// Separator 项原样返回，由 popup_layout / paint 层处理（跳过命中、绘制分隔线）。
pub(crate) fn menu_rows(open: usize, recent: &[String]) -> Vec<(String, &'static str, MenuAction)> {
    let Some(menu) = MENUS.get(open) else {
        return Vec::new();
    };
    let statics = || menu.items.iter().map(|(l, s, a)| (l.to_string(), *s, *a));
    // 仅文件菜单（下标 0）且有最近文件时动态插入
    if open != 0 || recent.is_empty() {
        return statics().collect();
    }
    // 插入点：最后一条分隔线之后（即「退出」区块之前）。
    // 结果结构：… 打印 │ 最近文件… │ 退出，最近文件块上下各有分隔线。
    let insert = menu
        .items
        .iter()
        .rposition(|(_, _, a)| matches!(a, MenuAction::Separator))
        .map(|i| i + 1)
        .unwrap_or(menu.items.len());
    let mut out: Vec<_> = statics().take(insert).collect();
    out.extend(
        recent
            .iter()
            .enumerate()
            .map(|(i, l)| (l.clone(), "", MenuAction::OpenRecent(i))),
    );
    out.push((String::new(), "", MenuAction::Separator));
    out.extend(statics().skip(insert));
    out
}

/// 弹出菜单布局：位置贴当前菜单标题左缘，宽取最长项（含快捷键）并留内边距。
pub struct PopupLayout {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// (项下标, x, y, w, h)；Separator 项不进 hit rect
    pub items: Vec<(usize, f32, f32, f32, f32)>,
    /// 分隔线段：(下标, y)；paint.rs 用
    pub separators: Vec<(usize, f32)>,
}

pub fn popup_layout(open: usize, scale: f32, recent: &[String]) -> PopupLayout {
    let rows = menu_rows(open, recent);
    if rows.is_empty() {
        return PopupLayout {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            items: Vec::new(),
            separators: Vec::new(),
        };
    }
    let x = menu_bar_rects(scale)
        .into_iter()
        .find(|&(i, _, _)| i == open)
        .map(|(_, x, _)| x)
        .unwrap_or(0.0);
    let pad = crate::theme::surface::padding(scale);
    let gap = 24.0 * scale;
    let widest = rows
        .iter()
        .filter(|(_, _, a)| !matches!(a, MenuAction::Separator))
        .map(|(label, sc, _)| {
            text_width(label, scale)
                + if sc.is_empty() {
                    0.0
                } else {
                    gap + text_width(sc, scale)
                }
        })
        .fold(0.0_f32, f32::max);
    let w = (widest + 2.0 * pad).max(150.0 * scale);
    let item_h = crate::theme::surface::row_height(scale);
    let sep_h = crate::theme::surface::divider_height(scale);
    let top_pad = 4.0 * scale;
    let y = MENU_H * scale;
    let mut cur_y = top_pad;
    let mut items = Vec::new();
    let mut separators = Vec::new();
    for (i, (_label, _sc, action)) in rows.iter().enumerate() {
        if matches!(action, MenuAction::Separator) {
            separators.push((i, y + cur_y + sep_h / 2.0));
            cur_y += sep_h;
        } else {
            items.push((i, x, y + cur_y, w, item_h));
            cur_y += item_h;
        }
    }
    PopupLayout {
        x,
        y,
        w,
        h: cur_y + top_pad,
        items,
        separators,
    }
}

/// 弹出菜单内命中项下标。
pub fn hit_popup_item(x: f32, y: f32, open: usize, scale: f32, recent: &[String]) -> Option<usize> {
    popup_layout(open, scale, recent)
        .items
        .into_iter()
        .find(|&(_, ix, iy, iw, ih)| x >= ix && x < ix + iw && y >= iy && y < iy + ih)
        .map(|(i, _, _, _, _)| i)
}

/// 点是否落在弹出菜单矩形内（含内边距；用于「点在菜单内但不在项上」时吞掉点击）。
pub fn popup_contains(x: f32, y: f32, open: usize, scale: f32, recent: &[String]) -> bool {
    let p = popup_layout(open, scale, recent);
    x >= p.x && x < p.x + p.w && y >= p.y && y < p.y + p.h
}

pub fn action_at(open: usize, item: usize, recent: &[String]) -> Option<MenuAction> {
    menu_rows(open, recent).get(item).map(|(_, _, a)| *a)
}

/// 最大滚动量（文档高不超过视口时恒 0）。
pub fn max_scroll(vp: Viewport, doc_h: f32) -> f32 {
    (doc_h - vp.h).max(0.0)
}

/// 滚动条滑块 (y, h)；文档不溢出视口时 None。
pub fn scroll_thumb(vp: Viewport, doc_h: f32, scroll: f32, scale: f32) -> Option<(f32, f32)> {
    if doc_h <= vp.h + 0.5 {
        return None;
    }
    // 退化视口（如最小化后 vp.h=1.0）根本无法容纳最小滑块，直接放弃；
    // 这也避免 clamp(THUMB_MIN_H*scale=36, vp.h=1) 时 min > max 的 panic。
    if vp.h < THUMB_MIN_H * scale {
        return None;
    }
    let h = (vp.h * vp.h / doc_h).clamp(THUMB_MIN_H * scale, vp.h);
    let max = max_scroll(vp, doc_h);
    if max <= 0.0 {
        return None;
    }
    let y = vp.y + (scroll.clamp(0.0, max) / max) * (vp.h - h);
    Some((y, h))
}

/// 滑块顶边位置 → 滚动量（拖动换算）。
pub fn scroll_from_thumb(vp: Viewport, doc_h: f32, thumb_y: f32, scale: f32) -> f32 {
    let Some((_, h)) = scroll_thumb(vp, doc_h, 0.0, scale) else {
        return 0.0;
    };
    let span = vp.h - h;
    if span <= 0.0 {
        return 0.0;
    }
    ((thumb_y - vp.y) / span).clamp(0.0, 1.0) * max_scroll(vp, doc_h)
}

/// 翻页（dir=±1），按视口高 90% 步进并钳位。
pub fn page_scroll(vp: Viewport, doc_h: f32, scroll: f32, dir: i32) -> f32 {
    (scroll + dir as f32 * vp.h * 0.9).clamp(0.0, max_scroll(vp, doc_h))
}

/// 滚动条命中：视口右缘 SCROLL_W 内，且文档确实溢出。
pub fn scrollbar_hit(vp: Viewport, doc_h: f32, x: f32, y: f32, scale: f32) -> bool {
    doc_h > vp.h + 0.5
        && y >= vp.y
        && y < vp.y + vp.h
        && x >= vp.x + vp.w - SCROLL_W * scale
        && x < vp.x + vp.w
}

/// 路径标签：`文件名 · 父目录`，无路径时返回「未命名草稿」。
/// 用于窗口标题、状态栏、notice 等所有需要展示路径的位置——统一避免完整路径太吵。
pub fn path_label(p: Option<&Path>) -> String {
    let Some(p) = p else {
        return "未命名草稿".to_string();
    };
    let stem = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("未命名草稿");
    let parent = p
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|n| n.to_str());
    match parent {
        Some(parent) if !parent.is_empty() => format!("{} · {}", stem, parent),
        _ => stem.to_string(),
    }
}

/// 状态栏文本：(左, 右)。左 = path_label + 脏标 + 外部变更提示 + 源码模式提示；
/// 右 = 全文字数 + 行列。
pub fn status_line(
    path: Option<&Path>,
    dirty: bool,
    external: ExternalState,
    source_mode: bool,
    words: usize,
    line: usize,
    col: usize,
) -> (String, String) {
    let mut left = path_label(path);
    if dirty {
        left.push_str(" *");
    }
    match external {
        ExternalState::None => {}
        ExternalState::Changed => left.push_str("（外部修改）"),
        ExternalState::BackedUp => left.push_str("（外部修改已备份）"),
        ExternalState::Missing => left.push_str("（文件缺失）"),
    }
    if source_mode {
        left.push_str("（源码模式）");
    }
    (left, format!("{words} 字 · 行 {line}，列 {col}"))
}

/// chrome 交互态与标签缓存。`notice` 为状态栏临时提示（动作失败/成功）。
#[derive(Default)]
pub struct Chrome {
    pub open: Option<usize>,
    pub hover_menu: Option<usize>,
    /// (菜单下标, 项下标)
    pub hover_item: Option<(usize, usize)>,
    pub notice: String,
    /// 拖动滚动条时鼠标相对滑块顶边的偏移。
    pub scroll_drag: Option<f32>,
    /// 指针悬停在滚动条上（滑块或轨道）：滑块加亮，与拖动态同一视觉。
    pub scroll_hover: bool,
    pub(crate) labels: LabelCache,
    /// 状态栏左文中段省略结果缓存（S1）：键 = (原文, 可用宽 bits, scale bits)。
    /// 打字期右文变而左文/窗口不变，命中避免每帧二分重排。
    pub(crate) status_fit: std::collections::HashMap<(String, u32, u32), String>,
}

impl Chrome {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn close(&mut self) {
        self.open = None;
        self.hover_item = None;
    }

    pub fn toggle(&mut self, menu: usize) {
        if self.open == Some(menu) {
            self.close();
        } else {
            self.open = Some(menu);
            self.hover_item = None;
        }
    }

    /// DPI 变化：标签缓存的像素尺寸按旧 scale 生成，必须整体作废。
    pub fn clear_label_cache(&mut self) {
        self.labels.clear();
        self.status_fit.clear();
    }
}

/// 主题菜单项与当前模式是否一致（用于「当前」标记；非主题项恒 false）。
pub(super) fn theme_action_matches(action: MenuAction, mode: ThemeMode) -> bool {
    matches!(
        (action, mode),
        (MenuAction::ThemeSystem, ThemeMode::System)
            | (MenuAction::ThemeLight, ThemeMode::Light)
            | (MenuAction::ThemeDark, ThemeMode::Dark)
    )
}

/// 菜单开关项的当前开启状态（「当前」提示的唯一数据源）+ 项可用性。
/// 不包含大纲面板（用户要求：大纲是面板开关而非写作模式，不显示任何状态标识）。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct MenuFlags {
    pub(crate) fullscreen: bool,
    pub(crate) focus: bool,
    pub(crate) typewriter: bool,
    pub(crate) source: bool,
    /// 有可保存内容（脏文档或未命名草稿）。
    pub(crate) can_save: bool,
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
}

/// 开关项是否处于开启态（非开关项恒 false）：决定弹出菜单该行右侧是否显示「当前」。
pub(crate) fn action_checked(action: MenuAction, flags: MenuFlags) -> bool {
    match action {
        MenuAction::ToggleFullscreen => flags.fullscreen,
        MenuAction::ToggleFocus => flags.focus,
        MenuAction::ToggleTypewriter => flags.typewriter,
        MenuAction::ToggleSourceMode => flags.source,
        _ => false,
    }
}

/// 菜单项是否可用：不可用时置灰、不可高亮、不可回车/点击执行。
/// 只表达「此刻必然无效果」的项；拿不准的（如剪贴板是否为空）一律保持可用。
pub(crate) fn action_enabled(action: MenuAction, flags: MenuFlags) -> bool {
    match action {
        MenuAction::Separator => false,
        MenuAction::Save => flags.can_save,
        MenuAction::Undo => flags.can_undo,
        MenuAction::Redo => flags.can_redo,
        _ => true,
    }
}

#[cfg(test)]
mod tests;
