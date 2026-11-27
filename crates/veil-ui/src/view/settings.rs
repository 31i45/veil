//! 设置模态：自绘表单，承载 Config 全部字段的即时编辑。
//!
//! 独立于 modal.rs：后者只处理「静态文本 + OK/Cancel」类弹窗；设置表单有 10+ 个
//! 可编辑控件（radio / toggle / number input / dropdown / 只读展示），需要独立的
//! 布局、绘制和交互路径。
#![allow(
    unused_variables,
    clippy::if_same_then_else,
    clippy::unnecessary_map_or
)]

use crate::geometry::Rect;
use crate::i18n::{t, Key};
use crate::input::Key as InputKey;
use crate::theme::{surface, Config, DocFontFamily, DocMargin, ThemeMode, UiLanguage};
use crate::tokens::{palette, typography};
use crate::view::modal::{self, ButtonRole, ModalButton, Text};
use crate::view::Frame;

/// 字段种类：决定渲染和键盘/鼠标交互。
#[derive(Clone, Debug)]
pub(crate) enum FieldKind {
    /// 主题模式：三选一 radio（跟随系统/浅色/深色）。
    ThemeMode,
    /// 界面语言：三选一（自动/中文/English），改动重启后生效。
    UiLanguage,
    /// 数字输入（字号 10..=32 / 页边距 5..=50 / tab 宽 2/4）。
    Number(NumberRange),
    /// 开关：布尔 toggle。
    Toggle,
    /// 字体族下拉：四档预置 + 任意已安装字族（逐个切换）。
    FontFamily,
    /// 版心：五选一（紧凑/标准/宽松/全宽/自定义）。
    DocMargin,
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
    /// 只读展示字段：不能获得焦点、不能被编辑（如窗口几何）。
    pub(crate) read_only: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum FieldValue {
    ThemeMode(ThemeMode),
    UiLanguage(UiLanguage),
    Number(f32),
    Toggle(bool),
    FontFamily(FontSelection),
    DocMargin(DocMargin),
    /// 窗口几何：Option<(w,h,x,y)>；None = 未记录。
    Geom(Option<(u32, u32, i32, i32)>),
}

/// 字体选择：四档预置，或某个具体已安装字族（`cjk` = 是否自带中文字形）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FontSelection {
    Preset(DocFontFamily),
    Installed { name: String, cjk: bool },
}

impl FontSelection {
    /// 在表单可选列表中的展示名。
    fn display(&self) -> String {
        match self {
            FontSelection::Preset(fam) => match fam {
                DocFontFamily::System => t(Key::SettingsFontSystem).into(),
                DocFontFamily::Sans => t(Key::SettingsFontSans).into(),
                DocFontFamily::Serif => t(Key::SettingsFontSerif).into(),
                DocFontFamily::Mono => t(Key::SettingsFontMono).into(),
            },
            FontSelection::Installed { name, .. } => name.clone(),
        }
    }
}

/// 设置表单：构造时从 Config 拷贝所有字段值，OK 时回写到 Config。
#[derive(Clone, Debug)]
pub(crate) struct SettingsForm {
    pub(crate) fields: Vec<SettingsField>,
    /// 字体字段的可选值：四档预置 + 打开设置时探测到的已安装字族。
    /// 仅在表单生命周期内使用（不落盘）；字体探测较贵，因此随表单快照一次。
    pub(crate) font_options: Vec<FontSelection>,
    /// 字体选择弹层（None = 未打开）。放在表单上而非 `SettingsState`：
    /// 后者是 Copy 的轻量状态，不宜持堆分配的查询串与候选表。
    pub(crate) font_picker: Option<FontPicker>,
}

/// 字体选择弹层：查询串 + 过滤后的候选下标（指向 `font_options`）+ 选中/滚动位置。
#[derive(Clone, Debug, Default)]
pub(crate) struct FontPicker {
    pub(crate) query: String,
    pub(crate) hits: Vec<usize>,
    pub(crate) sel: usize,
    pub(crate) top: usize,
}

/// 字体弹层几何：锚在字体控件正下方，行高与控件一致；绘制/命中/滚轮共用。
#[derive(Clone, Copy, Debug)]
pub(crate) struct FontPickerGeom {
    pub(crate) rect: Rect,
    pub(crate) row_h: f32,
    pub(crate) rows: usize,
}

/// 弹层最大行数（同时是键盘移动的保守上界）。
const FONT_PICKER_MAX_ROWS: usize = 8;

/// 计算字体弹层几何：无弹层/字段被滚动隐藏时返回 None。
/// 行数夹取到「卡片底边之上」且不超过 8 行，保证列表永不压住底部按钮。
pub(crate) fn font_picker_geom(
    layout: &SettingsLayout,
    form: &SettingsForm,
) -> Option<FontPickerGeom> {
    let picker = form.font_picker.as_ref()?;
    let idx = form
        .fields
        .iter()
        .position(|f| matches!(f.kind, FieldKind::FontFamily))?;
    let c = layout.fields.get(idx)?.control;
    if c.w <= 0.0 || c.h <= 0.0 {
        return None;
    }
    let row_h = c.h;
    let top = c.y + c.h + 2.0;
    let bottom = layout.card.y + layout.card.h - row_h * 0.5;
    let avail = (bottom - top).max(row_h);
    let max_rows = ((avail / row_h).floor() as usize).clamp(1, FONT_PICKER_MAX_ROWS);
    let rows = max_rows.min(picker.hits.len().max(1));
    Some(FontPickerGeom {
        rect: Rect {
            x: c.x,
            y: top,
            w: c.w,
            h: row_h * rows as f32,
        },
        row_h,
        rows,
    })
}

/// 四档预置（顺序固定：系统默认 → 无衬线 → 衬线 → 等宽）。
const FONT_PRESETS: [DocFontFamily; 4] = [
    DocFontFamily::System,
    DocFontFamily::Sans,
    DocFontFamily::Serif,
    DocFontFamily::Mono,
];

impl SettingsForm {
    pub(crate) fn from_config(cfg: &Config) -> Self {
        Self::from_config_with_families(cfg, &[])
    }

    /// 构造表单并纳入已安装字族（`(族名, 是否含中文字形)`，已去重排序）。
    pub(crate) fn from_config_with_families(cfg: &Config, installed: &[(String, bool)]) -> Self {
        let mut font_options: Vec<FontSelection> = FONT_PRESETS
            .iter()
            .copied()
            .map(FontSelection::Preset)
            .collect();
        // 含中文的族排在前面：最常见的选择（中文正文）无需穿过一长串西文族。
        for want_cjk in [true, false] {
            for (name, cjk) in installed.iter().filter(|(_, c)| *c == want_cjk) {
                font_options.push(FontSelection::Installed {
                    name: name.clone(),
                    cjk: *cjk,
                });
            }
        }
        let selected = match &cfg.doc_font_face {
            Some(name) => FieldValue::FontFamily(FontSelection::Installed {
                name: name.clone(),
                cjk: installed
                    .iter()
                    .find(|(n, _)| n == name)
                    .map_or(false, |(_, c)| *c),
            }),
            None => FieldValue::FontFamily(FontSelection::Preset(cfg.doc_font_family)),
        };
        // 配置里的族名可能已卸载：仍纳入列表，保证展示与「切走再切回」可用。
        if let FieldValue::FontFamily(sel @ FontSelection::Installed { .. }) = &selected {
            if !font_options.contains(sel) {
                font_options.push(sel.clone());
            }
        }
        let fields = vec![
            SettingsField {
                label: Key::SettingsThemeMode,
                kind: FieldKind::ThemeMode,
                value: FieldValue::ThemeMode(cfg.mode),
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsUiLanguage,
                kind: FieldKind::UiLanguage,
                value: FieldValue::UiLanguage(cfg.ui_language),
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
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsDocLineHeight,
                kind: FieldKind::Number(NumberRange {
                    min: 10.0,
                    max: 96.0,
                    step: 1.0,
                    integers_only: true,
                }),
                value: FieldValue::Number(cfg.doc_line_height),
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsDocFontFamily,
                kind: FieldKind::FontFamily,
                value: selected.clone(),
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsDocMargin,
                kind: FieldKind::DocMargin,
                value: FieldValue::DocMargin(cfg.doc_margin),
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsDocMarginPx,
                kind: FieldKind::Number(NumberRange {
                    min: crate::theme::DOC_MARGIN_PX_MIN,
                    max: crate::theme::DOC_MARGIN_PX_MAX,
                    step: 1.0,
                    integers_only: true,
                }),
                value: FieldValue::Number(cfg.doc_margin_px),
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsAllowRemoteImages,
                kind: FieldKind::Toggle,
                value: FieldValue::Toggle(cfg.allow_remote_images),
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsSourcePlainLines,
                kind: FieldKind::Toggle,
                value: FieldValue::Toggle(cfg.source_plain_lines),
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsWindowGeom,
                kind: FieldKind::WindowGeom,
                value: FieldValue::Geom(cfg.window.map(|w| (w.width, w.height, w.x, w.y))),
                read_only: true,
            },
            SettingsField {
                label: Key::SettingsExportPageNumbers,
                kind: FieldKind::Toggle,
                value: FieldValue::Toggle(cfg.export_page_numbers),
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsExportToc,
                kind: FieldKind::Toggle,
                value: FieldValue::Toggle(cfg.export_toc),
                read_only: false,
            },
            SettingsField {
                label: Key::SettingsExportMarginMm,
                kind: FieldKind::Number(NumberRange {
                    min: crate::theme::EXPORT_MARGIN_MM_MIN,
                    max: crate::theme::EXPORT_MARGIN_MM_MAX,
                    step: 1.0,
                    integers_only: false,
                }),
                value: FieldValue::Number(cfg.export_margin_mm),
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
                read_only: false,
            },
        ];
        Self {
            fields,
            font_options,
            font_picker: None,
        }
    }

