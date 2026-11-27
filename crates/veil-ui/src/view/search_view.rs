//! 查找视图层（P3-3）：把 `search_strategy` 产出的浮栏几何落成帧像素。
//!
//! 本模块只负责「显示」：查询/替换串按可用宽度截断（同时供 IME 候选框定位），
//! 以及浮栏卡片/焦点底色/按钮态/字形/计数/命中刻度的实际绘制。
//!
//! 不负责：匹配扫描与命中步进、浮栏几何与命中判定、按键路由、滚动与文档真值——
//! 前者在 `search_strategy`，交互在 interaction。这里只读状态、不重算布局。

use crate::app::App;
use crate::geometry::Rect;
use crate::i18n::{t, Key};
use crate::tokens::typography;
use crate::view::{search, Frame};

/// chrome 近似文本宽度：CJK 全角按字号、ASCII 按 0.6 字号估算。
/// 只用于截断与 IME 候选框定位，不参与文本光栅（与 outline 同一取舍）。
pub(crate) fn text_width(text: &str, scale: f32) -> f32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                typography::UI_SIZE * 0.6 * scale
            } else {
                typography::UI_SIZE * scale
            }
        })
        .sum()
}

/// 查询串按可用宽度截断：保留尾部（正在输入的一端），超出补前导 `…`。
pub(crate) fn fit_query(text: &str, max_w: f32, scale: f32) -> String {
    if text_width(text, scale) <= max_w {
        return text.to_string();
    }
    let avail = (max_w - typography::UI_SIZE * scale).max(0.0);
    let mut tail: Vec<char> = Vec::new();
    let mut w = 0.0;
    for c in text.chars().rev() {
        let cw = if c.is_ascii() {
            typography::UI_SIZE * 0.6 * scale
        } else {
            typography::UI_SIZE * scale
        };
        if w + cw > avail {
            break;
        }
        w += cw;
        tail.push(c);
    }
    tail.reverse();
    let mut out = String::from("…");
    out.extend(tail);
    out
}

