//! 离线 Mermaid 渲染适配层（唯一发行产物 veil.exe 的渲染器）。
//!
//! SVG 来自 QuickJS + mermaid.js（`veil-mermaid-offline`），全程在进程内完成，
//! 不发起任何网络请求。光栅化与导出用的 SVG 文本走同一条 resvg 管线。
//!
//! 惰性初始化：`OfflineMermaid::new()` 只构造空句柄，不 spawn 线程、不加载 QuickJS；
//! 首次 `request` / `render_blocking` 才创建 `MermaidEngine`。这样没有 Mermaid 块的
//! 文档不为未使用的引擎付空闲内存成本（见 evidence/decision/baseline.md §10）。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use veil_mermaid_offline::MermaidEngine;

/// 导出期同步渲染等待上限（离线本机渲染通常 < 1s）。
const BLOCKING_TIMEOUT: Duration = Duration::from_secs(30);

pub struct OfflineMermaid {
    /// 首次渲染请求时才创建；`None` 表示尚未初始化。
    engine: Option<MermaidEngine>,
    /// render_blocking 期间收到的其它块结果：暂存给下一次 poll，避免丢失。
    backlog: VecDeque<(String, Result<veil_ui::MermaidImage, String>)>,
    scale: f32,
}

impl OfflineMermaid {
    /// 惰性句柄：不 spawn、不加载 QuickJS，空闲进程零引擎成本。
    pub fn new() -> Self {
        Self {
            engine: None,
            backlog: VecDeque::new(),
            scale: 2.0,
        }
    }

    /// 首次使用时创建离线引擎；创建失败返回可读错误，下次请求重试（可恢复）。
    fn ensure_engine(&mut self) -> Result<(), String> {
        if self.engine.is_some() {
            return Ok(());
        }
        match MermaidEngine::new() {
            Ok(engine) => {
                self.engine = Some(engine);
                Ok(())
            }
            Err(e) => Err(e),
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

    #[cfg(test)]
    fn engine_started(&self) -> bool {
        self.engine.is_some()
    }
}

impl Default for OfflineMermaid {
    fn default() -> Self {
        Self::new()
    }
}

impl veil_ui::MermaidRenderer for OfflineMermaid {
    fn request(&mut self, label: String, source: String, dark: bool) {
        if let Err(e) = self.ensure_engine() {
            self.backlog.push_back((label, Err(e)));
            return;
        }
        if let Some(engine) = &self.engine {
            engine.submit(label, source, dark);
        }
    }

    fn poll(&mut self) -> Vec<(String, Result<veil_ui::MermaidImage, String>)> {
        let raw = match &self.engine {
            Some(engine) => engine.try_recv(),
            // 未初始化：没有在途渲染，只返回 backlog（含初始化失败的可读错误）。
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

    fn render_blocking(
        &mut self,
        label: String,
        source: String,
        dark: bool,
    ) -> Result<veil_ui::MermaidImage, String> {
        self.ensure_engine()?;
        if let Some(engine) = &self.engine {
            engine.submit(label.clone(), source, dark);
        }

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

    /// D1 前置条件：`new()` 不创建引擎；首次渲染请求才初始化，之后立即可用。
    #[test]
    fn new_is_lazy_and_first_render_initializes_engine() {
        let mut renderer = OfflineMermaid::new();
        assert!(!renderer.engine_started(), "new() 不得创建离线引擎");
        assert!(renderer.poll().is_empty(), "未初始化时 poll 只返回 backlog");

        let img = renderer
            .render_blocking("lazy".into(), "graph TD\n A-->B".into(), false)
            .expect("首次 render_blocking 应触发惰性初始化并成功");
        assert!(img.width > 0 && img.height > 0);
        assert!(
            renderer.engine_started(),
            "首次渲染后引擎应已初始化，后续请求复用同一实例"
        );

        // 同一个实例继续服务异步预览请求。
        renderer.request(
            "lazy-preview".into(),
            "pie title x\n \"a\" : 1".into(),
            false,
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some((label, result)) = renderer
                .poll()
                .into_iter()
                .find(|(label, _)| label == "lazy-preview")
            {
                assert_eq!(label, "lazy-preview");
                assert!(result.expect("惰性初始化后异步预览可用").width > 0);
                break;
            }
            assert!(Instant::now() < deadline, "offline preview timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// 完整版请求矩阵：异步预览与同步导出都走本地引擎，互不干扰。
    #[test]
    fn offline_mermaid_preview_and_export_are_always_local() {
        let mut renderer = super::super::mermaid::mermaid_renderer();

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
