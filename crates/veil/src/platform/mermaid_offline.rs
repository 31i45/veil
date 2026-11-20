//! 离线 Mermaid 渲染适配层（veil-full 专用）。
//!
//! 与远程路线（`mermaid.rs`）的差别只在「SVG 从哪来」：
//! 远程 = mermaid.ink HTTP；离线 = QuickJS + mermaid.js（`veil-mermaid-offline`）。
//! 光栅化与导出用的 SVG 文本走同一条 resvg 管线，保证两个发行版渲染一致。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use veil_mermaid_offline::MermaidEngine;

/// 导出期同步渲染等待上限（离线本机渲染通常 < 1s）。
const BLOCKING_TIMEOUT: Duration = Duration::from_secs(30);

pub struct OfflineMermaid {
    engine: Option<MermaidEngine>,
    /// 引擎初始化失败原因（失败后所有请求立即返回该错误，UI 降级为源码块）。
    init_error: Option<String>,
    /// render_blocking 期间收到的其它块结果：暂存给下一次 poll，避免丢失。
    backlog: VecDeque<(String, Result<veil_ui::MermaidImage, String>)>,
    scale: f32,
}

impl OfflineMermaid {
    pub fn new() -> Self {
        let (engine, init_error) = match MermaidEngine::new() {
            Ok(engine) => (Some(engine), None),
            Err(e) => (None, Some(e)),
        };
        Self {
            engine,
            init_error,
            backlog: VecDeque::new(),
            scale: 2.0,
        }
    }

    /// SVG → 位图（并保留原始 SVG 文本供 HTML 导出内嵌）。
    fn convert(&self, svg: String) -> Result<veil_ui::MermaidImage, String> {
        let bmp = super::mermaid::svg_to_bitmap(&svg, self.scale)?;
        Ok(veil_ui::MermaidImage {
            data: bmp.data,
            width: bmp.width,
            height: bmp.height,
            svg_text: Some(svg),
            px_to_pt: bmp.px_to_pt,
        })
    }

    fn init_failure(&self) -> String {
        self.init_error
            .clone()
            .unwrap_or_else(|| "offline mermaid unavailable".into())
    }
}

impl veil_ui::MermaidRenderer for OfflineMermaid {
    fn request(&mut self, label: String, source: String, dark: bool) {
        match &self.engine {
            Some(engine) => engine.submit(label, source, dark),
            None => self.backlog.push_back((label, Err(self.init_failure()))),
        }
    }

    fn poll(&mut self) -> Vec<(String, Result<veil_ui::MermaidImage, String>)> {
        let raw = match &self.engine {
            Some(engine) => engine.try_recv(),
            None => Vec::new(),
        };
        let mut out: Vec<_> = self.backlog.drain(..).collect();
        for (label, res) in raw {
            let img = res.and_then(|svg| self.convert(svg));
            out.push((label, img));
        }
        out
    }

    fn set_dpi_scale(&mut self, scale: f32) {
        self.scale = scale;
    }

    /// 完全本地渲染：忽略「阻止远程图片」开关，避免离线产物被网络开关误关。
    fn set_allowed(&mut self, _allowed: bool) {}

    fn requires_network(&self) -> bool {
        false
    }

    fn render_blocking(
        &mut self,
        label: String,
        source: String,
        dark: bool,
    ) -> Result<veil_ui::MermaidImage, String> {
        let Some(engine) = &self.engine else {
            return Err(self.init_failure());
        };
        engine.submit(label.clone(), source, dark);

        let deadline = Instant::now() + BLOCKING_TIMEOUT;
        loop {
            let raw = match &self.engine {
                Some(engine) => engine.try_recv(),
                None => Vec::new(),
            };
            for (l, res) in raw {
                let img = res.and_then(|svg| self.convert(svg));
                if l == label {
                    return img;
                }
                self.backlog.push_back((l, img));
            }
            if Instant::now() > deadline {
                return Err(format!("offline mermaid timeout: {label}"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veil_ui::MermaidRenderer;

    /// 端到端：离线引擎出 SVG → resvg 光栅化 → 位图 + 原始 SVG 一起返回。
    /// HTML/PDF 导出依赖这两个字段（矢量优先，位图兜底）。
    #[test]
    fn offline_render_yields_bitmap_and_svg() {
        let mut renderer = OfflineMermaid::new();
        let img = renderer
            .render_blocking("lbl".into(), "graph TD\n A[开始] --> B[结束]".into(), false)
            .expect("offline render");
        assert!(img.width > 0 && img.height > 0, "empty bitmap");
        assert_eq!(img.data.len(), img.width as usize * img.height as usize * 4);
        let svg = img.svg_text.as_deref().unwrap_or_default();
        assert!(svg.contains("<svg"), "svg text missing");
        assert!(svg.contains("<text"), "labels must be <text> for resvg");
    }

    /// 完整版请求矩阵：关闭普通远程图片后，Mermaid 的异步预览与同步导出仍走本地引擎。
    #[test]
    fn offline_mermaid_ignores_remote_image_switch_for_preview_and_export() {
        let mut renderer = super::super::mermaid::mermaid_renderer();
        assert!(!renderer.requires_network());
        renderer.set_allowed(false);

        let exported = renderer
            .render_blocking("offline-export".into(), "graph TD\n A-->B".into(), false)
            .expect("blocking export must remain offline-capable");
        assert!(exported
            .svg_text
            .as_deref()
            .is_some_and(|s| s.contains("<svg")));

        renderer.request(
            "offline-preview".into(),
            "sequenceDiagram\n A->>B: local".into(),
            false,
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some((label, result)) = renderer
                .poll()
                .into_iter()
                .find(|(label, _)| label == "offline-preview")
            {
                assert_eq!(label, "offline-preview");
                let preview = result.expect("async preview must remain offline-capable");
                assert!(preview
                    .svg_text
                    .as_deref()
                    .is_some_and(|s| s.contains("<svg")));
                break;
            }
            assert!(Instant::now() < deadline, "offline preview timed out");
            std::thread::sleep(Duration::from_millis(5));
        }

        let live = veil_core::mermaid_live_url("graph TD\n A-->B");
        assert!(live.starts_with("https://mermaid.live/edit#pako:"));
        assert!(!live.contains("mermaid.ink"));
    }

    /// 语法错误：返回可读错误而不是 panic，且引擎可继续服务后续请求。
    #[test]
    fn offline_render_error_is_recoverable() {
        let mut renderer = OfflineMermaid::new();
        assert!(renderer
            .render_blocking("bad".into(), "graph TD\n A -->".into(), false)
            .is_err());
        let ok = renderer
            .render_blocking("good".into(), "pie title x\n \"a\" : 1".into(), true)
            .expect("recover");
        assert!(ok.width > 0);
    }
}
