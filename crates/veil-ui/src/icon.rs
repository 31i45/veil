// 窗口图标像素：与 exe 内嵌 ICO 同源的设计稿（icons/ 下）导出的 32×32 RGBA 原始字节。
// 图标以图片为唯一真源（icons/Markdown.png、icons/veil.ico、icons/veil-doc.ico），
// 本模块只负责把窗口图标所需的 RGBA 视图提供给 winit。

const WINDOW_ICON_32: &[u8] = include_bytes!("../../../icons/veil-32.rgba");

/// 32×32 RGBA 像素（自顶向下，每像素 4 字节），供 winit `with_window_icon`。
pub fn icon_rgba() -> Vec<u8> {
    WINDOW_ICON_32.to_vec()
}
