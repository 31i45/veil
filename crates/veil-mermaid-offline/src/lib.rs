//! 离线 Mermaid 渲染核心：QuickJS 内运行完整 `mermaid.js`，配 linkedom 提供 headless DOM。
//!
//! 数据流：
//! ```text
//! MermaidEngine::submit(label, source, dark)
//!   → 渲染工作线程（单例 QuickJS Runtime，避免每次重建）
//!     → mermaid.initialize({theme, htmlLabels:false})
//!     → mermaid.render(id, source)  （异步，靠微任务泵推进）
//!     → 返回 SVG 文本
//! ```
//!
//! 为什么独立线程：QuickJS Runtime 不是 Send，且初始化（解析 ~3 MB JS）代价高；
//! 常驻一个工作线程既满足 trait 的非阻塞语义，又只付一次初始化成本。

#![allow(clippy::needless_return)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use rquickjs::{Context, Ctx, Function, Runtime};

#[doc(hidden)]
pub mod corpus;

/// 完整 mermaid UMD 产物（pin 版本见 assets/NOTICE.md）。
const MERMAID_JS: &str = include_str!("../assets/mermaid.min.js");
/// linkedom 构建期转换后的经典脚本。
const LINKEDOM_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/linkedom.js"));
/// headless DOM 引导（全局对象 + SVG 度量 + canvas 桩）。
const BOOTSTRAP_JS: &str = include_str!("../assets/bootstrap.js");

/// 单次渲染超时：正常图 < 1s；超过说明脚本卡死或死循环。
const RENDER_TIMEOUT: Duration = Duration::from_secs(20);
/// QuickJS 堆上限：完整 mermaid + 单个大型图绰绰有余，同时防止失控增长。
const MEMORY_LIMIT: usize = 512 * 1024 * 1024;

struct Job {
    label: String,
    source: String,
    dark: bool,
}

/// 渲染结果：(label, 成功为 SVG 文本 / 失败为可读原因)
pub type RenderReply = (String, Result<String, String>);

/// 离线 Mermaid 引擎句柄（可克隆给多处使用；内部只持有通道）。
pub struct MermaidEngine {
    jobs: Sender<Job>,
    results: Receiver<RenderReply>,
}

impl MermaidEngine {
    /// 启动渲染工作线程。初始化失败（QuickJS/脚本）以 Err 返回，调用方降级为源码块。
    pub fn new() -> Result<Self, String> {
        let (job_tx, job_rx) = channel::<Job>();
        let (res_tx, res_rx) = channel::<RenderReply>();
        std::thread::Builder::new()
            .name("veil-mermaid-offline".into())
            .spawn(move || worker(job_rx, res_tx))
            .map_err(|e| format!("spawn mermaid worker failed: {e}"))?;
        Ok(Self {
            jobs: job_tx,
            results: res_rx,
        })
    }

    /// 投递一次渲染（立即返回；结果经 `try_recv` 取回）。
    pub fn submit(&self, label: String, source: String, dark: bool) {
        let _ = self.jobs.send(Job {
            label,
            source,
            dark,
        });
    }

    /// 非阻塞取走全部已完成结果。
    pub fn try_recv(&self) -> Vec<RenderReply> {
        let mut out = Vec::new();
        while let Ok(item) = self.results.try_recv() {
            out.push(item);
        }
        out
    }
}

fn worker(jobs: Receiver<Job>, results: Sender<RenderReply>) {
    let mut session = Session::new();
    let init_error = session.as_ref().err().cloned();
    while let Ok(job) = jobs.recv() {
        let reply = match session.as_mut() {
            Ok(s) => s.render(&job.source, job.dark),
            Err(_) => Err(init_error.clone().unwrap_or_else(|| "init failed".into())),
        };
        if results.send((job.label, reply)).is_err() {
            break;
        }
    }
}

struct Session {
    /// 字段顺序即析构顺序：Context 必须先于 Runtime 释放。
    ctx: Context,
    /// 仅用于维持 Runtime 生命周期（Context 内部只持有裸指针）。
    #[allow(dead_code)]
    rt: Runtime,
    deadline: Rc<Cell<Option<Instant>>>,
    counter: u64,
}

impl Session {
    fn new() -> Result<Self, String> {
        let rt = Runtime::new().map_err(|e| format!("QuickJS runtime: {e}"))?;
        rt.set_memory_limit(MEMORY_LIMIT);
        let deadline = Rc::new(Cell::new(None));
        let interrupt_deadline = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            interrupt_deadline
                .get()
                .is_some_and(|deadline| Instant::now() >= deadline)
        })));
        // Chevrotain 的 parser 在 gitGraph / treemap 等图型中递归较深；
        // QuickJS 默认栈上限会在合法小图上提前溢出。
        rt.set_max_stack_size(4 * 1024 * 1024);
        let ctx = Context::full(&rt).map_err(|e| format!("QuickJS context: {e}"))?;

        // 文本度量桥：mermaid 的 getBBox 依赖真实字形宽度，否则节点/边尺寸塌缩。
        // FontSystem 只在本线程使用，用 Rc<RefCell> 共享给 JS 回调。
        let fonts = Rc::new(RefCell::new(FontSystem::new()));
        let fonts_cb = fonts.clone();

        ctx.with(|ctx| -> Result<(), String> {
            let measure = Function::new(
                ctx.clone(),
                move |text: String, _family: String, size: f64| -> f64 {
                    measure_text(&mut fonts_cb.borrow_mut(), &text, size as f32) as f64
                },
            )
            .map_err(|e| format!("bind measure: {e}"))?;
            ctx.globals()
                .set("__veilMeasureText", measure)
                .map_err(|e| format!("set measure: {e}"))?;

            run_script(&ctx, "linkedom", LINKEDOM_JS)?;
            run_script(&ctx, "bootstrap", BOOTSTRAP_JS)?;
            run_script(&ctx, "mermaid", MERMAID_JS)?;

            let probe: String = ctx
                .eval("typeof mermaid.render")
                .map_err(|e| format!("probe mermaid: {e}"))?;
            if probe != "function" {
                return Err("mermaid.render not available after load".into());
            }
            Ok(())
        })?;

        Ok(Self {
            ctx,
            rt,
            deadline,
            counter: 0,
        })
    }

    fn render(&mut self, source: &str, dark: bool) -> Result<String, String> {
        self.counter += 1;
        let id = format!("veil-mm-{}", self.counter);
        let theme = if dark { "dark" } else { "default" };
        // mindmap 根文字实际位于 .section-root>.label text，旧 .mindmap-node-label 选择器不会命中；
        // 只对根节点文字居中，避免改变其他节点、边标签或图型的锚点。
        // Mermaid 11.12 dark 的 mindmap cScaleLabel 为 lightgrey，细字抗锯齿后在近黑节点上偏暗；
        // 全部层级统一使用白色标签并加深根节点底色，选择器不命中浅主题或其他图型。
        // 浅色主题的分段底色均为浅色 pastel，而 Mermaid 部分 section 的 cScaleLabel 为白色：
        // kanban 列标题与 journey 任务标签因此不可读。这里统一把 section-0..11 的文字改深色；
        // 不覆盖 section-root（浅色 mindmap 根节点保持 Mermaid 默认白字）。
        // `&[aria-roledescription]` 限定图根，避免影响 light、其他图型及 active/done/crit 语义色。
        let theme_css = if dark {
            "themeCSS: '&[aria-roledescription=\"mindmap\"] .section-root>.label text{text-anchor:middle !important;} \
             .mindmap-node text{fill:#ffffff !important;} \
             .mindmap-node span{color:#ffffff !important;fill:#ffffff !important;} \
             .section-root rect,.section-root path,.section-root circle,.section-root polygon{fill:#242448 !important;} \
             &[aria-roledescription=\"mindmap\"] .section-edge-0{stroke:#b8c0cc !important;} \
             &[aria-roledescription=\"flowchart-v2\"] .edgeLabel rect{fill:#4b5563 !important;opacity:1 !important;} \
             &[aria-roledescription=\"flowchart-v2\"] .edgeLabel .label text{fill:#ffffff !important;} \
             &[aria-roledescription=\"gantt\"] .titleText{fill:#ffffff !important;} \
             &[aria-roledescription=\"gantt\"] .sectionTitle{fill:#ffffff !important;} \
             &[aria-roledescription=\"gantt\"] .task0,&[aria-roledescription=\"gantt\"] .task1, \
             &[aria-roledescription=\"gantt\"] .task2,&[aria-roledescription=\"gantt\"] .task3{fill:#8b949e !important;} \
             &[aria-roledescription=\"gantt\"] .taskText0,&[aria-roledescription=\"gantt\"] .taskText1, \
             &[aria-roledescription=\"gantt\"] .taskText2,&[aria-roledescription=\"gantt\"] .taskText3{fill:#111418 !important;} \
             &[aria-roledescription=\"gantt\"] .taskTextOutside0,&[aria-roledescription=\"gantt\"] .taskTextOutside1, \
             &[aria-roledescription=\"gantt\"] .taskTextOutside2,&[aria-roledescription=\"gantt\"] .taskTextOutside3{fill:#ffffff !important;} \
             &[aria-roledescription=\"gantt\"] .grid .tick{stroke:#8c8c8c !important;opacity:0.8 !important;} \
             &[aria-roledescription=\"gantt\"] .grid .tick text{fill:#ffffff !important;} \
             &[aria-roledescription=\"journey\"] text[fill=\"\"]{fill:#ffffff !important;} \
             &[aria-roledescription=\"c4\"] text[fill=\"#444444\"]{fill:#ffffff !important;} \
             &[aria-roledescription=\"c4\"] line[stroke=\"#444444\"]{stroke:#8b949e !important;} \
             &[aria-roledescription=\"radar\"] .radarTitle, \
             &[aria-roledescription=\"radar\"] .radarAxisLabel, \
             &[aria-roledescription=\"radar\"] .radarLegendText{fill:#ffffff !important;color:#ffffff !important;}', "
        } else {
            "themeCSS: '&[aria-roledescription=\"mindmap\"] .section-root>.label text{text-anchor:middle !important;} \
             .sections .cluster-label text,.sections .cluster-label tspan{fill:#333333 !important;} \
             .section-0 text,.section-1 text,.section-2 text,.section-3 text,.section-4 text, \
             .section-5 text,.section-6 text,.section-7 text,.section-8 text,.section-9 text, \
             .section-10 text,.section-11 text{fill:#333333 !important;} \
             text.task,text.task tspan,text.journey-section,text.journey-section tspan{fill:#333333 !important;}', "
        };
        // htmlLabels:false → 输出 <text> 而非 foreignObject：resvg 无法光栅化 foreignObject。
        let init = format!(
            "mermaid.initialize({{ startOnLoad: false, securityLevel: 'loose', \
             {theme_css}theme: '{theme}', htmlLabels: false, fontFamily: 'sans-serif', \
             flowchart: {{ htmlLabels: false }}, class: {{ htmlLabels: false }}, \
             journey: {{ htmlLabels: false, textPlacement: 'tspan' }}, state: {{ htmlLabels: false }}, \
             gantt: {{ fontSize: 14, sectionFontSize: 14 }} }}); 'ok'"
        );
        let call = format!(
            "globalThis.__veil_out = null; \
             globalThis.__veilErrorText = function(e) {{ \
               var message = String((e && e.message) || e); \
               var stack = (e && e.stack) ? String(e.stack) : ''; \
               return stack && stack !== message ? message + '\\n' + stack : message; \
             }}; \
             try {{ \
               mermaid.render({id}, {src}).then(\
                 function(r) {{ globalThis.__veil_out = {{ ok: globalThis.__veilFinalizeSvg(String(r.svg)) }}; }}, \
                 function(e) {{ globalThis.__veil_out = {{ err: globalThis.__veilErrorText(e) }}; }}\
               ); \
             }} catch(e) {{ \
               globalThis.__veil_out = {{ err: globalThis.__veilErrorText(e) }}; \
             }}; 'queued'",
            id = json_string(&id),
            src = json_string(source),
        );

        let started = Instant::now();
        self.deadline.set(Some(started + RENDER_TIMEOUT));
        let result = self.ctx.with(|ctx| -> Result<String, String> {
            run_script(&ctx, "init", &init)?;
            ctx.eval::<String, _>(call.as_str())
                .map_err(|e| format!("mermaid.render call: {}", error_text(&ctx, e)))?;

            let mut idle = 0u32;
            loop {
                let probe: String = ctx
                    .eval(
                        "(function(){ var o = globalThis.__veil_out; \
                         if (!o) return '__VEIL_PENDING__'; \
                         return (o.ok !== undefined) ? ('S' + o.ok) : ('E' + o.err); })()",
                    )
                    .map_err(|e| format!("probe result: {}", error_text(&ctx, e)))?;
                if probe == "__VEIL_PENDING__" {
                    if ctx.execute_pending_job() {
                        idle = 0;
                        continue;
                    }
                    // 微任务已空：推进被 shim 排队的定时器（若有）。
                    let flushed: i64 = ctx
                        .eval("globalThis.__veilFlushTimers()")
                        .map_err(|e| format!("flush timers: {}", error_text(&ctx, e)))?;
                    if flushed > 0 {
                        idle = 0;
                        continue;
                    }
                    idle += 1;
                    if idle > 3 {
                        return Err("render stalled: no pending microtasks".into());
                    }
                } else if let Some(svg) = probe.strip_prefix('S') {
                    // viewBox 必须由 Mermaid 在修正后的 DOM 测量上自行计算；
                    // 不在 Rust 侧根据部分坐标重写最终尺寸。
                    return Ok(svg.to_string());
                } else if let Some(err) = probe.strip_prefix('E') {
                    return Err(err.to_string());
                }
                if started.elapsed() > RENDER_TIMEOUT {
                    return Err(format!("timeout after {:?}", RENDER_TIMEOUT));
                }
            }
        });
        self.deadline.set(None);
        result
    }
}

