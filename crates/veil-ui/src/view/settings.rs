//! 设置模态：自绘表单，承载 Config 全部字段的即时编辑。
//!
//! 独立于 modal.rs：后者只处理「静态文本 + OK/Cancel」类弹窗；设置表单有 10+ 个
//! 可编辑控件（radio / toggle / number input / dropdown / 只读展示），需要独立的
//! 布局、绘制和交互路径。
#![allow(
    dead_code,
    unused_variables,
    clippy::if_same_then_else,
    clippy::unnecessary_map_or
)]

use crate::geometry::Rect;
use crate::i18n::{t, Key};
use crate::input::Key as InputKey;
use crate::theme::{surface, Config, DocFontFamily, ThemeMode};
use crate::tokens::{palette, typography};
use crate::view::modal::ButtonRole;
use crate::view::modal::ModalButton;
use crate::view::modal::Text;
use crate::view::Frame;

/// 字段种类：决定渲染和键盘/鼠标交互。
#[derive(Clone, Debug)]
pub(crate) enum FieldKind {
    /// 主题模式：三选一 radio（跟随系统/浅色/深色）。
    ThemeMode,
    /// 数字输入（字号 10..=32 / 边距 0..=50 / tab 宽 2/4）。
    Number(NumberRange),
    /// 开关：布尔 toggle。
    Toggle,
    /// 字体族下拉：四选一（系统默认/无衬线/衬线/等宽）。
    FontFamily,
    /// 窗口几何：只读展示当前 width×height / x,y + "清除" 按钮。
    WindowGeom,
}

/// 数字输入的允许范围：夹取到合法值。
#[derive(Clone, Copy, Debug)]
pub(crate) struct NumberRange {
    pub min: f32,
    pub max: f32,
    pub step: f32,
    /// 如果为 true，只允许整数（tab_width 场景）。
    pub integers_only: bool,
}

/// 单个表单字段：规格 + 当前值 + 可选的临时值（数字输入框的打字缓冲）。
#[derive(Clone, Debug)]
pub(crate) struct SettingsField {
    pub(crate) label: Key,
    pub(crate) kind: FieldKind,
    /// 主值：radio / toggle / number / dropdown 的选中值。
    pub(crate) value: FieldValue,
    /// 数字输入框的正在编辑文本（打字中）；None 表示无缓冲，值直接同步。
    pub(crate) buffer: Option<String>,
    /// 只读展示字段：不能获得焦点、不能被编辑（如窗口几何）。
    pub(crate) read_only: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum FieldValue {
    ThemeMode(ThemeMode),
    Number(f32),
    Toggle(bool),
    FontFamily(DocFontFamily),
    /// 窗口几何：Option<(w,h,x,y)>；None = 未记录。
    Geom(Option<(u32, u32, i32, i32)>),
}

impl SettingsField {
    pub(crate) fn value_is_valid(&self) -> bool {
        match (&self.kind, &self.value) {
            (FieldKind::Number(range), FieldValue::Number(v)) => *v >= range.min && *v <= range.max,
            _ => true,
        }
    }
}

/// 设置表单：构造时从 Config 拷贝所有字段值，OK 时回写到 Config。
#[derive(Clone, Debug)]
pub(crate) struct SettingsForm {
    pub(crate) fields: Vec<SettingsField>,
}

impl SettingsForm {
    pub(crate) fn from_config(cfg: &Config) -> Self {
        let fields = vec![
            SettingsField {
                label: Key::SettingsThemeMode,
                kind: FieldKind::ThemeMode,
                value: FieldValue::ThemeMode(cfg.mode),
                buffer: None,
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsDocFontSize,
                kind: FieldKind::Number(NumberRange {
                    min: 10.0,
                    max: 32.0,
                    step: 1.0,
                    integers_only: true,
                }),
                value: FieldValue::Number(cfg.doc_font_size),
                buffer: None,
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsDocFontFamily,
                kind: FieldKind::FontFamily,
                value: FieldValue::FontFamily(cfg.doc_font_family),
                buffer: None,
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsSmoothScroll,
                kind: FieldKind::Toggle,
                value: FieldValue::Toggle(cfg.smooth_scroll),
                buffer: None,
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsAllowRemoteImages,
                kind: FieldKind::Toggle,
                value: FieldValue::Toggle(cfg.allow_remote_images),
                buffer: None,
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsWindowGeom,
                kind: FieldKind::WindowGeom,
                value: FieldValue::Geom(cfg.window.map(|w| (w.width, w.height, w.x, w.y))),
                buffer: None,
                read_only: true,
            },
            SettingsField {
                label: Key::SettingsExportPageNumbers,
                kind: FieldKind::Toggle,
                value: FieldValue::Toggle(cfg.export_page_numbers),
                buffer: None,
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsExportToc,
                kind: FieldKind::Toggle,
                value: FieldValue::Toggle(cfg.export_toc),
                buffer: None,
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsExportMarginMm,
                kind: FieldKind::Number(NumberRange {
                    min: 0.0,
                    max: 50.0,
                    step: 1.0,
                    integers_only: false,
                }),
                value: FieldValue::Number(cfg.export_margin_mm),
                buffer: None,
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsTabWidth,
                kind: FieldKind::Number(NumberRange {
                    min: 2.0,
                    max: 4.0,
                    step: 2.0,
                    integers_only: true,
                }),
                value: FieldValue::Number(cfg.tab_width as f32),
                buffer: None,
                read_only: false,
            },
        ];
        Self { fields }
    }

    pub(crate) fn all_valid(&self) -> bool {
        self.fields.iter().all(|f| f.value_is_valid())
    }

