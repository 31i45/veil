//! Mermaid 渲染接口（后端中立）：源码 + 主题 → SVG 或统一矢量场景。
//!
//! 本模块只定义抽象与默认空实现，不引入任何渲染运行时：无 wasmtime、无 JS 引擎、
//! 无新增依赖。未安装渲染器时上层保持既有 v0 降级（卡片头 + 源码文本 +
//! mermaid.live 预览），行为与本轮之前逐字节一致。
//!
//! 三端（L1 内嵌块、预览面板、HTML、PDF）共用同一入口 [`MermaidRenderer`] 与
//! [`MermaidScene`]：屏幕与打印按场景绘制，HTML 优先内嵌后端自带的 SVG（经
//! [`sanitize_svg`] 清理），没有 SVG 时由 [`scene_to_svg`] 从场景派生。
//!
//! # 未来后端接入点
//!
//! 1. **wasmtime + mermaid-wasm**：把 mermaid.js 编译成 wasm，用极简 DOM shim 补齐
//!    其运行依赖；输出 SVG 直接填 [`MermaidDrawing::svg`]，矢量场景按 SVG 元素
//!    （rect/line/text/path）映射为 [`MermaidScene`]。
//! 2. **QuickJS（rquickjs 等）**：嵌入 JS 引擎运行 mermaid.js，SVG 解析同上；
//!    体积小于 wasmtime，但需自行做内存/超时隔离。
//! 3. **原生 Rust 渲染器**：直接产出 [`MermaidScene`]，无 JS 运行时，启动最快。
//!
//! 选型标准：首帧延迟、二进制体积、沙箱与超时可控性、SVG 保真度、许可证。
//! 评测完成前只保留接口，不锁定任何后端，也不宣称 Mermaid 已可渲染。

use crate::geometry::{BASE_FONT_SIZE, BASE_LINE_HEIGHT};
use crate::theme::{ExportStyle, Theme};
use crate::view::{Frame, LayoutEngine};

/// 渲染主题：只包含后端需要的颜色与度量（不暴露 UI 内部结构）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MermaidTheme {
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub accent: [u8; 3],
    pub muted: [u8; 3],
    /// 正文基准字号（逻辑像素）。
    pub font_size: f32,
    pub line_height: f32,
    pub dark: bool,
}

impl MermaidTheme {
    /// 屏幕主题快照。`scale` 为 DPI 缩放，仅用于派生正文度量。
    pub fn from_theme(theme: &Theme, scale: f32) -> Self {
        Self {
            fg: theme.fg,
            bg: [theme.bg[0], theme.bg[1], theme.bg[2]],
            accent: theme.accent,
            muted: theme.muted,
            font_size: BASE_FONT_SIZE * scale,
            line_height: BASE_LINE_HEIGHT * scale,
            dark: theme.dark,
        }
    }

    /// 导出主题快照：PDF/HTML 恒浅色页面，度量取 PDF 正文字号。
    pub fn from_export(style: &ExportStyle) -> Self {
        Self {
            fg: style.text,
            bg: style.bg,
            accent: style.link,
            muted: style.muted,
            font_size: 11.0,
            line_height: 16.0,
            dark: style.dark,
        }
    }
}

/// 矢量图元：只保留三端都能直接表达的最小集合。
#[derive(Clone, Debug, PartialEq)]
pub enum MermaidItem {
    /// 实心矩形（节点框/背景）。`fill[3] == 0` 时跳过绘制。
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: [u8; 4],
    },
    /// 线段（连线/边框）。`width` 为线宽（场景坐标）。
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        width: f32,
        color: [u8; 4],
    },
    /// 文本：`x`/`y` 为左上角，`size` 为字号（场景坐标）。
    Text {
        x: f32,
        y: f32,
        size: f32,
        text: String,
        color: [u8; 3],
    },
}

/// 矢量场景：宽高为场景自身坐标（后端自定单位，三端等比缩放）。
#[derive(Clone, Debug, PartialEq)]
pub struct MermaidScene {
    pub width: u32,
    pub height: u32,
    pub items: Vec<MermaidItem>,
}

