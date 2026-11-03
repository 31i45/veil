//! 公式渲染抽象（INSTANCE §6 平台接缝）：实现由产品层（veil）注入，
//! 与 Clipboard/FileStore 同模式——本层只见 trait，fake 注入即 headless 可测。
//! 渲染调度策略（何时渲染、光标在内显源码、失败降级）全在 shell；本文件只有类型。

/// 渲染产物：RGBA8 位图（frame.blit 的直接输入）。黑字透明底下 straight/预乘等价
/// （颜色通道恒为 0），latex-rust 的 tiny-skia 输出可直接消费。
pub struct MathImage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// 图顶到基线的像素：行内公式以此与文本基线对齐；块级 display 图整图替换块，置 0 不用。
    pub baseline: u32,
}

/// 公式渲染器。`display`：true = 块级 `$$` 展示模式，false = 行内 `$` 文本模式（小字号）。
/// `scale` 为 DPI 缩放因子，实现据此调整输出像素密度。
/// `color` 为字形颜色（主题前景色）：深色主题下公式须与正文同色才可读（切片 17）。
/// 失败返回 Err：视图降级为源码显示，不崩溃（ADR-002 后果段的既定降级策略）。
pub trait MathRenderer {
    fn render(
        &mut self,
        tex: &str,
        display: bool,
        scale: f32,
        color: [u8; 3],
    ) -> Result<MathImage, String>;
}
