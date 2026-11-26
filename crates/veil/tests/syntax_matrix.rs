#![cfg(windows)]
//! T5：语料驱动测试 harness。
//!
//! 读取仓库已提交的 `docs/qa/syntax-matrix.md`（163 条用例），自行解析用例结构，
//! 在 PARSE / HTML / PDF / PRINT / WYSIWYG / FOCUS / TYPEWRITER 表面执行断言，
//! 并把汇总写入 `docs/qa/report-2026-11-26.md`；HTML 金样写入/比对
//! `docs/qa/golden/html/<CASE>.html`（`UPDATE_GOLDEN=1` 时强制更新）。
//!
//! 约定：cwd = `crates/veil`，因此路径统一用 `../../docs/...`。存在 FAIL 时测试失败，
//! 但 `QA_ALLOW_FAIL=1` 允许生成完整报告后不 panic（用于首次取证/金样生成）。

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use veil_core::{BlockKind, Session};

const MATRIX: &str = "../../docs/qa/syntax-matrix.md";
const GOLDEN_DIR: &str = "../../docs/qa/golden/html";
const REPORT: &str = "../../docs/qa/report-2026-11-26.md";

// =============================================================================
// 用例模型与解析
// =============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Surface {
    Parse,
    Wysiwyg,
    Focus,
    Typewriter,
    Html,
    Pdf,
    Print,
}

impl Surface {
    fn name(self) -> &'static str {
        match self {
            Surface::Parse => "PARSE",
            Surface::Wysiwyg => "WYSIWYG",
            Surface::Focus => "FOCUS",
            Surface::Typewriter => "TYPEWRITER",
            Surface::Html => "HTML",
            Surface::Pdf => "PDF",
            Surface::Print => "PRINT",
        }
    }
}

const SURFACE_ORDER: [Surface; 7] = [
    Surface::Parse,
    Surface::Wysiwyg,
    Surface::Focus,
    Surface::Typewriter,
    Surface::Html,
    Surface::Pdf,
    Surface::Print,
];

const GROUPS: [&str; 10] = ["BLK", "INL", "LNK", "IMG", "TBL", "CODE", "EXT", "CJK", "EDGE", "SEC"];

const CHECK_KEYS: [&str; 7] = [
    "blocks",
    "kinds",
    "html_contains",
    "html_not_contains",
    "pdf_pages",
    "image_in_cell",
    "no_overlap",
];

const KIND_NAMES: [&str; 18] = [
    "Paragraph",
    "AtxHeading",
    "SetextHeading",
    "FencedCode",
    "IndentedCode",
    "Math",
    "Mermaid",
    "Table",
    "ListItem",
    "BlockQuote",
    "Admonition",
    "Callout",
    "ThematicBreak",
    "LinkRefDef",
    "FootnoteDef",
    "HtmlBlock",
    "Details",
    "FrontMatter",
];

#[allow(dead_code)]
struct Case {
    id: String,
    group: String,
    title: String,
    source: String,
    expect: String,
    surfaces: Option<Vec<Surface>>,
    na_reason: Option<String>,
    checks: Vec<(String, String)>,
    byte_opts: Vec<String>,
    line: usize,
}

impl Case {
    fn has(&self, s: Surface) -> bool {
        self.surfaces.as_ref().is_some_and(|v| v.contains(&s))
    }

    fn check_values<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.checks
            .iter()
            .filter(move |(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn check_int(&self, key: &str) -> Option<usize> {
        self.checks
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.parse().ok())
    }

    fn flag(&self, key: &str) -> bool {
        self.checks.iter().any(|(k, v)| k == key && v == "true")
    }

    /// 按 `- 字节：` 指令施加字节级变换，得到「落盘文件」的字节。
    fn encoded(&self) -> Vec<u8> {
        let mut text = self.source.clone();
        if self.byte_opts.iter().any(|b| b == "no-final-newline") {
            if let Some(t) = text.strip_suffix('\n') {
                text = t.to_string();
            }
        }
        if self.byte_opts.iter().any(|b| b == "crlf") {
            text = text.replace('\n', "\r\n");
        }
        let mut bytes = Vec::new();
        if self.byte_opts.iter().any(|b| b == "bom") {
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }
}

fn load_matrix() -> Vec<Case> {
    let text = std::fs::read_to_string(MATRIX).unwrap_or_else(|e| {
        panic!("无法读取 {MATRIX}：{e}（测试 cwd 应为 crates/veil）")
    });
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();

    let mut cases = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].starts_with("### CASE-") {
            let (case, next) = parse_case(&lines, i);
            cases.push(case);
            i = next;
        } else {
            i += 1;
        }
    }

    // 编号连续（每组从 001 开始，按文件出现顺序）与条数校验。
    let mut running: BTreeMap<&str, u32> = BTreeMap::new();
    for c in &cases {
        let slot = running.entry(c.group.as_str()).or_insert(0);
        let num = c
            .id
            .rsplit('-')
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or_else(|| panic!("{} 行 {}：无法解析编号", c.id, c.line));
        if num != *slot + 1 {
            panic!(
                "{} 行 {}：同组编号不连续（期望 {}，实际 {}）",
                c.id,
                c.line,
                *slot + 1,
                num
            );
        }
        *slot += 1;
    }
    let expected: [(&str, usize); 10] = [
        ("BLK", 35),
        ("INL", 20),
        ("LNK", 9),
        ("IMG", 11),
        ("TBL", 18),
        ("CODE", 10),
        ("EXT", 18),
        ("CJK", 15),
        ("EDGE", 15),
        ("SEC", 12),
    ];
    for (group, n) in expected {
        let got = running.get(group).copied().unwrap_or(0) as usize;
        if got != n {
            panic!("组 {group} 条数不符：契约 {n}，实际 {got}");
        }
    }
    if cases.len() != 163 {
        panic!("用例总数不符：契约 163，实际 {}", cases.len());
    }
    cases
}