impl MermaidScene {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: width.max(1),
            height: height.max(1),
            items: Vec::new(),
        }
    }

    pub fn rect(
        mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: [u8; 4],
    ) -> Self {
        self.items.push(MermaidItem::Rect { x, y, w, h, fill });
        self
    }

    pub fn line(mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: [u8; 4]) -> Self {
        self.items.push(MermaidItem::Line {
            x1,
            y1,
            x2,
            y2,
            width,
            color,
        });
        self
    }

    pub fn text(mut self, x: f32, y: f32, size: f32, text: impl Into<String>, color: [u8; 3]) -> Self {
        self.items.push(MermaidItem::Text {
            x,
            y,
            size,
            text: text.into(),
            color,
        });
        self
    }

    /// 等比缩放到目标框内（允许放大；至少保留 0.01 比例，返回宽高像素）。
    pub fn fit(&self, max_w: f32, max_h: f32) -> (f32, f32) {
        let w = self.width.max(1) as f32;
        let h = self.height.max(1) as f32;
        let s = (max_w / w).min(max_h / h).max(0.01);
        ((w * s).round().max(1.0), (h * s).round().max(1.0))
    }
}

/// 渲染结果：场景为跨端统一真值；`svg` 为后端自带 SVG（HTML 优先使用）。
#[derive(Clone, Debug, PartialEq)]
pub struct MermaidDrawing {
    pub scene: MermaidScene,
    pub svg: Option<String>,
}

impl MermaidDrawing {
    pub fn scene(scene: MermaidScene) -> Self {
        Self { scene, svg: None }
    }

    pub fn with_svg(scene: MermaidScene, svg: impl Into<String>) -> Self {
        Self {
            scene,
            svg: Some(svg.into()),
        }
    }
}

/// 渲染错误：消息直接展示给用户（预览面板）并回退到源码卡片。
#[derive(Clone, Debug, PartialEq)]
pub struct MermaidError {
    pub message: String,
}

impl MermaidError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for MermaidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 后端中立渲染抽象：实现方只依赖本文件的数据类型，不接触 UI/导出主流程。
pub trait MermaidRenderer {
    /// 把 Mermaid 源码渲染为矢量结果。返回 `Err` 时由界面展示错误并回退源码卡片。
    fn render(&self, source: &str, theme: &MermaidTheme)
        -> Result<MermaidDrawing, MermaidError>;
}

/// 渲染器注册表：`None` = 未启用，全部走既有降级。
#[derive(Default)]
pub struct MermaidRenderers {
    renderer: Option<Box<dyn MermaidRenderer>>,
}

impl MermaidRenderers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(&mut self, renderer: Box<dyn MermaidRenderer>) {
        self.renderer = Some(renderer);
    }

    pub fn clear(&mut self) {
        self.renderer = None;
    }

    pub fn is_enabled(&self) -> bool {
        self.renderer.is_some()
    }

    /// 当前渲染器（HTML/PDF 导出挂点需要借用它；未启用返回 None）。
    pub fn renderer(&self) -> Option<&dyn MermaidRenderer> {
        self.renderer.as_deref()
    }

    /// 未启用返回 `None`；启用后把结果与错误原样交给调用方。
    pub fn render(
        &self,
        source: &str,
        theme: &MermaidTheme,
    ) -> Option<Result<MermaidDrawing, MermaidError>> {
        self.renderer.as_ref().map(|r| r.render(source, theme))
    }
}

/// Mermaid 源码是否为空块（空行或纯 `%%` 注释）。
/// 与卡片头「空块提示」同一口径：空块不渲染、只给模板入口。
pub fn is_empty_source(source: &str) -> bool {
    !source
        .lines()
        .map(str::trim)
        .any(|l| !l.is_empty() && !l.starts_with("%%"))
}

