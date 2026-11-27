//! 仅 T5 语料 harness 使用的只读门面（`#[doc(hidden)]`）。
//!
//! 不改产品路径：只把既有的 HTML / PDF 导出入口以最小表面积暴露给
//! `crates/veil/tests/syntax_matrix.rs`。默认纯内存调用；图片类用例由 harness
//! 传入真实临时目录，以验证相对图片内嵌与越界降级。

use std::collections::HashMap;
use std::path::Path;

use veil_core::{BlockId, BlockKind, Session};

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

/// 同 [`html_of_doc`]，但先为每个 Mermaid 块注入真实离线渲染结果。
///
/// 渲染走产品同一实现（`platform::mermaid_offline::OfflineMermaid`：进程内
/// QuickJS + mermaid.js → resvg，无网络），再把 `BlockId → MermaidImage` 交给
/// 同一条 `export::html_of_doc`。任何一块渲染失败都返回 Err——调用方据此判 FAIL，
/// 而不是让 `mermaids=None` 静默降级成源码围栏。
#[cfg(feature = "mermaid-offline")]
pub fn html_of_doc_with_mermaid(md: &str, base_dir: Option<&Path>) -> Result<String, String> {
    let font = latex_rust::MathFont::stix_two_math()
        .map_err(|_| "STIX 公式字体不可用".to_string())?;
    let session = Session::open(md);
    let mermaids = render_mermaids(&session)?;
    Ok(crate::export::html_of_doc(
        &session,
        &veil_ui::ExportStyle::default(),
        &font,
        base_dir,
        (!mermaids.is_empty()).then_some(&mermaids),
    ))
}

/// 进程内复用的离线 Mermaid 渲染器。
///
/// QuickJS 首次初始化需解析约 3 MB JS，逐用例重建代价高；用 `OnceLock` 让整个
/// 测试进程只初始化一次，后续用例复用同一工作线程/引擎。
#[cfg(feature = "mermaid-offline")]
fn render_mermaids(session: &Session) -> Result<HashMap<BlockId, veil_ui::MermaidImage>, String> {
    use std::sync::{Mutex, OnceLock};

    use veil_ui::MermaidRenderer;

    let ids: Vec<BlockId> = session
        .blocks()
        .into_iter()
        .filter_map(|(id, kind, _)| (kind == BlockKind::Mermaid).then_some(id))
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    static RENDERER: OnceLock<Mutex<crate::platform::mermaid_offline::OfflineMermaid>> =
        OnceLock::new();
    let mut renderer = RENDERER
        .get_or_init(|| Mutex::new(crate::platform::mermaid_offline::OfflineMermaid::new()))
        .lock()
        .map_err(|_| "Mermaid 渲染器锁中毒".to_string())?;

    let mut out = HashMap::new();
    for id in ids {
        // block range 含 ``` 围栏行；导出/预览都只吃围栏**内容**。
        let source = session
            .fenced_code_source(id)
            .map(|(_, body)| body)
            .unwrap_or_default();
        let image = renderer.render_blocking(format!("case-{}", id.0), source, false)?;
        out.insert(id, image);
    }
    Ok(out)
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