fn parse_case(lines: &[&str], start: usize) -> (Case, usize) {
    let header = lines[start];
    let header_line = start + 1;
    let rest = header.strip_prefix("### CASE-").unwrap_or_else(|| {
        panic!("行 {header_line}：用例标题须为 `### CASE-...`")
    });
    let (tag, title) = match rest.split_once(' ') {
        Some((t, ti)) => (t, ti.trim().to_string()),
        None => (rest, String::new()),
    };
    if title.is_empty() {
        panic!("行 {header_line}：CASE 标题不得为空");
    }
    let (group, num_raw) = tag
        .split_once('-')
        .unwrap_or_else(|| panic!("行 {header_line}：CASE 编号须为 <组>-<三位编号>"));
    if !GROUPS.contains(&group) {
        panic!("行 {header_line}：未知组名 `{group}`");
    }
    if num_raw.len() != 3 || num_raw.parse::<u32>().is_err() {
        panic!("行 {header_line}：编号须为三位数字，实际 `{num_raw}`");
    }
    let id = format!("CASE-{tag}");

    let mut i = start + 1;

    // - 源：其后必须立即是围栏代码块。
    if i >= lines.len() || lines[i].trim_end() != "- 源：" {
        panic!(
            "{id} 行 {}：缺少 `- 源：`（或其前存在空行/其它字段）",
            i + 1
        );
    }
    i += 1;
    if i >= lines.len() {
        panic!("{id} 行 {}：`- 源：` 之后缺少围栏", i + 1);
    }
    let fence = lines[i];
    let ticks = fence.chars().take_while(|&c| c == '`').count();
    if ticks < 3 || &fence[ticks..] != "md" {
        panic!(
            "{id} 行 {}：围栏须为 ```` ```md ````（≥3 反引号，语言固定 md）",
            i + 1
        );
    }
    i += 1;
    let mut src_lines: Vec<&str> = Vec::new();
    let mut closed = false;
    while i < lines.len() {
        let l = lines[i];
        let is_close =
            l.chars().count() == ticks && l.chars().all(|c| c == '`');
        if is_close {
            closed = true;
            i += 1;
            break;
        }
        src_lines.push(l);
        i += 1;
    }
    if !closed {
        panic!("{id}：围栏未闭合（开启 {ticks} 个反引号，闭合须完全相等）");
    }
    let source = format!("{}\n", src_lines.join("\n"));

    let expect = read_field(lines, &mut i, "- 期望：", &id);
    let surface_raw = read_field(lines, &mut i, "- 表面：", &id);
    let (surfaces, na_reason) = parse_surfaces(&surface_raw, &id, i);

    let mut checks = Vec::new();
    let mut byte_opts = Vec::new();
    while i < lines.len() && lines[i].starts_with("- ") {
        let line = lines[i].trim_end();
        if let Some(v) = line.strip_prefix("- 检查：") {
            checks = parse_checks(v.trim(), &id, i + 1);
            i += 1;
        } else if let Some(v) = line.strip_prefix("- 字节：") {
            byte_opts = parse_bytes(v.trim(), &id, i + 1);
            i += 1;
        } else {
            break;
        }
    }
    if i < lines.len() && lines[i].trim_end().starts_with("- ") {
        panic!(
            "{id} 行 {}：未知或错序字段 `{}`",
            i + 1,
            lines[i].trim_end()
        );
    }

    (
        Case {
            id,
            group: group.to_string(),
            title,
            source,
            expect,
            surfaces,
            na_reason,
            checks,
            byte_opts,
            line: header_line,
        },
        i,
    )
}

fn read_field(lines: &[&str], i: &mut usize, prefix: &str, id: &str) -> String {
    if *i >= lines.len() {
        panic!("{id} 行 {}：缺少必填字段 `{prefix}`", *i + 1);
    }
    let line = lines[*i].trim_end();
    let Some(v) = line.strip_prefix(prefix) else {
        panic!("{id} 行 {}：期望 `{prefix}`，实际 `{line}`", *i + 1);
    };
    let value = v.trim().to_string();
    if value.is_empty() {
        panic!("{id} 行 {}：`{prefix}` 不得为空", *i + 1);
    }
    *i += 1;
    value
}

fn parse_surfaces(raw: &str, id: &str, at: usize) -> (Option<Vec<Surface>>, Option<String>) {
    if raw == "ALL" {
        return (
            Some(vec![
                Surface::Wysiwyg,
                Surface::Focus,
                Surface::Typewriter,
                Surface::Html,
                Surface::Pdf,
                Surface::Print,
            ]),
            None,
        );
    }
    if let Some(rest) = raw.strip_prefix("N/A") {
        let reason: String = rest
            .trim()
            .trim_matches(|c| matches!(c, '（' | '）' | '(' | ')' | ' '))
            .to_string();
        return (
            None,
            Some(if reason.is_empty() {
                "用例声明 N/A".to_string()
            } else {
                reason
            }),
        );
    }
    let mut out = Vec::new();
    for part in raw.split(',') {
        let p = part.trim();
        let s = match p {
            "WYSIWYG" => Surface::Wysiwyg,
            "FOCUS" => Surface::Focus,
            "TYPEWRITER" => Surface::Typewriter,
            "HTML" => Surface::Html,
            "PDF" => Surface::Pdf,
            "PRINT" => Surface::Print,
            _ => panic!("{id} 行 {at}：未知表面 `{p}`"),
        };
        if !out.contains(&s) {
            out.push(s);
        }
    }
    (Some(out), None)
}

