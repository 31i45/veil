//! 自绘 chrome：菜单栏、弹出菜单、状态栏与滚动条（切片 15）。
//!
//! 零 GUI 依赖：几何、命中、滚动换算、状态文本全部为纯函数，headless 可测；
//! 绘制只依赖 `Frame` 与 `LayoutEngine`（标签经 preedit 排版 + 光栅化）。
//! 策略（何时开菜单、动作如何执行）在 shell；本层只回答「画在哪/点到谁」。

use std::collections::HashMap;
use std::path::Path;

use crate::frame::Frame;
use crate::layout::{LayoutEngine, BASE_FONT_SIZE};
use crate::theme::{Theme, ThemeMode};

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
    Open,
    Save,
    SaveAs,
    Export,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    ThemeSystem,
    ThemeLight,
    ThemeDark,
    Quit,
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
        ],
    },
    MenuDef {
        title: "视图",
        items: &[
            ("跟随系统", "", MenuAction::ThemeSystem),
            ("浅色", "", MenuAction::ThemeLight),
            ("深色", "", MenuAction::ThemeDark),
        ],
    },
];

/// 文本宽度估算：CJK 全角按字号、ASCII 按半字号（菜单/状态栏布局无需真实字体度量，
/// 避免命中依赖上一帧绘制状态）。
fn text_width(text: &str, scale: f32) -> f32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                BASE_FONT_SIZE * 0.52 * scale
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

/// 弹出菜单布局：位置贴当前菜单标题左缘，宽取最长项（含快捷键）并留内边距。
pub struct PopupLayout {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// (项下标, x, y, w, h)
    pub items: Vec<(usize, f32, f32, f32, f32)>,
}

pub fn popup_layout(open: usize, scale: f32) -> PopupLayout {
    let Some(menu) = MENUS.get(open) else {
        return PopupLayout {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            items: Vec::new(),
        };
    };
    let x = menu_bar_rects(scale)
        .into_iter()
        .find(|&(i, _, _)| i == open)
        .map(|(_, x, _)| x)
        .unwrap_or(0.0);
    let pad = 12.0 * scale;
    let gap = 24.0 * scale;
    let widest = menu
        .items
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
    let h = 2.0 * top_pad + item_h * menu.items.len() as f32;
    let items = menu
        .items
        .iter()
        .enumerate()
        .map(|(i, _)| (i, x, y + top_pad + item_h * i as f32, w, item_h))
        .collect();
    PopupLayout { x, y, w, h, items }
}

/// 弹出菜单内命中项下标。
pub fn hit_popup_item(x: f32, y: f32, open: usize, scale: f32) -> Option<usize> {
    popup_layout(open, scale)
        .items
        .into_iter()
        .find(|&(_, ix, iy, iw, ih)| x >= ix && x < ix + iw && y >= iy && y < iy + ih)
        .map(|(i, _, _, _, _)| i)
}

/// 点是否落在弹出菜单矩形内（含内边距；用于「点在菜单内但不在项上」时吞掉点击）。
pub fn popup_contains(x: f32, y: f32, open: usize, scale: f32) -> bool {
    let p = popup_layout(open, scale);
    x >= p.x && x < p.x + p.w && y >= p.y && y < p.y + p.h
}

