//! 跨域共享的几何常量（排版度量与布局间距）。
//!
//! 这些常量是纯数值，不属于任何特定域的派生状态，故独立于 document/view/app，
//! 供各域按名引用，避免真值层（document）反向依赖派生层（view）的常量。

use crate::theme::{DocMargin, WindowGeom};

/// 窗口逻辑宽度下限（与配置解析范围一致）。
const MIN_WINDOW_WIDTH: u32 = 480;
/// 窗口逻辑高度下限（与配置解析范围一致）。
const MIN_WINDOW_HEIGHT: u32 = 360;
/// 窗口逻辑尺寸上限（与配置解析范围一致）。
const MAX_WINDOW_SIZE: u32 = 8192;

/// 显示器可见矩形（物理像素）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// 夹取窗口几何：宽夹到 480..=8192、高夹到 360..=8192；与任一屏幕无可见交集
/// （重叠宽高均 > 0）时在主屏居中。
///
/// 上下限与 `parse_config` 的窗口键校验范围一致，否则落盘后解析会被整组忽略。
/// 规则：
/// - 尺寸先夹到下限/上限；
/// - `screens` 为空时只夹尺寸、位置原样返回；
/// - 与任一屏幕有正面积交集则位置不动（部分可见即可，不强行拉回屏内）；
/// - 完全离屏则在 `screens[0]` 居中；若窗口尺寸大于该屏幕，则贴其左上角，
///   保证标题栏始终可见、用户能拖动回来。
pub fn clamp_window_geom(geom: WindowGeom, screens: &[ScreenRect]) -> WindowGeom {
    let width = geom.width.clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_SIZE);
    let height = geom.height.clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_SIZE);
    if screens.is_empty() {
        return WindowGeom {
            width,
            height,
            x: geom.x,
            y: geom.y,
        };
    }
    // 用 i64 计算交集，避免 i32 坐标 + u32 尺寸在极端值下溢出。
    let (left, top) = (geom.x as i64, geom.y as i64);
    let (right, bottom) = (left + width as i64, top + height as i64);
    let visible = screens.iter().any(|s| {
        let s_left = s.x as i64;
        let s_top = s.y as i64;
        let s_right = s_left + s.w as i64;
        let s_bottom = s_top + s.h as i64;
        let overlap_w = right.min(s_right) - left.max(s_left);
        let overlap_h = bottom.min(s_bottom) - top.max(s_top);
        overlap_w > 0 && overlap_h > 0
    });
    if visible {
        return WindowGeom {
            width,
            height,
            x: geom.x,
            y: geom.y,
        };
    }
    // 完全离屏：以第一块屏幕为「主屏」居中；窗口大于屏幕时 clamp 到左上角。
    let s = screens[0];
    let centered_x = s.x as i64 + (s.w as i64 - width as i64) / 2;
    let centered_y = s.y as i64 + (s.h as i64 - height as i64) / 2;
    let x = centered_x
        .max(s.x as i64)
        .clamp(i32::MIN as i64, i32::MAX as i64);
    let y = centered_y
        .max(s.y as i64)
        .clamp(i32::MIN as i64, i32::MAX as i64);
    WindowGeom {
        width,
        height,
        x: x as i32,
        y: y as i32,
    }
}

/// 正文字号（物理像素，缩放因子由调用方在 metrics/坐标上乘入）。
/// 单一来源：`tokens::typography::BODY_SIZE`。
pub const BASE_FONT_SIZE: f32 = crate::tokens::typography::BODY_SIZE;
/// 正文行高（物理像素）。
/// 单一来源：`tokens::typography::BODY_LINE_HEIGHT`。
pub const BASE_LINE_HEIGHT: f32 = crate::tokens::typography::BODY_LINE_HEIGHT;
/// 内容区左右边距（物理像素）。
/// 单一来源：`tokens::doc::MARGIN`。
pub const MARGIN: f32 = crate::tokens::doc::MARGIN;
/// 相邻块垂直间距（物理像素）。
/// 单一来源：`tokens::doc::PARA_GAP`。
pub const PARA_GAP: f32 = crate::tokens::doc::PARA_GAP;
/// 正文可读列宽上限（全角字符当量，随字号/缩放换算为物理像素）。
pub const READABLE_COLS: f32 = crate::tokens::doc::READABLE_COLS;
/// 历史常量：曾作为「窗口宽 > 1600 才封顶」的阈值（物理像素，scale = 1 基准）。
/// 版心封顶已改为按可用宽度与可读列宽比较，不再引用此值；保留仅为兼容旧引用。
pub const WIDE_WINDOW: f32 = crate::tokens::doc::WIDE_WINDOW;