/// 场景 → SVG（后端未提供 SVG 时由 HTML 导出派生；所有文本经 XML 转义）。
pub fn scene_to_svg(scene: &MermaidScene) -> String {
    let (w, h) = (scene.width.max(1), scene.height.max(1));
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">"
    );
    for item in &scene.items {
        match item {
            MermaidItem::Rect { x, y, w, h, fill } => {
                if fill[3] == 0 {
                    continue;
                }
                out.push_str(&format!(
                    "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" fill=\"#{:02x}{:02x}{:02x}\" fill-opacity=\"{:.2}\"/>",
                    fill[0],
                    fill[1],
                    fill[2],
                    fill[3] as f32 / 255.0
                ));
            }
            MermaidItem::Line {
                x1,
                y1,
                x2,
                y2,
                width,
                color,
            } => {
                if color[3] == 0 {
                    continue;
                }
                out.push_str(&format!(
                    "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" stroke=\"#{:02x}{:02x}{:02x}\" stroke-opacity=\"{:.2}\" stroke-width=\"{width:.1}\"/>",
                    color[0],
                    color[1],
                    color[2],
                    color[3] as f32 / 255.0
                ));
            }
            MermaidItem::Text {
                x,
                y,
                size,
                text,
                color,
            } => {
                out.push_str(&format!(
                    "<text x=\"{x:.1}\" y=\"{:.1}\" font-size=\"{size:.1}\" fill=\"#{:02x}{:02x}{:02x}\">{}</text>",
                    y + size,
                    color[0],
                    color[1],
                    color[2],
                    xml_escape(text)
                ));
            }
        }
    }
    out.push_str("</svg>");
    out
}

/// 清理后端 SVG：只保留 `<svg>` 结构，剥除脚本/外部对象/事件属性与可执行 URL。
/// 返回 None 表示不是可用的 SVG（调用方回退到场景派生或源码卡片）。
pub fn sanitize_svg(svg: &str) -> Option<String> {
    let trimmed = svg.trim();
    if !trimmed.to_ascii_lowercase().starts_with("<svg") {
        return None;
    }
    let mut out = trimmed.to_string();
    for tag in ["script", "foreignobject", "iframe", "object", "embed"] {
        out = strip_element(&out, tag);
    }
    out = strip_event_attrs(&out);
    out = strip_js_urls(&out);
    out.trim_start()
        .to_ascii_lowercase()
        .starts_with("<svg")
        .then_some(out)
}

/// 删除 `<tag ...>…</tag>`（大小写不敏感）；未闭合时删除到末尾。
fn strip_element(src: &str, tag: &str) -> String {
    let lower = src.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}");
    let mut out = String::with_capacity(src.len());
    let mut from = 0usize;
    while let Some(i) = lower[from..].find(&open) {
        let start = from + i;
        // 标签名边界：`<scriptx` 不算。
        let after = lower[start + open.len()..].chars().next();
        if after.is_some_and(|c| c.is_ascii_alphanumeric()) {
            out.push_str(&src[from..start + open.len()]);
            from = start + open.len();
            continue;
        }
        out.push_str(&src[from..start]);
        match lower[start..].find(&close) {
            Some(c) => {
                let end = start + c;
                let end = lower[end..]
                    .find('>')
                    .map_or(lower.len(), |p| end + p + 1);
                from = end;
            }
            None => return out,
        }
    }
    out.push_str(&src[from..]);
    out
}

