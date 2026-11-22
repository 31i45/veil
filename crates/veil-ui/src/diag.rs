//! 诊断内核（P0-1）：结构化事件环形缓冲 + 崩溃/诊断文本渲染。
//!
//! 纯逻辑、零 GUI、零新增依赖：事件记录与文本渲染全部 headless 可测。
//! 落盘与 panic hook 在产品层（`veil::diag`），本模块只负责内存取证与脱敏渲染。
//!
//! 隐私边界：事件 detail 与报告自由文本都经 `redact` 处理——换行压平、
//! 绝对路径替换为 `<path>`、超长截断；报告从不读取文档正文、剪贴板或配置值。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

/// 产品名（崩溃报告与诊断导出共用）。
pub const PRODUCT: &str = "Veil";
/// 版本号（编译期取自 Cargo.toml）。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// 构建提交（build.rs 注入短哈希，脏工作区带 `-dirty`，git 不可用时为 `unknown`）。
pub const BUILD_COMMIT: &str = env!("VEIL_BUILD_COMMIT");

/// 环形缓冲容量：只保留最近 64 条事件。
pub const RING_CAPACITY: usize = 64;

/// 单条 detail 的字符上限（截断后含省略号）。
const MAX_DETAIL_CHARS: usize = 256;
/// 回溯文本的字节上限。
const MAX_BACKTRACE_BYTES: usize = 8 * 1024;

/// 一条结构化事件：序号单调递增，时间为进程启动/首次记录起的毫秒数。
#[derive(Clone, Debug)]
pub struct Event {
    pub seq: u64,
    pub elapsed_ms: u64,
    pub kind: &'static str,
    pub detail: String,
}

struct State {
    seq: u64,
    events: VecDeque<Event>,
}

static STATE: Mutex<State> = Mutex::new(State {
    seq: 0,
    events: VecDeque::new(),
});
/// 计时起点：首次调用时固定，之后所有事件共享同一原点，避免系统时钟回拨影响。
static START: OnceLock<Instant> = OnceLock::new();
/// DPI 缩放以 f32 位模式存放，读取路径无锁、不会 panic，可在 panic hook 内安全调用。
static DPI_BITS: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0f32 的位模式

/// 记录结构化事件；detail 会经过脱敏与截断（禁止换行、绝对路径替换为 `<path>`、
/// 上限 256 字符）。锁中毒时恢复内部数据，绝不 panic。
pub fn record(kind: &'static str, detail: impl AsRef<str>) {
    let detail = redact(detail.as_ref());
    let elapsed = elapsed_ms();
    let mut state = state();
    state.seq += 1;
    let seq = state.seq;
    if state.events.len() >= RING_CAPACITY {
        state.events.pop_front();
    }
    state.events.push_back(Event {
        seq,
        elapsed_ms: elapsed,
        kind,
        detail,
    });
}

/// 事件快照（按时间升序拷贝）；只读，不暴露内部锁。
pub fn events() -> Vec<Event> {
    state().events.iter().cloned().collect()
}

/// 线程安全的 DPI 缩放记录（默认 1.0），供崩溃报告使用。
/// 非有限值或非正值回退为 1.0，避免报告出现 NaN/负缩放。
pub fn set_dpi_scale(scale: f32) {
    let value = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    DPI_BITS.store(value.to_bits(), Ordering::Relaxed);
}

pub fn dpi_scale() -> f32 {
    f32::from_bits(DPI_BITS.load(Ordering::Relaxed))
}

/// 清空环形缓冲（仅测试/诊断自检使用）；序号计数不复位，保持进程内单调。
pub fn clear() {
    state().events.clear();
}

/// 渲染崩溃报告。字段白名单：产品/版本/构建提交/OS/架构/线程/时间/DPI/
/// panic 消息/位置/最近事件/回溯；禁止出现文档正文、剪贴板与完整路径。
pub fn render_crash(
    os: &str,
    arch: &str,
    thread: &str,
    message: &str,
    location: &str,
    backtrace: Option<&str>,
) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("=== Veil 崩溃报告 ===\n");
    push_field(&mut out, "产品", PRODUCT);
    push_field(&mut out, "版本", APP_VERSION);
    push_field(&mut out, "构建提交", BUILD_COMMIT);
    push_field(&mut out, "系统", &sanitize_line(os));
    push_field(&mut out, "架构", &sanitize_line(arch));
    push_field(&mut out, "线程", &sanitize_line(thread));
    push_field(&mut out, "时间", &crate::format_unix_ms_utc(unix_ms()));
    push_field(&mut out, "DPI 缩放", &format!("{:.2}", dpi_scale()));
    push_field(&mut out, "Panic 消息", &sanitize_line(message));
    push_field(&mut out, "位置", &sanitize_line(location));
    push_events(&mut out);
    if let Some(bt) = backtrace {
        out.push_str("\n回溯:\n");
        // 回溯保留行结构（便于阅读），但仍替换路径，并按字节边界截断到 8 KiB。
        out.push_str(&truncate_bytes(&replace_paths(bt), MAX_BACKTRACE_BYTES));
        out.push('\n');
    }
    out
}

