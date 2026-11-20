//! mermaid.ink SVG 下载 + resvg 光栅化模块。
//!
//! 远程渲染管线：
//! ```text
//! trigger_mermaid_auto (veil-ui)
//!   → mermaid_ink_svg_url(source, dark)
//!   → mermaid::request(url)               // 后台线程
//!     → http::fetch_raw_bytes(url)        // WinHTTP 下载原始字节
//!     → String::from_utf8 → SVG 文本
//!     → usvg::Tree::from_str(svg)          // 解析 SVG
//!     → resvg::render(tree, transform, pixmap) // 光栅化
//!   → mermaid::poll() → 取回 Bitmap 结果
//!   → cache.insert_mermaid(url, bitmap)
//!   → LayoutEngine 消费
//! ```

use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

#[cfg(windows)]
use crate::platform::http::fetch_raw_response;
use crate::platform::http::RawResponse;

type Transport = dyn Fn(&str) -> Result<RawResponse, String> + Send + Sync + 'static;

#[cfg(windows)]
fn system_transport() -> Arc<Transport> {
    Arc::new(fetch_raw_response)
}

#[cfg(not(windows))]
fn system_transport() -> Arc<Transport> {
    Arc::new(|_| Err(veil_ui::i18n::t(veil_ui::i18n::Key::ErrMermaidHttpUnsupported).into()))
}

/// SVG → resvg 位图结果。
#[derive(Debug, Clone)]
pub struct MermaidBitmap {
    /// RGBA 像素缓冲区。
    pub data: Vec<u8>,
    /// 图像宽度（像素）。
    pub width: u32,
    /// 图像高度（像素）。
    pub height: u32,
    /// 原始 mermaid.ink 返回的 SVG 文本（HTML 导出内嵌用）。
    pub svg_text: Option<String>,
    /// 光栅化时使用的 scale 对应的 px→pt 换算比：72.0 / (96.0 * scale)。
    /// scale 是 clamp 到 [2.0, 4.0] 后 resvg 实际使用的值。
    pub px_to_pt: f32,
}

/// 把 mermaid.ink 返回的 SVG 文本光栅化为 RGBA 位图。
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
        svg_text: None,
        px_to_pt,
    })
}

/// 校验远程响应并转换为位图。Content-Type 必须先于 UTF-8/SVG 解析校验，
/// 避免把 HTML 错误页或二进制图片当成 Mermaid 结果。
fn response_to_bitmap(response: RawResponse, dpi_scale: f32) -> MermaidResult {
    let content_type = response.content_type.as_deref().unwrap_or("<missing>");
    let mime = content_type.split(';').next().unwrap_or("").trim();
    if !mime.eq_ignore_ascii_case("image/svg+xml") {
        return Err(format!(
            "unexpected Content-Type: {content_type} (expected image/svg+xml)"
        ));
    }
    let svg = String::from_utf8(response.body).map_err(|e| format!("UTF-8: {e}"))?;
    let mut bitmap = svg_to_bitmap(&svg, dpi_scale)?;
    bitmap.svg_text = Some(svg);
    Ok(bitmap)
}

fn fetch_bitmap(url: &str, dpi_scale: f32, transport: &Transport) -> MermaidResult {
    let response = transport(url).map_err(|e| format!("HTTP: {e}"))?;
    response_to_bitmap(response, dpi_scale)
}

/// 单个请求的完整结果（URL + 光栅化位图 或 错误）。
pub type MermaidResult = Result<MermaidBitmap, String>;

/// 后台线程下载 + resvg 光栅化的 FIFO 收集器。
pub struct MermaidFetcher {
    sender: mpsc::Sender<(String, MermaidResult)>,
    receiver: mpsc::Receiver<(String, MermaidResult)>,
    dpi_scale: f32,
    /// allow_remote_images 开关。false 时 request() 直接拒绝投递。
    allowed: bool,
    transport: Arc<Transport>,
    blocking_timeout: Duration,
}