fn parse_checks(raw: &str, id: &str, at: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if raw.is_empty() {
        return out;
    }
    for item in raw.split(';') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let Some((k, v)) = item.split_once('=') else {
            panic!("{id} 行 {at}：检查项 `{item}` 缺少 `=`");
        };
        let key = k.trim();
        let value = v.trim();
        if !CHECK_KEYS.contains(&key) {
            panic!("{id} 行 {at}：未知检查 key `{key}`");
        }
        if value.is_empty() {
            panic!("{id} 行 {at}：检查 key `{key}` 的值不得为空");
        }
        match key {
            "blocks" | "pdf_pages" => {
                value.parse::<usize>().unwrap_or_else(|_| {
                    panic!("{id} 行 {at}：`{key}` 须为整数，实际 `{value}`")
                });
            }
            "kinds" => {
                for name in value.split(',') {
                    if !KIND_NAMES.contains(&name) {
                        panic!("{id} 行 {at}：未知块类型名 `{name}`");
                    }
                }
            }
            "image_in_cell" | "no_overlap" => {
                if value != "true" {
                    panic!("{id} 行 {at}：`{key}` 只接受 true");
                }
            }
            _ => {}
        }
        out.push((key.to_string(), value.to_string()));
    }
    out
}

fn parse_bytes(raw: &str, id: &str, at: usize) -> Vec<String> {
    let mut out = Vec::new();
    for tok in raw.split(',') {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        if !matches!(t, "bom" | "crlf" | "no-final-newline") {
            panic!("{id} 行 {at}：未知字节指令 `{t}`");
        }
        out.push(t.to_string());
    }
    if out.is_empty() {
        panic!("{id} 行 {at}：`- 字节：` 不得为空");
    }
    out
}

// =============================================================================
// 断言结果与报告模型
// =============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    NotRun,
}

struct Row {
    case: String,
    surface: &'static str,
    status: Status,
    detail: String,
}

impl Row {
    fn new(case: &Case, surface: Surface, problems: Vec<String>, not_run: Option<String>) -> Self {
        let (status, detail) = match not_run {
            Some(reason) => (Status::NotRun, reason),
            None if problems.is_empty() => (Status::Pass, String::new()),
            None => (Status::Fail, problems.join("；")),
        };
        Self {
            case: case.id.clone(),
            surface: surface.name(),
            status,
            detail,
        }
    }
}