pub fn action_at(open: usize, item: usize) -> Option<MenuAction> {
    MENUS.get(open)?.items.get(item).map(|(_, _, a)| *a)
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

/// 外部变更状态（状态栏/标题提示，切片 16）。优先级：`Changed` > `BackedUp` > `Missing`。
/// 新的待备份版本必须盖过旧版本的「已备份」提示，否则用户会误以为当前磁盘版本已有副本。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ExternalState {
    #[default]
    None,
    /// 检测到外部内容变化，尚未备份（保存覆盖前会先备份）。
    Changed,
    /// 已把外部版本写成冲突副本。
    BackedUp,
    /// 磁盘文件被删除或不可读。
    Missing,
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

#[derive(Clone)]
struct CachedLabel {
    pixels: Vec<u8>,
    w: u32,
    h: u32,
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
    labels: HashMap<String, CachedLabel>,
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

    fn ensure_label(&mut self, engine: &mut LayoutEngine, text: &str, color: [u8; 3], scale: f32) {
        let key = label_key(scale, text, color);
        if self.labels.contains_key(&key) {
            return;
        }
        let layout = engine.layout_preedit(text);
        let (pixels, w, h) = engine.rasterize(&layout, color);
        self.labels.insert(key, CachedLabel { pixels, w, h });
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_left(
        &mut self,
        frame: &mut Frame,
        engine: &mut LayoutEngine,
        text: &str,
        x: f32,
        mid_y: f32,
        color: [u8; 3],
        scale: f32,
    ) -> f32 {
        self.ensure_label(engine, text, color, scale);
        let key = label_key(scale, text, color);
        let Some(l) = self.labels.get(&key) else {
            return 0.0;
        };
        let y = mid_y - l.h as f32 / 2.0;
        frame.blit(&l.pixels, l.w, l.h, x.round() as i32, y.round() as i32);
        l.w as f32
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_right(
        &mut self,
        frame: &mut Frame,
        engine: &mut LayoutEngine,
        text: &str,
        right_x: f32,
        mid_y: f32,
        color: [u8; 3],
        scale: f32,
    ) -> f32 {
        self.ensure_label(engine, text, color, scale);
        let key = label_key(scale, text, color);
        let Some(l) = self.labels.get(&key) else {
            return 0.0;
        };
        let x = right_x - l.w as f32;
        let y = mid_y - l.h as f32 / 2.0;
        frame.blit(&l.pixels, l.w, l.h, x.round() as i32, y.round() as i32);
        l.w as f32
    }

    /// 绘制全部 chrome（内容之后调用，不透明覆盖）。返回窗口宽高无关，仅按 vp 推导。
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        frame: &mut Frame,
        engine: &mut LayoutEngine,
        vp: Viewport,
        scale: f32,
        theme: &Theme,
        path: Option<&Path>,
        dirty: bool,
        external: ExternalState,
        line: usize,
        col: usize,
        doc_h: f32,
        scroll: f32,
    ) {
        let win_w = frame.width as f32;
        let win_h = frame.height as f32;
        let menu_h = MENU_H * scale;
        let status_h = STATUS_H * scale;

        // 菜单栏
        frame.fill_rect(0, 0, frame.width, menu_h.ceil() as u32, theme.chrome_bg);
        for (i, x, w) in menu_bar_rects(scale) {
            if self.open == Some(i) || self.hover_menu == Some(i) {
                frame.fill_rect(
                    x.round() as i32,
                    1,
                    w.ceil() as u32,
                    (menu_h - 1.0).max(1.0) as u32,
                    theme.hover,
                );
            }
            self.draw_left(
                frame,
                engine,
                MENUS[i].title,
                x + 14.0 * scale,
                menu_h / 2.0,
                theme.text,
                scale,
            );
        }
        frame.fill_rect(0, menu_h as i32 - 1, frame.width, 1, theme.chrome_border);

        // 状态栏
        let status_y = win_h - status_h;
        frame.fill_rect(
            0,
            status_y as i32,
            frame.width,
            status_h.ceil() as u32,
            theme.chrome_bg,
        );
        frame.fill_rect(0, status_y as i32, frame.width, 1, theme.chrome_border);
        let mid = status_y + status_h / 2.0;
        let (left, right) = status_line(path, dirty, external, line, col);
        self.draw_left(frame, engine, &left, 12.0 * scale, mid, theme.muted, scale);
        self.draw_right(
            frame,
            engine,
            &right,
            win_w - SCROLL_W * scale - 12.0 * scale,
            mid,
            theme.muted,
            scale,
        );
        if !self.notice.is_empty() {
            let notice = self.notice.clone();
            self.ensure_label(engine, &notice, theme.muted, scale);
            let key = label_key(scale, &notice, theme.muted);
            if let Some(l) = self.labels.get(&key) {
                let x = (win_w - l.w as f32) / 2.0;
                let y = mid - l.h as f32 / 2.0;
                frame.blit(&l.pixels, l.w, l.h, x.round() as i32, y.round() as i32);
            }
        }

        // 滚动条
        if let Some((ty, th)) = scroll_thumb(vp, doc_h, scroll, scale) {
            let x = win_w - SCROLL_W * scale;
            frame.fill_rect(
                x.round() as i32,
                vp.y as i32,
                (SCROLL_W * scale).ceil() as u32,
                vp.h.ceil() as u32,
                theme.chrome_bg,
            );
            frame.fill_rect(
                (x + 2.0 * scale).round() as i32,
                ty.round() as i32,
                ((SCROLL_W - 4.0) * scale).max(1.0) as u32,
                th.max(1.0) as u32,
                theme.thumb,
            );
        }

        // 弹出菜单（最后画：压过菜单栏下缘与内容）
        if let Some(open) = self.open {
            let p = popup_layout(open, scale);
            frame.fill_rect(
                p.x.round() as i32,
                p.y.round() as i32,
                p.w.ceil() as u32,
                p.h.ceil() as u32,
                theme.popup_bg,
            );
            frame.fill_rect(
                p.x.round() as i32,
                p.y.round() as i32,
                p.w.ceil() as u32,
                1,
                theme.chrome_border,
            );
            frame.fill_rect(
                (p.x + p.w).round() as i32 - 1,
                p.y.round() as i32,
                1,
                p.h.ceil() as u32,
                theme.chrome_border,
            );
            frame.fill_rect(
                p.x.round() as i32,
                (p.y + p.h).round() as i32 - 1,
                p.w.ceil() as u32,
                1,
                theme.chrome_border,
            );
            for (i, ix, iy, iw, ih) in &p.items {
                if self.hover_item == Some((open, *i)) {
                    frame.fill_rect(
                        ix.round() as i32,
                        iy.round() as i32,
                        iw.ceil() as u32,
                        ih.ceil() as u32,
                        theme.hover,
                    );
                }
                let (label, shortcut, action) = MENUS[open].items[*i];
                let pad = 12.0 * scale;
                self.draw_left(frame, engine, label, ix + pad, iy + ih / 2.0, theme.text, scale);
                // 主题项右侧显示「当前」标记（快捷键列复用；菜单最小宽度足以容纳）
                let hint = if theme_action_matches(action, theme.mode) {
                    "当前"
                } else {
                    shortcut
                };
                if !hint.is_empty() {
                    self.draw_right(
                        frame,
                        engine,
                        hint,
                        ix + iw - pad,
                        iy + ih / 2.0,
                        theme.muted,
                        scale,
                    );
                }
            }
        }
    }
}

/// 主题菜单项与当前模式是否一致（用于「当前」标记；非主题项恒 false）。
fn theme_action_matches(action: MenuAction, mode: ThemeMode) -> bool {
    matches!(
        (action, mode),
        (MenuAction::ThemeSystem, ThemeMode::System)
            | (MenuAction::ThemeLight, ThemeMode::Light)
            | (MenuAction::ThemeDark, ThemeMode::Dark)
    )
}

fn label_key(scale: f32, text: &str, color: [u8; 3]) -> String {
    format!(
        "{scale:?}|{:02x}{:02x}{:02x}|{text}",
        color[0], color[1], color[2]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Viewport {
        viewport(900, 700, 1.0)
    }

    #[test]
    fn viewport_excludes_menu_and_status() {
        let v = vp();
        assert_eq!(v.y, MENU_H);
        assert_eq!(v.h, 700.0 - MENU_H - STATUS_H);
        assert_eq!(v.w, 900.0);
    }

    #[test]
    fn menu_bar_hit_and_toggle_zones() {
        let rects = menu_bar_rects(1.0);
        assert_eq!(rects.len(), MENUS.len());
        assert_eq!(rects.len(), 3, "文件/编辑/视图");
        let (_, x0, w0) = rects[0];
        let (_, x1, _) = rects[1];
        let (_, x2, _) = rects[2];
        assert_eq!(hit_menu_bar(x0 + 1.0, 10.0, 1.0), Some(0));
        assert_eq!(hit_menu_bar(x1 + 1.0, 10.0, 1.0), Some(1));
        assert_eq!(hit_menu_bar(x2 + 1.0, 10.0, 1.0), Some(2));
        assert_eq!(
            hit_menu_bar(x0 + w0 - 0.1, 10.0, 1.0),
            Some(0),
            "右缘闭区间外"
        );
        assert_eq!(hit_menu_bar(x2 + 1000.0, 10.0, 1.0), None);
        assert_eq!(
            hit_menu_bar(x0 + 1.0, MENU_H + 1.0, 1.0),
            None,
            "菜单栏以下"
        );
    }

    #[test]
    fn popup_hit_maps_items_and_contains_padding() {
        let p = popup_layout(0, 1.0);
        assert!(p.h > 0.0);
        assert_eq!(p.y, MENU_H);
        for (i, ix, iy, iw, ih) in &p.items {
            assert_eq!(
                hit_popup_item(ix + iw / 2.0, iy + ih / 2.0, 0, 1.0),
                Some(*i)
            );
        }
        assert_eq!(
            hit_popup_item(p.x + 1.0, p.y + 1.0, 0, 1.0),
            None,
            "内边距非项区"
        );
        assert!(popup_contains(p.x + 1.0, p.y + 1.0, 0, 1.0));
        assert!(!popup_contains(p.x - 1.0, p.y + 1.0, 0, 1.0));
        assert_eq!(action_at(0, 0), Some(MenuAction::Open));
        assert_eq!(action_at(0, 99), None);
    }

    #[test]
    fn menu_definitions_cover_shortcuts() {
        let file: Vec<_> = MENUS[0].items.iter().map(|(_, _, a)| *a).collect();
        assert_eq!(
            file,
            vec![
                MenuAction::Open,
                MenuAction::Save,
                MenuAction::SaveAs,
                MenuAction::Export,
                MenuAction::Quit
            ]
        );
        let edit: Vec<_> = MENUS[1].items.iter().map(|(_, _, a)| *a).collect();
        assert_eq!(
            edit,
            vec![
                MenuAction::Undo,
                MenuAction::Redo,
                MenuAction::Cut,
                MenuAction::Copy,
                MenuAction::Paste
            ]
        );
    }

    #[test]
    fn scroll_thumb_none_without_overflow() {
        assert_eq!(scroll_thumb(vp(), 600.0, 0.0, 1.0), None);
        assert_eq!(scroll_thumb(vp(), 100.0, 0.0, 1.0), None);
    }

    #[test]
    fn scroll_thumb_position_and_mapping_roundtrip() {
        let v = vp();
        let doc = 3000.0;
        let (_, h) = scroll_thumb(v, doc, 0.0, 1.0).unwrap();
        assert!(h >= THUMB_MIN_H && h <= v.h);
        let (y0, _) = scroll_thumb(v, doc, 0.0, 1.0).unwrap();
        assert_eq!(y0, v.y);
        let (y1, _) = scroll_thumb(v, doc, max_scroll(v, doc), 1.0).unwrap();
        assert!((y1 - (v.y + v.h - h)).abs() < 0.01, "底部贴视口底");
        // 中位：滑块顶边 → 滚动量再回算一致
        let mid = (y0 + y1) / 2.0;
        let s = scroll_from_thumb(v, doc, mid, 1.0);
        let (ym, _) = scroll_thumb(v, doc, s, 1.0).unwrap();
        assert!((ym - mid).abs() < 0.01);
        assert!((s - max_scroll(v, doc) / 2.0).abs() < 0.5);
    }

    #[test]
    fn scroll_from_thumb_clamps() {
        let v = vp();
        let doc = 2000.0;
        assert_eq!(scroll_from_thumb(v, doc, -100.0, 1.0), 0.0);
        assert_eq!(
            scroll_from_thumb(v, doc, v.y + v.h + 100.0, 1.0),
            max_scroll(v, doc)
        );
        assert_eq!(scroll_from_thumb(v, 100.0, 50.0, 1.0), 0.0, "无溢出恒 0");
    }

    #[test]
    fn page_scroll_steps_and_clamps() {
        let v = vp();
        let doc = 5000.0;
        assert!(page_scroll(v, doc, 0.0, 1) > 0.0);
        assert_eq!(page_scroll(v, doc, 0.0, -1), 0.0);
        assert_eq!(
            page_scroll(v, doc, max_scroll(v, doc), 1),
            max_scroll(v, doc)
        );
        assert_eq!(page_scroll(v, 100.0, 0.0, 1), 0.0, "无溢出不滚动");
    }

    #[test]
    fn scrollbar_hit_requires_overflow_and_right_edge() {
        let v = vp();
        let x = v.x + v.w - SCROLL_W / 2.0;
        assert!(scrollbar_hit(v, 2000.0, x, v.y + 10.0, 1.0));
        assert!(!scrollbar_hit(v, 600.0, x, v.y + 10.0, 1.0), "不溢出不出现");
        assert!(
            !scrollbar_hit(v, 2000.0, x - SCROLL_W, v.y + 10.0, 1.0),
            "轨道左侧"
        );
        assert!(!scrollbar_hit(v, 2000.0, x, v.y - 1.0, 1.0), "视口上方");
    }

    #[test]
    fn status_line_variants() {
        assert_eq!(
            status_line(None, false, ExternalState::None, 1, 1),
            ("未命名草稿".into(), "行 1，列 1".into())
        );
        assert_eq!(
            status_line(Some(Path::new("dir/a.md")), true, ExternalState::BackedUp, 12, 3),
            ("dir/a.md *（外部修改已备份）".into(), "行 12，列 3".into())
        );
        assert_eq!(
            status_line(Some(Path::new("b.md")), false, ExternalState::None, 2, 7).0,
            "b.md"
        );
    }

    #[test]
    fn status_line_marks_external_states() {
        let p = Some(Path::new("a.md"));
        assert_eq!(
            status_line(p, false, ExternalState::Changed, 1, 1).0,
            "a.md（外部修改）"
        );
        assert_eq!(
            status_line(p, true, ExternalState::Missing, 1, 1).0,
            "a.md *（文件缺失）"
        );
    }

    #[test]
    fn paint_draws_menu_status_and_popup() {
        let mut engine = LayoutEngine::new();
        let mut frame = Frame::new(300, 200, [255, 255, 255, 255]);
        let theme = Theme::light();
        let mut chrome = Chrome {
            open: Some(0),
            ..Default::default()
        };
        let v = viewport(300, 200, 1.0);
        chrome.paint(
            &mut frame,
            &mut engine,
            v,
            1.0,
            &theme,
            None,
            false,
            ExternalState::None,
            1,
            1,
            2000.0,
            0.0,
        );
        assert_eq!(&frame.pixels[0..4], &theme.chrome_bg, "菜单栏底色");
        let status_y = 200 - STATUS_H as i32;
        let i = ((status_y + 5) as u32 * 300) as usize * 4;
        assert_eq!(&frame.pixels[i..i + 4], &theme.chrome_bg, "状态栏底色");
        let p = popup_layout(0, 1.0);
        let i = ((p.y + p.h / 2.0) as u32 * 300 + (p.x + p.w / 2.0) as u32) as usize * 4;
        assert_eq!(&frame.pixels[i..i + 4], &theme.popup_bg, "弹窗底色");
        assert!(!chrome.labels.is_empty());
        // 缓存复用：同帧重复绘制不增长
        let n = chrome.labels.len();
        chrome.paint(
            &mut frame,
            &mut engine,
            v,
            1.0,
            &theme,
            None,
            false,
            ExternalState::None,
            1,
            1,
            2000.0,
            0.0,
        );
        assert_eq!(chrome.labels.len(), n);
    }

    #[test]
    fn paint_uses_dark_palette() {
        let mut engine = LayoutEngine::new();
        let mut frame = Frame::new(300, 200, [1, 2, 3, 255]);
        let theme = Theme::dark();
        let mut chrome = Chrome::default();
        chrome.paint(
            &mut frame,
            &mut engine,
            viewport(300, 200, 1.0),
            1.0,
            &theme,
            None,
            false,
            ExternalState::None,
            1,
            1,
            10.0,
            0.0,
        );
        assert_eq!(&frame.pixels[0..4], &theme.chrome_bg, "深色菜单栏");
        assert_eq!(theme.chrome_bg, [38, 38, 38, 255]);
    }

    #[test]
    fn theme_menu_marks_current_mode() {
        assert_eq!(MENUS.len(), 3);
        assert_eq!(action_at(2, 0), Some(MenuAction::ThemeSystem));
        assert_eq!(action_at(2, 1), Some(MenuAction::ThemeLight));
        assert_eq!(action_at(2, 2), Some(MenuAction::ThemeDark));
        assert!(theme_action_matches(MenuAction::ThemeSystem, ThemeMode::System));
        assert!(theme_action_matches(MenuAction::ThemeDark, ThemeMode::Dark));
        assert!(!theme_action_matches(MenuAction::ThemeDark, ThemeMode::Light));
        assert!(!theme_action_matches(MenuAction::Save, ThemeMode::Dark));
    }

    #[test]
    fn scrollbar_thumb_painted_when_overflowing() {
        let mut engine = LayoutEngine::new();
        let mut frame = Frame::new(300, 200, [255, 255, 255, 255]);
        let theme = Theme::light();
        let mut chrome = Chrome::default();
        let v = viewport(300, 200, 1.0);
        chrome.paint(
            &mut frame,
            &mut engine,
            v,
            1.0,
            &theme,
            None,
            false,
            ExternalState::None,
            1,
            1,
            3000.0,
            0.0,
        );
        let x = (300.0 - SCROLL_W + 2.0) as u32;
        let y = (v.y + 2.0) as u32;
        let i = (y * 300 + x) as usize * 4;
        assert_eq!(&frame.pixels[i..i + 4], &theme.thumb);
    }
}