#[cfg_attr(feature = "mermaid-offline", allow(dead_code))]
impl MermaidFetcher {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            sender: tx,
            receiver: rx,
            dpi_scale: 2.0,
            allowed: true,
            transport: system_transport(),
            blocking_timeout: Duration::from_secs(3),
        }
    }

    pub fn set_dpi_scale(&mut self, scale: f32) {
        self.dpi_scale = scale;
    }

    /// 设置远程请求开关。设为 false 后 request() 不再接收新投递。
    pub fn set_allowed(&mut self, allowed: bool) {
        self.allowed = allowed;
    }

    /// 投递 mermaid.ink SVG 下载 + resvg 光栅化请求。
    /// 当 allowed = false 时直接拒绝（不 spawn 线程、不发送任何网络请求）。
    pub fn request(&mut self, url: String) {
        if !self.allowed {
            return;
        }
        let tx = self.sender.clone();
        let dpi = self.dpi_scale;
        let transport = Arc::clone(&self.transport);
        thread::spawn(move || {
            let label = url.clone();
            let result = fetch_bitmap(&url, dpi, transport.as_ref());
            let _ = tx.send((label, result));
        });
    }

    /// 同步渲染（导出/打印路径用）：阻塞至多 3s 后返回位图。
    /// 与 request() 走同一条 HTTP + resvg 管线，但不开 poll 队列、直接回传结果。
    /// allowed = false 时直接失败（不发起任何网络请求）。
    pub fn render_blocking(&mut self, url: String) -> MermaidResult {
        if !self.allowed {
            return Err("remote images disabled".into());
        }
        let (tx, rx) = mpsc::channel();
        let transport = Arc::clone(&self.transport);
        thread::spawn(move || {
            let result = fetch_bitmap(&url, 2.0, transport.as_ref());
            let _ = tx.send(result);
        });
        rx.recv_timeout(self.blocking_timeout)
            .map_err(|_| "timeout".to_string())?
    }

    /// 非阻塞取走全部已完成结果。
    pub fn poll(&mut self) -> Vec<(String, MermaidResult)> {
        let mut out = Vec::new();
        while let Ok(item) = self.receiver.try_recv() {
            out.push(item);
        }
        out
    }
}

