//! 常驻渲染线程：QuickJS Runtime 非 Send（只能在创建线程内使用）+ pie 深递归
//! 在默认栈上溢出（spike 实测 0xc00000fd），因此整个引擎生命周期钉在 64 MiB
//! 专用线程内，调用方线程经 channel 请求/响应同步等待（trait 签名保持同步）。
//!
//! 看门狗：[`AtomicI64`] 存 deadline（UNIX 毫秒时间戳），interrupt handler 在
//! JS 执行中周期检查、超时返回 true 中断——死循环源码按期中断为 Err，进程不
//! 受影响；无任务时 deadline 归零，handler 恒 false。内存上限 64 MB（spike
//! 实测三类图全部通过）。
//!
//! 引擎初始化（Runtime + shim + bundle 编译求值，实测 ~300ms）只做一次；
//! 主题指纹变化时重跑 `M.initialize`（注入 fontFamily/fontSize/themeVariables）。
//! bundle 以源码嵌入、运行时编译：字节码 5.72MB > 源码 1.72MB，嵌入体积优先
//! （spike RESULTS.md §5），一次性冷启放 worker 线程，UI 无感。

use std::cell::RefCell;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rquickjs::{Context, Function, Runtime};
use veil_ui::MermaidTheme;

use crate::metrics::TextMeasurer;

const SHIM: &str = include_str!("../assets/dom-shim.js");
const BUNDLE: &str = include_str!("../assets/mermaid-tiny.esm.min.js");

/// QuickJS 堆内存上限（spike 验证 64MB 足够渲染 flowchart/sequence/pie）。
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
/// JS 侧栈上限（QuickJS 自己的检查；Rust 栈见线程栈大小）。
const JS_STACK_LIMIT: usize = 4 * 1024 * 1024;
/// worker 线程栈：pie 深递归需要（spike：默认栈 debug 构建必溢出）。
const THREAD_STACK: usize = 64 * 1024 * 1024;

/// worker → 调用方 ready 信号：`None` = 冷启中；`Some(_)` = 终态（可复用于
/// worker 重建后的二次置位，调用方只在 `None` 时阻塞等待）。
pub(crate) type ReadyState = Arc<(Mutex<Option<Result<(), String>>>, Condvar)>;

/// 主题参数快照：worker 在指纹变化时重新 initialize（spec「themeVariables 映射」）。
#[derive(Clone)]
pub(crate) struct ThemeSpec {
    pub families: &'static [&'static str],
    pub font_size: f32,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub accent: [u8; 3],
    pub muted: [u8; 3],
    pub dark: bool,
}

impl ThemeSpec {
    pub(crate) fn of(theme: &MermaidTheme) -> Self {
        Self {
            families: theme.families,
            font_size: theme.font_size,
            fg: theme.fg,
            bg: theme.bg,
            accent: theme.accent,
            muted: theme.muted,
            dark: theme.dark,
        }
    }

    /// 主题指纹：颜色 + 字号位型 + 明暗 + 字体栈指针（`'static` 数组指针恒定
    /// 即内容恒定，指针比较避免逐串哈希）。
    pub(crate) fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.fg.hash(&mut h);
        self.bg.hash(&mut h);
        self.accent.hash(&mut h);
        self.muted.hash(&mut h);
        self.font_size.to_bits().hash(&mut h);
        self.dark.hash(&mut h);
        (self.families.as_ptr() as usize).hash(&mut h);
        h.finish()
    }
}

/// 一次渲染请求；worker 完成后经 `reply` 回 `Err(String)`（已中文化的短消息）。
pub(crate) struct Job {
    pub source: Arc<str>,
    pub theme: ThemeSpec,
    pub deadline: Duration,
    pub reply: mpsc::SyncSender<Result<String, String>>,
}

/// 启动 worker 线程，返回请求通道。线程创建失败时 ready 置终态（首图快速
/// 报错而非挂起）；发送失败由调用方重建（send Err 即 worker 掉线信号）。
pub(crate) fn spawn_thread(flag: Arc<AtomicI64>, ready: ReadyState) -> mpsc::SyncSender<Job> {
    let (tx, rx) = mpsc::sync_channel(8);
    let thread_ready = Arc::clone(&ready);
    let spawned = std::thread::Builder::new()
        .name("mermaid-worker".into())
        .stack_size(THREAD_STACK)
        .spawn(move || worker_loop(rx, flag, thread_ready));
    if spawned.is_err() {
        let (lock, cv) = &*ready;
        let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            *g = Some(Err("渲染线程创建失败".into()));
            cv.notify_all();
        }
    }
    tx
}