fn snippet(s: &str, limit: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= limit {
            out.push('…');
            break;
        }
        match c {
            '\n' => out.push('⏎'),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

fn body_snippet(html: &str) -> String {
    match html.split_once("<body>") {
        Some((_, body)) => snippet(body.trim_start(), 300),
        None => snippet(html, 300),
    }
}

fn around(hay: &str, needle: &str) -> String {
    match hay.find(needle) {
        Some(pos) => {
            let from = pos.saturating_sub(60);
            let mut out = String::new();
            for (i, c) in hay[from..].chars().enumerate() {
                if i >= 300 {
                    out.push('…');
                    break;
                }
                out.push(if c == '\n' { '⏎' } else { c });
            }
            out
        }
        None => body_snippet(hay),
    }
}

fn diff_snippet(old: &str, new: &str) -> String {
    let pos = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let from = pos.saturating_sub(20);
    snippet(&new.chars().skip(from).collect::<String>(), 300)
}

// =============================================================================
// 断言实现
// =============================================================================

fn kind_name(k: &BlockKind) -> &'static str {
    match k {
        BlockKind::Paragraph => "Paragraph",
        BlockKind::AtxHeading(_) => "AtxHeading",
        BlockKind::SetextHeading(_) => "SetextHeading",
        BlockKind::FencedCode(_) => "FencedCode",
        BlockKind::IndentedCode => "IndentedCode",
        BlockKind::Math => "Math",
        BlockKind::Mermaid => "Mermaid",
        BlockKind::Table => "Table",
        BlockKind::ListItem { .. } => "ListItem",
        BlockKind::BlockQuote => "BlockQuote",
        BlockKind::Admonition(_) => "Admonition",
        BlockKind::Callout(_) => "Callout",
        BlockKind::ThematicBreak => "ThematicBreak",
        BlockKind::LinkRefDef => "LinkRefDef",
        BlockKind::FootnoteDef => "FootnoteDef",
        BlockKind::HtmlBlock => "HtmlBlock",
        BlockKind::Details { .. } => "Details",
        BlockKind::FrontMatter(_) => "FrontMatter",
    }
}

struct UiRes {
    wysiwyg: Vec<String>,
    focus: Vec<String>,
    typewriter: Vec<String>,
    overlap: Vec<String>,
}

fn ui_checks(app: &mut veil_ui::qa::QaApp) -> UiRes {
    app.render();
    let blocks = app.blocks();
    let total = app.total_h();

    let mut overlap = Vec::new();
    for pair in blocks.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if b.top < a.top + a.height {
            overlap.push(format!(
                "相邻块重叠：块底 {:.2} > 下一块顶 {:.2}（容差 0）",
                a.top + a.height,
                b.top
            ));
        }
    }
    let mut wysiwyg = overlap.clone();
    for (i, b) in blocks.iter().enumerate() {
        if !b.top.is_finite() || !b.height.is_finite() {
            wysiwyg.push(format!("第 {i} 块几何非有限"));
            continue;
        }
        if b.top < -0.5 {
            wysiwyg.push(format!("第 {i} 块越过文档顶：top={:.2}", b.top));
        }
        // 可见块必须有正高度：图片占位、表格、代码围栏等一旦塌成 0 高，
        // 屏幕表现就是“内容凭空消失/大段空白”，属于跨表面分叉。
        // 折叠 `<details>` 的内部块按设计高度为 0，不算缺陷。
        if b.height <= 0.0 && !b.details_hidden {
            wysiwyg.push(format!(
                "第 {i} 块高度为 {:.2}（不可见）：kind={}",
                b.height,
                kind_name(&b.kind)
            ));
        }
        if total > 0.0 && b.top + b.height > total + 0.5 {
            wysiwyg.push(format!(
                "第 {i} 块越过文档底：bottom={:.2} > total={:.2}",
                b.top + b.height,
                total
            ));
        }
    }

    let base_kinds: Vec<&'static str> = blocks.iter().map(|b| kind_name(&b.kind)).collect();
    let base_text = app.text();
    let base_len = blocks.len();

    app.toggle_focus();
    app.render();
    let focus_blocks = app.blocks();
    let focus_kinds: Vec<&'static str> =
        focus_blocks.iter().map(|b| kind_name(&b.kind)).collect();
    let mut focus = Vec::new();
    if focus_kinds != base_kinds || focus_blocks.len() != base_len {
        focus.push(format!(
            "专注模式改变块序列：{:?} → {:?}",
            base_kinds, focus_kinds
        ));
    }
    if app.text() != base_text {
        focus.push("专注模式改变了源文本（应仅为视觉开关）".to_string());
    }
    app.toggle_focus();

    app.set_caret_middle();
    app.render();
    app.toggle_typewriter();
    app.render();
    app.scroll_to_caret();
    app.render();
    let c = app.caret();
    let mut typewriter = Vec::new();
    let visible = c.y >= c.viewport_y - 1.0
        && c.y + c.h <= c.viewport_y + c.viewport_h + 1.0;
    if !visible {
        typewriter.push(format!(
            "打字机模式光标不在视口：y={:.1} h={:.1}，视口 [{:.1},{:.1}]",
            c.y,
            c.h,
            c.viewport_y,
            c.viewport_y + c.viewport_h
        ));
    }
    if c.max_scroll <= 0.0 {
        if c.scroll != 0.0 {
            typewriter.push(format!("短文档不应发生位移，scroll={:.2}", c.scroll));
        }
    } else {
        let center = c.y + c.h / 2.0;
        let lo = c.viewport_y + c.viewport_h * 0.25;
        let hi = c.viewport_y + c.viewport_h * 0.60;
        if center < lo || center > hi {
            typewriter.push(format!(
                "光标中心 {:.1} 不在视口稳定带 [{:.1},{:.1}]",
                center, lo, hi
            ));
        }
        if c.scroll < -0.01 || c.scroll > c.max_scroll + 0.01 {
            typewriter.push(format!(
                "滚动越界：scroll={:.2}，max={:.2}",
                c.scroll, c.max_scroll
            ));
        }
    }

    UiRes {
        wysiwyg,
        focus,
        typewriter,
        overlap,
    }
}

fn golden_path(id: &str) -> PathBuf {
    Path::new(GOLDEN_DIR).join(format!("{id}.html"))
}

fn compare_golden(case: &Case, html: &str, problems: &mut Vec<String>) {
    let path = golden_path(&case.id);
    let update = std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1");
    if update || !path.exists() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, html) {
            problems.push(format!("金样写入失败：{e}"));
        }
        return;
    }
    match std::fs::read_to_string(&path) {
        Ok(old) if old == html => {}
        Ok(old) => problems.push(format!(
            "金样不一致（{}）：实际从 {} 起不同：`{}`",
            path.display(),
            old.chars()
                .zip(html.chars())
                .take_while(|(a, b)| a == b)
                .count(),
            diff_snippet(&old, html)
        )),
        Err(e) => problems.push(format!("金样读取失败：{e}")),
    }
}

/// HTML 检查里可作为「PDF 关键文本」的纯文本片段。
///
/// 只取含中日韩字符的片段：`html_contains` 里也有属性/协议类 ASCII 片段
/// （`checked`、`data:image/png` 等），它们不是版面可读文本，不能作为 PDF
/// 文本断言；ASCII 正文文本（`Veil`、`2026`）不强制，避免把属性歧义带进 PDF。
fn plain_key(v: &str) -> bool {
    v.chars().count() >= 2
        && v.chars().any(is_cjk)
        && v.chars().all(|c| {
            !matches!(
                c,
                '<' | '>' | '=' | '"' | '\'' | '&' | ';' | ':' | '/' | '\\' | '`'
            )
        })
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        | 0x3040..=0x30FF | 0xAC00..=0xD7AF)
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 断言口径（D4）：参与属性归一的开始标签。
///
/// 这些标签在导出时会附加 `id` / `style` / `open` 等属性，契约字面串只写标签名。
const ATTR_TAGS: [&str; 23] = [
    "h1", "h2", "h3", "h4", "h5", "h6", "details", "summary", "th", "td", "table", "thead",
    "tbody", "tr", "ul", "ol", "li", "p", "pre", "code", "figure", "blockquote", "mark",
];

