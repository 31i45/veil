//! 设计令牌（design tokens）：几何、间距、线宽、字阶与动效的唯一定义源。
//!
//! 颜色继续由 `theme::Theme` 承担；本模块只收敛与主题无关的标量几何、字阶与时间常量。
//! `surface` 为浮层几何令牌，自 `theme` 原样迁入，调用点经 `theme::surface` 零改动沿用。

/// surface 设计令牌（不可配置）：浮层（右键菜单 / 快速打开 / 大纲 / 搜索条）共用几何。
/// 所有长度以 1.0× scale 为基准，经取值函数按 scale 线性放大；深/浅主题共用几何，
/// 仅阴影 alpha 按主题微调（阴影是半透明黑叠加，深底下需稍强才有同等存在感）。
pub(crate) mod surface {
    /// 圆角半径（px，1.0× 基准）
    pub const CORNER_RADIUS: f32 = 6.0;
    /// 条目行高（px）
    pub const ROW_HEIGHT: f32 = 28.0;
    /// 内边距（px）
    pub const PADDING: f32 = 12.0;
    /// 悬停内缩（px）：悬停底色相对条目外边界内缩，形成「浮起」边距
    pub const HOVER_INSET: f32 = 3.0;
    /// 面板外边距（px）：浮起卡片与窗口边缘/滚动条之间的间距
    pub const PANEL_MARGIN: f32 = 8.0;
    /// 控件间距（px）：同一浮层内相邻控件或与内容视口之间的间隙
    pub const CONTROL_GAP: f32 = 8.0;
    /// 文本光标宽（px）
    pub const CARET_WIDTH: f32 = 2.0;
    /// 文本光标高（px）
    pub const CARET_HEIGHT: f32 = 16.0;
    /// 滚动条拇指最小高度（px）
    pub const SCROLLBAR_MIN_THUMB: f32 = 24.0;
    /// 阴影整体向下偏移（px）
    pub const SHADOW_OFFSET_Y: f32 = 2.0;
    /// 阴影层（外扩 px, 基础 alpha），自外向内排列：外层大而淡、内层小而浓，
    /// 叠加大致等效 8px 高斯模糊 + 2px 下移。alpha 总和 72（深色 ×1.25 ≈ 90），
    /// 远低于「显脏」阈值（浅底上 alpha 和 >140 开始凝成灰团）。
    pub const SHADOW_LAYERS: &[(f32, u8)] = &[(8.0, 4), (6.0, 8), (4.0, 14), (2.0, 20), (1.0, 26)];
    /// 深色主题阴影 alpha 增益
    pub const SHADOW_DARK_GAIN: f32 = 1.25;
    /// 模态卡片圆角（px）：比普通浮层大一级，强调「应用内对话框」的层级。
    pub const DIALOG_RADIUS: f32 = 8.0;
    /// 模态卡片内边距（px）
    pub const DIALOG_PADDING: f32 = 20.0;
    /// 模态卡片最小宽度（px）
    pub const DIALOG_MIN_WIDTH: f32 = 320.0;
    /// 模态卡片最大宽度（px）
    pub const DIALOG_MAX_WIDTH: f32 = 520.0;
    /// 模态按钮高（px）：与 ROW_HEIGHT 同值但语义独立，避免行高一改按钮跟着变。
    pub const DIALOG_BUTTON_HEIGHT: f32 = 28.0;
    /// 模态按钮最小宽（px）
    pub const DIALOG_BUTTON_MIN_WIDTH: f32 = 88.0;
    /// 模态按钮水平间距（px）
    pub const DIALOG_BUTTON_GAP: f32 = 8.0;
    /// 标题与正文间距（px）
    pub const DIALOG_TITLE_GAP: f32 = 8.0;
    /// 正文与按钮行间距（px）
    pub const DIALOG_BODY_GAP: f32 = 20.0;
    /// 模态卡片距视口边缘的最小留白（px）
    pub const DIALOG_MARGIN: f32 = 24.0;
    /// 浅色主题遮罩 alpha（黑）：足以压暗正文又把弹窗推前。
    pub const SCRIM_ALPHA_LIGHT: u8 = 80;
    /// 深色主题遮罩 alpha（黑）：深底需更重才有分离感。
    pub const SCRIM_ALPHA_DARK: u8 = 120;

