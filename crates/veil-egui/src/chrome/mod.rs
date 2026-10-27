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
/// 菜单项行高（基准值）。
pub const ITEM_H: f32 = 28.0;
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    /// 新建空白草稿（Ctrl+N）。
    New,
    Open,
    Save,
    SaveAs,
    Export,
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
    ThemeSystem,
    ThemeLight,
    ThemeDark,
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
            ("保存", "Ctrl+S", MenuAction::Save),
            ("另存为…", "Ctrl+Shift+S", MenuAction::SaveAs),
            ("导出 HTML…", "Ctrl+E", MenuAction::Export),
            ("退出", "", MenuAction::Quit),
        ],
    },
    MenuDef {
        title: "编辑",
        items: &[
            ("撤销", "Ctrl+Z", MenuAction::Undo),
            ("重做", "Ctrl+Shift+Z", MenuAction::Redo),
            ("剪切", "Ctrl+X", MenuAction::Cut),
            ("复制", "Ctrl+C", MenuAction::Copy),
            ("粘贴", "Ctrl+V", MenuAction::Paste),
            ("全选", "Ctrl+A", MenuAction::SelectAll),
            ("查找…", "Ctrl+F", MenuAction::Find),
            ("替换…", "Ctrl+H", MenuAction::Replace),
        ],
    },
    MenuDef {
        title: "视图",
        items: &[
            ("大纲面板", "Ctrl+Shift+O", MenuAction::ToggleOutline),
            ("跟随系统", "", MenuAction::ThemeSystem),
            ("浅色", "", MenuAction::ThemeLight),
            ("深色", "", MenuAction::ThemeDark),
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
/// 返回 (标签, 快捷键提示, 动作)；最近文件标签为文件名（无文件名回退全路径）。
/// 布局/命中/分发共用本函数，保证三者对同一菜单结构一致。
pub(crate) fn menu_rows(open: usize, recent: &[String]) -> Vec<(String, &'static str, MenuAction)> {
    let Some(menu) = MENUS.get(open) else {
        return Vec::new();
    };
    let statics = || menu.items.iter().map(|(l, s, a)| (l.to_string(), *s, *a));
    // 仅文件菜单（下标 0）且有最近文件时动态插入；插入点为最后一项（退出）之前
    if open != 0 || recent.is_empty() {
        return statics().collect();
    }
    let tail = menu.items.len() - 1;
    let mut out: Vec<_> = statics().take(tail).collect();
    out.extend(
        recent
            .iter()
            .enumerate()
            .map(|(i, l)| (l.clone(), "", MenuAction::OpenRecent(i))),
    );
    out.extend(statics().skip(tail));
    out
}

/// 弹出菜单布局：位置贴当前菜单标题左缘，宽取最长项（含快捷键）并留内边距。
pub struct PopupLayout {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// (项下标, x, y, w, h)
    pub items: Vec<(usize, f32, f32, f32, f32)>,
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
        };
    }
    let x = menu_bar_rects(scale)
        .into_iter()
        .find(|&(i, _, _)| i == open)
        .map(|(_, x, _)| x)
        .unwrap_or(0.0);
    let pad = 12.0 * scale;
    let gap = 24.0 * scale;
    let widest = rows
        .iter()
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
    let item_h = ITEM_H * scale;
    let top_pad = 4.0 * scale;
    let y = MENU_H * scale;
    let h = 2.0 * top_pad + item_h * rows.len() as f32;
    let items = rows
        .iter()
        .enumerate()
        .map(|(i, _)| (i, x, y + top_pad + item_h * i as f32, w, item_h))
        .collect();
    PopupLayout { x, y, w, h, items }
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

/// 状态栏文本：(左, 右)。左 = 路径或「未命名草稿」+ 脏标 + 外部变更提示；右 = 行列。
pub fn status_line(
    path: Option<&Path>,
    dirty: bool,
    external: ExternalState,
    line: usize,
    col: usize,
) -> (String, String) {
    let mut left = match path {
        Some(p) => p.display().to_string(),
        None => "未命名草稿".to_string(),
    };
    if dirty {
        left.push_str(" *");
    }
    match external {
        ExternalState::None => {}
        ExternalState::Changed => left.push_str("（外部修改）"),
        ExternalState::BackedUp => left.push_str("（外部修改已备份）"),
        ExternalState::Missing => left.push_str("（文件缺失）"),
    }
    (left, format!("行 {line}，列 {col}"))
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
    pub(crate) labels: LabelCache,
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

#[cfg(test)]
mod tests;
