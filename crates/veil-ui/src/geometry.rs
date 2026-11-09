//! 跨域共享的几何常量（排版度量与布局间距）。
//!
//! 这些常量是纯数值，不属于任何特定域的派生状态，故独立于 document/view/app，
//! 供各域按名引用，避免真值层（document）反向依赖派生层（view）的常量。

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