    pub fn corner_radius(scale: f32) -> f32 {
        CORNER_RADIUS * scale
    }
    pub fn row_height(scale: f32) -> f32 {
        ROW_HEIGHT * scale
    }
    pub fn padding(scale: f32) -> f32 {
        PADDING * scale
    }
    pub fn hover_inset(scale: f32) -> f32 {
        HOVER_INSET * scale
    }
    pub fn panel_margin(scale: f32) -> f32 {
        PANEL_MARGIN * scale
    }
    pub fn control_gap(scale: f32) -> f32 {
        CONTROL_GAP * scale
    }
    pub fn caret_width(scale: f32) -> f32 {
        CARET_WIDTH * scale
    }
    pub fn caret_height(scale: f32) -> f32 {
        CARET_HEIGHT * scale
    }
    /// 细滚动条拇指最小高度（大纲等浮层内使用；主滚动条沿用 chrome 的 THUMB_MIN_H）。
    pub fn scrollbar_min_thumb(scale: f32) -> f32 {
        SCROLLBAR_MIN_THUMB * scale
    }
    pub fn shadow_offset_y(scale: f32) -> f32 {
        SHADOW_OFFSET_Y * scale
    }
    pub fn dialog_radius(scale: f32) -> f32 {
        DIALOG_RADIUS * scale
    }
    pub fn dialog_padding(scale: f32) -> f32 {
        DIALOG_PADDING * scale
    }
    pub fn dialog_min_width(scale: f32) -> f32 {
        DIALOG_MIN_WIDTH * scale
    }
    pub fn dialog_max_width(scale: f32) -> f32 {
        DIALOG_MAX_WIDTH * scale
    }
    pub fn dialog_button_height(scale: f32) -> f32 {
        DIALOG_BUTTON_HEIGHT * scale
    }
    pub fn dialog_button_min_width(scale: f32) -> f32 {
        DIALOG_BUTTON_MIN_WIDTH * scale
    }
    pub fn dialog_button_gap(scale: f32) -> f32 {
        DIALOG_BUTTON_GAP * scale
    }
    pub fn dialog_title_gap(scale: f32) -> f32 {
        DIALOG_TITLE_GAP * scale
    }
    pub fn dialog_body_gap(scale: f32) -> f32 {
        DIALOG_BODY_GAP * scale
    }
    pub fn dialog_margin(scale: f32) -> f32 {
        DIALOG_MARGIN * scale
    }

    /// 遮罩 alpha：只随主题明暗切换，不随 scale 变化。
    pub fn scrim_alpha(dark: bool) -> u8 {
        if dark {
            SCRIM_ALPHA_DARK
        } else {
            SCRIM_ALPHA_LIGHT
        }
    }

    /// 阴影单层 alpha：深色主题按增益放大（钳制 255）。
    pub fn shadow_alpha(base: u8, dark: bool) -> u8 {
        if dark {
            (base as f32 * SHADOW_DARK_GAIN).round().min(255.0) as u8
        } else {
            base
        }
    }
}

/// 间距刻度（px，1.0× 基准）：4 的倍数，用于替代散落的间距字面量。
pub(crate) mod space {
    pub const XS: f32 = 4.0;
    #[allow(dead_code)] // 预留：待后续布局迁移消费，当前 paint 仅用到 XS/M/XL
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    #[allow(dead_code)] // 预留：待后续布局迁移消费
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// 线宽刻度（px）：浮层与 chrome 的唯一边界线。
pub(crate) mod stroke {
    pub const HAIRLINE: f32 = 1.0;
}

/// 交互状态矩阵（S4）：六态的单一颜色来源。
///
/// 各 `paint_*` 只从这里取状态色，不再自行挑 `theme.*`，保证同一状态在
/// 查找浮栏、菜单与状态栏处处一致。
pub(crate) mod state {
    use crate::theme::Theme;

    /// hover：指针悬停在可交互控件上。
    pub fn hover_bg(theme: &Theme) -> [u8; 4] {
        theme.hover
    }

    /// active：控件按下，或当前活动项（已展开的菜单标题）。
    pub fn pressed_bg(theme: &Theme) -> [u8; 4] {
        theme.sel
    }

    /// focus：输入焦点所在字段的安静底色（比 hover 更轻，不额外画线）。
    pub fn focus_bg(theme: &Theme) -> [u8; 4] {
        let h = theme.hover;
        [h[0], h[1], h[2], 170]
    }

    /// disabled：不可用控件的前景。
    pub fn disabled_fg(theme: &Theme) -> [u8; 3] {
        theme.muted
    }

    /// selected：开启/选中态前景。
    pub fn selected_fg(theme: &Theme) -> [u8; 3] {
        theme.accent
    }