/// 渲染用户导出的诊断文本：产品/版本/构建提交/OS/架构/时间/DPI/补充信息/最近事件。
/// 补充信息由调用方给出现成文本（配置摘要、功能开关、报告目录等），此处统一脱敏。
pub fn render_diagnostics(os: &str, arch: &str, extra: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("=== Veil 诊断信息 ===\n");
    push_field(&mut out, "产品", PRODUCT);
    push_field(&mut out, "版本", APP_VERSION);
    push_field(&mut out, "构建提交", BUILD_COMMIT);
    push_field(&mut out, "系统", &sanitize_line(os));
    push_field(&mut out, "架构", &sanitize_line(arch));
    push_field(&mut out, "时间", &crate::format_unix_ms_utc(unix_ms()));
    push_field(&mut out, "DPI 缩放", &format!("{:.2}", dpi_scale()));
    out.push_str("\n补充信息:\n");
    if extra.is_empty() {
        out.push_str("  (无)\n");
    } else {
        for (key, value) in extra {
            out.push_str(&format!("  {}: {}\n", sanitize_line(key), redact(value)));
        }
    }
    push_events(&mut out);
    out
}

/// 脱敏：把 `C:\...`、`\\?\...`、`/home/...` 等绝对路径片段替换成 `<path>`；
/// 去换行；截断到 256 字符（含省略号）。
pub fn redact(text: &str) -> String {
    truncate_chars(replace_paths(&flatten(text)), MAX_DETAIL_CHARS)
}