/// 删除事件属性（`on*="…"` / `on*='…'`）与危险协议属性值。
/// 只按 ASCII 字节定位属性名，输出始终按 UTF-8 切片拷贝（不破坏非 ASCII 文本）。
fn strip_event_attrs(src: &str) -> String {
    let b = src.as_bytes();
    let lower = src.to_ascii_lowercase();
    let mut out = String::with_capacity(src.len());
    let mut copy_from = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        // 属性名起点：前一个字符是空白。
        if b[i].is_ascii_whitespace() {
            let name_start = i + 1;
            let mut j = name_start;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'-' | b':' | b'_'))
            {
                j += 1;
            }
            let name = &lower[name_start..j];
            let mut k = j;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < b.len() && b[k] == b'=' && !name.is_empty() {
                let mut v = k + 1;
                while v < b.len() && b[v].is_ascii_whitespace() {
                    v += 1;
                }
                let (value, end) = if v < b.len() && (b[v] == b'"' || b[v] == b'\'') {
                    let q = b[v];
                    let close = src[v + 1..].find(q as char).map_or(b.len(), |p| v + 1 + p);
                    (lower[v + 1..close].trim(), (close + 1).min(b.len()))
                } else {
                    let e = (v..b.len())
                        .find(|&x| b[x].is_ascii_whitespace() || b[x] == b'>')
                        .unwrap_or(b.len());
                    (lower[v..e].trim(), e)
                };
                let dangerous = name.starts_with("on")
                    || ((name == "href" || name == "xlink:href" || name == "src")
                        && (value.starts_with("javascript:")
                            || value.starts_with("data:text/html")
                            || value.starts_with("vbscript:")));
                if dangerous {
                    out.push_str(&src[copy_from..i]);
                    copy_from = end;
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
    out.push_str(&src[copy_from..]);
    out
}

/// 删除 URL 型属性值里残留的 `javascript:`/`vbscript:`（覆盖非属性上下文）。
fn strip_js_urls(src: &str) -> String {
    let lower = src.to_ascii_lowercase();
    if !lower.contains("javascript:") && !lower.contains("vbscript:") {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    while i < src.len() {
        let rest = &lower[i..];
        let hit = ["javascript:", "vbscript:"]
            .iter()
            .filter_map(|p| rest.find(p))
            .min();
        match hit {
            Some(p) => {
                out.push_str(&src[i..i + p]);
                i += p;
                // 跳到属性值结束（引号/空白/`>`）。
                while i < src.len() {
                    let c = src.as_bytes()[i];
                    if c.is_ascii_whitespace() || c == b'"' || c == b'\'' || c == b'>' {
                        break;
                    }
                    i += 1;
                }
            }
            None => {
                out.push_str(&src[i..]);
                break;
            }
        }
    }
    out
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// 把场景画到窗口帧：矩形/线段用 Frame 原语，文本走 UI 标签位图缓存。
/// `origin` 为场景左上角屏幕坐标；场景坐标乘 `scale` 映射到物理像素。
#[allow(clippy::too_many_arguments)] // 绘制入口参数天然多（与 paint_under_decor 同）
pub(crate) fn paint_scene(
    frame: &mut Frame,
    labels: &mut crate::chrome::LabelCache,
    engine: &mut LayoutEngine,
    scene: &MermaidScene,
    origin: (f32, f32),
    scale: f32,
    ui_scale: f32,
) {
    for item in &scene.items {
        match item {
            MermaidItem::Rect { x, y, w, h, fill } => {
                if fill[3] == 0 {
                    continue;
                }
                frame.fill_rect(
                    (origin.0 + x * scale).round() as i32,
                    (origin.1 + y * scale).round() as i32,
                    (w * scale).max(1.0) as u32,
                    (h * scale).max(1.0) as u32,
                    *fill,
                );
            }
            MermaidItem::Line {
                x1,
                y1,
                x2,
                y2,
                width,
                color,
            } => {
                if color[3] == 0 {
                    continue;
                }
                draw_scene_line(
                    frame,
                    origin.0 + x1 * scale,
                    origin.1 + y1 * scale,
                    origin.0 + x2 * scale,
                    origin.1 + y2 * scale,
                    (width * scale).max(1.0),
                    *color,
                );
            }
            MermaidItem::Text {
                x,
                y,
                text,
                color,
                ..
            } => {
                if text.is_empty() {
                    continue;
                }
                if let Some(lab) = labels.get_or_insert(engine, text, *color, ui_scale) {
                    frame.blit(
                        &lab.pixels,
                        lab.w,
                        lab.h,
                        (origin.0 + x * scale).round() as i32,
                        (origin.1 + y * scale).round() as i32,
                    );
                }
            }
        }
    }
}

/// 直线像素化：沿主轴逐点落 1×1 方块（场景以横竖线为主，等价于细矩形）。
fn draw_scene_line(
    frame: &mut Frame,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    width: f32,
    color: [u8; 4],
) {
    let steps = (x2 - x1).abs().max((y2 - y1).abs()).round().max(1.0) as i32;
    let w = width.round().max(1.0) as u32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = x1 + (x2 - x1) * t;
        let y = y1 + (y2 - y1) * t;
        frame.fill_rect(x.round() as i32, y.round() as i32, w, w, color);
    }
}

/// 渲染器返回的原始 SVG 供 HTML 导出使用：优先用后端 SVG，其次从场景派生。
pub fn svg_of(drawing: &MermaidDrawing) -> Option<String> {
    drawing
        .svg
        .as_deref()
        .and_then(sanitize_svg)
        .or_else(|| Some(scene_to_svg(&drawing.scene)))
}

/// 预览面板：压暗背景 + 居中矢量图 + 标题/关闭提示；失败时居中显示错误消息。
/// 纯视图态：不写文档、不动光标、不进撤销栈。
pub(crate) fn paint_preview(app: &mut crate::app::App, frame: &mut Frame) {
    let (win_w, win_h) = app.win_size();
    let scale = app.input_state.scale;
    frame.fill_rect(0, 0, win_w, win_h, [0, 0, 0, 220]);
    let Some(pv) = app.ui_state.mermaid_preview.as_ref() else {
        return;
    };
    let (title, drawing, error) = (pv.title.clone(), pv.drawing.clone(), pv.error.clone());
    let pad = (40.0 * scale).max(8.0);
    let title = crate::i18n::tf(crate::i18n::Key::MermaidPreviewTitle, &[&title]);
    let hint = crate::i18n::t(crate::i18n::Key::MermaidPreviewHint).to_string();
    if let Some(lab) = app
        .ui_state
        .chrome
        .labels
        .get_or_insert(&mut app.engine, &title, [240, 240, 240], scale)
    {
        let x = ((win_w as f32 - lab.w as f32) * 0.5).max(0.0) as i32;
        frame.blit(&lab.pixels, lab.w, lab.h, x, (pad * 0.4) as i32);
    }
    match (drawing, error) {
        (Some(drawing), _) => {
            let scene = &drawing.scene;
            let (dw, dh) = scene.fit(win_w as f32 - 2.0 * pad, win_h as f32 - 3.0 * pad);
            let x = (win_w as f32 - dw) * 0.5;
            let y = (win_h as f32 - dh) * 0.5;
            frame.fill_rect(
                x as i32 - 3,
                y as i32 - 3,
                dw as u32 + 6,
                dh as u32 + 6,
                [0, 0, 0, 160],
            );
            let s = dw / scene.width.max(1) as f32;
            paint_scene(
                frame,
                &mut app.ui_state.chrome.labels,
                &mut app.engine,
                scene,
                (x, y),
                s,
                scale,
            );
        }
        (None, Some(err)) => {
            let msg = crate::i18n::tf(crate::i18n::Key::MermaidPreviewError, &[&err]);
            if let Some(lab) = app.ui_state.chrome.labels.get_or_insert(
                &mut app.engine,
                &msg,
                [235, 120, 120],
                scale,
            ) {
                let x = ((win_w as f32 - lab.w as f32) * 0.5).max(0.0) as i32;
                let y = ((win_h as f32 - lab.h as f32) * 0.5).max(0.0) as i32;
                frame.blit(&lab.pixels, lab.w, lab.h, x, y);
            }
        }
        (None, None) => {}
    }
    if let Some(lab) = app
        .ui_state
        .chrome
        .labels
        .get_or_insert(&mut app.engine, &hint, [170, 170, 170], scale)
    {
        let x = ((win_w as f32 - lab.w as f32) * 0.5).max(0.0) as i32;
        let y = (win_h as f32 - pad * 0.4 - lab.h as f32).round() as i32;
        frame.blit(&lab.pixels, lab.w, lab.h, x, y);
    }
}

/// 预览面板状态：成功时持有矢量结果，失败时持有可见错误消息。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MermaidPreview {
    pub(crate) title: String,
    pub(crate) drawing: Option<MermaidDrawing>,
    pub(crate) error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_scene() -> MermaidScene {
        MermaidScene::new(120, 60)
            .rect(0.0, 0.0, 120.0, 60.0, [240, 240, 240, 255])
            .line(10.0, 10.0, 110.0, 10.0, 2.0, [0, 0, 0, 255])
            .text(12.0, 20.0, 10.0, "A <&> B", [20, 20, 20])
    }

    #[test]
    fn scene_svg_escapes_text_and_keeps_primitives() {
        let svg = scene_to_svg(&sample_scene());
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains("<rect"));
        assert!(svg.contains("<line"));
        assert!(svg.contains("A &lt;&amp;&gt; B"), "文本应转义: {svg}");
        assert!(svg.ends_with("</svg>"));
    }

    #[test]
    fn sanitize_svg_strips_scripts_and_event_handlers() {
        let raw = r##"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script><rect width="10" height="10" onclick="evil()" fill="#000"/><a href="javascript:alert(1)">x</a></svg>"##;
        let safe = sanitize_svg(raw).expect("应保留 svg 外壳");
        assert!(!safe.contains("script"), "脚本应被剥除: {safe}");
        assert!(!safe.contains("onclick"), "事件属性应被剥除: {safe}");
        assert!(!safe.to_ascii_lowercase().contains("javascript:"), "{safe}");
        assert!(safe.contains("<rect"), "正常图元应保留: {safe}");
    }

    #[test]
    fn sanitize_svg_rejects_non_svg() {
        assert!(sanitize_svg("not an svg").is_none());
        assert!(sanitize_svg("<div>hi</div>").is_none());
    }

    #[test]
    fn empty_source_matches_comment_only_blocks() {
        assert!(is_empty_source(""));
        assert!(is_empty_source("  \n%% 注释\n\t%% 再一行\n"));
        assert!(!is_empty_source("%% c\ngraph TD\nA-->B\n"));
    }

    #[test]
    fn registry_disabled_returns_none_and_enabled_returns_drawing() {
        struct Fake;
        impl MermaidRenderer for Fake {
            fn render(
                &self,
                _source: &str,
                _theme: &MermaidTheme,
            ) -> Result<MermaidDrawing, MermaidError> {
                Ok(MermaidDrawing::scene(sample_scene()))
            }
        }
        let mut r = MermaidRenderers::new();
        assert!(!r.is_enabled());
        assert!(r.render("graph TD", &MermaidTheme::from_export(&ExportStyle::default()))
            .is_none());
        r.install(Box::new(Fake));
        assert!(r.is_enabled());
        let got = r
            .render(
                "graph TD",
                &MermaidTheme::from_export(&ExportStyle::default()),
            )
            .expect("已安装应有结果")
            .expect("假渲染器应成功");
        assert_eq!(got.scene.width, 120);
        r.clear();
        assert!(!r.is_enabled());
    }

    #[test]
    fn svg_of_prefers_backend_svg_and_falls_back_to_scene() {
        let scene = sample_scene();
        let with_svg = MermaidDrawing::with_svg(
            scene.clone(),
            r#"<svg xmlns="http://www.w3.org/2000/svg"><script>x</script><rect width="1" height="1"/></svg>"#,
        );
        let svg = svg_of(&with_svg).unwrap();
        assert!(svg.contains("<rect"));
        assert!(!svg.contains("script"));
        let scene_only = MermaidDrawing::scene(scene);
        assert!(svg_of(&scene_only).unwrap().contains("<rect"));
    }
}
