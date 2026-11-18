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

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Instant;

use veil_core::BlockId;

use crate::geometry::{BASE_FONT_SIZE, BASE_LINE_HEIGHT};
use crate::theme::{ExportStyle, Theme};
use crate::view::Frame;

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
    /// 正文无衬线字体族回退链：后端把它注入度量与 SVG `font-family`，
    /// 保证图内文字与正文字体栈一致（`typography::SANS_FAMILIES`）。
    pub families: &'static [&'static str],
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
            families: crate::tokens::typography::SANS_FAMILIES,
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
            families: crate::tokens::typography::SANS_FAMILIES,
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
/// 继承 [`MermaidBitmapRenderer`]（自动获得 Send + Sync 上界）：屏幕位图 blit、
/// 后台渲染 worker 与 HTML/PDF 导出共用同一实现对象。
pub trait MermaidRenderer: MermaidBitmapRenderer {
    /// 把 Mermaid 源码渲染为矢量结果。返回 `Err` 时由界面展示错误并回退源码卡片。
    fn render(&self, source: &str, theme: &MermaidTheme)
        -> Result<MermaidDrawing, MermaidError>;
}

/// 位图渲染结果：不透明 RGBA（每像素 4 字节，行主序）+ 物理像素尺寸。
/// `rgba` 用 `Arc` 包裹以便缓存层零拷贝命中（滚动重绘直绘位图）。
#[derive(Clone)]
pub struct MermaidBitmap {
    pub rgba: std::sync::Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for MermaidBitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 不打印像素数据：缓冲可达数十 MB，只输出尺寸与容量。
        f.debug_struct("MermaidBitmap")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("len", &self.rgba.len())
            .finish()
    }
}

/// 位图渲染抽象：屏幕 blit 路径用（与 [`MermaidRenderer`] 并列，后端中立）。
/// 实现方应缓存 SVG 与位图（按 `scale` 重栅格化），同一输入重复调用不得重布局。
pub trait MermaidBitmapRenderer: Send + Sync {
    /// `scale` 为栅格化倍率（设备 DPI）；`theme` 建议传逻辑度量快照
    /// （如 `MermaidTheme::from_theme(&theme, 1.0)`），避免字号与倍率双重缩放。
    fn render_bitmap(
        &self,
        source: &str,
        theme: &MermaidTheme,
        scale: f32,
    ) -> Result<MermaidBitmap, MermaidError>;
}

/// 渲染器注册表：`None` = 未启用，全部走既有降级。
/// `Arc` 持有：后台渲染 worker 需要 `'static` 的后端引用（请求自带克隆）。
#[derive(Default)]
pub struct MermaidRenderers {
    renderer: Option<std::sync::Arc<dyn MermaidRenderer>>,
}

impl MermaidRenderers {
    pub fn new() -> Self {
        Self::default()
    }

    /// 安装渲染器（获得所有权，内部转 `Arc` 供 worker 线程共享）。
    pub fn install(&mut self, renderer: Box<dyn MermaidRenderer>) {
        self.renderer = Some(std::sync::Arc::from(renderer));
    }

    /// 安装共享渲染器（产品层已持有 `Arc`，如 `QuickJSRenderer::new()` 的返回值）。
    pub fn install_shared(&mut self, renderer: std::sync::Arc<dyn MermaidRenderer>) {
        self.renderer = Some(renderer);
    }

    pub fn clear(&mut self) {
        self.renderer = None;
    }

    pub fn is_enabled(&self) -> bool {
        self.renderer.is_some()
    }