/// 把物理像素长度吸附到整数像素网格（四舍五入，幂等）。
///
/// 光栅化原语（`Frame::fill_rect` / `fill_rounded_rect`）以整数物理像素为网格，
/// 而设计令牌按 `dpi_scale × user_zoom` 线性放大：1.25 这类缩放会把 6px 圆角
/// 乘成 7.5、2px 小圆角乘成 2.5，落在半像素上让边缘发糊。几何令牌在乘完 scale 后
/// 统一走本函数，保证边界落在整数物理像素；scale = 1.0 时全部令牌保持原值，
/// 观感不变。
pub(crate) fn snap_to_pixel(v: f32) -> f32 {
    v.round()
}

/// 屏幕版心边距（逻辑像素，未乘 scale）：由配置档位解析出运行时值。
/// `Custom` 使用用户值并夹取到 4..=200；Standard 与 `MARGIN` 常量同源。
pub(crate) fn content_margin(mode: DocMargin, custom_px: f32) -> f32 {
    mode.margin_px(custom_px)
}

/// 可读列宽上限（全角当量）：`Full` 档返回 `None`（不封顶），其余取 `READABLE_COLS`。
pub(crate) fn readable_cols(mode: DocMargin) -> Option<f32> {
    mode.readable_cap().then_some(READABLE_COLS)
}

/// 屏幕版心几何：返回 `(x, width)`，均为物理像素。
///
/// `margin_px` 为逻辑像素边距（未乘 scale）；`readable_cols` 为 `None` 时按窗口铺满。
/// 只要可用宽度大于可读上限，就按上限封顶并在边距内居中；可用宽度不足上限时
/// 降级为可用宽度（窄窗口保持铺满）。
/// `reserved_right`/`reserved_left` 为滚动条、大纲/历史面板占用的横向空间。
pub(crate) fn content_span(
    window_w: f32,
    scale: f32,
    margin_px: f32,
    readable_cols: Option<f32>,
    reserved_right: f32,
    reserved_left: f32,
) -> (f32, f32) {
    let margin = margin_px * scale;
    let available = (window_w - 2.0 * margin - reserved_right - reserved_left).max(32.0);
    let width = match readable_cols {
        Some(cols) => available.min(cols * BASE_FONT_SIZE * scale),
        None => available,
    };
    let x = margin + ((available - width) / 2.0).max(0.0) + reserved_left;
    (x, width)
}

/// 视口坐标下的轴对齐矩形（物理像素）。
///
/// 为什么放在 geometry：模态布局需要跨 view 子模块共享同一个矩形语义
/// （命中/绘制/测试都用它），避免再定义第四份同形结构。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Rect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

impl Rect {
    /// 点命中：左闭右开、上闭下开（与绘制矩形的半开边界一致）。
    pub(crate) fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

impl Default for Rect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom(width: u32, height: u32, x: i32, y: i32) -> WindowGeom {
        WindowGeom {
            width,
            height,
            x,
            y,
        }
    }

    fn screen(x: i32, y: i32, w: u32, h: u32) -> ScreenRect {
        ScreenRect { x, y, w, h }
    }

    #[test]
    fn clamps_size_to_bounds() {
        let screens = [screen(0, 0, 1920, 1080)];
        let too_small = clamp_window_geom(geom(100, 200, 10, 10), &screens);
        assert_eq!((too_small.width, too_small.height), (480, 360));
        let too_large = clamp_window_geom(geom(9000, 20000, 10, 10), &screens);
        assert_eq!((too_large.width, too_large.height), (8192, 8192));
        // 尺寸被夹后仍与屏幕有正面积交集：位置保持原样
        assert_eq!((too_large.x, too_large.y), (10, 10));
    }

