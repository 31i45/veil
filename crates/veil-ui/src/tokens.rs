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
    /// Low 档阴影层（轻浮层：菜单/查找栏）：外扩 px + 基础 alpha，自外向内排列。
    /// 叠加大致等效 8px 高斯模糊 + 2px 下移。alpha 总和 72（深色 ×1.25 ≈ 90），
    /// 远低于「显脏」阈值（浅底上 alpha 和 >140 开始凝成灰团）。
    pub const SHADOW_LAYERS: &[(f32, u8)] = &[(8.0, 4), (6.0, 8), (4.0, 14), (2.0, 20), (1.0, 26)];
    /// Low 档深色主题阴影 alpha 增益
    pub const SHADOW_DARK_GAIN: f32 = 1.25;
    /// High 档阴影层（模态对话框）：比 Low 档多两层、外扩更大、alpha 更浓，
    /// 强化「应用内对话框」层级，叠加大致等效 10px 高斯模糊。
    #[allow(dead_code)] // 模态对话框 paint 改造后消费（当前仅测试锁定形态）
    pub const HIGH_SHADOW_LAYERS: &[(f32, u8)] = &[
        (10.0, 4),
        (8.0, 6),
        (6.0, 10),
        (4.0, 16),
        (3.0, 22),
        (2.0, 28),
        (1.0, 34),
    ];
    /// High 档深色主题阴影 alpha 增益（模态在深底上需更强存在感）
    #[allow(dead_code)] // 模态对话框 paint 改造后消费（当前仅 high_shadow_alpha / 测试使用）
    pub const HIGH_SHADOW_DARK_GAIN: f32 = 1.35;
    /// 模态卡片圆角（px）：比普通浮层大一级，强调「应用内对话框」的层级。
    pub const DIALOG_RADIUS: f32 = 8.0;
    /// 模态卡片内边距（px）
    pub const DIALOG_PADDING: f32 = 20.0;
    /// 模态卡片最小宽度（px）
    pub const DIALOG_MIN_WIDTH: f32 = 320.0;
    /// 模态卡片最大宽度（px）：需容纳设置弹窗的最长英文标签列（320）+ 控件列（380）。
    pub const DIALOG_MAX_WIDTH: f32 = 760.0;
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
    /// 标题行高（px）：标题字号更大，但行高与正文行距保持同一节奏。
    pub const DIALOG_TITLE_LINE_HEIGHT: f32 = 22.0;
    /// 正文行高（px）：错误/确认弹窗共用，禁止在弹窗内另立行距。
    pub const DIALOG_BODY_LINE_HEIGHT: f32 = 22.0;
    /// 语义图标边长（px）：错误态标题左侧的徽标，与标题同高不压字。
    pub const DIALOG_ICON_SIZE: f32 = 18.0;
    /// 语义图标与标题间距（px）。
    pub const DIALOG_ICON_GAP: f32 = 10.0;
    /// 模态卡片距视口边缘的最小留白（px）
    pub const DIALOG_MARGIN: f32 = 24.0;
    /// 浅色主题遮罩 alpha（黑）：足以压暗正文又把弹窗推前。
    pub const SCRIM_ALPHA_LIGHT: u8 = 80;
    /// 深色主题遮罩 alpha（黑）：深底需更重才有分离感。
    pub const SCRIM_ALPHA_DARK: u8 = 120;

    /// 浮层圆角（物理像素）：乘完 scale 后吸附到整数物理像素，
    /// 避免 1.25× 时 6 × 1.25 = 7.5 让四角落在半像素上。
    pub fn corner_radius(scale: f32) -> f32 {
        crate::geometry::snap_to_pixel(CORNER_RADIUS * scale)
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
    /// 模态圆角（物理像素）：与 `corner_radius` 同样吸附到整数物理像素。
    pub fn dialog_radius(scale: f32) -> f32 {
        crate::geometry::snap_to_pixel(DIALOG_RADIUS * scale)
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
    pub fn dialog_title_line_height(scale: f32) -> f32 {
        DIALOG_TITLE_LINE_HEIGHT * scale
    }
    pub fn dialog_body_line_height(scale: f32) -> f32 {
        DIALOG_BODY_LINE_HEIGHT * scale
    }
    pub fn dialog_icon_size(scale: f32) -> f32 {
        DIALOG_ICON_SIZE * scale
    }
    pub fn dialog_icon_gap(scale: f32) -> f32 {
        DIALOG_ICON_GAP * scale
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

    /// Low 档阴影单层 alpha：深色主题按增益放大（钳制 255）。
    pub fn shadow_alpha(base: u8, dark: bool) -> u8 {
        if dark {
            (base as f32 * SHADOW_DARK_GAIN).round().min(255.0) as u8
        } else {
            base
        }
    }

    /// High 档阴影单层 alpha：模态专用，深色增益更强。
    #[allow(dead_code)] // 模态对话框 paint 改造后消费（当前仅测试锁定行为）
    pub fn high_shadow_alpha(base: u8, dark: bool) -> u8 {
        if dark {
            (base as f32 * HIGH_SHADOW_DARK_GAIN).round().min(255.0) as u8
        } else {
            base
        }
    }
}

/// 间距刻度（px，1.0× 基准）：4 的倍数，用于替代散落的间距字面量。
pub(crate) mod space {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    #[allow(dead_code)] // 预留：待后续布局迁移消费，当前 paint 仅用到 XS/S/M/XL（L 仅测试引用）
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// 线宽刻度（px）：浮层与 chrome 的唯一边界线。
pub(crate) mod stroke {
    pub const HAIRLINE: f32 = 1.0;
}

/// Surface 底色语义层（S5 + S7）：Theme 四档 surface 的单一入口。
///
/// 所有 paint_* 必须从这里取底色；不得直接访问 `theme.bg` / `theme.chrome_bg` /
/// `theme.popup_bg` / `theme.modal_bg` / `theme.table_head_bg` / `theme.code_bg`。
///
/// 映射表：
/// | 语义 | Theme 字段 | 使用场景 |
/// |---|---|---|
/// | Base | bg | 页面/窗口底层背景 |
/// | Card | chrome_bg | chrome 区条带、表头、代码块（比 Base 深一层但不弹起） |
/// | Popup | popup_bg | 浮层/弹出（Low 档阴影弹起） |
/// | Modal | modal_bg | 模态对话框（High 档阴影弹起，最深一层） |
///
/// 层级约束：card 与 popup 的 OKLab 色差 ΔE_OK ≥ 0.02（≈ CIELAB ΔE 2），modal 与
/// popup/card 之一 ΔE_OK ≥ 0.01，且 modal 档底色 ≠ popup 档底色；门禁见
/// `surface_hierarchy_is_perceptually_separated`。此处比较的是含 a/b 轴的真实
/// 感知色差，不是只看明度差。
pub(crate) mod palette {
    use crate::theme::Theme;

    /// Base 档（页面底层）：正文区、chrome 底层。
    pub fn surface_base(theme: &Theme) -> [u8; 4] {
        theme.bg
    }

    /// Card 档：chrome 区条带（表头、菜单栏底）。
    pub fn surface_card(theme: &Theme) -> [u8; 4] {
        theme.chrome_bg
    }

    /// Popup 档（浮层）：菜单、查找栏、大纲、设置（Low 档阴影）。
    pub fn surface_popup(theme: &Theme) -> [u8; 4] {
        theme.popup_bg
    }

    /// Modal 档（模态）：确认/警告/错误对话框（High 档阴影，底色需与 popup 档有可感知分离）。
    pub fn surface_modal(theme: &Theme) -> [u8; 4] {
        theme.modal_bg
    }

    /// 代码块底色：Card 档的特化（需要和正文拉开一点色差），独立访问器。
    pub fn code_bg(theme: &Theme) -> [u8; 4] {
        theme.code_bg
    }

    /// 表头底色：Card 档特化（表格列头比 Card 稍浅），独立访问器。
    pub fn table_head_bg(theme: &Theme) -> [u8; 4] {
        theme.table_head_bg
    }

    /// Details 折叠卡片底色：暗色取 Card 档代码表面，浅色保留原有两档视觉。
    pub fn details_card_bg(theme: &Theme, expanded: bool) -> [u8; 4] {
        if theme.dark {
            theme.code_bg
        } else if expanded {
            [250, 250, 250, 255]
        } else {
            [252, 252, 252, 255]
        }
    }
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
    /// 模态标题字号：比 UI 正文大一档，形成清晰的层级（正文仍用 UI_SIZE）。
    pub const DIALOG_TITLE_SIZE: f32 = 15.0;
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

/// 正文排版标尺（B5）：行距/块距/容器内边距/图片限高的唯一定义源。
///
/// 长度均为 1.0× scale 基准的物理像素，调用方按 scale 线性放大。
/// 与 `space` 的区别：`space` 是 UI chrome 的 4 倍数刻度，这里是正文排版语义标尺，
/// 保留既有排版节奏（`PARA_GAP=10` 不是 4 的倍数，改它会整体改变文档观感）。
pub(crate) mod doc {
    /// 内容区左右边距的 Standard 档基准（逻辑像素）：屏幕实际边距由设置档位
    /// 在运行时解析（见 `theme::DocMargin`），打印路径仍取此常量基准。
    pub const MARGIN: f32 = 16.0;
    /// 相邻块（无空行分隔）的垂直间距。
    pub const PARA_GAP: f32 = 10.0;
    /// 标题块之后与下一个块的最小间距（层级感：大于正文段距）。
    /// 保留既有 1.6× 比例（10 × 1.6 = 16），与 `PARA_GAP` 同源派生。
    pub const HEADING_GAP: f32 = PARA_GAP * 1.6;
    /// 正文可读列宽上限（全角字符当量）。
    pub const READABLE_COLS: f32 = 40.0;
    /// 空行高度（以正文行高为单位）：块间存在空行时额外占 1 行。
    pub const BLANK_LINE_LINES: f32 = 1.0;
    /// 文末可点击输入空间（以正文行高为单位，B5）：末行下方至少保留 2 行空白。
    pub const BOTTOM_PAD_LINES: f32 = 2.0;
    /// 行内图片（与文字同段）高度上限（以正文行高为单位）：4 行——既能让
    /// 小图在行内可辨认，又不至于把整段行距撑得过大。仅缩小、不放大：
    /// 原图小于该上限时按原尺寸显示（见 `view::fit_size_box` 的 1.0 系数）。
    pub const IMG_INLINE_LINES: f32 = 4.0;
    /// 独占一行（块级 / 段落仅含图片）图片的高度上限占「窗口可视高度」的比例。
    /// 不再用行高倍数：固定 6 行会把正常截图压成小黑块；改用视口比例后
    /// 大图能占满一屏的 80%，同时避免超过一屏导致整块不可见。
    pub const IMG_STANDALONE_VIEWPORT_RATIO: f32 = 0.8;
    /// 显式 `width`/`height` 的极大值上限（版心可用宽的倍数）：作者写了离谱尺寸时
    /// 仍只按此上限等比收拢，防止单张图撑出数屏宽的空白。
    pub const IMG_EXPLICIT_MAX_RATIO: f32 = 4.0;
    /// 表格单元格内图片高度上限（行高倍数，B4）。
    pub const IMG_TABLE_LINES: f32 = 1.5;
    /// 代码卡外部内边距：左右 / 上下（绘制底色外扩）。
    pub const CODE_PAD_X: f32 = 4.0;
    pub const CODE_PAD_Y: f32 = 2.0;
    /// 引用块左竖条：相对版心左缘的内缩 / 条宽。
    pub const QUOTE_INSET: f32 = 8.0;
    pub const QUOTE_RAIL: f32 = 3.0;
    /// Details 卡片相对版心的内边距。
    pub const DETAILS_PAD: f32 = 4.0;
    /// 非活动态 Front Matter 折叠条高度（逻辑像素）：保留可点击入口，
    /// 避免整块 0 高后既看不见也无法点进编辑；打印态仍为 0。
    pub const FM_COLLAPSED_H: f32 = 18.0;
    /// 表格单元格内边距（水平 / 垂直）。
    pub const TABLE_PAD_X: f32 = 8.0;
    pub const TABLE_PAD_Y: f32 = 5.0;
    /// 容器内表格/图片的额外水平缩进步长：引用层 / 列表层。
    pub const QUOTE_STEP: f32 = 14.0;
    pub const LIST_STEP: f32 = 6.0;
}

/// 动效令牌（S1 / S6）：S6 的过渡实现只从这里取时长与缓动。
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

    /// 弹窗视觉令牌必须单点定义，且标题/正文/图标/按钮处在同一节奏上。
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn dialog_tokens_are_single_source_and_consistent() {
        use super::surface;
        assert!(
            typography::DIALOG_TITLE_SIZE > typography::UI_SIZE,
            "标题须大于正文字号"
        );
        assert!(
            typography::DIALOG_TITLE_SIZE < typography::BODY_SIZE,
            "标题不得大于正文字号"
        );
        assert!(surface::DIALOG_TITLE_LINE_HEIGHT >= typography::DIALOG_TITLE_SIZE);
        assert!(surface::DIALOG_BODY_LINE_HEIGHT >= typography::UI_SIZE);
        assert!(surface::DIALOG_ICON_SIZE >= typography::DIALOG_TITLE_SIZE);
        assert!(surface::DIALOG_ICON_GAP > 0.0);
        assert!(surface::DIALOG_BUTTON_HEIGHT >= surface::DIALOG_BUTTON_MIN_WIDTH / 4.0);
        assert!(surface::DIALOG_MAX_WIDTH > surface::DIALOG_MIN_WIDTH);
        assert!(
            surface::DIALOG_RADIUS > surface::CORNER_RADIUS,
            "模态圆角须大于普通浮层"
        );
        // 几何访问器必须按 scale 线性放大（150% / 200% 不脱节）。
        for s in [1.0f32, 1.5, 2.0] {
            assert!((surface::dialog_icon_size(s) - surface::DIALOG_ICON_SIZE * s).abs() < 1e-6);
            assert!(
                (surface::dialog_title_line_height(s) - surface::DIALOG_TITLE_LINE_HEIGHT * s)
                    .abs()
                    < 1e-6
            );
        }
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

    /// 像素对齐回归锁（抽样 100/125/150/200%）：1px 发丝线、2px 小圆角与浮层/模态
    /// 圆角乘完 scale 后都必须落在整数物理像素上，不得停在 .25/.5/.75。
    ///
    /// 真实缺陷来源（1.25×）：`CORNER_RADIUS(6) × 1.25 = 7.5`、`2 × 1.25 = 2.5`；
    /// 前者经 `surface::corner_radius` 统一吸附，后者走 `snap_to_pixel`。
    #[test]
    fn scaled_geometry_snaps_to_integer_physical_pixels() {
        use super::surface;
        use crate::geometry::snap_to_pixel;

        for scale in [1.0f32, 1.25, 1.5, 2.0] {
            let hairline = snap_to_pixel(stroke::HAIRLINE * scale);
            assert!(hairline >= 1.0, "线宽不得消失：scale={scale}");
            assert_eq!(
                hairline,
                hairline.round(),
                "1px 分隔线必须落在整数物理像素：scale={scale} 得到 {hairline}"
            );

            let small_radius = snap_to_pixel(2.0 * scale);
            assert!(small_radius >= 1.0, "圆角不得消失：scale={scale}");
            assert_eq!(
                small_radius,
                small_radius.round(),
                "2px 圆角必须落在整数物理像素：scale={scale} 得到 {small_radius}"
            );

            for r in [surface::corner_radius(scale), surface::dialog_radius(scale)] {
                assert_eq!(
                    r,
                    r.round(),
                    "圆角令牌必须落在整数物理像素：scale={scale} 得到 {r}"
                );
            }
        }

        // 1.0× 保持原值（浅色默认观感不变）。
        assert_eq!(surface::corner_radius(1.0), surface::CORNER_RADIUS);
        assert_eq!(surface::dialog_radius(1.0), surface::DIALOG_RADIUS);
        // 已知半像素点：1.25× 下 6px → 7.5、2px → 2.5；吸附后分别为 8、3。
        assert_eq!(surface::CORNER_RADIUS * 1.25, 7.5);
        assert_eq!(surface::corner_radius(1.25), 8.0);
        assert_eq!(snap_to_pixel(2.0 * 1.25), 3.0);
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

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn high_shadow_layers_are_seven_layers_and_have_stronger_dark_gain() {
        use super::surface::{HIGH_SHADOW_DARK_GAIN, HIGH_SHADOW_LAYERS, SHADOW_DARK_GAIN};
        assert_eq!(HIGH_SHADOW_LAYERS.len(), 7, "High 档阴影必须 7 层");
        assert!(
            HIGH_SHADOW_DARK_GAIN > SHADOW_DARK_GAIN,
            "High 档 dark 增益 > Low 档"
        );
    }

    #[test]
    fn high_shadow_alpha_clamps_at_255() {
        use super::surface::high_shadow_alpha;
        // 深色 + 大 base → 钳制到 255
        assert_eq!(high_shadow_alpha(200, true), 255);
        // 浅色不变
        assert_eq!(high_shadow_alpha(20, false), 20);
    }

    #[test]
    fn palette_module_maps_theme_fields_correctly() {
        use super::palette;
        use crate::theme::Theme;
        for t in [Theme::light(), Theme::dark()] {
            assert_eq!(palette::surface_base(&t), t.bg);
            assert_eq!(palette::surface_card(&t), t.chrome_bg);
            assert_eq!(palette::surface_popup(&t), t.popup_bg);
            assert_eq!(palette::surface_modal(&t), t.modal_bg);
            assert_eq!(palette::code_bg(&t), t.code_bg);
            assert_eq!(palette::table_head_bg(&t), t.table_head_bg);
            for expanded in [false, true] {
                let expected = if t.dark {
                    t.code_bg
                } else if expanded {
                    [250, 250, 250, 255]
                } else {
                    [252, 252, 252, 255]
                };
                assert_eq!(palette::details_card_bg(&t, expanded), expected);
            }
        }
    }

    /// sRGB 分量（0..255）→ 线性光（0..1），IEC 61966-2-1 分段函数。
    fn srgb_to_linear(v: u8) -> f32 {
        let u = v as f32 / 255.0;
        if u <= 0.040_45 {
            u / 12.92
        } else {
            ((u + 0.055) / 1.055).powf(2.4)
        }
    }

    /// sRGB 色 → OKLab（Björn Ottosson, 2020）：线性 RGB → LMS → 立方根 → Lab。
    /// alpha 被忽略（surface 底色均为不透明）。
    fn oklab(c: [u8; 4]) -> [f32; 3] {
        let (r, g, b) = (
            srgb_to_linear(c[0]),
            srgb_to_linear(c[1]),
            srgb_to_linear(c[2]),
        );
        let l = 0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_99 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_84 * g + 0.629_978_7 * b;
        let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
        [
            0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        ]
    }

    /// OKLab 欧氏色差 ΔE_OK：0 = 同色；灰阶下 0.02 ≈ CIELAB ΔE 2（可感知阈值）。
    fn de_ok(a: [u8; 4], b: [u8; 4]) -> f32 {
        let (x, y) = (oklab(a), oklab(b));
        ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
    }

    /// Surface 四档底色层级分离（S5/S7）：用 OKLab ΔE（含明度与色度三轴），不是只看 L。
    ///
    /// 阈值按 CIELAB「可感知」2 单位换算：ΔE_OK ≈ 0.02（灰阶下两者近似 100:1）。
    /// card 与 popup 是同屏相邻的两档（chrome 条带 vs 浮起卡片），要求 ΔE_OK ≥ 0.02；
    /// modal 与 popup/card 之一 ΔE_OK ≥ 0.01，另加 modal ≠ popup 的硬门禁。
    #[test]
    fn surface_hierarchy_is_perceptually_separated() {
        use super::palette;
        use crate::theme::Theme;
        for t in [Theme::light(), Theme::dark()] {
            let card = palette::surface_card(&t);
            let popup = palette::surface_popup(&t);
            let modal = palette::surface_modal(&t);
            let card_popup = de_ok(card, popup);
            let popup_modal = de_ok(popup, modal);
            let card_modal = de_ok(card, modal);
            assert!(
                card_popup >= 0.02,
                "card-popup 层级不可感知: dark={} ΔE_OK={card_popup:.4} < 0.02",
                t.dark
            );
            assert!(
                popup_modal >= 0.01 || card_modal >= 0.01,
                "modal 应至少与 popup 或 card 有感知分离: dark={} ΔE_OK popup-modal={popup_modal:.4} card-modal={card_modal:.4}（阈值 0.01）",
                t.dark
            );
            // modal ≠ popup（硬门禁）
            assert_ne!(
                palette::surface_modal(&t),
                palette::surface_popup(&t),
                "modal 底色不得与 popup 底色相同（dark={}）",
                t.dark
            );
        }
    }

    #[test]
    fn details_card_bg_keeps_theme_foreground_readable() {
        use super::palette;
        use crate::theme::Theme;
        fn lum(c: [u8; 3]) -> f32 {
            let f = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
        }
        for t in [Theme::light(), Theme::dark()] {
            for expanded in [false, true] {
                let bg = palette::details_card_bg(&t, expanded);
                let (a, b) = (lum(t.fg), lum([bg[0], bg[1], bg[2]]));
                let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(
                    ratio >= 4.5,
                    "details 前景/底色对比度 {ratio:.2} < 4.5（dark={} expanded={expanded}）",
                    t.dark
                );
            }
            // modal 档也需保证前景可读（模态是弹起最深层，前景 text/muted 在其上 ≥ 4.5）
            let modal = palette::surface_modal(&t);
            let (a, b) = (lum(t.fg), lum([modal[0], modal[1], modal[2]]));
            let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(
                ratio >= 4.5,
                "modal 前景/底色对比度 {ratio:.2} < 4.5（dark={}）",
                t.dark
            );
        }
    }
}