/// 若 `s`（以 `<` 开头）是待归一的开始标签，返回标签名长度。
fn match_attr_tag(s: &str) -> Option<usize> {
    let rest = s.strip_prefix('<')?;
    for tag in ATTR_TAGS {
        if let Some(tail) = rest.strip_prefix(tag) {
            let boundary = tail
                .chars()
                .next()
                .is_some_and(|c| c == '>' || c == '/' || c.is_whitespace());
            if boundary {
                return Some(tag.len());
            }
        }
    }
    None
}

/// 返回开始标签结束符 `>` 的字节下标；属性值中的引号内 `>` 不计。
fn find_tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '>' => return Some(i),
                _ => {}
            },
        }
    }
    None
}

/// 去掉指定开始标签上的属性（保留标签名与 `>`）。
///
/// 文本内容与结束标签不受影响；`<h1 id="x">` → `<h1>`、`<details open>` → `<details>`、
/// `<th style="…">` → `<th>`。片段在属性中间结束时（如 `<details open`）只保留标签名。
fn strip_tag_attrs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let Some(pos) = rest.find('<') else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        match match_attr_tag(after) {
            Some(name_len) => {
                out.push('<');
                out.push_str(&after[1..1 + name_len]);
                match find_tag_end(after) {
                    Some(gt) => {
                        out.push('>');
                        rest = &after[gt + 1..];
                    }
                    None => return out,
                }
            }
            None => {
                out.push('<');
                rest = &after[1..];
            }
        }
    }
}

/// `<body>…</body>` 之间的正文；缺失 `<body>` 时退回全文。
fn body_html(html: &str) -> &str {
    match html.find("<body>") {
        Some(pos) => {
            let start = pos + "<body>".len();
            let end = html[start..]
                .find("</body>")
                .map(|i| start + i)
                .unwrap_or(html.len());
            &html[start..end]
        }
        None => html,
    }
}

// =============================================================================
// 图片夹具
// =============================================================================

/// 1×1 透明 PNG（导出侧只嗅探文件头判定 MIME，无需真实解码）。
const TINY_PNG: [u8; 67] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
    0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
    0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
    0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
    0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// 图片类用例的真实临时目录：`docs/` 为文档目录，`assets/` 与根下 `assets/`
/// 分别覆盖正常相对路径和 `../assets` 越界目标；`empty.png` 为 0 字节。
fn image_fixture_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("veil-syntax-matrix-{}", std::process::id()));
        let base = root.join("docs");
        std::fs::create_dir_all(base.join("assets")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        for path in [
            base.join("a.png"),
            base.join("assets").join("a.png"),
            root.join("assets").join("a.png"),
        ] {
            std::fs::write(path, TINY_PNG).unwrap();
        }
        std::fs::write(base.join("empty.png"), b"").unwrap();
        base
    })
}

/// 需要真实图片文件的用例：`IMG` 全组、表格内/贴邻图片、0 字节与越界边界。
fn image_case_dir(id: &str) -> Option<&'static Path> {
    let wants = id.starts_with("CASE-IMG-")
        || matches!(
            id,
            "CASE-TBL-007" | "CASE-TBL-012" | "CASE-TBL-013" | "CASE-EDGE-013" | "CASE-EDGE-014"
        );
    wants.then(image_fixture_dir)
}

// =============================================================================
// 主测试
// =============================================================================

