//! Mermaid 图表渲染 trait（veil-ui 平台能力抽象）。
//!
//! 与 MathRenderer 同架构：veil-ui 只见 trait，veil 平台层注入实现
//! （当前走 mermaid.ink SVG 远程下载 + resvg 光栅化；预留 Build Feature
//! 切换到本地 QuickJS + mermaid.js 的离线实现）。

/// resvg 光栅化位图 + 可选原始 SVG 文本。
/// HTML 导出优先用 svg_text 内嵌 SVG（矢量清晰），无则退化为 base64 PNG。
#[derive(Clone)]
pub struct MermaidImage {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// 原始 mermaid.ink 返回的 SVG 文本（导出用）。
    pub svg_text: Option<String>,
    /// 光栅化时使用的 scale 对应的 px→pt 换算比：72.0 / (96.0 * scale)。
    /// 用于 PDF 布局把像素尺寸转成 pt（1pt = 1/72 inch）。
    pub px_to_pt: f32,
}

/// 远程/本地 Mermaid SVG → 位图 渲染能力。
///
/// 返回 `None` 表示「本帧无新完成的请求」；Err 表示渲染失败（源码错误、
/// 网络不可达、resvg 解析失败等）——上层降级为源码块 + 灰色提示。
///
/// 请求携带三要素：
/// - `label`：调用方生成的稳定键（当前是 mermaid.ink URL），结果回传时原样带回，
///   上层用它匹配 pending 队列 / 缓存；
/// - `source`：Mermaid 源码（离线实现直接渲染它，远程实现可忽略）；
/// - `dark`：当前主题（决定配色）。
pub trait MermaidRenderer {
    /// 投递一次渲染请求。
    fn request(&mut self, label: String, source: String, dark: bool);
    /// 非阻塞取走全部已完成的光栅化结果（label + 位图/错误）。
    fn poll(&mut self) -> Vec<(String, Result<MermaidImage, String>)>;
    /// 设置 DPI scale（影响 resvg 光栅化倍率）。
    fn set_dpi_scale(&mut self, scale: f32);
    /// 远程图片总开关。设为 false 时后台线程应拒绝接收新 request()。
    /// 默认空实现——测试 stub / 离线实现无需关心网络开关。
    fn set_allowed(&mut self, _allowed: bool) {}
    /// 该渲染器是否依赖网络。默认 true（远程实现）；
    /// 离线实现返回 false，使「阻止远程图片」开关不影响本地产图。
    fn requires_network(&self) -> bool {
        true
    }
    /// 同步渲染（导出/打印路径用）：阻塞至多内部超时后返回位图。
    /// 默认实现直接失败——测试 stub 无需关心。
    fn render_blocking(
        &mut self,
        _label: String,
        _source: String,
        _dark: bool,
    ) -> Result<MermaidImage, String> {
        Err("blocking render unsupported".into())
    }
}