fn worker_loop(rx: mpsc::Receiver<Job>, flag: Arc<AtomicI64>, ready: ReadyState) {
    let boot = boot_engine(&flag);
    {
        let (lock, cv) = &*ready;
        let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            *g = Some(match &boot {
                Ok(_) => Ok(()),
                Err(msg) => Err(msg.clone()),
            });
            cv.notify_all();
        }
    }
    match boot {
        Err(msg) => {
            // 引擎起不来：所有后续请求快速失败（不再触碰 runtime）。
            for job in rx.iter() {
                let _ = job.reply.send(Err(msg.clone()));
            }
        }
        Ok(mut engine) => {
            for job in rx.iter() {
                let out = engine.run_job(&job, &flag);
                let _ = job.reply.send(out);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 引擎（只在 worker 线程内触碰）
// ---------------------------------------------------------------------------

struct Engine {
    runtime: Runtime,
    ctx: Context,
    /// 已 initialize 的主题指纹：变化时重跑 initialize（避免每图重置配置）。
    theme_fp: Option<u64>,
}

fn boot_engine(flag: &Arc<AtomicI64>) -> Result<Engine, String> {
    let runtime = Runtime::new().map_err(|e| format!("QuickJS 运行时创建失败：{e}"))?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(JS_STACK_LIMIT);
    let f = Arc::clone(flag);
    runtime.set_interrupt_handler(Some(Box::new(move || {
        let d = f.load(Ordering::Relaxed);
        d > 0 && unix_millis() > d
    })));
    let ctx = Context::full(&runtime).map_err(|e| format!("QuickJS 上下文创建失败：{e}"))?;
    // 注入度量桥 + DOM shim（纯 JS，spike 已验证；宿主函数须先于 shim 求值存在）。
    ctx.with(|ctx| -> Result<(), rquickjs::Error> {
        let f = Function::new(ctx.clone(), measure_text_raw)?;
        ctx.globals().set("__measureTextRaw", f)?;
        ctx.eval::<(), _>(SHIM)?;
        Ok(())
    })
    .map_err(|e| format!("DOM shim 求值失败：{e}"))?;
    ctx.with(|ctx| -> Result<(), String> {
        let m = rquickjs::Module::declare(ctx.clone(), "mermaid-tiny", BUNDLE)
            .map_err(|e| format!("bundle 编译失败：{e}"))?;
        let (_evaluated, _promise) = m.eval().map_err(|e| format!("bundle 求值失败：{e}"))?;
        Ok(())
    })?;
    // 泵完模块求值与顶层微任务。
    pump_jobs(&runtime);
    let ready: bool = ctx
        .with(|ctx| {
            ctx.eval::<bool, _>(
                "typeof globalThis.mermaidTiny === 'object' && typeof globalThis.mermaidTiny.render === 'function'",
            )
        })
        .unwrap_or(false);
    if !ready {
        return Err("mermaid 引擎未就绪".into());
    }
    Ok(Engine {
        runtime,
        ctx,
        theme_fp: None,
    })
}

impl Engine {
    fn run_job(&mut self, job: &Job, flag: &AtomicI64) -> Result<String, String> {
        let deadline_ms = unix_millis().saturating_add(job.deadline.as_millis() as i64);
        flag.store(deadline_ms, Ordering::Relaxed);
        let out = self.render_inner(job, flag);
        flag.store(0, Ordering::Relaxed);
        out
    }

    fn render_inner(&mut self, job: &Job, flag: &AtomicI64) -> Result<String, String> {
        let fp = job.theme.fingerprint();
        if self.theme_fp != Some(fp) {
            install_measurer(&job.theme);
            self.eval(&init_script(&job.theme), flag)?;
            self.theme_fp = Some(fp);
        }
        self.eval(&render_script(&job.source), flag)?;
        self.wait_result(job)
    }

    /// eval 一段脚本并泵微任务；看门狗中断 → 统一「渲染超时」。
    fn eval(&self, script: &str, flag: &AtomicI64) -> Result<(), String> {
        self.ctx
            .with(|ctx| ctx.eval::<(), _>(script))
            .map_err(|e| eval_err_message(&e, flag))?;
        pump_jobs(&self.runtime);
        Ok(())
    }

    /// 渲染泵循环（spike render_case 模式）：微任务（execute_pending_job）与
    /// shim 虚拟时钟定时器（__pump）交替推进，直到 `__out`/`__err` 出现或超时。
    fn wait_result(&self, job: &Job) -> Result<String, String> {
        let started = Instant::now();
        loop {
            if started.elapsed() > job.deadline {
                return Err("渲染超时".into());
            }
            let done: bool = self
                .ctx
                .with(|ctx| {
                    ctx.eval::<bool, _>("globalThis.__out !== null || globalThis.__err !== null")
                        .unwrap_or(false)
                });
            if done {
                break;
            }
            if self.runtime.is_job_pending() {
                let _ = self.runtime.execute_pending_job();
                continue;
            }
            let pumped: i32 = self
                .ctx
                .with(|ctx| ctx.eval::<i32, _>("globalThis.__pump()"))
                .unwrap_or(0);
            if pumped == 0 {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        self.ctx.with(|ctx| -> Result<String, String> {
            let err: Option<String> = ctx.eval("globalThis.__err").unwrap_or(None);
            if let Some(e) = err.filter(|e| !e.is_empty()) {
                return Err(shorten(&e));
            }
            let svg: Option<String> = ctx
                .eval("globalThis.__out && globalThis.__out.svg ? globalThis.__out.svg : null")
                .map_err(|e| format!("读取渲染结果失败：{e}"))?;
            svg.ok_or_else(|| "渲染未产出 SVG".to_string())
        })
    }
}

fn pump_jobs(runtime: &Runtime) {
    while runtime.is_job_pending() {
        let _ = runtime.execute_pending_job();
    }
}

/// eval 失败分类：看门狗中断（interrupt 或 deadline 已过）→「渲染超时」；
/// 其余转发简短信息。零 panic：错误一律转 Err。
fn eval_err_message(e: &rquickjs::Error, flag: &AtomicI64) -> String {
    let d = flag.load(Ordering::Relaxed);
    let expired = d > 0 && unix_millis() > d;
    if expired || e.to_string().to_ascii_lowercase().contains("interrupt") {
        "渲染超时".to_string()
    } else {
        format!("脚本执行失败：{e}")
    }
}

// ---------------------------------------------------------------------------
// 渲染脚本（spike 验证过的 then-handler + 全局 __out/__err 模式）
// ---------------------------------------------------------------------------

/// `var M`（非 const）：initialize 随主题指纹变化重跑，const 全局重复声明会抛
/// SyntaxError。securityLevel:'loose' 跳过 DOMPurify（依赖浏览器 HTML 解析，
/// shim 不提供）；宿主侧以 veil-ui `sanitize_svg` 兜底。htmlLabels:false 必须
/// 全局设置（v12 的 flowchart.htmlLabels 读值路径不同，双保险均设）。
fn init_script(theme: &ThemeSpec) -> String {
    let fam = js_str(&font_family_css(theme.families));
    let theme_name = if theme.dark { "dark" } else { "base" };
    let (bg, fg, accent, muted) = (hex(theme.bg), hex(theme.fg), hex(theme.accent), hex(theme.muted));
    let size = theme.font_size.round().max(1.0);
    format!(
        "var M = globalThis.M || (globalThis.M = mermaidTiny.default || mermaidTiny);\n\
         M.initialize({{\n\
           startOnLoad: false,\n\
           securityLevel: 'loose',\n\
           logLevel: 'fatal',\n\
           theme: '{theme_name}',\n\
           fontFamily: \"{fam}\",\n\
           htmlLabels: false,\n\
           flowchart: {{ htmlLabels: false, useMaxWidth: true }},\n\
           sequence: {{ useMaxWidth: true }},\n\
           pie: {{ useMaxWidth: true }},\n\
           themeVariables: {{\n\
             fontSize: \"{size}px\",\n\
             fontFamily: \"{fam}\",\n\
             primaryColor: \"{bg}\",\n\
             primaryTextColor: \"{fg}\",\n\
             primaryBorderColor: \"{accent}\",\n\
             lineColor: \"{muted}\",\n\
             mainBkg: \"{bg}\",\n\
             clusterBkg: \"{bg}\",\n\
             clusterBorder: \"{muted}\",\n\
             edgeLabelBackground: \"{bg}\"\n\
           }}\n\
         }});"
    )
}

fn render_script(source: &str) -> String {
    format!(
        "globalThis.__out = null; globalThis.__err = null;\n\
         globalThis.__seq = (globalThis.__seq || 0) + 1;\n\
         var M = globalThis.M || (globalThis.M = mermaidTiny.default || mermaidTiny);\n\
         M.render(\"veil-\" + globalThis.__seq, \"{src}\").then(\n\
           (v) => {{ globalThis.__out = v; }},\n\
           (e) => {{ globalThis.__err = String((e && (e.stack || e.message)) || e); }},\n\
         );",
        src = js_str(source)
    )
}

fn font_family_css(families: &[&str]) -> String {
    families
        .iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// JS 双引号字符串转义：源码含换行/引号/反斜杠时仍为合法单行字面量（spike 版）。
fn js_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// 错误消息截断到 300 字符（字符边界安全）——错误卡直显，异常栈不刷屏。
fn shorten(s: &str) -> String {
    if s.len() <= 300 {
        return s.to_string();
    }
    let mut end = 300;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 度量注入（thread_local：度量器与 Runtime 同住 worker 线程）
// ---------------------------------------------------------------------------

thread_local! {
    /// 度量器随主题（字体栈）变化在 worker 线程内重建；thread_local 保证与
    /// Runtime 同生命周期且无跨线程访问。
    static MEASURE: RefCell<Option<TextMeasurer>> = const { RefCell::new(None) };
}

/// shim 度量桥：`__measureTextRaw(text,fontSize,weight,family) -> "w|h"`。
/// 度量器未就绪/借用冲突时回 "0|0"（布局退化但不 panic）。
fn measure_text_raw(text: String, font_size: f64, weight: f64, family: String) -> String {
    MEASURE.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return "0|0".to_string();
        };
        match slot.as_mut() {
            Some(m) => m.measure(&text, font_size, weight, &family),
            None => "0|0".to_string(),
        }
    })
}

fn install_measurer(theme: &ThemeSpec) {
    MEASURE.with(|slot| {
        *slot.borrow_mut() = Some(TextMeasurer::new(theme.families, theme.font_size.max(1.0)));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_str_escapes_quotes_and_newlines() {
        assert_eq!(js_str("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn fingerprint_separates_color_size_dark_and_families() {
        let base = ThemeSpec {
            families: &["A"],
            font_size: 16.0,
            fg: [10, 10, 10],
            bg: [250, 250, 250],
            accent: [0, 120, 212],
            muted: [120, 120, 120],
            dark: false,
        };
        let same = ThemeSpec {
            families: &["A"],
            ..base.clone()
        };
        assert_eq!(base.fingerprint(), same.fingerprint());
        let other_family = ThemeSpec {
            families: &["B"],
            ..base.clone()
        };
        assert_ne!(base.fingerprint(), other_family.fingerprint());
        let other_size = ThemeSpec {
            font_size: 18.0,
            ..base.clone()
        };
        assert_ne!(base.fingerprint(), other_size.fingerprint());
        let other_dark = ThemeSpec { dark: true, ..base.clone() };
        assert_ne!(base.fingerprint(), other_dark.fingerprint());
    }

    #[test]
    fn init_script_carries_family_and_colors() {
        let spec = ThemeSpec {
            families: &["Microsoft YaHei UI", "Segoe UI"],
            font_size: 16.0,
            fg: [10, 20, 30],
            bg: [250, 250, 250],
            accent: [0, 120, 212],
            muted: [120, 120, 120],
            dark: false,
        };
        let s = init_script(&spec);
        assert!(s.contains(r#"fontFamily: "\"Microsoft YaHei UI\", \"Segoe UI\"""#), "{s}");
        assert!(s.contains("primaryTextColor: \"#0a141e\""), "{s}");
        assert!(s.contains("theme: 'base'"), "{s}");
        assert!(s.contains("htmlLabels: false"), "{s}");
        let dark = ThemeSpec { dark: true, ..spec };
        assert!(init_script(&dark).contains("theme: 'dark'"));
    }

    #[test]
    fn render_script_escapes_source_and_uses_pump_globals() {
        let s = render_script("flowchart TD\n A[\"x\"] --> B");
        assert!(s.contains(r#"M.render("veil-" + globalThis.__seq, "flowchart TD\n A[\"x\"] --> B")"#), "{s}");
        assert!(s.contains("__out") && s.contains("__err"));
    }

    #[test]
    fn shorten_truncates_on_char_boundary() {
        assert_eq!(shorten("短"), "短");
        let long = "汉".repeat(200);
        let cut = shorten(&long);
        assert!(cut.len() <= 306 && cut.ends_with('…'));
        assert!(cut.starts_with("汉"));
    }
}
