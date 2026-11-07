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
