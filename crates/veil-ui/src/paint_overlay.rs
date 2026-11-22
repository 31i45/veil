use crate::app::App;
use crate::chrome;
use crate::geometry::{Rect, MARGIN};
use crate::i18n::{t, tf, Key};
use crate::paint::{
    caret_visual, link_tip_max_text_w, link_tip_rect, truncate_link_tip, CaretVisual,
    DROP_DIM_ALPHA,
};
use crate::view::{ctx_menu, outline, quick_open, search, BlockLayout, Frame, LayoutEngine};
use veil_core::Range;

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

impl App {
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
    pub(crate) fn paint_modal(&mut self, frame: &mut Frame) {
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
    pub(crate) fn paint_settings(&mut self, frame: &mut Frame) {
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
            let Some((label, action)) = items.get(*i) else {
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
            let color = if ctx_menu::action_enabled(*action) {
                theme.text
            } else {
                theme.muted
            };
            if let Some(lab) =
                self.ui_state
                    .chrome
                    .labels
                    .get_or_insert(&mut self.engine, t(*label), color, scale)
            {
                let y = (iy + l.item_h / 2.0 - lab.h as f32 / 2.0).round() as i32;
                frame.blit(&lab.pixels, lab.w, lab.h, (l.x + l.pad).round() as i32, y);
            }
            if let Some(shortcut) = ctx_menu::shortcut(*action) {
                if let Some(lab) = self.ui_state.chrome.labels.get_or_insert(
                    &mut self.engine,
                    t(shortcut),
                    theme.muted,
                    scale,
                ) {
                    let x = (l.x + l.w - l.pad - lab.w as f32).round() as i32;
                    let y = (iy + l.item_h / 2.0 - lab.h as f32 / 2.0).round() as i32;
                    frame.blit(&lab.pixels, lab.w, lab.h, x, y);
                }
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
        if cy < vp.y {
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
        self.window_state.scroll_y = self.window_state.goal_y;
    }
}