fn run_script(ctx: &Ctx, name: &str, src: &str) -> Result<(), String> {
    ctx.eval::<(), _>(src)
        .map_err(|e| format!("{name} script: {}", error_text(ctx, e)))
}

/// rquickjs 的 Display 不含异常消息，必须用 `ctx.catch()` 取回可读文本。
fn error_text(ctx: &Ctx, err: rquickjs::Error) -> String {
    let exc = ctx.catch();
    let detail = exc
        .as_string()
        .and_then(|s| s.to_string().ok())
        .unwrap_or_else(|| format!("{exc:?}"));
    format!("{err}: {detail}")
}

/// 最小 JSON 字符串转义（只用于把 JS 源码/ID 安全嵌进脚本字面量）。
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 用 cosmic-text 度量一行文本宽度（与编辑器同一字体栈）。
fn measure_text(fonts: &mut FontSystem, text: &str, size: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let size = if size.is_finite() && size > 0.0 {
        size
    } else {
        16.0
    };
    let metrics = Metrics::new(size, size * 1.35);
    let mut buffer = Buffer::new(fonts, metrics);
    let attrs = Attrs::new().family(Family::SansSerif);
    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    buffer.set_size(None, None);
    buffer.shape_until_scroll(fonts, false);
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0f32, f32::max)
}

