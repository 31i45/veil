//! chrome 绘制：菜单栏、状态栏、滚动条与弹出菜单。
//! 输入为模型层（super::*）的几何/命中与 cache 的标签位图，自身不做命中判定。

use std::path::Path;

use super::{
    action_checked, action_enabled, disabled_reason, menu_bar_rects, menu_rows, pick_status_tier,
    popup_layout, scroll_thumb, status_parts, status_stats, status_words, theme_action_matches,
    Chrome, MenuFlags, StatusTier, Viewport, MENUS, MENU_H, SCROLL_W, STATUS_H,
};
use crate::document::ExternalState;
use crate::i18n::{t, Key};
use crate::theme::Theme;
use crate::tokens::{palette, space, state, stroke};
use crate::view::{Frame, LayoutEngine};

/// 文本排版宽度：UI 字阶（13px）排版后取文末光标的 x（单行 = 总宽）。
/// 纯测量路径，不经标签缓存（S1）。
pub(super) fn measure_w(engine: &mut LayoutEngine, text: &str) -> f32 {
    let layout = engine.layout_ui(text);
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
        let tail_start = if tail == 0 {
            text.len()
        } else {
            offs[n - tail]
        };
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

/// 状态栏左右文本布局结果（像素宽为排版实测值）。
pub(super) struct StatusFit {
    pub left: String,
    /// 文件身份原文（通知压缩左文时复用）。
    pub identity: String,
    pub right: String,
    pub left_w: f32,
    pub right_w: f32,
}

/// 通知的落位方式。任何路径都必须真正绘制，不存在「直接不画」。
#[derive(Clone, Copy, PartialEq, Debug)]
enum NoticeSlot {
    /// 左右文本之间的空白：中心对齐。
    Middle { x0: f32, x1: f32 },
    /// 最小浮层：中段与右区都不够时，在状态栏上方铺一块带边框的提示条。
    Overlay,
}

impl Chrome {
    /// 状态栏左文适配（S1）：超宽时中段省略为 head…tail（尾多留，路径文件名优先可见）。
    /// 结果按 (原文, 可用宽, scale) 缓存；测量走 layout_ui，不污染标签缓存。
    fn fit_middle(
        &mut self,
        engine: &mut LayoutEngine,
        text: &str,
        avail: f32,
        scale: f32,
    ) -> String {
        let key = (text.to_string(), avail.to_bits(), scale.to_bits());
        if let Some(hit) = self.status_fit.get(&key) {
            return hit.clone();
        }
        let fitted = ellipsize_middle(engine, text, avail);
        self.status_fit.insert(key, fitted.clone());
        fitted
    }

    /// 状态栏左右文本布局（纯计算，不绘制）：按优先级丢低优先段，必要时让右文
    /// 退化为仅字数；文件身份与保存态始终保留，文件名可中段省略但不整体消失。
    #[allow(clippy::too_many_arguments)] // 状态栏聚合字段
    pub(super) fn layout_status(
        &mut self,
        engine: &mut LayoutEngine,
        path: Option<&Path>,
        dirty: bool,
        external: ExternalState,
        source_mode: bool,
        saved_hhmm: Option<&str>,
        words: usize,
        line: usize,
        col: usize,
        win_w: f32,
        scale: f32,
    ) -> StatusFit {
        let parts = status_parts(path, dirty, external, source_mode, saved_hhmm);
        let stats_full = status_stats(words, line, col);
        let stats_min = status_words(words);
        let right_edge = win_w - SCROLL_W * scale - space::M * scale;
        let left_x = space::M * scale;
        let gap = space::M * scale;
        let (full, no_src, core) = (parts.full(), parts.without_source(), parts.core());
        let w_full = measure_w(engine, &full);
        let w_no_src = measure_w(engine, &no_src);
        let stats_full_w = measure_w(engine, &stats_full);
        let stats_min_w = measure_w(engine, &stats_min);
        let avail_full = (right_edge - stats_full_w - left_x - gap).max(0.0);
        let avail_min = (right_edge - stats_min_w - left_x - gap).max(0.0);
        let tier = pick_status_tier(w_full, w_no_src, avail_full);
        // 只有连「保存态 + 文件名最少可读宽度」都放不下时，右文才退化为仅字数；
        // 否则保留完整行列，改为收缩文件名（700px 长路径场景）。
        let save_w = measure_w(engine, &parts.save);
        let (right, right_w, avail) = match tier {
            StatusTier::Full | StatusTier::NoSource => (stats_full, stats_full_w, avail_full),
            StatusTier::Core if avail_full >= save_w + space::XL * scale => {
                (stats_full, stats_full_w, avail_full)
            }
            StatusTier::Core => (stats_min, stats_min_w, avail_min),
        };
        let mut left = match tier {
            StatusTier::Full => full,
            StatusTier::NoSource => no_src,
            StatusTier::Core => core,
        };
        let mut left_w = measure_w(engine, &left);
        // 兜底：仍超宽时收缩文件名，保存态完整保留。
        if left_w > avail {
            let save_w = measure_w(engine, &parts.save);
            let room = (avail - save_w).max(0.0);
            let identity = self.fit_middle(engine, &parts.identity, room, scale);
            left = format!("{identity}{}", parts.save);
            left_w = measure_w(engine, &left);
            if left_w > avail {
                left = self.fit_middle(engine, &left, avail, scale);
                left_w = measure_w(engine, &left);
            }
        }
        StatusFit {
            left,
            identity: parts.identity,
            right,
            left_w,
            right_w,
        }
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
        saved_hhmm: Option<&str>,
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
        frame.fill_rect(
            0,
            0,
            frame.width,
            menu_h.ceil() as u32,
            palette::surface_card(theme),
        );
        let menu_fade = self.menu_fade.view(self.hover_menu);
        for (i, x, w) in menu_bar_rects(scale) {
            // active（已展开）用按下色，hover 用悬停色：六态单源。
            // hover 走过渡不透明度（S6），active 即时到位。
            let bg = if self.open == Some(i) {
                Some(state::pressed_bg(theme))
            } else if let Some(a) = menu_fade.alpha_of(i) {
                let h = state::hover_bg(theme);
                Some([h[0], h[1], h[2], a])
            } else {
                None
            };
            if let Some(color) = bg {
                frame.fill_rect(
                    x.round() as i32,
                    1,
                    w.ceil() as u32,
                    (menu_h - 1.0).max(1.0) as u32,
                    color,
                );
            }
            self.draw_left(
                frame,
                engine,
                t(MENUS[i].title),
                x + 14.0 * scale,
                menu_h / 2.0,
                theme.text,
                scale,
            );
        }
        frame.fill_rect(
            0,
            menu_h as i32 - 1,
            frame.width,
            stroke::HAIRLINE as u32,
            theme.chrome_border,
        );

        // 状态栏
        let status_y = win_h - status_h;
        frame.fill_rect(
            0,
            status_y as i32,
            frame.width,
            status_h.ceil() as u32,
            palette::surface_card(theme),
        );
        frame.fill_rect(
            0,
            status_y as i32,
            frame.width,
            stroke::HAIRLINE as u32,
            theme.chrome_border,
        );
        let mid = status_y + status_h / 2.0;
        let fit = self.layout_status(
            engine,
            path,
            dirty,
            external,
            flags.source,
            saved_hhmm,
            words,
            line,
            col,
            win_w,
            scale,
        );
        let StatusFit {
            mut left,
            identity,
            right,
            mut left_w,
            right_w,
        } = fit;
        let left_x = space::M * scale;
        let right_edge = win_w - SCROLL_W * scale - space::M * scale;
        let notice_gap = crate::theme::surface::control_gap(scale);
        let min_notice = space::XL * scale;
        let notice_text = self.notice.clone();

        // 通知落位预判：优先中段；不足则压缩左文；再不足占用右文区域；最后浮层。
        // 任何分支都必须能画出通知（不存在静默丢弃）。
        let mut slot = None;
        let mut draw_right = true;
        if !notice_text.is_empty() {
            let x0 = left_x + left_w + notice_gap;
            let x1 = right_edge - right_w - notice_gap;
            if x1 - x0 >= min_notice {
                slot = Some(NoticeSlot::Middle { x0, x1 });
            } else {
                // (a) 压缩左文到仅文件名：保存态暂时让位于通知。
                let room = (right_edge - right_w - notice_gap - left_x).max(0.0);
                left = self.fit_middle(engine, &identity, room, scale);
                left_w = measure_w(engine, &left);
                let x0 = left_x + left_w + notice_gap;
                let x1 = right_edge - right_w - notice_gap;
                if x1 - x0 >= min_notice {
                    slot = Some(NoticeSlot::Middle { x0, x1 });
                } else {
                    // (b) 临时占用右文区域：右文本次让位给通知。
                    let x0 = left_x + left_w + notice_gap;
                    let x1 = right_edge;
                    if x1 - x0 >= min_notice {
                        draw_right = false;
                        slot = Some(NoticeSlot::Middle { x0, x1 });
                    } else {
                        // (c) 最小浮层：状态栏上方的一条提示带（右文保留不重叠）。
                        slot = Some(NoticeSlot::Overlay);
                    }
                }
            }
        }

        self.draw_left(
            frame,
            engine,
            &left,
            left_x,
            mid,
            state::disabled_fg(theme),
            scale,
        );
        if draw_right {
            self.draw_right(
                frame,
                engine,
                &right,
                right_edge,
                mid,
                state::disabled_fg(theme),
                scale,
            );
        }
        match slot {
            Some(NoticeSlot::Middle { x0, x1 }) => {
                let avail_notice = (x1 - x0).max(1.0);
                let fitted = self.fit_middle(engine, &notice_text, avail_notice, scale);
                if let Some(l) =
                    self.labels
                        .get_or_insert(engine, &fitted, state::disabled_fg(theme), scale)
                {
                    let x = x0 + (avail_notice - l.w as f32) / 2.0;
                    let y = mid - l.h as f32 / 2.0;
                    frame.blit(&l.pixels, l.w, l.h, x.round() as i32, y.round() as i32);
                }
            }
            Some(NoticeSlot::Overlay) => {
                let pad = space::M * scale;
                let ow = (win_w - 2.0 * pad).max(1.0);
                let oh = status_h;
                let oy = (status_y - oh).max(0.0);
                frame.fill_surface_card(
                    pad.round() as i32,
                    oy.round() as i32,
                    ow.ceil() as u32,
                    oh.ceil() as u32,
                    scale,
                    theme.dark,
                    palette::surface_popup(theme),
                    theme.chrome_border,
                );
                let inner = (ow - 2.0 * space::S * scale).max(1.0);
                let fitted = self.fit_middle(engine, &notice_text, inner, scale);
                self.draw_left(
                    frame,
                    engine,
                    &fitted,
                    pad + space::S * scale,
                    oy + oh / 2.0,
                    state::disabled_fg(theme),
                    scale,
                );
            }
            None => {}
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
                palette::surface_card(theme),
            );
            let tw = ((SCROLL_W - space::XS) * scale).max(1.0);
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
            let p = popup_layout(open, scale, recent, win_w);
            frame.fill_surface_card(
                p.x.round() as i32,
                p.y.round() as i32,
                p.w.ceil() as u32,
                p.h.ceil() as u32,
                scale,
                theme.dark,
                palette::surface_popup(theme),
                theme.chrome_border,
            );
            let pad = crate::theme::surface::padding(scale);
            let popup_fade = self.popup_fade.view(self.hover_item.map(|(_, i)| i));
            for (i, ix, iy, iw, ih) in &p.items {
                let Some((label, shortcut, action)) = rows.get(*i) else {
                    continue;
                };
                let enabled = action_enabled(*action, flags);
                // 禁用项不参与高亮（原生菜单语义：置灰且无悬停反馈）
                // 悬停高亮走过渡不透明度（S6）。
                if enabled {
                    if let Some(a) = popup_fade.alpha_of(*i) {
                        let h = state::hover_bg(theme);
                        frame.fill_hover(
                            ix.round() as i32,
                            iy.round() as i32,
                            iw.ceil() as u32,
                            ih.ceil() as u32,
                            scale,
                            [h[0], h[1], h[2], a],
                        );
                    }
                }
                // 主题项 / 开关项：开启态在右侧显示「当前」（快捷键列复用）。
                // 禁用项不用置灰了事：右侧改为 muted 的不可用原因，空间不足时截断。
                let is_current =
                    theme_action_matches(*action, theme.mode) || action_checked(*action, flags);
                let reason = if enabled {
                    None
                } else {
                    disabled_reason(*action, flags)
                };
                let hint = if is_current {
                    t(Key::MenuCurrent)
                } else if let Some(reason) = reason {
                    t(reason)
                } else {
                    shortcut
                };
                let inner = iw - 2.0 * pad;
                // 提示最多占行内 62%：长原因可截断，标签仍保持可读。
                let hint_cap = (inner * 0.62).max(0.0);
                let hint_shown: String = if hint.is_empty() {
                    String::new()
                } else if measure_w(engine, hint) > hint_cap && hint_cap > space::S * scale {
                    ellipsize_middle(engine, hint, hint_cap)
                } else {
                    hint.to_string()
                };
                let hint_w = if hint_shown.is_empty() {
                    0.0
                } else {
                    space::XL * scale + measure_w(engine, &hint_shown)
                };
                let avail = (inner - hint_w).max(0.0);
                let shown = if measure_w(engine, label) > avail {
                    ellipsize_middle(engine, label, avail)
                } else {
                    label.to_string()
                };
                let label_color = if enabled {
                    theme.text
                } else {
                    state::disabled_fg(theme)
                };
                self.draw_left(
                    frame,
                    engine,
                    &shown,
                    ix + pad,
                    iy + ih / 2.0,
                    label_color,
                    scale,
                );
                if !hint_shown.is_empty() {
                    self.draw_right(
                        frame,
                        engine,
                        &hint_shown,
                        ix + iw - pad,
                        iy + ih / 2.0,
                        if is_current {
                            state::selected_fg(theme)
                        } else {
                            state::disabled_fg(theme)
                        },
                        scale,
                    );
                }
            }
        }
    }
}