    #[test]
    fn fully_offscreen_geom_is_centered_on_primary() {
        let screens = [screen(0, 0, 1920, 1080)];
        let g = clamp_window_geom(geom(800, 600, 5000, 5000), &screens);
        assert_eq!((g.width, g.height), (800, 600));
        assert_eq!((g.x, g.y), ((1920 - 800) / 2, (1080 - 600) / 2));
        // 负坐标方向的完全离屏同样居中（第二块屏不影响「主屏 = screens[0]」）
        let screens = [screen(0, 0, 1920, 1080), screen(-1920, 0, 1920, 1080)];
        let g = clamp_window_geom(geom(800, 600, -9000, -9000), &screens);
        assert_eq!((g.x, g.y), (560, 240));
    }

    #[test]
    fn oversized_window_sticks_to_screen_top_left() {
        // 窗口大于主屏：居中会算出负偏移，必须贴左上角保证标题栏可见
        let screens = [screen(0, 0, 1280, 720)];
        let g = clamp_window_geom(geom(2000, 1500, 9000, 9000), &screens);
        assert_eq!((g.width, g.height), (2000, 1500));
        assert_eq!((g.x, g.y), (0, 0));
        // 非零原点的屏幕：贴的是该屏幕的左上角
        let screens = [screen(1920, 0, 1280, 720)];
        let g = clamp_window_geom(geom(2000, 1500, 9000, 9000), &screens);
        assert_eq!((g.x, g.y), (1920, 0));
    }

    #[test]
    fn partially_visible_geom_keeps_position() {
        let screens = [screen(0, 0, 1920, 1080)];
        // 只露出右下 20×80 像素
        let g = clamp_window_geom(geom(800, 600, 1900, 1000), &screens);
        assert_eq!((g.x, g.y), (1900, 1000));
        // 只露出左上 1 像素也算可见（重叠宽高均 > 0）
        let g = clamp_window_geom(geom(800, 600, -799, -599), &screens);
        assert_eq!((g.x, g.y), (-799, -599));
    }

    #[test]
    fn empty_screens_only_clamps_size() {
        let g = clamp_window_geom(geom(100, 9000, -12345, 6789), &[]);
        assert_eq!((g.width, g.height), (480, 8192));
        assert_eq!((g.x, g.y), (-12345, 6789));
    }

    /// 版心档位 → 边距（逻辑像素）：四档固定值 + Custom 夹取。
    #[test]
    fn doc_margin_presets_and_custom_clamp() {
        let presets = [
            (DocMargin::Compact, 8.0),
            (DocMargin::Standard, 16.0),
            (DocMargin::Relaxed, 32.0),
            (DocMargin::Full, 16.0),
        ];
        for (mode, want) in presets {
            assert_eq!(
                content_margin(mode, 999.0),
                want,
                "{mode:?} 不应受自定义值影响"
            );
        }
        // Custom：正常值原样，越界夹到 4..=200。
        assert_eq!(content_margin(DocMargin::Custom, 64.0), 64.0);
        assert_eq!(content_margin(DocMargin::Custom, 0.0), 4.0);
        assert_eq!(content_margin(DocMargin::Custom, -5.0), 4.0);
        assert_eq!(content_margin(DocMargin::Custom, 999.0), 200.0);
        // 可读上限：仅 Full 关闭。
        assert_eq!(readable_cols(DocMargin::Full), None);
        for mode in [
            DocMargin::Compact,
            DocMargin::Standard,
            DocMargin::Relaxed,
            DocMargin::Custom,
        ] {
            assert_eq!(readable_cols(mode), Some(READABLE_COLS), "{mode:?}");
        }
    }