    /// empty：无结果/空态前景。
    pub fn empty_fg(theme: &Theme) -> [u8; 3] {
        theme.danger
    }
}

/// 字阶与字体族（D8）：UI 13 / 正文 16 / 代码 14。
///
/// 字号以 1.0× scale 为基准，调用方按 scale 线性放大。H1–H6 属内容排版，
/// 由 `block_scale` 相对正文字号派生，不占用本模块的 UI 字阶。
pub(crate) mod typography {
    /// UI 字号（菜单栏、状态栏、浮层、模态）。
    pub const UI_SIZE: f32 = 13.0;
    /// UI 行高（单行浮层/标签）。
    pub const UI_LINE_HEIGHT: f32 = 18.0;
    /// 正文字号（正文块与表格单元格）。
    pub const BODY_SIZE: f32 = 16.0;
    /// 正文行高。
    pub const BODY_LINE_HEIGHT: f32 = 22.0;
    /// 代码块字号（正文的 0.875 倍）。
    pub const CODE_SIZE: f32 = 14.0;
    /// 代码块字号相对正文的倍率：字号与行高同比缩放。
    pub const CODE_SCALE: f32 = CODE_SIZE / BODY_SIZE;

    /// 无衬线族回退链（按序探测存在性）。
    pub const SANS_FAMILIES: &[&str] = &["Microsoft YaHei UI", "Segoe UI"];
    /// 等宽族回退链（按序探测存在性）。
    pub const MONO_FAMILIES: &[&str] = &["Consolas", "Courier New"];
}

#[allow(dead_code)] // A9 动效落地前占位
pub(crate) mod motion {
    use std::time::Duration;

    /// 快速过渡时长。
    pub const FAST: Duration = Duration::from_millis(120);

    /// 标准 cubic ease-out：1-(1-t)^3，输入夹到 [0,1]。
    pub fn ease_out_cubic(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        1.0 - (1.0 - t).powi(3)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{motion, space, state, stroke, typography};

    #[test]
    #[allow(clippy::assertions_on_constants)] // 令牌为编译期常量，仍按运行时断言校验
    fn type_scale_is_ui_body_code() {
        assert_eq!(typography::UI_SIZE, 13.0);
        assert_eq!(typography::BODY_SIZE, 16.0);
        assert_eq!(typography::CODE_SIZE, 14.0);
        assert!((typography::CODE_SCALE - 0.875).abs() < 1e-6);
        assert!(typography::UI_LINE_HEIGHT > typography::UI_SIZE);
        assert!(typography::BODY_LINE_HEIGHT > typography::BODY_SIZE);
    }

    #[test]
    fn font_fallbacks_are_ordered_and_non_empty() {
        assert_eq!(typography::SANS_FAMILIES[0], "Microsoft YaHei UI");
        assert_eq!(typography::MONO_FAMILIES[0], "Consolas");
        assert!(typography::SANS_FAMILIES.len() >= 2);
        assert!(typography::MONO_FAMILIES.len() >= 2);
    }

    #[test]
    fn space_tokens_are_multiples_of_four() {
        for v in [space::XS, space::S, space::M, space::L, space::XL] {
            assert_eq!(v % 4.0, 0.0, "space 令牌必须是 4 的倍数: {v}");
        }
    }

    #[test]
    #[allow(clippy::assertions_on_constants)] // 令牌为编译期常量，仍按运行时断言校验
    fn stroke_tokens_are_positive() {
        assert!(stroke::HAIRLINE > 0.0);
    }

    #[test]
    fn state_matrix_has_single_source_and_distinct_states() {
        let light = crate::theme::Theme::light();
        let dark = crate::theme::Theme::dark();
        assert_eq!(state::hover_bg(&light), light.hover);
        assert_eq!(state::pressed_bg(&light), light.sel);
        assert_eq!(state::focus_bg(&light)[3], 170, "focus 比 hover 更轻");
        assert_eq!(state::disabled_fg(&light), light.muted);
        assert_eq!(state::selected_fg(&light), light.accent);
        assert_eq!(state::empty_fg(&light), light.danger);
        assert_ne!(state::hover_bg(&dark), state::pressed_bg(&dark));
    }

    #[test]
    fn motion_tokens_are_within_budget() {
        assert!(motion::FAST <= Duration::from_millis(120));
    }

    #[test]
    fn ease_out_cubic_hits_endpoints_and_midpoint() {
        assert_eq!(motion::ease_out_cubic(0.0), 0.0);
        assert_eq!(motion::ease_out_cubic(1.0), 1.0);
        // f32::round 取整到整数（0.875 → 1.0），故先按三位小数取整再比较。
        let mid = (motion::ease_out_cubic(0.5) * 1000.0).round() / 1000.0;
        assert!((mid - 0.875).abs() < 1e-6);
    }
}
