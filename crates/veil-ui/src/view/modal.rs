//! 应用内自绘模态对话框：规格、状态、纯几何布局与绘制。
//!
//! 纯逻辑零 GUI：不使用 Win32 任务对话框，全部走应用自有渲染栈，因此与
//! 菜单/大纲共享 `theme::surface` 令牌；文本度量与栅格化经 `Text` 接缝注入，
//! 本模块只依赖 `Frame` 的像素原语，headless 可测。

use crate::geometry::Rect;
use crate::input::Key;
use crate::theme::{surface, Theme};
use crate::tokens::{palette, typography};
use crate::view::Frame;

/// 按钮横向前后留白（px）：文字两侧各留 12，保证按钮不像贴字。
const BUTTON_PAD_X: f32 = 12.0;

/// 弹窗语义类型：决定标题色与标题左侧徽标；几何/按钮/间距完全共用。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ModalKind {
    /// 普通确认/信息弹窗。
    #[default]
    Normal,
    /// 失败/错误态：危险色标题 + 警示徽标，必须配可操作出口。
    Error,
}

/// 按钮语义角色：决定填充/描边/文字色，避免调用方在绘制层分支。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ButtonRole {
    /// 主操作：accent 填充、反色文字。
    Primary,
    /// 危险操作：仅危险色描边与文字，不抢主操作视觉。
    Danger,
    /// 次要操作：中性描边，可承载取消。
    Secondary,
}

/// 模态按钮：`id` 由调用方定义并在命中时原样返回，避免用下标当业务标识。
///
/// 助记符单独存字段而非写进标签（如「保存(&S)」）：标签是要原样上屏的文字，
/// 我们的绘制层不做 `&` 解析，写进标签会把 `&S` 直接画出来。
#[derive(Clone, Debug)]
pub(crate) struct ModalButton {
    pub(crate) id: u16,
    pub(crate) label: String,
    pub(crate) role: ButtonRole,
    pub(crate) mnemonic: Option<char>,
}

impl ModalButton {
    pub(crate) fn new(id: u16, label: impl Into<String>, role: ButtonRole) -> Self {
        Self {
            id,
            label: label.into(),
            role,
            mnemonic: None,
        }
    }

    /// 附加快捷键（单击字母触发）：中文标签本身不带 ASCII 字母，故显式声明。
    pub(crate) fn with_mnemonic(mut self, c: char) -> Self {
        self.mnemonic = Some(c.to_ascii_lowercase());
        self
    }

    /// 上屏文字：助记符既参与按键匹配，也以半角大写括号后缀上屏（如「保存 (S)」），
    /// 否则用户无从得知该按哪个字母；布局测宽与绘制共用本方法以保证两者一致。
    pub(crate) fn display_label(&self) -> String {
        match self.mnemonic {
            Some(c) => format!("{} ({})", self.label, c.to_ascii_uppercase()),
            None => self.label.clone(),
        }
    }
}

/// 模态内容规格：标题、正文与按钮序列，打开后不变（瞬态弹窗）。
#[derive(Clone, Debug)]
pub(crate) struct ModalSpec {
    pub(crate) title: String,
    pub(crate) message: String,
    pub(crate) buttons: Vec<ModalButton>,
    pub(crate) kind: ModalKind,
}

impl ModalSpec {
    pub(crate) fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        buttons: Vec<ModalButton>,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            buttons,
            kind: ModalKind::Normal,
        }
    }

    /// 错误态弹窗：同一套卡片/按钮/间距，只是标题色与徽标走危险语义。
    pub(crate) fn error(
        title: impl Into<String>,
        message: impl Into<String>,
        buttons: Vec<ModalButton>,
    ) -> Self {
        Self {
            kind: ModalKind::Error,
            ..Self::new(title, message, buttons)
        }
    }

    /// 主按钮 id（第一个 Primary）；没有 Primary 时取最后一个按钮。
    pub(crate) fn primary_id(&self) -> Option<u16> {
        self.buttons
            .iter()
            .find(|b| b.role == ButtonRole::Primary)
            .or_else(|| self.buttons.last())
            .map(|b| b.id)
    }

    /// 按 id 查按钮：调用方据此把命中结果映射回业务动作。
    pub(crate) fn find(&self, id: u16) -> Option<&ModalButton> {
        self.buttons.iter().find(|b| b.id == id)
    }
}

/// 模态交互状态：键盘焦点与鼠标悬停（都是瞬态，默认落在主按钮）。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ModalState {
    pub(crate) focused: usize,
    pub(crate) hovered: Option<usize>,
    /// 悬停过渡（S6）：按钮底色淡入/淡出。
    pub(crate) fade: crate::anim::Fade,
}

impl ModalState {
    /// 初始焦点 = 主按钮下标；无按钮时安全落在 0。
    pub(crate) fn new(spec: &ModalSpec) -> Self {
        let focused = spec
            .buttons
            .iter()
            .position(|b| b.role == ButtonRole::Primary)
            .unwrap_or_else(|| spec.buttons.len().saturating_sub(1));
        Self {
            focused,
            hovered: None,
            fade: crate::anim::Fade::default(),
        }
    }

    /// 键盘处理：返回要执行的按钮 id（焦点移动键返回 None）。
    /// 文本接缝：`Text(" ")` 即 Space（shell 已把 NamedKey::Space 折成文本），
    /// `Text("s")` 等按标签助记符命中；IME 组合态下只有逻辑键（`Key::S` 等）时同样命中。
    pub(crate) fn on_key(&mut self, key: &Key, shift: bool, spec: &ModalSpec) -> Option<u16> {
        match key {
            Key::Enter => spec.primary_id(),
            Key::Escape => spec
                .buttons
                .iter()
                .rev()
                .find(|b| b.role == ButtonRole::Secondary)
                .map(|b| b.id),
            Key::Tab => {
                let n = spec.buttons.len();
                if n == 0 {
                    return None;
                }
                self.focused %= n;
                self.focused = if shift {
                    (self.focused + n - 1) % n
                } else {
                    (self.focused + 1) % n
                };
                None
            }
            Key::Text(t) => {
                if t.chars().count() != 1 {
                    return None;
                }
                let c = t.chars().next()?;
                if c == ' ' {
                    return spec.buttons.get(self.focused).map(|b| b.id);
                }
                self.mnemonic(c, spec)
            }
            // IME 组合态/切换瞬间字母没有文本，只到逻辑键；弹窗期间同样按助记符处理。
            Key::S => self.mnemonic('s', spec),
            Key::N => self.mnemonic('n', spec),
            Key::C => self.mnemonic('c', spec),
            Key::R => self.mnemonic('r', spec),
            Key::K => self.mnemonic('k', spec),
            Key::A => self.mnemonic('a', spec),
            Key::O => self.mnemonic('o', spec),
            _ => None,
        }
    }

