//! 仅 T5 语料 harness 使用的只读门面（`#[doc(hidden)]`）。
//!
//! 不改产品路径：只把既有的 HTML / PDF 导出入口以最小表面积暴露给
//! `crates/veil/tests/syntax_matrix.rs`。默认纯内存调用；图片类用例由 harness
//! 传入真实临时目录，以验证相对图片内嵌与越界降级。

use std::collections::HashMap;
use std::path::Path;

use veil_core::Session;

/// 导出 HTML 全文（与产品导出同源：`html_of_doc(base_dir)`）。
/// `base_dir` 为 `Some` 时把该目录内的相对图片内嵌为 data URI。
/// 返回 Err 仅表示公式字体不可用（此时无法构造导出所需字体）。
pub fn html_of_doc(md: &str, base_dir: Option<&Path>) -> Result<String, String> {
    let font = latex_rust::MathFont::stix_two_math()
        .map_err(|_| "STIX 公式字体不可用".to_string())?;
    let session = Session::open(md);
    Ok(crate::export::html_of_doc(
        &session,
        &veil_ui::ExportStyle::default(),
        &font,
        base_dir,
        None,
    ))
}

/// PDF 版面/渲染产物：页数、按绘制顺序拼接的文本、PDF 字节长度。
pub struct PdfOutcome {
    pub page_count: usize,
    pub text: String,
    pub bytes_len: usize,
}

/// 本机 PDF 导出是否具备字体条件（无 CJK 字体时导出入口不可运行）。
pub fn pdf_available() -> bool {
    crate::platform::pdf::find_cjk_font().is_some()
}

/// 走现有 PDF 导出链路：FontMetrics → layout_document → render_pages。
/// 与打印/导出共用同一版面函数；不含图片解码（媒体接缝传 None，降级为源码/占位）。
pub fn pdf_of_doc(md: &str) -> Result<PdfOutcome, String> {
    use crate::platform::pdf::{find_cjk_font, render_pages, DrawOp, FontMetrics};
    use crate::platform::pdf_layout::{layout_document, LayoutOpts, PageGeom};

    let font_path = find_cjk_font().ok_or_else(|| "未找到 CJK 字体".to_string())?;
    let session = Session::open(md);
    let metrics = FontMetrics::load(Path::new(font_path), md.chars())?;
    let layout = layout_document(
        &session,
        &metrics,
        PageGeom::a4(20.0),
        LayoutOpts::default(),
        None,
        &HashMap::new(),
    );
    let bytes = render_pages(
        &layout.pages,
        &layout.images,
        &layout.outline,
        &layout.links,
        Path::new(font_path),
    )?;
    let mut text = String::new();
    for page in &layout.pages {
        for op in page {
            if let DrawOp::Text(t) = op {
                text.push_str(&t.text);
            }
        }
    }
    Ok(PdfOutcome {
        page_count: layout.pages.len(),
        text,
        bytes_len: bytes.len(),
    })
}