impl Default for MermaidFetcher {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// veil-ui trait 桥：MermaidFetcher → veil_ui::MermaidRenderer
// ---------------------------------------------------------------------------

impl veil_ui::MermaidRenderer for MermaidFetcher {
    fn request(&mut self, label: String, _source: String, _dark: bool) {
        MermaidFetcher::request(self, label);
    }
    fn poll(&mut self) -> Vec<(String, Result<veil_ui::MermaidImage, String>)> {
        MermaidFetcher::poll(self)
            .into_iter()
            .map(|(url, r)| {
                let mapped = r.map(|b| veil_ui::MermaidImage {
                    data: b.data,
                    width: b.width,
                    height: b.height,
                    svg_text: b.svg_text,
                    px_to_pt: b.px_to_pt,
                });
                (url, mapped)
            })
            .collect()
    }
    fn set_dpi_scale(&mut self, scale: f32) {
        MermaidFetcher::set_dpi_scale(self, scale);
    }
    fn set_allowed(&mut self, allowed: bool) {
        MermaidFetcher::set_allowed(self, allowed);
    }
    fn render_blocking(
        &mut self,
        label: String,
        _source: String,
        _dark: bool,
    ) -> Result<veil_ui::MermaidImage, String> {
        MermaidFetcher::render_blocking(self, label).map(|b| veil_ui::MermaidImage {
            data: b.data,
            width: b.width,
            height: b.height,
            svg_text: b.svg_text,
            px_to_pt: b.px_to_pt,
        })
    }
}

/// 工厂函数：返回 MermaidRenderer trait 的平台实现。
///
/// - 默认（veil.exe）：mermaid.ink 远程 SVG + 本地 resvg 光栅化（联网可用）；
/// - `mermaid-offline` feature（veil-full.exe）：QuickJS + 完整 mermaid.js，完全离线。
#[cfg(not(feature = "mermaid-offline"))]
pub fn mermaid_renderer() -> Box<dyn veil_ui::MermaidRenderer> {
    Box::new(MermaidFetcher::new())
}

#[cfg(feature = "mermaid-offline")]
pub fn mermaid_renderer() -> Box<dyn veil_ui::MermaidRenderer> {
    Box::new(super::mermaid_offline::OfflineMermaid::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const VALID_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" viewBox="0 0 40 20"><rect width="40" height="20" fill="white"/><text x="2" y="14">OK</text></svg>"#;

    fn response(body: &str, content_type: Option<&str>) -> RawResponse {
        RawResponse {
            body: body.as_bytes().to_vec(),
            content_type: content_type.map(str::to_owned),
        }
    }

    fn mocked(
        transport: impl Fn(&str) -> Result<RawResponse, String> + Send + Sync + 'static,
    ) -> MermaidFetcher {
        let mut fetcher = MermaidFetcher::new();
        fetcher.transport = Arc::new(transport);
        fetcher
    }

    #[test]
    fn remote_svg_mock_succeeds_for_preview_and_export() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport_calls = Arc::clone(&calls);
        let mut fetcher = mocked(move |url| {
            assert_eq!(url, "https://example.com/diagram.svg");
            transport_calls.fetch_add(1, Ordering::SeqCst);
            Ok(response(VALID_SVG, Some("Image/SVG+XML; charset=utf-8")))
        });

        fetcher.request("https://example.com/diagram.svg".into());
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let preview = loop {
            if let Some((label, result)) = fetcher.poll().into_iter().next() {
                assert_eq!(label, "https://example.com/diagram.svg");
                break result.expect("mock preview SVG");
            }
            assert!(std::time::Instant::now() < deadline, "preview timed out");
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(preview.svg_text.as_deref(), Some(VALID_SVG));
        assert!(!preview.data.is_empty());

        let export = fetcher
            .render_blocking("https://example.com/diagram.svg".into())
            .expect("mock export SVG");
        assert_eq!(export.svg_text.as_deref(), Some(VALID_SVG));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn remote_svg_mock_rejects_wrong_content_type_and_invalid_svg() {
        for (body, content_type, expected) in [
            (VALID_SVG, Some("text/html"), "Content-Type"),
            (VALID_SVG, None, "Content-Type"),
            ("<html>not svg</html>", Some("image/svg+xml"), "SVG parse"),
        ] {
            let mut fetcher = mocked(move |_| Ok(response(body, content_type)));
            let error = fetcher
                .render_blocking("https://example.com/mock.svg".into())
                .expect_err("invalid mock response must fail");
            assert!(error.contains(expected), "unexpected error: {error}");
        }
    }

    #[test]
    fn remote_svg_mock_times_out_and_recovers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport_calls = Arc::clone(&calls);
        let mut fetcher = mocked(move |_| {
            if transport_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                thread::sleep(Duration::from_millis(100));
            }
            Ok(response(VALID_SVG, Some("image/svg+xml")))
        });
        fetcher.blocking_timeout = Duration::from_millis(20);

        let error = fetcher
            .render_blocking("https://example.com/slow.svg".into())
            .expect_err("slow mock must time out");
        assert!(error.contains("timeout"), "{error}");
        fetcher.blocking_timeout = Duration::from_secs(3);
        let recovered = fetcher
            .render_blocking("https://example.com/recovered.svg".into())
            .expect("later request must recover");
        assert_eq!(recovered.svg_text.as_deref(), Some(VALID_SVG));
    }

    #[test]
    fn remote_disabled_never_calls_transport() {
        let calls = Arc::new(AtomicUsize::new(0));
        let transport_calls = Arc::clone(&calls);
        let mut fetcher = mocked(move |_| {
            transport_calls.fetch_add(1, Ordering::SeqCst);
            Ok(response(VALID_SVG, Some("image/svg+xml")))
        });
        fetcher.set_allowed(false);
        fetcher.request("https://example.com/blocked.svg".into());
        assert!(fetcher
            .render_blocking("https://example.com/blocked.svg".into())
            .is_err());
        thread::sleep(Duration::from_millis(20));
        assert!(fetcher.poll().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[cfg(not(feature = "mermaid-offline"))]
    #[test]
    fn lightweight_network_contract_uses_ink_and_live_only_for_editing() {
        let renderer = mermaid_renderer();
        assert!(renderer.requires_network());

        let ink = veil_core::mermaid_ink_svg_url("graph TD\n A-->B", false);
        assert!(ink.starts_with("https://mermaid.ink/svg/"));
        assert!(!ink.contains("mermaid.live"));
        let live = veil_core::mermaid_live_url("graph TD\n A-->B");
        assert!(live.starts_with("https://mermaid.live/edit#pako:"));
        assert!(!live.contains("mermaid.ink"));
    }
}