    /// 助记符命中：移动焦点并返回按钮 id；未命中返回 None（按键被弹窗吞掉）。
    fn mnemonic(&mut self, c: char, spec: &ModalSpec) -> Option<u16> {
        let c = c.to_ascii_lowercase();
        let hit = spec
            .buttons
            .iter()
            .position(|b| mnemonic_of(b).is_some_and(|m| m == c));
        if let Some(i) = hit {
            self.focused = i;
            return spec.buttons.get(i).map(|b| b.id);
        }
        None
    }

    /// 悬停更新：返回是否有变化（App 据此决定是否重绘）。
    pub(crate) fn on_hover(&mut self, mouse: (f32, f32), layout: &ModalLayout) -> bool {
        let hit = hit_index(mouse, &layout.buttons);
        if self.hovered == hit {
            false
        } else {
            self.hovered = hit;
            true
        }
    }

    /// 左键点击：只认按钮矩形；遮罩与卡片空白处一律 None（模态不可误触关闭）。
    pub(crate) fn on_click(&mut self, mouse: (f32, f32), layout: &ModalLayout) -> Option<u16> {
        let i = hit_index(mouse, &layout.buttons)?;
        layout.ids.get(i).copied()
    }
}

/// 文本接缝：`width` 只度量，`draw` 才栅格化落帧；App 用 LayoutEngine + 标签位图缓存实现。
pub(crate) trait Text {
    fn width(&mut self, s: &str) -> f32;
    fn draw(&mut self, frame: &mut Frame, s: &str, color: [u8; 3], x: i32, y: i32);
    /// 标题度量：默认与正文同宽（纯几何测试无需区分字阶）。
    fn title_width(&mut self, s: &str) -> f32 {
        self.width(s)
    }
    /// 标题绘制：默认落到正文路径。
    fn draw_title(&mut self, frame: &mut Frame, s: &str, color: [u8; 3], x: i32, y: i32) {
        self.draw(frame, s, color, x, y)
    }
}

/// 模态几何：卡片与按钮矩形。按钮 id 随几何一起保存，点击命中无需再传 spec。
#[derive(Clone, Debug)]
pub(crate) struct ModalLayout {
    pub(crate) card: Rect,
    pub(crate) buttons: Vec<Rect>,
    /// 与 `buttons` 同序的业务 id（私有：调用方只读命中结果，不应自行构造）。
    ids: Vec<u16>,
}

/// 纯几何：卡片在视口居中，宽度按标题/正文实测宽度自适应并夹在 [min,max] 与视口内。
pub(crate) fn layout(vp: Rect, scale: f32, spec: &ModalSpec, text: &mut dyn Text) -> ModalLayout {
    let p = surface::dialog_padding(scale);
    let title_line_h = surface::dialog_title_line_height(scale);
    let body_line_h = surface::dialog_body_line_height(scale);
    let bh = surface::dialog_button_height(scale);
    let gap = surface::dialog_button_gap(scale);

    let title_w = text.title_width(&spec.title);
    // 错误态标题左侧有徽标：宽度与折行起点都要让出这段视觉标记。
    let icon_w = if spec.kind == ModalKind::Error {
        surface::dialog_icon_size(scale) + surface::dialog_icon_gap(scale)
    } else {
        0.0
    };
    let body_w = spec
        .message
        .split('\n')
        .map(|line| text.width(line))
        .fold(0.0, f32::max);
    let widths = natural_button_widths(spec, scale, text);
    let buttons_w = widths.iter().sum::<f32>()
        + if widths.len() > 1 {
            gap * (widths.len() - 1) as f32
        } else {
            0.0
        };
    let desired = (title_w + icon_w).max(body_w).max(buttons_w) + 2.0 * p;

    let margin = surface::dialog_margin(scale);
    let max_vp_w = (vp.w - 2.0 * margin).max(0.0);
    let hi = surface::dialog_max_width(scale).min(max_vp_w);
    let lo = surface::dialog_min_width(scale).min(hi);
    let card_w = desired.clamp(lo, hi);
    let inner_w = (card_w - 2.0 * p).max(0.0);

    let max_vp_h = (vp.h - 2.0 * margin).max(0.0);
    let has_body = !spec.message.is_empty();
    let fixed_h = 2.0 * p
        + title_line_h
        + bh
        + if has_body {
            surface::dialog_title_gap(scale) + surface::dialog_body_gap(scale)
        } else {
            0.0
        };
    let max_lines = if has_body && body_line_h > 0.0 {
        ((max_vp_h - fixed_h).max(0.0) / body_line_h).floor() as usize
    } else {
        0
    };
    let lines = if has_body {
        visible_lines(wrap_text(&spec.message, inner_w, text), max_lines)
    } else {
        Vec::new()
    };
    let body_h = lines.len() as f32 * body_line_h;
    let card_h = (fixed_h + body_h).min(max_vp_h);

    let card = Rect {
        x: vp.x + (vp.w - card_w) * 0.5,
        y: vp.y + (vp.h - card_h) * 0.5,
        w: card_w,
        h: card_h,
    };
    let (buttons, ids) = place_buttons(card, &widths, spec, scale);
    ModalLayout { card, buttons, ids }
}