    /// 1200 逻辑宽：可用宽（≥ 640）已超过可读上限，四档 + Custom 都封顶并居中。
    #[test]
    fn content_span_caps_at_1200_when_available_exceeds_measure() {
        // 封顶宽 = 40 全角 × 16px = 640；居中后左缘 = 边距 + (可用宽 − 640)/2。
        // 各行边距恰好互补，故左缘统一落在 280。
        let cases = [
            (DocMargin::Compact, 16.0, 280.0, 640.0),
            (DocMargin::Standard, 16.0, 280.0, 640.0),
            (DocMargin::Relaxed, 16.0, 280.0, 640.0),
            (DocMargin::Full, 16.0, 16.0, 1168.0),
            (DocMargin::Custom, 4.0, 280.0, 640.0),
            (DocMargin::Custom, 200.0, 280.0, 640.0),
        ];
        for (mode, custom_px, want_x, want_w) in cases {
            let (x, w) = content_span(
                1200.0,
                1.0,
                content_margin(mode, custom_px),
                readable_cols(mode),
                0.0,
                0.0,
            );
            assert!((x - want_x).abs() < 1e-4, "{mode:?}/{custom_px}: x={x}");
            assert!((w - want_w).abs() < 1e-4, "{mode:?}/{custom_px}: w={w}");
        }
    }

    /// 窄窗口（可用宽 ≤ 可读上限）：不封顶，铺满可用宽、原点等于左边距。
    #[test]
    fn content_span_narrow_window_fills_available_width() {
        // 640 宽 / 16px 边距：可用宽 608 < 640，保持铺满。
        let (x, w) = content_span(
            640.0,
            1.0,
            content_margin(DocMargin::Standard, 16.0),
            readable_cols(DocMargin::Standard),
            0.0,
            0.0,
        );
        assert!((x - 16.0).abs() < 1e-4, "x={x}");
        assert!((w - 608.0).abs() < 1e-4, "w={w}");
        // 边界：可用宽恰好等于上限（672 − 32 = 640）时不缩窄、不偏移。
        let (x, w) = content_span(
            672.0,
            1.0,
            content_margin(DocMargin::Standard, 16.0),
            readable_cols(DocMargin::Standard),
            0.0,
            0.0,
        );
        assert!((x - 16.0).abs() < 1e-4, "边界 x={x}");
        assert!((w - 640.0).abs() < 1e-4, "边界 w={w}");
    }

    /// 1920 逻辑宽：Standard 封顶 40 全角并居中；Full 铺满且保留边距。
    #[test]
    fn content_span_at_1920_caps_and_centers_except_full() {
        // 封顶宽 = 40 全角 × 16px = 640；居中后左缘 = 边距 + (可用宽 − 640)/2。
        for (mode, custom_px) in [
            (DocMargin::Compact, 16.0),
            (DocMargin::Standard, 16.0),
            (DocMargin::Relaxed, 16.0),
            (DocMargin::Custom, 4.0),
        ] {
            let margin = content_margin(mode, custom_px);
            let (x, w) = content_span(1920.0, 1.0, margin, readable_cols(mode), 0.0, 0.0);
            assert!(
                (w - READABLE_COLS * BASE_FONT_SIZE).abs() < 1e-4,
                "{mode:?}: w={w}"
            );
            let available = 1920.0 - 2.0 * margin;
            let want_x = margin + (available - w) / 2.0;
            assert!((x - want_x).abs() < 1e-4, "{mode:?}: x={x}");
        }
        // Full：不封顶，铺满可用宽并保留 16px 边距。
        let (x, w) = content_span(
            1920.0,
            1.0,
            content_margin(DocMargin::Full, 16.0),
            readable_cols(DocMargin::Full),
            0.0,
            0.0,
        );
        assert!((x - 16.0).abs() < 1e-4, "Full x={x}");
        assert!((w - 1888.0).abs() < 1e-4, "Full w={w}");
        // 最小边距：Custom 取 0 仍按 4px 留白。
        let (x, _) = content_span(
            1200.0,
            1.0,
            content_margin(DocMargin::Custom, 0.0),
            readable_cols(DocMargin::Custom),
            0.0,
            0.0,
        );
        assert!(x >= 4.0, "自定义边距不得小于 4px：x={x}");
    }
}