/// 同步渲染（测试/脚本用）：内部起一个临时引擎，阻塞直到结果返回。
pub fn render_blocking(source: &str, dark: bool) -> Result<String, String> {
    let engine = MermaidEngine::new()?;
    engine.submit("__sync__".into(), source.to_string(), dark);
    let deadline = Instant::now() + RENDER_TIMEOUT + Duration::from_secs(10);
    loop {
        for (label, res) in engine.try_recv() {
            if label == "__sync__" {
                return res;
            }
        }
        if Instant::now() > deadline {
            return Err("sync render timeout".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 冒烟：完整 mermaid.js 在 QuickJS 内出图，且不依赖 foreignObject（resvg 不支持）。
    #[test]
    fn renders_flowchart_as_text_svg() {
        let svg = render_blocking("graph TD\n A[开始] --> B{判断}\n B -->|是| C[结束]", false)
            .expect("flowchart render");
        assert!(svg.contains("<svg"), "svg root missing");
        assert!(svg.contains("<text"), "expected <text> labels");
        assert!(
            !svg.to_ascii_lowercase().contains("foreignobject"),
            "htmlLabels must be disabled for resvg"
        );
    }

    /// 深色主题应产出与浅色不同的样式（缓存/重排依赖该差异）。
    #[test]
    fn dark_theme_differs() {
        let light = render_blocking("graph TD\n A --> B", false).expect("light");
        let dark = render_blocking("graph TD\n A --> B", true).expect("dark");
        assert_ne!(light, dark);
    }

    /// Task 8B.2：边标签颜色必须来自最终 SVG 的生效规则，并覆盖真实“是/否”元素。
    #[test]
    fn dark_flowchart_edge_label_contrast_uses_final_svg_css() {
        let source = "flowchart LR\n  A[开始] --> B{判断}\n  B -->|是| C[完成]\n  B -->|否| A";
        let mut session = Session::new().expect("initialize Mermaid");
        let dark = session.render(source, true).expect("dark flowchart");
        let light = session.render(source, false).expect("light flowchart");
        let doc = roxmltree::Document::parse(&dark).expect("dark flowchart SVG XML");
        let labels: Vec<_> = doc
            .descendants()
            .filter(|node| {
                node.tag_name().name() == "g"
                    && has_class(*node, "edgeLabel")
                    && !normalized_text(*node).is_empty()
            })
            .collect();
        assert_eq!(labels.len(), 2, "expected visible 是/否 edge labels");
        for label in labels {
            assert!(matches!(normalized_text(label).as_str(), "是" | "否"));
            assert!(
                label
                    .descendants()
                    .any(|node| node.tag_name().name() == "rect"),
                "edge label background rect missing"
            );
            assert!(
                label
                    .descendants()
                    .any(|node| node.tag_name().name() == "text"),
                "edge label text missing"
            );
        }

        let text = css_property(last_css_rule(&dark, ".edgeLabel .label text{"), "fill");
        let rect_rule = last_css_rule(&dark, ".edgeLabel rect{");
        let background = css_property(rect_rule, "fill");
        let opacity = css_property(rect_rule, "opacity")
            .parse::<f64>()
            .expect("numeric edge label opacity");
        assert_eq!(text, "#ffffff");
        assert_eq!(background, "#4b5563");
        assert_eq!(opacity, 1.0);
        let ratio = contrast_ratio_css(text, background);
        assert!(
            ratio >= 4.5,
            "dark flowchart edge label contrast {ratio:.2} ({text} on {background})"
        );
        assert!(
            !light.contains("[aria-roledescription=\"flowchart-v2\"] .edgeLabel"),
            "light flowchart must not receive dark edge label overrides"
        );
    }

    /// 语法错误必须返回可读错误（上层降级为源码块），且引擎仍可继续使用。
    #[test]
    fn invalid_source_is_recoverable() {
        let err = render_blocking("graph TD\n A -->", false).expect_err("should fail");
        assert!(!err.is_empty());
        let ok = render_blocking("sequenceDiagram\n A->>B: hi", false).expect("recover");
        assert!(ok.contains("<svg"));
    }

    /// mindmap 渲染：cytoscape+cose-bilkent 布局，不依赖 canvas 2D。
    /// linkedom 提供容器尺寸与 SVG 几何测量，径向初始化沿横向分配主分支。
    #[test]
    fn mindmap_renders_svg() {
        let svg = render_blocking(
            "mindmap\n  root((中心))\n    子节点一\n      孙节点\n    子节点二",
            false,
        )
        .expect("mindmap should render SVG");
        assert!(svg.contains("<svg"), "svg root missing");
        assert!(svg.contains("<text"), "text labels missing");
        assert!(
            svg.contains("<path") || svg.contains("<rect") || svg.contains("<ellipse"),
            "nodes/edges missing (expected path/rect/ellipse)"
        );
        assert!(
            !svg.to_ascii_lowercase().contains("foreignobject"),
            "htmlLabels must be disabled"
        );
    }

    /// DOM 测量必须按 path 命令解析 h/v/q，而不是把数字两两分组。
    #[test]
    fn path_bbox_respects_command_arity() {
        let session = Session::new().expect("initialize Mermaid");
        let bbox: String = session.ctx.with(|ctx| {
            ctx.eval(
                r#"(function() {
                    var path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
                    path.setAttribute('d', 'M-52 9.6 v-19.2 q0,-5 5,-5 h94 q5,0 5,5 v19.2 q0,5 -5,5 h-94 q-5,0 -5,-5 Z');
                    document.documentElement.appendChild(path);
                    var b = path.getBBox();
                    return [b.x, b.y, b.width, b.height].join(',');
                })()"#,
            )
            .expect("measure command-aware path bbox")
        });
        let values: Vec<f64> = bbox
            .split(',')
            .map(str::parse)
            .collect::<Result<_, _>>()
            .expect("numeric bbox");
        let expected = [-52.0, -14.6, 104.0, 29.2];
        assert_eq!(values.len(), expected.len());
        for (actual, expected) in values.into_iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 1e-6,
                "bbox mismatch: {actual} != {expected}"
            );
        }
    }

    // =========================================================================
    // 补充：sequence / gantt / pie 渲染 + 超时 + 错误可恢复
    // =========================================================================

    /// sequenceDiagram：参与者框 + 消息箭头。
    #[test]
    fn renders_sequence_svg() {
        let svg = render_blocking(
            "sequenceDiagram\n    Alice->>+Bob: Hello Bob\n    Bob-->>-Alice: Hi Alice\n    Alice->>Bob: How are you?",
            false,
        )
        .expect("sequence render");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("<text"), "sequence 应有 <text> 标签");
        assert!(
            !svg.to_ascii_lowercase().contains("foreignobject"),
            "htmlLabels 必须关闭"
        );
    }

    /// gantt 时间线：横向甘特条。
    #[test]
    fn renders_gantt_svg() {
        let svg = render_blocking(
            "gantt\n    title 项目进度\n    dateFormat  YYYY-MM-DD\n    section 开发\n    设计阶段    :a1, 2025-01-01, 14d\n    编码阶段    :after a1, 20d\n    section 测试\n    单元测试    :2025-01-20, 10d",
            false,
        )
        .expect("gantt render");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("<text"), "gantt 应有 <text> 标签");
        assert!(
            !svg.to_ascii_lowercase().contains("foreignobject"),
            "htmlLabels 必须关闭"
        );
    }

    /// Task 7.1：gantt 任务/分节文字默认 11px 偏小；通过 `gantt.fontSize` /
    /// `sectionFontSize` 显式抬到 14px（Mermaid 公开配置项，不改布局算法）。
    #[test]
    fn gantt_task_and_section_text_have_min_font_size() {
        let (_, source) = corpus::MERMAID_11_12_CORPUS
            .iter()
            .find(|(name, _)| *name == "gantt")
            .expect("gantt corpus entry");
        let svg = render_blocking(source, false).expect("gantt render");
        let doc = roxmltree::Document::parse(&svg).expect("gantt SVG XML");
        for class in ["taskText", "sectionTitle"] {
            let nodes: Vec<_> = doc
                .descendants()
                .filter(|node| {
                    node.tag_name().name() == "text"
                        && node.attribute("class").is_some_and(|value| {
                            value.split_whitespace().any(|token| token == class)
                        })
                })
                .collect();
            assert!(!nodes.is_empty(), "{class} missing in gantt SVG");
            for node in nodes {
                let size = node
                    .attribute("font-size")
                    .and_then(|value| value.parse::<f64>().ok())
                    .unwrap_or_else(|| panic!("{class} missing numeric font-size"));
                assert!(
                    size >= 14.0,
                    "{class} font-size must be >= 14px, got {size}"
                );
            }
        }
    }

    /// Task 7.1 / 7.4：深色 gantt 的任务文字与任务条底色达到 WCAG AA。
    /// 颜色取自 Mermaid 主题 CSS（`.taskText0` / `.task0`），非人工目检。
    #[test]
    fn dark_gantt_task_text_has_readable_contrast() {
        let (_, source) = corpus::MERMAID_11_12_CORPUS
            .iter()
            .find(|(name, _)| *name == "gantt")
            .expect("gantt corpus entry");
        let mut session = Session::new().expect("initialize Mermaid");
        let dark = session.render(source, true).expect("dark gantt render");
        let text = css_property(last_css_rule(&dark, ".taskText0"), "fill");
        let bar = css_property(last_css_rule(&dark, ".task0"), "fill");
        let ratio = contrast_ratio_css(text, bar);
        assert!(
            ratio >= 4.5,
            "dark gantt task text contrast {ratio:.2} ({text} on {bar})"
        );
    }

    /// Task 8A.3 / 8B.2：dark gantt 的普通任务、标题、分节、网格和轴文字满足对比度要求；
    /// 所有颜色均从最终 SVG CSS 提取，tick 还需按其实际 opacity 合成后再计算。
    #[test]
    fn dark_gantt_bars_grid_and_axis_have_readable_contrast() {
        let (_, source) = corpus::MERMAID_11_12_CORPUS
            .iter()
            .find(|(name, _)| *name == "gantt")
            .expect("gantt corpus entry");
        let mut session = Session::new().expect("initialize Mermaid");
        let dark = session.render(source, true).expect("dark gantt render");
        let light = session.render(source, false).expect("light gantt render");

        for index in 0..4 {
            let bar_selector = format!(".task{index}");
            let text_selector = format!(".taskText{index}");
            let outside_selector = format!(".taskTextOutside{index}");
            let bar = css_property(last_css_rule(&dark, &bar_selector), "fill");
            let text = css_property(last_css_rule(&dark, &text_selector), "fill");
            let outside = css_property(last_css_rule(&dark, &outside_selector), "fill");
            assert_eq!(bar, "#8b949e");
            assert_eq!(text, "#111418");
            assert_eq!(outside, "#ffffff");

            let text_bar_ratio = contrast_ratio_css(text, bar);
            assert!(
                text_bar_ratio >= 4.5,
                "dark gantt taskText{index} contrast {text_bar_ratio:.2} ({text} on {bar})"
            );
            for page in ["#282828", "#1e1e1e"] {
                let bar_page_ratio = contrast_ratio_css(bar, page);
                assert!(
                    bar_page_ratio >= 3.0,
                    "dark gantt task{index} contrast {bar_page_ratio:.2} ({bar} on {page})"
                );
                let outside_page_ratio = contrast_ratio_css(outside, page);
                assert!(
                    outside_page_ratio >= 4.5,
                    "dark gantt taskTextOutside{index} contrast {outside_page_ratio:.2} ({outside} on {page})"
                );
            }
        }

        let doc = roxmltree::Document::parse(&dark).expect("dark gantt SVG XML");
        for (selector, class) in [
            (".titleText{", "titleText"),
            (".sectionTitle{", "sectionTitle"),
        ] {
            let color = css_property(last_css_rule(&dark, selector), "fill");
            assert_eq!(color, "#ffffff");
            assert!(
                doc.descendants().any(|node| has_class(node, class)),
                "dark gantt must contain an actual {class} element"
            );
            for page in ["#282828", "#1e1e1e"] {
                let ratio = contrast_ratio_css(color, page);
                assert!(
                    ratio >= 4.5,
                    "dark gantt {class} contrast {ratio:.2} ({color} on {page})"
                );
            }
        }

        let grid_rule = last_css_rule(&dark, ".grid .tick{");
        let grid = css_property(grid_rule, "stroke");
        let axis = css_property(last_css_rule(&dark, ".grid .tick text{"), "fill");
        assert_eq!(grid, "#8c8c8c");
        assert_eq!(axis, "#ffffff");
        let tick_opacity = css_property(grid_rule, "opacity")
            .parse::<f64>()
            .expect("numeric dark gantt tick opacity");
        let grid_ratio = contrast_ratio_css_composited(grid, "#282828", tick_opacity);
        assert!(
            grid_ratio >= 3.0,
            "dark gantt grid contrast {grid_ratio:.2} ({grid} at opacity {tick_opacity} on #282828)"
        );
        let axis_ratio = contrast_ratio_css_composited(axis, "#282828", tick_opacity);
        assert!(
            axis_ratio >= 4.5,
            "dark gantt axis contrast {axis_ratio:.2} ({axis} at opacity {tick_opacity} on #282828)"
        );
        assert!(
            !light.contains("[aria-roledescription=\"gantt\"]"),
            "light gantt must not receive dark overrides"
        );
    }

    /// Task 7.1：c4context 的 stereotype 字号由 Mermaid c4 渲染器写死（12px），
    /// 没有公开配置项；放大 CSS 会超出按 12px 测量出的固定方框，因此记为
    /// **已知降级**，这里只锁定当前值并验证深色底色上的对比度。
    #[test]
    fn dark_c4_stereotype_contrast_and_fixed_font_size() {
        let (_, source) = corpus::MERMAID_11_12_CORPUS
            .iter()
            .find(|(name, _)| *name == "c4context")
            .expect("c4context corpus entry");
        let mut session = Session::new().expect("initialize Mermaid");
        let dark = session.render(source, true).expect("dark c4 render");
        let doc = roxmltree::Document::parse(&dark).expect("c4 SVG XML");
        // 已知降级：stereotype 固定 12px；深色主题沿用 c4 自带深蓝底 + 白字。
        for (stereotype, background) in [("<<person>>", "#08427B"), ("<<system>>", "#1168BD")] {
            let node = doc
                .descendants()
                .find(|node| {
                    node.tag_name().name() == "text" && normalized_text(*node) == stereotype
                })
                .unwrap_or_else(|| panic!("c4 stereotype {stereotype} missing"));
            assert_eq!(
                node.attribute("font-size"),
                Some("12"),
                "c4 stereotype 字号由 Mermaid 固定（约 12px）；若上游变更需重新评估"
            );
            let fill = node.attribute("fill").expect("stereotype fill");
            let ratio = contrast_ratio(fill, background);
            assert!(
                ratio >= 4.5,
                "c4 stereotype {stereotype} contrast {ratio:.2} ({fill} on {background})"
            );
        }
    }

    /// pie 饼图：简单数值列表。
    #[test]
    fn renders_pie_svg() {
        let svg = render_blocking(
            "pie\n    title 语言分布\n    \"Rust\" : 45\n    \"Go\" : 30\n    \"Python\" : 25",
            false,
        )
        .expect("pie render");
        assert!(svg.contains("<svg"));
        assert!(
            svg.contains("<path") || svg.contains("<ellipse"),
            "pie 应有 path/ellipse 扇区"
        );
        assert!(
            !svg.to_ascii_lowercase().contains("foreignobject"),
            "htmlLabels 必须关闭"
        );
    }

    /// 正常的环形依赖图不应误触发超时；语法错误仍须可读。
    #[test]
    fn cyclic_graph_and_invalid_source_remain_recoverable() {
        let result = render_blocking("graph TD\n A-->B\n B-->C\n C-->A", false);
        assert!(result.is_ok(), "正常的环形依赖图也应渲染成功");

        // 真正无法渲染的源码（语法错误或语义错误）应返回错误
        let err = render_blocking("graph TD\n A --> [B\n C -->", false).expect_err("broken");
        assert!(!err.is_empty(), "错误消息必须可读：{err}");
    }

    #[test]
    #[ignore]
    fn pathological_eval_child() {
        let Ok(mode) = std::env::var("VEIL_PATHOLOGICAL_EVAL") else {
            return;
        };
        let session = Session::new().expect("initialize Mermaid");
        session
            .deadline
            .set(Some(Instant::now() + Duration::from_millis(200)));
        session.ctx.with(|ctx| {
            if mode == "loop" {
                assert!(ctx.eval::<(), _>("while (true) {}").is_err());
            } else {
                ctx.eval::<(), _>(
                    "Promise.resolve().then(function spin() { Promise.resolve().then(spin); });",
                )
                .expect("queue microtask loop");
                while ctx.execute_pending_job() {}
            }
        });
        session.deadline.set(None);
        let answer: i32 = session
            .ctx
            .with(|ctx| ctx.eval("21 * 2"))
            .expect("runtime remains usable after interruption");
        assert_eq!(answer, 42);
    }

    #[test]
    fn pathological_single_eval_has_a_real_deadline() {
        for mode in ["loop", "microtask"] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::pathological_eval_child",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("VEIL_PATHOLOGICAL_EVAL", mode)
                .spawn()
                .expect("spawn pathological eval child");
            let started = Instant::now();
            loop {
                if let Some(status) = child.try_wait().expect("poll pathological eval child") {
                    assert!(status.success(), "{mode} child failed: {status}");
                    break;
                }
                if started.elapsed() > Duration::from_secs(5) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{mode} eval did not terminate when the deadline elapsed");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    #[derive(Debug)]
    struct SvgCheck {
        view_box: [f64; 4],
        bitmap_width: u32,
        bitmap_height: u32,
        non_transparent_pixels: usize,
    }

    struct Raster {
        width: u32,
        height: u32,
        pixmap: resvg::tiny_skia::Pixmap,
    }

    fn svg_view_box(doc: &roxmltree::Document<'_>) -> Result<[f64; 4], String> {
        let root = doc.root_element();
        let raw_view_box = root
            .attribute("viewBox")
            .or_else(|| root.attribute("viewbox"))
            .ok_or_else(|| "missing viewBox".to_string())?;
        let values: Vec<f64> = raw_view_box
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|part| !part.is_empty())
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|e| format!("invalid viewBox {raw_view_box}: {e}"))?;
        let view_box: [f64; 4] = values
            .try_into()
            .map_err(|_| format!("viewBox must have four numbers: {raw_view_box}"))?;
        let [_, _, width, height] = view_box;
        if !view_box.iter().all(|v| v.is_finite()) || width < 16.0 || height < 16.0 {
            return Err(format!("unreasonable viewBox dimensions: {view_box:?}"));
        }
        let aspect = (width / height).max(height / width);
        if aspect > 20.0 {
            return Err(format!(
                "degenerate viewBox aspect ratio {aspect:.2}: {view_box:?}"
            ));
        }
        Ok(view_box)
    }

    fn validate_root_dimensions(
        doc: &roxmltree::Document<'_>,
        view_box: [f64; 4],
    ) -> Result<(), String> {
        let root = doc.root_element();
        if root.attribute("viewBox").is_none() {
            return Err("standalone SVG missing case-sensitive viewBox".into());
        }
        for (attribute, expected) in [("width", view_box[2]), ("height", view_box[3])] {
            let raw = root
                .attribute(attribute)
                .ok_or_else(|| format!("standalone SVG missing numeric {attribute}"))?;
            let actual: f64 = raw
                .parse()
                .map_err(|_| format!("standalone SVG {attribute} must be numeric: {raw:?}"))?;
            if !actual.is_finite() || actual <= 0.0 || (actual - expected).abs() > 0.001 {
                return Err(format!(
                    "standalone SVG {attribute}={actual} does not match viewBox size {expected}"
                ));
            }
        }
        Ok(())
    }

    fn rasterize_svg(svg: &str) -> Result<(Raster, [f64; 4]), String> {
        let doc = roxmltree::Document::parse(svg).map_err(|e| format!("XML: {e}"))?;
        let view_box = svg_view_box(&doc)?;
        let mut options = resvg::usvg::Options::default();
        options.fontdb_mut().load_system_fonts();
        let tree =
            resvg::usvg::Tree::from_str(svg, &options).map_err(|e| format!("resvg parse: {e}"))?;
        let size = tree.size();
        // 最长边固定为 512：旧的超宽 timeline/treemap 会在这里退化成 1–2 像素高。
        let scale = 512.0 / size.width().max(size.height());
        let width = (size.width() * scale).round().max(1.0) as u32;
        let height = (size.height() * scale).round().max(1.0) as u32;
        if width < 16 || height < 16 {
            return Err(format!(
                "degenerate resvg bitmap {width}x{height} from viewBox {view_box:?}"
            ));
        }
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| format!("pixmap allocation failed: {width}x{height}"))?;
        resvg::render(
            &tree,
            tiny_skia_path::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        Ok((
            Raster {
                width,
                height,
                pixmap,
            },
            view_box,
        ))
    }

    /// 不只检查 SVG 标签：视图尺寸、关键几何和真实 resvg 像素都必须有效。
    fn validate_corpus_svg(name: &str, svg: &str) -> Result<SvgCheck, String> {
        let doc = roxmltree::Document::parse(svg).map_err(|e| format!("XML: {e}"))?;
        let view_box = svg_view_box(&doc)?;
        validate_root_dimensions(&doc, view_box)?;
        validate_key_geometry(name, &doc, view_box)?;

        let (raster, raster_view_box) = rasterize_svg(svg)?;
        if name == "treemap" {
            validate_treemap_bitmap(&doc, raster_view_box, &raster)?;
        }
        let non_transparent_pixels = raster
            .pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] != 0)
            .count();
        if non_transparent_pixels < 16 {
            return Err(format!(
                "resvg produced only {non_transparent_pixels} non-transparent pixels"
            ));
        }
        Ok(SvgCheck {
            view_box,
            bitmap_width: raster.width,
            bitmap_height: raster.height,
            non_transparent_pixels,
        })
    }

    fn attr_f64(node: roxmltree::Node<'_, '_>, name: &str) -> f64 {
        node.attribute(name)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0.0)
    }

    fn presentation_property(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
        node.attribute("style")
            .and_then(|style| {
                style.split(';').find_map(|declaration| {
                    let (property, value) = declaration.split_once(':')?;
                    (property.trim() == name).then(|| value.trim().to_string())
                })
            })
            .or_else(|| node.attribute(name).map(str::to_string))
    }

    fn require_visible_paint(
        node: roxmltree::Node<'_, '_>,
        property: &str,
        subject: &str,
    ) -> Result<(), String> {
        let paint = presentation_property(node, property)
            .ok_or_else(|| format!("{subject} missing {property}"))?;
        if matches!(
            paint.trim().to_ascii_lowercase().as_str(),
            "none" | "transparent"
        ) {
            return Err(format!("{subject} {property} must be non-transparent"));
        }
        for opacity_name in [format!("{property}-opacity"), "opacity".to_string()] {
            if let Some(opacity) = presentation_property(node, &opacity_name) {
                let opacity: f64 = opacity
                    .parse()
                    .map_err(|_| format!("invalid {subject} {opacity_name}: {opacity}"))?;
                if !opacity.is_finite() || opacity <= 0.0 {
                    return Err(format!("{subject} {opacity_name} must be positive"));
                }
            }
        }
        Ok(())
    }

    fn normalized_text(node: roxmltree::Node<'_, '_>) -> String {
        let raw = node
            .descendants()
            .filter(|descendant| descendant.is_text())
            .filter_map(|descendant| descendant.text())
            .collect::<String>();
        raw.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn rect_bounds(node: roxmltree::Node<'_, '_>) -> [f64; 4] {
        let (tx, ty) = translation(node);
        [
            attr_f64(node, "x") + tx,
            attr_f64(node, "y") + ty,
            attr_f64(node, "width"),
            attr_f64(node, "height"),
        ]
    }

    fn translation(node: roxmltree::Node<'_, '_>) -> (f64, f64) {
        node.ancestors()
            .filter_map(|ancestor| ancestor.attribute("transform"))
            .filter_map(|transform| {
                let inner = transform
                    .trim()
                    .strip_prefix("translate(")?
                    .strip_suffix(')')?;
                let values: Vec<f64> = inner
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|part| !part.is_empty())
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .ok()?;
                Some((
                    values.first().copied().unwrap_or(0.0),
                    values.get(1).copied().unwrap_or(0.0),
                ))
            })
            .fold((0.0, 0.0), |(x, y), (tx, ty)| (x + tx, y + ty))
    }

    fn contains_point(view_box: [f64; 4], x: f64, y: f64) -> bool {
        let [vx, vy, width, height] = view_box;
        let tolerance = 0.51; // Mermaid 部分 viewBox 会 round 到整数。
        x >= vx - tolerance
            && y >= vy - tolerance
            && x <= vx + width + tolerance
            && y <= vy + height + tolerance
    }

    fn raster_alpha_at(raster: &Raster, view_box: [f64; 4], x: f64, y: f64) -> u8 {
        let px = ((x - view_box[0]) * f64::from(raster.width) / view_box[2]).floor() as i32;
        let py = ((y - view_box[1]) * f64::from(raster.height) / view_box[3]).floor() as i32;
        if px < 0 || py < 0 || px >= raster.width as i32 || py >= raster.height as i32 {
            return 0;
        }
        let index = ((py as u32 * raster.width + px as u32) * 4) as usize;
        raster.pixmap.data()[index + 3]
    }

    fn validate_treemap_bitmap(
        doc: &roxmltree::Document<'_>,
        view_box: [f64; 4],
        raster: &Raster,
    ) -> Result<(), String> {
        // 在每叶下部、远离中央文字和边缘 stroke 的区域采样。透明叶上即使仍有
        // 文字像素，也不能让分块可见性检查假阳性。
        for (leaf_index, leaf) in doc
            .descendants()
            .filter(|node| {
                node.tag_name().name() == "rect"
                    && node
                        .attribute("class")
                        .is_some_and(|class| class.split_whitespace().any(|c| c == "treemapLeaf"))
            })
            .enumerate()
        {
            let [x, y, width, height] = rect_bounds(leaf);
            let mut covered = 0;
            for row in 0..4 {
                for column in 0..10 {
                    let sample_x = x + width * (0.15 + 0.70 * column as f64 / 9.0);
                    let sample_y = y + height * (0.78 + 0.12 * row as f64 / 3.0);
                    covered +=
                        usize::from(raster_alpha_at(raster, view_box, sample_x, sample_y) >= 24);
                }
            }
            if covered < 36 {
                return Err(format!(
                    "treemap leaf {leaf_index} background pixel coverage failed: {covered}/40"
                ));
            }
        }

        // 每个标签单独检查其基线附近的高不透明度像素，避免只有分块而没有文字。
        for (text, label) in doc
            .descendants()
            .filter(|node| {
                node.tag_name().name() == "text"
                    && node.attribute("class").is_some_and(|class| {
                        class
                            .split_whitespace()
                            .any(|c| c == "treemapLabel" || c == "treemapValue")
                    })
            })
            .map(|node| (normalized_text(node), node))
        {
            if !matches!(text.as_str(), "Phones" | "30" | "Laptops" | "20") {
                continue;
            }
            let (tx, ty) = translation(label);
            let x = attr_f64(label, "x") + tx;
            let y = attr_f64(label, "y") + ty;
            let font_size = presentation_property(label, "font-size")
                .and_then(|value| value.trim_end_matches("px").parse::<f64>().ok())
                .unwrap_or(16.0);
            let x_radius = (text.chars().count() as f64 * font_size * 0.45).max(font_size);
            let is_value = label
                .attribute("class")
                .is_some_and(|class| class.split_whitespace().any(|c| c == "treemapValue"));
            let (top, bottom) = if is_value {
                (y, y + font_size * 1.2)
            } else {
                (y - font_size * 0.8, y + font_size * 0.45)
            };
            let sx = f64::from(raster.width) / view_box[2];
            let sy = f64::from(raster.height) / view_box[3];
            let left = ((x - x_radius - view_box[0]) * sx)
                .floor()
                .clamp(0.0, f64::from(raster.width)) as u32;
            let right = ((x + x_radius - view_box[0]) * sx)
                .ceil()
                .clamp(0.0, f64::from(raster.width)) as u32;
            let top = ((top - view_box[1]) * sy)
                .floor()
                .clamp(0.0, f64::from(raster.height)) as u32;
            let bottom = ((bottom - view_box[1]) * sy)
                .ceil()
                .clamp(0.0, f64::from(raster.height)) as u32;
            let mut opaque = 0;
            for py in top..bottom {
                for px in left..right {
                    let index = ((py * raster.width + px) * 4) as usize;
                    opaque += usize::from(raster.pixmap.data()[index + 3] >= 200);
                }
            }
            if opaque < 8 {
                return Err(format!(
                    "treemap label {text:?} has only {opaque} opaque resvg pixels"
                ));
            }
        }
        Ok(())
    }

    fn validate_mindmap_geometry(
        doc: &roxmltree::Document<'_>,
        view_box: [f64; 4],
    ) -> Result<(), String> {
        let mut centers = [None; 4];
        for node in doc.descendants().filter(|node| {
            node.tag_name().name() == "g"
                && node
                    .attribute("class")
                    .is_some_and(|class| class.split_whitespace().any(|c| c == "mindmap-node"))
        }) {
            let index = node
                .attribute("id")
                .and_then(|id| id.strip_prefix("node_"))
                .and_then(|index| index.parse::<usize>().ok())
                .ok_or_else(|| "mindmap node id missing".to_string())?;
            if index >= centers.len() || centers[index].is_some() {
                return Err(format!("unexpected or duplicate mindmap node_{index}"));
            }
            centers[index] = Some(translation(node));
        }
        let [Some(root), Some(first), Some(grandchild), Some(second)] = centers else {
            return Err("mindmap corpus must contain node_0 through node_3".into());
        };
        for (index, (x, y)) in [root, first, grandchild, second].into_iter().enumerate() {
            if !contains_point(view_box, x, y) {
                return Err(format!("mindmap node_{index} center outside viewBox"));
            }
        }
        let horizontal_span = root.0.max(first.0).max(grandchild.0).max(second.0)
            - root.0.min(first.0).min(grandchild.0).min(second.0);
        if view_box[2] < view_box[3] * 1.5 || horizontal_span < view_box[2] * 0.60 {
            return Err(format!(
                "mindmap horizontal layout required: viewBox={view_box:?}, center span={horizontal_span:.2}"
            ));
        }
        if root.0 - first.0 < 40.0 || second.0 - root.0 < 40.0 {
            return Err(format!(
                "mindmap primary branches must extend to both sides: root={root:?}, first={first:?}, second={second:?}"
            ));
        }
        if first.0 - grandchild.0 < 40.0 {
            return Err(format!(
                "mindmap grandchild must continue the left branch: first={first:?}, grandchild={grandchild:?}"
            ));
        }
        Ok(())
    }

    fn has_class(node: roxmltree::Node<'_, '_>, expected: &str) -> bool {
        node.attribute("class")
            .is_some_and(|class| class.split_whitespace().any(|c| c == expected))
    }

    fn validate_relationship_labels(
        doc: &roxmltree::Document<'_>,
        view_box: [f64; 4],
        box_class: &str,
        expected_count: usize,
        requirement: bool,
    ) -> Result<(), String> {
        let css = doc
            .descendants()
            .find(|node| node.tag_name().name() == "style")
            .map(normalized_text)
            .unwrap_or_default();
        if !css.contains(&format!(".{box_class}{{fill:")) {
            return Err(format!("diagram theme is missing .{box_class} fill"));
        }

        let labels: Vec<_> = doc
            .descendants()
            .filter(|node| node.tag_name().name() == "g" && has_class(*node, "edgeLabel"))
            .collect();
        if labels.len() != expected_count {
            return Err(format!(
                "expected {expected_count} relationship labels, found {}",
                labels.len()
            ));
        }
        for label in labels {
            let backgrounds: Vec<_> = label
                .descendants()
                .filter(|node| node.tag_name().name() == "rect" && has_class(*node, "background"))
                .collect();
            let [background] = backgrounds.as_slice() else {
                return Err("relationship label must have exactly one background rect".into());
            };
            if !has_class(*background, box_class) {
                return Err(format!("relationship label background missing {box_class}"));
            }
            let bounds = rect_bounds(*background);
            if bounds[2] <= 0.0
                || bounds[3] <= 0.0
                || !contains_point(view_box, bounds[0], bounds[1])
                || !contains_point(view_box, bounds[0] + bounds[2], bounds[1] + bounds[3])
            {
                return Err(format!(
                    "relationship label box outside viewBox: {bounds:?}"
                ));
            }

            let text = label
                .descendants()
                .find(|node| node.tag_name().name() == "text")
                .ok_or_else(|| "relationship label text missing".to_string())?;
            let value = normalized_text(text);
            if requirement {
                let valid = value
                    .strip_prefix("<<")
                    .and_then(|inner| inner.strip_suffix(">>"))
                    .is_some_and(|inner| {
                        !inner.is_empty()
                            && inner
                                .chars()
                                .all(|character| character.is_ascii_alphabetic())
                    });
                if !valid {
                    return Err(format!(
                        "requirement relation must be a decoded <<stereotype>>, found {value:?}"
                    ));
                }
            } else if value.is_empty() {
                return Err("ER relationship label is empty".into());
            }
            let (tx, ty) = translation(text);
            let text_x = attr_f64(text, "x") + tx;
            let text_y = attr_f64(text, "y") + ty;
            if !contains_point(bounds, text_x, text_y) {
                return Err(format!(
                    "relationship label text {value:?} is outside its background"
                ));
            }
        }
        Ok(())
    }

    fn path_commands(d: &str) -> Result<Vec<(char, Vec<f64>)>, String> {
        let commands: Vec<_> = d
            .match_indices(|character: char| "MmLlHhVvCcSsQqTtAaZz".contains(character))
            .collect();
        let Some((first, _)) = commands.first() else {
            return Err("marker path has no drawing commands".into());
        };
        if !d[..*first].trim().is_empty() {
            return Err("marker path has data before its first command".into());
        }
        let mut parsed = Vec::new();
        for (index, (start, command)) in commands.iter().enumerate() {
            let end = commands.get(index + 1).map_or(d.len(), |(next, _)| *next);
            let numbers = d[start + command.len()..end]
                .split(|character: char| character.is_whitespace() || character == ',')
                .filter(|part| !part.is_empty())
                .map(str::parse)
                .collect::<Result<Vec<f64>, _>>()
                .map_err(|error| format!("invalid marker path number: {error}"))?;
            parsed.push((command.chars().next().expect("SVG command"), numbers));
        }
        Ok(parsed)
    }

    fn absolute_endpoint(command: char, numbers: &[f64]) -> Option<(f64, f64)> {
        let arity = match command {
            'M' | 'L' => 2,
            'C' => 6,
            'Q' => 4,
            _ => return None,
        };
        if numbers.is_empty() || !numbers.len().is_multiple_of(arity) {
            return None;
        }
        Some((numbers[numbers.len() - 2], numbers[numbers.len() - 1]))
    }

    fn validate_marker_paths(doc: &roxmltree::Document<'_>) -> Result<(), String> {
        for path in doc.descendants().filter(|node| {
            node.tag_name().name() == "path"
                && (node.attribute("marker-end").is_some()
                    || node.attribute("marker-start").is_some())
        }) {
            let d = path
                .attribute("d")
                .ok_or_else(|| "marker path missing d".to_string())?;
            let commands = path_commands(d)?;
            for (command, numbers) in &commands {
                if *command == 'C' {
                    if numbers.is_empty() || !numbers.len().is_multiple_of(6) {
                        return Err(format!("malformed cubic marker path: {d}"));
                    }
                    for cubic in numbers.as_chunks::<6>().0 {
                        if cubic[0] == cubic[2]
                            && cubic[2] == cubic[4]
                            && cubic[1] == cubic[3]
                            && cubic[3] == cubic[5]
                        {
                            return Err(format!("zero-length cubic breaks marker tangent: {d}"));
                        }
                    }
                }
            }
            for index in 1..commands.len() {
                let (command, numbers) = &commands[index];
                if *command != 'L' {
                    continue;
                }
                let (previous_command, previous_numbers) = &commands[index - 1];
                if let (Some(end), Some(previous)) = (
                    absolute_endpoint(*command, numbers),
                    absolute_endpoint(*previous_command, previous_numbers),
                ) {
                    if end == previous {
                        return Err(format!(
                            "duplicate line endpoint breaks marker tangent: {d}"
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_key_geometry(
        name: &str,
        doc: &roxmltree::Document<'_>,
        view_box: [f64; 4],
    ) -> Result<(), String> {
        validate_marker_paths(doc)?;
        match name {
            "timeline" => {
                let title = doc
                    .descendants()
                    .find(|node| {
                        node.tag_name().name() == "text"
                            && node.attribute("font-size") == Some("4ex")
                    })
                    .ok_or_else(|| "timeline title missing".to_string())?;
                let (tx, ty) = translation(title);
                let title_x = attr_f64(title, "x") + tx;
                let title_y = attr_f64(title, "y") + ty;
                if !contains_point(view_box, title_x, title_y) {
                    return Err(format!(
                        "timeline title outside viewBox: ({title_x}, {title_y})"
                    ));
                }
                let mut line_count = 0;
                for line in doc
                    .descendants()
                    .filter(|node| node.tag_name().name() == "line")
                {
                    let (tx, ty) = translation(line);
                    for (x_name, y_name) in [("x1", "y1"), ("x2", "y2")] {
                        let x = attr_f64(line, x_name) + tx;
                        let y = attr_f64(line, y_name) + ty;
                        if !contains_point(view_box, x, y) {
                            return Err(format!("timeline line outside viewBox: ({x}, {y})"));
                        }
                    }
                    line_count += 1;
                }
                if line_count < 3 {
                    return Err("timeline structural lines missing".into());
                }
                let axis = doc
                    .descendants()
                    .find(|node| {
                        node.tag_name().name() == "line"
                            && node.attribute("stroke-width") == Some("4")
                    })
                    .ok_or_else(|| "timeline main axis missing".to_string())?;
                if (attr_f64(axis, "x2") - attr_f64(axis, "x1")).abs() < 600.0 {
                    return Err("timeline main axis collapsed".into());
                }
                let event_lines = doc
                    .descendants()
                    .filter(|node| {
                        node.tag_name().name() == "line"
                            && node.attribute("stroke-width") == Some("2")
                    })
                    .count();
                if event_lines != 2 {
                    return Err(format!(
                        "expected two timeline event lines, found {event_lines}"
                    ));
                }
                for expected in ["2024", "First event", "2025", "Second event"] {
                    let label = doc
                        .descendants()
                        .find(|node| {
                            node.tag_name().name() == "text" && normalized_text(*node) == expected
                        })
                        .ok_or_else(|| format!("timeline label {expected:?} missing"))?;
                    if presentation_property(label, "display").as_deref() == Some("none") {
                        return Err(format!("timeline label {expected:?} is hidden"));
                    }
                    let (tx, ty) = translation(label);
                    if !contains_point(
                        view_box,
                        attr_f64(label, "x") + tx,
                        attr_f64(label, "y") + ty,
                    ) {
                        return Err(format!("timeline label {expected:?} outside viewBox"));
                    }
                }
            }
            "treemap" => {
                let mut leaves = Vec::new();
                for leaf in doc.descendants().filter(|node| {
                    node.tag_name().name() == "rect"
                        && node.attribute("class").is_some_and(|class| {
                            class.split_whitespace().any(|c| c == "treemapLeaf")
                        })
                }) {
                    let [x, y, width, height] = rect_bounds(leaf);
                    let right = x + width;
                    let bottom = y + height;
                    if !contains_point(view_box, x, y) || !contains_point(view_box, right, bottom) {
                        return Err(format!(
                            "treemap leaf outside viewBox: ({x}, {y})..({right}, {bottom})"
                        ));
                    }
                    require_visible_paint(leaf, "fill", "treemap leaf")?;
                    require_visible_paint(leaf, "stroke", "treemap leaf")?;
                    leaves.push([x, y, width, height]);
                }
                if leaves.len() != 2 {
                    return Err(format!(
                        "expected two treemap leaves, found {}",
                        leaves.len()
                    ));
                }

                for expected in ["Phones", "30", "Laptops", "20"] {
                    let matching: Vec<_> = doc
                        .descendants()
                        .filter(|node| {
                            node.tag_name().name() == "text"
                                && node.attribute("class").is_some_and(|class| {
                                    class
                                        .split_whitespace()
                                        .any(|c| c == "treemapLabel" || c == "treemapValue")
                                })
                                && normalized_text(*node) == expected
                        })
                        .collect();
                    if matching.len() != 1 {
                        return Err(format!(
                            "treemap label {expected:?} must occur once, found {}",
                            matching.len()
                        ));
                    }
                    let label = matching[0];
                    if presentation_property(label, "display").as_deref() == Some("none")
                        || presentation_property(label, "visibility").as_deref() == Some("hidden")
                    {
                        return Err(format!("treemap label {expected:?} is hidden"));
                    }
                    require_visible_paint(label, "fill", "treemap label")?;
                    let (tx, ty) = translation(label);
                    let x = attr_f64(label, "x") + tx;
                    let y = attr_f64(label, "y") + ty;
                    if !leaves.iter().any(|[lx, ly, width, height]| {
                        x >= *lx && x <= *lx + *width && y >= *ly && y <= *ly + *height
                    }) {
                        return Err(format!("treemap label {expected:?} is outside both leaves"));
                    }
                }
            }
            "er" => validate_relationship_labels(doc, view_box, "relationshipLabelBox", 2, false)?,
            "requirement" => validate_relationship_labels(doc, view_box, "reqLabelBox", 1, true)?,
            "block" => {
                let edges = doc
                    .descendants()
                    .filter(|node| {
                        node.tag_name().name() == "path" && node.attribute("marker-end").is_some()
                    })
                    .count();
                if edges != 1 {
                    return Err(format!("expected one block marker edge, found {edges}"));
                }
            }
            "mindmap" => validate_mindmap_geometry(doc, view_box)?,
            _ => {}
        }
        Ok(())
    }

    fn last_css_rule<'a>(svg: &'a str, selector: &str) -> &'a str {
        let selector_start = svg
            .rfind(selector)
            .unwrap_or_else(|| panic!("missing CSS selector {selector}"));
        let rule_start = svg[selector_start..]
            .find('{')
            .map(|offset| selector_start + offset + 1)
            .expect("CSS rule start");
        let rule_end = svg[rule_start..]
            .find('}')
            .map(|offset| rule_start + offset)
            .expect("CSS rule end");
        &svg[rule_start..rule_end]
    }

    fn css_property<'a>(rule: &'a str, property: &str) -> &'a str {
        rule.split(';')
            .filter_map(|declaration| declaration.split_once(':'))
            .find(|(name, _)| name.trim() == property)
            .map(|(_, value)| value.trim().trim_end_matches("!important").trim())
            .unwrap_or_else(|| panic!("missing CSS property {property} in {rule}"))
    }

    fn relative_luminance_rgb(channels: [f64; 3]) -> f64 {
        let channels = channels.map(|value| {
            let value = value / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]
    }

    fn relative_luminance(color: &str) -> f64 {
        assert_eq!(color.len(), 7, "expected #RRGGBB: {color}");
        let channels = [1, 3, 5].map(|start| {
            f64::from(u8::from_str_radix(&color[start..start + 2], 16).expect("hex color"))
        });
        relative_luminance_rgb(channels)
    }

    fn contrast_ratio(foreground: &str, background: &str) -> f64 {
        let foreground = relative_luminance(foreground);
        let background = relative_luminance(background);
        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
    }

    /// CSS 颜色（`#rgb` / `#rrggbb` / `hsl(h,s%,l%)`）→ RGB 0..255。
    /// 只覆盖 Mermaid 主题样式实际使用的写法；其他格式直接失败，避免静默跳过断言。
    fn css_color_rgb(value: &str) -> [f64; 3] {
        let value = value.trim();
        if let Some(hex) = value.strip_prefix('#') {
            return match hex.len() {
                6 => [0, 2, 4].map(|i| f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap())),
                3 => [0, 1, 2]
                    .map(|i| f64::from(u8::from_str_radix(&hex[i..i + 1], 16).unwrap() * 17)),
                _ => panic!("unsupported hex color {value}"),
            };
        }
        let inner = value
            .strip_prefix("hsl(")
            .and_then(|rest| rest.strip_suffix(')'))
            .unwrap_or_else(|| panic!("unsupported CSS color {value}"));
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        assert_eq!(parts.len(), 3, "malformed hsl() {value}");
        let hue = parts[0].trim_end_matches("deg").parse::<f64>().unwrap();
        let sat = parts[1].trim_end_matches('%').parse::<f64>().unwrap() / 100.0;
        let light = parts[2].trim_end_matches('%').parse::<f64>().unwrap() / 100.0;
        let c = (1.0 - (2.0 * light - 1.0).abs()) * sat;
        let h = (hue.rem_euclid(360.0)) / 60.0;
        let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = light - c / 2.0;
        [r + m, g + m, b + m].map(|channel| (channel * 255.0).round())
    }

    fn contrast_ratio_css(foreground: &str, background: &str) -> f64 {
        let foreground = relative_luminance_rgb(css_color_rgb(foreground));
        let background = relative_luminance_rgb(css_color_rgb(background));
        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
    }

    fn contrast_ratio_css_composited(foreground: &str, background: &str, opacity: f64) -> f64 {
        assert!((0.0..=1.0).contains(&opacity));
        let foreground = css_color_rgb(foreground);
        let background_rgb = css_color_rgb(background);
        let composited: [f64; 3] = std::array::from_fn(|index| {
            foreground[index] * opacity + background_rgb[index] * (1.0 - opacity)
        });
        let foreground = relative_luminance_rgb(composited);
        let background = relative_luminance_rgb(background_rgb);
        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
    }

    fn assert_mindmap_root_label_centered_inside(svg: &str) {
        let doc = roxmltree::Document::parse(svg).expect("mindmap SVG XML");
        let svg_root = doc.root_element();
        let svg_id = svg_root.attribute("id").expect("mindmap SVG id");
        assert_eq!(svg_root.attribute("aria-roledescription"), Some("mindmap"));
        let root = doc
            .descendants()
            .find(|node| has_class(*node, "section-root"))
            .expect("mindmap section-root");
        assert_eq!(normalized_text(root), "Veil");
        let label = root
            .children()
            .find(|node| has_class(*node, "label"))
            .expect("root .label container");
        let text = label
            .descendants()
            .find(|node| node.tag_name().name() == "text")
            .expect("root text");
        assert_eq!(normalized_text(text), "Veil");

        // SVG style 属于 XML 文本，Mermaid 序列化会把子选择器的 `>` 转义为 `&gt;`。
        let selector =
            format!("#{svg_id}[aria-roledescription=\"mindmap\"] .section-root&gt;.label text{{");
        let anchor = css_property(last_css_rule(svg, &selector), "text-anchor");
        assert_eq!(anchor, "middle");

        // 文字与圆心处于同一 section-root 局部坐标系；居中后左右边界都必须落在圆内。
        let circle = root
            .children()
            .find(|node| node.tag_name().name() == "circle")
            .expect("root circle");
        let center_x = attr_f64(circle, "cx");
        let radius = attr_f64(circle, "r");
        let text_x = text
            .descendants()
            .find(|node| node.tag_name().name() == "tspan" && node.attribute("x").is_some())
            .and_then(|node| node.attribute("x"))
            .unwrap_or("0")
            .parse::<f64>()
            .expect("numeric root text x");
        let anchor_x = translation(text).0 - translation(root).0 + text_x;
        let root_style = last_css_rule(svg, &format!("#{svg_id}{{"));
        let font_size = css_property(root_style, "font-size")
            .trim_end_matches("px")
            .parse::<f32>()
            .expect("mindmap root font size in px");
        let text_width = f64::from(measure_text(&mut FontSystem::new(), "Veil", font_size));
        assert!(
            anchor_x - text_width / 2.0 >= center_x - radius
                && anchor_x + text_width / 2.0 <= center_x + radius,
            "centered Veil bounds [{}, {}] exceed circle [{}, {}]",
            anchor_x - text_width / 2.0,
            anchor_x + text_width / 2.0,
            center_x - radius,
            center_x + radius
        );

        // 光栅差分直接防止白字越出圆形后落到透明页面；只隐藏根文字，不改布局尺寸。
        let node_id = root.attribute("id").expect("root node id");
        let node_marker = format!("id=\"{node_id}\"");
        let node_start = svg.find(&node_marker).expect("root node in SVG");
        let text_start = node_start + svg[node_start..].find("<text ").expect("root text in SVG");
        let mut without_label = svg.to_string();
        without_label.insert_str(text_start + "<text".len(), " style=\"display:none\"");
        let (actual, _) = rasterize_svg(svg).expect("rasterize mindmap");
        let (background, _) = rasterize_svg(&without_label).expect("rasterize root background");
        let mut inside = 0usize;
        let mut escaped = 0usize;
        for (actual, background) in actual
            .pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(background.pixmap.data().as_chunks::<4>().0)
        {
            if actual != background {
                if background[3] == u8::MAX {
                    inside += 1;
                } else {
                    escaped += 1;
                }
            }
        }
        assert!(inside >= 16, "root Veil has too few visible text pixels");
        assert_eq!(escaped, 0, "root Veil pixels escaped the opaque circle");
    }

    fn validate_mindmap_css_theme(light: &str, dark: &str) {
        let dark_text = css_property(last_css_rule(dark, ".mindmap-node text"), "fill");
        let dark_span = css_property(last_css_rule(dark, ".mindmap-node span"), "color");
        let dark_background = css_property(last_css_rule(dark, ".section-root rect"), "fill");
        assert_eq!(dark_text, "#ffffff");
        assert_eq!(dark_span, "#ffffff");
        assert_eq!(dark_background, "#242448");
        assert!(
            contrast_ratio(dark_text, dark_background) >= 4.5,
            "dark mindmap root contrast must meet WCAG AA"
        );

        // 浅主题保持 Mermaid 默认样式，不套用深色的全层级文字覆盖。
        assert_eq!(
            css_property(last_css_rule(light, ".section-root text"), "fill"),
            "#ffffff"
        );
        assert_eq!(
            css_property(last_css_rule(light, ".section-root span"), "color"),
            "#ffffff"
        );
        assert!(!light.contains("#242448"));
        assert!(!light.contains(".mindmap-node text{fill:#ffffff!important;"));
        assert!(!light.contains(".mindmap-node span{color:#ffffff!important;"));
    }

    fn mindmap_label_pixel_contrast(svg: &str, node_index: usize) -> (f64, [f64; 3], [f64; 3]) {
        let node_marker = format!("id=\"node_{node_index}\"");
        let node_start = svg
            .find(&node_marker)
            .unwrap_or_else(|| panic!("missing {node_marker}"));
        let text_start = node_start
            + svg[node_start..]
                .find("<text ")
                .unwrap_or_else(|| panic!("{node_marker} label missing"));
        let mut without_label = svg.to_string();
        // 仅构造测试对照图：隐藏同一节点的文字，以获得每个像素的真实节点底色。
        without_label.insert_str(text_start + "<text".len(), " style=\"display:none\"");

        let (actual, _) = rasterize_svg(svg).expect("rasterize dark mindmap");
        let (background, _) = rasterize_svg(&without_label).expect("rasterize label background");
        assert_eq!(
            (actual.width, actual.height),
            (background.width, background.height)
        );
        let actual_pixels = actual.pixmap.data().as_chunks::<4>().0;
        let background_pixels = background.pixmap.data().as_chunks::<4>().0;
        let mut foreground_sum = [0.0; 3];
        let mut background_sum = [0.0; 3];
        let mut contrasts = Vec::new();
        for (actual, background) in actual_pixels.iter().zip(background_pixels) {
            // 节点内的两种光栅均不透明；比较最终合成色，而非 CSS 颜色字符串。
            if actual[3] == u8::MAX && background[3] == u8::MAX && actual[..3] != background[..3] {
                for channel in 0..3 {
                    foreground_sum[channel] += f64::from(actual[channel]);
                    background_sum[channel] += f64::from(background[channel]);
                }
                let foreground_luminance = relative_luminance_rgb([
                    f64::from(actual[0]),
                    f64::from(actual[1]),
                    f64::from(actual[2]),
                ]);
                let background_luminance = relative_luminance_rgb([
                    f64::from(background[0]),
                    f64::from(background[1]),
                    f64::from(background[2]),
                ]);
                contrasts.push(
                    (foreground_luminance.max(background_luminance) + 0.05)
                        / (foreground_luminance.min(background_luminance) + 0.05),
                );
            }
        }
        assert!(
            contrasts.len() >= 16,
            "{node_marker} has too few rasterized text pixels"
        );
        let count = contrasts.len();
        contrasts.sort_by(f64::total_cmp);
        // 取字芯（最实心的十分之一像素）而不是中位数：抗锯齿边缘像素天然变暗，
        // 其占比随平台字体与 hinting 变化（CI 与本地中位数可差 2:1），不能作为门禁。
        // 字芯达 AA 且平均色正常，才能说明标签不是「暗底暗字」。
        let ratio = contrasts[count * 9 / 10];
        let foreground = foreground_sum.map(|sum| sum / count as f64);
        let background = background_sum.map(|sum| sum / count as f64);
        (ratio, foreground, background)
    }

    #[test]
    fn dark_mindmap_all_label_levels_have_readable_pixels() {
        let mut session = Session::new().expect("initialize Mermaid");
        let (_, source) = corpus::MERMAID_11_12_CORPUS
            .iter()
            .find(|(name, _)| *name == "mindmap")
            .expect("mindmap corpus entry");
        let light = session.render(source, false).expect("light mindmap");
        let dark = session.render(source, true).expect("dark mindmap");
        validate_mindmap_css_theme(&light, &dark);
        let mut failures = Vec::new();
        for (node_index, level) in [(0, "root"), (1, "child"), (2, "grandchild"), (3, "child")] {
            let (ratio, foreground, background) = mindmap_label_pixel_contrast(&dark, node_index);
            println!(
                "dark mindmap node_{node_index} ({level}): contrast={ratio:.2}, mean text RGB={foreground:?}, background RGB={background:?}"
            );
            if ratio < 4.5 {
                failures.push(format!("{level} raster contrast {ratio:.2}"));
            }
        }
        assert!(
            failures.is_empty(),
            "dark mindmap labels must meet WCAG AA: {}",
            failures.join(", ")
        );
    }

    /// Task 8B.2：完整 Veil 必须在两种主题下居中并留在根圆内；dark 连接线按实际 path 验收。
    #[test]
    fn mindmap_root_alignment_and_dark_connector_contrast() {
        let source = "mindmap\n  root((Veil))\n    编辑\n      中文排版\n    导出\n      HTML\n      PDF\n    信任\n      本地 Mermaid";
        let mut session = Session::new().expect("initialize Mermaid");
        let light = session.render(source, false).expect("light mindmap");
        let dark = session.render(source, true).expect("dark mindmap");
        assert_mindmap_root_label_centered_inside(&light);
        assert_mindmap_root_label_centered_inside(&dark);

        let doc = roxmltree::Document::parse(&dark).expect("dark mindmap SVG XML");
        assert!(
            doc.descendants().any(|node| {
                node.tag_name().name() == "path" && has_class(node, "section-edge-0")
            }),
            "actual section-edge-0 connector missing"
        );
        let stroke = css_property(last_css_rule(&dark, ".section-edge-0{"), "stroke");
        assert_eq!(stroke, "#b8c0cc");
        for page in ["#282828", "#1e1e1e"] {
            let ratio = contrast_ratio_css(stroke, page);
            assert!(
                ratio >= 3.0,
                "dark mindmap connector contrast {ratio:.2} ({stroke} on {page})"
            );
        }
        assert!(
            !light.contains("[aria-roledescription=\"mindmap\"] .section-edge-0"),
            "light mindmap must not receive dark connector overrides"
        );
    }

    /// 旧窄图依赖浏览器 300×150 默认尺寸；所有六类失败样本都必须被固有尺寸检查拒绝。
    #[test]
    fn legacy_percent_sized_standalone_svgs_are_rejected() {
        let legacy = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-5 -45 29.48046875 90" width="100%"><rect width="20" height="80" fill="white"/></svg>"#;
        for name in ["flowchart", "class", "state", "er", "requirement", "block"] {
            let error = validate_corpus_svg(name, legacy)
                .expect_err("percentage-sized legacy standalone SVG must fail");
            assert!(
                error.contains("numeric") || error.contains("missing numeric"),
                "{name}: unexpected error: {error}"
            );
        }

        let fixed = legacy.replace("width=\"100%\"", "width=\"29.48046875\" height=\"90\"");
        validate_corpus_svg("flowchart", &fixed)
            .expect("numeric standalone dimensions must rasterize normally");
        let lowercase = fixed.replace("viewBox", "viewbox");
        assert!(validate_corpus_svg("packet", &lowercase)
            .expect_err("browser-inert lowercase viewbox must fail")
            .contains("case-sensitive viewBox"));
    }

    /// 旧 ER/requirement 标签与 block 箭头即使 XML/resvg 可解析，也必须被结构几何拒绝。
    #[test]
    fn legacy_relationship_labels_and_block_tangent_are_rejected() {
        let er = r#"<svg viewBox="0 0 100 100"><style>.relationshipLabelBox{fill:white}</style>
            <g class="edgeLabel"><rect class="background" width="40" height="20"/><text x="5" y="15">places</text></g>
            <g class="edgeLabel"><rect class="background" y="30" width="40" height="20"/><text x="5" y="45">contains</text></g></svg>"#;
        let er_doc = roxmltree::Document::parse(er).expect("legacy ER XML");
        assert!(
            validate_key_geometry("er", &er_doc, [0.0, 0.0, 100.0, 100.0])
                .expect_err("untyped ER label boxes must fail")
                .contains("relationshipLabelBox")
        );

        let requirement = r#"<svg viewBox="0 0 120 40"><style>.reqLabelBox{fill:white}</style>
            <g class="edgeLabel"><rect class="background" width="110" height="30"/>
            <text x="5" y="20">&amp;lt;&amp;lt;satisfies&amp;gt;&amp;gt;</text></g></svg>"#;
        let requirement_doc =
            roxmltree::Document::parse(requirement).expect("legacy requirement XML");
        assert!(
            validate_key_geometry("requirement", &requirement_doc, [0.0, 0.0, 120.0, 40.0])
                .expect_err("untyped requirement label box must fail")
                .contains("reqLabelBox")
        );
        let encoded_only =
            requirement.replace("class=\"background\"", "class=\"background reqLabelBox\"");
        let encoded_doc =
            roxmltree::Document::parse(&encoded_only).expect("encoded requirement XML");
        assert!(
            validate_key_geometry("requirement", &encoded_doc, [0.0, 0.0, 120.0, 40.0])
                .expect_err("double-encoded requirement relation must fail")
                .contains("decoded <<stereotype>>")
        );

        let block = r#"<svg viewBox="-5 -45 29.48046875 90">
            <path marker-end="url(#arrow)" d="M9.74,-4L9.74,-3.333C9.74,-2.667,9.74,-1.333,9.74,-0.667C9.74,0,9.74,0,9.74,0L9.74,0"/></svg>"#;
        let block_doc = roxmltree::Document::parse(block).expect("legacy block XML");
        assert!(
            validate_key_geometry("block", &block_doc, [-5.0, -45.0, 29.48046875, 90.0])
                .expect_err("zero-length terminal cubic must fail")
                .contains("marker tangent")
        );
    }

    /// 关系解码不能只为 satisfies 写特例；同一通用路径覆盖其他合法 stereotype。
    #[test]
    fn requirement_relation_stereotypes_render_once_decoded() {
        let (_, template) = corpus::MERMAID_11_12_CORPUS
            .iter()
            .find(|(name, _)| *name == "requirement")
            .expect("requirement corpus entry");
        let mut session = Session::new().expect("initialize Mermaid");
        for relation in ["satisfies", "verifies", "refines"] {
            let source = template.replacen("satisfies", relation, 1);
            let svg = session
                .render(&source, false)
                .unwrap_or_else(|error| panic!("{relation} render failed: {error}"));
            validate_corpus_svg("requirement", &svg)
                .unwrap_or_else(|error| panic!("{relation} validation failed: {error}"));
            let doc = roxmltree::Document::parse(&svg).expect("requirement XML");
            let label = doc
                .descendants()
                .find(|node| node.tag_name().name() == "g" && has_class(*node, "edgeLabel"))
                .and_then(|label| {
                    label
                        .descendants()
                        .find(|node| node.tag_name().name() == "text")
                })
                .map(normalized_text)
                .expect("requirement relation text");
            assert_eq!(label, format!("<<{relation}>>"));
            assert!(!svg.contains("&amp;lt;"));
            assert!(!svg.contains("&amp;gt;"));
        }
    }

    /// 浏览器验收发现的两份旧 viewBox 必须同时被比例和关键几何检查拒绝。
    #[test]
    fn legacy_degenerate_viewboxes_are_rejected() {
        let timeline = r#"<svg viewBox="-50 -62.8 63150.9296875 119.2">
            <text font-size="4ex" x="31350.40234375" y="20">History</text>
            <g class="lineWrapper"><line x1="150" y1="168" x2="63450.8046875" y2="168"/></g>
            <line x1="295" y1="118" x2="295" y2="366"/>
            <line x1="495" y1="118" x2="495" y2="366"/>
        </svg>"#;
        let timeline_doc = roxmltree::Document::parse(timeline).expect("legacy timeline XML");
        let timeline_box = [-50.0, -62.8, 63150.9296875, 119.2];
        assert!(
            validate_key_geometry("timeline", &timeline_doc, timeline_box)
                .expect_err("old timeline geometry must fail")
                .contains("outside viewBox")
        );
        assert!(validate_corpus_svg("timeline", timeline)
            .expect_err("old timeline aspect must fail")
            .contains("aspect ratio"));

        let treemap = r#"<svg viewBox="-8 -20.8 10101.3046875 35.2">
            <g transform="translate(10,35)"><rect class="treemapLeaf" width="584" height="355"/></g>
            <g transform="translate(604,35)"><rect class="treemapLeaf" width="386" height="355"/></g>
        </svg>"#;
        let treemap_doc = roxmltree::Document::parse(treemap).expect("legacy treemap XML");
        let treemap_box = [-8.0, -20.8, 10101.3046875, 35.2];
        assert!(validate_key_geometry("treemap", &treemap_doc, treemap_box)
            .expect_err("old treemap geometry must fail")
            .contains("outside viewBox"));
        assert!(validate_corpus_svg("treemap", treemap)
            .expect_err("old treemap aspect must fail")
            .contains("aspect ratio"));
    }

    /// 浏览器失败的透明 treemap 使用健康 viewBox：结构样式与局部位图都必须独立拒绝它。
    #[test]
    fn legacy_transparent_treemap_is_rejected() {
        let treemap = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="2 -3.4 996 401.4" width="996" height="401.4">
            <g transform="translate(10,35)"><rect class="treemapLeaf" width="584" height="355" fill="transparent" stroke="transparent" fill-opacity="0.3"/></g>
            <g transform="translate(604,35)"><rect class="treemapLeaf" width="386" height="355" fill="transparent" stroke="transparent" fill-opacity="0.3"/></g>
            <text class="treemapLabel" x="292" y="177.5" style="text-anchor:middle;dominant-baseline:middle;font-size:38px;fill:#ffffff">Phones</text>
            <text class="treemapValue" x="292" y="198.5" style="text-anchor:middle;dominant-baseline:hanging;font-size:23px;fill:#ffffff">30</text>
            <text class="treemapLabel" x="797" y="177.5" style="text-anchor:middle;dominant-baseline:middle;font-size:38px;fill:black">Laptops</text>
            <text class="treemapValue" x="797" y="198.5" style="text-anchor:middle;dominant-baseline:hanging;font-size:23px;fill:black">20</text>
        </svg>"##;
        let doc = roxmltree::Document::parse(treemap).expect("legacy treemap XML");
        let view_box = [2.0, -3.4, 996.0, 401.4];
        assert!(validate_key_geometry("treemap", &doc, view_box)
            .expect_err("transparent fill must fail")
            .contains("non-transparent"));
        let (raster, raster_view_box) = rasterize_svg(treemap).expect("rasterize legacy treemap");
        assert!(validate_treemap_bitmap(&doc, raster_view_box, &raster)
            .expect_err("text pixels must not hide a transparent leaf")
            .contains("background pixel coverage"));
        assert!(validate_corpus_svg("treemap", treemap)
            .expect_err("complete validation must reject the browser failure")
            .contains("non-transparent"));
    }

    /// 旧纵向单列即使所有节点都在 viewBox 内，也必须被横向分支几何拒绝。
    #[test]
    fn legacy_vertical_mindmap_is_rejected() {
        let mindmap = r#"<svg viewBox="-10 -32.4 153.98722442421308 376.30584746813884">
            <g id="node_0" class="node mindmap-node section-root" transform="translate(80,93)"/>
            <g id="node_1" class="node mindmap-node section-0" transform="translate(80,180)"/>
            <g id="node_2" class="node mindmap-node section-0" transform="translate(75,260)"/>
            <g id="node_3" class="node mindmap-node section-1" transform="translate(85,340)"/>
        </svg>"#;
        let doc = roxmltree::Document::parse(mindmap).expect("legacy mindmap XML");
        let view_box = [-10.0, -32.4, 153.98722442421308, 376.30584746813884];
        assert!(validate_mindmap_geometry(&doc, view_box)
            .expect_err("vertical mindmap must fail")
            .contains("horizontal"));
    }

    /// 浅色 kanban 的 section-2 默认是白字 + 浅紫底（对比度不足 2:1），必须覆盖为深色标签；
    /// 深色主题不套用该覆盖（其分段底色较暗，保留 Mermaid 的浅色标签）。
    #[test]
    fn kanban_light_section_labels_have_readable_contrast() {
        let mut session = Session::new().expect("initialize Mermaid");
        let source = corpus::MERMAID_11_12_CORPUS
            .iter()
            .find(|(name, _)| *name == "kanban")
            .expect("kanban corpus")
            .1;
        let light = session.render(source, false).expect("light kanban");
        let rule = last_css_rule(&light, ".sections .cluster-label text");
        assert_eq!(css_property(rule, "fill"), "#333333");
        // section-2 底色为 hsl(270,100%,86.27%)，约合 #dcb8ff。
        assert!(
            contrast_ratio("#333333", "#dcb8ff") >= 4.5,
            "kanban 浅色列标题必须达到 WCAG AA"
        );

        let dark = session.render(source, true).expect("dark kanban");
        assert!(
            !dark.contains(".sections .cluster-label"),
            "深色主题不得套用浅色文字覆盖"
        );
    }

    /// 浅色 journey 的任务标签默认同样是白字 + 浅紫底（对比度接近 1:1），
    /// 必须与 kanban 一样落到 section 级深色文字覆盖。
    #[test]
    fn journey_light_task_labels_have_readable_contrast() {
        let mut session = Session::new().expect("initialize Mermaid");
        let source = corpus::MERMAID_11_12_CORPUS
            .iter()
            .find(|(name, _)| *name == "journey")
            .expect("journey corpus")
            .1;
        let light = session.render(source, false).expect("light journey");
        // journey 的任务文字是 `text.task`，段落条文字是 `text.journey-section`，
        // 都带 `fill="#fff"` 表现属性，必须用元素+类选择器覆盖。
        let rule = last_css_rule(&light, "text.task");
        assert_eq!(css_property(rule, "fill"), "#333333");
        let section_rule = last_css_rule(&light, "text.journey-section");
        assert_eq!(css_property(section_rule, "fill"), "#333333");
        assert!(
            contrast_ratio("#333333", "#dcb8ff") >= 4.5,
            "journey 浅色任务标签必须达到 WCAG AA"
        );
    }

    /// 内嵌 11.12.0 的全部受支持用户图型：同一 Session 逐类渲染，避免只测常见样本。
    /// 设置 VEIL_MERMAID_EVIDENCE_DIR 时同时输出本地 SVG 与浏览器并排材料。
    #[test]
    fn mermaid_11_12_corpus_renders_and_recovers() {
        let mut session = Session::new().expect("initialize Mermaid 11.12.0");
        let mut failures = Vec::new();
        let mut rendered = Vec::new();

        for &(name, source) in corpus::MERMAID_11_12_CORPUS {
            match session.render(source, false) {
                Ok(svg) => {
                    let lower = svg.to_ascii_lowercase();
                    if svg.len() < 200 || !lower.contains("<svg") || !lower.contains("</svg>") {
                        failures.push(format!("{name}: empty or incomplete SVG"));
                    } else if lower.contains("foreignobject") {
                        failures.push(format!("{name}: foreignObject is not supported by resvg"));
                    } else if ![
                        "<path",
                        "<rect",
                        "<circle",
                        "<ellipse",
                        "<polygon",
                        "<polyline",
                        "<line",
                    ]
                    .iter()
                    .any(|tag| lower.contains(tag))
                    {
                        failures.push(format!("{name}: no drawable SVG geometry"));
                    } else {
                        match validate_corpus_svg(name, &svg) {
                            Ok(check) => {
                                if matches!(name, "timeline" | "treemap") {
                                    println!(
                                        "{name}: viewBox={:?}, resvg={}x{}, non-transparent={}",
                                        check.view_box,
                                        check.bitmap_width,
                                        check.bitmap_height,
                                        check.non_transparent_pixels
                                    );
                                }
                                rendered.push((name, svg));
                            }
                            Err(error) => failures.push(format!("{name}: {error}")),
                        }
                    }
                }
                Err(error) => failures.push(format!("{name}: {error}")),
            }
        }

        // 深色主题：同一 corpus 全部图型逐个渲染，作为 F-011「16 图型深色补齐」证据。
        // 只校验成功、与浅色不同、可栅格化且非全透明；观感（深字压深底/裁切）另做目检。
        let mut dark_rendered = Vec::new();
        for &(name, source) in corpus::MERMAID_11_12_CORPUS {
            match session.render(source, true) {
                Ok(svg) => match validate_corpus_svg(name, &svg) {
                    Ok(check) => {
                        let light = rendered
                            .iter()
                            .find(|(n, _)| *n == name)
                            .map(|(_, s)| s)
                            .expect("light svg must precede dark");
                        if light == &svg {
                            failures.push(format!("{name}: dark SVG identical to light"));
                        } else if check.non_transparent_pixels == 0 {
                            failures.push(format!("{name}: dark SVG raster is fully transparent"));
                        } else {
                            dark_rendered.push((name, svg));
                        }
                    }
                    Err(error) => failures.push(format!("{name} (dark): {error}")),
                },
                Err(error) => failures.push(format!("{name} (dark): {error}")),
            }
        }

        if let Err(error) = session.render("graph TD\n  A -->", false) {
            assert!(!error.is_empty(), "syntax error must remain readable");
        } else {
            failures.push("invalid source unexpectedly rendered".into());
        }
        match session.render("sequenceDiagram\n  A->>B: recovered", false) {
            Ok(svg) if svg.contains("<svg") => {}
            Ok(_) => failures.push("recovery returned incomplete SVG".into()),
            Err(error) => failures.push(format!("recovery after syntax error failed: {error}")),
        }

        assert!(
            failures.is_empty(),
            "Mermaid corpus failures ({} / {}):\n{}",
            failures.len(),
            corpus::MERMAID_11_12_CORPUS.len(),
            failures.join("\n")
        );
        assert_eq!(rendered.len(), corpus::MERMAID_11_12_CORPUS.len());
        assert_eq!(
            dark_rendered.len(),
            corpus::MERMAID_11_12_CORPUS.len(),
            "每个 corpus 图型都必须产出可栅格化的深色 SVG"
        );

        let (_, mindmap_source) = corpus::MERMAID_11_12_CORPUS
            .iter()
            .find(|(name, _)| *name == "mindmap")
            .expect("mindmap corpus entry");
        let dark = session.render(mindmap_source, true).expect("dark mindmap");
        let light = &rendered
            .iter()
            .find(|(name, _)| *name == "mindmap")
            .expect("rendered mindmap")
            .1;
        for svg in [light, &dark] {
            assert!(
                svg.contains("aria-roledescription=\"mindmap\""),
                "mindmap structure marker"
            );
            assert!(svg.contains("<path") || svg.contains("<rect") || svg.contains("<ellipse"));
        }
        assert_ne!(
            *light, dark,
            "mindmap light/dark theme evidence must differ"
        );
        validate_mindmap_css_theme(light, &dark);
        let dark_check =
            validate_corpus_svg("mindmap", &dark).expect("dark mindmap geometry and pixels");
        println!(
            "mindmap: light viewBox={:?}, dark viewBox={:?}, dark resvg={}x{}",
            svg_view_box(&roxmltree::Document::parse(light).expect("light mindmap XML"))
                .expect("light mindmap viewBox"),
            dark_check.view_box,
            dark_check.bitmap_width,
            dark_check.bitmap_height
        );

        let (_, kanban_source) = corpus::MERMAID_11_12_CORPUS
            .iter()
            .find(|(name, _)| *name == "kanban")
            .expect("kanban corpus entry");
        let dark_kanban = session.render(kanban_source, true).expect("dark kanban");
        let kanban_check =
            validate_corpus_svg("kanban", &dark_kanban).expect("dark kanban geometry and pixels");
        println!(
            "kanban: dark viewBox={:?}, dark resvg={}x{}",
            kanban_check.view_box, kanban_check.bitmap_width, kanban_check.bitmap_height
        );

        if let Some(dir) = std::env::var_os("VEIL_MERMAID_EVIDENCE_DIR") {
            write_corpus_evidence(std::path::Path::new(&dir), &rendered, &dark_rendered);
        }
    }

    /// F-011：corpus 全部图型深色渲染成功、与浅色不同、可栅格化且非全透明。
    /// 对已确认 fill 规则稳定的图型追加深底对比度断言（journey 标题 / pie 图例 /
    /// packet 字段标签 / c4 Rel / radar 轴标签）；其余图型靠逐图目检（证据 PNG
    /// 已合成到应用深色底）留档。
    #[test]
    fn dark_corpus_renders_differs_and_rasterizes_opaque() {
        /// 深色主题正文底色（veil-ui `Theme::dark().bg` = GRAY_30）。
        const PAGE: &str = "#1e1e1e";
        let mut session = Session::new().expect("initialize Mermaid 11.12.0");
        let mut checked = 0usize;
        for &(name, source) in corpus::MERMAID_11_12_CORPUS {
            let light = session
                .render(source, false)
                .unwrap_or_else(|e| panic!("{name} light: {e}"));
            let dark = session
                .render(source, true)
                .unwrap_or_else(|e| panic!("{name} dark: {e}"));
            assert_ne!(dark, light, "{name}: 深色 SVG 必须与浅色不同");
            assert!(
                !dark.contains("<foreignObject"),
                "{name}: 深色 SVG 不得包含 resvg 无法栅格化的 foreignObject"
            );
            let check = validate_corpus_svg(name, &dark)
                .unwrap_or_else(|e| panic!("{name} 深色 SVG 校验失败: {e}"));
            assert!(
                check.non_transparent_pixels >= 16,
                "{name}: 深色 SVG 栅格化后近全透明（{} 像素）",
                check.non_transparent_pixels
            );
            match name {
                // Mermaid 的 journey 标题是 fill=""（无效值），resvg 会按黑色绘制；
                // 该图型限定覆盖保证标题在深底上为白色。
                "journey" => {
                    let fill = css_property(last_css_rule(&dark, "text[fill=\"\"]"), "fill");
                    let ratio = contrast_ratio_css(fill, PAGE);
                    assert!(
                        ratio >= 4.5,
                        "journey 标题对比度 {ratio:.2}（{fill} on {PAGE}）"
                    );
                }
                // pie 图例文字 fill 由 Mermaid dark 主题固定为浅色。
                "pie" => {
                    let fill = css_property(last_css_rule(&dark, ".legend text"), "fill");
                    let ratio = contrast_ratio_css(fill, PAGE);
                    assert!(
                        ratio >= 4.5,
                        "pie 图例对比度 {ratio:.2}（{fill} on {PAGE}）"
                    );
                }
                // packet 字段标签落在实心 packetBlock（fill #333）上。
                "packet" => {
                    let fill = css_property(last_css_rule(&dark, ".packetLabel"), "fill");
                    let ratio = contrast_ratio_css(fill, "#333333");
                    assert!(
                        ratio >= 4.5,
                        "packet 字段标签对比度 {ratio:.2}（{fill} on #333333）"
                    );
                }
                // c4 Rel 标签/连线由 Mermaid 固定写死 #444444；深色下按 aria 限定
                // 精确覆盖（本语料中 #444444 只出现在 Rel 元素上）。
                "c4context" => {
                    let fill = css_property(last_css_rule(&dark, "text[fill=\"#444444\"]"), "fill");
                    let ratio = contrast_ratio_css(fill, PAGE);
                    assert!(
                        ratio >= 4.5,
                        "c4 Rel 标签对比度 {ratio:.2}（{fill} on {PAGE}）"
                    );
                }
                // radar 轴标签原为 lightgrey（`color` 不影响 fill，实际继承 #ccc），
                // 压在半透明浅色网格上合成约 #585858；改为白字后对比度约 7:1。
                "radar" => {
                    let fill = css_property(last_css_rule(&dark, ".radarAxisLabel"), "fill");
                    let ratio = contrast_ratio_css(fill, "#585858");
                    assert!(
                        ratio >= 4.5,
                        "radar 轴标签对比度 {ratio:.2}（{fill} on 网格合成 #585858）"
                    );
                }
                _ => {}
            }
            checked += 1;
        }
        assert_eq!(checked, corpus::MERMAID_11_12_CORPUS.len());
    }

    /// 应用深色主题正文底色（veil-ui `Theme::dark().bg` = `GRAY_30` = `[30, 30, 30]`）。
    const DARK_EVIDENCE_BG: [u8; 3] = [30, 30, 30];

    /// 证据 PNG：`bg = None` 保持透明（浅色对照）；`Some(rgb)` 时把 resvg 的透明画布
    /// 合成到不透明底色上——深色证据必须与应用内真实叠底（#1e1e1e）一致，
    /// 否则在查看器里会被白底渲染，对比度结论失真。
    fn write_png_evidence(dir: &std::path::Path, name: &str, svg: &str, bg: Option<[u8; 3]>) {
        let (mut raster, _) = rasterize_svg(svg).expect("rasterize Mermaid evidence");
        if let Some(bg) = bg {
            for px in raster.pixmap.data_mut().as_chunks_mut::<4>().0 {
                let alpha = u32::from(px[3]);
                let inv = 255 - alpha;
                for (channel, base) in px.iter_mut().zip(bg) {
                    *channel = (u32::from(*channel) + u32::from(base) * inv / 255) as u8;
                }
                px[3] = 255;
            }
        }
        let png = raster
            .pixmap
            .encode_png()
            .expect("encode Mermaid evidence PNG");
        std::fs::write(dir.join(format!("{name}.png")), png).expect("write Mermaid evidence PNG");
    }

    fn write_corpus_evidence(
        dir: &std::path::Path,
        rendered: &[(&str, String)],
        dark_rendered: &[(&str, String)],
    ) {
        std::fs::create_dir_all(dir).expect("create Mermaid evidence directory");
        let mut html = String::from(
            "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><title>Veil Mermaid 11.12.0 corpus</title>\
             <style>body{font:16px system-ui;margin:24px;background:#eee;color:#222}section{background:white;margin:16px 0;padding:16px}\
             img{max-width:100%;height:auto;border:1px solid #ccc}.pair{display:grid;grid-template-columns:1fr 1fr;gap:16px}.dark{background:#1e1e1e}</style></head><body>\
             <h1>Mermaid 11.12.0 离线 corpus</h1><p>本地 SVG；本页不加载任何脚本或远程资源。</p>",
        );
        for (name, svg) in rendered {
            let filename = format!("{name}.svg");
            std::fs::write(dir.join(&filename), svg).expect("write corpus SVG");
            // 浅色证据保持透明画布；深色证据合成到应用深色底。
            write_png_evidence(dir, name, svg, None);
            html.push_str(&format!(
                "<section><h2>{name}</h2><p><a href=\"{filename}\">SVG</a> · <a href=\"{name}.png\">PNG</a></p><img src=\"{filename}\" alt=\"{name}\"></section>\n"
            ));
            // 深色证据：每个图型同时落盘 `<name>-dark.svg/png`（F-011 逐图目检输入）。
            let dark_name = format!("{name}-dark");
            let dark_filename = format!("{dark_name}.svg");
            if let Some((_, dark_svg)) = dark_rendered.iter().find(|(n, _)| *n == *name) {
                std::fs::write(dir.join(&dark_filename), dark_svg).expect("write dark corpus SVG");
                write_png_evidence(dir, &dark_name, dark_svg, Some(DARK_EVIDENCE_BG));
                html.push_str(&format!(
                    "<section><h2>{name} 深色</h2><p><a href=\"{dark_filename}\">Dark SVG</a> · <a href=\"{dark_name}.png\">Dark PNG</a></p><div class=\"dark\"><img src=\"{dark_filename}\" alt=\"{name} dark\"></div></section>\n"
                ));
            }
            if *name == "mindmap" {
                html.push_str(
                    "<section><h2>mindmap 主题并排</h2><p><a href=\"mindmap-dark.svg\">Dark SVG</a> · <a href=\"mindmap-dark.png\">Dark PNG</a></p><div class=\"pair\"><div><h3>Light</h3><img src=\"mindmap.svg\"></div>\
                     <div class=\"dark\"><h3>Dark</h3><img src=\"mindmap-dark.svg\"></div></div></section>\n",
                );
            }
        }
        html.push_str(
            "<section><h2>kanban 主题并排</h2><p><a href=\"kanban-dark.svg\">Dark SVG</a> · <a href=\"kanban-dark.png\">Dark PNG</a></p><div class=\"pair\"><div><h3>Light</h3><img src=\"kanban.svg\"></div>\
             <div class=\"dark\"><h3>Dark</h3><img src=\"kanban-dark.svg\"></div></div></section>\n",
        );
        html.push_str("<section><h2>明确不支持</h2><ul>");
        for (name, reason) in corpus::UNSUPPORTED_DIAGRAMS {
            html.push_str(&format!("<li>{name}: {reason}</li>"));
        }
        html.push_str("</ul></section></body></html>\n");
        std::fs::write(dir.join("index.html"), html).expect("write corpus browser index");
    }

    /// 有持续诊断价值的 viewBox 检查：路径可移植，且只在显式运行时写临时目录。
    #[test]
    #[ignore]
    fn mindmap_viewbox_debug() {
        let mut session = Session::new().expect("initialize Mermaid");
        let svg = session
            .render(
                "mindmap\n  root((中心))\n    子节点一\n      孙节点\n    子节点二",
                false,
            )
            .expect("should render");
        let path = std::env::temp_dir().join(format!("veil-mindmap-{}.svg", std::process::id()));
        std::fs::write(&path, &svg).expect("write diagnostic SVG");
        println!("SVG: {} ({} bytes)", path.display(), svg.len());
        if let Some(start) = svg.find("viewBox=") {
            let end = svg[start..].find('>').unwrap_or(svg.len() - start) + start;
            println!("{}", &svg[start..end]);
        }
    }
}
