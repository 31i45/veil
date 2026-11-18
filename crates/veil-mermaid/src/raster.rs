//! resvg 栅格化封装：mermaid SVG → 不透明 RGBA 位图（调用方线程执行）。
//!
//! fontdb 全局缓存（OnceLock）：spike 实测系统字体冷加载 105–255ms/次，
//! 缓存后 parse ≈1ms。sans-serif 兜底设为 Microsoft YaHei，保证 CJK 不缺字。
//! 背景按主题 bg 预填（浅色主题即白底）：透明底 + 深色主题浅字在查看器中
//! 不可辨（spike 踩坑结论），且位图恒不透明，屏幕 blit / PDF 嵌入零分支。

use std::sync::{Arc, OnceLock};

use veil_ui::{MermaidBitmap, MermaidError};

/// 位图防护：异常超大 viewBox 不得撑爆内存。
const MAX_SIDE: u32 = 16384;
const MAX_PIXELS: u64 = 4096 * 4096;

/// 全局共享 fontdb（只读）：加载一次系统字体，所有栅格化复用。
fn fontdb() -> &'static Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        db.set_sans_serif_family("Microsoft YaHei");
        Arc::new(db)
    })
}

/// SVG → RGBA。`scale` 为栅格化倍率（设备 DPI）；`bg` 为预填背景色。
pub(crate) fn rasterize(svg: &str, scale: f32, bg: [u8; 3]) -> Result<MermaidBitmap, MermaidError> {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let options = usvg::Options {
        fontdb: Arc::clone(fontdb()),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|e| MermaidError::new(format!("Mermaid SVG 解析失败：{e}")))?;
    let size = tree.size();
    let w = ((size.width() * scale).round() as u32).clamp(1, MAX_SIDE);
    let h = ((size.height() * scale).round() as u32).clamp(1, MAX_SIDE);
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(MermaidError::new("Mermaid 图过大，无法栅格化"));
    }
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(w, h).ok_or_else(|| MermaidError::new("Mermaid 位图分配失败"))?;
    // 预填不透明背景（逐像素写比 fill_rect 通用；背景色随主题）。
    for px in pixmap.data_mut().chunks_exact_mut(4) {
        px.copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
    }
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(MermaidBitmap {
        rgba: Arc::new(pixmap.take()),
        width: w,
        height: h,
    })
}