impl App {
    /// 查找栏（P3-3）：一张卡片（唯一一条边界线）→ 焦点字段底色 → 按钮
    /// hover/active 底色 → 查询文本/IME → 计数 → 按钮字形。替换模式（Ctrl+H）
    /// 在下方多一行替换输入；两行之间只用间距分组，不画任何内部分隔线（S3）。
    /// 栏不透明覆盖内容；标签复用 chrome 位图缓存（随主题失效）。
    pub(crate) fn paint_search(&mut self, frame: &mut Frame) {
        let Some(l) = self.search_rect() else {
            return;
        };
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let search = &self.ui_state.search;
        let replace = search.replace_open;
        // 过渡目标与 animate 侧同源：pressed 优先，其次 hover。
        let fade = search.fade.view(
            search
                .pressed
                .or(search.hover)
                .and_then(search::Hit::anim_index),
        );
        let rl = replace.then(|| search::replace_layout(&l, scale));
        let bar = l.bar;
        let bx = bar.x.round() as i32;
        let by = bar.y.round() as i32;
        let bw = bar.w.ceil() as u32;
        let bottom = rl.map_or(bar.y + bar.h, |r| r.row.y + r.row.h).round() as i32;
        let total_h = (bottom - by).max(1) as u32;
        frame.fill_surface_card(
            bx,
            by,
            bw,
            total_h,
            scale,
            theme.dark,
            crate::tokens::palette::surface_popup(&theme),
            theme.chrome_border,
        );
        // S4 focus：焦点字段用安静底色表达，外加与背景保持 ≥3:1 的 1px 焦点环
        // （深色主题 accent 直接压在 popup 上会被吞掉，由 modal 的同一色源回退）。
        let popup = crate::tokens::palette::surface_popup(&theme);
        let ring = crate::view::modal::focus_ring_color(&theme, [popup[0], popup[1], popup[2]]);
        let field_radius = (crate::theme::surface::corner_radius(scale) - 2.0 * scale).max(0.0);
        let paint_field = |frame: &mut Frame, r: Rect| {
            let x = r.x.round() as i32;
            let y = r.y.round() as i32;
            let w = r.w.ceil() as u32;
            let h = r.h.ceil() as u32;
            let t = scale.round().max(1.0) as u32;
            if w > 2 * t && h > 2 * t {
                let radius_inner = (field_radius - t as f32).max(0.0);
                frame.fill_rounded_rect(x, y, w, h, field_radius, [ring[0], ring[1], ring[2], 255]);
                // 先把环内铺回不透明 popup，再叠半透明焦点底色：
                // 否则 focus_bg 会与环色混合，字段比静息底色更暗。
                frame.fill_rounded_rect(
                    x + t as i32,
                    y + t as i32,
                    w - 2 * t,
                    h - 2 * t,
                    radius_inner,
                    popup,
                );
                frame.fill_rounded_rect(
                    x + t as i32,
                    y + t as i32,
                    w - 2 * t,
                    h - 2 * t,
                    radius_inner,
                    crate::tokens::state::focus_bg(&theme),
                );
            }
        };
        if search.focus == search::Field::Query {
            paint_field(frame, l.text);
        }
        if let Some(r) = rl {
            if search.focus == search::Field::Replace {
                paint_field(frame, r.text);
            }
        }
        // S4 hover / active：六态矩阵是唯一色源。
        let mut buttons = vec![
            (search::Hit::Prev, l.prev),
            (search::Hit::Next, l.next),
            (search::Hit::Close, l.close),
            (search::Hit::ToggleCase, l.case),
            (search::Hit::ToggleRegex, l.regex),
        ];
        if let Some(r) = rl {
            buttons.push((search::Hit::ReplaceOne, r.one));
            buttons.push((search::Hit::ReplaceAll, r.all));
        }
        for (idx, (hit, r)) in buttons.into_iter().enumerate() {
            // active（按下）即时到位；hover 走过渡不透明度（S6）。
            let color = if search.pressed == Some(hit) {
                Some(crate::tokens::state::pressed_bg(&theme))
            } else {
                let h = crate::tokens::state::hover_bg(&theme);
                fade.alpha_of(idx).map(|a| [h[0], h[1], h[2], a])
            };
            let Some(color) = color else {
                continue;
            };
            frame.fill_hover(
                r.x.round() as i32,
                r.y.round() as i32,
                r.w.ceil() as u32,
                r.h.ceil() as u32,
                scale,
                color,
            );
        }
        let mid = bar.y + bar.h / 2.0;
        let text_y = |h: u32| (mid - h as f32 / 2.0).round() as i32;
        // 计数并入输入区右侧：查询文本可用宽度先扣除计数占位。
        let count = search.label();
        let count_w = if count.is_empty() {
            0.0
        } else {
            search::COUNT_W * scale
        };
        let query_avail = (l.text.w - count_w).max(0.0);
        // 查询文本：空串显示占位提示；非空保留尾部（正在输入的一端）
        // 光标 x 只跟随真实查询文本（占位提示不计入）：空串时落在输入区左端，
        // 有文本时落在文本末端——两者共同表达「插入点」，占位文案不参与定位。
        let mut qw = 0.0;
        if search.query.is_empty() {
            if self.editor.preedit().is_none() {
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    t(Key::SearchPlaceholder),
                    theme.muted,
                    scale,
                ) {
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        l.text.x.round() as i32,
                        text_y(lab.h),
                    );
                }
            }
        } else {
            let shown = search::fit_query(&search.query, query_avail, scale);
            // 非法正则：查询文本用危险色提示（计数区同时显示「正则错误」）。
            let color = if search.regex_error {
                theme.danger
            } else {
                theme.text
            };
            if let Some(lab) =
                self.ui_state
                    .chrome
                    .labels
                    .get_or_insert(&mut self.engine, &shown, color, scale)
            {
                qw = lab.w as f32;
                frame.blit(
                    &lab.pixels,
                    lab.w,
                    lab.h,
                    l.text.x.round() as i32,
                    text_y(lab.h),
                );
            }
        }
        let focus = search.focus;
        // 查询框光标：聚焦 + 无 IME preedit → 可见文本末端画 2×ch 竖线
        if focus == search::Field::Query && self.editor.preedit().is_none() {
            let cx = (l.text.x + qw).round() as i32;
            let cw = crate::theme::surface::caret_width(scale).round().max(1.0) as u32;
            let ch = crate::theme::surface::caret_height(scale);
            let cy = (mid - ch / 2.0).round() as i32;
            frame.fill_rect(
                cx,
                cy,
                cw,
                ch as u32,
                [theme.fg[0], theme.fg[1], theme.fg[2], 255],
            );
        }
        // IME 组合串：贴在查询文本之后（贴右缘钳位），下划线同文档 preedit；
        // 替换模式下焦点在替换框时由替换行负责绘制
        if focus == search::Field::Query || !replace {
            if let Some(pre) = self.editor.preedit() {
                let text = pre.text.clone();
                let pl = self.engine.layout_ui(&text);
                let (px, w, h) = self.engine.rasterize(&pl, theme.fg);
                let x0 = (l.text.x + qw).min((l.text.x + l.text.w - w as f32).max(l.text.x));
                let y0 = mid - h as f32 / 2.0;
                frame.blit(&px, w, h, x0.round() as i32, y0.round() as i32);
                frame.fill_rect(
                    x0.round() as i32,
                    (y0 + h as f32) as i32 - 1,
                    w,
                    1,
                    [theme.fg[0], theme.fg[1], theme.fg[2], 255],
                );
            }
        }
        // 替换行：文本框 → IME 组合串 → 两个按钮字形（焦点底色已在卡片后绘制）
        if let Some(r) = rl {
            let rmid = r.row.y + r.row.h / 2.0;
            let rtext_y = |h: u32| (rmid - h as f32 / 2.0).round() as i32;
            let mut rw = 0.0;
            if search.replace.is_empty() {
                if !(focus == search::Field::Replace && self.editor.preedit().is_some()) {
                    if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                        &mut self.engine,
                        t(Key::SearchReplacePlaceholder),
                        theme.muted,
                        scale,
                    ) {
                        frame.blit(
                            &lab.pixels,
                            lab.w,
                            lab.h,
                            r.text.x.round() as i32,
                            rtext_y(lab.h),
                        );
                    }
                }
            } else {
                let shown = search::fit_query(&search.replace, r.text.w, scale);
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    &shown,
                    theme.text,
                    scale,
                ) {
                    rw = lab.w as f32;
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        r.text.x.round() as i32,
                        rtext_y(lab.h),
                    );
                }
            }
            // 替换框光标：聚焦 + 无 IME preedit → 可见文本末端画 2×ch 竖线
            if focus == search::Field::Replace && self.editor.preedit().is_none() {
                let cx = (r.text.x + rw).round() as i32;
                let cw = crate::theme::surface::caret_width(scale).round().max(1.0) as u32;
                let ch = crate::theme::surface::caret_height(scale);
                let cy = (rmid - ch / 2.0).round() as i32;
                frame.fill_rect(
                    cx,
                    cy,
                    cw,
                    ch as u32,
                    [theme.fg[0], theme.fg[1], theme.fg[2], 255],
                );
            }
            if focus == search::Field::Replace {
                if let Some(pre) = self.editor.preedit() {
                    let text = pre.text.clone();
                    let pl = self.engine.layout_ui(&text);
                    let (px, w, h) = self.engine.rasterize(&pl, theme.fg);
                    let x0 = (r.text.x + rw).min((r.text.x + r.text.w - w as f32).max(r.text.x));
                    let y0 = rmid - h as f32 / 2.0;
                    frame.blit(&px, w, h, x0.round() as i32, y0.round() as i32);
                    frame.fill_rect(
                        x0.round() as i32,
                        (y0 + h as f32) as i32 - 1,
                        w,
                        1,
                        [theme.fg[0], theme.fg[1], theme.fg[2], 255],
                    );
                }
            }
            for (btn, glyph) in [
                (r.one, t(Key::SearchReplace)),
                (r.all, t(Key::SearchReplaceAll)),
            ] {
                let color = if search.matches.is_empty() {
                    crate::tokens::state::disabled_fg(&theme)
                } else {
                    theme.text
                };
                if let Some(lab) =
                    self.ui_state
                        .chrome
                        .labels
                        .get_or_insert(&mut self.engine, glyph, color, scale)
                {
                    let x = btn.x + (btn.w - lab.w as f32) / 2.0;
                    let y = btn.y + (btn.h - lab.h as f32) / 2.0;
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        x.round() as i32,
                        y.round() as i32,
                    );
                }
            }
        }
        // 计数「i/n」：右对齐于输入区右缘；无命中用中性色表达空态
        //（空态不是错误；danger 只留给非法正则等真错误）。
        if !count.is_empty() {
            let color = if search.no_matches() {
                crate::tokens::state::empty_fg(&theme)
            } else {
                theme.muted
            };
            if let Some(lab) =
                self.ui_state
                    .chrome
                    .labels
                    .get_or_insert(&mut self.engine, &count, color, scale)
            {
                let x = (l.count_right - lab.w as f32).round() as i32;
                frame.blit(&lab.pixels, lab.w, lab.h, x, text_y(lab.h));
            }
        }
        // 「区分大小写」开关字形：开启时用选中态前景（悬停底色另行绘制）。
        let case_color = if search.case_sensitive {
            crate::tokens::state::selected_fg(&theme)
        } else {
            theme.text
        };
        if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
            &mut self.engine,
            t(Key::SearchCaseSensitive),
            case_color,
            scale,
        ) {
            let x = l.case.x + (l.case.w - lab.w as f32) / 2.0;
            let y = l.case.y + (l.case.h - lab.h as f32) / 2.0;
            frame.blit(
                &lab.pixels,
                lab.w,
                lab.h,
                x.round() as i32,
                y.round() as i32,
            );
        }
        // 「正则模式」开关字形：开启时用选中态前景（与 Aa 同一表达）。
        let regex_color = if search.regex {
            crate::tokens::state::selected_fg(&theme)
        } else {
            theme.text
        };
        if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
            &mut self.engine,
            t(Key::SearchRegex),
            regex_color,
            scale,
        ) {
            let x = l.regex.x + (l.regex.w - lab.w as f32) / 2.0;
            let y = l.regex.y + (l.regex.h - lab.h as f32) / 2.0;
            frame.blit(
                &lab.pixels,
                lab.w,
                lab.h,
                x.round() as i32,
                y.round() as i32,
            );
        }
        // 查询行按钮字形（居中）：无命中时翻页按钮为禁用前景。
        let page_color = if search.matches.is_empty() {
            crate::tokens::state::disabled_fg(&theme)
        } else {
            theme.text
        };
        for (r, glyph, color) in [
            (l.prev, "‹", page_color),
            (l.next, "›", page_color),
            (l.close, "×", theme.text),
        ] {
            if let Some(lab) =
                self.ui_state
                    .chrome
                    .labels
                    .get_or_insert(&mut self.engine, glyph, color, scale)
            {
                let x = r.x + (r.w - lab.w as f32) / 2.0;
                let y = r.y + (r.h - lab.h as f32) / 2.0;
                frame.blit(
                    &lab.pixels,
                    lab.w,
                    lab.h,
                    x.round() as i32,
                    y.round() as i32,
                );
            }
        }
    }

    /// L2 命中刻度：查找打开且有命中时，在主滚动条轨道标出命中位置。
    /// 只作位置提示，不改变滚动/命中语义；按视口像素高度采样，避免长文档
    /// 上千命中逐条绘制；当前命中最后绘制并用 accent 强调。
    pub(crate) fn paint_search_ticks(&mut self, frame: &mut Frame, vp: crate::chrome::Viewport) {
        let search = &self.ui_state.search;
        if !search.open || search.matches.is_empty() {
            return;
        }
        let total = self.editor.session().text_len().max(1);
        let scale = self.input_state.scale;
        let theme = self.input_state.theme;
        let lane = (crate::chrome::SCROLL_W * scale).ceil() as u32;
        let w = lane.saturating_sub((6.0 * scale) as u32).max(1);
        let x = (frame.width as f32 - lane as f32 + 3.0 * scale).round() as i32;
        let h = (2.0 * scale).round().max(1.0) as u32;
        let tick_y = |start: usize| {
            let t = start.min(total) as f32 / total as f32;
            (vp.y + t * vp.h - h as f32 / 2.0).round() as i32
        };
        let stride = (search.matches.len() as f32 / vp.h.max(1.0))
            .ceil()
            .max(1.0) as usize;
        for (i, m) in search.matches.iter().enumerate().step_by(stride) {
            if i == search.current {
                continue;
            }
            frame.fill_rect(x, tick_y(m.start), w, h, theme.thumb);
        }
        if let Some(m) = search.matches.get(search.current) {
            let a = theme.accent;
            frame.fill_rect(x, tick_y(m.start), w, h, [a[0], a[1], a[2], 255]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_query_keeps_short_and_ellipsizes_head() {
        assert_eq!(fit_query("abc", 200.0, 1.0), "abc");
        let long = "一个很长很长很长很长的查询字符串";
        let out = fit_query(long, 60.0, 1.0);
        assert!(out.starts_with('…'), "保留尾部须加前导省略号: {out}");
        assert!(out.chars().count() < long.chars().count());
        // 极窄区域仍返回省略号（不 panic）
        assert_eq!(fit_query(long, 1.0, 1.0), "…");
    }
}