#[test]
fn syntax_matrix_surfaces() {
    let cases = load_matrix();
    let mut rows: Vec<Row> = Vec::new();

    for case in &cases {
        // ---- 源字节 → 文本（与产品打开路径一致：BOM 由 decode 剥离） ----
        let bytes = case.encoded();
        let text = match veil_ui::decode(&bytes) {
            Ok((t, _)) => t,
            Err(e) => {
                rows.push(Row::new(case, Surface::Parse, vec![format!("源字节无法解码：{e}")], None));
                continue;
            }
        };

        // ---- PARSE ----
        let mut parse_problems = Vec::new();
        let parsed = catch_unwind(AssertUnwindSafe(|| Session::open(&text)));
        match parsed {
            Err(_) => parse_problems.push("Session::open panic".to_string()),
            Ok(session) => {
                let blocks = session.blocks();
                if let Some(want) = case.check_int("blocks") {
                    if blocks.len() != want {
                        parse_problems.push(format!(
                            "blocks 期望 {want}，实际 {}（kind 序列 {:?}）",
                            blocks.len(),
                            blocks
                                .iter()
                                .map(|(_, k, _)| kind_name(k))
                                .collect::<Vec<_>>()
                        ));
                    }
                }
                for want in case.check_values("kinds") {
                    let expected: Vec<&str> = want.split(',').collect();
                    let actual: Vec<&'static str> =
                        blocks.iter().map(|(_, k, _)| kind_name(k)).collect();
                    if expected != actual {
                        parse_problems.push(format!(
                            "kinds 期望 [{}]，实际 [{}]",
                            expected.join(","),
                            actual.join(",")
                        ));
                    }
                }
            }
        }
        rows.push(Row::new(case, Surface::Parse, parse_problems, None));

        if let Some(reason) = &case.na_reason {
            for s in SURFACE_ORDER.iter().skip(1) {
                rows.push(Row::new(case, *s, Vec::new(), Some(reason.clone())));
            }
            continue;
        }

        // ---- 一次 headless App 供 UI 与 PRINT 共用 ----
        let need_ui = case.has(Surface::Wysiwyg) || case.has(Surface::Focus) || case.has(Surface::Typewriter);
        let need_print = case.has(Surface::Print);
        let mut app = if need_ui || need_print {
            match catch_unwind(AssertUnwindSafe(|| veil_ui::qa::QaApp::new(&text))) {
                Ok(a) => Some(a),
                Err(_) => {
                    for s in [Surface::Wysiwyg, Surface::Focus, Surface::Typewriter, Surface::Print] {
                        if case.has(s) {
                            rows.push(Row::new(
                                case,
                                s,
                                vec!["headless App 构造 panic".to_string()],
                                None,
                            ));
                        }
                    }
                    None
                }
            }
        } else {
            None
        };

        let mut overlap: Option<Vec<String>> = None;
        if need_ui {
            match app.as_mut() {
                None => {}
                Some(app) => match catch_unwind(AssertUnwindSafe(|| ui_checks(app))) {
                    Ok(res) => {
                        overlap = Some(res.overlap);
                        if case.has(Surface::Wysiwyg) {
                            rows.push(Row::new(case, Surface::Wysiwyg, res.wysiwyg, None));
                        }
                        if case.has(Surface::Focus) {
                            rows.push(Row::new(case, Surface::Focus, res.focus, None));
                        }
                        if case.has(Surface::Typewriter) {
                            rows.push(Row::new(case, Surface::Typewriter, res.typewriter, None));
                        }
                    }
                    Err(_) => {
                        for s in [Surface::Wysiwyg, Surface::Focus, Surface::Typewriter] {
                            if case.has(s) {
                                rows.push(Row::new(
                                    case,
                                    s,
                                    vec!["UI 布局断言 panic".to_string()],
                                    None,
                                ));
                            }
                        }
                    }
                },
            }
        }

        // ---- HTML ----
        if case.has(Surface::Html) {
            let mermaid_svg = case
                .check_values("html_contains")
                .any(|v| v.contains("<svg"));
            if mermaid_svg {
                rows.push(Row::new(
                    case,
                    Surface::Html,
                    Vec::new(),
                    Some(
                        "harness 未注入 Mermaid 渲染器：html_of_doc(mermaids=None) 会按产品降级为源码围栏，<svg 断言不可得"
                            .to_string(),
                    ),
                ));
            } else {
                match catch_unwind(AssertUnwindSafe(|| {
                    veil::qa::html_of_doc(&text, image_case_dir(&case.id))
                })) {
                    Err(_) => rows.push(Row::new(
                        case,
                        Surface::Html,
                        vec!["HTML 导出 panic".to_string()],
                        None,
                    )),
                    Ok(Err(e)) => rows.push(Row::new(
                        case,
                        Surface::Html,
                        vec![format!("HTML 导出入口不可用：{e}")],
                        None,
                    )),
                    Ok(Ok(html)) => {
                        let mut problems = Vec::new();
                        // 口径（D4）：比较前对正文与期望片段做开始标签属性归一，
                        // 使 `<h1 id=…>` ≡ `<h1>`、`<details open>` ≡ `<details>`、
                        // `<th style=…>` ≡ `<th>`；文本内容与结束标签不动。
                        let norm_html = strip_tag_attrs(&html);
                        let body = body_html(&html);
                        let body_norm = strip_tag_attrs(body);
                        for want in case.check_values("html_contains") {
                            // 原文命中（属性类断言如 class=/id=/alt= 保持严格）或归一后命中。
                            if !html.contains(want) && !norm_html.contains(&strip_tag_attrs(want)) {
                                problems.push(format!(
                                    "缺少片段 `{}`；实际正文片段：`{}`",
                                    snippet(want, 120),
                                    body_snippet(&html)
                                ));
                            }
                        }
                        for banned in case.check_values("html_not_contains") {
                            // 口径（D4）：仅在 <body>…</body> 内检查，避免 head 里的
                            // <style>/<title> 被误判为正文泄漏。
                            let banned_norm = strip_tag_attrs(banned);
                            if body.contains(banned) || body_norm.contains(&banned_norm) {
                                problems.push(format!(
                                    "出现禁止片段 `{}`（正文范围内）；上下文：`{}`",
                                    snippet(banned, 120),
                                    around(&html, banned)
                                ));
                            }
                        }
                        if html.contains("<script") {
                            problems.push(format!(
                                "通用不变量失败：出现未转义 `<script`；上下文：`{}`",
                                around(&html, "<script")
                            ));
                        }
                        let source_has_fence = case.source.contains("```");
                        if !source_has_fence && html.contains("```") {
                            problems.push(format!(
                                "通用不变量失败：导出泄漏围栏标记 ```；上下文：`{}`",
                                around(&html, "```")
                            ));
                        }
                        if case.flag("image_in_cell") {
                            let cell_img = html
                                .split("<td")
                                .skip(1)
                                .any(|cell| cell.split("</td>").next().is_some_and(|c| c.contains("<img")));
                            if !cell_img {
                                problems.push(
                                    "image_in_cell：未找到任何包含 <img 的 <td>…</td>".to_string(),
                                );
                            }
                            if !html.contains("td img,th img{max-height:2.4em}") {
                                problems.push(
                                    "image_in_cell：CSS 缺少 `td img,th img{max-height:2.4em}`"
                                        .to_string(),
                                );
                            }
                        }
                        compare_golden(case, &html, &mut problems);
                        rows.push(Row::new(case, Surface::Html, problems, None));
                    }
                }
            }
        }

        // ---- PDF ----
        if case.has(Surface::Pdf) {
            if !veil::qa::pdf_available() {
                rows.push(Row::new(
                    case,
                    Surface::Pdf,
                    Vec::new(),
                    Some("未找到 CJK 字体（pdf::find_cjk_font 返回 None），PDF 导出入口不可运行".to_string()),
                ));
            } else {
                match catch_unwind(AssertUnwindSafe(|| veil::qa::pdf_of_doc(&text))) {
                    Err(_) => rows.push(Row::new(
                        case,
                        Surface::Pdf,
                        vec!["PDF 导出 panic".to_string()],
                        None,
                    )),
                    Ok(Err(e)) => rows.push(Row::new(
                        case,
                        Surface::Pdf,
                        vec![format!("PDF 导出失败：{e}")],
                        None,
                    )),
                    Ok(Ok(out)) => {
                        let mut problems = Vec::new();
                        if let Some(want) = case.check_int("pdf_pages") {
                            if out.page_count != want {
                                problems.push(format!(
                                    "pdf_pages 期望 {want}，实际 {}",
                                    out.page_count
                                ));
                            }
                        }
                        if out.bytes_len == 0 {
                            problems.push("PDF 字节为空".to_string());
                        }
                        let pdf_text = norm(&out.text);
                        for want in case.check_values("html_contains") {
                            if !plain_key(want) {
                                continue;
                            }
                            if !pdf_text.contains(&norm(want)) {
                                problems.push(format!(
                                    "PDF 版面文本缺少 `{}`；实际文本片段：`{}`",
                                    snippet(want, 120),
                                    snippet(&out.text, 300)
                                ));
                            }
                        }
                        if case.flag("no_overlap") {
                            match &overlap {
                                Some(overlap) if overlap.is_empty() => {}
                                Some(overlap) => problems.extend(overlap.iter().cloned()),
                                None => problems.push(
                                    "no_overlap 需 headless UI 块盒，但 UI 布局不可用".to_string(),
                                ),
                            }
                        }
                        rows.push(Row::new(case, Surface::Pdf, problems, None));
                    }
                }
            }
        }

        // ---- PRINT（仅分页模型；物理打印统一 NOT RUN，见报告） ----
        if case.has(Surface::Print) {
            match app.as_mut() {
                None => {}
                Some(app) => {
                    let res = catch_unwind(AssertUnwindSafe(|| app.print_model(1240, 1754)));
                    match res {
                        Err(_) => rows.push(Row::new(
                            case,
                            Surface::Print,
                            vec!["打印模型 panic".to_string()],
                            None,
                        )),
                        Ok(pm) => {
                            let mut problems = Vec::new();
                            if pm.pages == 0 {
                                problems.push("分页结果 0 页".to_string());
                            }
                            if pm.breaks.len() != pm.pages {
                                problems.push(format!(
                                    "页数 {} 与断点数 {} 不一致",
                                    pm.pages,
                                    pm.breaks.len()
                                ));
                            }
                            if pm.breaks.first().copied() != Some(0.0) {
                                problems.push("页首断点不是 0".to_string());
                            }
                            for pair in pm.breaks.windows(2) {
                                if pair[1] <= pair[0] {
                                    problems.push(format!(
                                        "分页断点非递增：{:.2} → {:.2}",
                                        pair[0], pair[1]
                                    ));
                                    break;
                                }
                            }
                            if let Some(last) = pm.breaks.last() {
                                if *last > pm.total_h + 1.0 {
                                    problems.push(format!(
                                        "最后页首 {:.2} 超出文档高度 {:.2}",
                                        last, pm.total_h
                                    ));
                                }
                            }
                            rows.push(Row::new(case, Surface::Print, problems, None));
                        }
                    }
                }
            }
        }
    }

    write_report(&rows, cases.len());
    let fails = rows.iter().filter(|r| r.status == Status::Fail).count();
    if fails > 0 && std::env::var("QA_ALLOW_FAIL").as_deref() != Ok("1") {
        panic!("{fails} 条断言失败；完整报告见 {REPORT}");
    }
}