    /// 将表单值应用到 Config（clone 后返回新 Config）。
    pub(crate) fn apply(&self, cfg: &Config) -> Config {
        let mut out = *cfg;
        for f in &self.fields {
            match &f.label {
                Key::SettingsThemeMode => {
                    if let FieldValue::ThemeMode(m) = f.value {
                        out.mode = m;
                    }
                }
                Key::SettingsDocFontSize => {
                    if let FieldValue::Number(v) = f.value {
                        out.doc_font_size = v.clamp(10.0, 32.0);
                    }
                }
                Key::SettingsDocFontFamily => {
                    if let FieldValue::FontFamily(v) = f.value {
                        out.doc_font_family = v;
                    }
                }
                Key::SettingsSmoothScroll => {
                    if let FieldValue::Toggle(v) = f.value {
                        out.smooth_scroll = v;
                    }
                }
                Key::SettingsAllowRemoteImages => {
                    if let FieldValue::Toggle(v) = f.value {
                        out.allow_remote_images = v;
                    }
                }
                Key::SettingsExportPageNumbers => {
                    if let FieldValue::Toggle(v) = f.value {
                        out.export_page_numbers = v;
                    }
                }
                Key::SettingsExportToc => {
                    if let FieldValue::Toggle(v) = f.value {
                        out.export_toc = v;
                    }
                }
                Key::SettingsExportMarginMm => {
                    if let FieldValue::Number(v) = f.value {
                        out.export_margin_mm = v.clamp(0.0, 50.0);
                    }
                }
                Key::SettingsTabWidth => {
                    if let FieldValue::Number(v) = f.value {
                        let v = v as u8;
                        out.tab_width = if v == 2 || v == 4 { v } else { 4 };
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// 按钮 id（统一常量，避免魔法数字）。
pub(crate) const BTN_OK: u16 = 1;
pub(crate) const BTN_RESET_DEFAULTS: u16 = 2;
pub(crate) const BTN_CANCEL: u16 = 3;

/// 键盘焦点顺序：OK → Cancel → Restore Defaults（值为 buttons() 中的索引）。
const BUTTON_TAB_ORDER: [usize; 3] = [0, 2, 1];

/// 「清除」按钮的水平内边距：按钮宽度按标签实测宽 + 两侧内边距，
/// 不再用固定 120px（小于标签宽时文字会溢出描边，看起来不像按钮）。
const GEOM_CLEAR_PAD_X: f32 = 10.0;
/// 值文本左内边距（与只读控件内文一致）。
const GEOM_VALUE_PAD_X: f32 = 4.0;
/// 值文本与「清除」按钮之间的最小间隔：给按钮边界留呼吸位，避免贴边。
const GEOM_VALUE_GAP: f32 = 8.0;

/// 控件矩形：用于命中判定和键盘导航。
#[derive(Clone, Debug)]
pub(crate) struct FieldRects {
    pub(crate) label: Rect,
    /// 主控件（radio 群 / toggle / number input / dropdown / 只读展示）。
    pub(crate) control: Rect,
}

/// 设置模态布局：卡片 + 每个字段的矩形 + 按钮矩形。
#[derive(Clone, Debug)]
pub(crate) struct SettingsLayout {
    pub(crate) card: Rect,
    pub(crate) fields: Vec<FieldRects>,
    pub(crate) buttons: Vec<Rect>,
    pub(crate) button_ids: Vec<u16>,
    pub(crate) window_geom_clear: Option<Rect>,
    /// 窗口几何值文本的可用矩形：右边界已让出按钮与间隔，绘制时必须按此裁切。
    pub(crate) window_geom_value: Option<Rect>,
}

pub(crate) fn buttons() -> Vec<ModalButton> {
    vec![
        ModalButton::new(BTN_OK, t(Key::BtnConfirm).to_string(), ButtonRole::Primary),
        ModalButton::new(
            BTN_RESET_DEFAULTS,
            t(Key::BtnRestoreDefaults).to_string(),
            ButtonRole::Secondary,
        ),
        ModalButton::new(
            BTN_CANCEL,
            t(Key::BtnCancel).to_string(),
            ButtonRole::Secondary,
        ),
    ]
}

/// 纯函数：根据视口 + scale + form + text，产出 SettingsLayout。
/// 不在模块内持有任何状态，headless 可测。
pub(crate) fn layout(
    vp: Rect,
    scale: f32,
    form: &SettingsForm,
    text: &mut dyn Text,
    buttons: &[ModalButton],
) -> SettingsLayout {
    let p = surface::dialog_padding(scale);
    let title_line_h = surface::dialog_title_line_height(scale);
    let body_line_h = surface::dialog_body_line_height(scale);
    let bh = surface::dialog_button_height(scale);
    let gap = surface::dialog_button_gap(scale);
    let row_gap = body_line_h * 0.7;
    let field_gap_x = 10.0 * scale;
    let label_col_w_max = 220.0 * scale;
    let control_h = body_line_h * 1.4;

    let title = t(Key::ModalSettingsTitle);
    let title_w = text.title_width(title);

    // 标签列宽：实测所有标签最大宽，夹到 label_col_w_max
    let label_col_w = form
        .fields
        .iter()
        .map(|f| text.width(t(f.label)))
        .fold(0.0, f32::max)
        .min(label_col_w_max);

    // 控件列自然宽
    let control_col_w = 380.0 * scale;

    let inner_w = label_col_w + field_gap_x + control_col_w;

    // 按钮宽
    let widths: Vec<f32> = buttons
        .iter()
        .map(|b| {
            let pad = 2.0 * 12.0 * scale; // BUTTON_PAD_X
            (text.width(&b.display_label()) + pad).max(surface::dialog_button_min_width(scale))
        })
        .collect();
    let buttons_w = widths.iter().sum::<f32>()
        + if widths.len() > 1 {
            gap * (widths.len() - 1) as f32
        } else {
            0.0
        };

    let desired = (title_w).max(inner_w).max(buttons_w) + 2.0 * p;

    let margin = surface::dialog_margin(scale);
    let max_vp_w = (vp.w - 2.0 * margin).max(0.0);
    let hi = surface::dialog_max_width(scale).min(max_vp_w);
    let lo = surface::dialog_min_width(scale).min(hi);
    let card_w = desired.clamp(lo, hi);

    let max_vp_h = (vp.h - 2.0 * margin).max(0.0);
    let n_fields = form.fields.len().max(1) as f32;
    let fixed_top = 2.0 * p + title_line_h + surface::dialog_title_gap(scale);
    let fixed_bottom = bh + surface::dialog_body_gap(scale);
    let avail_h = (max_vp_h - fixed_top - fixed_bottom).max(0.0);
    let max_rows_avail = (avail_h / (control_h + row_gap)).floor();
    let shown_rows = n_fields.min(max_rows_avail).min(n_fields);
    let body_h = shown_rows * control_h + (shown_rows - 1.0).max(0.0) * row_gap;
    let card_h = (fixed_top + body_h + fixed_bottom).min(max_vp_h);

    let card = Rect {
        x: vp.x + (vp.w - card_w) * 0.5,
        y: vp.y + (vp.h - card_h) * 0.5,
        w: card_w,
        h: card_h,
    };

    // 排版字段行（只排显示的那些；超出部分截断）
    let inner_w_actual = (card.w - 2.0 * p).max(0.0);
    let label_w = label_col_w.min(inner_w_actual * 0.45);
    let control_x = card.x + p + label_w + field_gap_x;
    let control_w = (card.x + card.w - p - control_x).max(0.0);

    let mut field_rects = Vec::with_capacity(form.fields.len());
    let mut top_y = card.y + p + title_line_h + surface::dialog_title_gap(scale);

    for f in &form.fields {
        let label = Rect {
            x: card.x + p,
            y: top_y + (control_h - body_line_h) * 0.5,
            w: label_w,
            h: body_line_h,
        };
        let control = Rect {
            x: control_x,
            y: top_y,
            w: control_w,
            h: control_h,
        };
        field_rects.push(FieldRects { label, control });
        top_y += control_h + row_gap;
    }

    // 按钮排布（自右向左）
    let y = card.y + card.h - p - bh;
    let mut button_rects: Vec<Rect> = vec![
        Rect {
            x: 0.0,
            y,
            w: 0.0,
            h: bh
        };
        buttons.len()
    ];
    let ids: Vec<u16> = buttons.iter().map(|b| b.id).collect();
    let mut cursor_x = card.x + card.w - p;
    let gaps = if buttons.len() > 1 {
        gap * (buttons.len() - 1) as f32
    } else {
        0.0
    };
    let avail_buttons = ((card.w - 2.0 * p) - gaps).max(0.0);
    let sum: f32 = widths.iter().sum();
    let fitted: Vec<f32> = if sum > avail_buttons && sum > 0.0 {
        widths.iter().map(|w| w * avail_buttons / sum).collect()
    } else {
        widths.to_vec()
    };
    for (i, w) in fitted.iter().enumerate().rev() {
        let x = cursor_x - *w;
        button_rects[i] = Rect { x, y, w: *w, h: bh };
        cursor_x = x - gap;
    }

    // "清除"窗口几何按钮位置（在 WindowGeom 控件矩形内偏右）
    let window_geom_idx = form
        .fields
        .iter()
        .position(|f| matches!(f.kind, FieldKind::WindowGeom));
    let window_geom_clear = window_geom_idx.map(|idx| {
        let control = &field_rects[idx].control;
        // 宽度按标签实测：文字必须落在描边内，否则会退化成"无边界纯文本"。
        let label_w = text.width(t(Key::SettingsResetWindowGeom));
        let bw = (label_w + 2.0 * GEOM_CLEAR_PAD_X * scale)
            .min(control.w)
            .max(0.0);
        Rect {
            x: control.x + control.w - bw,
            y: control.y + (control.h - body_line_h) * 0.5 - 2.0,
            w: bw,
            h: body_line_h + 4.0,
        }
    });

    // 值文本可用宽度 = 控件宽 − 左内边距 − 按钮宽 − 间隔；剩余宽度交给截断，
    // 从布局层面保证值文本与按钮永不重叠（绘制只按这个矩形裁切）。
    let window_geom_value = window_geom_idx.zip(window_geom_clear).map(|(idx, clear)| {
        let control = &field_rects[idx].control;
        let line_h = typography::UI_LINE_HEIGHT * scale;
        let x = control.x + GEOM_VALUE_PAD_X * scale;
        Rect {
            x,
            y: control.y + (control.h - line_h) * 0.5,
            w: (clear.x - GEOM_VALUE_GAP * scale - x).max(0.0),
            h: line_h,
        }
    });

    SettingsLayout {
        card,
        fields: field_rects,
        buttons: button_rects,
        button_ids: ids,
        window_geom_clear,
        window_geom_value,
    }
}

/// 设置模态交互状态：焦点字段（Tab 切）+ 悬停（按钮/控件）+ 清除按钮悬停。
#[derive(Clone, Copy, Debug)]
pub(crate) struct SettingsState {
    pub(crate) focused_field: usize,
    pub(crate) focused_button: Option<usize>,
    pub(crate) hovered_button: Option<usize>,
    pub(crate) hovered_clear: bool,
}

impl SettingsState {
    pub(crate) fn new(form: &SettingsForm) -> Self {
        // 初始焦点落在第一个非只读字段上；没有则落在第一个按钮。
        let first_editable = form.fields.iter().position(|f| !f.read_only);
        Self {
            focused_field: first_editable.unwrap_or(0),
            focused_button: first_editable.is_none().then_some(BUTTON_TAB_ORDER[0]),
            hovered_button: None,
            hovered_clear: false,
        }
    }

    /// Tab 循环：非只读字段 → OK → Cancel → Restore Defaults；Shift+Tab 反向。
    pub(crate) fn tab(&mut self, form: &SettingsForm, shift: bool) {
        let non_readonly: Vec<usize> = form
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.read_only)
            .map(|(i, _)| i)
            .collect();
        let cur_pos = match self.focused_button {
            Some(button) => {
                non_readonly.len()
                    + BUTTON_TAB_ORDER
                        .iter()
                        .position(|&index| index == button)
                        .unwrap_or(0)
            }
            None => non_readonly
                .iter()
                .position(|&i| i == self.focused_field)
                .unwrap_or(0),
        };
        let total = non_readonly.len() + BUTTON_TAB_ORDER.len();
        let next = if shift {
            (cur_pos + total - 1) % total
        } else {
            (cur_pos + 1) % total
        };
        if next < non_readonly.len() {
            self.focused_field = non_readonly[next];
            self.focused_button = None;
        } else {
            self.focused_button = Some(BUTTON_TAB_ORDER[next - non_readonly.len()]);
        }
    }

    /// 字段值切换：toggle / radio / dropdown 点击或空格键触发。
    pub(crate) fn toggle_field(&mut self, form: &mut SettingsForm) {
        if self.focused_button.is_some() {
            return;
        }
        let Some(f) = form.fields.get_mut(self.focused_field) else {
            return;
        };
        if f.read_only {
            return;
        }
        match (&f.kind, &mut f.value) {
            (FieldKind::Toggle, FieldValue::Toggle(b)) => *b = !*b,
            (FieldKind::ThemeMode, FieldValue::ThemeMode(m)) => {
                *m = match *m {
                    ThemeMode::System => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                    ThemeMode::Dark => ThemeMode::System,
                };
            }
            (FieldKind::FontFamily, FieldValue::FontFamily(fam)) => {
                *fam = match *fam {
                    DocFontFamily::System => DocFontFamily::Sans,
                    DocFontFamily::Sans => DocFontFamily::Serif,
                    DocFontFamily::Serif => DocFontFamily::Mono,
                    DocFontFamily::Mono => DocFontFamily::System,
                };
            }
            (FieldKind::Number(range), FieldValue::Number(v)) => {
                let step = range.step;
                if range.integers_only {
                    let cur = (*v as i32) as f32;
                    let next = cur + step;
                    *v = if next <= range.max { next } else { range.min };
                } else {
                    let next = *v + step;
                    *v = if next <= range.max { next } else { range.min };
                }
            }
            _ => {}
        }
    }

    /// 数字输入框按键（用于数字输入的 +/- 调整）。
    pub(crate) fn adjust_number(&mut self, form: &mut SettingsForm, delta: f32) {
        if self.focused_button.is_some() {
            return;
        }
        let Some(f) = form.fields.get_mut(self.focused_field) else {
            return;
        };
        if f.read_only {
            return;
        }
        let FieldKind::Number(range) = &f.kind else {
            return;
        };
        if let FieldValue::Number(v) = &mut f.value {
            let raw = if range.integers_only {
                *v as i32
            } else {
                *v as i32
            };
            let step = range.step;
            let nudge = (delta as i32 * step as i32) as f32;
            *v = (*v + nudge).clamp(range.min, range.max);
            if range.integers_only {
                *v = *v as i32 as f32;
            }
            // tab_width 只允许 2 或 4
            if range.min == 2.0 && range.max == 4.0 && range.integers_only {
                let n = *v as u8;
                *v = if n == 2 || n == 4 { n as f32 } else { 4.0 };
            }
            _ = raw;
        }
    }

    /// 点击处理：返回被激活的按钮 id，或 None（切换了字段但没点击按钮）。
    pub(crate) fn on_click(
        &mut self,
        mouse: (f32, f32),
        layout: &SettingsLayout,
        form: &mut SettingsForm,
    ) -> Option<u16> {
        // 先检按钮
        for (i, r) in layout.buttons.iter().enumerate() {
            if hit(mouse, r) {
                return layout.button_ids.get(i).copied();
            }
        }
        // 清除窗口几何
        if let Some(r) = layout.window_geom_clear {
            if hit(mouse, &r) {
                if let Some(f) = form
                    .fields
                    .iter_mut()
                    .find(|f| matches!(f.kind, FieldKind::WindowGeom))
                {
                    f.value = FieldValue::Geom(None);
                }
                return None;
            }
        }
        // 检字段矩形（切换焦点 + 触发值变更）
        for (i, r) in layout.fields.iter().enumerate() {
            if hit(mouse, &r.control) || hit(mouse, &r.label) {
                if form.fields.get(i).map_or(true, |f| f.read_only) {
                    // 只读字段：只切焦点不改变值
                    if !form.fields[i].read_only {
                        // 不应到达这里，但防御性处理
                    }
                } else {
                    self.focused_field = i;
                    self.focused_button = None;
                    self.toggle_field(form);
                }
                return None;
            }
        }
        None
    }

    /// 悬停更新
    pub(crate) fn on_hover(&mut self, mouse: (f32, f32), layout: &SettingsLayout) -> bool {
        let prev_btn = self.hovered_button;
        let prev_clear = self.hovered_clear;

        self.hovered_button = layout
            .buttons
            .iter()
            .enumerate()
            .find(|(_, r)| hit(mouse, r))
            .map(|(i, _)| i);
        self.hovered_clear = layout
            .window_geom_clear
            .as_ref()
            .map(|r| hit(mouse, r))
            .unwrap_or(false);

        prev_btn != self.hovered_button || prev_clear != self.hovered_clear
    }

    /// 键盘处理：返回被激活的按钮 id（Enter/Escape 立即返回按钮 id），
    /// 或 None（只是导航/值变更）。
    pub(crate) fn on_key(
        &mut self,
        key: &InputKey,
        mods: crate::input::Mods,
        form: &mut SettingsForm,
        layout: &SettingsLayout,
    ) -> Option<u16> {
        let shift = mods.shift;
        match key {
            InputKey::Escape => {
                // 返回最后一个按钮 id（通常是 Cancel）
                layout.button_ids.last().copied()
            }
            InputKey::Enter => {
                // 如果有聚焦按钮 → 激活；否则激活 OK
                self.focused_button
                    .and_then(|i| layout.button_ids.get(i).copied())
                    .or_else(|| layout.button_ids.first().copied())
            }
            InputKey::Tab => {
                self.tab(form, shift);
                None
            }
            InputKey::Up => {
                self.adjust_number(form, -1.0);
                None
            }
            InputKey::Down => {
                self.adjust_number(form, 1.0);
                None
            }
            InputKey::Left
                if form
                    .fields
                    .get(self.focused_field)
                    .map_or(false, |f| matches!(f.kind, FieldKind::Number(_))) =>
            {
                self.adjust_number(form, -1.0);
                None
            }
            InputKey::Right
                if form
                    .fields
                    .get(self.focused_field)
                    .map_or(false, |f| matches!(f.kind, FieldKind::Number(_))) =>
            {
                self.adjust_number(form, 1.0);
                None
            }
            InputKey::Text(t) if t == " " => {
                if let Some(i) = self.focused_button {
                    layout.button_ids.get(i).copied()
                } else {
                    self.toggle_field(form);
                    None
                }
            }
            _ => None,
        }
    }
}

fn hit(mouse: (f32, f32), r: &Rect) -> bool {
    let (x, y) = mouse;
    r.w > 0.0 && r.h > 0.0 && x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

/// 绘制设置模态：遮罩 → 卡片 → 标题 → 字段 → 按钮。
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    frame: &mut Frame,
    vp: Rect,
    scale: f32,
    theme: &crate::theme::Theme,
    form: &SettingsForm,
    state: &SettingsState,
    layout: &SettingsLayout,
    buttons: &[ModalButton],
    text: &mut dyn Text,
) {
    // 遮罩
    frame.fill_rect(
        vp.x as i32,
        vp.y as i32,
        vp.w.max(0.0) as u32,
        vp.h.max(0.0) as u32,
        [0, 0, 0, surface::scrim_alpha(theme.dark)],
    );
    paint_card(frame, scale, theme, layout.card);

    let card = layout.card;
    let p = surface::dialog_padding(scale);
    let title_line_h = surface::dialog_title_line_height(scale);
    let title = t(Key::ModalSettingsTitle);
    text.draw_title(
        frame,
        title,
        theme.text,
        (card.x + p) as i32,
        (card.y + p) as i32,
    );

    // 绘制每个字段
    let body_top = card.y + p + title_line_h + surface::dialog_title_gap(scale);
    let _ = body_top; // body_top 通过 layout.fields 的 label rects 承载

    for (i, (f, fr)) in form.fields.iter().zip(layout.fields.iter()).enumerate() {
        let is_focused = state.focused_button.is_none() && state.focused_field == i;
        paint_field_label(frame, text, f, fr, theme);
        paint_field_control(frame, text, f, fr, theme, scale, is_focused);
    }

    // 窗口几何行：值文本先按可用宽度截断，再画按钮。两者共用 layout 的矩形，
    // 因此绘制区域与命中区域天然一致，且值文本永远不会覆盖按钮。
    if let (Some(f), Some(value_rect)) = (
        form.fields
            .iter()
            .find(|f| matches!(f.kind, FieldKind::WindowGeom)),
        layout.window_geom_value,
    ) {
        let full = match &f.value {
            FieldValue::Geom(Some((w, h, x, y))) => {
                format!("{} × {}  (x={}, y={})", w, h, x, y)
            }
            FieldValue::Geom(None) => t(Key::SettingsWindowGeomNone).into(),
            _ => String::new(),
        };
        // 用真实文本度量做二分截断：复用 Text::width，不另造一套估算表。
        let fitted = elide_with_ellipsis(&full, value_rect.w, |s| text.width(s));
        text.draw(
            frame,
            &fitted,
            theme.muted,
            value_rect.x as i32,
            value_rect.y as i32,
        );

        if let Some(r) = layout.window_geom_clear.as_ref() {
            if matches!(&f.value, FieldValue::Geom(Some(_))) {
                paint_buttonish(
                    frame,
                    text,
                    t(Key::SettingsResetWindowGeom),
                    theme,
                    r,
                    scale,
                    state.hovered_clear,
                );
            } else {
                paint_clear_stub(frame, theme, r);
            }
        }
    }

    // 按钮
    paint_buttons(frame, scale, theme, buttons, state, layout, text);
}

fn paint_field_label(
    frame: &mut Frame,
    text: &mut dyn Text,
    f: &SettingsField,
    fr: &FieldRects,
    theme: &crate::theme::Theme,
) {
    let label_text = t(f.label);
    // 只读字段（窗口几何）仍可操作「清除」，标签不能画成禁用灰，
    // 否则整行会被误读为不可用。
    text.draw(
        frame,
        label_text,
        theme.text,
        fr.label.x as i32,
        fr.label.y as i32,
    );
}

fn paint_field_control(
    frame: &mut Frame,
    text: &mut dyn Text,
    f: &SettingsField,
    fr: &FieldRects,
    theme: &crate::theme::Theme,
    scale: f32,
    is_focused: bool,
) {
    let c = fr.control;
    let cr = surface::corner_radius(scale);

    if f.read_only {
        // 只读：画浅灰底；值文本由 paint() 统一按可用矩形的裁切后绘制，
        // 这里不再直接落字（避免值文本越过右侧「清除」按钮）。
        frame.fill_rounded_rect(
            c.x.round() as i32,
            c.y.round() as i32,
            c.w.max(0.0).round() as u32,
            c.h.max(0.0).round() as u32,
            cr,
            palette::surface_popup(theme),
        );
        return;
    }

    // 非只读：画控件底（描边 + 焦点环）
    let border_color = if is_focused {
        theme.accent
    } else {
        [
            theme.chrome_border[0],
            theme.chrome_border[1],
            theme.chrome_border[2],
        ]
    };
    stroke_frame(frame, c, cr, border_color, palette::surface_popup(theme));

    // 焦点环先画（在值文本下层），避免覆盖文本
    if is_focused {
        focus_ring(frame, c, cr, theme.accent, palette::surface_popup(theme));
    }

    // 控件值文本（在焦点环上层，始终可见）
    let inner_x = c.x + 6.0 * scale;
    let inner_y = c.y + (c.h - typography::UI_LINE_HEIGHT * scale) * 0.5;
    let value_text = field_value_display(f);

    text.draw(
        frame,
        &value_text,
        if is_focused { theme.text } else { theme.muted },
        inner_x as i32,
        inner_y as i32,
    );
}

fn field_value_display(f: &SettingsField) -> String {
    match &f.value {
        FieldValue::ThemeMode(m) => match m {
            ThemeMode::System => t(Key::SettingsFollowSystem).into(),
            ThemeMode::Light => t(Key::SettingsLight).into(),
            ThemeMode::Dark => t(Key::SettingsDark).into(),
        },
        FieldValue::Number(v) => {
            if let FieldKind::Number(range) = &f.kind {
                if range.integers_only {
                    format!("{}", *v as i32)
                } else {
                    format!("{:.1}", v)
                }
            } else {
                format!("{}", v)
            }
        }
        FieldValue::Toggle(b) => t(if *b { Key::ToggleOn } else { Key::ToggleOff }).into(),
        FieldValue::FontFamily(fam) => match fam {
            DocFontFamily::System => t(Key::SettingsFontSystem).into(),
            DocFontFamily::Sans => t(Key::SettingsFontSans).into(),
            DocFontFamily::Serif => t(Key::SettingsFontSerif).into(),
            DocFontFamily::Mono => t(Key::SettingsFontMono).into(),
        },
        FieldValue::Geom(_) => String::new(), // 只读路径处理
    }
}

fn stroke_frame(frame: &mut Frame, r: Rect, radius: f32, border: [u8; 3], bg: [u8; 4]) {
    let x = r.x.round() as i32;
    let y = r.y.round() as i32;
    let w = r.w.max(0.0).round() as u32;
    let h = r.h.max(0.0).round() as u32;
    frame.fill_rounded_rect(x, y, w, h, radius, [border[0], border[1], border[2], 255]);
    if w > 2 && h > 2 {
        frame.fill_rounded_rect(x + 1, y + 1, w - 2, h - 2, (radius - 1.0).max(0.0), bg);
    }
}

fn focus_ring(frame: &mut Frame, r: Rect, radius: f32, color: [u8; 3], inner: [u8; 4]) {
    let x = r.x.round() as i32 + 1;
    let y = r.y.round() as i32 + 1;
    let w = r.w.round().max(0.0) as u32;
    let h = r.h.round().max(0.0) as u32;
    if w <= 2 || h <= 2 {
        return;
    }
    let cr = (radius - 1.0).max(0.0);
    frame.fill_rounded_rect(x, y, w - 2, h - 2, cr, [color[0], color[1], color[2], 255]);
    if w > 4 && h > 4 {
        frame.fill_rounded_rect(x + 1, y + 1, w - 4, h - 4, (cr - 1.0).max(0.0), inner);
    }
}

fn paint_card(frame: &mut Frame, scale: f32, theme: &crate::theme::Theme, card: Rect) {
    let x = card.x.round() as i32;
    let y = card.y.round() as i32;
    let w = card.w.max(0.0).round() as u32;
    let h = card.h.max(0.0).round() as u32;
    let r = surface::dialog_radius(scale);
    frame.fill_shadow(x, y, w, h, r, scale, theme.dark);
    frame.fill_rounded_rect(x, y, w, h, r, theme.chrome_border);
    if w > 2 && h > 2 {
        frame.fill_rounded_rect(
            x + 1,
            y + 1,
            w - 2,
            h - 2,
            (r - 1.0).max(0.0),
            palette::surface_popup(theme),
        );
    }
}

fn paint_buttons(
    frame: &mut Frame,
    scale: f32,
    theme: &crate::theme::Theme,
    buttons: &[ModalButton],
    state: &SettingsState,
    layout: &SettingsLayout,
    text: &mut dyn Text,
) {
    let radius = surface::corner_radius(scale);
    let line_h = typography::UI_LINE_HEIGHT * scale;
    for (i, button) in buttons.iter().enumerate() {
        let Some(&rect) = layout.buttons.get(i) else {
            continue;
        };
        if rect.w <= 0.0 || rect.h <= 0.0 {
            continue;
        }
        let x = rect.x.round() as i32;
        let y = rect.y.round() as i32;
        let w = rect.w.round() as u32;
        let h = rect.h.round() as u32;
        let hovered = state.hovered_button == Some(i);
        let focused = state.focused_button == Some(i);
        match button.role {
            ButtonRole::Primary => {
                frame.fill_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    radius,
                    [theme.accent[0], theme.accent[1], theme.accent[2], 255],
                );
                let fg = theme.accent_text;
                if focused {
                    let inner = [theme.accent[0], theme.accent[1], theme.accent[2], 255];
                    focus_ring(frame, rect, radius, fg, inner);
                }
                draw_btn_text(frame, text, &button.display_label(), fg, rect, line_h);
            }
            ButtonRole::Danger | ButtonRole::Secondary => {
                let (border, fg) = if button.role == ButtonRole::Danger {
                    (theme.danger, theme.danger)
                } else {
                    (
                        [
                            theme.chrome_border[0],
                            theme.chrome_border[1],
                            theme.chrome_border[2],
                        ],
                        theme.text,
                    )
                };
                stroke_frame(frame, rect, radius, border, palette::surface_popup(theme));
                if hovered {
                    frame.fill_hover(
                        x,
                        y,
                        w,
                        h,
                        scale,
                        [theme.hover[0], theme.hover[1], theme.hover[2], 128],
                    );
                }
                if focused {
                    let inner = palette::surface_popup(theme);
                    focus_ring(frame, rect, radius, theme.accent, inner);
                }
                draw_btn_text(frame, text, &button.display_label(), fg, rect, line_h);
            }
        }
    }
}

fn draw_btn_text(
    frame: &mut Frame,
    text: &mut dyn Text,
    label: &str,
    color: [u8; 3],
    rect: Rect,
    line_h: f32,
) {
    let tw = text.width(label);
    let x = rect.x + (rect.w - tw) * 0.5;
    let y = rect.y + (rect.h - line_h) * 0.5;
    text.draw(frame, label, color, x as i32, y as i32);
}

fn paint_clear_stub(frame: &mut Frame, theme: &crate::theme::Theme, r: &Rect) {
    let x = r.x.round() as i32;
    let y = r.y.round() as i32;
    let w = r.w.round() as u32;
    let h = r.h.round() as u32;
    // 半透明占位：有位置但无描边/无文字，明确不是可点按钮
    // （之前实心灰块更像"禁用按钮"，反而暗示这里能点）。
    frame.fill_rounded_rect(
        x,
        y,
        w,
        h,
        2.0,
        [
            theme.chrome_border[0],
            theme.chrome_border[1],
            theme.chrome_border[2],
            48,
        ],
    );
}

fn paint_buttonish(
    frame: &mut Frame,
    text: &mut dyn Text,
    label: &str,
    theme: &crate::theme::Theme,
    r: &Rect,
    scale: f32,
    hovered: bool,
) {
    let x = r.x.round() as i32;
    let y = r.y.round() as i32;
    let w = r.w.round() as u32;
    let h = r.h.round() as u32;
    let cr = surface::corner_radius(scale);
    // 浅底（Card 档）+ 1px 描边：与只读控件的 Popup 底色区分，hover 时描边转强调色。
    let bg = palette::surface_card(theme);
    let border = if hovered {
        theme.accent
    } else {
        [
            theme.chrome_border[0],
            theme.chrome_border[1],
            theme.chrome_border[2],
        ]
    };
    stroke_frame(frame, *r, cr, border, bg);
    if hovered {
        frame.fill_hover(
            x,
            y,
            w,
            h,
            scale,
            [theme.hover[0], theme.hover[1], theme.hover[2], 128],
        );
    }
    let tw = text.width(label);
    let line_h = typography::UI_LINE_HEIGHT * scale;
    let tx = r.x + (r.w - tw) * 0.5;
    let ty = r.y + (r.h - line_h) * 0.5;
    text.draw(frame, label, theme.text, tx as i32, ty as i32);
}

/// 尾部省略：在 `max_w` 内尽量保留前缀，末尾补 `…`。
///
/// `measure` 由调用方注入（生产即 `Text::width`，测试用等宽假度量），
/// 复用真实栅格化度量，避免另造一套与绘制不一致的宽度估算；
/// 前缀越长越宽，故二分找最长可行前缀，只需 O(log n) 次度量。
pub(crate) fn elide_with_ellipsis(
    text: &str,
    max_w: f32,
    mut measure: impl FnMut(&str) -> f32,
) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if measure(text) <= max_w {
        return text.to_string();
    }
    // 连省略号都放不下：留空，避免画出半截字形更难辨认。
    if measure("…") > max_w {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let mut candidate: String = chars[..mid].iter().collect();
        candidate.push('…');
        if measure(&candidate) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut out: String = chars[..lo].iter().collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Mods;

    const EPSILON: f32 = 0.001;

    /// 确定性文本度量：只影响宽度，不在几何测试中绘制像素。
    struct FakeText;

    impl Text for FakeText {
        fn width(&mut self, s: &str) -> f32 {
            s.chars().count() as f32 * 8.0
        }

        fn draw(&mut self, _frame: &mut Frame, _s: &str, _color: [u8; 3], _x: i32, _y: i32) {}
    }

    fn viewport(width: f32, height: f32) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
        }
    }

    fn layout_for(vp: Rect, form: &SettingsForm) -> SettingsLayout {
        let mut text = FakeText;
        layout(vp, 1.0, form, &mut text, &buttons())
    }

    fn default_form() -> SettingsForm {
        SettingsForm::from_config(&Config::default())
    }

    fn rectangles(layout: &SettingsLayout) -> Vec<Rect> {
        let mut rects = vec![layout.card];
        for field in &layout.fields {
            rects.push(field.label);
            rects.push(field.control);
        }
        rects.extend(layout.buttons.iter().copied());
        rects.extend(layout.window_geom_clear);
        rects
    }

    fn assert_layout_fits(vp: Rect, layout: &SettingsLayout) {
        for rect in rectangles(layout) {
            assert!(
                rect.x >= vp.x && rect.y >= vp.y,
                "矩形起点越过视口上/左缘: {rect:?}, viewport={vp:?}"
            );
            assert!(
                rect.x + rect.w <= vp.x + vp.w + EPSILON
                    && rect.y + rect.h <= vp.y + vp.h + EPSILON,
                "矩形终点越过视口下/右缘: {rect:?}, viewport={vp:?}"
            );
        }
    }

    fn editable_indices(form: &SettingsForm) -> Vec<usize> {
        form.fields
            .iter()
            .enumerate()
            .filter(|(_, field)| !field.read_only)
            .map(|(index, _)| index)
            .collect()
    }

    #[test]
    fn field_rows_are_strictly_separated() {
        let form = default_form();
        let layout = layout_for(viewport(1280.0, 800.0), &form);

        for (index, pair) in layout.fields.windows(2).enumerate() {
            let previous = &pair[0];
            let next = &pair[1];
            assert!(
                next.control.y > previous.control.y + previous.control.h,
                "控件行 {index} 与 {} 重叠",
                index + 1
            );
            assert!(
                next.label.y > previous.label.y + previous.label.h,
                "标签行 {index} 与 {} 重叠",
                index + 1
            );
        }
    }

    #[test]
    fn every_rectangle_fits_at_minimum_typical_and_large_sizes() {
        let form = default_form();
        for (width, height) in [(800.0, 600.0), (1280.0, 800.0), (1920.0, 1080.0)] {
            let vp = viewport(width, height);
            let layout = layout_for(vp, &form);
            assert_layout_fits(vp, &layout);
        }
    }

    #[test]
    fn action_buttons_have_clickable_target_sizes() {
        let form = default_form();
        let layout = layout_for(viewport(1280.0, 800.0), &form);

        assert_eq!(layout.button_ids, [BTN_OK, BTN_RESET_DEFAULTS, BTN_CANCEL]);
        for (id, rect) in layout.button_ids.iter().zip(&layout.buttons) {
            assert!(rect.w >= 80.0, "按钮 {id} 宽度不足: {rect:?}");
            assert!(rect.h >= 28.0, "按钮 {id} 高度不足: {rect:?}");
        }
    }

    #[test]
    fn extreme_font_values_keep_labels_and_controls_contained() {
        let config = Config {
            doc_font_size: 32.0,
            doc_font_family: DocFontFamily::Serif,
            ..Config::default()
        };
        let form = SettingsForm::from_config(&config);
        let vp = viewport(1280.0, 800.0);
        let layout = layout_for(vp, &form);
        assert_layout_fits(vp, &layout);

        let mut text = FakeText;
        for key in [Key::SettingsDocFontSize, Key::SettingsDocFontFamily] {
            let index = form
                .fields
                .iter()
                .position(|field| field.label == key)
                .expect("测试字段必须存在");
            let field = &form.fields[index];
            let rects = &layout.fields[index];
            assert!(
                text.width(t(field.label)) <= rects.label.w + EPSILON,
                "字段标签超出标签矩形: {key:?}"
            );
            assert!(
                text.width(&field_value_display(field)) <= rects.control.w + EPSILON,
                "字段值超出控件矩形: {key:?}"
            );
        }
    }

    #[test]
    fn light_and_dark_modes_have_identical_geometry() {
        let light_config = Config {
            mode: ThemeMode::Light,
            ..Config::default()
        };
        let dark_config = Config {
            mode: ThemeMode::Dark,
            ..Config::default()
        };
        let light_form = SettingsForm::from_config(&light_config);
        let dark_form = SettingsForm::from_config(&dark_config);
        let vp = viewport(1280.0, 800.0);

        let light = layout_for(vp, &light_form);
        let dark = layout_for(vp, &dark_form);
        assert_eq!(rectangles(&light), rectangles(&dark));
        assert_eq!(light.button_ids, dark.button_ids);
    }

    fn focus_button(state: &mut SettingsState, layout: &SettingsLayout, id: u16) {
        state.focused_button = Some(
            layout
                .button_ids
                .iter()
                .position(|&candidate| candidate == id)
                .expect("测试按钮必须存在"),
        );
    }

    #[test]
    fn tab_moves_through_editable_fields_before_buttons() {
        let form = default_form();
        let editable = editable_indices(&form);
        let mut state = SettingsState::new(&form);
        assert_eq!(state.focused_field, editable[0]);

        for &expected in editable.iter().skip(1) {
            state.tab(&form, false);
            assert_eq!(state.focused_field, expected);
            assert_eq!(state.focused_button, None);
        }
        state.tab(&form, false);
        assert_eq!(state.focused_button, Some(BUTTON_TAB_ORDER[0]));
    }

    #[test]
    fn shift_tab_reverses_fields_and_action_buttons() {
        let form = default_form();
        let editable = editable_indices(&form);
        let layout = layout_for(viewport(1280.0, 800.0), &form);
        let mut state = SettingsState::new(&form);

        for expected_id in [BTN_RESET_DEFAULTS, BTN_CANCEL, BTN_OK] {
            state.tab(&form, true);
            let expected = layout
                .button_ids
                .iter()
                .position(|&id| id == expected_id)
                .expect("测试按钮必须存在");
            assert_eq!(state.focused_button, Some(expected));
        }
        for &expected in editable.iter().rev() {
            state.tab(&form, true);
            assert_eq!(state.focused_field, expected);
            assert_eq!(state.focused_button, None);
        }

        state.tab(&form, true);
        let reset = layout
            .button_ids
            .iter()
            .position(|&id| id == BTN_RESET_DEFAULTS)
            .expect("Restore Defaults 按钮必须存在");
        assert_eq!(state.focused_button, Some(reset));
    }

    #[test]
    fn tab_order_also_includes_every_action_button() {
        let form = default_form();
        let editable = editable_indices(&form);
        let layout = layout_for(viewport(1280.0, 800.0), &form);
        let expected_stops = editable.len() + layout.buttons.len();
        let mut state = SettingsState::new(&form);
        let mut actual_stops = 1;

        for &expected in editable.iter().skip(1) {
            state.tab(&form, false);
            actual_stops += 1;
            assert_eq!(state.focused_field, expected);
            assert_eq!(state.focused_button, None);
        }
        for expected_id in [BTN_OK, BTN_CANCEL, BTN_RESET_DEFAULTS] {
            state.tab(&form, false);
            actual_stops += 1;
            let expected = layout
                .button_ids
                .iter()
                .position(|&id| id == expected_id)
                .expect("测试按钮必须存在");
            assert_eq!(state.focused_button, Some(expected));
        }
        assert_eq!(actual_stops, expected_stops);

        state.tab(&form, false);
        assert_eq!(state.focused_field, editable[0]);
        assert_eq!(state.focused_button, None);
    }

    #[test]
    fn enter_activates_focused_ok_button() {
        let mut form = default_form();
        let layout = layout_for(viewport(1280.0, 800.0), &form);
        let mut state = SettingsState::new(&form);
        focus_button(&mut state, &layout, BTN_OK);

        assert_eq!(
            state.on_key(&InputKey::Enter, Mods::default(), &mut form, &layout),
            Some(BTN_OK)
        );
    }

    #[test]
    fn enter_activates_focused_cancel_button() {
        let mut form = default_form();
        let layout = layout_for(viewport(1280.0, 800.0), &form);
        let mut state = SettingsState::new(&form);
        focus_button(&mut state, &layout, BTN_CANCEL);

        assert_eq!(
            state.on_key(&InputKey::Enter, Mods::default(), &mut form, &layout),
            Some(BTN_CANCEL)
        );
    }

    #[test]
    fn space_activates_focused_button() {
        let mut form = default_form();
        let layout = layout_for(viewport(1280.0, 800.0), &form);
        let mut state = SettingsState::new(&form);
        let space = InputKey::Text(" ".into());

        for id in [BTN_OK, BTN_CANCEL, BTN_RESET_DEFAULTS] {
            focus_button(&mut state, &layout, id);
            assert_eq!(
                state.on_key(&space, Mods::default(), &mut form, &layout),
                Some(id)
            );
        }
    }

    /// 等宽假度量：每字符固定宽度，便于把"字符宽度"作为纯函数入参验证。
    fn equal_width(char_w: f32) -> impl Fn(&str) -> f32 {
        move |s: &str| s.chars().count() as f32 * char_w
    }

    #[test]
    fn elide_keeps_text_when_it_fits() {
        assert_eq!(
            elide_with_ellipsis("abcde", 50.0, equal_width(10.0)),
            "abcde"
        );
    }

    #[test]
    fn elide_drops_last_char_when_one_char_too_wide() {
        // 全文 60 > 59：5 字符 + … = 60 仍超，故只保留 4 字符。
        assert_eq!(
            elide_with_ellipsis("abcdef", 59.0, equal_width(10.0)),
            "abcd…"
        );
    }

    #[test]
    fn elide_is_empty_when_even_ellipsis_does_not_fit() {
        assert_eq!(elide_with_ellipsis("abcdef", 9.0, equal_width(10.0)), "");
    }

    fn geom_form(w: u32, h: u32, x: i32, y: i32) -> SettingsForm {
        let config = Config {
            window: Some(crate::theme::WindowGeom {
                width: w,
                height: h,
                x,
                y,
            }),
            ..Config::default()
        };
        SettingsForm::from_config(&config)
    }

    #[test]
    fn geom_value_text_stays_left_of_clear_button() {
        for (w, h, x, y) in [(2560u32, 1440u32, -1920i32, -200i32), (1681, 864, 8, 8)] {
            let form = geom_form(w, h, x, y);
            let layout = layout_for(viewport(1280.0, 800.0), &form);
            let value_rect = layout
                .window_geom_value
                .expect("窗口几何行必须有值文本矩形");
            let clear = layout
                .window_geom_clear
                .expect("窗口几何行必须有清除按钮矩形");

            // 可用宽度必须整体落在按钮左侧（含间隔）。
            assert!(
                value_rect.x + value_rect.w <= clear.x + EPSILON,
                "值文本矩形越过清除按钮: {value_rect:?} clear={clear:?}"
            );

            // 按生产同一截断函数 + 等宽度量验证绘制右边界。
            let full = format!("{} × {}  (x={}, y={})", w, h, x, y);
            let fitted = elide_with_ellipsis(&full, value_rect.w, equal_width(8.0));
            let right = value_rect.x + 8.0 * fitted.chars().count() as f32;
            assert!(
                right <= clear.x + EPSILON,
                "值文本 {fitted:?} 右边界 {right} 越过清除按钮左边界 {}",
                clear.x
            );
        }
    }
}