    /// 共享克隆：后台渲染 worker 的请求自带 `'static` 后端引用，跨线程执行。
    pub fn arc(&self) -> Option<std::sync::Arc<dyn MermaidRenderer>> {
        self.renderer.clone()
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

/// Tiny mermaid bundle 支持的图型（KEEP 集）：flowchart / sequence / pie。
/// 其余图型（class/state/er/gantt…）不提交渲染，保持源码卡片（与未安装渲染器一致，
/// 避免产出无图形的空结果）。
pub fn mermaid_supported_source(source: &str) -> bool {
    matches!(
        veil_core::detect_type(source),
        Some("flowchart" | "sequence" | "pie")
    )
}

/// 引擎级错误前缀白名单（与 veil-mermaid worker 的错误消息口径一致）：
/// 命中 = 引擎/基础设施问题（超时、引擎不可用、输出无效、worker 断线等）；
/// 其余消息（QuickJS 抛出的 JS 解析异常等）是用户源码问题。
/// 两者都按块记忆不重试；目前仅用于日志与将来错误卡分级。
pub fn is_engine_failure(message: &str) -> bool {
    const ENGINE_PREFIXES: [&str; 9] = [
        "渲染超时",
        "渲染引擎",
        "Mermaid 渲染输出无效",
        "Mermaid 源码为空",
        "读取渲染结果",
        "渲染未产出",
        "QuickJS",
        "DOM shim",
        "bundle",
    ];
    ENGINE_PREFIXES.iter().any(|p| message.starts_with(p))
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

// =============================================================================
// 后台渲染调度（渲染不占 UI 帧：防抖合并提交，单 worker 串行执行，tick 收割）
// =============================================================================

/// 防抖窗口：击键 / 滚动 / resize 引发的重复提交在此窗口内合并为一次后台渲染。
const JOB_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(280);
/// 在途结果收割间隔：worker 无法主动唤醒事件循环，按此节奏 try_recv。
const JOB_POLL: std::time::Duration = std::time::Duration::from_millis(30);

/// 一次后台渲染请求：数据自持（renderer 克隆 + 快照），worker 线程零借用。
pub(crate) struct MermaidJobRequest {
    /// 提交时的渲染后端快照：install/clear 换代后旧请求照旧执行，结果按 epoch 丢弃。
    pub(crate) renderer: std::sync::Arc<dyn MermaidRenderer>,
    /// 后端代次：与请求同批提交，回包时与 App 当前值比对。
    pub(crate) epoch: u32,
    pub(crate) kind: MermaidJobKind,
}

/// 请求类别：L1 内嵌块（结果进位图缓存）与预览面板（结果回填单槽）。
pub(crate) enum MermaidJobKind {
    Block {
        id: BlockId,
        source: std::sync::Arc<str>,
        /// 屏幕 1x 主题快照（字号不随 DPI，缩放全部由 fit 倍率承担，避免双重缩放）。
        theme: MermaidTheme,
        /// 目标物理像素框（版心宽 × 图片限高的提交时快照）。
        fit_w: f32,
        fit_h: f32,
        theme_rev: u32,
        session_rev: u32,
    },
    Preview {
        title: String,
        source: std::sync::Arc<str>,
        theme: MermaidTheme,
        fit_w: f32,
        fit_h: f32,
    },
}

/// 块渲染成功产物：位图 + 栅格化倍率 + 1x 场景尺寸（即位图缓存条目的全部原料）。
pub(crate) struct MermaidBlockBitmap {
    pub(crate) bitmap: MermaidBitmap,
    /// 生成位图时的 fit 倍率（1x 场景尺寸 → 物理像素；resize 后据此判断是否重栅格化）。
    pub(crate) k: f32,
    pub(crate) scene_w: u32,
    pub(crate) scene_h: u32,
}

impl MermaidBlockBitmap {
    /// 当前目标框下的 fit 倍率（与 [`MermaidScene::fit`] 同口径）。
    pub(crate) fn fit_scale(&self, max_w: f32, max_h: f32) -> f32 {
        let w = self.scene_w.max(1) as f32;
        let h = self.scene_h.max(1) as f32;
        (max_w / w).min(max_h / h).max(0.01)
    }

    /// 当前目标框下的显示尺寸（物理像素）。
    pub(crate) fn display_size(&self, max_w: f32, max_h: f32) -> (f32, f32) {
        let s = self.fit_scale(max_w, max_h);
        let w = self.scene_w.max(1) as f32;
        let h = self.scene_h.max(1) as f32;
        ((w * s).round().max(1.0), (h * s).round().max(1.0))
    }

    /// 显示尺寸的量化倍率（×100 取整）：与条目 `k` 比对判断是否需要重栅格化。
    pub(crate) fn scale_bucket(k: f32) -> u32 {
        (k * 100.0).round().max(0.0) as u32
    }
}

/// worker 回包：块渲染 = [`MermaidBlockBitmap`]；预览 = 位图。
pub(crate) enum MermaidJobOutcome {
    Block {
        epoch: u32,
        id: BlockId,
        theme_rev: u32,
        session_rev: u32,
        result: Result<MermaidBlockBitmap, MermaidError>,
    },
    Preview {
        title: String,
        result: Result<MermaidBitmap, MermaidError>,
    },
}

/// 渲染管线（worker 线程执行）：`render` 得 1x 场景 → fit 出倍率 → 位图重栅格化。
/// 两步共用后端 SVG 缓存：同一输入只做一次 JS 布局（Task 3.2「缓存命中直绘」）。
fn render_job(
    renderer: &dyn MermaidRenderer,
    source: &str,
    theme: &MermaidTheme,
    fit_w: f32,
    fit_h: f32,
) -> Result<MermaidBlockBitmap, MermaidError> {
    let drawing = renderer.render(source, theme)?;
    let (dw, _dh) = drawing.scene.fit(fit_w, fit_h);
    let k = dw / drawing.scene.width.max(1) as f32;
    let bitmap = renderer.render_bitmap(source, theme, k)?;
    Ok(MermaidBlockBitmap {
        bitmap,
        k,
        scene_w: drawing.scene.width,
        scene_h: drawing.scene.height,
    })
}

/// 单 worker 线程主循环：串行消费请求（QuickJS 引擎生命周期钉在单线程内，串行即互斥）。
/// 请求通道关闭（App 已丢弃）时退出线程。
fn dispatch_loop(
    rx: mpsc::Receiver<MermaidJobRequest>,
    tx: mpsc::Sender<MermaidJobOutcome>,
) {
    while let Ok(req) = rx.recv() {
        let outcome = match req.kind {
            MermaidJobKind::Block {
                id,
                source,
                theme,
                fit_w,
                fit_h,
                theme_rev,
                session_rev,
            } => MermaidJobOutcome::Block {
                epoch: req.epoch,
                id,
                theme_rev,
                session_rev,
                result: render_job(&*req.renderer, &source, &theme, fit_w, fit_h),
            },
            MermaidJobKind::Preview {
                title,
                source,
                theme,
                fit_w,
                fit_h,
            } => MermaidJobOutcome::Preview {
                title,
                result: render_job(&*req.renderer, &source, &theme, fit_w, fit_h)
                    .map(|b| b.bitmap),
            },
        };
        if tx.send(outcome).is_err() {
            break; // 回包通道已断（App 已丢弃）：退出线程
        }
    }
}

/// 后台渲染调度器：App 主线程提交（防抖合并），单 worker 线程执行，tick 收割结果。
/// 全部字段 owned（mpsc + Arc + 快照），随 App 一起只活在主线程；worker 只触碰请求。
pub(crate) struct MermaidJobs {
    pending: HashMap<BlockId, MermaidJobRequest>,
    /// 预览单槽：同一时刻只渲染最后一次请求的面板。
    preview: Option<MermaidJobRequest>,
    /// 防抖到期时刻：最后一次提交后 `JOB_DEBOUNCE` 统一投递。
    debounce_at: Option<Instant>,
    /// 在途结果收割点：有任务在途时按 `JOB_POLL` 轮询，收齐即撤。
    poll_at: Option<Instant>,
    /// 已投递未收割的请求数。
    in_flight: u32,
    /// 后端代次：install/clear 时递增，回包按此丢弃旧后端结果。
    epoch: u32,
    /// 后端是否已拉起（懒启动：首个投递才创建线程）。
    spawned: bool,
    /// 请求通道发送端（App → worker）。
    tx: Option<mpsc::Sender<MermaidJobRequest>>,
    /// 请求通道接收端：随线程移入 dispatch_loop。
    req_rx: Option<mpsc::Receiver<MermaidJobRequest>>,
    /// 回包通道发送端（worker → App）：随线程移入 dispatch_loop。
    out_tx: Option<mpsc::Sender<MermaidJobOutcome>>,
    /// 回包通道接收端（App 收割结果）。
    rx: Option<mpsc::Receiver<MermaidJobOutcome>>,
}

impl MermaidJobs {
    pub(crate) fn new() -> Self {
        let (tx, req_rx) = mpsc::channel::<MermaidJobRequest>();
        let (out_tx, rx) = mpsc::channel::<MermaidJobOutcome>();
        Self {
            pending: HashMap::new(),
            preview: None,
            debounce_at: None,
            poll_at: None,
            in_flight: 0,
            epoch: 0,
            spawned: false,
            tx: Some(tx),
            req_rx: Some(req_rx),
            out_tx: Some(out_tx),
            rx: Some(rx),
        }
    }

    /// 提交块渲染请求：同块覆盖旧快照（只渲染最新内容），并刷新防抖窗口。
    pub(crate) fn submit_block(&mut self, req: MermaidJobRequest) {
        let MermaidJobKind::Block { id, .. } = &req.kind else {
            return;
        };
        self.pending.insert(*id, req);
        self.debounce_at = Some(Instant::now() + JOB_DEBOUNCE);
    }

    /// 提交预览请求（覆盖旧请求：面板只展示最后一次点击的图）。
    pub(crate) fn submit_preview(&mut self, req: MermaidJobRequest) {
        if !matches!(req.kind, MermaidJobKind::Preview { .. }) {
            return;
        }
        self.preview = Some(req);
        self.debounce_at = Some(Instant::now() + JOB_DEBOUNCE);
    }

    /// 某块是否已有待投递请求（防重复提交）。
    pub(crate) fn is_pending(&self, id: BlockId) -> bool {
        self.pending.contains_key(&id)
    }

    /// 当前后端代次（提交时随请求盖章）。
    pub(crate) fn epoch(&self) -> u32 {
        self.epoch
    }

    /// 后端换代（install/clear）：丢弃待投递请求并递增代次，旧后端在途回包按代次丢弃。
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.preview = None;
        self.debounce_at = None;
        self.epoch += 1;
    }

    /// 下一次需要唤醒事件循环的时刻（防抖到期 / 在途收割，取较早者）。
    pub(crate) fn wake_at(&self) -> Option<Instant> {
        match (self.debounce_at, self.poll_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// 防抖窗口是否已到（`now` 来自 tick 注入的时钟）。
    pub(crate) fn debounce_due(&self, now: Instant) -> bool {
        self.debounce_at.is_some_and(|t| now >= t)
    }

    /// 投递全部待渲染请求：拉起（或复用）worker 线程，登记收割点。
    pub(crate) fn flush(&mut self, now: Instant) {
        self.debounce_at = None;
        let preview = self.preview.take();
        let batch: Vec<MermaidJobRequest> = self.pending.drain().map(|(_, r)| r).collect();
        if batch.is_empty() && preview.is_none() {
            return;
        }
        // 懒启动：首次投递才拉起 worker（无 mermaid 块的会话零线程开销）。
        if !self.spawned {
            self.spawned = true;
            let (Some(req_rx), Some(out_tx)) = (self.req_rx.take(), self.out_tx.take()) else {
                return;
            };
            // 线程启动失败：请求就地丢弃（退回源码卡片降级），不 panic。
            let _ = std::thread::Builder::new()
                .name("mermaid-render".into())
                .spawn(move || dispatch_loop(req_rx, out_tx));
        }
        let mut sent = 0u32;
        for req in batch {
            if self.send(req) {
                sent += 1;
            }
        }
        if let Some(p) = preview {
            if self.send(p) {
                sent += 1;
            }
        }
        self.in_flight += sent;
        if sent > 0 {
            self.poll_at = Some(now + JOB_POLL);
        }
    }

    fn send(&mut self, req: MermaidJobRequest) -> bool {
        match &self.tx {
            Some(tx) => tx.send(req).is_ok(),
            None => false,
        }
    }

    /// 非阻塞收割全部在途结果（无结果返回空Vec；收齐后撤掉轮询点）。
    pub(crate) fn drain(&mut self) -> Vec<MermaidJobOutcome> {
        let mut out = Vec::new();
        if let Some(rx) = self.rx.as_ref() {
            while let Ok(o) = rx.try_recv() {
                self.in_flight = self.in_flight.saturating_sub(1);
                out.push(o);
            }
        }
        if self.in_flight == 0 {
            self.poll_at = None;
        }
        out
    }

    /// 全部任务已结清（无待投递 / 无在途）：测试泵循环的退出条件。
    #[cfg(test)]
    pub(crate) fn idle(&self) -> bool {
        self.pending.is_empty() && self.preview.is_none() && self.in_flight == 0
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

/// 预览面板：压暗背景 + 居中位图 + 标题/关闭提示；渲染中显示占位、失败时居中显示错误消息。
/// 纯视图态：不写文档、不动光标、不进撤销栈。
pub(crate) fn paint_preview(app: &mut crate::app::App, frame: &mut Frame) {
    let (win_w, win_h) = app.win_size();
    let scale = app.input_state.scale;
    frame.fill_rect(0, 0, win_w, win_h, [0, 0, 0, 220]);
    let Some(pv) = app.ui_state.mermaid_preview.as_ref() else {
        return;
    };
    let (title, bitmap, error, loading) =
        (pv.title.clone(), pv.bitmap.clone(), pv.error.clone(), pv.loading);
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
    match (bitmap, error, loading) {
        // 位图已按提交时的目标框栅格化：居中 blit；窗口变小后等比缩小即时重绘。
        (Some(bm), _, _) => {
            let avail_w = (win_w as f32 - 2.0 * pad).max(1.0);
            let avail_h = (win_h as f32 - 3.0 * pad).max(1.0);
            let s = (avail_w / bm.width as f32)
                .min(avail_h / bm.height as f32)
                .min(1.0);
            let dw = (bm.width as f32 * s).round().max(1.0) as u32;
            let dh = (bm.height as f32 * s).round().max(1.0) as u32;
            let x = ((win_w as f32 - dw as f32) * 0.5).round() as i32;
            let y = ((win_h as f32 - dh as f32) * 0.5).round() as i32;
            frame.fill_rect(x - 3, y - 3, dw + 6, dh + 6, [0, 0, 0, 160]);
            if dw == bm.width && dh == bm.height {
                frame.blit(&bm.rgba, bm.width, bm.height, x, y);
            } else {
                frame.blit_scaled(&bm.rgba, bm.width, bm.height, dw, dh, x, y);
            }
        }
        (None, Some(err), _) => {
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
        // 渲染中占位（后台 worker 执行期间面板不空白）。
        (None, None, true) => {
            let msg = crate::i18n::t(crate::i18n::Key::MermaidPreviewLoading).to_string();
            if let Some(lab) = app
                .ui_state
                .chrome
                .labels
                .get_or_insert(&mut app.engine, &msg, [170, 170, 170], scale)
            {
                let x = ((win_w as f32 - lab.w as f32) * 0.5).max(0.0) as i32;
                let y = ((win_h as f32 - lab.h as f32) * 0.5).max(0.0) as i32;
                frame.blit(&lab.pixels, lab.w, lab.h, x, y);
            }
        }
        (None, None, false) => {}
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

/// 预览面板状态：成功时持有位图（后台渲染完成），失败时持有可见错误消息，
/// 渲染中为 `loading`（占位文案）。
#[derive(Clone, Debug)]
pub(crate) struct MermaidPreview {
    pub(crate) title: String,
    pub(crate) bitmap: Option<MermaidBitmap>,
    pub(crate) error: Option<String>,
    /// true = 后台渲染尚未回包。
    pub(crate) loading: bool,
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
        impl MermaidBitmapRenderer for Fake {
            fn render_bitmap(
                &self,
                _source: &str,
                _theme: &MermaidTheme,
                _scale: f32,
            ) -> Result<MermaidBitmap, MermaidError> {
                Ok(MermaidBitmap {
                    rgba: std::sync::Arc::new(vec![0; 120 * 60 * 4]),
                    width: 120,
                    height: 60,
                })
            }
        }
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
        // arc() 克隆给 worker 线程：独立计数，原注册表仍可用。
        assert!(r.arc().is_some());
        assert!(r.renderer().is_some());
        r.clear();
        assert!(!r.is_enabled());
        assert!(r.arc().is_none());
    }

    #[test]
    fn supported_source_covers_tiny_bundle_keep_set_only() {
        assert!(mermaid_supported_source("flowchart TD\nA-->B"));
        assert!(mermaid_supported_source("graph LR\nA-->B"));
        assert!(mermaid_supported_source("%% 注释\nsequenceDiagram\nA->>B: 你好"));
        assert!(mermaid_supported_source("pie title 销量\n\"A\": 40"));
        // Tiny bundle 未打包的图型：不渲染（保持源码卡片）。
        assert!(!mermaid_supported_source("classDiagram\nA <|-- B"));
        assert!(!mermaid_supported_source("stateDiagram-v2\n[*] --> A"));
        assert!(!mermaid_supported_source("gantt\n无"));
        assert!(!mermaid_supported_source(""));
    }

    #[test]
    fn engine_failure_whitelist_matches_infra_only_messages() {
        assert!(is_engine_failure("渲染超时"));
        assert!(is_engine_failure("渲染引擎不可用"));
        assert!(is_engine_failure("渲染引擎初始化失败：boom"));
        assert!(is_engine_failure("Mermaid 渲染输出无效"));
        assert!(is_engine_failure("QuickJS 运行时错误：x"));
        assert!(is_engine_failure("bundle 加载失败"));
        // 用户源码问题（JS 解析异常原样字符串）不属于引擎失败。
        assert!(!is_engine_failure("Parse error on line 2: ..."));
        assert!(!is_engine_failure("Diagram 变量未定义"));
    }

    #[test]
    fn jobs_debounce_merges_and_clear_bumps_epoch() {
        let theme = MermaidTheme::from_export(&ExportStyle::default());
        let mut jobs = MermaidJobs::new();
        let req = |id: BlockId| MermaidJobRequest {
            renderer: request_recorder_renderer(),
            epoch: 0,
            kind: MermaidJobKind::Block {
                id,
                source: std::sync::Arc::from("flowchart TD\nA-->B"),
                theme,
                fit_w: 600.0,
                fit_h: 400.0,
                theme_rev: 0,
                session_rev: 0,
            },
        };
        let a_id = BlockId(1);
        let b_id = BlockId(2);
        assert!(jobs.idle());
        jobs.submit_block(req(a_id));
        assert!(jobs.is_pending(a_id));
        assert!(!jobs.is_pending(b_id));
        let first = jobs.wake_at().expect("提交后应有防抖期限");
        std::thread::sleep(std::time::Duration::from_millis(5));
        jobs.submit_block(req(a_id)); // 同块覆盖旧快照
        jobs.submit_block(req(b_id));
        assert!(
            jobs.wake_at().expect("覆盖提交应刷新期限") > first,
            "防抖窗口应被刷新"
        );
        assert_eq!(jobs.epoch(), 0);
        jobs.clear();
        assert!(!jobs.is_pending(a_id));
        assert!(!jobs.is_pending(b_id));
        assert_eq!(jobs.epoch(), 1, "换代应递增代次");
        assert!(jobs.wake_at().is_none(), "换代后不应再有防抖期限");
        assert!(jobs.idle());
    }

    /// 最小渲染器替身（调度器测试不真正执行渲染）。
    fn request_recorder_renderer() -> std::sync::Arc<dyn MermaidRenderer> {
        struct Fake;
        impl MermaidBitmapRenderer for Fake {
            fn render_bitmap(
                &self,
                _s: &str,
                _t: &MermaidTheme,
                _sc: f32,
            ) -> Result<MermaidBitmap, MermaidError> {
                Err(MermaidError::new("未执行"))
            }
        }
        impl MermaidRenderer for Fake {
            fn render(
                &self,
                _s: &str,
                _t: &MermaidTheme,
            ) -> Result<MermaidDrawing, MermaidError> {
                Err(MermaidError::new("未执行"))
            }
        }
        std::sync::Arc::new(Fake)
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
