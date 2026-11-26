//! Mermaid SVG → resvg 光栅化模块。
//!
//! 离线渲染管线：
//! ```text
//! trigger_mermaid_auto (veil-ui)
//!   → OfflineMermaid::request (QuickJS + mermaid.js，进程内出 SVG)
//!   → svg_to_bitmap(svg)
//!     → usvg::Tree::from_str(svg)          // 解析 SVG
//!     → resvg::render(tree, transform, pixmap) // 光栅化
//!   → mermaid::poll() → 取回 Bitmap 结果
//!   → cache.insert_mermaid(label, bitmap)
//!   → LayoutEngine 消费
//! ```
//!
//! 本模块不发起任何网络请求：SVG 只来自进程内离线引擎。

/// SVG → resvg 位图结果。
#[derive(Debug, Clone)]
pub struct MermaidBitmap {
    /// RGBA 像素缓冲区。
    pub data: Vec<u8>,
    /// 图像宽度（像素）。
    pub width: u32,
    /// 图像高度（像素）。
    pub height: u32,
    /// 光栅化时使用的 scale 对应的 px→pt 换算比：72.0 / (96.0 * scale)。
    /// scale 是 clamp 到 [2.0, 4.0] 后 resvg 实际使用的值。
    pub px_to_pt: f32,
}

/// 把 SVG 文本光栅化为 RGBA 位图。
///
/// resvg 0.50 API：`resvg::render(&usvg::Tree, Transform, &mut PixmapMut)`
pub fn svg_to_bitmap(svg_text: &str, dpi_scale: f32) -> Result<MermaidBitmap, String> {
    let scale = dpi_scale.clamp(2.0, 4.0);

    let mut opt = resvg::usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();

    let tree = resvg::usvg::Tree::from_str(svg_text, &opt)
        .map_err(|e| format!("SVG parse failed: {e}"))?;

    // usvg 0.50: Tree::size() 返回 SVG viewport 尺寸
    let orig = tree.size();
    let w = (orig.width() * scale).round().clamp(1.0, 4096.0) as u32;
    let h = (orig.height() * scale).round().clamp(1.0, 4096.0) as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| format!("pixmap alloc failed: {w}x{h}"))?;

    // tiny-skia-path 0.12: Transform::from_scale(sx, sy)
    let transform = tiny_skia_path::Transform::from_scale(scale, scale);

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // px_to_pt：72pt = 1 inch，基准 DPI = 96（CSS px），所以 px_to_pt = 72/(96*scale)
    let px_to_pt = 72.0 / (96.0 * scale);

    Ok(MermaidBitmap {
        data: pixmap.data().to_vec(),
        width: w,
        height: h,
        px_to_pt,
    })
}

/// 工厂函数：返回 MermaidRenderer trait 的平台实现。
///
/// 唯一实现是进程内离线渲染（QuickJS + 完整 mermaid.js），首次请求时才初始化
/// 引擎；不存在任何远程/在线回退路径。
#[cfg(feature = "mermaid-offline")]
pub fn mermaid_renderer() -> Box<dyn veil_ui::MermaidRenderer> {
    Box::new(super::mermaid_offline::OfflineMermaid::new())
}

#[cfg(not(feature = "mermaid-offline"))]
compile_error!("Veil 的 Mermaid 只在进程内渲染，必须启用 `mermaid-offline` feature");

#[cfg(test)]
mod tests {
    use super::svg_to_bitmap;

    /// 最小有效 SVG（resvg 可解析）。
    const VALID_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50"><rect x="5" y="5" width="90" height="40"/><text x="50" y="28">Hello</text></svg>"#;

    // =========================================================================
    // 测试 1 — svg_to_bitmap happy path
    // =========================================================================

    #[test]
    fn svg_to_bitmap_produces_nonempty_rgba() {
        let bm = svg_to_bitmap(VALID_SVG, 2.0).expect("有效 SVG 应光栅化成功");
        assert!(bm.width > 0, "宽度应 > 0");
        assert!(bm.height > 0, "高度应 > 0");
        assert!(!bm.data.is_empty(), "像素缓冲区非空");
        assert_eq!(bm.data.len() as u32, bm.width * bm.height * 4);
    }

    /// scale=3.0 时位图更大（scale 被 clamp 到 [2.0, 4.0]）。
    #[test]
    fn svg_to_bitmap_scale_changes_dimensions() {
        let bm2 = svg_to_bitmap(VALID_SVG, 2.0).expect("scale 2");
        let bm3 = svg_to_bitmap(VALID_SVG, 3.0).expect("scale 3");
        assert!(
            bm3.width >= bm2.width,
            "scale 3 应至少不小于 scale 2 的宽度：{} vs {}",
            bm3.width,
            bm2.width
        );
    }

    // =========================================================================
    // 测试 2 — svg_to_bitmap 错误路径
    // =========================================================================

    #[test]
    fn svg_to_bitmap_rejects_garbage_input() {
        let result = svg_to_bitmap("this is not svg at all", 2.0);
        assert!(result.is_err(), "垃圾文本必须 Err：{result:?}");
    }

    #[test]
    fn svg_to_bitmap_rejects_empty_input() {
        let result = svg_to_bitmap("", 2.0);
        assert!(result.is_err(), "空字符串必须 Err");
    }
}