// =============================================================================
// 解析器自检（缺字段 / 未知检查 key 必须 panic 且带 CASE ID 与行号）
// =============================================================================

fn panic_message(err: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "<非字符串 panic>".into()
    }
}

#[test]
fn parser_rejects_unknown_check_key_with_case_id_and_line() {
    let lines = [
        "### CASE-BLK-001 标题",
        "- 源：",
        "```md",
        "正文",
        "```",
        "- 期望：可渲染",
        "- 表面：ALL",
        "- 检查：bogus=1",
    ];
    let err = match catch_unwind(|| parse_case(&lines, 0)) {
        Ok(_) => panic!("未知 key 必须 panic"),
        Err(e) => e,
    };
    let msg = panic_message(err);
    assert!(msg.contains("CASE-BLK-001"), "panic 应含 CASE ID：{msg}");
    assert!(msg.contains("行 8"), "panic 应含行号：{msg}");
}

#[test]
fn parser_rejects_missing_required_field_with_case_id_and_line() {
    let lines = [
        "### CASE-BLK-002 标题",
        "- 源：",
        "```md",
        "正文",
        "```",
        "- 表面：ALL",
    ];
    let err = match catch_unwind(|| parse_case(&lines, 0)) {
        Ok(_) => panic!("缺 期望 必须 panic"),
        Err(e) => e,
    };
    let msg = panic_message(err);
    assert!(msg.contains("CASE-BLK-002"), "panic 应含 CASE ID：{msg}");
    assert!(msg.contains("行 6"), "panic 应含行号：{msg}");
}