    /// 重置为默认配置，但保留本次打开设置时探测到的字族列表。
    pub(crate) fn reset_defaults(&self) -> Self {
        let mut form = Self::from_config(&Config::default());
        form.font_options = self.font_options.clone();
        form
    }

    /// 当前字体字段的选择（缺字段时为 None）。
    pub(crate) fn font_selection(&self) -> Option<&FontSelection> {
        self.fields.iter().find_map(|f| match &f.value {
            FieldValue::FontFamily(sel) if matches!(f.kind, FieldKind::FontFamily) => Some(sel),
            _ => None,
        })
    }

    /// 所选具体字族是否缺少中文字形（预置档位走系统回退链，恒视为可用）。
    pub(crate) fn font_missing_cjk(&self) -> bool {
        matches!(
            self.font_selection(),
            Some(FontSelection::Installed { cjk: false, .. })
        )
    }

    /// 在可选字族列表中前后切换（`delta` 为 ±1，越界循环）。
    pub(crate) fn cycle_font(&mut self, delta: i32) {
        let Some(idx) = self
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::FontFamily))
        else {
            return;
        };
        let current = match self.fields.get(idx).map(|f| &f.value) {
            Some(FieldValue::FontFamily(sel)) => sel.clone(),
            _ => return,
        };
        let Some(pos) = self.font_options.iter().position(|o| *o == current) else {
            return;
        };
        let n = self.font_options.len() as i32;
        if n == 0 {
            return;
        }
        let next = (pos as i32 + delta).rem_euclid(n) as usize;
        self.fields[idx].value = FieldValue::FontFamily(self.font_options[next].clone());
    }

    /// 打开字体选择弹层：默认选中当前值，并把它滚进可视区。
    pub(crate) fn open_font_picker(&mut self) {
        let current = self.font_selection().cloned();
        let mut picker = FontPicker {
            query: String::new(),
            hits: (0..self.font_options.len()).collect(),
            sel: 0,
            top: 0,
        };
        if let Some(current) = current {
            picker.sel = self
                .font_options
                .iter()
                .position(|o| *o == current)
                .unwrap_or(0);
        }
        picker.top = picker.sel.saturating_sub(FONT_PICKER_MAX_ROWS - 1);
        self.font_picker = Some(picker);
    }

    pub(crate) fn close_font_picker(&mut self) {
        self.font_picker = None;
    }

    /// 查询串追加（IME 提交或普通字符）后重新过滤。
    pub(crate) fn font_query_push(&mut self, s: &str) {
        let Some(mut picker) = self.font_picker.take() else {
            return;
        };
        picker.query.push_str(s);
        Self::refilter_fonts(&self.font_options, &mut picker);
        self.font_picker = Some(picker);
    }

    /// 退格：删除查询串最后一个字符并重新过滤。
    pub(crate) fn font_query_pop(&mut self) {
        let Some(mut picker) = self.font_picker.take() else {
            return;
        };
        picker.query.pop();
        Self::refilter_fonts(&self.font_options, &mut picker);
        self.font_picker = Some(picker);
    }

    fn refilter_fonts(options: &[FontSelection], picker: &mut FontPicker) {
        let q = picker.query.trim().to_lowercase();
        picker.hits = options
            .iter()
            .enumerate()
            .filter(|(_, o)| q.is_empty() || o.display().to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        picker.sel = 0;
        picker.top = 0;
    }

    /// 键盘上下移动选中项（越界循环），并把选中项保持在可视窗口内。
    pub(crate) fn font_picker_move(&mut self, delta: i32) {
        let Some(p) = self.font_picker.as_mut() else {
            return;
        };
        if p.hits.is_empty() {
            return;
        }
        let n = p.hits.len() as i32;
        p.sel = (p.sel as i32 + delta).rem_euclid(n) as usize;
        if p.sel < p.top {
            p.top = p.sel;
        } else if p.sel >= p.top + FONT_PICKER_MAX_ROWS {
            p.top = p.sel + 1 - FONT_PICKER_MAX_ROWS;
        }
    }

    /// 滚轮滚动候选窗口，并夹取选中项到窗口内。
    pub(crate) fn font_picker_scroll(&mut self, delta: i32, rows: usize) {
        let Some(p) = self.font_picker.as_mut() else {
            return;
        };
        let rows = rows.max(1);
        let max_top = p.hits.len().saturating_sub(rows);
        p.top = (p.top as i32 + delta).clamp(0, max_top as i32) as usize;
        if p.sel < p.top {
            p.sel = p.top;
        } else if p.sel >= p.top + rows {
            p.sel = (p.top + rows - 1).min(p.hits.len().saturating_sub(1));
        }
    }

    /// Enter：把当前选中候选写回字体字段并关闭弹层。
    pub(crate) fn apply_font_picker(&mut self) {
        let Some(p) = self.font_picker.take() else {
            return;
        };
        let Some(&opt) = p.hits.get(p.sel) else {
            return;
        };
        self.set_font_option(opt);
    }

    /// 鼠标点击第 `pos` 个过滤结果（下标指向 `hits`），选中并关闭。
    pub(crate) fn pick_font_hit(&mut self, pos: usize) {
        let Some(p) = self.font_picker.take() else {
            return;
        };
        let Some(&opt) = p.hits.get(pos) else {
            return;
        };
        self.set_font_option(opt);
    }

    fn set_font_option(&mut self, opt: usize) {
        let Some(value) = self.font_options.get(opt).cloned() else {
            return;
        };
        if let Some(field) = self
            .fields
            .iter_mut()
            .find(|f| matches!(f.kind, FieldKind::FontFamily))
        {
            field.value = FieldValue::FontFamily(value);
        }
    }

    /// 版心档位循环（紧凑 → 标准 → 宽松 → 全宽 → 自定义，越界循环）。
    pub(crate) fn cycle_doc_margin(&mut self, delta: i32) {
        const ORDER: [DocMargin; 5] = [
            DocMargin::Compact,
            DocMargin::Standard,
            DocMargin::Relaxed,
            DocMargin::Full,
            DocMargin::Custom,
        ];
        let Some(f) = self
            .fields
            .iter_mut()
            .find(|f| matches!(f.kind, FieldKind::DocMargin))
        else {
            return;
        };
        let FieldValue::DocMargin(m) = &mut f.value else {
            return;
        };
        let pos = ORDER.iter().position(|x| x == m).unwrap_or(1);
        let next = (pos as i32 + delta).rem_euclid(ORDER.len() as i32) as usize;
        *m = ORDER[next];
    }

    /// 将表单值应用到 Config（clone 后返回新 Config）。
    pub(crate) fn apply(&self, cfg: &Config) -> Config {
        let mut out = cfg.clone();
        for f in &self.fields {
            match &f.label {
                Key::SettingsThemeMode => {
                    if let FieldValue::ThemeMode(m) = f.value {
                        out.mode = m;
                    }
                }
                Key::SettingsUiLanguage => {
                    if let FieldValue::UiLanguage(l) = f.value {
                        out.ui_language = l;
                    }
                }
                Key::SettingsDocMargin => {
                    if let FieldValue::DocMargin(m) = f.value {
                        out.doc_margin = m;
                    }
                }
                Key::SettingsDocMarginPx => {
                    if let FieldValue::Number(v) = f.value {
                        out.doc_margin_px = crate::theme::clamp_doc_margin_px(v);
                    }
                }
                Key::SettingsDocFontSize => {
                    if let FieldValue::Number(v) = f.value {
                        out.doc_font_size = v.clamp(10.0, 32.0);
                    }
                }
                Key::SettingsDocLineHeight => {
                    if let FieldValue::Number(v) = f.value {
                        // 行高下限跟随字号、上限 3 倍字号；字号先于本行写入 out。
                        out.doc_line_height = v.clamp(out.doc_font_size, out.doc_font_size * 3.0);
                    }
                }
                Key::SettingsDocFontFamily => {
                    if let FieldValue::FontFamily(sel) = &f.value {
                        match sel {
                            FontSelection::Preset(fam) => {
                                out.doc_font_family = *fam;
                                out.doc_font_face = None;
                            }
                            FontSelection::Installed { name, .. }
                                if name.len() <= crate::theme::DOC_FONT_FACE_MAX_LEN =>
                            {
                                out.doc_font_face = Some(name.clone());
                            }
                            FontSelection::Installed { .. } => {}
                        }
                    }
                }
                Key::SettingsAllowRemoteImages => {
                    if let FieldValue::Toggle(v) = f.value {
                        out.allow_remote_images = v;
                    }
                }
                Key::SettingsSourcePlainLines => {
                    if let FieldValue::Toggle(v) = f.value {
                        out.source_plain_lines = v;
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
                        out.export_margin_mm = v.clamp(
                            crate::theme::EXPORT_MARGIN_MM_MIN,
                            crate::theme::EXPORT_MARGIN_MM_MAX,
                        );
                    }
                }
                Key::SettingsTabWidth => {
                    if let FieldValue::Number(v) = f.value {
                        let v = v as u8;
                        out.tab_width = if v == 2 || v == 4 { v } else { 4 };
                    }
                }
                // 几何字段参与 apply（DEF-T07-002）：清除/恢复默认后的 None 必须
                // 覆盖旧值，OK 才会把移除后的状态写盘并在重启时生效。
                Key::SettingsWindowGeom => {
                    if let FieldValue::Geom(geom) = f.value {
                        out.window = geom.map(|(width, height, x, y)| crate::theme::WindowGeom {
                            width,
                            height,
                            x,
                            y,
                        });
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

/// 键盘焦点顺序：按视觉从左到右的阅读顺序 OK → Restore Defaults → Cancel
/// （值为 buttons() 中的索引；按钮自右向左排布，故索引与视觉顺序一致）。
const BUTTON_TAB_ORDER: [usize; 3] = [0, 1, 2];

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
    /// 当前滚动到的首行下标（DEF-T07-E1-001）。
    pub(crate) scroll: usize,
    /// 当前视口内实际可显示的行数（含被滚动隐藏的字段不绘制）。
    pub(crate) shown_rows: usize,
    /// 最大滚动行数；0 表示所有字段都能同时显示。
    pub(crate) max_scroll: usize,
    /// 字族缺中文提示行（仅当字体行可见且所选族无中文字形时存在）。
    pub(crate) font_hint: Option<Rect>,
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

/// 卡片宽度 → (标签列宽, 控件列宽)。布局与回归测试共用，避免上限/比例常量两处漂移。
fn label_and_control_widths(card_w: f32, scale: f32, label_col_w: f32) -> (f32, f32) {
    let p = surface::dialog_padding(scale);
    let field_gap_x = 10.0 * scale;
    let inner_w = (card_w - 2.0 * p).max(0.0);
    // 0.5 比例上限：760 宽卡片下 inner_w = 720，label_w 可达 320（≥ 最长英文标签）。
    let label_w = label_col_w.min(inner_w * 0.5);
    (label_w, (inner_w - label_w - field_gap_x).max(0.0))
}

/// 纯函数：根据视口 + scale + form + text，产出 SettingsLayout。
/// 不在模块内持有任何状态，headless 可测。
pub(crate) fn layout(
    vp: Rect,
    scale: f32,
    form: &SettingsForm,
    text: &mut dyn Text,
    buttons: &[ModalButton],
    scroll_rows: usize,
) -> SettingsLayout {
    let p = surface::dialog_padding(scale);
    let title_line_h = surface::dialog_title_line_height(scale);
    let body_line_h = surface::dialog_body_line_height(scale);
    let bh = surface::dialog_button_height(scale);
    let gap = surface::dialog_button_gap(scale);
    let row_gap = body_line_h * 0.7;
    let field_gap_x = 10.0 * scale;
    // 标签列上限：英文最长标签「Include Table of Contents in PDF Export」约 312px。
    let label_col_w_max = 320.0 * scale;
    let control_h = body_line_h * 1.4;

    let title = t(Key::ModalSettingsTitle);
    let title_w = text.title_width(title);

    // 标签列宽：只测当前语言（语言在进程内启动后固定，不会运行时切换），
    // 避免中文界面按英文长标签预留出大段空白。
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
    let hint_h = if form.font_missing_cjk() {
        body_line_h
    } else {
        0.0
    };
    let max_rows_avail = ((avail_h - hint_h).max(0.0) / (control_h + row_gap)).floor();
    let shown_rows = (n_fields.min(max_rows_avail).max(1.0)) as usize;
    let max_scroll = (form.fields.len().saturating_sub(shown_rows)).min(form.fields.len());
    let scroll = scroll_rows.min(max_scroll);
    let body_h =
        shown_rows as f32 * control_h + (shown_rows as f32 - 1.0).max(0.0) * row_gap + hint_h;
    let card_h = (fixed_top + body_h + fixed_bottom).min(max_vp_h);

    let card = Rect {
        x: vp.x + (vp.w - card_w) * 0.5,
        y: vp.y + (vp.h - card_h) * 0.5,
        w: card_w,
        h: card_h,
    };

    // 排版字段行（只排显示的那些；超出部分截断）
    let (label_w, control_w) = label_and_control_widths(card.w, scale, label_col_w);
    let control_x = card.x + p + label_w + field_gap_x;

    let mut field_rects = Vec::with_capacity(form.fields.len());
    let mut font_hint = None;
    let mut top_y = card.y + p + title_line_h + surface::dialog_title_gap(scale);
    let visible_end = (scroll + shown_rows).min(form.fields.len());

    for (idx, f) in form.fields.iter().enumerate() {
        // 超出可视区的行给零矩形：命中/绘制都按空矩形跳过，绝不越出卡片
        // 覆盖到按钮区域（DEF-T07-E1-001）。
        if idx < scroll || idx >= visible_end {
            field_rects.push(FieldRects {
                label: Rect::default(),
                control: Rect::default(),
            });
            continue;
        }
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
        if matches!(f.kind, FieldKind::FontFamily) && hint_h > 0.0 {
            font_hint = Some(Rect {
                x: control_x,
                y: top_y + control_h + 2.0 * scale,
                w: control_w,
                h: body_line_h,
            });
        }
        top_y += control_h + hint_h + row_gap;
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
    let window_geom_clear = window_geom_idx.and_then(|idx| {
        let control = &field_rects[idx].control;
        if control.w <= 0.0 || control.h <= 0.0 {
            return None;
        }
        // 宽度按标签实测：文字必须落在描边内，否则会退化成"无边界纯文本"。
        let label_w = text.width(t(Key::SettingsResetWindowGeom));
        let bw = (label_w + 2.0 * GEOM_CLEAR_PAD_X * scale)
            .min(control.w)
            .max(0.0);
        Some(Rect {
            x: control.x + control.w - bw,
            y: control.y + (control.h - body_line_h) * 0.5 - 2.0,
            w: bw,
            h: body_line_h + 4.0,
        })
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
        scroll,
        shown_rows,
        max_scroll,
        font_hint,
    }
}

/// 设置模态交互状态：焦点字段（Tab 切）+ 悬停（按钮/控件）+ 清除按钮悬停。
#[derive(Clone, Copy, Debug)]
pub(crate) struct SettingsState {
    pub(crate) focused_field: usize,
    pub(crate) focused_button: Option<usize>,
    pub(crate) hovered_button: Option<usize>,
    pub(crate) hovered_clear: bool,
    /// 低高度/窄窗口下的字段滚动首行（DEF-T07-E1-001）。
    pub(crate) scroll_rows: usize,
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
            scroll_rows: 0,
        }
    }

    /// 滚轮/键盘滚动字段区：`delta` > 0 表示向上（朝首行）。
    /// 由 shell 在设置模态打开时调用，不触碰正文滚动。
    pub(crate) fn scroll(&mut self, delta: i32, max_scroll: usize) {
        let next = (self.scroll_rows as i32 + delta).clamp(0, max_scroll as i32);
        self.scroll_rows = next as usize;
    }

    /// 保证键盘焦点行落在可视区：焦点在窗口外时把可视区滚到包含它。
    pub(crate) fn reveal_focused(&mut self, scroll: usize, shown_rows: usize) {
        if self.focused_button.is_some() || shown_rows == 0 {
            return;
        }
        if self.focused_field < scroll {
            self.scroll_rows = self.focused_field;
        } else if self.focused_field >= scroll + shown_rows {
            self.scroll_rows = self.focused_field + 1 - shown_rows;
        }
    }

    /// Tab 顺序：非只读字段（按视觉行序）→ OK → Restore Defaults → Cancel；
    /// Shift+Tab 反向。线性不环形：首项再 Shift+Tab / 末项再 Tab 原地不动，
    /// 避免键盘用户在面板里「绕圈」丢失当前位置。
    pub(crate) fn tab(&mut self, form: &SettingsForm, shift: bool) {
        let non_readonly: Vec<usize> = form
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.read_only)
            .map(|(i, _)| i)
            .collect();
        let total = non_readonly.len() + BUTTON_TAB_ORDER.len();
        if total == 0 {
            return;
        }
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
        let next = if shift {
            cur_pos.saturating_sub(1)
        } else {
            (cur_pos + 1).min(total - 1)
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
        // 字体字段：打开可搜索的候选弹层（←/→ 仍保留为快速微调）。
        if matches!(
            form.fields.get(self.focused_field).map(|f| &f.kind),
            Some(FieldKind::FontFamily)
        ) {
            form.open_font_picker();
            return;
        }
        if matches!(
            form.fields.get(self.focused_field).map(|f| &f.kind),
            Some(FieldKind::DocMargin)
        ) {
            form.cycle_doc_margin(1);
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
            (FieldKind::UiLanguage, FieldValue::UiLanguage(l)) => {
                *l = match *l {
                    UiLanguage::Auto => UiLanguage::ZhCn,
                    UiLanguage::ZhCn => UiLanguage::EnUs,
                    UiLanguage::EnUs => UiLanguage::Auto,
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
        // 字体弹层优先：点候选即选中关闭；点其它位置则先关闭弹层再按常规路由。
        if form.font_picker.is_some() {
            if let Some(g) = font_picker_geom(layout, form) {
                if hit(mouse, &g.rect) {
                    let row = ((mouse.1 - g.rect.y) / g.row_h).floor().max(0.0) as usize;
                    let row = row.min(g.rows.saturating_sub(1));
                    let top = form.font_picker.as_ref().map_or(0, |p| p.top);
                    form.pick_font_hit(top + row);
                    return None;
                }
            }
            form.close_font_picker();
        }
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
        // 字体弹层打开时独占键盘：输入即筛选，↑↓ 选择，Enter 确认，Esc 取消。
        if form.font_picker.is_some() {
            match key {
                InputKey::Escape => form.close_font_picker(),
                InputKey::Up => form.font_picker_move(-1),
                InputKey::Down => form.font_picker_move(1),
                InputKey::PageUp => form.font_picker_move(-(FONT_PICKER_MAX_ROWS as i32)),
                InputKey::PageDown => form.font_picker_move(FONT_PICKER_MAX_ROWS as i32),
                InputKey::Enter => form.apply_font_picker(),
                InputKey::Backspace => form.font_query_pop(),
                InputKey::Text(t) => form.font_query_push(t),
                _ => {}
            }
            return None;
        }
        match key {
            InputKey::Escape => {
                // 返回最后一个按钮 id（通常是 Cancel）
                layout.button_ids.last().copied()
            }
            InputKey::Enter => {
                // 字段上 Enter 与 Space 等价：打开字体弹层 / 切换主题、语言、开关。
                // 只有焦点落在按钮上时才作为「按下按钮」返回 id。
                match self.focused_button {
                    Some(i) => layout.button_ids.get(i).copied(),
                    None => {
                        self.toggle_field(form);
                        None
                    }
                }
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
                    .map_or(false, |f| matches!(f.kind, FieldKind::FontFamily)) =>
            {
                form.cycle_font(-1);
                None
            }
            InputKey::Right
                if form
                    .fields
                    .get(self.focused_field)
                    .map_or(false, |f| matches!(f.kind, FieldKind::FontFamily)) =>
            {
                form.cycle_font(1);
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
        // 滚动隐藏的行是空矩形：不画标签也不画控件，避免溢到按钮区。
        if fr.control.w <= 0.0 || fr.control.h <= 0.0 {
            continue;
        }
        let is_focused = state.focused_button.is_none() && state.focused_field == i;
        // 字体弹层打开时，字体控件本身变成查询输入框（无独立搜索行，控件即搜索框）。
        let query = if matches!(f.kind, FieldKind::FontFamily) {
            form.font_picker.as_ref().map(|p| p.query.as_str())
        } else {
            None
        };
        paint_field_label(frame, text, f, fr, theme);
        paint_field_control(frame, text, f, fr, theme, scale, is_focused, query);
    }
    paint_font_picker(frame, text, form, layout, theme, scale);

    // 字族缺中文提示：不阻塞选择，仅说明正文会回退到系统字体。
    if let Some(r) = layout.font_hint {
        let line = elide_with_ellipsis(t(Key::SettingsFontMissingCjk), r.w, |s| text.width(s));
        text.draw(frame, &line, theme.muted, r.x as i32, r.y as i32);
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

#[allow(clippy::too_many_arguments)] // 绘制原语：控件/主题/缩放/焦点/查询天然多参
fn paint_field_control(
    frame: &mut Frame,
    text: &mut dyn Text,
    f: &SettingsField,
    fr: &FieldRects,
    theme: &crate::theme::Theme,
    scale: f32,
    is_focused: bool,
    font_query: Option<&str>,
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

    // 焦点环先画（在值文本下层），避免覆盖文本；与模态/查找共用对比度与 DPI 策略。
    if is_focused {
        let inner = palette::surface_popup(theme);
        let ring = modal::focus_ring_color(theme, [inner[0], inner[1], inner[2]]);
        modal::focus_ring(frame, c, cr, ring, inner, scale);
    }

    // 控件值文本（在焦点环上层，始终可见）
    let inner_x = c.x + 6.0 * scale;
    let inner_y = c.y + (c.h - typography::UI_LINE_HEIGHT * scale) * 0.5;
    let value_text = match font_query {
        Some("") => t(Key::SettingsFontSearch).to_string(),
        Some(q) => q.to_string(),
        None => field_value_display(f),
    };
    // 字族名可达 64 字节，按控件可用宽度截断，避免压到卡片边缘。
    let fitted = elide_with_ellipsis(&value_text, (c.w - 12.0 * scale).max(0.0), |s| {
        text.width(s)
    });

    text.draw(
        frame,
        &fitted,
        if is_focused { theme.text } else { theme.muted },
        inner_x as i32,
        inner_y as i32,
    );
}

/// 字体候选弹层：选中行用 hover 底，其余行 muted；无匹配时给一行提示。
fn paint_font_picker(
    frame: &mut Frame,
    text: &mut dyn Text,
    form: &SettingsForm,
    layout: &SettingsLayout,
    theme: &crate::theme::Theme,
    scale: f32,
) {
    let Some(g) = font_picker_geom(layout, form) else {
        return;
    };
    let Some(picker) = form.font_picker.as_ref() else {
        return;
    };
    let cr = surface::corner_radius(scale);
    stroke_frame(
        frame,
        g.rect,
        cr,
        [
            theme.chrome_border[0],
            theme.chrome_border[1],
            theme.chrome_border[2],
        ],
        palette::surface_popup(theme),
    );
    let line_h = typography::UI_LINE_HEIGHT * scale;
    let pad_x = 6.0 * scale;
    let x = (g.rect.x + pad_x) as i32;
    if picker.hits.is_empty() {
        let ty = g.rect.y + (g.row_h - line_h) * 0.5;
        text.draw(
            frame,
            t(Key::SettingsFontNoMatch),
            theme.muted,
            x,
            ty.round() as i32,
        );
        return;
    }
    for row in 0..g.rows {
        let Some(&opt) = picker.hits.get(picker.top + row) else {
            break;
        };
        let row_y = g.rect.y + row as f32 * g.row_h;
        let selected = picker.top + row == picker.sel;
        if selected {
            frame.fill_rect(
                g.rect.x.round() as i32,
                row_y.round() as i32,
                g.rect.w.max(0.0) as u32,
                g.row_h.ceil() as u32,
                theme.hover,
            );
        }
        let label = form.font_options[opt].display();
        let fitted = elide_with_ellipsis(&label, (g.rect.w - 12.0 * scale).max(0.0), |s| {
            text.width(s)
        });
        let ty = row_y + (g.row_h - line_h) * 0.5;
        text.draw(
            frame,
            &fitted,
            if selected { theme.text } else { theme.muted },
            x,
            ty.round() as i32,
        );
    }
}

fn field_value_display(f: &SettingsField) -> String {
    match &f.value {
        FieldValue::ThemeMode(m) => match m {
            ThemeMode::System => t(Key::SettingsFollowSystem).into(),
            ThemeMode::Light => t(Key::SettingsLight).into(),
            ThemeMode::Dark => t(Key::SettingsDark).into(),
        },
        FieldValue::UiLanguage(l) => match l {
            UiLanguage::Auto => t(Key::SettingsLangAuto).into(),
            UiLanguage::ZhCn => t(Key::SettingsLangZhCn).into(),
            UiLanguage::EnUs => t(Key::SettingsLangEnUs).into(),
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
        FieldValue::FontFamily(sel) => sel.display(),
        FieldValue::DocMargin(m) => match m {
            DocMargin::Compact => t(Key::SettingsMarginCompact).into(),
            DocMargin::Standard => t(Key::SettingsMarginStandard).into(),
            DocMargin::Relaxed => t(Key::SettingsMarginRelaxed).into(),
            DocMargin::Full => t(Key::SettingsMarginFull).into(),
            DocMargin::Custom => t(Key::SettingsMarginCustom).into(),
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

fn paint_card(frame: &mut Frame, scale: f32, theme: &crate::theme::Theme, card: Rect) {
    let x = card.x.round() as i32;
    let y = card.y.round() as i32;
    let w = card.w.max(0.0).round() as u32;
    let h = card.h.max(0.0).round() as u32;
    let r = surface::dialog_radius(scale);
    frame.fill_shadow_high(x, y, w, h, r, scale, theme.dark);
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
                    modal::focus_ring(frame, rect, radius, fg, inner, scale);
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
                    let inner = if hovered {
                        theme.hover
                    } else {
                        palette::surface_popup(theme)
                    };
                    let ring = modal::focus_ring_color(theme, [inner[0], inner[1], inner[2]]);
                    modal::focus_ring(frame, rect, radius, ring, inner, scale);
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
    use crate::i18n::{translate, LOCALES};
    use crate::input::Mods;
    use std::time::Instant;

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
        layout(vp, 1.0, form, &mut text, &buttons(), 0)
    }

    fn layout_scrolled(
        vp: Rect,
        form: &SettingsForm,
        scale: f32,
        scroll_rows: usize,
    ) -> SettingsLayout {
        let mut text = FakeText;
        layout(vp, scale, form, &mut text, &buttons(), scroll_rows)
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

        let visible: Vec<&FieldRects> = layout
            .fields
            .iter()
            .filter(|fr| fr.control.w > 0.0)
            .collect();
        for (index, pair) in visible.windows(2).enumerate() {
            let previous = pair[0];
            let next = pair[1];
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

    /// DEF-T07-E1-001：低高度下只布局可见行，隐藏行的零矩形不得进入卡片/按钮区。
    #[test]
    fn low_height_rows_do_not_overflow_card_or_overlap_buttons() {
        let form = default_form();
        // 150% DPI 下 720px 高窗口：9 行放不下，必须截断/滚动。
        let vp = viewport(720.0, 720.0);
        let layout = layout_scrolled(vp, &form, 1.5, 0);
        assert!(layout.shown_rows < form.fields.len(), "低高度应截断可见行");
        assert!(layout.max_scroll > 0, "应提供滚动余量");

        let full = layout_scrolled(vp, &form, 1.5, layout.max_scroll);
        let card = full.card;
        for fr in &full.fields {
            if fr.control.w <= 0.0 {
                continue;
            }
            assert!(
                fr.control.y + fr.control.h <= card.y + card.h + EPSILON,
                "字段行越过卡片底缘: {fr:?} card={card:?}"
            );
            for button in &full.buttons {
                assert!(
                    fr.control.y + fr.control.h <= button.y + EPSILON
                        || fr.control.y >= button.y + button.h + EPSILON,
                    "字段行与按钮重叠: {fr:?} button={button:?}"
                );
            }
        }
        // 滚到底后最后一行必须可见（可编辑）。
        let last = full.fields.last().expect("字段存在");
        assert!(last.control.w > 0.0, "滚动到底后最后一行必须可见");
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

    /// 回归：英文长标签必须落在 320px 标签列上限内，且 760 宽卡片下控件列不被压缩。
    /// 最长标签为 `SettingsExportToc`（"Include Table of Contents in PDF Export"，
    /// FakeText 8px/char ≈ 312px）。语言在进程内固定，布局按当前语言取宽，
    /// 因此这里直接对英文词条与容量上限做断言，不依赖 zh-CN 布局。
    #[test]
    fn english_labels_fit_inside_label_column() {
        let form = default_form();
        let mut text = FakeText;
        let mut widest: Option<(Key, f32)> = None;
        for field in &form.fields {
            let en = translate(LOCALES, "en-US", field.label)
                .unwrap_or_else(|| panic!("缺少英文词条: {:?}", field.label));
            let w = text.width(en);
            if widest.map_or(true, |(_, max)| w >= max) {
                widest = Some((field.label, w));
            }
        }
        let (widest_key, widest_w) = widest.expect("表单至少有一个字段");

        assert_eq!(
            widest_key,
            Key::SettingsExportToc,
            "最长英文标签应覆盖「导出 PDF 时包含目录页」"
        );
        let label_cap = 320.0;
        assert!(
            widest_w <= label_cap + EPSILON,
            "最长英文标签 {widest_w}px 超出标签列上限 {label_cap}px（{widest_key:?}）"
        );

        // 760 宽（DIALOG_MAX_WIDTH）卡片 + 320 标签上限：label_w 可达上限，控件列仍充足。
        let (label_w, control_w) =
            label_and_control_widths(surface::DIALOG_MAX_WIDTH, 1.0, label_cap);
        assert!(
            label_w >= widest_w - EPSILON,
            "760 宽卡片下标签列 {label_w}px 不足以容纳最长英文标签 {widest_w}px"
        );
        assert!(
            control_w >= 350.0,
            "标签列变宽后控件列不得被压缩: {control_w}"
        );
    }

    /// zh-CN：按当前语言取宽，标签不越界、控件列可用，且不按英文宽度预留空白。
    #[test]
    fn zh_labels_fit_and_control_column_stays_usable() {
        let form = default_form();
        // 新增「正文行高」后 14 行在 800 高下会触发滚动；用足够高的视口验证全部行。
        let layout = layout_for(viewport(1280.0, 1000.0), &form);
        let mut text = FakeText;
        let mut widest_zh = 0.0f32;
        for (field, rects) in form.fields.iter().zip(&layout.fields) {
            assert!(rects.control.w > 0.0, "控件列必须有宽度: {:?}", field.label);
            let w = text.width(t(field.label));
            widest_zh = widest_zh.max(w);
            assert!(
                w <= rects.label.w + EPSILON,
                "zh 标签越界: {:?} {w}px > {}px",
                field.label,
                rects.label.w
            );
        }
        assert!(widest_zh < 320.0, "zh 最长标签不应达到英文标签上限");
        assert!(
            layout.card.w < surface::DIALOG_MAX_WIDTH,
            "zh 卡片不应按英文标签预留到最大宽度: {}",
            layout.card.w
        );
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

    fn paint_for_test(
        theme: &crate::theme::Theme,
        scale: f32,
        form: &SettingsForm,
        state: &SettingsState,
        layout: &SettingsLayout,
    ) -> Frame {
        let vp = viewport(1280.0, 800.0);
        let mut frame = Frame::new(vp.w as u32, vp.h as u32, palette::surface_base(theme));
        let mut text = FakeText;
        paint(
            &mut frame,
            vp,
            scale,
            theme,
            form,
            state,
            layout,
            &buttons(),
            &mut text,
        );
        frame
    }

    fn top_edge_pixel(frame: &Frame, rect: Rect, offset: u32) -> [u8; 3] {
        let pixel = frame.get(
            (rect.x + rect.w * 0.5).round() as u32,
            rect.y.round() as u32 + offset,
        );
        [pixel[0], pixel[1], pixel[2]]
    }

    #[test]
    fn focused_field_and_secondary_use_shared_contrast_policy() {
        for theme in [
            crate::theme::Theme::light(),
            crate::theme::Theme::dark(),
            crate::theme::Theme::high_contrast(),
        ] {
            let form = default_form();
            let layout = layout_scrolled(viewport(1280.0, 800.0), &form, 1.0, 0);
            let popup = palette::surface_popup(&theme);

            let field_state = SettingsState::new(&form);
            let field = paint_for_test(&theme, 1.0, &form, &field_state, &layout);
            let field_rect = layout.fields[field_state.focused_field].control;
            let expected = modal::focus_ring_color(&theme, [popup[0], popup[1], popup[2]]);
            assert_eq!(
                top_edge_pixel(&field, field_rect, 1),
                expected,
                "字段焦点色必须共享模态策略: dark={}",
                theme.dark
            );

            let mut button_state = SettingsState::new(&form);
            focus_button(&mut button_state, &layout, BTN_CANCEL);
            let button_index = button_state.focused_button.expect("Cancel 焦点必须存在");
            let button_rect = layout.buttons[button_index];
            for hovered in [false, true] {
                button_state.hovered_button = hovered.then_some(button_index);
                let frame = paint_for_test(&theme, 1.0, &form, &button_state, &layout);
                let inner = if hovered { theme.hover } else { popup };
                let expected = modal::focus_ring_color(&theme, [inner[0], inner[1], inner[2]]);
                assert_eq!(
                    top_edge_pixel(&frame, button_rect, 1),
                    expected,
                    "Secondary 焦点色必须匹配实际底色: dark={} hovered={hovered}",
                    theme.dark
                );
            }
        }
    }

    #[test]
    fn field_and_button_focus_rings_share_dpi_thickness() {
        for theme in [
            crate::theme::Theme::light(),
            crate::theme::Theme::dark(),
            crate::theme::Theme::high_contrast(),
        ] {
            let form = default_form();
            let scale = 1.5;
            let layout = layout_scrolled(viewport(1280.0, 800.0), &form, scale, 0);
            let popup = palette::surface_popup(&theme);
            let popup_rgb = [popup[0], popup[1], popup[2]];

            let field_state = SettingsState::new(&form);
            let mut primary_state = SettingsState::new(&form);
            focus_button(&mut primary_state, &layout, BTN_OK);
            let primary_index = primary_state.focused_button.expect("OK 焦点必须存在");
            let mut secondary_state = SettingsState::new(&form);
            focus_button(&mut secondary_state, &layout, BTN_CANCEL);
            let secondary_index = secondary_state.focused_button.expect("Cancel 焦点必须存在");

            let cases = [
                (
                    field_state,
                    layout.fields[field_state.focused_field].control,
                    modal::focus_ring_color(&theme, popup_rgb),
                    popup,
                ),
                (
                    primary_state,
                    layout.buttons[primary_index],
                    theme.accent_text,
                    [theme.accent[0], theme.accent[1], theme.accent[2], 255],
                ),
                (
                    secondary_state,
                    layout.buttons[secondary_index],
                    modal::focus_ring_color(&theme, popup_rgb),
                    popup,
                ),
            ];
            for (state, rect, ring, inner) in cases {
                let frame = paint_for_test(&theme, scale, &form, &state, &layout);
                for offset in [1, 2] {
                    assert_eq!(
                        top_edge_pixel(&frame, rect, offset),
                        ring,
                        "150% DPI 焦点环必须为两个物理像素: dark={} offset={offset}",
                        theme.dark
                    );
                }
                assert_eq!(
                    top_edge_pixel(&frame, rect, 3),
                    [inner[0], inner[1], inner[2]],
                    "焦点环内侧必须恢复实际底色: dark={}",
                    theme.dark
                );
            }
        }
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

        // 首项再 Shift+Tab：线性不环形，位置保持不动。
        state.tab(&form, true);
        assert_eq!(state.focused_field, editable[0]);
        assert_eq!(state.focused_button, None);

        // 先走到末项（最后一个按钮 Cancel），再反向走完整条链。
        for _ in 0..(editable.len() + BUTTON_TAB_ORDER.len() - 1) {
            state.tab(&form, false);
        }
        let cancel = layout
            .button_ids
            .iter()
            .position(|&id| id == BTN_CANCEL)
            .expect("Cancel 按钮必须存在");
        assert_eq!(state.focused_button, Some(cancel));

        for expected_id in [BTN_RESET_DEFAULTS, BTN_OK] {
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

        // 已在首项：Shift+Tab 停在原地，不回环到末项。
        let first = state.focused_field;
        state.tab(&form, true);
        assert_eq!(state.focused_field, first);
        assert_eq!(state.focused_button, None);
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
        for expected_id in [BTN_OK, BTN_RESET_DEFAULTS, BTN_CANCEL] {
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

        // 末项再 Tab：线性不环形，保持在最后一个按钮上。
        state.tab(&form, false);
        let cancel = layout
            .button_ids
            .iter()
            .position(|&id| id == BTN_CANCEL)
            .expect("Cancel 按钮必须存在");
        assert_eq!(state.focused_button, Some(cancel));
    }

    /// Tab 顺序数组：字段按视觉行序 → OK → Restore Defaults → Cancel，线性不跳不回环。
    #[test]
    fn tab_order_matches_linear_visual_reading_order() {
        #[derive(Debug, PartialEq, Eq)]
        enum Stop {
            Field(usize),
            Button(u16),
        }

        let form = default_form();
        let layout = layout_for(viewport(1280.0, 800.0), &form);
        let editable = editable_indices(&form);
        let mut state = SettingsState::new(&form);
        let mut order = vec![Stop::Field(state.focused_field)];
        for _ in 1..(editable.len() + BUTTON_TAB_ORDER.len()) {
            state.tab(&form, false);
            order.push(match state.focused_button {
                Some(i) => Stop::Button(layout.button_ids[i]),
                None => Stop::Field(state.focused_field),
            });
        }

        let mut expected: Vec<Stop> = editable.into_iter().map(Stop::Field).collect();
        expected.extend([
            Stop::Button(BTN_OK),
            Stop::Button(BTN_RESET_DEFAULTS),
            Stop::Button(BTN_CANCEL),
        ]);
        assert_eq!(order, expected, "Tab 顺序必须是可预期的线性阅读顺序");
    }

    /// Esc 分级：弹层打开时第一次 Esc 只关弹层，第二次才关整个设置模态。
    #[test]
    fn escape_closes_font_picker_before_settings_modal() {
        let mut form = SettingsForm::from_config_with_families(
            &Config::default(),
            &[("Arial".to_string(), false)],
        );
        let layout = layout_for(viewport(1280.0, 800.0), &form);
        let mut state = SettingsState::new(&form);
        form.open_font_picker();

        assert_eq!(
            state.on_key(&InputKey::Escape, Mods::default(), &mut form, &layout),
            None,
            "弹层打开时 Esc 不直接关闭模态"
        );
        assert!(form.font_picker.is_none(), "第一次 Esc 只关闭字体弹层");
        assert_eq!(
            state.on_key(&InputKey::Escape, Mods::default(), &mut form, &layout),
            Some(BTN_CANCEL),
            "第二次 Esc 才以取消关闭整个设置模态"
        );
    }

    /// 源码纯文本行开关：绑定 Config.source_plain_lines；Space/Enter 翻转并即时反映到 apply 结果。
    #[test]
    fn source_plain_lines_toggle_binds_and_writes_back_config() {
        let config = Config::default();
        assert!(
            config.source_plain_lines,
            "默认开启纯文本行视图（空行可见）"
        );

        let mut form = SettingsForm::from_config(&config);
        let idx = form
            .fields
            .iter()
            .position(|f| f.label == Key::SettingsSourcePlainLines)
            .expect("源码纯文本行开关必须存在");
        assert!(matches!(form.fields[idx].value, FieldValue::Toggle(true)));
        let layout = layout_for(viewport(1280.0, 800.0), &form);
        let mut state = SettingsState::new(&form);
        state.focused_field = idx;
        state.focused_button = None;

        let space = InputKey::Text(" ".to_string());
        assert_eq!(
            state.on_key(&space, Mods::default(), &mut form, &layout),
            None
        );
        assert!(
            !form.apply(&config).source_plain_lines,
            "Space 必须写入关闭"
        );

        assert_eq!(
            state.on_key(&InputKey::Enter, Mods::default(), &mut form, &layout),
            None,
            "字段上的 Enter 与 Space 等价"
        );
        assert!(form.apply(&config).source_plain_lines, "Enter 必须写回开启");

        form.fields[idx].value = FieldValue::Toggle(false);
        assert!(
            form.reset_defaults().apply(&config).source_plain_lines,
            "恢复默认必须回到开启"
        );
    }

    /// Space / Enter 在字体字段上打开候选弹层（不是一次循环）。
    #[test]
    fn space_and_enter_open_font_picker() {
        for key in [InputKey::Text(" ".to_string()), InputKey::Enter] {
            let mut form = SettingsForm::from_config_with_families(
                &Config::default(),
                &[("Arial".to_string(), false)],
            );
            let layout = layout_for(viewport(1280.0, 800.0), &form);
            let mut state = SettingsState::new(&form);
            state.focused_field = font_field_index(&form);
            state.focused_button = None;

            assert_eq!(
                state.on_key(&key, Mods::default(), &mut form, &layout),
                None
            );
            assert!(form.font_picker.is_some(), "Space/Enter 必须打开字体弹层");
        }
    }

    /// 数值项：方向键微调（既有实现），左右与上下都按步长增减并夹取范围。
    #[test]
    fn arrow_keys_adjust_number_fields() {
        let mut form = SettingsForm::from_config(&Config::default());
        let layout = layout_for(viewport(1280.0, 800.0), &form);
        let mut state = SettingsState::new(&form);
        let idx = form
            .fields
            .iter()
            .position(|f| f.label == Key::SettingsDocFontSize)
            .expect("正文字号字段必须存在");
        state.focused_field = idx;
        state.focused_button = None;

        let number = |form: &SettingsForm| match form.fields[idx].value {
            FieldValue::Number(v) => v,
            _ => panic!("字号字段必须是数值"),
        };
        let start = number(&form);

        state.on_key(&InputKey::Right, Mods::default(), &mut form, &layout);
        assert_eq!(number(&form), start + 1.0, "Right 增加一个步长");
        state.on_key(&InputKey::Left, Mods::default(), &mut form, &layout);
        assert_eq!(number(&form), start, "Left 回落一个步长");
        state.on_key(&InputKey::Down, Mods::default(), &mut form, &layout);
        assert_eq!(number(&form), start + 1.0, "Down 增加一个步长");
        state.on_key(&InputKey::Up, Mods::default(), &mut form, &layout);
        assert_eq!(number(&form), start, "Up 减少一个步长");

        form.fields[idx].value = FieldValue::Number(32.0);
        state.on_key(&InputKey::Right, Mods::default(), &mut form, &layout);
        assert_eq!(number(&form), 32.0, "方向键不得越过上限");
        form.fields[idx].value = FieldValue::Number(10.0);
        state.on_key(&InputKey::Left, Mods::default(), &mut form, &layout);
        assert_eq!(number(&form), 10.0, "方向键不得越过下限");
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

    /// DEF-T07-002：几何字段参与 apply；清除/恢复默认后的 None 必须覆盖旧值。
    #[test]
    fn geometry_field_participates_in_apply_and_can_be_cleared() {
        let config = Config {
            window: Some(crate::theme::WindowGeom {
                width: 900,
                height: 700,
                x: -40,
                y: 30,
            }),
            ..Config::default()
        };
        let mut form = SettingsForm::from_config(&config);
        assert_eq!(
            form.apply(&config).window,
            config.window,
            "未改动时几何原样保留"
        );

        // 模拟「清除」：值变 None，OK 后必须移除 window。
        let geom_idx = form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::WindowGeom))
            .expect("几何字段存在");
        form.fields[geom_idx].value = FieldValue::Geom(None);
        assert_eq!(
            form.apply(&config).window,
            None,
            "清除后 apply 不得复活旧几何"
        );

        // 模拟「恢复默认」：默认 Config 无几何记录。
        let defaults = SettingsForm::from_config(&Config::default());
        assert_eq!(defaults.apply(&config).window, None);
    }

    /// 版心档位：五档循环；apply 写回 Config，自定义像素值夹取 4..=200。
    #[test]
    fn doc_margin_field_cycles_and_applies() {
        let config = Config::default();
        let mut form = SettingsForm::from_config(&config);
        let idx = form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::DocMargin))
            .expect("版心字段存在");
        assert_eq!(
            field_value_display(&form.fields[idx]),
            t(Key::SettingsMarginStandard)
        );

        // 循环顺序：标准 → 宽松 → 全宽 → 自定义 → 紧凑。
        for want in [
            DocMargin::Relaxed,
            DocMargin::Full,
            DocMargin::Custom,
            DocMargin::Compact,
        ] {
            form.cycle_doc_margin(1);
            assert!(matches!(
                form.fields[idx].value,
                FieldValue::DocMargin(m) if m == want
            ));
        }
        let out = form.apply(&config);
        assert_eq!(out.doc_margin, DocMargin::Compact);

        // 自定义像素字段：越界值在 apply 时夹取。
        let px_idx = form
            .fields
            .iter()
            .position(|f| f.label == Key::SettingsDocMarginPx)
            .expect("自定义版心边距字段存在");
        form.fields[px_idx].value = FieldValue::Number(500.0);
        assert_eq!(form.apply(&config).doc_margin_px, 200.0);
        form.fields[px_idx].value = FieldValue::Number(0.0);
        assert_eq!(form.apply(&config).doc_margin_px, 4.0);
        form.fields[px_idx].value = FieldValue::Number(48.0);
        assert_eq!(form.apply(&config).doc_margin_px, 48.0);
    }

    /// PDF 页边距字段与配置/PDF 共用 5–50mm：UI 不提供会被排版静默修正的值。
    #[test]
    fn export_margin_field_range_matches_pdf_interval() {
        let config = Config::default();
        let mut form = SettingsForm::from_config(&config);
        let idx = form
            .fields
            .iter()
            .position(|f| f.label == Key::SettingsExportMarginMm)
            .expect("PDF 页边距字段存在");
        assert!(
            matches!(&form.fields[idx].kind, FieldKind::Number(r)
                if r.min == 5.0 && r.max == 50.0),
            "页边距可选范围必须与 PDF 排版一致（5–50mm）"
        );
        form.fields[idx].value = FieldValue::Number(0.0);
        assert_eq!(form.apply(&config).export_margin_mm, 5.0);
        form.fields[idx].value = FieldValue::Number(80.0);
        assert_eq!(form.apply(&config).export_margin_mm, 50.0);
        form.fields[idx].value = FieldValue::Number(20.0);
        assert_eq!(form.apply(&config).export_margin_mm, 20.0);
    }

    /// 正文行高：设置表单可编辑 10..=96，apply 时按字号夹到 [字号, 3×字号]。
    #[test]
    fn doc_line_height_field_applies_clamped_to_font_size() {        let config = Config::default();
        let mut form = SettingsForm::from_config(&config);
        let idx = form
            .fields
            .iter()
            .position(|f| f.label == Key::SettingsDocLineHeight)
            .expect("正文行高字段存在");
        assert!(
            matches!(&form.fields[idx].kind, FieldKind::Number(r)
                if r.min == 10.0 && r.max == 96.0 && r.step == 1.0 && r.integers_only),
            "行高字段应为整数步进 10..=96"
        );

        // 默认 16 字号：低位夹到 16、高位夹到 48。
        form.fields[idx].value = FieldValue::Number(5.0);
        assert_eq!(form.apply(&config).doc_line_height, 16.0);
        form.fields[idx].value = FieldValue::Number(96.0);
        assert_eq!(form.apply(&config).doc_line_height, 48.0);
        form.fields[idx].value = FieldValue::Number(24.0);
        assert_eq!(form.apply(&config).doc_line_height, 24.0);

        // 字号变大后下限跟随字号；字号先于行高写入 out。
        let font_idx = form
            .fields
            .iter()
            .position(|f| f.label == Key::SettingsDocFontSize)
            .expect("正文字号字段存在");
        form.fields[font_idx].value = FieldValue::Number(24.0);
        form.fields[idx].value = FieldValue::Number(10.0);
        assert_eq!(form.apply(&config).doc_line_height, 24.0);

        // 未改动时保留配置原值（默认 22）。
        assert_eq!(
            SettingsForm::from_config(&Config::default())
                .apply(&config)
                .doc_line_height,
            22.0
        );
    }

    /// 界面语言：三档循环；apply 写回 Config（会话内不热切换，重启后生效）。
    #[test]
    fn ui_language_field_cycles_and_applies() {
        let config = Config::default();
        let mut form = SettingsForm::from_config(&config);
        let idx = form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::UiLanguage))
            .expect("界面语言字段存在");
        assert_eq!(
            field_value_display(&form.fields[idx]),
            t(Key::SettingsLangAuto)
        );

        // 循环顺序：自动 → 中文 → English → 自动。
        let mut state = SettingsState::new(&form);
        state.focused_field = idx;
        state.focused_button = None;
        for (want, display) in [
            (UiLanguage::ZhCn, t(Key::SettingsLangZhCn)),
            (UiLanguage::EnUs, t(Key::SettingsLangEnUs)),
            (UiLanguage::Auto, t(Key::SettingsLangAuto)),
        ] {
            state.toggle_field(&mut form);
            assert!(matches!(
                form.fields[idx].value,
                FieldValue::UiLanguage(l) if l == want
            ));
            assert_eq!(field_value_display(&form.fields[idx]), display);
        }

        form.fields[idx].value = FieldValue::UiLanguage(UiLanguage::EnUs);
        assert_eq!(form.apply(&config).ui_language, UiLanguage::EnUs);
        // 恢复默认：回到自动跟随系统。
        assert_eq!(
            form.reset_defaults().apply(&config).ui_language,
            UiLanguage::Auto
        );
    }

    fn font_field_index(form: &SettingsForm) -> usize {
        form.fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::FontFamily))
            .expect("字体字段存在")
    }

    /// 自由字族：可选任意已安装族；apply 写入 `doc_font_face`，切回预置则清空。
    #[test]
    fn font_field_cycles_installed_families_and_applies_choice() {
        let config = Config {
            doc_font_family: DocFontFamily::Serif,
            ..Config::default()
        };
        // 含中文族应排在其它族之前。
        let installed = vec![
            ("Arial".to_string(), false),
            ("Consolas".to_string(), false),
            ("SimSun".to_string(), true),
        ];
        let mut form = SettingsForm::from_config_with_families(&config, &installed);
        let cjk_pos = form
            .font_options
            .iter()
            .position(|o| matches!(o, FontSelection::Installed { name, .. } if name == "SimSun"))
            .expect("SimSun 应可选");
        let latin_pos = form
            .font_options
            .iter()
            .position(|o| matches!(o, FontSelection::Installed { name, .. } if name == "Arial"))
            .expect("Arial 应可选");
        assert!(cjk_pos < latin_pos, "含中文字族应排在其它族之前");

        let target = FontSelection::Installed {
            name: "SimSun".to_string(),
            cjk: true,
        };
        let mut steps = 0;
        while form.font_selection() != Some(&target) {
            form.cycle_font(1);
            steps += 1;
            assert!(
                steps <= form.font_options.len(),
                "循环切换应能在列表长度内到达任意字族"
            );
        }
        let out = form.apply(&config);
        assert_eq!(out.doc_font_face.as_deref(), Some("SimSun"));
        assert_eq!(out.doc_font_family, DocFontFamily::Serif, "预置档位保留");

        // 切回预置：显式族名必须清空，避免预置被覆盖。
        let idx = font_field_index(&form);
        form.fields[idx].value = FieldValue::FontFamily(FontSelection::Preset(DocFontFamily::Sans));
        let out = form.apply(&config);
        assert_eq!(out.doc_font_face, None);
        assert_eq!(out.doc_font_family, DocFontFamily::Sans);
        assert_eq!(font_field_index(&form), idx);
    }

    /// 字体弹层：输入即筛选、Enter 写回；无匹配时保持原值；空格/点击打开的是弹层而非循环。
    #[test]
    fn font_picker_filters_and_applies() {
        let installed = vec![
            ("Consolas".to_string(), false),
            ("Cascadia Code".to_string(), false),
            ("SimSun".to_string(), true),
        ];
        let config = Config::default();
        let mut form = SettingsForm::from_config_with_families(&config, &installed);

        // 空格/点击进入的是可搜索弹层，而不是一次循环。
        let idx = font_field_index(&form);
        let mut state = SettingsState::new(&form);
        state.focused_field = idx;
        state.focused_button = None;
        state.toggle_field(&mut form);
        assert!(form.font_picker.is_some(), "字体字段应打开候选弹层");

        form.font_query_push("sim");
        let hit = form.font_picker.as_ref().unwrap().hits.clone();
        assert_eq!(hit.len(), 1, "查询应过滤到唯一候选");
        assert!(matches!(
            &form.font_options[hit[0]],
            FontSelection::Installed { name, .. } if name == "SimSun"
        ));

        form.apply_font_picker();
        assert!(form.font_picker.is_none(), "Enter 后弹层关闭");
        assert_eq!(form.apply(&config).doc_font_face.as_deref(), Some("SimSun"));

        // 无匹配：Enter 不改变已选值。
        form.open_font_picker();
        form.font_query_push("no-such-font");
        assert!(form.font_picker.as_ref().unwrap().hits.is_empty());
        form.apply_font_picker();
        assert_eq!(form.apply(&config).doc_font_face.as_deref(), Some("SimSun"));
    }

    /// 弹层键盘移动：越界循环，且选中项始终留在可视窗口内。
    #[test]
    fn font_picker_move_wraps_and_keeps_selection_visible() {
        let installed: Vec<(String, bool)> =
            (0..20).map(|i| (format!("Font{i:02}"), false)).collect();
        let mut form = SettingsForm::from_config_with_families(&Config::default(), &installed);
        form.open_font_picker();
        form.font_picker_move(-1); // 从 0 往上越界 → 末项
        let p = form.font_picker.as_ref().unwrap();
        assert_eq!(p.sel, p.hits.len() - 1);
        assert!(p.top + FONT_PICKER_MAX_ROWS > p.sel, "末项必须可见");
        form.font_picker_move(1);
        assert_eq!(
            form.font_picker.as_ref().unwrap().sel,
            0,
            "再下一项回到首项"
        );
    }

    /// 大列表首帧预算：220 个已安装字族下，构造表单 + 打开弹层 + 首次布局。
    /// 口径：使用 `FakeText` 的度量接缝，覆盖真实热点——220 条候选表构建、
    /// 打开弹层后的过滤，以及首帧可见窗口（8 行）的布局。
    ///
    /// 诚实标注：本测试**不含整帧像素光栅化**（遮罩/卡片/阴影的软件填充）。
    /// 实测（同一机器）：本路径 release 96µs / debug 360µs；
    /// 若把 `Frame::new` + `paint` 也计入，release 整帧约 13.8ms，
    /// 开销主要来自 1280×800 遮罩与卡片填充，与字族数量无关。
    /// 本测试锁定的正是「200+ 字族」这条路径，debug 与 release 均按 16ms 断言。
    #[test]
    fn font_picker_220_families_first_frame_within_budget() {
        let installed: Vec<(String, bool)> = (0..220)
            .map(|i| (format!("Family {i:03}"), i % 3 == 0))
            .collect();

        let started = Instant::now();
        let mut form = SettingsForm::from_config_with_families(&Config::default(), &installed);
        form.open_font_picker();
        let vp = viewport(1280.0, 800.0);
        let buttons = buttons();
        let mut text = FakeText;
        let layout = layout(vp, 1.0, &form, &mut text, &buttons, 0);
        let elapsed = started.elapsed();

        assert!(
            form.font_options.len() >= 220,
            "候选表必须包含 220 个字族: {}",
            form.font_options.len()
        );
        assert_eq!(
            form.font_picker.as_ref().map(|p| p.hits.len()),
            Some(form.font_options.len()),
            "空查询必须命中全部候选"
        );
        assert!(
            font_picker_geom(&layout, &form).is_some(),
            "首帧必须给出弹层几何"
        );
        let budget = std::time::Duration::from_millis(16);
        eprintln!(
            "font picker 220 families first frame (list+layout): {elapsed:?} \
             (budget {budget:?}, debug={})",
            cfg!(debug_assertions)
        );
        assert!(
            elapsed < budget,
            "220 字族首帧超出预算: {elapsed:?} >= {budget:?}"
        );
    }

    /// 缺中文提示：仅具体西文字族触发，并排在该字段下方；预置/含中文族不触发。
    #[test]
    fn font_missing_cjk_hint_only_for_latin_installed_family() {
        let installed = vec![
            ("Consolas".to_string(), false),
            ("SimSun".to_string(), true),
        ];
        let vp = viewport(1280.0, 800.0);

        let latin = Config {
            doc_font_face: Some("Consolas".to_string()),
            ..Config::default()
        };
        let form = SettingsForm::from_config_with_families(&latin, &installed);
        assert!(form.font_missing_cjk(), "西文字族应提示缺中文");
        let layout = layout_for(vp, &form);
        let hint = layout.font_hint.expect("缺中文时必须给出提示行");
        let idx = font_field_index(&form);
        assert!(
            hint.y >= layout.fields[idx].control.y + layout.fields[idx].control.h,
            "提示行必须在字体字段下方"
        );
        if let Some(next) = layout.fields.get(idx + 1).filter(|fr| fr.control.w > 0.0) {
            assert!(
                hint.y + hint.h <= next.control.y + EPSILON,
                "提示行不得与下一字段重叠: {hint:?} next={next:?}"
            );
        }

        let cjk = Config {
            doc_font_face: Some("SimSun".to_string()),
            ..Config::default()
        };
        let form = SettingsForm::from_config_with_families(&cjk, &installed);
        assert!(!form.font_missing_cjk());
        assert!(layout_for(vp, &form).font_hint.is_none());

        let preset = SettingsForm::from_config(&Config::default());
        assert!(!preset.font_missing_cjk());
        assert!(layout_for(vp, &preset).font_hint.is_none());
    }

    /// 配置里的字族已被卸载：仍显示并保留在列表中，提示缺中文但不阻塞选择。
    #[test]
    fn configured_face_missing_from_system_stays_selectable() {
        let cfg = Config {
            doc_font_face: Some("Ghost Font".to_string()),
            ..Config::default()
        };
        let form = SettingsForm::from_config_with_families(&cfg, &[]);
        assert_eq!(
            form.font_selection(),
            Some(&FontSelection::Installed {
                name: "Ghost Font".to_string(),
                cjk: false
            })
        );
        assert!(form
            .font_options
            .iter()
            .any(|o| matches!(o, FontSelection::Installed { name, .. } if name == "Ghost Font")));
        assert!(form.font_missing_cjk());
        let out = form.apply(&cfg);
        assert_eq!(out.doc_font_face.as_deref(), Some("Ghost Font"));
    }
}
