//! 诊断内核（P0-1）：结构化事件环形缓冲 + 崩溃/诊断文本渲染。
//!
//! 纯逻辑、零 GUI、零新增依赖：事件记录与文本渲染全部 headless 可测。
//! 落盘与 panic hook 在产品层（`veil::diag`），本模块只负责内存取证与脱敏渲染。
//!
//! 隐私边界：事件 detail 与报告自由文本都经 `redact` 处理——换行压平、
//! 绝对路径替换为 `<path>`、超长截断；报告从不读取文档正文、剪贴板或配置值。
//! 路径识别不依赖前导分隔符：任何位置的 `file:`、盘符、UNC、用户目录与
//! POSIX 根目录起点都会一直吞到硬终止符（含空格，宁可过度替换）。

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

/// 单元测试串行锁（仅测试构建）：环形缓冲是进程级状态，`veil-ui` 的测试
/// 并行执行时，任何依赖事件内容的断言都必须与 `record_diag` 共用这把锁。
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

/// 获取测试锁；锁中毒时恢复内部守卫，避免二次 panic。
#[cfg(test)]
pub(crate) fn test_lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
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

/// 脱敏：把 `file:` URL 与本机绝对路径片段（`C:\...`、`C:/...`、`\\server\...`、
/// `\Users\...`、`/home/...`、`/Users/...` 等）替换成 `<path>`。
///
/// 起点在文本任意位置都生效，不要求前一个字符是分隔符；替换一直持续到硬终止符
/// （换行、引号、括号、中文标点或任意 CJK 字符）。ASCII 空格不是终止符，因此
/// `Program Files`、`Alice Smith` 这类带空格路径会被整体吞掉（宁可过度替换）。
/// 最后压平换行并截断到 256 字符（含省略号）。
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

