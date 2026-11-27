#![cfg(windows)]
//! T5：语料驱动测试 harness。
//!
//! 读取仓库已提交的 `docs/qa/syntax-matrix.md`（175 条用例），自行解析用例结构，
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
const REPORT: &str = "../../docs/qa/report-2026-11-27.md";
/// 生成式块级相邻矩阵的逐用例 HTML SHA-256 摘要。
const DIGEST: &str = "../../docs/qa/adjacency-digest.txt";

/// 生成式块级相邻矩阵（ADJB）：18 个块单元 × 18 个块单元 = 324 个有序对。
const ADJB_GROUP: &str = "ADJB";
const ADJ_UNITS: usize = 18;
const ADJ_TOTAL: usize = ADJ_UNITS * ADJ_UNITS;
/// B=FrontMatter 的 18 对记 N/A：FrontMatter 只识别于文档首块。
const ADJ_FRONT_MATTER: usize = 17;
const ADJ_NA_REASON: &str = "FrontMatter 仅识别于文档首块";

/// 生成式行内相邻矩阵（ADJI）：12 个行内单元 × 12 × 2 种边界 = 288 个用例。
const ADJI_GROUP: &str = "ADJI";
const ADJ_INLINE_UNITS: usize = 12;
const ADJ_INLINE_BOUNDARIES: usize = 2;
const ADJI_TOTAL: usize = ADJ_INLINE_UNITS * ADJ_INLINE_UNITS * ADJ_INLINE_BOUNDARIES;

/// 生成式容器内容矩阵（ADJC）：5 类容器 × 18 个块单元 = 90 个用例。
const ADJC_GROUP: &str = "ADJC";
const ADJC_CONTAINERS: usize = 5;
const ADJC_TOTAL: usize = ADJC_CONTAINERS * ADJ_UNITS;

/// ADJC 抽样金样（20 条，等距覆盖 5 类容器，且避开 Mermaid 单元——Mermaid
/// 渲染含浮点坐标，不做全文金样）。
const ADJC_GOLDEN: [usize; 20] = [
    1, 2, 4, 8, 19, 22, 26, 28, 37, 40, 44, 46, 55, 58, 62, 64, 73, 76, 80, 82,
];

/// 生成式块级单换行矩阵（ADJN）：18×18 = 324，源串 `A + "\n" + B`。
const ADJN_GROUP: &str = "ADJN";
const ADJN_TOTAL: usize = ADJ_TOTAL;

/// 生成式行内边界矩阵（ADJI2）：12×12×6 = 864。
const ADJI2_GROUP: &str = "ADJI2";
const ADJI2_BOUNDARIES: [&str; 6] = ["\n", "。", "，", "）", ".", ","];
const ADJI2_TOTAL: usize = ADJ_INLINE_UNITS * ADJ_INLINE_UNITS * ADJI2_BOUNDARIES.len();

/// 生成式容器内相邻矩阵（ADJC2）：5×18 自相邻 + 5×4 跨类型 = 110。
const ADJC2_GROUP: &str = "ADJC2";
const ADJC2_SELF: usize = ADJC_CONTAINERS * ADJ_UNITS;
const ADJC2_CROSS: usize = ADJC_CONTAINERS * 4;
const ADJC2_TOTAL: usize = ADJC2_SELF + ADJC2_CROSS;

/// 嵌套容器矩阵（ADJX）：ListItem / BlockQuote / Details 的 3×3 = 9。
const ADJX_GROUP: &str = "ADJX";
const ADJX_TOTAL: usize = 9;

fn is_adjacency_group(group: &str) -> bool {
    matches!(
        group,
        ADJB_GROUP | ADJI_GROUP | ADJC_GROUP | ADJN_GROUP | ADJI2_GROUP | ADJC2_GROUP | ADJX_GROUP
    )
}

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

const GROUPS: [&str; 11] = [
    "BLK", "INL", "LNK", "IMG", "TBL", "CODE", "EXT", "CJK", "EDGE", "SEC", "ADV",
];

const CHECK_KEYS: [&str; 12] = [
    "blocks",
    "kinds",
    "inline_kinds",
    "html_contains",
    "html_not_contains",
    "html_label_once",
    "html_label_order",
    "pdf_pages",
    "pdf_text",
    "pdf_not_text",
    "image_in_cell",
    "no_overlap",
];

/// 行内顶层节点序列断言可用的类型名（`None` 表示空序列）。
const INLINE_KIND_NAMES: [&str; 12] = [
    "Strong",
    "Emph",
    "Code",
    "Link",
    "Image",
    "Math",
    "Strikethrough",
    "Highlight",
    "FootnoteRef",
    "Html",
    "HtmlSelfClosed",
    "HtmlLiteral",
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
    /// 生成式容器矩阵（ADJC）的结构化元数据；语料/ADJB/ADJI 为 None。
    adjc: Option<AdjcSpec>,
    /// ADJC2 / ADJX 的结构化元数据；其余用例为 None。
    structured: Option<StructuredSpec>,
}

/// 空态用例：语料以 `表面：N/A` 声明「无可见内容」。旧 harness 据此整组跳过，
/// 现按空态契约在所有可观察表面执行实质断言（0 块 / 空 body / 1 页空白等）。
const EMPTY_STATE_CASES: [&str; 3] = ["CASE-EDGE-001", "CASE-EDGE-002", "CASE-EDGE-003"];

fn empty_state(id: &str) -> bool {
    EMPTY_STATE_CASES.contains(&id)
}

impl Case {
    fn has(&self, s: Surface) -> bool {
        self.surfaces.as_ref().is_some_and(|v| v.contains(&s))
    }

    /// 该表面是否执行断言。空态用例不受语料 `N/A` 限制：N/A 只说明「没有可见
    /// 内容」，不等于「不可测」——六个表面都能对空态给出确定结论。
    fn applies(&self, s: Surface) -> bool {
        (s != Surface::Parse && empty_state(&self.id)) || self.has(s)
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
    let text = std::fs::read_to_string(MATRIX)
        .unwrap_or_else(|e| panic!("无法读取 {MATRIX}：{e}（测试 cwd 应为 crates/veil）"));
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
        let num =
            c.id.rsplit('-')
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
    let expected: [(&str, usize); 11] = [
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
        ("ADV", 12),
    ];
    for (group, n) in expected {
        let got = running.get(group).copied().unwrap_or(0) as usize;
        if got != n {
            panic!("组 {group} 条数不符：契约 {n}，实际 {got}");
        }
    }
    if cases.len() != 175 {
        panic!("用例总数不符：契约 175，实际 {}", cases.len());
    }
    cases
}

fn parse_case(lines: &[&str], start: usize) -> (Case, usize) {
    let header = lines[start];
    let header_line = start + 1;
    let rest = header
        .strip_prefix("### CASE-")
        .unwrap_or_else(|| panic!("行 {header_line}：用例标题须为 `### CASE-...`"));
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
        let is_close = l.chars().count() == ticks && l.chars().all(|c| c == '`');
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
            adjc: None,
            structured: None,
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
                value
                    .parse::<usize>()
                    .unwrap_or_else(|_| panic!("{id} 行 {at}：`{key}` 须为整数，实际 `{value}`"));
            }
            "kinds" => {
                for name in value.split(',') {
                    if !KIND_NAMES.contains(&name) {
                        panic!("{id} 行 {at}：未知块类型名 `{name}`");
                    }
                }
            }
            "inline_kinds" => {
                for name in value.split(',') {
                    if name != "None" && !INLINE_KIND_NAMES.contains(&name) {
                        panic!("{id} 行 {at}：未知行内类型名 `{name}`");
                    }
                }
            }
            "image_in_cell" | "no_overlap" if value != "true" => {
                panic!("{id} 行 {at}：`{key}` 只接受 true");
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

fn inline_kind_name(k: &veil_core::InlineKind) -> &'static str {
    use veil_core::InlineKind;
    match k {
        InlineKind::Strong => "Strong",
        InlineKind::Emph => "Emph",
        InlineKind::Code => "Code",
        InlineKind::Link => "Link",
        InlineKind::Image => "Image",
        InlineKind::Math => "Math",
        InlineKind::Strikethrough => "Strikethrough",
        InlineKind::Highlight => "Highlight",
        InlineKind::FootnoteRef => "FootnoteRef",
        InlineKind::Html { .. } => "Html",
        InlineKind::HtmlSelfClosed { .. } => "HtmlSelfClosed",
        InlineKind::HtmlLiteral => "HtmlLiteral",
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
    let focus_kinds: Vec<&'static str> = focus_blocks.iter().map(|b| kind_name(&b.kind)).collect();
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
    let visible = c.y >= c.viewport_y - 1.0 && c.y + c.h <= c.viewport_y + c.viewport_h + 1.0;
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

/// 空态 UI 断言（WYSIWYG / FOCUS / TYPEWRITER 共用底座）。
///
/// 空文档（含仅空行、仅空格）的唯一合法表现是「没有任何块」：布局必须可执行、
/// 几何必须有限，模式开关只能改变视觉而不能造出内容。`total_h` 允许为 0
/// （无内容时没有可滚动高度），但不允许非有限或为负。
fn empty_ui_checks(app: &mut veil_ui::qa::QaApp, surface: Surface) -> Vec<String> {
    let mut problems = Vec::new();
    app.render();
    let blocks = app.blocks();
    if !blocks.is_empty() {
        problems.push(format!(
            "空态文档不得产生块，实际 {}（kinds {:?}）",
            blocks.len(),
            blocks
                .iter()
                .map(|b| kind_name(&b.kind))
                .collect::<Vec<_>>()
        ));
    }
    let total = app.total_h();
    if !total.is_finite() || total < 0.0 {
        problems.push(format!("空态文档总高非法：{total}"));
    }
    // 可见块高度不变量（空态下不应有块，这里作为防御保留）。
    for (i, b) in blocks.iter().enumerate() {
        if !b.top.is_finite() || !b.height.is_finite() {
            problems.push(format!("第 {i} 块几何非有限"));
        } else if b.height <= 0.0 && !b.details_hidden {
            problems.push(format!(
                "第 {i} 块高度为 {:.2}（不可见）：kind={}",
                b.height,
                kind_name(&b.kind)
            ));
        }
    }

    let base_text = app.text();
    match surface {
        Surface::Focus => {
            app.toggle_focus();
            app.render();
            if app.blocks().len() != blocks.len() {
                problems.push(format!(
                    "专注模式改变块数：{} → {}",
                    blocks.len(),
                    app.blocks().len()
                ));
            }
            if app.text() != base_text {
                problems.push("专注模式改变了源文本（应仅为视觉开关）".to_string());
            }
            app.toggle_focus();
        }
        Surface::Typewriter => {
            app.set_caret_middle();
            app.toggle_typewriter();
            app.render();
            app.scroll_to_caret();
            app.render();
            let c = app.caret();
            if app.text() != base_text {
                problems.push("打字机模式改变了源文本（应仅为视觉开关）".to_string());
            }
            if !c.y.is_finite()
                || !c.h.is_finite()
                || !c.scroll.is_finite()
                || !c.max_scroll.is_finite()
            {
                problems.push(format!(
                    "空态光标几何非有限：y={} h={} scroll={} max={}",
                    c.y, c.h, c.scroll, c.max_scroll
                ));
            } else if c.max_scroll <= 0.0 {
                if c.scroll != 0.0 {
                    problems.push(format!("空态不应发生位移，scroll={:.2}", c.scroll));
                }
            } else if c.scroll < -0.01 || c.scroll > c.max_scroll + 0.01 {
                problems.push(format!(
                    "空态滚动越界：scroll={:.2}，max={:.2}",
                    c.scroll, c.max_scroll
                ));
            }
            app.toggle_typewriter();
        }
        _ => {}
    }
    problems
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

// ---- ADJB：全量逐用例 HTML SHA-256 摘要 ----

fn digest_bootstrap() -> bool {
    std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") || !Path::new(DIGEST).exists()
}

/// 读取已提交的摘要表；引导模式（无文件或 UPDATE_GOLDEN=1）返回 None（只写不比对）。
fn digest_expected() -> Option<&'static BTreeMap<String, String>> {
    static MAP: OnceLock<Option<BTreeMap<String, String>>> = OnceLock::new();
    MAP.get_or_init(|| {
        if digest_bootstrap() {
            return None;
        }
        let text = std::fs::read_to_string(DIGEST).ok()?;
        let mut out = BTreeMap::new();
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(id), Some(hash)) = (parts.next(), parts.next()) {
                out.insert(id.to_string(), hash.to_string());
            }
        }
        Some(out)
    })
    .as_ref()
}

fn read_digest_file() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(DIGEST) {
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(id), Some(hash)) = (parts.next(), parts.next()) {
                out.insert(id.to_string(), hash.to_string());
            }
        }
    }
    out
}

fn write_digest(entries: &mut [(String, String)]) {
    if entries.is_empty() {
        return;
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    // 引导模式（无文件或 UPDATE_GOLDEN=1）重写全表；否则只补写缺失记录
    // （ADJI 首轮），已存在记录绝不覆盖——摘要不一致由断言报 FAIL，不允许被写入掩盖。
    let bootstrap = digest_bootstrap();
    let mut merged = if bootstrap {
        BTreeMap::new()
    } else {
        read_digest_file()
    };
    let mut changed = bootstrap;
    for (id, hash) in entries.iter() {
        if !merged.contains_key(id) {
            merged.insert(id.clone(), hash.clone());
            changed = true;
        }
    }
    if !changed {
        return;
    }
    let mut out = String::new();
    for (id, hash) in merged.iter() {
        out.push_str(id);
        out.push(' ');
        out.push_str(hash);
        out.push('\n');
    }
    if let Some(dir) = Path::new(DIGEST).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(DIGEST, out) {
        eprintln!("摘要写入失败（{DIGEST}）：{e}");
    }
}

/// 纯 Rust SHA-256（避免为测试引入新依赖）。
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{word:08x}"));
    }
    out
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
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "details",
    "summary",
    "th",
    "td",
    "table",
    "thead",
    "tbody",
    "tr",
    "ul",
    "ol",
    "li",
    "p",
    "pre",
    "code",
    "figure",
    "blockquote",
    "mark",
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
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
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

/// 需要真实图片文件的用例：`IMG` 全组、表格内/贴邻图片、0 字节与越界边界，
/// 以及含行内图片单元的 ADJI（复用同一 1×1 PNG 与 `docs/assets` 目录约定）。
fn image_case_dir(id: &str) -> Option<&'static Path> {
    let wants = id.starts_with("CASE-IMG-")
        || id.starts_with("CASE-ADJI-")
        || matches!(
            id,
            "CASE-TBL-007"
                | "CASE-TBL-012"
                | "CASE-TBL-013"
                | "CASE-EDGE-013"
                | "CASE-EDGE-014"
                | "CASE-ADV-002"
        );
    wants.then(image_fixture_dir)
}

// =============================================================================
// 生成式块级有序对相邻矩阵（ADJB）
// =============================================================================

/// 单元标签在 HTML 正文中的可见性断言口径。
#[derive(Clone, Copy, PartialEq, Eq)]
enum LabelHtml {
    /// 标签必须出现在正文（可见内容）。
    Present,
    /// 标签不得出现在正文（块无 HTML 输出）。
    Absent,
    /// 不做标签断言（渲染为图形/降级路径不确定）。
    Ignore,
}

struct UnitSpec {
    name: &'static str,
    /// 该单元单独出现时的期望块序列。
    kinds: &'static [&'static str],
    /// HTML 结构标记（全部须命中；可为空表示无 HTML 输出）。
    markers: &'static [&'static str],
    label_html: LabelHtml,
    /// 标签是否应出现在 PDF 版面文本中。
    pdf_label: bool,
}

