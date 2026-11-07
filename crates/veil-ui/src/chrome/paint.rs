//! chrome 绘制：菜单栏、状态栏、滚动条与弹出菜单。
//! 输入为模型层（super::*）的几何/命中与 cache 的标签位图，自身不做命中判定。

use std::path::Path;

use super::{
    action_checked, action_enabled, menu_bar_rects, menu_rows, popup_layout, scroll_thumb,
    status_line, theme_action_matches, Chrome, MenuFlags, Viewport, MENUS, MENU_H, SCROLL_W,
    STATUS_H,
};
use crate::document::ExternalState;
use crate::theme::Theme;
use crate::view::{Frame, LayoutEngine};

/// 文本排版宽度：preedit 排版后取文末光标的 x（单行 = 总宽）。
/// 纯测量路径，不经标签缓存（S1）。
pub(super) fn measure_w(engine: &mut LayoutEngine, text: &str) -> f32 {
    let layout = engine.layout_preedit(text);
    engine.caret_xy(&layout, layout.layout_text().len()).0
}

/// 中段省略（S1）：放不下时二分保留字符数，拼 `头…尾`。
/// head/tail 随保留数单调扩增、候选宽度单调不减，二分收敛；
/// keep = 全长时候选为「全文插省略号」必超宽，上界不变量成立。
pub(super) fn ellipsize_middle(engine: &mut LayoutEngine, text: &str, avail: f32) -> String {
    if measure_w(engine, text) <= avail {
        return text.to_string();
    }
    let offs: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let n = offs.len();
    let build = |keep: usize| -> String {
        let head = keep / 2;
        let tail = keep - head; // 尾 >= 头：文件名/扩展名优先可见
        let tail_start = if tail == 0 { text.len() } else { offs[n - tail] };
        format!("{}…{}", &text[..offs[head]], &text[tail_start..])
    };
    let (mut lo, mut hi) = (0usize, n); // build(lo) 可放下、build(hi) 放不下
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if measure_w(engine, &build(mid)) <= avail {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    build(lo)
}

impl Chrome {
    /// 状态栏左文适配（S1）：超宽时中段省略为 head…tail（尾多留，路径文件名优先可见）。
    /// 结果按 (原文, 可用宽, scale) 缓存；测量走 layout_preedit，不污染标签缓存。
    fn fit_middle(&mut self, engine: &mut LayoutEngine, text: &str, avail: f32, scale: f32) -> String {
        let key = (text.to_string(), avail.to_bits(), scale.to_bits());
        if let Some(hit) = self.status_fit.get(&key) {
            return hit.clone();
        }
        let fitted = ellipsize_middle(engine, text, avail);
        self.status_fit.insert(key, fitted.clone());
        fitted
    }

    #[allow(clippy::too_many_arguments)] // 内部绘制函数参数天然多
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
        let Some(l) = self.labels.get_or_insert(engine, text, color, scale) else {
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
        let Some(l) = self.labels.get_or_insert(engine, text, color, scale) else {
            return 0.0;
        };
        let x = right_x - l.w as f32;
        let y = mid_y - l.h as f32 / 2.0;
        frame.blit(&l.pixels, l.w, l.h, x.round() as i32, y.round() as i32);
        l.w as f32
    }

    /// 绘制全部 chrome（内容之后调用，不透明覆盖）。返回窗口宽高无关，仅按 vp 推导。
    /// `recent` 为最近文件标签（0b 动态菜单项；与命中/分发共用同一份）。
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
        words: usize,
        line: usize,
        col: usize,
        doc_h: f32,
        scroll: f32,
        recent: &[String],
        flags: MenuFlags,
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
        let (left, right) = status_line(path, dirty, external, flags.source, words, line, col);
        // S1：右侧字数行列优先完整固定，左文超宽时中段省略（head…tail），杜绝左右重叠。
        let right_edge = win_w - SCROLL_W * scale - 12.0 * scale;
        let left_x = 12.0 * scale;
        let right_w = measure_w(engine, &right);
        let avail = right_edge - right_w - left_x - 12.0 * scale;
        let left_fitted = self.fit_middle(engine, &left, avail, scale);
        let left_w = measure_w(engine, &left_fitted);
        self.draw_left(frame, engine, &left_fitted, left_x, mid, theme.muted, scale);
        self.draw_right(frame, engine, &right, right_edge, mid, theme.muted, scale);
        if !self.notice.is_empty() {
            // 通知只在中段空白内绘制：避让左右文本，空间不足则直接不画
            let gap = crate::theme::surface::control_gap(scale);
            let x0 = left_x + left_w + gap;
            let x1 = right_edge - right_w - gap;
            let avail_notice = x1 - x0;
            if avail_notice >= 24.0 * scale {
                let notice = self.notice.clone();
                let fitted = self.fit_middle(engine, &notice, avail_notice, scale);
                if let Some(l) = self.labels.get_or_insert(engine, &fitted, theme.muted, scale) {
                    let x = x0 + (avail_notice - l.w as f32) / 2.0;
                    let y = mid - l.h as f32 / 2.0;
                    frame.blit(&l.pixels, l.w, l.h, x.round() as i32, y.round() as i32);
                }
            }
        }

        // 滚动条：胶囊滑块；悬停/拖动时 alpha 由 170 → 255 加亮（深浅主题都可见），
        // 尺寸不变避免抖动。最小高度由 scroll_thumb 的 THUMB_MIN_H 保证。
        if let Some((ty, th)) = scroll_thumb(vp, doc_h, scroll, scale) {
            let x = win_w - SCROLL_W * scale;
            frame.fill_rect(
                x.round() as i32,
                vp.y as i32,
                (SCROLL_W * scale).ceil() as u32,
                vp.h.ceil() as u32,
                theme.chrome_bg,
            );
            let tw = ((SCROLL_W - 4.0) * scale).max(1.0);
            let active = self.scroll_hover || self.scroll_drag.is_some();
            let alpha = if active { 255 } else { 170 };
            frame.fill_rounded_rect(
                (x + 2.0 * scale).round() as i32,
                ty.round() as i32,
                tw.ceil() as u32,
                th.max(1.0) as u32,
                tw / 2.0,
                [theme.thumb[0], theme.thumb[1], theme.thumb[2], alpha],
            );
        }

        // 弹出菜单（最后画：压过菜单栏下缘与内容）
        if let Some(open) = self.open {
            let rows = menu_rows(open, recent);
            let p = popup_layout(open, scale, recent);
            frame.fill_surface_card(
                p.x.round() as i32,
                p.y.round() as i32,
                p.w.ceil() as u32,
                p.h.ceil() as u32,
                scale,
                theme.dark,
                theme.popup_bg,
                theme.chrome_border,
            );
            let pad = crate::theme::surface::padding(scale);
            // 分隔线：左右内缩与菜单项文字起止对齐（不顶到弹层边框）
            for (_, sy) in &p.separators {
                frame.fill_rect(
                    (p.x + pad).round() as i32,
                    sy.round() as i32,
                    (p.w - 2.0 * pad).max(1.0) as u32,
                    1,
                    theme.chrome_border,
                );
            }
            for (i, ix, iy, iw, ih) in &p.items {
                let Some((label, shortcut, action)) = rows.get(*i) else {
                    continue;
                };
                let enabled = action_enabled(*action, flags);
                // 禁用项不参与高亮（原生菜单语义：置灰且无悬停反馈）
                if enabled && self.hover_item == Some((open, *i)) {
                    frame.fill_hover(
                        ix.round() as i32,
                        iy.round() as i32,
                        iw.ceil() as u32,
                        ih.ceil() as u32,
                        scale,
                        theme.hover,
                    );
                }
                let label_color = if enabled { theme.text } else { theme.muted };
                self.draw_left(
                    frame,
                    engine,
                    label,
                    ix + pad,
                    iy + ih / 2.0,
                    label_color,
                    scale,
                );
                // 主题项 / 开关项：开启态在右侧显示「当前」（快捷键列复用）
                let hint = if theme_action_matches(*action, theme.mode)
                    || action_checked(*action, flags)
                {
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