fn state() -> MutexGuard<'static, State> {
    // 锁中毒只说明此前有 panic，内部数据仍完整可读；恢复而不是二次 panic。
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn elapsed_ms() -> u64 {
    let start = START.get_or_init(Instant::now);
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn push_field(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

fn push_events(out: &mut String) {
    out.push_str("\n最近事件 (最多 ");
    out.push_str(&RING_CAPACITY.to_string());
    out.push_str(" 条):\n");
    let list = events();
    if list.is_empty() {
        out.push_str("  (无)\n");
        return;
    }
    for event in list.iter().take(RING_CAPACITY) {
        out.push_str(&format!(
            "  #{} +{}ms {}: {}\n",
            event.seq, event.elapsed_ms, event.kind, event.detail
        ));
    }
}

/// 压平换行/制表符：报告字段保持单行，避免伪造段落。
fn flatten(text: &str) -> String {
    text.chars()
        .map(|c| {
            if matches!(c, '\n' | '\r' | '\t') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn sanitize_line(text: &str) -> String {
    replace_paths(&flatten(text))
}

/// 把绝对路径片段替换为 `<path>`（保留其余文本与换行）。
fn replace_paths(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match path_end(&chars, i) {
            Some(end) => {
                out.push_str("<path>");
                i = end;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// 若 `chars[i]` 是绝对路径起点，返回路径片段结束下标（不含）。
/// 起点必须落在词边界上，避免把 URL scheme（`http://`）或相对路径误判为绝对路径。
fn path_end(chars: &[char], i: usize) -> Option<usize> {
    let boundary = i == 0 || is_delimiter(chars[i - 1]);
    if !boundary {
        return None;
    }
    let c = chars[i];
    let drive = c.is_ascii_alphabetic()
        && chars.get(i + 1) == Some(&':')
        && matches!(chars.get(i + 2), Some('\\') | Some('/'));
    let unc = c == '\\' && chars.get(i + 1) == Some(&'\\');
    let unix = c == '/'
        && chars.get(i + 1).is_some_and(|next| *next != '/')
        && !(i > 0 && chars[i - 1] == ':');
    if !(drive || unc || unix) {
        return None;
    }
    let mut end = i;
    while end < chars.len() && !is_delimiter(chars[end]) {
        end += 1;
    }
    Some(end)
}

fn is_delimiter(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\''
                | '`'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | ','
                | ';'
                | '|'
                | '='
        )
}

fn truncate_chars(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[backtrace truncated]", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全局环形缓冲/DPI 是进程级状态：测试串行执行，避免相互踩踏。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn guard() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn ring_buffer_keeps_newest_and_seq_increases() {
        let _guard = guard();
        clear();
        for i in 0..100 {
            record("test", format!("e{i}"));
        }
        let events = events();
        assert_eq!(events.len(), RING_CAPACITY, "超出容量只保留最近 64 条");
        let details: Vec<&str> = events.iter().map(|e| e.detail.as_str()).collect();
        assert_eq!(details.first().copied(), Some("e36"), "最旧的 36 条被淘汰");
        assert_eq!(details.last().copied(), Some("e99"));
        assert!(
            events.windows(2).all(|w| w[1].seq == w[0].seq + 1),
            "seq 必须严格递增"
        );
        assert!(events.iter().all(|e| e.kind == "test"));
        assert!(events.iter().all(|e| e.elapsed_ms <= elapsed_ms()));
    }

    #[test]
    fn redact_replaces_absolute_paths_and_limits_length() {
        let _guard = guard();
        assert_eq!(redact(r"C:\Users\alice\secret\a.md"), "<path>");
        assert_eq!(redact(r"\\?\C:\x"), "<path>");
        assert_eq!(redact("/home/alice/a.md"), "<path>");
        let mixed = redact("打开 C:\\Users\\alice\\secret\\a.md 失败\r\n下一行");
        assert!(mixed.contains("<path>"));
        assert!(!mixed.contains("alice") && !mixed.contains("secret"));
        assert!(
            !mixed.contains('\n') && !mixed.contains('\r'),
            "换行必须压平"
        );
        assert_eq!(redact(&"长".repeat(1000)).chars().count(), MAX_DETAIL_CHARS);
        // URL 不应被误判为 Unix 绝对路径
        assert_eq!(redact("https://example.com/a"), "https://example.com/a");
    }

    #[test]
    fn crash_and_diagnostics_render_required_fields_without_private_paths() {
        let _guard = guard();
        clear();
        record("doc", r"保存 C:\Users\alice\secret\a.md 失败");
        set_dpi_scale(1.25);
        let message = r"打开 C:\Users\alice\secret\a.md 时失败";
        let location = r"C:\Users\alice\secret\src\main.rs:12:3";
        let backtrace = "0: main\n at C:\\Users\\alice\\secret\\a.md\n";
        let report = render_crash(
            "windows",
            "x86_64",
            "main",
            message,
            location,
            Some(backtrace),
        );
        for required in [
            PRODUCT,
            APP_VERSION,
            BUILD_COMMIT,
            "windows",
            "x86_64",
            "main",
        ] {
            assert!(report.contains(required), "缺少必需字段：{required}");
        }
        assert!(report.contains("DPI 缩放: 1.25"));
        assert!(report.contains("<path>"), "路径必须被替换");
        assert!(!report.contains("alice") && !report.contains("secret"));
        // 报告只取白名单字段，任何文档正文样例都不得借道出现（正文从未被读取）。
        let doc_body_sample = "# 机密章节\n这是文档正文";
        assert!(!report.contains(doc_body_sample));
        assert!(!report.contains("机密章节") && !report.contains("文档正文"));

        let diagnostics = render_diagnostics(
            "windows",
            "x86_64",
            &[
                ("配置", "theme=dark".to_string()),
                ("路径", r"C:\Users\alice\secret\x".to_string()),
            ],
        );
        for required in [PRODUCT, APP_VERSION, BUILD_COMMIT, "windows", "x86_64"] {
            assert!(diagnostics.contains(required), "缺少必需字段：{required}");
        }
        assert!(diagnostics.contains("theme=dark"));
        assert!(!diagnostics.contains("alice") && !diagnostics.contains("secret"));
        assert!(!diagnostics.contains("机密章节") && !diagnostics.contains("文档正文"));
        set_dpi_scale(1.0);
    }

    #[test]
    fn dpi_scale_roundtrip() {
        let _guard = guard();
        set_dpi_scale(1.5);
        assert!((dpi_scale() - 1.5).abs() < f32::EPSILON);
        assert_eq!(dpi_scale(), 1.5);
        set_dpi_scale(f32::NAN);
        assert_eq!(dpi_scale(), 1.0, "非法值回退默认");
        set_dpi_scale(1.0);
    }
}