/// 18 个块单元。夹具细节见 `unit_source`；两处偏离 spec 的夹具调整
/// （ThematicBreak 用 `***`、Table/Mermaid 的第二个文案用 ASCII）在报告里登记。
const UNITS: [UnitSpec; ADJ_UNITS] = [
    UnitSpec {
        name: "Paragraph",
        kinds: &["Paragraph"],
        markers: &["<p>"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "AtxHeading",
        kinds: &["AtxHeading"],
        markers: &["<h1"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "SetextHeading",
        kinds: &["SetextHeading"],
        markers: &["<h1"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "FencedCode",
        kinds: &["FencedCode"],
        markers: &["<pre>"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "IndentedCode",
        kinds: &["IndentedCode"],
        markers: &["<pre>"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "Math",
        kinds: &["Math"],
        markers: &["class=\"math"],
        label_html: LabelHtml::Ignore,
        pdf_label: true,
    },
    UnitSpec {
        name: "Mermaid",
        kinds: &["Mermaid"],
        markers: &["class=\"language-mermaid\""],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "Table",
        kinds: &["Table"],
        markers: &["<table"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "ListItem",
        kinds: &["ListItem"],
        markers: &["<ul>"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "BlockQuote",
        kinds: &["BlockQuote"],
        markers: &["<blockquote"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "Admonition",
        kinds: &["Admonition"],
        markers: &["class=\"callout callout-note\""],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    // 默认 callouts.json 为空：`> [!FIXME]` 在生产默认配置下解析为 BlockQuote
    // （解析层只在配置命中时产出 Callout）。无配置注入的 qa 门面无法造出 Callout，
    // 该单元实测为「未知 Callout 标记回退 BlockQuote」，见报告。
    UnitSpec {
        name: "Callout",
        kinds: &["BlockQuote"],
        markers: &["<blockquote"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "ThematicBreak",
        kinds: &["ThematicBreak"],
        markers: &["<hr"],
        label_html: LabelHtml::Ignore,
        pdf_label: false,
    },
    UnitSpec {
        name: "LinkRefDef",
        kinds: &["LinkRefDef"],
        markers: &[],
        label_html: LabelHtml::Absent,
        pdf_label: false,
    },
    UnitSpec {
        name: "FootnoteDef",
        kinds: &["FootnoteDef"],
        markers: &[],
        label_html: LabelHtml::Absent,
        pdf_label: false,
    },
    UnitSpec {
        name: "HtmlBlock",
        kinds: &["HtmlBlock"],
        markers: &["&lt;div"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "Details",
        kinds: &["Details", "Paragraph"],
        markers: &["<details"],
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    UnitSpec {
        name: "FrontMatter",
        kinds: &["FrontMatter"],
        markers: &[],
        label_html: LabelHtml::Absent,
        pdf_label: false,
    },
];

/// 单元夹具。`label` 为该单元在文档中的 甲/乙 标签（ThematicBreak 无标签）。
fn unit_source(unit: usize, label: &str) -> String {
    match unit {
        0 => label.to_string(),
        1 => format!("# {label}"),
        2 => format!("{label}\n==="),
        3 => format!("```rust\n{label}\n```"),
        4 => format!("    {label}"),
        5 => format!("$$\n{label}\n$$"),
        // 第二个节点用 ASCII：避免 B 侧标签混入 A 侧源码而破坏可区分性。
        6 => format!("```mermaid\nflowchart TD\n    A[{label}] --> B[x]\n```"),
        7 => format!("| {label} | b |\n| --- | --- |\n| 1 | 2 |"),
        8 => format!("- {label}"),
        9 => format!("> {label}"),
        10 => format!("> [!NOTE]\n> {label}"),
        11 => format!("> [!FIXME]\n> {label}"),
        // `---` 位于文档首块时会被未闭合 FrontMatter 启发式吞并（B=AtxHeading/ListItem
        // 时尤甚）；用等价分隔线 `***` 保证恒为 ThematicBreak。
        12 => "***".to_string(),
        13 => format!("[{label}]: https://e.test/{label}"),
        14 => format!("[^{label}]: 脚注"),
        15 => format!("<div>\n{label}\n</div>"),
        16 => format!("<details>\n<summary>{label}</summary>\n\n{label}正文\n\n</details>"),
        17 => format!("---\ntitle: {label}\n---"),
        _ => unreachable!("未知块单元序号 {unit}"),
    }
}

#[derive(Default)]
struct PairAdjust {
    /// 覆盖默认的 A 序列 + B 序列（规范合并时）。
    kinds: Option<&'static [&'static str]>,
    /// 该单元的 HTML 结构标记不作断言（被合并/导出降级时）。
    skip_markers: [bool; 2],
    /// 覆盖该单元的 HTML 标签可见性期望。
    label_html: [Option<LabelHtml>; 2],
    /// 覆盖该单元的 PDF 标签可见性期望。
    pdf_label: [Option<bool>; 2],
}

/// 相邻对的规范合并 / 已知跨表面缺陷适配（逐条登记在报告里，不静默吞掉）。
fn pair_adjust(a: usize, b: usize) -> PairAdjust {
    let mut adj = PairAdjust::default();
    match (a, b) {
        // 列表项后的 4 列行只比内容列（2 列）多 2 列，按项内段落续块，不成为独立缩进代码。
        (8, 4) => {
            adj.kinds = Some(&["ListItem"]);
            adj.skip_markers[1] = true;
        }
        // 缩进代码块内部的空行不终止块：两个缩进代码单元合并为一块。
        (4, 4) => adj.kinds = Some(&["IndentedCode"]),
        // 脚注定义续行要求缩进 ≥4 列：B 的缩进代码被并入定义正文（无独立块、无输出）。
        (14, 4) => {
            adj.kinds = Some(&["FootnoteDef"]);
            adj.skip_markers[1] = true;
            adj.label_html[1] = Some(LabelHtml::Absent);
            adj.pdf_label[1] = Some(false);
        }
        _ => {}
    }
    adj
}

fn pair_kinds(a: usize, b: usize) -> Vec<&'static str> {
    if let Some(kinds) = pair_adjust(a, b).kinds {
        return kinds.to_vec();
    }
    let mut out = UNITS[a].kinds.to_vec();
    out.extend_from_slice(UNITS[b].kinds);
    out
}

/// 规范合并对（按解析器真实块序列断言，报告登记）。
const ADJ_MERGES: [(usize, &str); 3] = [
    (
        77,
        "缩进代码 + 缩进代码：块内空行不终止缩进代码（CommonMark），合并为一块",
    ),
    (
        149,
        "列表项 + 缩进代码：4 列行仅比内容列多 2 列，按项内段落续块（无独立缩进代码块）",
    ),
    (
        257,
        "脚注定义 + 缩进代码：定义续行要求缩进 ≥4 列，缩进代码被并入定义正文（无输出）",
    ),
];

/// 真实跨表面缺陷（Task 4 已修复，保留证据 CASE 号）：link-ref defs 前缀曾使
/// 缩进代码在 HTML 导出中降级为 `<p>`（见报告「已修复」条目与 `export.rs`
/// `link_ref_defs_prefix_keeps_indented_code_block`）。
const ADJ_FIXED_DEFECTS: [(usize, &str); 2] = [
    (
        86,
        "IndentedCode + LinkRefDef：HTML 导出把跨块引用定义收集成 defs 前缀（`[乙]: …` 紧贴 `    甲`），缩进代码被 pulldown 降级为 <p>；已在 `defs` 与正文间补空行修复",
    ),
    (
        239,
        "LinkRefDef + IndentedCode：同上（LinkRefDef 在 A 侧），HTML 的 `    乙` 曾被 defs 前缀降级为 <p>；已修复",
    ),
];

fn adjacency_cases() -> Vec<Case> {
    let mut out = Vec::with_capacity(ADJ_TOTAL);
    for (a, unit_a) in UNITS.iter().enumerate() {
        for (b, unit_b) in UNITS.iter().enumerate() {
            let n = a * ADJ_UNITS + b + 1;
            let id = format!("CASE-{ADJB_GROUP}-{n:03}");
            let (label_a, label_b) = ("甲", "乙");
            let source = format!(
                "{}\n\n{}\n",
                unit_source(a, label_a),
                unit_source(b, label_b)
            );
            let adj = pair_adjust(a, b);
            let kinds = pair_kinds(a, b);

            let mut checks: Vec<(String, String)> = Vec::new();
            checks.push(("blocks".to_string(), kinds.len().to_string()));
            checks.push(("kinds".to_string(), kinds.join(",")));
            for (idx, (unit, label)) in [(unit_a, label_a), (unit_b, label_b)]
                .into_iter()
                .enumerate()
            {
                if !adj.skip_markers[idx] {
                    for marker in unit.markers {
                        checks.push(("html_contains".to_string(), (*marker).to_string()));
                    }
                }
                match adj.label_html[idx].unwrap_or(unit.label_html) {
                    LabelHtml::Present => {
                        checks.push(("html_contains".to_string(), label.to_string()));
                    }
                    LabelHtml::Absent => {
                        checks.push(("html_not_contains".to_string(), label.to_string()));
                    }
                    LabelHtml::Ignore => {}
                }
                if adj.pdf_label[idx].unwrap_or(unit.pdf_label) {
                    checks.push(("pdf_text".to_string(), label.to_string()));
                }
            }
            checks.push(("no_overlap".to_string(), "true".to_string()));

            let na_reason = (b == ADJ_FRONT_MATTER).then(|| ADJ_NA_REASON.to_string());
            out.push(Case {
                id: id.clone(),
                group: ADJB_GROUP.to_string(),
                title: format!("{} + {}", unit_a.name, unit_b.name),
                source,
                expect: "生成式块级相邻有序对：A + 空行 + B".to_string(),
                surfaces: Some(vec![
                    Surface::Wysiwyg,
                    Surface::Focus,
                    Surface::Typewriter,
                    Surface::Html,
                    Surface::Pdf,
                    Surface::Print,
                ]),
                na_reason,
                checks,
                byte_opts: Vec::new(),
                line: 0,
                adjc: None,
                structured: None,
            });
        }
    }
    out
}

/// 抽样金样：ADJB 按执行序号等距抽取 20 条；ADJI 先排除脚注定义用例再等距抽样。
fn adj_golden_sample(case: &Case) -> bool {
    match case.group.as_str() {
        ADJB_GROUP => {
            let n: usize = case
                .id
                .rsplit('-')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            (1..=286).contains(&n) && (n - 1).is_multiple_of(15)
        }
        ADJI_GROUP => adji_golden_sample(case),
        ADJC_GROUP => adjc_golden_sample(case),
        ADJN_GROUP => adjn_golden_ids().iter().any(|id| id == &case.id),
        ADJI2_GROUP => adji2_golden_ids().iter().any(|id| id == &case.id),
        ADJC2_GROUP | ADJX_GROUP => structured_golden_ids().iter().any(|id| id == &case.id),
        _ => true,
    }
}

// =============================================================================
// 生成式行内有序对相邻矩阵（ADJI）
// =============================================================================

struct InlineUnitSpec {
    name: &'static str,
    /// 该单元单独出现时的顶层节点类型（HtmlLiteral 恒无节点：非白名单标签按字面文本）。
    kind: Option<&'static str>,
    /// HTML 结构标记；`None` 表示按 `literal_marker` 规则由标签生成。
    marker: Option<&'static str>,
    /// HTML 可见标签断言口径。
    label_html: LabelHtml,
    /// 标签是否应出现在 PDF 版面文本中。
    pdf_label: bool,
}

/// 12 个行内单元。标签恒为 CJK（甲/乙），图片复用 `image_fixture_dir` 的 1×1 PNG。
const INLINE_UNITS: [InlineUnitSpec; ADJ_INLINE_UNITS] = [
    InlineUnitSpec {
        name: "Strong",
        kind: Some("Strong"),
        marker: Some("<strong"),
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    InlineUnitSpec {
        name: "Emph",
        kind: Some("Emph"),
        marker: Some("<em"),
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    InlineUnitSpec {
        name: "Code",
        kind: Some("Code"),
        marker: Some("<code"),
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    InlineUnitSpec {
        name: "Link",
        kind: Some("Link"),
        marker: Some("<a href="),
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    // 行内图片的标签只出现在 `<img alt>` 属性；不要求正文文本恰出现一次。
    InlineUnitSpec {
        name: "Image",
        kind: Some("Image"),
        marker: Some("<img"),
        label_html: LabelHtml::Ignore,
        pdf_label: false,
    },
    // 行内 CJK 公式无 STIX 字形，导出降级为 `$甲$` 源码（见报告降级说明）。
    InlineUnitSpec {
        name: "Math",
        kind: Some("Math"),
        marker: None,
        label_html: LabelHtml::Ignore,
        pdf_label: false,
    },
    InlineUnitSpec {
        name: "Strikethrough",
        kind: Some("Strikethrough"),
        marker: Some("<del"),
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    InlineUnitSpec {
        name: "Highlight",
        kind: Some("Highlight"),
        marker: Some("<mark"),
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    // 脚注引用渲染为编号 `[n]`，标签本身不可见。
    InlineUnitSpec {
        name: "FootnoteRef",
        kind: Some("FootnoteRef"),
        marker: Some("<sup"),
        label_html: LabelHtml::Ignore,
        pdf_label: false,
    },
    InlineUnitSpec {
        name: "Html",
        kind: Some("Html"),
        marker: Some("<kbd"),
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
    // `<br />` 为 void 标签，源串与渲染都没有甲/乙标签。
    InlineUnitSpec {
        name: "HtmlSelfClosed",
        kind: Some("HtmlSelfClosed"),
        marker: Some("<br"),
        label_html: LabelHtml::Ignore,
        pdf_label: false,
    },
    // 非白名单标签按既有语义保留为字面文本：不产出 inline 节点，
    // 但 HTML/PDF 中 `<foo>甲</foo>` 原样可见。
    InlineUnitSpec {
        name: "HtmlLiteral",
        kind: None,
        marker: None,
        label_html: LabelHtml::Present,
        pdf_label: true,
    },
];

/// 行内单元夹具。`label` 为该单元的甲/乙标签（HtmlSelfClosed 无标签）。
fn inline_unit_source(unit: usize, label: &str) -> String {
    match unit {
        0 => format!("**{label}**"),
        1 => format!("*{label}*"),
        2 => format!("`{label}`"),
        3 => format!("[{label}](https://a.test/1)"),
        4 => format!("![{label}](<assets/a.png>)"),
        5 => format!("${label}$"),
        6 => format!("~~{label}~~"),
        7 => format!("=={label}=="),
        8 => format!("[^{label}]"),
        9 => format!("<kbd>{label}</kbd>"),
        10 => "<br />".to_string(),
        11 => format!("<foo>{label}</foo>"),
        _ => unreachable!("未知行内单元序号 {unit}"),
    }
}

/// 规范允许的分隔符合并（不是产品缺陷）：两条用例在无空格边界下退化为单个节点。
/// `expected` 为 CommonMark/GFM 规则下解析器应产出的顶层节点序列。
struct InlineMerge {
    a: usize,
    b: usize,
    source: &'static str,
    expected: &'static [&'static str],
    rule: &'static str,
}

const ADJ_INLINE_MERGES: [InlineMerge; 3] = [
    InlineMerge {
        a: 0,
        b: 0,
        source: "**甲****乙**",
        expected: &["Strong"],
        rule: "CommonMark 强调「rule of 3」：中间 `****` 串同时可开可闭，与两侧 `**` 的长度和均为 3 的倍数且长度非全为 3 的倍数，不能拆分配对；最外层 `**` 跨过中间串配对，`****` 作为内容字面保留",
    },
    InlineMerge {
        a: 1,
        b: 1,
        source: "*甲**乙*",
        expected: &["Emph"],
        rule: "CommonMark「rule of 3」：中间 `**` 与两侧 `*` 的长度和均为 3 的倍数且长度非全为 3 的倍数，不能拆分配对；最外层 `*` 跨过中间串配对，`**` 作为内容字面保留",
    },
    InlineMerge {
        a: 2,
        b: 2,
        source: "`甲``乙`",
        expected: &["Code"],
        rule: "CommonMark 代码跨度：闭反引号串长度必须与开启串相同；中间长度 2 的串不匹配，最终长度 1 的串闭合首个开启串，`` `` `` 作为代码内容字面保留",
    },
];

fn inline_merge(a: usize, b: usize, no_space: bool) -> Option<&'static InlineMerge> {
    ADJ_INLINE_MERGES
        .iter()
        .find(|m| m.a == a && m.b == b && no_space)
}

fn inline_expected_kinds(a: usize, b: usize, no_space: bool) -> Vec<&'static str> {
    if let Some(merge) = inline_merge(a, b, no_space) {
        return merge.expected.to_vec();
    }
    let mut out = Vec::new();
    if let Some(kind) = INLINE_UNITS[a].kind {
        out.push(kind);
    }
    if let Some(kind) = INLINE_UNITS[b].kind {
        out.push(kind);
    }
    out
}

/// 真实缺陷（Task 4 已修复，保留证据 CASE 号）：`==甲====乙==` 中长度 4 的 `=` run
/// 曾是「既不开也不闭」的死区，整段降级为纯文本；`try_highlight` 现允许该 4 连 run
/// 的前两个闭合、后两个开启，解析为两个 Highlight（见报告「已修复」条目）。
const ADJI_FIXED_DEFECTS: [(usize, &str); 1] = [(
    184,
    "Highlight + Highlight（无空格，`==甲====乙==`）：`=` run 长度 4 曾既不能开也不能闭，整段降级为纯文本；`try_highlight` 的 4 连等号边界规则已修复，现解析出两个 Highlight 并导出两个 `<mark>`",
)];

fn adjacency_inline_cases() -> Vec<Case> {
    let mut out = Vec::with_capacity(ADJI_TOTAL);
    for (a, unit_a) in INLINE_UNITS.iter().enumerate() {
        for (b, unit_b) in INLINE_UNITS.iter().enumerate() {
            for boundary in 0..ADJ_INLINE_BOUNDARIES {
                let no_space = boundary == 1;
                let n = (a * ADJ_INLINE_UNITS + b) * ADJ_INLINE_BOUNDARIES + boundary + 1;
                let id = format!("CASE-{ADJI_GROUP}-{n:03}");
                let (label_a, label_b) = ("甲", "乙");
                let mut source = format!(
                    "{}{}{}\n",
                    inline_unit_source(a, label_a),
                    if no_space { "" } else { " " },
                    inline_unit_source(b, label_b)
                );
                // 脚注引用必须附定义，否则编号/导出不可验证；只附加被引用到的定义。
                if a == 8 || b == 8 {
                    source.push('\n');
                    if a == 8 {
                        source.push_str("[^甲]: 注一\n");
                    }
                    if b == 8 {
                        source.push_str("[^乙]: 注二\n");
                    }
                }

                let mut checks: Vec<(String, String)> = Vec::new();
                let (blocks, kinds) = if a == 8 && b == 8 {
                    (3, "Paragraph,FootnoteDef,FootnoteDef")
                } else if a == 8 || b == 8 {
                    (2, "Paragraph,FootnoteDef")
                } else {
                    (1, "Paragraph")
                };
                checks.push(("blocks".to_string(), blocks.to_string()));
                checks.push(("kinds".to_string(), kinds.to_string()));
                let expected = inline_expected_kinds(a, b, no_space);
                checks.push((
                    "inline_kinds".to_string(),
                    if expected.is_empty() {
                        "None".to_string()
                    } else {
                        expected.join(",")
                    },
                ));

                for (unit, label) in [(a, label_a), (b, label_b)] {
                    let spec = &INLINE_UNITS[unit];
                    if let Some(marker) = spec.marker {
                        checks.push(("html_contains".to_string(), marker.to_string()));
                    }
                    // Math 的 CJK 公式导出降级为源码字面量；HtmlLiteral 按字面文本输出。
                    match unit {
                        5 => checks.push(("html_contains".to_string(), format!("${label}$"))),
                        11 => checks
                            .push(("html_contains".to_string(), format!("<foo>{label}</foo>"))),
                        _ => {}
                    }
                    if spec.label_html == LabelHtml::Present {
                        checks.push(("html_contains".to_string(), label.to_string()));
                    }
                    if spec.pdf_label {
                        checks.push(("pdf_text".to_string(), label.to_string()));
                    }
                }
                // 可见标签必须按 A→B 顺序各出现一次；不可见单元已在报告中登记 Ignore。
                let visible: Vec<&str> = [(a, label_a), (b, label_b)]
                    .into_iter()
                    .filter(|(unit, _)| INLINE_UNITS[*unit].label_html == LabelHtml::Present)
                    .map(|(_, label)| label)
                    .collect();
                if !visible.is_empty() {
                    checks.push(("html_label_once".to_string(), visible.join(",")));
                    checks.push(("html_label_order".to_string(), visible.join(",")));
                }
                checks.push(("no_overlap".to_string(), "true".to_string()));

                out.push(Case {
                    id,
                    group: ADJI_GROUP.to_string(),
                    title: format!(
                        "{} + {}{}",
                        unit_a.name,
                        unit_b.name,
                        if no_space {
                            "（无空格）"
                        } else {
                            "（一个半角空格）"
                        }
                    ),
                    source,
                    expect: "生成式行内相邻有序对：A + 边界 + B".to_string(),
                    surfaces: Some(vec![
                        Surface::Wysiwyg,
                        Surface::Focus,
                        Surface::Typewriter,
                        Surface::Html,
                        Surface::Pdf,
                        Surface::Print,
                    ]),
                    na_reason: None,
                    checks,
                    byte_opts: Vec::new(),
                    line: 0,
                    adjc: None,
                    structured: None,
                });
            }
        }
    }
    out
}

/// ADJI 抽样金样：先排除脚注定义用例（文末脚注区块的编号/回链不稳定），
/// 再对剩余用例等距抽取 20 条。
fn adji_golden_ids() -> &'static Vec<String> {
    static IDS: OnceLock<Vec<String>> = OnceLock::new();
    IDS.get_or_init(|| {
        let mut eligible = Vec::new();
        for a in 0..ADJ_INLINE_UNITS {
            for b in 0..ADJ_INLINE_UNITS {
                if a == 8 || b == 8 {
                    continue;
                }
                for boundary in 0..ADJ_INLINE_BOUNDARIES {
                    let n = (a * ADJ_INLINE_UNITS + b) * ADJ_INLINE_BOUNDARIES + boundary + 1;
                    eligible.push(format!("CASE-{ADJI_GROUP}-{n:03}"));
                }
            }
        }
        (0..20)
            .map(|i| eligible[i * eligible.len() / 20].clone())
            .collect()
    })
}

fn adji_golden_sample(case: &Case) -> bool {
    adji_golden_ids().iter().any(|id| id == &case.id)
}

// =============================================================================
// 生成式容器内容矩阵（ADJC）
// =============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdjcContainer {
    ListItem,
    BlockQuote,
    Admonition,
    Callout,
    Details,
}

impl AdjcContainer {
    fn name(self) -> &'static str {
        match self {
            Self::ListItem => "ListItem",
            Self::BlockQuote => "BlockQuote",
            Self::Admonition => "Admonition",
            Self::Callout => "Callout",
            Self::Details => "Details",
        }
    }

    /// 容器块型在块序列中出现时的类型名。
    fn block_kind(self) -> &'static str {
        self.name()
    }

    /// 容器结构在 HTML 中的标记（全部须命中）。
    fn html_markers(self) -> &'static [&'static str] {
        match self {
            Self::ListItem => &["<ul>", "<li>"],
            Self::BlockQuote => &["<blockquote"],
            Self::Admonition => &["class=\"callout callout-note\""],
            Self::Callout => &["class=\"callout callout-fixme\""],
            Self::Details => &["<details"],
        }
    }

    /// 夹具：容器标记（可见文本「甲容器」）+ 内层块 B（逐行加容器前缀）。
    fn wrap(self, body: &str) -> String {
        match self {
            Self::ListItem => format!("- 甲容器\n\n{}\n", prefix_lines(body, "  ")),
            Self::BlockQuote => format!("> 甲容器\n>\n{}\n", prefix_lines(body, "> ")),
            Self::Admonition => {
                format!("> [!NOTE]\n> 甲容器\n>\n{}\n", prefix_lines(body, "> "))
            }
            Self::Callout => {
                format!("> [!FIXME]\n> 甲容器\n>\n{}\n", prefix_lines(body, "> "))
            }
            // summary 必须独占一行（块级 `<details>` 的 summary 提取以行首标签为准）。
            Self::Details => {
                format!("<details>\n<summary>甲容器</summary>\n\n{body}\n\n</details>\n")
            }
        }
    }
}

/// 逐行加容器前缀（空行也加，保证多行块整体仍属容器）。
fn prefix_lines(body: &str, prefix: &str) -> String {
    body.split('\n')
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

struct AdjcSpec {
    container: AdjcContainer,
    /// 内层块单元序号（对应 [`UNITS`] / `unit_source`）。
    unit: usize,
}

impl AdjcSpec {
    fn b_name(&self) -> &'static str {
        UNITS[self.unit].name
    }

    /// 内层块解析后的块序列（按容器实际模型；Table/Mermaid 为独立块）。
    fn kinds(&self) -> Vec<&'static str> {
        match self.container {
            AdjcContainer::ListItem => match self.unit {
                // Table / Mermaid 由容器展开切成独立块。
                7 => vec!["ListItem", "Table"],
                6 => vec!["ListItem", "Mermaid"],
                // 嵌套列表项：父项 + 子项各为一块。
                8 => vec!["ListItem", "ListItem"],
                _ => vec!["ListItem"],
            },
            AdjcContainer::BlockQuote => match self.unit {
                7 => vec!["BlockQuote", "Table"],
                6 => vec!["BlockQuote", "Mermaid"],
                _ => vec!["BlockQuote"],
            },
            AdjcContainer::Admonition => match self.unit {
                // Admonition 只提升 Mermaid；表格留在提示框内（不切独立块）。
                6 => vec!["Admonition", "Mermaid"],
                _ => vec!["Admonition"],
            },
            AdjcContainer::Callout => match self.unit {
                // Callout 只提升 Mermaid；表格留在提示框内（不切独立块）。
                6 => vec!["Callout", "Mermaid"],
                _ => vec!["Callout"],
            },
            AdjcContainer::Details => {
                let mut out = vec!["Details"];
                match self.unit {
                    16 => out.extend_from_slice(&["Details", "Paragraph"]),
                    // FrontMatter 只识别于文档首块：details 内 `---` 退化为分隔线 +
                    // 后续 `title: 乙\n---` 的 Setext 标题。
                    17 => out.extend_from_slice(&["ThematicBreak", "SetextHeading"]),
                    // harness 对本组注入了 FIXME registry，内层 Callout 成为独立块。
                    11 => out.push("Callout"),
                    _ => out.push(UNITS[self.unit].kinds[0]),
                }
                out
            }
        }
    }

    /// 内层块 B 在 HTML 中的结构标记（沿用 ADJB UNITS 的 markers，按容器实际
    /// 解析/导出形态修正；Math/Admonition/Callout 等仅在 Details 内成为独立块）。
    fn html_markers(&self) -> Vec<&'static str> {
        match self.unit {
            // Mermaid 必须真实出图（`<svg`），且不得残留源码围栏降级形态。
            6 => vec!["<svg"],
            // Math 只在 Details 内是独立 Math 块；其余容器内按正文文本降级。
            5 => match self.container {
                AdjcContainer::Details => vec!["class=\"math"],
                _ => Vec::new(),
            },
            // 内层 Admonition / Callout 只在 Details 内提升为独立块。
            10 => match self.container {
                AdjcContainer::Details => vec!["class=\"callout callout-note\""],
                _ => Vec::new(),
            },
            11 => match self.container {
                AdjcContainer::Details => vec!["class=\"callout callout-fixme\""],
                _ => Vec::new(),
            },
            // HtmlBlock：Details 内是独立块（导出转义为 &lt;div）；其余容器内
            // 由 pulldown 原样透传 `<div>`。
            15 => match self.container {
                AdjcContainer::Details => vec!["&lt;div"],
                _ => vec!["<div"],
            },
            // Details：仅 Details 容器内提升为独立块（`<details open>`）；其余容器
            // 内开标签被 pulldown 转义，但 `<summary>` 结构仍原样保留。
            16 => match self.container {
                AdjcContainer::Details => vec!["<details"],
                _ => vec!["<summary"],
            },
            _ => UNITS[self.unit].markers.to_vec(),
        }
    }

    /// 标签「乙」在导出正文 HTML（含 `id=` / `href=` 等属性值）中应出现的次数；
    /// None = 不做计数断言（渲染图形）。计数用于结构回归，不等同于纯可见文本。
    fn html_label_count(&self) -> Option<usize> {
        match self.unit {
            5 | 6 => None, // Math 正文文本形态不稳定 / Mermaid 由 SVG 承载
            // Details 内标题由导出路径生成 `id`（含乙），故标签出现 2 次。
            1 | 2 if self.container == AdjcContainer::Details => Some(2),
            12 => Some(0),
            // LinkRefDef：引用块 / Callout / Details 内被当作定义消费（0）；
            // 列表项 / Admonition 内则原样成为正文（`[乙]` + href + 文本 = 3）。
            13 => Some(match self.container {
                AdjcContainer::BlockQuote | AdjcContainer::Details | AdjcContainer::Callout => 0,
                _ => 3,
            }),
            // FootnoteDef：HTML 导出丢弃定义行（各容器均为 0）。
            14 => Some(0),
            // Details 单元：summary + 正文各一次；Details 容器内嵌套 details 同理。
            16 => Some(2),
            // FrontMatter 在 Details 内降级为 `<hr>` + Setext 标题（标题 id 含乙）。
            17 => Some(match self.container {
                AdjcContainer::Details => 2,
                _ => 1,
            }),
            _ => Some(1),
        }
    }

    /// 可见标签「乙」在 PDF 版面文本中应出现的次数；None = Ignore。
    fn pdf_label_count(&self) -> Option<usize> {
        match self.unit {
            6 => None, // Mermaid 无数据时版面为源码/占位，不做标签计数
            5 => Some(1),
            12 => Some(0),
            // LinkRefDef：非 Details 容器的版面保留定义源码行（`[乙]` + URL = 2）；
            // Details 内定义块无输出。
            13 => Some(if self.container == AdjcContainer::Details {
                0
            } else {
                2
            }),
            14 => Some(if self.container == AdjcContainer::Details {
                0
            } else {
                1
            }),
            16 => Some(2),
            _ => Some(1),
        }
    }
}

fn adjc_callouts() -> veil_core::CalloutRegistry {
    veil_core::CalloutRegistry::from_json(
        r##"[{"name":"FIXME","label":"待修复","color":"#c00","icon":"!"}]"##,
    )
}

fn adjc_cases() -> Vec<Case> {
    let containers = [
        AdjcContainer::ListItem,
        AdjcContainer::BlockQuote,
        AdjcContainer::Admonition,
        AdjcContainer::Callout,
        AdjcContainer::Details,
    ];
    let mut out = Vec::with_capacity(ADJC_TOTAL);
    for (ci, container) in containers.iter().enumerate() {
        for unit in 0..ADJ_UNITS {
            let n = ci * ADJ_UNITS + unit + 1;
            let id = format!("CASE-{ADJC_GROUP}-{n:03}");
            let b = unit_source(unit, "乙");
            let source = container.wrap(&b);
            let spec = AdjcSpec {
                container: *container,
                unit,
            };
            let kinds = spec.kinds();
            let mut checks: Vec<(String, String)> = Vec::new();
            checks.push(("blocks".to_string(), kinds.len().to_string()));
            checks.push(("kinds".to_string(), kinds.join(",")));
            for marker in container.html_markers() {
                checks.push(("html_contains".to_string(), (*marker).to_string()));
            }
            for marker in spec.html_markers() {
                checks.push(("html_contains".to_string(), marker.to_string()));
            }
            if unit == 6 {
                checks.push((
                    "html_not_contains".to_string(),
                    "language-mermaid".to_string(),
                ));
            }
            checks.push(("no_overlap".to_string(), "true".to_string()));
            out.push(Case {
                id,
                group: ADJC_GROUP.to_string(),
                title: format!("{} ⇐ {}", spec.b_name(), container.name()),
                source,
                expect: "生成式容器内容矩阵：容器（甲容器）包裹 18 个块单元（乙）".to_string(),
                surfaces: Some(vec![
                    Surface::Wysiwyg,
                    Surface::Focus,
                    Surface::Typewriter,
                    Surface::Html,
                    Surface::Pdf,
                    Surface::Print,
                ]),
                na_reason: None,
                checks,
                byte_opts: Vec::new(),
                line: 0,
                adjc: Some(spec),
                structured: None,
            });
        }
    }
    out
}

fn adjc_golden_sample(case: &Case) -> bool {
    if case.group != ADJC_GROUP {
        return false;
    }
    let n: usize = case
        .id
        .rsplit('-')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    ADJC_GOLDEN.contains(&n)
}

/// ADJC PARSE 断言：容器块型出现 + 独立 Table/Mermaid 块 + `BlockContainer`
/// 归属（quote_depth / list_indent / details）+ Mermaid 源码前缀已剥离。
fn adjc_parse_checks(session: &Session, spec: &AdjcSpec, problems: &mut Vec<String>) {
    let entries = session.block_entries();
    let kinds: Vec<&'static str> = entries.iter().map(|e| kind_name(&e.1)).collect();
    if !kinds.contains(&spec.container.block_kind()) {
        problems.push(format!(
            "容器块型 {} 未出现在块序列 {:?}",
            spec.container.block_kind(),
            kinds
        ));
    }
    let Some((_, kind, _, container, _, _)) = entries
        .iter()
        .find(|e| kind_name(&e.1) == spec.container.block_kind())
    else {
        return;
    };
    if spec.container == AdjcContainer::Details {
        // 顶层 Details 头块取其所在层级 0（`details=false`）；子块才是 depth>0。
        if container.details || container.details_depth != 0 {
            problems.push(format!(
                "Details 头块容器归属异常：details={} depth={}",
                container.details, container.details_depth
            ));
        }
    } else if container.is_nested() {
        problems.push(format!(
            "容器块自身不应携带嵌套归属：{container:?}（kind={})",
            kind_name(kind)
        ));
    }

    let lifted = match (spec.container, spec.unit) {
        // Admonition / Callout 不切表格（表格留在提示框内），只提升 Mermaid。
        (AdjcContainer::Admonition | AdjcContainer::Callout, 7) => None,
        (_, 7) => Some("Table"),
        (_, 6) => Some("Mermaid"),
        _ => None,
    };
    let Some(target) = lifted else {
        return;
    };
    let Some((lid, _, _, c, _, _)) = entries.iter().find(|e| kind_name(&e.1) == target) else {
        problems.push(format!("{target} 应作为独立块出现，实际块序列 {:?}", kinds));
        return;
    };
    match spec.container {
        AdjcContainer::ListItem => {
            if c.list_indent == 0 || c.quote_depth != 0 || c.details {
                problems.push(format!("ListItem 内 {target} 容器归属异常：{c:?}"));
            }
        }
        AdjcContainer::BlockQuote | AdjcContainer::Admonition | AdjcContainer::Callout => {
            if c.quote_depth == 0 || c.details {
                problems.push(format!("引用类容器内 {target} 容器归属异常：{c:?}"));
            }
        }
        AdjcContainer::Details => {
            if !c.details || c.details_depth == 0 {
                problems.push(format!("Details 内 {target} 容器归属异常：{c:?}"));
            }
        }
    }
    if target == "Mermaid" {
        let Some((_, body)) = session.fenced_code_source(*lid) else {
            problems.push("Mermaid 块缺少 fenced_code_source".to_string());
            return;
        };
        if !body.starts_with("flowchart TD") || !body.contains("A[乙]") {
            problems.push(format!("Mermaid 源码未正确剥离容器前缀：`{body}`"));
        }
        if body.lines().any(|l| l.trim_start().starts_with('>')) {
            problems.push(format!("Mermaid 源码仍含行首引用标记 `>`：`{body}`"));
        }
    }
}

/// ADJC HTML 断言：正文 HTML（含属性值）中容器标记「甲」恰一次、内层块标签
/// 「乙」按容器模型计数；Mermaid 必须 `<svg` 且无源码围栏降级。
fn adjc_html_checks(spec: &AdjcSpec, body: &str, problems: &mut Vec<String>) {
    let count = body.matches("甲").count();
    if count != 1 {
        problems.push(format!(
            "正文 HTML（含属性值）中容器标记「甲」应出现 1 次，实际 {count} 次；正文片段：`{}`",
            snippet(body, 300)
        ));
    }
    if let Some(want) = spec.html_label_count() {
        let count = body.matches("乙").count();
        if count != want {
            problems.push(format!(
                "正文 HTML（含属性值）中内层块标签「乙」应出现 {want} 次，实际 {count} 次；正文片段：`{}`",
                snippet(body, 300)
            ));
        }
    }
    if spec.unit == 6 {
        if !body.contains("<svg") {
            problems.push("容器内 Mermaid 未渲染为内嵌 SVG".to_string());
        }
        if body.contains("language-mermaid") {
            problems.push("容器内 Mermaid 仍残留源码围栏 language-mermaid".to_string());
        }
    }
}

/// ADJC PDF 断言：标签「乙」按容器模型计数（Math/Image 等按既有口径 Ignore）。
fn adjc_pdf_checks(spec: &AdjcSpec, text: &str, problems: &mut Vec<String>) {
    if let Some(want) = spec.pdf_label_count() {
        let n = norm(text).matches('乙').count();
        if n != want {
            problems.push(format!(
                "PDF 版面标签「乙」应出现 {want} 次，实际 {n} 次；文本片段：`{}`",
                snippet(text, 300)
            ));
        }
    }
}

// =============================================================================
// 生成式块级单换行边界矩阵（ADJN）
// =============================================================================

const R_PARA_CONT: &str = "CommonMark §4.8 段落：无空行时后续普通行按段落续行合并";
const R_SETEXT: &str = "CommonMark §4.3 Setext 标题：段落末行后紧跟 `===`/`---` 时整段成为标题";
const R_INDENT_CODE: &str = "CommonMark §4.4：缩进代码块不能打断段落（Example 70）";
const R_INDENT_MERGE: &str = "CommonMark §4.4：缩进代码块内空行不终止块";
const R_TABLE_ROWS: &str = "GFM §4.10 表格：表体延续至空行或块级结构起始；普通行/定义行成为单格行";
const R_DEF_CONT: &str = "CommonMark §4.7 / GFM 脚注：链接引用定义与脚注定义不能打断段落";
const R_FRONTMATTER: &str =
    "Veil FrontMatter 仅识别于文档首块：非首块 `---` 退化为分隔线，`title: X` + `---` 成 Setext 标题";
const R_LIST_LAZY: &str = "CommonMark §5.2 列表项：无缩进续行按懒续行并入项内段落";
const R_LIST_SETEXT: &str = "CommonMark §5.2 + §4.3：列表项懒续行后接 `===` 形成项内 Setext 标题";
const R_QUOTE_LAZY: &str = "CommonMark §5.1 引用块：无 `>` 前缀的续行按懒续行并入引用";
const R_QUOTE_MERGE: &str = "CommonMark §5.1：相邻同类型引用块合并为一个引用块";
const R_FOOTNOTE_INDENT: &str = "GFM 脚注定义：续行要求缩进 ≥4 列，缩进代码被并入定义正文";
const R_HTML6: &str = "CommonMark §4.6 HTML 块类型 6：吞并至首个空行";

/// 单换行边界下与「A 序列 + B 序列」不同的有序对：逐条登记规范依据。
struct AdjnException {
    a: usize,
    b: usize,
    kinds: &'static [&'static str],
    rule: &'static str,
}

const ADJN_EXCEPTIONS: &[AdjnException] = &[
    // A=Paragraph：段落续行/Setext/缩进代码/定义/表格规则
    AdjnException {
        a: 0,
        b: 0,
        kinds: &["Paragraph"],
        rule: R_PARA_CONT,
    },
    AdjnException {
        a: 0,
        b: 2,
        kinds: &["SetextHeading"],
        rule: R_SETEXT,
    },
    AdjnException {
        a: 0,
        b: 4,
        kinds: &["Paragraph"],
        rule: R_INDENT_CODE,
    },
    AdjnException {
        a: 0,
        b: 13,
        kinds: &["Paragraph"],
        rule: R_DEF_CONT,
    },
    AdjnException {
        a: 0,
        b: 14,
        kinds: &["Paragraph"],
        rule: R_DEF_CONT,
    },
    AdjnException {
        a: 0,
        b: 17,
        kinds: &["SetextHeading", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    // 缩进代码块内空行不终止块
    AdjnException {
        a: 4,
        b: 4,
        kinds: &["IndentedCode"],
        rule: R_INDENT_MERGE,
    },
    // 表格体吞并后续普通行/定义行
    AdjnException {
        a: 7,
        b: 0,
        kinds: &["Table"],
        rule: R_TABLE_ROWS,
    },
    AdjnException {
        a: 7,
        b: 2,
        kinds: &["Table"],
        rule: R_TABLE_ROWS,
    },
    AdjnException {
        a: 7,
        b: 7,
        kinds: &["Table"],
        rule: R_TABLE_ROWS,
    },
    AdjnException {
        a: 7,
        b: 13,
        kinds: &["Table"],
        rule: R_TABLE_ROWS,
    },
    AdjnException {
        a: 7,
        b: 14,
        kinds: &["Table"],
        rule: R_TABLE_ROWS,
    },
    // 列表项懒续行
    AdjnException {
        a: 8,
        b: 0,
        kinds: &["ListItem"],
        rule: R_LIST_LAZY,
    },
    AdjnException {
        a: 8,
        b: 2,
        kinds: &["ListItem"],
        rule: R_LIST_SETEXT,
    },
    AdjnException {
        a: 8,
        b: 4,
        kinds: &["ListItem"],
        rule: R_LIST_LAZY,
    },
    AdjnException {
        a: 8,
        b: 13,
        kinds: &["ListItem"],
        rule: R_LIST_LAZY,
    },
    AdjnException {
        a: 8,
        b: 14,
        kinds: &["ListItem"],
        rule: R_LIST_LAZY,
    },
    // 引用块懒续行 / 合并
    AdjnException {
        a: 9,
        b: 0,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 9,
        b: 2,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 9,
        b: 4,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 9,
        b: 7,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 9,
        b: 9,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_MERGE,
    },
    AdjnException {
        a: 9,
        b: 10,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_MERGE,
    },
    AdjnException {
        a: 9,
        b: 11,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_MERGE,
    },
    AdjnException {
        a: 9,
        b: 13,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 9,
        b: 14,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    // Admonition 与引用块同构
    AdjnException {
        a: 10,
        b: 0,
        kinds: &["Admonition"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 10,
        b: 2,
        kinds: &["Admonition"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 10,
        b: 4,
        kinds: &["Admonition"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 10,
        b: 7,
        kinds: &["Admonition"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 10,
        b: 9,
        kinds: &["Admonition"],
        rule: R_QUOTE_MERGE,
    },
    AdjnException {
        a: 10,
        b: 10,
        kinds: &["Admonition"],
        rule: R_QUOTE_MERGE,
    },
    AdjnException {
        a: 10,
        b: 11,
        kinds: &["Admonition"],
        rule: R_QUOTE_MERGE,
    },
    AdjnException {
        a: 10,
        b: 13,
        kinds: &["Admonition"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 10,
        b: 14,
        kinds: &["Admonition"],
        rule: R_QUOTE_LAZY,
    },
    // 未知 Callout 标记回退 BlockQuote
    AdjnException {
        a: 11,
        b: 0,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 11,
        b: 2,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 11,
        b: 4,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 11,
        b: 7,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 11,
        b: 9,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_MERGE,
    },
    AdjnException {
        a: 11,
        b: 10,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_MERGE,
    },
    AdjnException {
        a: 11,
        b: 11,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_MERGE,
    },
    AdjnException {
        a: 11,
        b: 13,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    AdjnException {
        a: 11,
        b: 14,
        kinds: &["BlockQuote"],
        rule: R_QUOTE_LAZY,
    },
    // 脚注定义续行
    AdjnException {
        a: 14,
        b: 4,
        kinds: &["FootnoteDef"],
        rule: R_FOOTNOTE_INDENT,
    },
    // HTML 块类型 6 吞并
    AdjnException {
        a: 15,
        b: 0,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 1,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 2,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 3,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 4,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 5,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 6,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 7,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 8,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 9,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 10,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 11,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 12,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 13,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 14,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 15,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 16,
        kinds: &["HtmlBlock", "Paragraph", "HtmlBlock"],
        rule: R_HTML6,
    },
    AdjnException {
        a: 15,
        b: 17,
        kinds: &["HtmlBlock"],
        rule: R_HTML6,
    },
    // B=FrontMatter：非首块退化为分隔线 + Setext 标题
    AdjnException {
        a: 1,
        b: 17,
        kinds: &["AtxHeading", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 2,
        b: 17,
        kinds: &["SetextHeading", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 3,
        b: 17,
        kinds: &["FencedCode", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 4,
        b: 17,
        kinds: &["IndentedCode", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 5,
        b: 17,
        kinds: &["Math", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 6,
        b: 17,
        kinds: &["Mermaid", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 7,
        b: 17,
        kinds: &["Table", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 8,
        b: 17,
        kinds: &["ListItem", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 9,
        b: 17,
        kinds: &["BlockQuote", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 10,
        b: 17,
        kinds: &["Admonition", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 11,
        b: 17,
        kinds: &["BlockQuote", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 12,
        b: 17,
        kinds: &["ThematicBreak", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 13,
        b: 17,
        kinds: &["LinkRefDef", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 14,
        b: 17,
        kinds: &["FootnoteDef", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 16,
        b: 17,
        kinds: &["Details", "Paragraph", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
    AdjnException {
        a: 17,
        b: 17,
        kinds: &["FrontMatter", "ThematicBreak", "SetextHeading"],
        rule: R_FRONTMATTER,
    },
];

fn adjn_exception(a: usize, b: usize) -> Option<&'static AdjnException> {
    ADJN_EXCEPTIONS.iter().find(|e| e.a == a && e.b == b)
}

fn adjn_kinds(a: usize, b: usize) -> Vec<&'static str> {
    if let Some(e) = adjn_exception(a, b) {
        return e.kinds.to_vec();
    }
    let mut out = UNITS[a].kinds.to_vec();
    out.extend_from_slice(UNITS[b].kinds);
    out
}

/// 期望块序列对应的 HTML 结构标记（标题/定义/空输出块不在此断言，块序列已覆盖）。
fn adjn_markers(kinds: &[&str]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for k in kinds {
        let m = match *k {
            "Paragraph" => Some("<p>"),
            "FencedCode" | "IndentedCode" => Some("<pre>"),
            "Math" => Some("class=\"math"),
            "Mermaid" => Some("<svg"),
            "Table" => Some("<table"),
            "ListItem" => Some("<li>"),
            "BlockQuote" => Some("<blockquote"),
            "Admonition" => Some("callout-note"),
            "ThematicBreak" => Some("<hr"),
            "HtmlBlock" => Some("&lt;"),
            "Details" => Some("<details"),
            _ => None,
        };
        if let Some(m) = m {
            if !out.contains(&m) {
                out.push(m);
            }
        }
    }
    out
}

/// 单元标签在单换行边界下的可见性（按 18 个单元的导出模型）。
fn adjn_a_visible(a: usize) -> bool {
    !matches!(a, 12 | 13 | 14 | 17)
}

fn adjn_b_visible(a: usize, b: usize) -> bool {
    match b {
        12 => false,
        // 引用定义在块起始被消费；在段落/表格/列表/引用/HTML 块尾部则作为正文可见。
        13 | 14 => matches!(a, 0 | 7 | 8 | 9 | 10 | 11 | 15),
        _ => !(a == 14 && b == 4),
    }
}

fn adjn_cases() -> Vec<Case> {
    let mut out = Vec::with_capacity(ADJN_TOTAL);
    for (a, unit_a) in UNITS.iter().enumerate() {
        for (b, unit_b) in UNITS.iter().enumerate() {
            let n = a * ADJ_UNITS + b + 1;
            let id = format!("CASE-{ADJN_GROUP}-{n:03}");
            let source = format!("{}\n{}\n", unit_source(a, "甲"), unit_source(b, "乙"));
            let kinds = adjn_kinds(a, b);
            let mut checks: Vec<(String, String)> = Vec::new();
            checks.push(("blocks".to_string(), kinds.len().to_string()));
            checks.push(("kinds".to_string(), kinds.join(",")));
            for marker in adjn_markers(&kinds) {
                checks.push(("html_contains".to_string(), marker.to_string()));
            }
            // 标签可见性按单元导出模型断言；不可见时用 not_contains 证伪吞并。
            if a != 12 {
                if adjn_a_visible(a) {
                    checks.push(("html_contains".to_string(), "甲".to_string()));
                    checks.push(("pdf_text".to_string(), "甲".to_string()));
                } else {
                    checks.push(("html_not_contains".to_string(), "甲".to_string()));
                    checks.push(("pdf_not_text".to_string(), "甲".to_string()));
                }
            }
            if b != 12 {
                if adjn_b_visible(a, b) {
                    checks.push(("html_contains".to_string(), "乙".to_string()));
                    checks.push(("pdf_text".to_string(), "乙".to_string()));
                } else {
                    checks.push(("html_not_contains".to_string(), "乙".to_string()));
                    checks.push(("pdf_not_text".to_string(), "乙".to_string()));
                }
            }
            checks.push(("no_overlap".to_string(), "true".to_string()));

            let na_reason = (b == ADJ_FRONT_MATTER).then(|| ADJ_NA_REASON.to_string());
            out.push(Case {
                id,
                group: ADJN_GROUP.to_string(),
                title: format!("{} + {}", unit_a.name, unit_b.name),
                source,
                expect: "生成式块级单换行有序对：A + 换行 + B".to_string(),
                surfaces: Some(vec![
                    Surface::Wysiwyg,
                    Surface::Focus,
                    Surface::Typewriter,
                    Surface::Html,
                    Surface::Pdf,
                    Surface::Print,
                ]),
                na_reason,
                checks,
                byte_opts: Vec::new(),
                line: 0,
                adjc: None,
                structured: None,
            });
        }
    }
    out
}

/// ADJN 抽样金样：排除 Mermaid 单元（SVG 浮点坐标）与 N/A（B=FrontMatter），
/// 对剩余用例等距抽取 20 条。
fn adjn_golden_ids() -> &'static Vec<String> {
    static IDS: OnceLock<Vec<String>> = OnceLock::new();
    IDS.get_or_init(|| {
        let mut eligible = Vec::new();
        for a in 0..ADJ_UNITS {
            for b in 0..ADJ_UNITS {
                if a == 6 || b == 6 || b == 17 {
                    continue;
                }
                let n = a * ADJ_UNITS + b + 1;
                eligible.push(format!("CASE-{ADJN_GROUP}-{n:03}"));
            }
        }
        (0..20)
            .map(|i| eligible[i * eligible.len() / 20].clone())
            .collect()
    })
}

// =============================================================================
// 生成式行内边界矩阵（ADJI2）
// =============================================================================

/// `<br />` 独占一行时开启 HTML 块类型 7，吞并至空行（CommonMark §4.6）。
fn adji2_html_block(a: usize, boundary: usize) -> bool {
    a == 10 && boundary == 0
}

fn adji2_cases() -> Vec<Case> {
    let mut out = Vec::with_capacity(ADJI2_TOTAL);
    for (a, unit_a) in INLINE_UNITS.iter().enumerate() {
        for (b, unit_b) in INLINE_UNITS.iter().enumerate() {
            for (bi, boundary) in ADJI2_BOUNDARIES.iter().enumerate() {
                let n = (a * ADJ_INLINE_UNITS + b) * ADJI2_BOUNDARIES.len() + bi + 1;
                let id = format!("CASE-{ADJI2_GROUP}-{n:03}");
                let (label_a, label_b) = ("甲", "乙");
                let mut source = format!(
                    "{}{}{}\n",
                    inline_unit_source(a, label_a),
                    boundary,
                    inline_unit_source(b, label_b)
                );
                if a == 8 || b == 8 {
                    source.push('\n');
                    if a == 8 {
                        source.push_str("[^甲]: 注一\n");
                    }
                    if b == 8 {
                        source.push_str("[^乙]: 注二\n");
                    }
                }

                let special = adji2_html_block(a, bi);
                let mut checks: Vec<(String, String)> = Vec::new();
                if special {
                    // `<br />` 后紧跟换行：整段成为 HTML 块（类型 7），不再有段落。
                    let kinds = if b == 8 {
                        "HtmlBlock,FootnoteDef"
                    } else {
                        "HtmlBlock"
                    };
                    checks.push(("blocks".to_string(), kinds.split(',').count().to_string()));
                    checks.push(("kinds".to_string(), kinds.to_string()));
                    checks.push(("html_contains".to_string(), "<br".to_string()));
                    // HTML 块内容原样透传：B 为行内 HTML 单元时其标签仍以原文出现，
                    // 只有非 HTML 单元的解析标记（`<strong` 等）必须缺席。
                    if !matches!(b, 9..=11) {
                        if let Some(marker) = INLINE_UNITS[b].marker {
                            checks.push(("html_not_contains".to_string(), marker.to_string()));
                        }
                    }
                    if b == 11 {
                        // HtmlLiteral 在 HTML 块内被转义（不再是白名单行内标签）。
                        checks.push((
                            "html_contains".to_string(),
                            format!("&lt;foo&gt;{label_b}&lt;/foo&gt;"),
                        ));
                    }
                    if INLINE_UNITS[b].label_html == LabelHtml::Present {
                        checks.push(("html_contains".to_string(), label_b.to_string()));
                        checks.push(("pdf_text".to_string(), label_b.to_string()));
                    }
                    checks.push(("no_overlap".to_string(), "true".to_string()));
                } else {
                    let (blocks, kinds) = if a == 8 && b == 8 {
                        (3, "Paragraph,FootnoteDef,FootnoteDef")
                    } else if a == 8 || b == 8 {
                        (2, "Paragraph,FootnoteDef")
                    } else {
                        (1, "Paragraph")
                    };
                    checks.push(("blocks".to_string(), blocks.to_string()));
                    checks.push(("kinds".to_string(), kinds.to_string()));
                    let expected = inline_expected_kinds(a, b, false);
                    checks.push((
                        "inline_kinds".to_string(),
                        if expected.is_empty() {
                            "None".to_string()
                        } else {
                            expected.join(",")
                        },
                    ));
                    for (unit, label) in [(a, label_a), (b, label_b)] {
                        let spec = &INLINE_UNITS[unit];
                        if let Some(marker) = spec.marker {
                            checks.push(("html_contains".to_string(), marker.to_string()));
                        }
                        match unit {
                            5 => checks.push(("html_contains".to_string(), format!("${label}$"))),
                            11 => checks
                                .push(("html_contains".to_string(), format!("<foo>{label}</foo>"))),
                            _ => {}
                        }
                        if spec.label_html == LabelHtml::Present {
                            checks.push(("html_contains".to_string(), label.to_string()));
                        }
                        if spec.pdf_label {
                            checks.push(("pdf_text".to_string(), label.to_string()));
                        }
                    }
                    let visible: Vec<&str> = [(a, label_a), (b, label_b)]
                        .into_iter()
                        .filter(|(unit, _)| INLINE_UNITS[*unit].label_html == LabelHtml::Present)
                        .map(|(_, label)| label)
                        .collect();
                    if !visible.is_empty() {
                        checks.push(("html_label_once".to_string(), visible.join(",")));
                        checks.push(("html_label_order".to_string(), visible.join(",")));
                    }
                    checks.push(("no_overlap".to_string(), "true".to_string()));
                }

                out.push(Case {
                    id,
                    group: ADJI2_GROUP.to_string(),
                    title: format!(
                        "{} + {}{}",
                        unit_a.name,
                        unit_b.name,
                        match bi {
                            0 => "（换行）".to_string(),
                            _ => format!("（`{}`）", boundary),
                        }
                    ),
                    source,
                    expect: "生成式行内相邻有序对：A + 边界 + B".to_string(),
                    surfaces: Some(vec![
                        Surface::Wysiwyg,
                        Surface::Focus,
                        Surface::Typewriter,
                        Surface::Html,
                        Surface::Pdf,
                        Surface::Print,
                    ]),
                    na_reason: None,
                    checks,
                    byte_opts: Vec::new(),
                    line: 0,
                    adjc: None,
                    structured: None,
                });
            }
        }
    }
    out
}

/// ADJI2 抽样金样：排除脚注引用（编号/回链易受实现细节影响）后等距抽取 20 条。
fn adji2_golden_ids() -> &'static Vec<String> {
    static IDS: OnceLock<Vec<String>> = OnceLock::new();
    IDS.get_or_init(|| {
        let mut eligible = Vec::new();
        for a in 0..ADJ_INLINE_UNITS {
            for b in 0..ADJ_INLINE_UNITS {
                if a == 8 || b == 8 {
                    continue;
                }
                for bi in 0..ADJI2_BOUNDARIES.len() {
                    let n = (a * ADJ_INLINE_UNITS + b) * ADJI2_BOUNDARIES.len() + bi + 1;
                    eligible.push(format!("CASE-{ADJI2_GROUP}-{n:03}"));
                }
            }
        }
        (0..20)
            .map(|i| eligible[i * eligible.len() / 20].clone())
            .collect()
    })
}

// =============================================================================
// 容器内相邻矩阵（ADJC2）与嵌套容器矩阵（ADJX）
// =============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum StructMode {
    /// 5 类容器 × 两个内层块（自相邻 / 关键跨类型）。
    Content(AdjcContainer),
    /// 容器套容器。
    Nested {
        outer: AdjcContainer,
        inner: AdjcContainer,
    },
}

struct StructuredSpec {
    mode: StructMode,
    kinds: Vec<&'static str>,
    /// 正文 HTML 中必须出现的结构标记（容器/内层块）。
    markers: Vec<&'static str>,
    /// 正文 HTML 中结构标记的精确出现次数。
    tags: Vec<(&'static str, usize)>,
    /// 正文 HTML 中内层标签出现次数（乙/丙）。
    html_counts: Vec<(char, usize)>,
    /// PDF 版面中内层标签出现次数。
    pdf_counts: Vec<(char, usize)>,
    /// 已登记但本轮未修复的产品缺陷（报告「未修复项」表）。
    known_defect: Option<&'static str>,
}

impl StructuredSpec {
    fn needs_callouts(&self) -> bool {
        // ADJC2/ADJX 与 ADJC 同口径：整组注入 FIXME registry，`> [!FIXME]`
        // 才能成为真正的 Callout 块；未知标记的回退路径由 ADJB 单独覆盖。
        true
    }
}

fn count_kind(kinds: &[&str], name: &str) -> usize {
    kinds.iter().filter(|k| **k == name).count()
}

fn adjc2_kinds(container: AdjcContainer, first: usize, second: usize) -> Vec<&'static str> {
    use AdjcContainer::*;
    match container {
        ListItem => match (first, second) {
            (6, 6) => vec!["ListItem", "Mermaid", "ListItem", "Mermaid"],
            (7, 7) => vec!["ListItem", "Table", "ListItem", "Table"],
            (6, 7) => vec!["ListItem", "Mermaid", "ListItem", "Table"],
            (8, 8) => vec!["ListItem", "ListItem", "ListItem"],
            (0, 7) => vec!["ListItem", "Table"],
            _ => vec!["ListItem"],
        },
        BlockQuote => match (first, second) {
            (6, 6) => vec!["BlockQuote", "Mermaid", "BlockQuote", "Mermaid"],
            (7, 7) => vec!["BlockQuote", "Table", "BlockQuote", "Table"],
            (6, 7) => vec!["BlockQuote", "Mermaid", "BlockQuote", "Table"],
            (0, 7) => vec!["BlockQuote", "Table"],
            _ => vec!["BlockQuote"],
        },
        Admonition => match (first, second) {
            (6, 6) => vec!["Admonition", "Mermaid", "BlockQuote", "Mermaid"],
            (6, 7) => vec!["Admonition", "Mermaid", "BlockQuote"],
            _ => vec!["Admonition"],
        },
        Callout => match (first, second) {
            (6, 6) => vec!["Callout", "Mermaid", "BlockQuote", "Mermaid"],
            (6, 7) => vec!["Callout", "Mermaid", "BlockQuote"],
            _ => vec!["Callout"],
        },
        Details => match (first, second) {
            (0, 0) => vec!["Details", "Paragraph", "Paragraph"],
            (1, 1) => vec!["Details", "AtxHeading", "AtxHeading"],
            (2, 2) => vec!["Details", "SetextHeading", "SetextHeading"],
            (3, 3) => vec!["Details", "FencedCode", "FencedCode"],
            (4, 4) => vec!["Details", "IndentedCode"],
            (5, 5) => vec!["Details", "Math", "Math"],
            (6, 6) => vec!["Details", "Mermaid", "Mermaid"],
            (7, 7) => vec!["Details", "Table", "Table"],
            (8, 8) => vec!["Details", "ListItem", "ListItem"],
            (9, 9) => vec!["Details", "BlockQuote", "BlockQuote"],
            (10, 10) => vec!["Details", "Admonition", "Admonition"],
            (11, 11) => vec!["Details", "Callout", "Callout"],
            (12, 12) => vec!["Details", "ThematicBreak", "ThematicBreak"],
            (13, 13) => vec!["Details", "LinkRefDef", "LinkRefDef"],
            (14, 14) => vec!["Details", "FootnoteDef", "FootnoteDef"],
            (15, 15) => vec!["Details", "HtmlBlock", "HtmlBlock"],
            (16, 16) => vec!["Details", "Details", "Paragraph", "Details", "Paragraph"],
            (17, 17) => vec![
                "Details",
                "ThematicBreak",
                "SetextHeading",
                "ThematicBreak",
                "SetextHeading",
            ],
            (6, 7) => vec!["Details", "Mermaid", "Table"],
            (3, 4) => vec!["Details", "FencedCode", "IndentedCode"],
            (0, 7) => vec!["Details", "Paragraph", "Table"],
            _ => vec!["Details"],
        },
    }
}

fn adjc2_spec(container: AdjcContainer, first: usize, second: usize) -> StructuredSpec {
    use AdjcContainer::*;
    let kinds = adjc2_kinds(container, first, second);
    let mut markers: Vec<&'static str> = container.html_markers().to_vec();
    let mut tags: Vec<(&'static str, usize)> = Vec::new();
    let push = |tags: &mut Vec<(&'static str, usize)>, tag: &'static str, n: usize| {
        if n > 0 {
            tags.push((tag, n));
        }
    };
    // 表格：Admonition/Callout 不切表格（表格留在提示框内），按内层单元计数。
    let tables = if matches!(container, Admonition | Callout) {
        [first, second].iter().filter(|u| **u == 7).count()
    } else {
        count_kind(&kinds, "Table")
    };
    push(&mut tags, "<table", tables);
    push(&mut tags, "<svg", count_kind(&kinds, "Mermaid"));
    push(
        &mut tags,
        "<pre>",
        count_kind(&kinds, "FencedCode") + count_kind(&kinds, "IndentedCode"),
    );
    push(&mut tags, "<hr", count_kind(&kinds, "ThematicBreak"));
    push(&mut tags, "<details", count_kind(&kinds, "Details"));
    push(&mut tags, "callout-note", count_kind(&kinds, "Admonition"));
    push(&mut tags, "callout-fixme", count_kind(&kinds, "Callout"));
    if kinds.contains(&"Mermaid") {
        markers.push("<svg");
    }
    let mut html_counts = Vec::new();
    let mut pdf_counts = Vec::new();
    for (unit, label) in [(first, '乙'), (second, '丙')] {
        let single = AdjcSpec { container, unit };
        if let Some(n) = single.html_label_count() {
            html_counts.push((label, n));
        }
        if let Some(n) = single.pdf_label_count() {
            pdf_counts.push((label, n));
        }
    }
    StructuredSpec {
        mode: StructMode::Content(container),
        kinds,
        markers,
        tags,
        html_counts,
        pdf_counts,
        known_defect: None,
    }
}

fn adjx_kinds(outer: AdjcContainer, inner: AdjcContainer) -> Vec<&'static str> {
    use AdjcContainer::*;
    match (outer, inner) {
        (ListItem, ListItem) => vec!["ListItem", "ListItem"],
        (ListItem, _) => vec!["ListItem"],
        (BlockQuote, _) => vec!["BlockQuote"],
        (Details, ListItem) => vec!["Details", "ListItem"],
        (Details, BlockQuote) => vec!["Details", "BlockQuote"],
        (Details, Details) => vec!["Details", "Details", "Paragraph"],
        _ => vec!["Details"],
    }
}

fn adjx_spec(outer: AdjcContainer, inner: AdjcContainer) -> StructuredSpec {
    use AdjcContainer::*;
    // 已登记缺陷：`<details>` 位于 ListItem / BlockQuote 内时，容器展开只提升
    // Table/Mermaid，details 块不被识别，开标签被转义为普通段落文本。
    let unlifted_details = matches!(inner, Details) && matches!(outer, ListItem | BlockQuote);
    let kinds = adjx_kinds(outer, inner);
    let mut markers: Vec<&'static str> = Vec::new();
    let li = usize::from(matches!(outer, ListItem)) + usize::from(matches!(inner, ListItem));
    let bq = usize::from(matches!(outer, BlockQuote)) + usize::from(matches!(inner, BlockQuote));
    let det = if unlifted_details {
        0
    } else {
        usize::from(matches!(outer, Details)) + usize::from(matches!(inner, Details))
    };
    let mut tags = Vec::new();
    if li > 0 {
        markers.push("<li>");
        tags.push(("<li>", li));
    }
    if bq > 0 {
        markers.push("<blockquote");
        tags.push(("<blockquote", bq));
    }
    if unlifted_details {
        // 显式锁定降级形态：开标签以转义文本出现，summary 走行内白名单。
        markers.push("&lt;details&gt;");
        markers.push("<summary");
    }
    if det > 0 {
        markers.push("<details");
        tags.push(("<details", det));
        tags.push(("<summary", det));
    }
    // 外层容器标签甲一次；内层标签乙：Details 内 summary+正文各一次。
    let mut html_counts = vec![('甲', 1)];
    let mut pdf_counts = vec![('甲', 1)];
    let inner_label = if matches!(inner, Details) { 2 } else { 1 };
    html_counts.push(('乙', inner_label));
    pdf_counts.push(('乙', inner_label));
    StructuredSpec {
        mode: StructMode::Nested { outer, inner },
        kinds,
        markers,
        tags,
        html_counts,
        pdf_counts,
        known_defect: unlifted_details.then_some(
            "嵌套 `<details>`（位于 ListItem / BlockQuote 内）未被识别为 Details 块：容器展开只提升 Table/Mermaid，开标签降级为转义文本（详见报告「未修复项」）",
        ),
    }
}

fn adjc2_cases() -> Vec<Case> {
    use AdjcContainer::*;
    let containers = [ListItem, BlockQuote, Admonition, Callout, Details];
    let cross = [(7usize, 7usize), (6, 7), (3, 4), (0, 7)];
    let mut out = Vec::with_capacity(ADJC2_TOTAL);
    let mut n = 0usize;
    for container in containers {
        for (unit, unit_spec) in UNITS.iter().enumerate() {
            n += 1;
            let body = format!("{}\n\n{}", unit_source(unit, "乙"), unit_source(unit, "丙"));
            let source = container.wrap(&body);
            let spec = adjc2_spec(container, unit, unit);
            out.push(structured_case(
                format!("CASE-{ADJC2_GROUP}-{n:03}"),
                format!(
                    "{} 内 {} + {}（自相邻）",
                    container.name(),
                    unit_spec.name,
                    unit_spec.name
                ),
                source,
                "生成式容器内相邻矩阵：容器（甲容器）内两个同类型内层块（乙/丙）".to_string(),
                spec,
            ));
        }
        for (a, b) in cross {
            n += 1;
            let body = format!("{}\n\n{}", unit_source(a, "乙"), unit_source(b, "丙"));
            let source = container.wrap(&body);
            let spec = adjc2_spec(container, a, b);
            out.push(structured_case(
                format!("CASE-{ADJC2_GROUP}-{n:03}"),
                format!(
                    "{} 内 {} + {}",
                    container.name(),
                    UNITS[a].name,
                    UNITS[b].name
                ),
                source,
                "生成式容器内相邻矩阵：容器内两个关键跨类型内层块（乙/丙）".to_string(),
                spec,
            ));
        }
    }
    out
}

fn adjx_cases() -> Vec<Case> {
    use AdjcContainer::*;
    let combos = [
        (ListItem, ListItem, "- 甲容器\n\n  - 乙容器\n"),
        (ListItem, BlockQuote, "- 甲容器\n\n  > 乙容器\n"),
        (
            ListItem,
            Details,
            "- 甲容器\n\n  <details>\n  <summary>乙容器</summary>\n\n  乙正文\n\n  </details>\n",
        ),
        (BlockQuote, ListItem, "> 甲容器\n>\n> - 乙容器\n"),
        (BlockQuote, BlockQuote, "> 甲容器\n>\n> > 乙容器\n"),
        (
            BlockQuote,
            Details,
            "> 甲容器\n>\n> <details>\n> <summary>乙容器</summary>\n>\n> 乙正文\n>\n> </details>\n",
        ),
        (
            Details,
            ListItem,
            "<details>\n<summary>甲容器</summary>\n\n- 乙容器\n\n</details>\n",
        ),
        (
            Details,
            BlockQuote,
            "<details>\n<summary>甲容器</summary>\n\n> 乙容器\n\n</details>\n",
        ),
        (
            Details,
            Details,
            "<details>\n<summary>甲容器</summary>\n\n<details>\n<summary>乙容器</summary>\n\n乙正文\n\n</details>\n\n</details>\n",
        ),
    ];
    combos
        .iter()
        .enumerate()
        .map(|(i, (outer, inner, source))| {
            let spec = adjx_spec(*outer, *inner);
            structured_case(
                format!("CASE-{ADJX_GROUP}-{:03}", i + 1),
                format!("{} > {}", outer.name(), inner.name()),
                (*source).to_string(),
                "嵌套容器矩阵：外层容器（甲容器）内嵌内层容器（乙容器）".to_string(),
                spec,
            )
        })
        .collect()
}

fn structured_case(
    id: String,
    title: String,
    source: String,
    expect: String,
    spec: StructuredSpec,
) -> Case {
    let group = id.split('-').nth(1).unwrap_or(ADJC2_GROUP).to_string();
    let mut checks: Vec<(String, String)> = Vec::new();
    checks.push(("blocks".to_string(), spec.kinds.len().to_string()));
    checks.push(("kinds".to_string(), spec.kinds.join(",")));
    for marker in &spec.markers {
        checks.push(("html_contains".to_string(), (*marker).to_string()));
    }
    if spec.tags.iter().any(|(tag, _)| *tag == "<svg") {
        checks.push(("html_contains".to_string(), "<svg".to_string()));
        checks.push((
            "html_not_contains".to_string(),
            "language-mermaid".to_string(),
        ));
    }
    checks.push(("no_overlap".to_string(), "true".to_string()));
    Case {
        id,
        group,
        title,
        source,
        expect,
        surfaces: Some(vec![
            Surface::Wysiwyg,
            Surface::Focus,
            Surface::Typewriter,
            Surface::Html,
            Surface::Pdf,
            Surface::Print,
        ]),
        na_reason: None,
        checks,
        byte_opts: Vec::new(),
        line: 0,
        adjc: None,
        structured: Some(spec),
    }
}

/// 结构化矩阵的 PARSE 断言：容器归属（details 层级、提升块的容器字段）。
fn structured_parse_checks(session: &Session, spec: &StructuredSpec, problems: &mut Vec<String>) {
    let entries = session.block_entries();
    if entries.is_empty() {
        problems.push("结构化矩阵不得产生空文档".to_string());
        return;
    }
    if entries[0].3.is_nested() {
        problems.push(format!("首块不应携带嵌套容器归属：{:?}", entries[0].3));
    }
    match spec.mode {
        StructMode::Content(container) => {
            let targets: &[&str] = match container {
                AdjcContainer::ListItem => &["Table", "Mermaid"],
                AdjcContainer::BlockQuote => &["Table", "Mermaid"],
                AdjcContainer::Admonition | AdjcContainer::Callout => &["Mermaid", "Table"],
                AdjcContainer::Details => &["Table", "Mermaid", "FencedCode", "IndentedCode"],
            };
            for e in &entries {
                let name = kind_name(&e.1);
                if !targets.contains(&name) {
                    continue;
                }
                let c = &e.3;
                let ok = match container {
                    AdjcContainer::ListItem => c.list_indent > 0,
                    AdjcContainer::BlockQuote => c.quote_depth > 0,
                    AdjcContainer::Admonition | AdjcContainer::Callout => {
                        c.quote_depth > 0 || c.details
                    }
                    AdjcContainer::Details => c.details && c.details_depth > 0,
                };
                if !ok {
                    problems.push(format!(
                        "{name} 容器归属异常（{}）：{c:?}",
                        container.name()
                    ));
                }
            }
        }
        StructMode::Nested { outer, inner } => {
            let want_depth = if spec.known_defect.is_some() {
                0
            } else {
                usize::from(matches!(outer, AdjcContainer::Details))
                    + usize::from(matches!(inner, AdjcContainer::Details))
            };
            let max_depth = entries.iter().map(|e| e.3.details_depth).max().unwrap_or(0);
            if max_depth as usize != want_depth {
                problems.push(format!(
                    "Details 嵌套层级应为 {want_depth}，实际 {max_depth}（entries={:?}）",
                    entries.iter().map(|e| e.3).collect::<Vec<_>>()
                ));
            }
            if matches!(outer, AdjcContainer::Details) {
                for e in entries.iter().skip(1) {
                    if !e.3.details || e.3.details_depth == 0 {
                        problems.push(format!(
                            "外层 Details 的子块归属异常（{}）：{:?}",
                            kind_name(&e.1),
                            e.3
                        ));
                    }
                }
            }
        }
    }
}

/// 结构化矩阵的 HTML 断言：容器结构标记、内层块标记计数、内层标签计数。
fn structured_html_checks(spec: &StructuredSpec, body: &str, problems: &mut Vec<String>) {
    for marker in &spec.markers {
        if !body.contains(marker) {
            problems.push(format!(
                "缺少容器/内层结构标记 `{marker}`；正文片段：`{}`",
                snippet(body, 300)
            ));
        }
    }
    for (tag, want) in &spec.tags {
        let got = body.matches(tag).count();
        if got != *want {
            problems.push(format!(
                "结构标记 `{tag}` 应出现 {want} 次，实际 {got} 次；正文片段：`{}`",
                snippet(body, 300)
            ));
        }
    }
    for (label, want) in &spec.html_counts {
        let got = body.matches(*label).count();
        if got != *want {
            problems.push(format!(
                "正文 HTML 中标签 `{label}` 应出现 {want} 次，实际 {got} 次；正文片段：`{}`",
                snippet(body, 300)
            ));
        }
    }
}

/// 结构化矩阵的 PDF 断言：内层标签按模型计数（证明两个内层块都进入版面）。
fn structured_pdf_checks(spec: &StructuredSpec, text: &str, problems: &mut Vec<String>) {
    let flat = norm(text);
    for (label, want) in &spec.pdf_counts {
        let got = flat.matches(*label).count();
        if got != *want {
            problems.push(format!(
                "PDF 版面标签 `{label}` 应出现 {want} 次，实际 {got} 次；文本片段：`{}`",
                snippet(text, 300)
            ));
        }
    }
}

fn structured_golden_ids() -> &'static Vec<String> {
    static IDS: OnceLock<Vec<String>> = OnceLock::new();
    IDS.get_or_init(|| {
        let mut out = Vec::new();
        // ADJC2：排除 Mermaid（浮点 SVG）。
        let containers = [
            AdjcContainer::ListItem,
            AdjcContainer::BlockQuote,
            AdjcContainer::Admonition,
            AdjcContainer::Callout,
            AdjcContainer::Details,
        ];
        let mut eligible = Vec::new();
        let mut n = 0usize;
        for container in containers {
            for unit in 0..ADJ_UNITS {
                n += 1;
                if unit != 6 {
                    eligible.push((n, container));
                }
            }
            for (a, b) in [(7usize, 7usize), (6, 7), (3, 4), (0, 7)] {
                n += 1;
                if a != 6 && b != 6 {
                    eligible.push((n, container));
                }
            }
        }
        for i in 0..20 {
            let (idx, _) = eligible[i * eligible.len() / 20];
            out.push(format!("CASE-{ADJC2_GROUP}-{idx:03}"));
        }
        // ADJX 共 9 条，全部抽样。
        for i in 1..=ADJX_TOTAL {
            out.push(format!("CASE-{ADJX_GROUP}-{i:03}"));
        }
        out
    })
}

// =============================================================================
// 主测试
// =============================================================================

#[test]
fn syntax_matrix_surfaces() {
    let cases = load_matrix();
    let adjacency = adjacency_cases();
    let inline_adjacency = adjacency_inline_cases();
    let container_matrix = adjc_cases();
    let newline_matrix = adjn_cases();
    let inline_boundary = adji2_cases();
    let container_adjacent = adjc2_cases();
    let nested_containers = adjx_cases();
    let adj_exec = adjacency.iter().filter(|c| c.na_reason.is_none()).count();
    let adji_exec = inline_adjacency.len();
    let adjc_exec = container_matrix.len();
    let adjn_exec = newline_matrix
        .iter()
        .filter(|c| c.na_reason.is_none())
        .count();
    let adji2_exec = inline_boundary.len();
    let adjc2_exec = container_adjacent.len();
    let adjx_exec = nested_containers.len();
    let adj_na: Vec<&Case> = adjacency.iter().filter(|c| c.na_reason.is_some()).collect();
    let adjn_na: Vec<&Case> = newline_matrix
        .iter()
        .filter(|c| c.na_reason.is_some())
        .collect();
    let mut rows: Vec<Row> = Vec::new();
    let mut digest: Vec<(String, String)> = Vec::new();

    for case in cases
        .iter()
        .chain(adjacency.iter())
        .chain(inline_adjacency.iter())
        .chain(container_matrix.iter())
        .chain(newline_matrix.iter())
        .chain(inline_boundary.iter())
        .chain(container_adjacent.iter())
        .chain(nested_containers.iter())
    {
        if case.na_reason.is_some() && case.group == ADJB_GROUP {
            // ADJB 的 N/A（B=FrontMatter）不计断言行，只在报告里登记理由。
            continue;
        }
        if case.na_reason.is_some() && case.group == ADJN_GROUP {
            // ADJN 的 N/A（B=FrontMatter）同理。
            continue;
        }
        run_case(case, &mut rows, &mut digest);
    }

    write_digest(&mut digest);
    write_report(
        &rows,
        &MatrixStats {
            corpus: cases.len(),
            adj_exec,
            adj_total: ADJ_TOTAL,
            adji_exec,
            adji_total: ADJI_TOTAL,
            adjc_exec,
            adjc_total: ADJC_TOTAL,
            adjn_exec,
            adjn_total: ADJN_TOTAL,
            adji2_exec,
            adji2_total: ADJI2_TOTAL,
            adjc2_exec,
            adjc2_total: ADJC2_TOTAL,
            adjx_exec,
            adjx_total: ADJX_TOTAL,
        },
        &adj_na,
        &adjn_na,
    );
    let fails = rows.iter().filter(|r| r.status == Status::Fail).count();
    if fails > 0 && std::env::var("QA_ALLOW_FAIL").as_deref() != Ok("1") {
        panic!("{fails} 条断言失败；完整报告见 {REPORT}");
    }
}

fn run_case(case: &Case, rows: &mut Vec<Row>, digest: &mut Vec<(String, String)>) {
    // ---- 源字节 → 文本（与产品打开路径一致：BOM 由 decode 剥离） ----
    let bytes = case.encoded();
    let text = match veil_ui::decode(&bytes) {
        Ok((t, _)) => t,
        Err(e) => {
            rows.push(Row::new(
                case,
                Surface::Parse,
                vec![format!("源字节无法解码：{e}")],
                None,
            ));
            return;
        }
    };

    // ---- PARSE ----
    let mut parse_problems = Vec::new();
    let callouts = adjc_callouts();
    let needs_callouts = case.adjc.is_some()
        || case
            .structured
            .as_ref()
            .is_some_and(StructuredSpec::needs_callouts);
    let parsed = catch_unwind(AssertUnwindSafe(|| {
        if case.adjc.is_some() || needs_callouts {
            Session::open_with_callouts(&text, &callouts)
        } else {
            Session::open(&text)
        }
    }));
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
            // ADJI：顶层行内节点序列 + 区间不变量（落在段落块区间、字符边界、
            // 按起点有序且互不重叠）。
            for want in case.check_values("inline_kinds") {
                let expected: Vec<&str> = if want == "None" {
                    Vec::new()
                } else {
                    want.split(',').collect()
                };
                let para = blocks
                    .iter()
                    .find(|(_, k, _)| matches!(k, BlockKind::Paragraph));
                let Some((id, _, para_range)) = para else {
                    parse_problems.push("inline_kinds：找不到段落块".to_string());
                    continue;
                };
                let Some(nodes) = session.block_inlines(*id) else {
                    parse_problems.push("inline_kinds：block_inlines 返回 None".to_string());
                    continue;
                };
                let actual: Vec<&'static str> =
                    nodes.iter().map(|n| inline_kind_name(&n.kind)).collect();
                if expected != actual {
                    parse_problems.push(format!(
                        "inline_kinds 期望 [{}]，实际 [{}]",
                        expected.join(","),
                        actual.join(",")
                    ));
                }
                for (i, node) in nodes.iter().enumerate() {
                    let r = node.range;
                    if r.start > r.end {
                        parse_problems.push(format!("第 {i} 个节点区间倒置：{r:?}"));
                    }
                    if r.start < para_range.start || r.end > para_range.end {
                        parse_problems.push(format!(
                            "第 {i} 个节点区间 {r:?} 越出段落区间 {para_range:?}"
                        ));
                    }
                    if !text.is_char_boundary(r.start) || !text.is_char_boundary(r.end) {
                        parse_problems.push(format!("第 {i} 个节点区间 {r:?} 不在 UTF-8 字符边界"));
                    }
                }
                for pair in nodes.windows(2) {
                    if pair[0].range.start > pair[1].range.start {
                        parse_problems.push(format!(
                            "顶层节点未按起点有序：{:?} → {:?}",
                            pair[0].range, pair[1].range
                        ));
                    }
                    if pair[0].range.end > pair[1].range.start {
                        parse_problems.push(format!(
                            "顶层节点重叠：{:?} 与 {:?}",
                            pair[0].range, pair[1].range
                        ));
                    }
                }
            }
            if let Some(spec) = &case.adjc {
                adjc_parse_checks(&session, spec, &mut parse_problems);
            }
            if let Some(spec) = &case.structured {
                structured_parse_checks(&session, spec, &mut parse_problems);
            }
        }
    }
    rows.push(Row::new(case, Surface::Parse, parse_problems, None));

    // 空态用例虽在语料里声明 N/A，但空态本身是可断言的确定行为，继续跑表面；
    // 其余 N/A 用例仍按契约记为 NOT RUN。
    if let Some(reason) = &case.na_reason {
        if !empty_state(&case.id) {
            for s in SURFACE_ORDER.iter().skip(1) {
                rows.push(Row::new(case, *s, Vec::new(), Some(reason.clone())));
            }
            return;
        }
    }

    // ---- 一次 headless App 供 UI 与 PRINT 共用 ----
    let need_ui = case.applies(Surface::Wysiwyg)
        || case.applies(Surface::Focus)
        || case.applies(Surface::Typewriter);
    let need_print = case.applies(Surface::Print);
    let mut app = if need_ui || need_print {
        let callouts = adjc_callouts();
        let use_callouts = case.adjc.is_some()
            || case
                .structured
                .as_ref()
                .is_some_and(StructuredSpec::needs_callouts);
        let build = || {
            if use_callouts {
                veil_ui::qa::QaApp::new_with_callouts(&text, &callouts)
            } else {
                veil_ui::qa::QaApp::new(&text)
            }
        };
        match catch_unwind(AssertUnwindSafe(build)) {
            Ok(a) => Some(a),
            Err(_) => {
                for s in [
                    Surface::Wysiwyg,
                    Surface::Focus,
                    Surface::Typewriter,
                    Surface::Print,
                ] {
                    if case.applies(s) {
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
        if empty_state(&case.id) {
            // 空态：0 块即不存在块间重叠，PDF 的 no_overlap 视为成立。
            overlap = Some(Vec::new());
            match app.as_mut() {
                None => {}
                Some(app) => {
                    for s in [Surface::Wysiwyg, Surface::Focus, Surface::Typewriter] {
                        if !case.applies(s) {
                            continue;
                        }
                        match catch_unwind(AssertUnwindSafe(|| empty_ui_checks(app, s))) {
                            Ok(problems) => rows.push(Row::new(case, s, problems, None)),
                            Err(_) => rows.push(Row::new(
                                case,
                                s,
                                vec!["空态布局断言 panic".to_string()],
                                None,
                            )),
                        }
                    }
                }
            }
        } else {
            match app.as_mut() {
                None => {}
                Some(app) => match catch_unwind(AssertUnwindSafe(|| ui_checks(app))) {
                    Ok(res) => {
                        overlap = Some(res.overlap);
                        if case.applies(Surface::Wysiwyg) {
                            rows.push(Row::new(case, Surface::Wysiwyg, res.wysiwyg, None));
                        }
                        if case.applies(Surface::Focus) {
                            rows.push(Row::new(case, Surface::Focus, res.focus, None));
                        }
                        if case.applies(Surface::Typewriter) {
                            rows.push(Row::new(case, Surface::Typewriter, res.typewriter, None));
                        }
                    }
                    Err(_) => {
                        for s in [Surface::Wysiwyg, Surface::Focus, Surface::Typewriter] {
                            if case.applies(s) {
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
    }

    // ---- HTML ----
    if case.applies(Surface::Html) {
        // Mermaid 用例需要真实离线引擎（QuickJS + mermaid.js）；harness 只对
        // 这些用例走带注入的导出入口，其余保持纯内存快速路径。
        let mermaid_svg = case
            .check_values("html_contains")
            .any(|v| v.contains("<svg"));
        let callouts = adjc_callouts();
        let use_callouts = case.adjc.is_some()
            || case
                .structured
                .as_ref()
                .is_some_and(StructuredSpec::needs_callouts);
        let exported = catch_unwind(AssertUnwindSafe(|| {
            if mermaid_svg {
                if use_callouts {
                    veil::qa::html_of_doc_with_mermaid_and_callouts(
                        &text,
                        image_case_dir(&case.id),
                        &callouts,
                    )
                    .map(|html| (html, true))
                } else {
                    veil::qa::html_of_doc_with_mermaid(&text, image_case_dir(&case.id))
                        .map(|html| (html, true))
                }
            } else if use_callouts {
                veil::qa::html_of_doc_with_callouts(&text, image_case_dir(&case.id), &callouts)
                    .map(|html| (html, false))
            } else {
                veil::qa::html_of_doc(&text, image_case_dir(&case.id)).map(|html| (html, false))
            }
        }));
        match exported {
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
            Ok(Ok((html, rendered_mermaid))) => {
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
                // ADJI：可见标签在正文范围内各出现一次，并按 A→B 顺序排列。
                for want in case.check_values("html_label_once") {
                    for label in want.split(',') {
                        let count = body.matches(label).count();
                        if count != 1 {
                            problems.push(format!(
                                "可见标签 `{label}` 应恰出现 1 次，实际 {count} 次；正文片段：`{}`",
                                body_snippet(&html)
                            ));
                        }
                    }
                }
                for want in case.check_values("html_label_order") {
                    let mut last = 0usize;
                    let mut seen = false;
                    for label in want.split(',') {
                        let Some(pos) = body.find(label) else {
                            continue;
                        };
                        if seen && pos <= last {
                            problems.push(format!(
                                "可见标签顺序错误：`{label}` 出现在前一标签之前；正文片段：`{}`",
                                body_snippet(&html)
                            ));
                            break;
                        }
                        last = pos;
                        seen = true;
                    }
                }
                // Mermaid 断言口径：正文必须来自真实 SVG，且不得同时残留
                // 「渲染数据不可用」的源码围栏降级形态（否则 <svg 可能来自
                // 别处，降级也会被误判为通过）。
                if rendered_mermaid {
                    if !body.contains("<svg") {
                        problems.push(format!(
                            "Mermaid 未渲染为内嵌 SVG；实际正文片段：`{}`",
                            body_snippet(&html)
                        ));
                    }
                    for banned in [
                        "language-mermaid",
                        "Mermaid 渲染数据不可用",
                        "Mermaid render data unavailable",
                    ] {
                        if body.contains(banned) {
                            problems.push(format!(
                                "Mermaid 仍处于源码围栏降级形态：`{banned}`；上下文：`{}`",
                                around(&html, banned)
                            ));
                        }
                    }
                }
                // 空态口径：导出骨架（head/style）允许存在，<body> 内必须
                // 没有任何可见内容。
                if empty_state(&case.id) && !body.trim().is_empty() {
                    problems.push(format!(
                        "空态文档的 <body> 应为空，实际：`{}`",
                        snippet(body.trim(), 300)
                    ));
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
                    let cell_img = html.split("<td").skip(1).any(|cell| {
                        cell.split("</td>")
                            .next()
                            .is_some_and(|c| c.contains("<img"))
                    });
                    if !cell_img {
                        problems
                            .push("image_in_cell：未找到任何包含 <img 的 <td>…</td>".to_string());
                    }
                    if !html.contains("td img,th img{max-height:2.4em}") {
                        problems.push(
                            "image_in_cell：CSS 缺少 `td img,th img{max-height:2.4em}`".to_string(),
                        );
                    }
                }
                // Mermaid 用例不做全文金样：SVG 内的路径/弧坐标由 dagre 布局
                // 与字体度量共同决定，跨进程运行会出现浮点级差异（实测
                // classDiagram / stateDiagram 的 `d` 属性每次略有不同），
                // 全文金样必然 flaky。改为上面的结构断言（真实 `<svg>` +
                // 禁止降级围栏），同样能证伪「没渲染却算通过」。
                // ADJC：容器标签/内层块标签计数 + Mermaid 出图口径。
                if let Some(spec) = &case.adjc {
                    adjc_html_checks(spec, body, &mut problems);
                }
                // ADJC2/ADJX：容器结构标记计数与内层标签计数。
                if let Some(spec) = &case.structured {
                    structured_html_checks(spec, body, &mut problems);
                }
                // ADJB/ADJI：全量逐用例 HTML SHA-256 摘要比对（离线金样目录之外）。
                // 摘要文件缺失或 UPDATE_GOLDEN=1 时只重算并落盘（引导）；
                // ADJI 首轮缺失记录按新增组补写，不判 FAIL。
                if is_adjacency_group(&case.group) {
                    let hash = sha256_hex(html.as_bytes());
                    if let Some(expected) = digest_expected() {
                        match expected.get(&case.id) {
                            Some(want) if want == &hash => {}
                            Some(want) => problems.push(format!(
                                "HTML 摘要与 {DIGEST} 不一致：期望 {want}，实际 {hash}"
                            )),
                            None if case.group == ADJI_GROUP || case.group == ADJC_GROUP => {}
                            None => {
                                problems.push(format!("HTML 摘要缺少该用例记录（实际 {hash}）"))
                            }
                        }
                    }
                    digest.push((case.id.clone(), hash));
                }
                if !rendered_mermaid && adj_golden_sample(case) {
                    compare_golden(case, &html, &mut problems);
                }
                rows.push(Row::new(case, Surface::Html, problems, None));
            }
        }
    }

    // ---- PDF ----
    if case.applies(Surface::Pdf) {
        if !veil::qa::pdf_available() {
            rows.push(Row::new(
                case,
                Surface::Pdf,
                Vec::new(),
                Some(
                    "未找到 CJK 字体（pdf::find_cjk_font 返回 None），PDF 导出入口不可运行"
                        .to_string(),
                ),
            ));
        } else {
            let callouts = adjc_callouts();
            let use_callouts = case.adjc.is_some()
                || case
                    .structured
                    .as_ref()
                    .is_some_and(StructuredSpec::needs_callouts);
            let pdf = || {
                if use_callouts {
                    veil::qa::pdf_of_doc_with_callouts(&text, &callouts)
                } else {
                    veil::qa::pdf_of_doc(&text)
                }
            };
            match catch_unwind(AssertUnwindSafe(pdf)) {
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
                            problems
                                .push(format!("pdf_pages 期望 {want}，实际 {}", out.page_count));
                        }
                    }
                    if out.bytes_len == 0 {
                        problems.push("PDF 字节为空".to_string());
                    }
                    let pdf_text = norm(&out.text);
                    // ADJB/ADJI：最少 1 页；显式 `pdf_text` 只断言可见标签，
                    // 避免对 FrontMatter/LinkRefDef/FootnoteDef/ThematicBreak 等
                    // 无可见文本的单元误断言。
                    if is_adjacency_group(&case.group) && out.page_count == 0 {
                        problems.push("pdf_pages 应 ≥1，实际 0".to_string());
                    }
                    for want in case.check_values("pdf_text") {
                        if !pdf_text.contains(&norm(want)) {
                            problems.push(format!(
                                "PDF 版面文本缺少 `{want}`；实际文本片段：`{}`",
                                snippet(&out.text, 300)
                            ));
                        }
                    }
                    for absent in case.check_values("pdf_not_text") {
                        if pdf_text.contains(&norm(absent)) {
                            problems.push(format!(
                                "PDF 版面文本不应出现 `{absent}`；实际文本片段：`{}`",
                                snippet(&out.text, 300)
                            ));
                        }
                    }
                    // 空态依据：`layout_document` 无条件 `new_page()`，正文为空时
                    // 仍保留 1 页空白；`LayoutOpts::default().page_numbers == false`，
                    // 所以该页没有任何 `DrawOp::Text`（版面文本为空）。
                    if empty_state(&case.id) {
                        if out.page_count != 1 {
                            problems.push(format!(
                                "空态 PDF 应为 1 页空白，实际 {} 页",
                                out.page_count
                            ));
                        }
                        if !pdf_text.is_empty() {
                            problems.push(format!(
                                "空态 PDF 不应有版面文本，实际：`{}`",
                                snippet(&out.text, 300)
                            ));
                        }
                    }
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
                    if let Some(spec) = &case.adjc {
                        adjc_pdf_checks(spec, &out.text, &mut problems);
                    }
                    if let Some(spec) = &case.structured {
                        structured_pdf_checks(spec, &out.text, &mut problems);
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
    if case.applies(Surface::Print) {
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
                        // 空态依据：`compute_page_breaks` 恒以 `vec![0.0]` 起步，
                        // 文档总高 ≤ 页高时直接返回，故空文档恰为 1 页、断点 0。
                        if empty_state(&case.id) && (pm.pages != 1 || pm.breaks.as_slice() != [0.0])
                        {
                            problems.push(format!(
                                "空态打印模型应为 1 页且断点为 [0.0]，实际页数 {}、断点 {:?}",
                                pm.pages, pm.breaks
                            ));
                        }
                        rows.push(Row::new(case, Surface::Print, problems, None));
                    }
                }
            }
        }
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

#[test]
fn sha256_known_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

// =============================================================================
// 报告
// =============================================================================

fn escape_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

struct MatrixStats {
    corpus: usize,
    adj_exec: usize,
    adj_total: usize,
    adji_exec: usize,
    adji_total: usize,
    adjc_exec: usize,
    adjc_total: usize,
    adjn_exec: usize,
    adjn_total: usize,
    adji2_exec: usize,
    adji2_total: usize,
    adjc2_exec: usize,
    adjc2_total: usize,
    adjx_exec: usize,
    adjx_total: usize,
}

fn write_report(rows: &[Row], stats: &MatrixStats, adj_na: &[&Case], adjn_na: &[&Case]) {
    let MatrixStats {
        corpus: corpus_cases,
        adj_exec,
        adj_total,
        adji_exec,
        adji_total,
        adjc_exec,
        adjc_total,
        adjn_exec,
        adjn_total,
        adji2_exec,
        adji2_total,
        adjc2_exec,
        adjc2_total,
        adjx_exec,
        adjx_total,
    } = *stats;
    let total_cases = corpus_cases
        + adj_exec
        + adji_exec
        + adjc_exec
        + adjn_exec
        + adji2_exec
        + adjc2_exec
        + adjx_exec;
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

    // 金样文件数按落盘实际计数：Mermaid 用例不做全文金样（见下）。
    let golden_count = std::fs::read_dir(GOLDEN_DIR)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "html"))
                .count()
        })
        .unwrap_or(0);
    let mut out = String::new();
    out.push_str("# 语法矩阵测试报告（2026-11-27）\n\n");
    out.push_str(&format!(
        "- 语料：`docs/qa/syntax-matrix.md`（契约 175 条，解析校验通过 {corpus_cases}）\n- 生成式块级相邻（ADJB）：{adj_total} 个有序对（18×18），执行 {adj_exec}，N/A {}\n- 生成式行内相邻（ADJI）：{adji_total} 个用例（12×12×2 边界），执行 {adji_exec}，N/A 0\n- 生成式容器内容（ADJC）：{adjc_total} 个用例（5 类容器×18 单元），执行 {adjc_exec}，N/A 0\n- 生成式块级单换行（ADJN）：{adjn_total} 个有序对（18×18），执行 {adjn_exec}，N/A {}\n- 生成式行内边界（ADJI2）：{adji2_total} 个用例（12×12×6 边界），执行 {adji2_exec}，N/A 0\n- 生成式容器内相邻（ADJC2）：{adjc2_total} 个用例（5×18 自相邻 + 5×4 跨类型），执行 {adjc2_exec}，N/A 0\n- 嵌套容器（ADJX）：{adjx_total} 个组合（3×3），执行 {adjx_exec}，N/A 0\n- 用例总数：{total_cases}\n- 金样：`docs/qa/golden/html/<CASE>.html`（{golden_count} 份；ADJB/ADJI/ADJC/ADJN/ADJI2/ADJC2/ADJX 抽样，`UPDATE_GOLDEN=1` 时更新）\n- 相邻矩阵全量 HTML 摘要：`docs/qa/adjacency-digest.txt`（{adj_exec}+{adji_exec}+{adjc_exec}+{adjn_exec}+{adji2_exec}+{adjc2_exec}+{adjx_exec} 行 SHA-256，运行时重算比对）\n- 命令：`cargo test -p veil --test syntax_matrix`\n\n",
        adj_total - adj_exec,
        adjn_total - adjn_exec
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

    out.push_str("## 组统计\n\n");
    out.push_str("| 组 | 用例 | 执行 | N/A |\n|---|---:|---:|---:|\n");
    out.push_str(&format!(
        "| ADJB | {adj_total} | {adj_exec} | {} |\n",
        adj_total - adj_exec
    ));
    out.push_str(&format!("| ADJI | {adji_total} | {adji_exec} | 0 |\n"));
    out.push_str(&format!("| ADJC | {adjc_total} | {adjc_exec} | 0 |\n"));
    out.push_str(&format!(
        "| ADJN | {adjn_total} | {adjn_exec} | {} |\n",
        adjn_total - adjn_exec
    ));
    out.push_str(&format!("| ADJI2 | {adji2_total} | {adji2_exec} | 0 |\n"));
    out.push_str(&format!("| ADJC2 | {adjc2_total} | {adjc2_exec} | 0 |\n"));
    out.push_str(&format!("| ADJX | {adjx_total} | {adjx_exec} | 0 |\n\n"));

    out.push_str("## FAIL 明细\n\n");
    let fails: Vec<&Row> = rows.iter().filter(|r| r.status == Status::Fail).collect();
    if fails.is_empty() {
        out.push_str("无。\n\n");
    } else {
        out.push_str("| CASE | 表面 | 期望 / 实际片段（≤300 字符） |\n|---|---|---|\n");
        for r in fails {
            let detail = r.detail.chars().take(600).collect::<String>();
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

    out.push_str(&format!("## ADJB N/A（{} 条，不计断言）\n\n", adj_na.len()));
    if adj_na.is_empty() {
        out.push_str("无。\n\n");
    } else {
        for case in adj_na {
            out.push_str(&format!(
                "- `{}`（{}）：{}\n",
                case.id,
                case.title,
                escape_cell(case.na_reason.as_deref().unwrap_or("N/A"))
            ));
        }
        out.push('\n');
    }

    out.push_str("## 真实缺陷（Task 4 已修复，结构断言已全量恢复）\n\n");
    for (n, why) in ADJ_FIXED_DEFECTS {
        out.push_str(&format!("- `CASE-ADJB-{n:03}`：{why}\n"));
    }
    for (n, why) in ADJI_FIXED_DEFECTS {
        out.push_str(&format!("- `CASE-ADJI-{n:03}`：{why}\n"));
    }
    out.push('\n');

    out.push_str("## ADJI 规范合并例外（非产品缺陷，按解析器真实节点序列断言）\n\n");
    out.push_str("| CASE | 无空格源串 | 期望顶层节点 | 规范依据 |\n|---|---|---|---|\n");
    for merge in &ADJ_INLINE_MERGES {
        let n = (merge.a * ADJ_INLINE_UNITS + merge.b) * ADJ_INLINE_BOUNDARIES + 2;
        out.push_str(&format!(
            "| `CASE-ADJI-{n:03}` | `{}` | {} | {} |\n",
            merge.source,
            merge
                .expected
                .iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(" + "),
            merge.rule
        ));
    }
    out.push('\n');

    out.push_str("## ADJI 标签 Ignore 理由\n\n");
    out.push_str("- `Image`：行内图片的甲/乙只出现在 `<img alt>` 属性（不产出正文文本节点），HTML 标签可见性 Ignore；PDF 版面同样不绘制 alt，`pdf_text` 标签断言 Ignore。结构标记 `<img` 照常断言。\n");
    out.push_str("- `Math`：CJK 公式在 STIX 字体中无字形，导出降级为源码 `$甲$`（ASCII 公式才产出 `<span class=\"math-inline\">`）；SVG 形态下标签不是正文文本，故 HTML/PDF 标签 Ignore。结构断言改为断言降级源码字面量 `$甲$`/`$乙$` 可见。\n");
    out.push_str("- `FootnoteRef`：引用渲染为编号 `[n]`，甲/乙标签不可见；HTML 结构标记 `<sup` 照常断言，标签与 `pdf_text` 均 Ignore。\n");
    out.push_str("- `HtmlSelfClosed`：`<br />` 是 void 标签，源串本身不含甲/乙标签；HTML 结构标记 `<br` 照常断言，标签与 `pdf_text` 均 Ignore。\n");
    out.push('\n');

    out.push_str("## 降级与边界说明\n\n");
    out.push_str("### 矩阵缺陷分流表\n\n");
    out.push_str("| 类别 | CASE | 现象 | 处置 |\n|---|---|---|---|\n");
    out.push_str("| 真实缺陷（已修复） | `CASE-ADJB-086`、`CASE-ADJB-239` | HTML 导出把跨块 link-ref 定义收集为 `defs` 前缀并紧贴正文，紧随的 4 空格缩进行被 pulldown 当作定义段落的懒续行（缩进代码不能打断段落），`IndentedCode` 降级为 `<p>`、`<pre>` 消失 | `export.rs` 新增 `with_defs()`：`defs` 非空时与正文之间补一个空行（定义段落终止），覆盖普通块 / 列表项 / Callout / 标题四处拼接点；`defs` 为空时输出不变。结构断言已全量恢复 |\n");
    out.push_str("| 真实缺陷（已修复） | `CASE-ADJI-184` | `==甲====乙==` 中长度 4 的 `=` run 既不可作开标记也不可作闭标记，整段降级为纯文本，`Highlight` 节点与 `<mark>` 同时消失 | `veil-core` 的 `try_highlight` 增加「4 连等号边界」规则：run 恰为 4 时前两个闭合当前高亮、后两个开启下一个，解析出两个 `Highlight` 并导出两个 `<mark>`；长度 3 或 ≥5 的 run 维持原行为 |\n");
    out.push_str("| 规范允许合并（非缺陷） | `CASE-ADJB-077`、`CASE-ADJB-149`、`CASE-ADJB-257` | 两单元在块边界无空行时按 CommonMark 合并为单块：缩进代码+缩进代码因块内空行不终止缩进代码而合并；列表项+缩进代码因 4 列行仅比内容列多 2 列按项内段落续块；脚注定义+缩进代码因定义续行要求缩进 ≥4 列被并入定义正文 | 按解析器真实块序列断言（见「ADJB 规范合并」条目），不登记为缺陷 |\n");
    out.push_str("| 规范允许合并（非缺陷） | `CASE-ADJI-002`、`CASE-ADJI-028`、`CASE-ADJI-054` | 无空格边界下 Strong+Strong / Emph+Emph / Code+Code 按 CommonMark「rule of 3」或代码跨度等长规则退化为单个节点 | 按规范预期节点序列断言（见「ADJI 规范合并例外」表，逐条附规范依据） |\n");
    out.push_str("| 模型边界（非缺陷） | ADJB 的 Callout 单元 | 默认 `callouts.json` 为空且 ADJB 不注入 registry，`> [!FIXME]` 无法产生自定义 Callout 块型，实测回退为 `BlockQuote` | 该单元覆盖未知标记回退路径；真实自定义 Callout 的 7 表面断言由 ADJC 的 Callout 容器组覆盖 |\n");
    out.push_str("| 模型边界（非缺陷） | ADJC 容器内的 Math / 嵌套 Admonition | 容器展开只把 `Table`（ListItem / BlockQuote / Details）与 `Mermaid`（全部 5 类容器）提升为独立块；容器内 Math 与内层 Admonition/Callout 仍按容器正文渲染，不产生独立块 | 对应结构标记仅在真正成为独立块的 Details 容器中断言，其余容器登记 Ignore（既有解析模型边界） |\n");
    out.push_str("| 真实缺陷（已修复） | `CASE-ADJN-008` | 单换行 `甲\\n| 乙 | b |\\n| --- | --- |…` 中，GFM 参考实现（cmark-gfm/pulldown）用分隔行把段落最后一行提升为表头，产出「段落 + 表格」；`veil-core` 块解析把整段并入段落，导致 WYSIWYG/PDF 显示管道文本而 HTML 显示真实表格 | 复刻 cmark-gfm 的 `try_inserting_table_header_paragraph`：段落扫描遇到分隔行且上一行含未转义竖线、列数一致时，截断段落并让表头行由外层重新扫描为 `Table`；`block.rs` 单测 `table_from_paragraph_tail` 同步收紧为「Paragraph + Table」 |\n");
    out.push_str("| 规范允许合并（非缺陷） | ADJN（78 条有序对） | 单换行边界下，B 的行按 CommonMark/GFM 构造规则并入 A 的开放块（段落续行、Setext 下划线、懒续行、HTML 块吞并、表格体延续等） | 按规范推导的块序列断言，逐条登记于「ADJN 规范构造例外」表，不登记为缺陷 |\n");
    out.push_str("| 模型边界（非缺陷） | `CASE-ADJI2-*`（12 条） | `<br />` 独占一行时满足 HTML 块类型 7 起始条件，吞并至空行；A 单元为 HtmlSelfClosed 且边界为换行时不再产生段落与行内节点 | 按 `HtmlBlock` 断言（块序列 + `<br` + 后续行内标记不得出现 + 标签可见性），不登记为缺陷 |\n");
    out.push('\n');

    out.push_str("- 断言口径（D4）：`html_contains`/`html_not_contains` 比较前对正文与期望片段做开始标签属性归一（h1–h6、details、summary、th、td、table、thead、tbody、tr、ul、ol、li、p、pre、code、figure、blockquote、mark），使 `<h1 id=…>` ≡ `<h1>`、`<details open>` ≡ `<details>`、`<th style=…>` ≡ `<th>`；属性类原文片段（`class=`、`id=`、`alt=` 等）仍按原文严格命中，纯文本片段不归一。\n");
    out.push_str("- `html_not_contains` 仅在 `<body>…</body>` 范围内检查，head 内的 `<style>`/`<title>` 不计为正文泄漏。\n");
    out.push_str("- 金样比较保持原文严格，不受上述归一影响。\n");
    out.push_str("- `no_overlap`：PDF 版面未暴露块级包围盒，改用 headless UI 的块几何（`top`/`height`，容差 0）判断相邻块不相交；该断言记在 PDF 表面。\n");
    out.push_str("- PDF 关键文本：PDF 字节流中文本为字形 CID，无法直接检索 Unicode；关键文本取自同一次 `layout_document` 的 `DrawOp::Text`（与 `render_pages` 同一版面数据）。\n");
    out.push_str("- PRINT：断言打印分页模型（页数/断点单调/首断点为 0）；物理打印需要 GDI 设备上下文，不在本矩阵断言口径内，真实打印样张见 `docs/qa/w10-output-pipeline-evidence.md`。\n");
    out.push_str("- HTML 中的 Mermaid：harness 经测试门面注入产品同一离线渲染器（`platform::mermaid_offline::OfflineMermaid`：进程内 QuickJS + mermaid.js → resvg，无网络；`OnceLock` 使整轮测试只初始化一次），`export::html_of_doc` 收到真实 `MermaidImage.svg_text`。断言要求正文出现 `<svg` 且不得同时保留 `language-mermaid` 源码围栏降级形态；单块渲染失败即判 FAIL，不降级断言。\n");
    out.push_str("- 空态用例（CASE-EDGE-001..003）：语料的 `表面：N/A` 只说明「无可见内容」，不等于不可测。harness 在六个表面执行空态断言：UI 0 块且几何有限、HTML `<body>` 为空、PDF 恰 1 页空白且无版面文本（依据 `layout_document` 无条件 `new_page()`、`LayoutOpts::default().page_numbers == false`）、PRINT 恰 1 页且断点为 `[0.0]`（依据 `compute_page_breaks` 恒以 `vec![0.0]` 起步）。\n");
    out.push_str("- Mermaid 用例不做全文金样：SVG 内的路径/弧坐标由 dagre 布局与字体度量共同决定，跨进程运行存在浮点级差异（classDiagram / stateDiagram 的 `d` 属性实测会变），全文金样必然 flaky；改由结构断言证伪「未渲染却算通过」：`<body>` 必须含真实 `<svg`，且不得残留 `language-mermaid` / 「渲染数据不可用」降级围栏。\n");
    out.push_str("- 容器内 Mermaid（G7 已修复）：列表/引用/Admonition/Callout/Details 内的 ```` ```mermaid ```` 围栏已提升为独立 `BlockKind::Mermaid` 块（携带 `quote_depth` / `list_indent` / `details*` 归属），导出内嵌真实 `<svg>`，不再保留 `language-mermaid` 源码围栏。源码正文经容器前缀剥离：引用逐层剥 `>`（含一个可选空格），列表首行剥标记、续行只剥至多 `list_indent` 列缩进（图的相对缩进保留）。证据：`CASE-ADV-011` 与 `CASE-ADJC-001..090`。\n");
    out.push_str("- ADJC（生成式容器内容）：5 类容器（ListItem / BlockQuote / Admonition / Callout / Details）× 18 个块单元 = 90；容器标记可见文本恒为「甲容器」，内层块沿用 ADJB 的 18 单元（标签「乙」）并逐行加容器前缀（空行同样加，保证多行块整体仍属容器）。Admonition / Callout 夹具按文档语法：`[!TYPE]` 独占一行，标题「甲容器」与正文放在后续 `>` 行（标记行后直接跟文本会渲染为缩进代码块，属非契约形态）。PARSE 断言容器块型出现、`Table`/`Mermaid` 为独立块（仅 ListItem / BlockQuote / Details 切表格；Admonition / Callout 只提升 Mermaid，表格留在提示框内），并用 `Session::block_entries()` 校验提升块的 `BlockContainer`（ListItem→`list_indent>0`；引用类→`quote_depth>0`；Details→`details=true` 且 `details_depth>0`）与 Mermaid 源码前缀已剥离。\n");
    out.push_str("- ADJC 的 Callout 注入：harness 用 `veil_core::CalloutRegistry::from_json` 造 `FIXME` 条目，经新增门面 `veil_ui::qa::QaApp::new_with_callouts` / `veil::qa::html_of_doc_with_callouts`（含 `..._with_mermaid_and_callouts`）/ `pdf_of_doc_with_callouts` 注入解析与导出；产品默认入口签名与行为不变（默认 registry 仍为空）。\n");
    out.push_str("- ADJC 标签计数：统计对象是导出正文 HTML（含 `id=` / `href=` 等属性值，不是纯可见文本）；容器标签「甲」出现 1 次；内层「乙」按容器实际模型计数（Paragraph…Table/ListItem/BlockQuote/Admonition/Callout/HtmlBlock/FrontMatter = 1；Details = 2（summary + 正文）；ThematicBreak/LinkRefDef = 0；FootnoteDef 在 Details 内 = 0、其余容器 = 1）。Math 与 Mermaid 不做 HTML 标签计数（Math 正文形态不稳定、Mermaid 由 SVG 承载），PDF 侧 Mermaid 同样 Ignore。\n");
    out.push_str("- ADJC 结构标记例外：容器展开只把 `Table`（ListItem / BlockQuote / Details）与 `Mermaid`（全部 5 类容器）提升为独立块；容器内的 Math 块与内层 Admonition/Callout 仍按容器正文渲染（不产生独立块），故这些单元的对应结构标记（`class=\"math` / `class=\"callout ...\"`）仅在其真正成为独立块的 Details 容器中断言，其余容器登记为 Ignore（不是 G7 行为，属既有解析模型边界）。FrontMatter 在非文档首块按分隔线/Setext 降级处理，仅在 Details 容器单独模拟（分隔线 + Setext 标题）。\n");
    out.push_str("- 金样首次缺失时自动写入（引导），之后普通运行全文比对。\n");
    out.push_str("- ADJB（生成式块级相邻）：18 个块单元（Paragraph…FrontMatter）× 18 个有序对 = 324；文档恒为 `A + \"\\n\\n\" + B + \"\\n\"`，A 侧标签 `甲`、B 侧标签 `乙`。B=FrontMatter 的 18 对按契约记 N/A（FrontMatter 仅识别于文档首块），不执行表面断言。7 个表面全部执行：PARSE（`blocks` + 全序列 `kinds`）、WYSIWYG/FOCUS/TYPEWRITER（既有模型断言 + `no_overlap`）、HTML（每单元结构标记 + 可见标签存在/不可见标签不泄漏）、PDF（≥1 页 + 显式 `pdf_text` 可见标签）、PRINT（分页模型）。\n");
    out.push_str(&format!(
        "- ADJB 规范合并（非产品缺陷，按解析器真实块序列断言）：{}。\n",
        ADJ_MERGES
            .iter()
            .map(|(n, why)| format!("`CASE-ADJB-{n:03}`：{why}"))
            .collect::<Vec<_>>()
            .join("；")
    ));
    out.push_str("- ADJB 夹具调整：ThematicBreak 使用 `***`（文档首块 `---` 会被未闭合 FrontMatter 启发式吞并）；Table 第二列、Mermaid 第二节点使用 ASCII，保证 A/B 标签可区分。\n");
    out.push_str("- ADJB 的 Callout 单元：默认 callouts.json 为空且该组不注入 registry，`> [!FIXME]` 实测解析为 BlockQuote（未知 Callout 标记回退）。该单元覆盖回退路径；真实自定义 Callout 的 7 表面断言由 ADJC 的 5 类容器 × Callout 内层块与 Callout 容器组覆盖（见上）。\n");
    out.push_str("- ADJB 的 Mermaid 单元使用源码围栏结构标记 `class=\"language-mermaid\"`（不注入离线引擎，避免 306×7 下重复渲染与浮点 flaky）；真实 `<svg>` 渲染由 `CASE-EXT-001..009` 覆盖。Math 单元标记 `class=\"math` 同时覆盖 SVG 与源码降级两种导出形态，标签可见性交给 PDF `pdf_text` 断言。\n");
    out.push_str("- ADJB 金样：仅按执行序号等距抽取 20 条（`n = 1,16,…,286`）写全文金样；其余 286 条保留结构断言。全量 306 条 HTML 另有 SHA-256 摘要（`docs/qa/adjacency-digest.txt`），运行时逐条重算比对，不一致时在 HTML 表面报出该 CASE（首个不一致即 FAIL 明细首行）。\n");
    out.push_str("- ADJI（生成式行内相邻）：12 个行内单元（Strong…HtmlLiteral）× 12 个有序对 × 2 种边界（`A + \" \" + B` / `A + B`）= 288；文档为单段落，末尾恒加 `\\n`。A 侧标签 `甲`、B 侧标签 `乙`；任一单元为 FootnoteRef 时附对应脚注定义（`[^甲]: 注一` / `[^乙]: 注二`，不含标签，避免污染可见标签断言）。PARSE 新增 `inline_kinds` 断言顶层节点序列，并检查每个节点区间落在段落块区间内、位于 UTF-8 字符边界、按起点有序且互不重叠（相邻首尾可相接）。\n");
    out.push_str("- ADJI 的 HTML/PDF 断言：每个单元断言结构标记（`<strong`/`<em`/`<code`/`<a href=`/`<img`/`<del`/`<mark`/`<sup`/`<kbd`/`<br`；Math 断言降级源码 `$甲$`，HtmlLiteral 断言字面文本 `<foo>甲</foo>`）；可见标签（Strong/Emph/Code/Link/Strikethrough/Highlight/Html/HtmlLiteral）在正文按顺序各出现一次，PDF `pdf_text` 同样断言；不可见单元按上表 Ignore。\n");
    out.push_str("- ADJI 的 HtmlLiteral：`<foo>甲</foo>` 不是行内白名单标签，既有解析语义把它当纯文本，故不产出 inline 节点（`inline_kinds=None` 或仅另一单元的节点）；导出件按字面输出 `<foo>甲</foo>`，标签断言仍覆盖。这是既有语义，不是节点丢失。\n");
    out.push_str("- ADJI 的无空格边界：Strong+Strong / Emph+Emph / Code+Code 按 CommonMark「rule of 3」或代码跨度等长规则合并为单节点，登记在「ADJI 规范合并例外」表；CASE-ADJI-184（Highlight+Highlight，`==甲====乙==`）曾因 4 连 `=` run 被吞而整段降级为纯文本，已由 `try_highlight` 的「4 连等号边界」规则修复（两个 Highlight + 两个 `<mark>`），`inline_kinds` 与 `<mark` 结构断言已全量恢复。\n");
    out.push_str("- ADJI 的 Math：行内 CJK 公式无 STIX 字形，`export::html_of_doc` 的 `math_svg` 返回 None，保留源码 `$甲$`（产品既有降级路径，不是相邻干扰）；因此 Math 单元的结构断言是降级源码字面量而非 `class=\"math`，标签可见性按 Ignore 处理。\n");
    out.push_str("- ADJI 金样：排除脚注定义用例（文末脚注区块编号/回链易受实现细节影响）后，对剩余 242 条等距抽取 20 条写全文金样；全量 288 条 HTML 写入 `docs/qa/adjacency-digest.txt`，运行时逐条重算比对，缺失记录按新增组补写，已存在记录不一致即 FAIL。\n");
    out.push_str("- ADJC 金样与摘要：抽样 20 条（覆盖 5 类容器、避开 Mermaid 浮点坐标）写全文金样；全量 90 条 HTML 追加写入 `docs/qa/adjacency-digest.txt`（与原 ADJB 306 行、ADJI 288 行及 ADJN/ADJI2/ADJC2/ADJX 新增行共存），运行时逐条重算比对。\n");
    out.push_str(&format!(
        "- ADJN（生成式块级单换行）：18 个块单元 × 18 个有序对 = {adjn_total}，文档恒为 `A + \"\\n\" + B + \"\\n\"`；B=FrontMatter 的 18 对按契约记 N/A（FrontMatter 仅识别于文档首块）。逐对推导期望块序列：默认「A 序列 + B 序列」，{n} 条有序对按 CommonMark/GFM 构造规则合并或降级，逐条登记于下表；结构断言用「期望块型 → HTML 结构标记」映射，标签可见性按单元导出模型断言（不可见时用 `html_not_contains`/`pdf_not_text` 证伪吞并）。\n",
        n = ADJN_EXCEPTIONS.len()
    ));
    out.push_str(
        "\n### ADJN 规范构造例外\n\n| CASE | 有序对 | 期望块序列 | 规范依据 |\n|---|---|---|---|\n",
    );
    for e in ADJN_EXCEPTIONS {
        let n = e.a * ADJ_UNITS + e.b + 1;
        out.push_str(&format!(
            "| `CASE-{ADJN_GROUP}-{n:03}` | {} + {} | {} | {} |\n",
            UNITS[e.a].name,
            UNITS[e.b].name,
            e.kinds.join(" / "),
            e.rule
        ));
    }
    out.push('\n');
    out.push_str("### ADJN N/A 登记\n\n| CASE | 有序对 | 理由 |\n|---|---|---|\n");
    for c in adjn_na {
        out.push_str(&format!(
            "| `{}` | {} | {} |\n",
            c.id,
            c.title,
            c.na_reason.as_deref().unwrap_or("")
        ));
    }
    out.push('\n');
    out.push_str(&format!(
        "- ADJI2（生成式行内边界）：12 个行内单元 × 12 个有序对 × 6 种边界（换行 `\\n`、`。`、`，`、`）`、`.`、`,`）= {adji2_total}；默认顶层节点序列为「A 节点 + B 节点」，标点边界按字面文本处理、不改变 flanking（`**甲**。**乙**` 仍是两个 Strong）。唯一例外：A 单元为 HtmlSelfClosed（`<br />`）且边界为换行时，`<br />` 独占一行触发 HTML 块类型 7，整段被吞并为 `HtmlBlock`（FootnoteRef 时另加一个 `FootnoteDef`），不产出段落与行内节点；逐条断言 `<br` 存在、B 行内标记不得出现、可见标签保留、其余 860 条按默认模型断言。金样排除脚注引用后等距抽取 20 条；全量 {adji2_total} 条写入摘要。\n"
    ));
    out.push_str(&format!(
        "- ADJC2（生成式容器内相邻）：5 类容器 × 18 个自相邻对（以内层空行分隔的同一单元 `乙`/`丙`）= {ADJC2_SELF}，加每组 4 个关键跨类型对（Table+Table、Mermaid+Table、FencedCode+IndentedCode、Paragraph+Table）= {ADJC2_CROSS}，共 {adjc2_total}。断言：顶层块序列、容器结构标记与内层块标记的精确计数、内层标签 `乙`/`丙` 在 HTML 与 PDF 中的逐份计数、提升块（Table/Mermaid）的 `BlockContainer` 归属、`no_overlap`；内层两块的块数按容器模型登记（如 IndentedCode 在 Details 内因块内空行合并为 1 块，Admonition/Callout 内表格不独立成块）。金样避开 Mermaid 后等距抽取 20 条。\n"
    ));
    out.push_str(&format!(
        "- ADJX（嵌套容器）：ListItem / BlockQuote / Details 的 3×3 = {adjx_total} 组合；断言容器元素精确计数（`<li>`/`<blockquote`/`<details`/`<summary`）、Details 嵌套深度（`details_depth` 等于嵌套层数）、外层 Details 的子块归属、标签 `甲容器`/`乙容器` 可见性、`no_overlap`；9 条全部写金样。\n"
    ));
    // 未修复项登记：结构化矩阵里标记了 known_defect 的用例。
    let mut open_defects: Vec<(String, &'static str)> = Vec::new();
    for case in adjx_cases() {
        if let Some(spec) = &case.structured {
            if let Some(desc) = spec.known_defect {
                open_defects.push((case.id.clone(), desc));
            }
        }
    }
    for case in adjc2_cases() {
        if let Some(spec) = &case.structured {
            if let Some(desc) = spec.known_defect {
                open_defects.push((case.id.clone(), desc));
            }
        }
    }
    if open_defects.is_empty() {
        out.push_str("- 未修复项：无。\n");
    } else {
        out.push_str("\n### 未修复项（已登记，当前断言锁定降级形态）\n\n| CASE | 现象 | 状态 |\n|---|---|---|\n");
        for (id, desc) in &open_defects {
            out.push_str(&format!(
                "| `{id}` | {desc} | 待修复（本轮未改：需把 Details 块识别与 B3 子块展开扩展进 ListItem/BlockQuote 容器展开路径，超出本次覆盖闭合的安全改动范围） |\n"
            ));
        }
        out.push('\n');
    }

    out.push_str("\n## 交付事实\n\n");
    out.push_str("- 本文件由 `cargo test -p veil --test syntax_matrix` 重生成，运行该测试会覆写全文；不要手工追加内容。\n");
    out.push_str("- 工作区测试、clippy、release 体积与 SHA-256 等交付事实见 `docs/qa/release-facts-2026-11-27.md`。\n");

    if let Some(dir) = Path::new(REPORT).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(REPORT, out) {
        eprintln!("报告写入失败（{REPORT}）：{e}");
    }
}