/// 把 `file:` URL 与绝对路径片段替换为 `<path>`（保留其余文本与换行）。
fn replace_paths(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match path_end(&chars, i) {
            Some(end) if end > i => {
                out.push_str("<path>");
                i = end;
            }
            _ => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// 若 `chars[i]` 是路径/`file:` URL 起点，返回片段结束下标（不含）。
///
/// 不做词边界检查：中文前缀、冒号、全角括号之后同样识别。只有盘符与 `file:`
/// 需要排除「前一个字符是 ASCII 字母数字」的情形，避免把 `https://` 的 `s:/`
/// 或 `profile:` 之类的普通单词误判成路径。
fn path_end(chars: &[char], i: usize) -> Option<usize> {
    if let Some(end) = file_url_end(chars, i) {
        return Some(end);
    }
    if starts_absolute_path(chars, i) {
        return Some(scan_until_terminator(chars, i));
    }
    None
}

/// 是否为本机绝对路径起点：盘符、UNC、裸用户目录或 POSIX 根目录。
fn starts_absolute_path(chars: &[char], i: usize) -> bool {
    let c = chars[i];
    let at_word_start = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
    // 盘符：ASCII 字母 + ':' + '\\' 或 '/'（如 `C:\`、`d:/`）。
    if at_word_start
        && c.is_ascii_alphabetic()
        && chars.get(i + 1) == Some(&':')
        && matches!(chars.get(i + 2), Some('\\') | Some('/'))
    {
        return true;
    }
    // UNC：连续两个反斜杠（含 `\\?\C:\...` 与 `\\server\share\...`）。
    if c == '\\' && chars.get(i + 1) == Some(&'\\') {
        return true;
    }
    // 裸用户目录：`\Users\`、`\Documents and Settings\`（大小写不敏感）。
    if c == '\\'
        && (starts_with_ignore_ascii_case(&chars[i..], r"\Users\")
            || starts_with_ignore_ascii_case(&chars[i..], r"\Documents and Settings\"))
    {
        return true;
    }
    // POSIX 绝对路径起点。
    c == '/'
        && ["/home/", "/Users/", "/root/", "/tmp/"]
            .iter()
            .any(|prefix| starts_with_ignore_ascii_case(&chars[i..], prefix))
}

/// `file:` URL 的结束下标；`file:` 后必须真的跟着本机路径，否则不替换。
/// 覆盖 `file:///C:/...`、`file://C:\...`、`file:C:\...`、`file:///home/...`。
fn file_url_end(chars: &[char], i: usize) -> Option<usize> {
    const PREFIX: [char; 5] = ['f', 'i', 'l', 'e', ':'];
    if i + PREFIX.len() > chars.len() {
        return None;
    }
    // `profile:` 之类的单词不得被当作 `file:` 起点。
    if i > 0 && chars[i - 1].is_ascii_alphanumeric() {
        return None;
    }
    for (k, expected) in PREFIX.iter().enumerate() {
        if chars[i + k].to_ascii_lowercase() != *expected {
            return None;
        }
    }
    let mut j = i + PREFIX.len();
    let mut slashes = 0;
    while slashes < 3 && chars.get(j) == Some(&'/') {
        j += 1;
        slashes += 1;
    }
    let is_path = match chars.get(j).copied() {
        Some(c) => {
            let drive = c.is_ascii_alphabetic()
                && chars.get(j + 1) == Some(&':')
                && matches!(chars.get(j + 2), Some('\\') | Some('/'));
            let unc = c == '\\' && chars.get(j + 1) == Some(&'\\');
            let rooted = slashes > 0 && !c.is_whitespace() && !is_hard_terminator(c);
            drive || unc || rooted
        }
        None => false,
    };
    if !is_path {
        return None;
    }
    Some(scan_until_terminator(chars, j))
}

/// 从起点一直吞到硬终止符（不含）；ASCII 空格不是终止符。
fn scan_until_terminator(chars: &[char], start: usize) -> usize {
    let mut end = start;
    while end < chars.len() && !is_hard_terminator(chars[end]) {
        end += 1;
    }
    end
}

/// 硬终止符：换行、引号、括号、中文标点与任意 CJK 字符。
fn is_hard_terminator(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '"'
            | '\''
            | '<'
            | '>'
            | ')'
            | ']'
            | '}'
            | '。'
            | '，'
            | '；'
            | '：'
            | '、'
            | '（'
            | '）'
            | '！'
            | '？'
            | '《'
            | '》'
            | '…'
    ) || is_cjk(c)
}

/// CJK 及全角字符范围（含标点），作为路径的硬终止符。
fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x2E80..=0x2FFF
            | 0x3000..=0x303F
            | 0x3040..=0x30FF
            | 0x3100..=0x312F
            | 0x31A0..=0x31BF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFFEF
            | 0x20000..=0x3FFFF
    )
}

/// 大小写不敏感的 ASCII 前缀匹配（用于 `\Users\`、`/home/` 等）。
fn starts_with_ignore_ascii_case(chars: &[char], prefix: &str) -> bool {
    let mut k = 0;
    for expected in prefix.chars() {
        match chars.get(k) {
            Some(c) if c.eq_ignore_ascii_case(&expected) => k += 1,
            _ => return false,
        }
    }
    true
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
    fn guard() -> MutexGuard<'static, ()> {
        test_lock()
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
    fn redact_replaces_absolute_paths_at_any_position() {
        let _guard = guard();
        // 审查给出的旁路反例：起点前面是中文、冒号、全角括号或行首反斜杠。
        let cases = [
            (
                r"打开C:\Users\alice\Documents\secret.md失败",
                "打开<path>失败",
            ),
            (r"路径:C:\Users\alice\a.md", "路径:<path>"),
            ("打开file:///C:/Users/alice/a.md", "打开<path>"),
            (r"（C:\Users\alice\a.md）", "（<path>）"),
            (r"\Users\alice\a.md", "<path>"),
            (r"\\server\share\x", "<path>"),
            (r"\Documents and Settings\alice\a.md", "<path>"),
            (r"C:\Users\Alice Smith\a.md", "<path>"),
            (r"D:/work/a.md", "<path>"),
            (r"\\server\share\a.txt", "<path>"),
            (r"C:\Users\alice\secret\a.md", "<path>"),
            ("C:/Users/alice/secret/a.md", "<path>"),
            (r"\\?\C:\x", "<path>"),
            ("/home/alice/a.md", "<path>"),
            ("/Users/alice/a.md", "<path>"),
            ("/root/.config/x", "<path>"),
            ("/tmp/x", "<path>"),
            // file: URL 的各种写法（含盘符、UNC 与 Unix 根）。
            ("file:///C:/Users/alice/a.md", "<path>"),
            (r"file://C:\Users\alice\a.md", "<path>"),
            (r"file:C:\Users\alice\a.md", "<path>"),
            ("file:///Users/alice/a.md", "<path>"),
            ("FILE:///C:/Users/alice/a.md", "<path>"),
            // 带空格路径必须整体脱敏（不能只替换到第一个空格）。
            (r"C:\Users\Alice Smith\My Documents\a.md", "<path>"),
            ("C:/Users/Alice Smith/My Documents/a.md", "<path>"),
            ("file:///C:/Users/Alice Smith/My Documents/a.md", "<path>"),
            (r"\\server\share\Alice Smith\a.md", "<path>"),
        ];
        for (input, expected) in cases {
            assert_eq!(redact(input), expected, "输入：{input}");
        }

        // 所有路径样例都不得残留用户名、服务器名或盘符前缀。
        for input in [
            r"打开C:\Users\alice\Documents\secret.md失败",
            r"路径:C:\Users\alice\a.md",
            "打开file:///C:/Users/alice/a.md",
            r"（C:\Users\alice\a.md）",
            r"\Users\alice\a.md",
            r"\\server\share\x",
            r"C:\Users\Alice Smith\a.md",
            r"D:/work/a.md",
            r"\\server\share\a.txt",
        ] {
            let out = redact(input);
            assert!(
                !out.contains("alice") && !out.contains("Smith") && !out.contains("server"),
                "输出残留敏感片段：{out}（输入：{input}）"
            );
            assert!(
                !out.contains(r"C:\") && !out.contains("C:/") && !out.contains("D:/"),
                "输出残留盘符前缀：{out}（输入：{input}）"
            );
        }
    }

    #[test]
    fn redact_flattens_newlines_and_limits_length() {
        let _guard = guard();
        let mixed = redact("打开 C:\\Users\\alice\\secret\\a.md 失败\r\n下一行");
        assert!(mixed.contains("<path>"));
        assert!(!mixed.contains("alice") && !mixed.contains("secret"));
        assert!(
            !mixed.contains('\n') && !mixed.contains('\r'),
            "换行必须压平"
        );
        assert_eq!(redact(&"长".repeat(1000)).chars().count(), MAX_DETAIL_CHARS);
    }

    #[test]
    fn redact_keeps_urls_and_relative_paths() {
        let _guard = guard();
        // 普通 URL scheme 与相对路径不得被误判为绝对路径。
        assert_eq!(redact("https://example.com/a"), "https://example.com/a");
        assert_eq!(
            redact("http://example.com/path/to/a.md"),
            "http://example.com/path/to/a.md"
        );
        assert_eq!(redact("file: notes.txt"), "file: notes.txt");
        assert_eq!(redact("docs/readme.md"), "docs/readme.md");
        assert_eq!(redact("src/main.rs:12:3"), "src/main.rs:12:3");
    }

    #[test]
    fn diagnostics_render_is_deterministic_and_hides_user_paths() {
        let _guard = guard();
        clear();
        record("doc", "save ok");
        let extra = vec![
            ("崩溃报告数", "0".to_string()),
            ("崩溃报告目录", r"%APPDATA%\Veil\logs".to_string()),
            ("最近崩溃报告", "none".to_string()),
        ];
        let first = render_diagnostics("windows", "x86_64", &extra);
        let second = render_diagnostics("windows", "x86_64", &extra);
        let without_time = |text: &str| {
            text.lines()
                .filter(|line| !line.starts_with("时间: "))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(
            without_time(&first),
            without_time(&second),
            "同一状态连续两次导出除时间行外必须完全一致"
        );
        assert!(first.contains("崩溃报告目录"));
        assert!(first.contains("%APPDATA%") || first.contains("%VEIL_HOME%"));
        assert!(!first.contains("C:\\Users"), "不得出现用户绝对路径");
        assert!(!first.contains("C:/Users"), "不得出现用户绝对路径");
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