#[test]
fn parser_rejects_source_not_followed_by_fence() {
    let lines = [
        "### CASE-BLK-003 标题",
        "- 源：",
        "",
        "```md",
        "正文",
        "```",
        "- 期望：x",
        "- 表面：ALL",
    ];
    let err = match catch_unwind(|| parse_case(&lines, 0)) {
        Ok(_) => panic!("源后空行必须 panic"),
        Err(e) => e,
    };
    let msg = panic_message(err);
    assert!(msg.contains("CASE-BLK-003"), "panic 应含 CASE ID：{msg}");
}

// =============================================================================
// 报告
// =============================================================================

fn escape_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn write_report(rows: &[Row], total_cases: usize) {
    let mut counts: BTreeMap<&'static str, [usize; 3]> = BTreeMap::new();
    for s in SURFACE_ORDER {
        counts.insert(s.name(), [0, 0, 0]);
    }
    for r in rows {
        let c = counts.entry(r.surface).or_insert([0, 0, 0]);
        match r.status {
            Status::Pass => c[0] += 1,
            Status::Fail => c[1] += 1,
            Status::NotRun => c[2] += 1,
        }
    }

    let html_ran = counts["HTML"][0] + counts["HTML"][1];
    let mut out = String::new();
    out.push_str("# 语法矩阵测试报告（2026-11-26）\n\n");
    out.push_str(&format!(
        "- 语料：`docs/qa/syntax-matrix.md`（契约 163 条，解析校验通过 163）\n- 用例总数：{total_cases}\n- 金样：`docs/qa/golden/html/<CASE>.html`（{html_ran} 份；`UPDATE_GOLDEN=1` 时更新）\n- 命令：`cargo test -p veil --test syntax_matrix`\n\n"
    ));

    out.push_str("## 按表面统计\n\n");
    out.push_str("| 表面 | PASS | FAIL | NOT RUN |\n|---|---:|---:|---:|\n");
    let mut totals = [0usize; 3];
    for s in SURFACE_ORDER {
        let c = counts[s.name()];
        for i in 0..3 {
            totals[i] += c[i];
        }
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            s.name(),
            c[0],
            c[1],
            c[2]
        ));
    }
    out.push_str(&format!(
        "| 合计 | {} | {} | {} |\n\n",
        totals[0], totals[1], totals[2]
    ));

    out.push_str("## FAIL 明细\n\n");
    let fails: Vec<&Row> = rows.iter().filter(|r| r.status == Status::Fail).collect();
    if fails.is_empty() {
        out.push_str("无。\n\n");
    } else {
        out.push_str("| CASE | 表面 | 期望 / 实际片段（≤300 字符） |\n|---|---|---|\n");
        for r in fails {
            let detail = r
                .detail
                .chars()
                .take(600)
                .collect::<String>();
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                r.case,
                r.surface,
                escape_cell(&detail)
            ));
        }
        out.push('\n');
    }

    out.push_str("## NOT RUN 原因\n\n");
    let notrun: Vec<&Row> = rows.iter().filter(|r| r.status == Status::NotRun).collect();
    if notrun.is_empty() {
        out.push_str("无。\n\n");
    } else {
        for r in notrun {
            out.push_str(&format!(
                "- `{}` / {}：{}\n",
                r.case,
                r.surface,
                escape_cell(&r.detail)
            ));
        }
        out.push('\n');
    }

    out.push_str("## 降级与边界说明\n\n");
    out.push_str("- 断言口径（D4）：`html_contains`/`html_not_contains` 比较前对正文与期望片段做开始标签属性归一（h1–h6、details、summary、th、td、table、thead、tbody、tr、ul、ol、li、p、pre、code、figure、blockquote、mark），使 `<h1 id=…>` ≡ `<h1>`、`<details open>` ≡ `<details>`、`<th style=…>` ≡ `<th>`；属性类原文片段（`class=`、`id=`、`alt=` 等）仍按原文严格命中，纯文本片段不归一。\n");
    out.push_str("- `html_not_contains` 仅在 `<body>…</body>` 范围内检查，head 内的 `<style>`/`<title>` 不计为正文泄漏。\n");
    out.push_str("- 金样比较保持原文严格，不受上述归一影响。\n");
    out.push_str("- `no_overlap`：PDF 版面未暴露块级包围盒，改用 headless UI 的块几何（`top`/`height`，容差 0）判断相邻块不相交；该断言记在 PDF 表面。\n");
    out.push_str("- PDF 关键文本：PDF 字节流中文本为字形 CID，无法直接检索 Unicode；关键文本取自同一次 `layout_document` 的 `DrawOp::Text`（与 `render_pages` 同一版面数据）。\n");
    out.push_str("- PRINT：仅断言打印分页模型（页数/断点单调/首断点为 0）；物理打印需要 GDI 设备上下文，全部记为 NOT RUN。\n");
    out.push_str("- HTML 中的 Mermaid：harness 不启动 QuickJS 离线引擎，`mermaids=None` 按产品既定策略降级为源码围栏，故 `<svg` 断言记 NOT RUN（不伪造通过）。\n");
    out.push_str("- 金样首次缺失时自动写入（引导），之后普通运行全文比对。\n");

    if let Some(dir) = Path::new(REPORT).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(REPORT, out) {
        eprintln!("报告写入失败（{REPORT}）：{e}");
    }
}
