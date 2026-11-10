//! 跨域共享的几何常量（排版度量与布局间距）。
//!
//! 这些常量是纯数值，不属于任何特定域的派生状态，故独立于 document/view/app，
//! 供各域按名引用，避免真值层（document）反向依赖派生层（view）的常量。

use crate::theme::WindowGeom;

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
pub const BASE_FONT_SIZE: f32 = 16.0;
/// 正文行高（物理像素）。
pub const BASE_LINE_HEIGHT: f32 = 22.0;
/// 内容区左右边距（物理像素）。
pub const MARGIN: f32 = 16.0;
/// 块间垂直间距（物理像素）。
pub const PARA_GAP: f32 = 10.0;
/// 正文可读列宽上限（全角字符当量，随字号/缩放换算为物理像素）。
pub const READABLE_COLS: f32 = 40.0;
/// 启用可读列宽上限的窗口宽度阈值（物理像素，scale = 1 基准）。
/// 窄窗口保持满宽排版，只有超宽窗口才左右留白居中。
pub const WIDE_WINDOW: f32 = 1600.0;

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
}