/// 错误态徽标的几何：贴卡片内边距，垂直居中于标题行。
pub(crate) fn icon_rect(card: Rect, scale: f32) -> Rect {
    let p = surface::dialog_padding(scale);
    let size = surface::dialog_icon_size(scale);
    let title_line_h = surface::dialog_title_line_height(scale);
    Rect {
        x: card.x + p,
        y: card.y + p + (title_line_h - size) * 0.5,
        w: size,
        h: size,
    }
}

/// 绘制：遮罩 → 卡片 → 标题 → 正文 → 按钮（焦点环压在最上层）。
#[allow(clippy::too_many_arguments)] // 绘制接缝参数天然多（同 Frame 位图原语）
pub(crate) fn paint(
    frame: &mut Frame,
    vp: Rect,
    scale: f32,
    theme: &Theme,
    spec: &ModalSpec,
    state: &ModalState,
    layout: &ModalLayout,
    text: &mut dyn Text,
) {
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
    let body_line_h = surface::dialog_body_line_height(scale);
    let mut title_x = card.x + p;
    if spec.kind == ModalKind::Error {
        let icon = icon_rect(card, scale);
        paint_error_icon(frame, icon, theme);
        title_x += surface::dialog_icon_size(scale) + surface::dialog_icon_gap(scale);
    }
    let title_color = if spec.kind == ModalKind::Error {
        theme.danger
    } else {
        theme.text
    };
    text.draw_title(
        frame,
        &spec.title,
        title_color,
        title_x as i32,
        (card.y + p) as i32,
    );

    let has_body = !spec.message.is_empty();
    if has_body {
        let body_top = card.y + p + title_line_h + surface::dialog_title_gap(scale);
        let bh = surface::dialog_button_height(scale);
        let body_bottom = card.y + card.h - p - bh - surface::dialog_body_gap(scale);
        let max_lines = if body_line_h > 0.0 {
            ((body_bottom - body_top).max(0.0) / body_line_h).floor() as usize
        } else {
            0
        };
        let inner_w = (card.w - 2.0 * p).max(0.0);
        let lines = visible_lines(wrap_text(&spec.message, inner_w, text), max_lines);
        for (i, line) in lines.iter().enumerate() {
            let y = body_top + i as f32 * body_line_h;
            text.draw(frame, line, theme.muted, (card.x + p) as i32, y as i32);
        }
    }

    paint_buttons(frame, scale, theme, spec, state, layout, text);
}

/// 错误徽标：危险色实心圆 + 反色感叹号（纯像素原语，与卡片同源，不引入图标依赖）。
fn paint_error_icon(frame: &mut Frame, icon: Rect, theme: &Theme) {
    let x = icon.x.round() as i32;
    let y = icon.y.round() as i32;
    let w = icon.w.round().max(2.0) as u32;
    let h = icon.h.round().max(2.0) as u32;
    let r = (w.min(h) as f32) * 0.5;
    frame.fill_rounded_rect(
        x,
        y,
        w,
        h,
        r,
        [theme.danger[0], theme.danger[1], theme.danger[2], 255],
    );
    let mark = theme.accent_text;
    let bar_w = (w as f32 * 0.11).round().max(2.0) as u32;
    let bar_h = (h as f32 * 0.42).round().max(3.0) as u32;
    let cx = x + (w / 2) as i32 - (bar_w / 2) as i32;
    let top = y + (h as f32 * 0.22).round() as i32;
    frame.fill_rect(cx, top, bar_w, bar_h, [mark[0], mark[1], mark[2], 255]);
    let dot = (w as f32 * 0.12).round().max(2.0) as u32;
    let dot_y = top + bar_h as i32 + (h as f32 * 0.10).round() as i32;
    frame.fill_rect(cx, dot_y, dot, dot, [mark[0], mark[1], mark[2], 255]);
}

/// 卡片：8px 圆角 + 投影 + 1px 边框；不复用固定 6px 的 `fill_surface_card`。
fn paint_card(frame: &mut Frame, scale: f32, theme: &Theme, card: Rect) {
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

/// 按钮行：角色决定填充/描边/文字色，焦点环最后画以免被底色覆盖。
fn paint_buttons(
    frame: &mut Frame,
    scale: f32,
    theme: &Theme,
    spec: &ModalSpec,
    state: &ModalState,
    layout: &ModalLayout,
    text: &mut dyn Text,
) {
    let radius = surface::corner_radius(scale);
    let line_h = typography::UI_LINE_HEIGHT * scale;
    let fade = state.fade.view(state.hovered);
    for (i, button) in spec.buttons.iter().enumerate() {
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
        let hover_alpha = fade.alpha_of(i);
        let focused = state.focused == i;
        match button.role {
            ButtonRole::Primary => {
                let fill = [theme.accent[0], theme.accent[1], theme.accent[2], 255];
                frame.fill_rounded_rect(x, y, w, h, radius, fill);
                let fg = theme.accent_text;
                if focused {
                    // Primary 底色就是 accent，焦点环改用反色才可见。
                    focus_ring(frame, rect, radius, fg, fill, scale);
                }
                draw_button_text(frame, text, &button.display_label(), fg, rect, line_h);
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
                stroke_rounded(
                    frame,
                    x,
                    y,
                    w,
                    h,
                    radius,
                    border,
                    palette::surface_popup(theme),
                );
                if let Some(a) = hover_alpha {
                    // 叠在描边内层，3px 内缩不会盖住 1px 边框。
                    frame.fill_hover(
                        x,
                        y,
                        w,
                        h,
                        scale,
                        [theme.hover[0], theme.hover[1], theme.hover[2], a],
                    );
                }
                if focused {
                    let inner = if hover_alpha.is_some() {
                        theme.hover
                    } else {
                        palette::surface_popup(theme)
                    };
                    let ring = focus_ring_color(theme, [inner[0], inner[1], inner[2]]);
                    focus_ring(frame, rect, radius, ring, inner, scale);
                }
                draw_button_text(frame, text, &button.display_label(), fg, rect, line_h);
            }
        }
    }
}

/// 1px 圆角描边：外框色 + 内缩 1px 底色，避免直边接缝。
#[allow(clippy::too_many_arguments)] // 位图描边参数天然多（同 Frame 原语）
fn stroke_rounded(
    frame: &mut Frame,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: f32,
    color: [u8; 3],
    bg: [u8; 4],
) {
    frame.fill_rounded_rect(x, y, w, h, radius, [color[0], color[1], color[2], 255]);
    if w > 2 && h > 2 {
        frame.fill_rounded_rect(x + 1, y + 1, w - 2, h - 2, (radius - 1.0).max(0.0), bg);
    }
}

/// 焦点环颜色：accent 在该底上不足 3:1（非文本对比门禁）时退回正文前景色。
/// 深色主题 accent/popup 实测约 2.1:1，直接画会被底色吞没。
pub(crate) fn focus_ring_color(theme: &Theme, bg: [u8; 3]) -> [u8; 3] {
    if contrast_ratio(theme.accent, bg) >= 3.0 {
        theme.accent
    } else {
        theme.text
    }
}

/// WCAG 相对亮度（sRGB 线性化）。
fn relative_luminance(c: [u8; 3]) -> f32 {
    let ch = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
}

/// 两色对比度（1:1 … 21:1）。
fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (l1, l2) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

/// 焦点环：按钮矩形内缩 1px 起画 `scale` 像素描边（150% DPI 下不因 1px 变细而隐形），
/// 内层填回底色/悬停色。
fn focus_ring(
    frame: &mut Frame,
    rect: Rect,
    radius: f32,
    color: [u8; 3],
    inner: [u8; 4],
    scale: f32,
) {
    let t = scale.round().max(1.0) as u32;
    let w = rect.w.round().max(0.0) as u32;
    let h = rect.h.round().max(0.0) as u32;
    if w <= 2 + 2 * t || h <= 2 + 2 * t {
        return;
    }
    let x = rect.x.round() as i32 + 1;
    let y = rect.y.round() as i32 + 1;
    let r = (radius - 1.0).max(0.0);
    frame.fill_rounded_rect(x, y, w - 2, h - 2, r, [color[0], color[1], color[2], 255]);
    frame.fill_rounded_rect(
        x + t as i32,
        y + t as i32,
        w - 2 - 2 * t,
        h - 2 - 2 * t,
        (r - t as f32).max(0.0),
        inner,
    );
}

/// 按钮文字居中（水平按实测宽、垂直按正文行高），不做截断（按钮尺寸由文字决定）。
fn draw_button_text(
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

/// 按钮自然宽：文字宽 + 两侧留白，但不小于最小宽（保证点击目标）。
fn natural_button_widths(spec: &ModalSpec, scale: f32, text: &mut dyn Text) -> Vec<f32> {
    let min_w = surface::dialog_button_min_width(scale);
    let pad = 2.0 * BUTTON_PAD_X * scale;
    spec.buttons
        .iter()
        .map(|b| (text.width(&b.display_label()) + pad).max(min_w))
        .collect()
}

/// 自右向左排按钮：返回与 spec 同序的矩形与 id；空间不足时等比压缩到内宽。
fn place_buttons(
    card: Rect,
    widths: &[f32],
    spec: &ModalSpec,
    scale: f32,
) -> (Vec<Rect>, Vec<u16>) {
    let p = surface::dialog_padding(scale);
    let gap = surface::dialog_button_gap(scale);
    let bh = surface::dialog_button_height(scale);
    let n = widths.len();
    if n == 0 {
        return (Vec::new(), Vec::new());
    }
    let inner_w = (card.w - 2.0 * p).max(0.0);
    let gaps = if n > 1 { gap * (n - 1) as f32 } else { 0.0 };
    let avail = (inner_w - gaps).max(0.0);
    let sum: f32 = widths.iter().sum();
    let fitted: Vec<f32> = if sum > avail && sum > 0.0 {
        widths.iter().map(|w| w * avail / sum).collect()
    } else {
        widths.to_vec()
    };
    let y = (card.y + card.h - p - bh).max(card.y);
    let mut rects = vec![
        Rect {
            x: card.x,
            y,
            w: 0.0,
            h: bh,
        };
        n
    ];
    let mut cursor = card.x + card.w - p;
    for (i, w) in fitted.iter().enumerate().rev() {
        let x = cursor - *w;
        rects[i] = Rect { x, y, w: *w, h: bh };
        cursor = x - gap;
    }
    let ids = spec.buttons.iter().map(|b| b.id).collect();
    (rects, ids)
}

/// 点是否落在某按钮内；返回下标（半开区间，零宽矩形永不命中）。
fn hit_index(mouse: (f32, f32), buttons: &[Rect]) -> Option<usize> {
    let (x, y) = mouse;
    buttons.iter().position(|r| {
        r.w > 0.0 && r.h > 0.0 && x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
    })
}

/// 助记符：显式声明优先，否则取标签里第一个 ASCII 字母（大小写不敏感匹配）。
fn mnemonic_of(b: &ModalButton) -> Option<char> {
    b.mnemonic
        .or_else(|| b.label.chars().find(|c| c.is_ascii_alphabetic()))
        .map(|c| c.to_ascii_lowercase())
}

/// 按内宽贪心折行：中文按字断，英文优先在最近空格断；保留显式换行。
fn wrap_text(message: &str, max_w: f32, text: &mut dyn Text) -> Vec<String> {
    let mut out = Vec::new();
    if message.is_empty() {
        return out;
    }
    for para in message.split('\n') {
        if para.is_empty() {
            out.push(String::new());
            continue;
        }
        if max_w <= 0.0 {
            out.push(para.to_string());
            continue;
        }
        let mut start = 0usize;
        let mut line_w = 0.0f32;
        let mut last_space: Option<usize> = None;
        for (byte_idx, ch) in para.char_indices() {
            let cw = text.width(&ch.to_string());
            if ch == ' ' {
                last_space = Some(byte_idx);
            }
            if line_w + cw > max_w && byte_idx > start {
                let split = last_space.filter(|&s| s > start).unwrap_or(byte_idx);
                out.push(para[start..split].trim_end().to_string());
                start = if split == byte_idx {
                    byte_idx
                } else {
                    split + 1
                };
                line_w = 0.0;
                last_space = None;
                for (i, c) in para[start..byte_idx].char_indices() {
                    if c == ' ' {
                        last_space = Some(start + i);
                    }
                    line_w += text.width(&c.to_string());
                }
            }
            line_w += cw;
        }
        out.push(para[start..].trim_end().to_string());
    }
    out
}

/// 正文行数上限：超出可用高度的行直接截断（不画省略号，至少不溢出卡片）。
fn visible_lines(mut lines: Vec<String>, max_lines: usize) -> Vec<String> {
    if lines.len() > max_lines {
        lines.truncate(max_lines);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 确定性测试文本：每字符宽 8、行高 18；draw 只落一个 1px 竖条供像素采样。
    struct FakeText {
        char_w: f32,
        line_h: f32,
        colors: Vec<[u8; 3]>,
        texts: Vec<String>,
    }

    impl FakeText {
        fn new() -> Self {
            Self {
                char_w: 8.0,
                line_h: 18.0,
                colors: Vec::new(),
                texts: Vec::new(),
            }
        }
    }

    impl Text for FakeText {
        fn width(&mut self, s: &str) -> f32 {
            s.chars().count() as f32 * self.char_w
        }

        fn draw(&mut self, frame: &mut Frame, s: &str, color: [u8; 3], x: i32, y: i32) {
            if !s.is_empty() {
                self.colors.push(color);
                self.texts.push(s.to_string());
                frame.fill_rect(
                    x,
                    y,
                    1,
                    self.line_h as u32,
                    [color[0], color[1], color[2], 255],
                );
            }
        }
    }

    fn buttons() -> Vec<ModalButton> {
        vec![
            ModalButton::new(1, "保存", ButtonRole::Primary).with_mnemonic('s'),
            ModalButton::new(2, "不保存", ButtonRole::Danger).with_mnemonic('c'),
            ModalButton::new(3, "取消", ButtonRole::Secondary),
        ]
    }

    fn spec() -> ModalSpec {
        ModalSpec::new("保存更改", "文档有未保存的修改。", buttons())
    }

    fn vp() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        }
    }

    fn layout_for(vp: Rect, spec: &ModalSpec) -> ModalLayout {
        let mut text = FakeText::new();
        layout(vp, 1.0, spec, &mut text)
    }

    fn center(r: &Rect) -> (f32, f32) {
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    }

    #[test]
    fn card_is_centered_in_viewport() {
        let vp = vp();
        let l = layout_for(vp, &spec());
        let left = l.card.x - vp.x;
        let right = vp.x + vp.w - (l.card.x + l.card.w);
        let top = l.card.y - vp.y;
        let bottom = vp.y + vp.h - (l.card.y + l.card.h);
        assert!(
            (left - right).abs() < 0.01,
            "左右留白不等: {left} vs {right}"
        );
        assert!(
            (top - bottom).abs() < 0.01,
            "上下留白不等: {top} vs {bottom}"
        );
    }

    #[test]
    fn buttons_sit_inside_card_non_overlapping_and_right_aligned() {
        let l = layout_for(vp(), &spec());
        let p = surface::dialog_padding(1.0);
        for b in &l.buttons {
            assert!(b.x >= l.card.x - 0.01, "按钮越过卡片左缘");
            assert!(b.x + b.w <= l.card.x + l.card.w + 0.01, "按钮越过卡片右缘");
            assert!(b.y >= l.card.y - 0.01, "按钮越过卡片上缘");
            assert!(b.y + b.h <= l.card.y + l.card.h + 0.01, "按钮越过卡片下缘");
        }
        for pair in l.buttons.windows(2) {
            assert!(
                pair[0].x + pair[0].w <= pair[1].x + 0.01,
                "按钮重叠: {:?} vs {:?}",
                pair[0],
                pair[1]
            );
        }
        let last = l.buttons.last().expect("测试规格恒有按钮");
        assert!(
            (last.x + last.w - (l.card.x + l.card.w - p)).abs() < 0.01,
            "最后一个按钮未右对齐到内边距"
        );
    }

    #[test]
    fn geometry_scales_linearly_with_dpi() {
        // 150% / 200% 缩放下不能只放大字号而漏掉卡片与按钮几何，否则视觉比例会变形。
        let s = spec();
        let mut text = FakeText::new();
        let base = layout(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1200.0,
                h: 900.0,
            },
            1.0,
            &s,
            &mut text,
        );
        for scale in [1.5f32, 2.0] {
            let big = layout(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1200.0 * scale,
                    h: 900.0 * scale,
                },
                scale,
                &s,
                &mut text,
            );
            assert!(
                (big.card.w - base.card.w * scale).abs() <= 2.0 * scale,
                "卡片宽未按 scale 线性放大: {scale}"
            );
            assert!(
                (big.buttons[0].h - base.buttons[0].h * scale).abs() <= 1.0 * scale,
                "按钮高未按 scale 线性放大: {scale}"
            );
            let gap = big.buttons[1].x - (big.buttons[0].x + big.buttons[0].w);
            let base_gap = base.buttons[1].x - (base.buttons[0].x + base.buttons[0].w);
            assert!(
                (gap - base_gap * scale).abs() <= 1.0 * scale,
                "按钮间距未按 scale 线性放大: {scale}"
            );
        }
    }

    #[test]
    fn card_width_is_clamped_and_narrow_viewport_stays_contained() {
        let l = layout_for(vp(), &spec());
        assert!(l.card.w >= surface::dialog_min_width(1.0));
        assert!(l.card.w <= surface::dialog_max_width(1.0));

        let small = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 120.0,
        };
        let l = layout_for(small, &spec());
        assert!(l.card.x >= small.x - 0.01);
        assert!(l.card.y >= small.y - 0.01);
        assert!(l.card.x + l.card.w <= small.x + small.w + 0.01);
        assert!(l.card.y + l.card.h <= small.y + small.h + 0.01);
        assert!(l.card.w <= surface::dialog_max_width(1.0));
    }

    #[test]
    fn long_message_wraps_without_exceeding_card() {
        let message = "这是一段很长很长的正文，用来验证自动折行与高度截断。".repeat(20);
        let s = ModalSpec::new("标题", &message, buttons());
        let vp = vp();
        let mut text = FakeText::new();
        let l = layout(vp, 1.0, &s, &mut text);

        let p = surface::dialog_padding(1.0);
        let inner_w = l.card.w - 2.0 * p;
        let lines = wrap_text(&s.message, inner_w, &mut text);
        assert!(lines.len() > 1, "超长正文应折成多行");
        assert!(lines.iter().all(|line| text.width(line) <= inner_w + 0.01));

        let line_h = surface::dialog_body_line_height(1.0);
        let fixed_h = p * 2.0
            + line_h
            + surface::dialog_button_height(1.0)
            + surface::dialog_title_gap(1.0)
            + surface::dialog_body_gap(1.0);
        let max_vp_h = vp.h - 2.0 * surface::dialog_margin(1.0);
        let max_lines = ((max_vp_h - fixed_h).max(0.0) / line_h).floor() as usize;
        let shown = visible_lines(lines, max_lines);
        assert!(shown.len() <= max_lines);
        assert!(
            fixed_h + shown.len() as f32 * line_h <= max_vp_h + 0.01,
            "正文行总高溢出可用高度"
        );
        assert!(l.card.h <= max_vp_h + 0.01);
    }

    #[test]
    fn keyboard_activation_tab_cycle_and_mnemonics() {
        let s = spec();
        let mut st = ModalState::new(&s);
        assert_eq!(st.focused, 0, "初始焦点落在主按钮");
        assert_eq!(st.on_key(&Key::Enter, false, &s), Some(1));

        assert_eq!(
            st.on_key(&Key::Escape, false, &s),
            Some(3),
            "Esc 取最后一个 Secondary"
        );

        assert_eq!(st.on_key(&Key::Tab, false, &s), None);
        assert_eq!(st.focused, 1);
        assert_eq!(st.on_key(&Key::Tab, false, &s), None);
        assert_eq!(st.focused, 2);
        assert_eq!(st.on_key(&Key::Tab, false, &s), None);
        assert_eq!(st.focused, 0, "Tab 循环回主按钮");
        assert_eq!(st.on_key(&Key::Tab, true, &s), None);
        assert_eq!(st.focused, 2, "Shift+Tab 反向循环");

        assert_eq!(st.on_key(&Key::Text("s".into()), false, &s), Some(1));
        assert_eq!(st.focused, 0);
        assert_eq!(st.on_key(&Key::Text("c".into()), false, &s), Some(2));
        assert_eq!(st.focused, 1);
        assert_eq!(
            st.on_key(&Key::Text(" ".into()), false, &s),
            Some(2),
            "Space 激活焦点按钮"
        );
        assert_eq!(st.on_key(&Key::Left, false, &s), None, "其它键不产生动作");

        // 中文输入法启用时字母没有 Text，只到逻辑键；弹窗仍须响应助记符。
        assert_eq!(st.on_key(&Key::S, false, &s), Some(1));
        assert_eq!(st.focused, 0);
        assert_eq!(st.on_key(&Key::C, false, &s), Some(2));
        assert_eq!(st.focused, 1);
        assert_eq!(
            st.on_key(&Key::X, false, &s),
            None,
            "规格未声明 x 助记符时按键被吞掉"
        );
        assert_eq!(st.focused, 1, "未命中不改焦点");

        let no_secondary = ModalSpec::new(
            "确认",
            "不可撤销。",
            vec![ModalButton::new(7, "确定", ButtonRole::Primary)],
        );
        let mut st = ModalState::new(&no_secondary);
        assert_eq!(st.on_key(&Key::Escape, false, &no_secondary), None);

        let no_primary = ModalSpec::new(
            "确认",
            "没有主按钮。",
            vec![ModalButton::new(8, "删除", ButtonRole::Danger).with_mnemonic('d')],
        );
        assert_eq!(
            no_primary.primary_id(),
            Some(8),
            "无 Primary 时取最后一个按钮"
        );
        assert_eq!(ModalState::new(&no_primary).focused, 0);
    }

    /// 未保存关闭对话框：默认焦点必须是最安全动作（保存），绝不能落在「不保存」；
    /// Esc 也必须通往安全出口「取消」。
    #[test]
    fn unsaved_close_dialog_defaults_to_safe_action() {
        use crate::i18n::{t, Key as I18nKey};
        let s = ModalSpec::new(
            "是否保存对「a.md」的更改？",
            t(I18nKey::ModalUnsavedBody),
            vec![
                ModalButton::new(1, t(I18nKey::BtnSave), ButtonRole::Primary).with_mnemonic('s'),
                ModalButton::new(2, t(I18nKey::BtnDontSave), ButtonRole::Danger).with_mnemonic('c'),
                ModalButton::new(3, t(I18nKey::BtnCancel), ButtonRole::Secondary),
            ],
        );
        let mut st = ModalState::new(&s);
        assert_ne!(
            s.buttons[st.focused].label,
            t(I18nKey::BtnDontSave),
            "默认焦点不得是「不保存」"
        );
        assert_eq!(st.on_key(&Key::Enter, false, &s), Some(1), "回车默认=保存");
        assert_eq!(
            st.on_key(&Key::Escape, false, &s),
            Some(3),
            "Esc=取消（安全出口）"
        );
    }

    /// 助记符必须以半角大写括号后缀上屏，否则用户不知道按哪个字母；无声明则原样。
    #[test]
    fn mnemonic_suffix_is_displayed_on_buttons() {
        let bs = buttons();
        assert_eq!(bs[0].display_label(), "保存 (S)");
        assert_eq!(bs[1].display_label(), "不保存 (C)");
        assert_eq!(bs[2].display_label(), "取消", "未声明助记符不加后缀");
    }

    /// 提示后缀参与测宽：带助记符的按钮不窄于同组未声明助记符的版本，
    /// 且仍满足「在卡片内、互不重叠」的既有约束。
    #[test]
    fn hinted_buttons_widen_and_stay_inside_card() {
        let hinted = spec();
        let plain = ModalSpec {
            buttons: buttons()
                .into_iter()
                .map(|b| ModalButton {
                    mnemonic: None,
                    ..b
                })
                .collect(),
            ..spec()
        };
        let mut text = FakeText {
            char_w: 12.0,
            ..FakeText::new()
        };
        let l_hinted = layout(vp(), 1.0, &hinted, &mut text);
        let l_plain = layout(vp(), 1.0, &plain, &mut text);
        assert_eq!(l_hinted.buttons.len(), l_plain.buttons.len());
        for (h, p) in l_hinted.buttons.iter().zip(&l_plain.buttons) {
            assert!(
                h.w >= p.w - 0.01,
                "带提示的按钮不应更窄: {} vs {}",
                h.w,
                p.w
            );
        }
        for b in &l_hinted.buttons {
            assert!(b.x >= l_hinted.card.x - 0.01, "按钮越过卡片左缘");
            assert!(
                b.x + b.w <= l_hinted.card.x + l_hinted.card.w + 0.01,
                "按钮越过卡片右缘"
            );
        }
        for pair in l_hinted.buttons.windows(2) {
            assert!(
                pair[0].x + pair[0].w <= pair[1].x + 0.01,
                "按钮重叠: {:?} vs {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// 绘制路径必须真的把「标签 (S)」交给文本接缝，而不是只度量不画。
    #[test]
    fn paint_draws_mnemonic_hint() {
        let theme = Theme::light();
        let s = spec();
        let vp = vp();
        let mut text = FakeText::new();
        let l = layout(vp, 1.0, &s, &mut text);
        let st = ModalState::new(&s);
        let mut frame = Frame::new(vp.w as u32, vp.h as u32, palette::surface_base(&theme));
        let mut paint_text = FakeText::new();
        paint(&mut frame, vp, 1.0, &theme, &s, &st, &l, &mut paint_text);
        assert!(
            paint_text.texts.iter().any(|t| t.contains("(S)")),
            "主按钮须绘制 (S) 提示: {:?}",
            paint_text.texts
        );
        assert!(
            paint_text.texts.iter().any(|t| t.contains("(C)")),
            "危险按钮须绘制 (C) 提示: {:?}",
            paint_text.texts
        );
    }

    #[test]
    fn click_hits_only_buttons_and_hover_tracks_changes() {
        let s = spec();
        let l = layout_for(vp(), &s);
        let mut st = ModalState::new(&s);

        let c = center(&l.buttons[0]);
        assert_eq!(st.on_click(c, &l), Some(1));
        assert_eq!(
            st.on_click((vp().x + 1.0, vp().y + 1.0), &l),
            None,
            "遮罩点击不可关闭模态"
        );
        assert_eq!(
            st.on_click((l.card.x + 2.0, l.card.y + 2.0), &l),
            None,
            "卡片空白处不命中按钮"
        );

        assert!(st.on_hover(c, &l), "进入按钮应返回变化");
        assert!(!st.on_hover(c, &l), "同位置重复不应重绘");
        assert!(
            st.on_hover((vp().x + 1.0, vp().y + 1.0), &l),
            "离开按钮应返回变化"
        );
        assert!(!st.on_hover((vp().x + 1.0, vp().y + 1.0), &l));
    }

    /// 焦点环必须在浅/深主题、弹窗底与悬停底上都达到非文本对比 3:1；
    /// 深色主题的 accent 直接压在 popup 上不足 3:1，必须回退到正文前景色。
    #[test]
    fn focus_ring_meets_non_text_contrast_in_both_themes() {
        for theme in [Theme::light(), Theme::dark()] {
            let popup = palette::surface_popup(&theme);
            let hover = theme.hover;
            for bg in [
                [popup[0], popup[1], popup[2]],
                [hover[0], hover[1], hover[2]],
            ] {
                let ring = focus_ring_color(&theme, bg);
                let c = contrast_ratio(ring, bg);
                assert!(
                    c >= 3.0,
                    "焦点环对比不足: dark={} ring={ring:?} bg={bg:?} c={c:.2}",
                    theme.dark
                );
            }
            // 深色主题若仍用 accent 会被底色吞没，显式锁死回退行为。
            if theme.dark {
                let popup = [popup[0], popup[1], popup[2]];
                assert_eq!(
                    focus_ring_color(&theme, popup),
                    theme.text,
                    "深色主题焦点环必须回退到正文前景色"
                );
            }
        }
    }

    /// 150% DPI：焦点环按 scale 加粗到 2px，避免 1px 物理线在高 DPI 下变细而隐形。
    #[test]
    fn focus_ring_thickens_with_scale() {
        let rect = Rect {
            x: 10.0,
            y: 10.0,
            w: 60.0,
            h: 40.0,
        };
        let mut frame = Frame::new(80, 60, [0, 0, 0, 255]);
        focus_ring(&mut frame, rect, 0.0, [255, 0, 0], [0, 0, 255, 255], 1.5);
        assert_eq!([frame.get(11, 11)[0], frame.get(11, 11)[1]], [255, 0]);
        assert_eq!([frame.get(12, 12)[0], frame.get(12, 12)[1]], [255, 0]);
        let inner = frame.get(13, 13);
        assert_eq!([inner[0], inner[1], inner[2]], [0, 0, 255], "环内仍为底色");
    }

    fn pixel_case(theme: Theme) {
        let vp = Rect {
            x: 0.0,
            y: 0.0,
            w: 480.0,
            h: 320.0,
        };
        let s = spec();
        let mut text = FakeText::new();
        let l = layout(vp, 1.0, &s, &mut text);
        let st = ModalState::new(&s);
        let mut frame = Frame::new(vp.w as u32, vp.h as u32, palette::surface_base(&theme));
        paint(&mut frame, vp, 1.0, &theme, &s, &st, &l, &mut text);

        // 1) 视口角落 = bg 与黑遮罩混合（比 bg 暗）
        let alpha = surface::scrim_alpha(theme.dark) as u32;
        let corner = frame.get(0, 0);
        let bg = palette::surface_base(&theme);
        for ch in 0..3 {
            let expect = (bg[ch] as u32 * (255 - alpha) / 255) as u8;
            assert!(
                (corner[ch] as i32 - expect as i32).abs() <= 1,
                "遮罩混合值不符: got {corner:?} expect {expect} dark={}",
                theme.dark
            );
            assert!(corner[ch] < bg[ch], "遮罩应压暗底色");
        }

        // 2) 卡片中心 = popup_bg（文字与按钮都不覆盖此点）
        let cx = (l.card.x + l.card.w * 0.5).round() as u32;
        let cy = (l.card.y + l.card.h * 0.5).round() as u32;
        let card_px = frame.get(cx, cy);
        let popup = palette::surface_popup(&theme);
        assert_eq!(
            [card_px[0], card_px[1], card_px[2]],
            [popup[0], popup[1], popup[2]],
            "卡片中心应为 popup_bg dark={}",
            theme.dark
        );

        // 3) 圆角外侧 2px = 遮罩区域（被阴影进一步压暗），绝非卡片色
        let ox = (l.card.x.round() as i32 - 2).max(0) as u32;
        let oy = (l.card.y.round() as i32 - 2).max(0) as u32;
        let outside = frame.get(ox, oy);
        assert_ne!(
            [outside[0], outside[1], outside[2]],
            [popup[0], popup[1], popup[2]],
            "圆角外侧不应是卡片色"
        );
        assert!(
            outside[0] < bg[0] || outside[1] < bg[1] || outside[2] < bg[2],
            "圆角外侧应被遮罩/阴影压暗"
        );

        // 4) Primary 按钮中心 = accent
        let b = &l.buttons[0];
        let bx = (b.x + b.w * 0.5).round() as u32;
        let by = (b.y + b.h * 0.5).round() as u32;
        let btn = frame.get(bx, by);
        assert_eq!(
            [btn[0], btn[1], btn[2]],
            theme.accent,
            "Primary 按钮中心应为 accent dark={}",
            theme.dark
        );
    }

    #[test]
    fn paint_layers_hold_for_light_theme() {
        pixel_case(Theme::light());
    }

    #[test]
    fn paint_layers_hold_for_dark_theme() {
        pixel_case(Theme::dark());
    }

    /// 错误态与普通弹窗同源：同一卡片几何，只在标题色与徽标上体现危险语义；
    /// 徽标必须让出标题横向空间，不得压字。
    #[test]
    fn error_modal_uses_danger_title_and_icon_without_overlapping_text() {
        let theme = Theme::light();
        let vp = Rect {
            x: 0.0,
            y: 0.0,
            w: 480.0,
            h: 320.0,
        };
        let spec = ModalSpec::error(
            "打开失败",
            "文件不存在或已被移动。可重试或重新选择文件。",
            vec![
                ModalButton::new(1, "重试", ButtonRole::Primary),
                ModalButton::new(2, "重新选择", ButtonRole::Secondary),
                ModalButton::new(3, "取消", ButtonRole::Secondary),
            ],
        );
        assert_eq!(spec.kind, ModalKind::Error);
        let mut text = FakeText::new();
        let l = layout(vp, 1.0, &spec, &mut text);
        let st = ModalState::new(&spec);
        let mut frame = Frame::new(vp.w as u32, vp.h as u32, palette::surface_base(&theme));
        let mut paint_text = FakeText::new();
        paint(&mut frame, vp, 1.0, &theme, &spec, &st, &l, &mut paint_text);

        // 徽标在卡片内且为危险色实心圆（采样圆内左上区域，避开反色感叹号）。
        let icon = icon_rect(l.card, 1.0);
        assert!(icon.x >= l.card.x && icon.x + icon.w <= l.card.x + l.card.w);
        assert!(icon.y >= l.card.y && icon.y + icon.h <= l.card.y + l.card.h);
        let px = frame.get(icon.x as u32 + 3, icon.y as u32 + 3);
        assert_eq!(
            [px[0], px[1], px[2]],
            theme.danger,
            "错误徽标必须引用主题危险色"
        );
        // 标题色 = 危险色；且标题起点在徽标右侧（不压徽标）。
        assert!(
            paint_text.colors.contains(&theme.danger),
            "错误标题必须用危险色绘制"
        );
        let title_x = l.card.x
            + surface::dialog_padding(1.0)
            + surface::dialog_icon_size(1.0)
            + surface::dialog_icon_gap(1.0);
        assert!(title_x >= icon.x + icon.w, "标题必须让出徽标横向空间");
    }

    /// 错误弹窗的按钮层级：主操作/取消/危险三种角色都来自同一组令牌与主题色。
    #[test]
    fn modal_button_roles_map_to_theme_tokens() {
        let theme = Theme::light();
        let vp = Rect {
            x: 0.0,
            y: 0.0,
            w: 480.0,
            h: 320.0,
        };
        let spec = ModalSpec::new(
            "确认",
            "删除后不可恢复。",
            vec![
                ModalButton::new(1, "确定", ButtonRole::Primary),
                ModalButton::new(2, "删除", ButtonRole::Danger),
                ModalButton::new(3, "取消", ButtonRole::Secondary),
            ],
        );
        let mut text = FakeText::new();
        let l = layout(vp, 1.0, &spec, &mut text);
        let st = ModalState::new(&spec);
        let mut frame = Frame::new(vp.w as u32, vp.h as u32, palette::surface_base(&theme));
        let mut paint_text = FakeText::new();
        paint(&mut frame, vp, 1.0, &theme, &spec, &st, &l, &mut paint_text);
        // Primary = accent 填充；Danger = danger 描边；Secondary = chrome_border 描边。
        let sample_border = |i: usize, y_off: u32| {
            let b = &l.buttons[i];
            frame.get(b.x as u32 + b.w as u32 / 2, b.y as u32 + y_off)
        };
        let primary = sample_border(0, 0);
        assert_eq!([primary[0], primary[1], primary[2]], theme.accent);
        let danger = sample_border(1, 0);
        assert_eq!([danger[0], danger[1], danger[2]], theme.danger);
        let secondary = sample_border(2, 0);
        assert_eq!(
            [secondary[0], secondary[1], secondary[2]],
            [
                theme.chrome_border[0],
                theme.chrome_border[1],
                theme.chrome_border[2]
            ]
        );
    }
}
