//! CommonMark 0.31.2 一致性门禁（652 例官方规范语料，固化自 P1 评审 harness）。
//!
//! 四套判定（口径详见 docs/review/P1-commonmark-conformance.md §2）：
//! - Suite A：Veil 自有块解析器的顶层「Markdown 生成块」骨架 vs 期望 HTML 的同类骨架
//!   （原始 HTML 块两侧都不计：源码原样输出，输出侧无法可靠区分；渲染由 Suite B 覆盖）。
//! - Suite B：产品导出路径复刻（块分段 + pulldown-cmark）的 HTML vs 规范 HTML。
//! - Suite C：Veil 自有行内节点计数 vs pulldown-cmark 源码行内事件计数
//!   （弱判定；原始 HTML 不产生语义节点，图片子树按 alt 纯文本模型视为叶子）。
//! - Suite D：行内前序森林签名 (kind, depth) 逐例比对，覆盖 C 的「计数抵消」盲区
//!   （如并列 vs 嵌套 strong 计数相同）。另按 26 节设 C 分段基线：改动只允许
//!   目标节改善、任何节不得退化。
//!
//! 门禁方向：四套指标只允许改善、不允许退化（`>=` 基线）。修复提升通过率后，
//! 应同步上调基线常量。

use std::collections::BTreeMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};
use veil_core::{BlockKind, InlineKind, InlineNode, Range, Session};

/// 一致性基线（0d harness 校正后，2026-10-28 实测；只允许改善）。
/// 本轮变更：
/// - A 口径改为「Markdown 生成块骨架」，原始 HTML 块两侧都不计（原先按标签深度
///   扫描期望 HTML，未闭合/嵌套原始标签会把后续生成块误记为嵌套而漏数）。
/// - C/D 参照改为 pulldown-cmark 源码行内事件（原先从期望 HTML 数标签，会把源码
///   原样输出的 `<a>`/`<img>`/`<del>` 误计为语义节点），图片子树按 Veil 的
///   「alt 为纯文本」模型视为叶子。
/// - 产品侧同步修正：类型 6 块级标签表补齐 table/td/tbody 等 18 个规范标签
///   （Example 148/160 此前把 `<table>` 行解析成段落）。
const BASELINE_A: usize = 652;
const BASELINE_B_EXACT: usize = 629;
const BASELINE_B_CANON: usize = 630;
const BASELINE_C: usize = 652;
/// 条件基线（仅 A 通过时统计）。
const BASELINE_B_EXACT_WHEN_A: usize = 629;
const BASELINE_C_WHEN_A: usize = 652;
/// Suite D 基线（行内结构签名；与 C 同源参照）。
const BASELINE_D: usize = 652;
const BASELINE_D_WHEN_A: usize = 652;
/// 分节 C 基线（0d 参照校正后实测，等于各节总例数）：任何一节只允许改善。
/// 与 SECTIONS 打印同序（BTreeMap 字典序）。
const BASELINE_C_SECTION: &[(&str, usize)] = &[
    ("ATX headings", 18),
    ("Autolinks", 19),
    ("Backslash escapes", 13),
    ("Blank lines", 1),
    ("Block quotes", 25),
    ("Code spans", 22),
    ("Emphasis and strong emphasis", 132),
    ("Entity and numeric character references", 17),
    ("Fenced code blocks", 29),
    ("HTML blocks", 44),
    ("Hard line breaks", 15),
    ("Images", 22),
    ("Indented code blocks", 12),
    ("Inlines", 1),
    ("Link reference definitions", 27),
    ("Links", 90),
    ("List items", 48),
    ("Lists", 26),
    ("Paragraphs", 8),
    ("Precedence", 1),
    ("Raw HTML", 20),
    ("Setext headings", 27),
    ("Soft line breaks", 2),
    ("Tabs", 11),
    ("Textual content", 3),
    ("Thematic breaks", 19),
];

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/conformance/examples");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum B {
    P,
    H(u8),
    Pre,
    Quote,
    List,
    Table,
    Rule,
    Other,
}

/// 期望 HTML 的顶层「Markdown 生成块」序列。
///
/// 口径：只看 Markdown 生成块，原始 HTML 块不计——源码原样输出，从输出侧无法与
/// 生成标签可靠区分（`<div>` 未闭合、`<pre>` 无 `<code>` 等）；其渲染正确性由
/// Suite B 导出比对覆盖。扫描按行，生成块总是在行首开始标签：
/// `<p>` / `<hN>` / `<pre><code`（原始 `<pre>` 不算）/ `<blockquote>` /
/// `<ul>` / `<ol>` / `<hr />`。引用与列表整体折叠为一个顶层块，内部块不进入序列
/// （与实际块序列口径一致）。
fn expected_blocks(html: &str) -> Vec<B> {
    let lines: Vec<&str> = html.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if l.trim_end() == "<hr />" {
            out.push(B::Rule);
            i += 1;
            continue;
        }
        let (b, close): (B, String) = if l.starts_with("<pre><code") {
            (B::Pre, "</pre>".to_string())
        } else if l.starts_with("<p>") {
            (B::P, "</p>".to_string())
        } else if l.starts_with("<blockquote>") {
            (B::Quote, "</blockquote>".to_string())
        } else if l.starts_with("<ul>") || l.starts_with("<ul ") {
            (B::List, "</ul>".to_string())
        } else if l.starts_with("<ol>") || l.starts_with("<ol ") {
            (B::List, "</ol>".to_string())
        } else if let Some(level) = heading_level(l) {
            (B::H(level), format!("</h{level}>"))
        } else {
            i += 1;
            continue;
        };
        out.push(b);
        i = if matches!(b, B::Quote | B::List) {
            skip_container(&lines, i, &close)
        } else {
            skip_until(&lines, i, &close)
        };
    }
    out
}

/// 行首生成标题 `<h1>`–`<h6>`（排除 `<hr`、`<html`）。
fn heading_level(l: &str) -> Option<u8> {
    let rest = l.strip_prefix("<h")?.as_bytes();
    match (rest.first(), rest.get(1)) {
        (Some(d), Some(b'>')) if (b'1'..=b'6').contains(d) => Some(d - b'0'),
        _ => None,
    }
}

/// 单行容器（p/pre/hN）：跳至首个含闭合标签的行。
fn skip_until(lines: &[&str], mut i: usize, close: &str) -> usize {
    while i < lines.len() {
        let hit = lines[i].contains(close);
        i += 1;
        if hit {
            break;
        }
    }
    i
}

/// 引用/列表容器：按开关标签计深度，跳过整个容器（嵌套块不进入顶层序列）。
fn skip_container(lines: &[&str], mut i: usize, close: &str) -> usize {
    let mut depth = 0i32;
    while i < lines.len() {
        let l = lines[i];
        if close == "</blockquote>" {
            depth += l.matches("<blockquote").count() as i32;
            depth -= l.matches("</blockquote").count() as i32;
        } else {
            depth += l.matches("<ul").count() as i32 + l.matches("<ol").count() as i32;
            depth -= l.matches("</ul>").count() as i32 + l.matches("</ol>").count() as i32;
        }
        i += 1;
        if depth <= 0 {
            break;
        }
    }
    i
}

/// Veil 自有块解析器的顶层块序列；同签名的连续列表项折叠为一个 List（与规范
/// `<ul>`/`<ol>` 对齐；0c P0-3：标记变化如 `-`→`+` / `.`→`)` 起新列表，见
/// Example 301/302）。续块（首行无合法标记，签名为 None）静默随前项归并。
/// LinkRefDef 无 HTML 输出，跳过（规范期望序列中定义行不对应任何顶层块）。
fn actual_blocks(s: &Session) -> Vec<B> {
    let mut out = Vec::new();
    let mut cur_sig: Option<(bool, u8)> = None;
    for (_, kind, range) in s.blocks() {
        if let BlockKind::ListItem { .. } = kind {
            let text = s.text_slice(range).unwrap_or_default();
            let first = text.split_inclusive('\n').next().unwrap_or(&text);
            // 首行有合法标记：签名同前项则续列归并，否则开新 List；
            // 续块（None）随前项归并（保持当前签名上下文，不产生新块）
            if let Some(sig) = veil_core::list_signature(first) {
                if cur_sig != Some(sig) {
                    cur_sig = Some(sig);
                    out.push(B::List);
                }
            }
            continue;
        }
        cur_sig = None;
        let b = match kind {
            BlockKind::Paragraph => B::P,
            BlockKind::AtxHeading(n) | BlockKind::SetextHeading(n) => B::H(n),
            BlockKind::FencedCode(_) | BlockKind::IndentedCode => B::Pre,
            BlockKind::Math => B::Other,
            BlockKind::Table => B::Table,
            BlockKind::BlockQuote => B::Quote,
            BlockKind::ThematicBreak => B::Rule,
            BlockKind::LinkRefDef => continue,
            // 原始 HTML 块与期望侧口径一致：整体不计入顶层生成块序列（Suite B 覆盖渲染）。
            BlockKind::HtmlBlock => continue,
            BlockKind::ListItem { .. } => unreachable!("列表项已在上方处理"),
        };
        out.push(b);
    }
    out
}

/// 产品导出路径的分段渲染：连续列表项合并为一段源码，其余块逐块渲染。
/// 引用式链接跨块可见性：非 Math 块源码前拼接全部定义源码（与产品 export.rs 一致）。
fn export_body(s: &Session) -> String {
    let mut body = String::new();
    let blocks = s.blocks();
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut defs = String::new();
    for (_, kind, range) in &blocks {
        if *kind == BlockKind::LinkRefDef {
            if let Some(t) = s.text_slice(*range) {
                defs.push_str(&t);
                if !t.ends_with('\n') {
                    defs.push('\n');
                }
            }
        }
    }
    let mut i = 0;
    while i < blocks.len() {
        let (_, kind, range) = blocks[i].clone();
        if kind == BlockKind::Math || kind == BlockKind::LinkRefDef {
            i += 1;
            continue;
        }
        if matches!(kind, BlockKind::ListItem { .. }) {
            let start = range.start;
            let mut end = range.end;
            let mut j = i + 1;
            while let Some((_, k, r)) = blocks.get(j) {
                if matches!(k, BlockKind::ListItem { .. }) {
                    end = r.end;
                    j += 1;
                } else {
                    break;
                }
            }
            if let Some(text) = s.text_slice(Range::new(start, end)) {
                html::push_html(&mut body, Parser::new_ext(&format!("{defs}{text}"), opts));
            }
            i = j;
            continue;
        }
        if let Some(text) = s.text_slice(range) {
            html::push_html(&mut body, Parser::new_ext(&format!("{defs}{text}"), opts));
        }
        i += 1;
    }
    body
}

/// 语义近似：标签保留；`<pre>` 外文本折叠空白，标签间空白文本丢弃。
fn canonical(html: &str) -> String {
    let mut out = String::new();
    let mut pre = 0i32;
    let mut i = 0usize;
    while i < html.len() {
        let Some(off) = html[i..].find('<') else {
            push_text(&mut out, &html[i..], pre > 0);
            break;
        };
        if off > 0 {
            push_text(&mut out, &html[i..i + off], pre > 0);
            i += off;
        }
        let Some(end) = html[i..].find('>') else {
            push_text(&mut out, &html[i..], pre > 0);
            break;
        };
        let tag = &html[i..=i + end];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<pre") {
            pre += 1;
        } else if lower.starts_with("</pre") {
            pre -= 1;
        }
        out.push_str(tag);
        i += end + 1;
    }
    out
}

fn push_text(out: &mut String, text: &str, keep: bool) {
    if keep {
        out.push_str(text);
    } else {
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push_str(&collapsed);
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
struct Counts {
    strong: usize,
    emph: usize,
    code: usize,
    link: usize,
    image: usize,
    del: usize,
}

/// 行内参照：pulldown-cmark（CommonMark 参考实现）在源码上的行内事件。
/// 原始 HTML 以 `Event::Html` 透传，不产生 Link/Image/Del 事件，因此天然避开
/// 「期望 HTML 中源码原样输出的 `<a>`/`<img>`/`<del>` 被误计」的问题。
/// 返回 (计数, 前序签名)；Math 不计（与 Veil 口径一致）。
fn reference_inline(md: &str) -> (Counts, Vec<(&'static str, usize)>) {
    fn start_kind(tag: &Tag) -> Option<&'static str> {
        match tag {
            Tag::Emphasis => Some("em"),
            Tag::Strong => Some("strong"),
            Tag::Link { .. } => Some("link"),
            Tag::Image { .. } => Some("image"),
            Tag::Strikethrough => Some("del"),
            _ => None,
        }
    }
    fn end_kind(end: &TagEnd) -> Option<&'static str> {
        match end {
            TagEnd::Emphasis => Some("em"),
            TagEnd::Strong => Some("strong"),
            TagEnd::Link => Some("link"),
            TagEnd::Image => Some("image"),
            TagEnd::Strikethrough => Some("del"),
            _ => None,
        }
    }
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut c = Counts::default();
    let mut sig = Vec::new();
    let mut stack: Vec<&'static str> = Vec::new();
    // Veil 模型：图片 alt 为纯文本、不产生子节点（CommonMark 描述内可含行内标记，
    // 渲染时并入 alt 文本）。参照侧同口径：图片子树整体视为叶子。
    let mut image_depth = 0usize;
    for ev in Parser::new_ext(md, opts) {
        if image_depth > 0 {
            match ev {
                Event::Start(Tag::Image { .. }) => image_depth += 1,
                Event::End(TagEnd::Image) => image_depth -= 1,
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(Tag::Image { .. }) => {
                c.image += 1;
                sig.push(("image", stack.len()));
                image_depth = 1;
            }
            Event::Start(tag) => {
                if let Some(k) = start_kind(&tag) {
                    match k {
                        "em" => c.emph += 1,
                        "strong" => c.strong += 1,
                        "link" => c.link += 1,
                        _ => c.del += 1,
                    }
                    sig.push((k, stack.len()));
                    stack.push(k);
                }
            }
            Event::End(end) => {
                if let Some(k) = end_kind(&end) {
                    if stack.last() == Some(&k) {
                        stack.pop();
                    }
                }
            }
            Event::Code(_) => {
                c.code += 1;
                sig.push(("code", stack.len()));
            }
            _ => {}
        }
    }
    (c, sig)
}

fn actual_inline(s: &Session) -> Counts {
    fn walk(nodes: &[InlineNode], c: &mut Counts) {
        for n in nodes {
            match n.kind {
                InlineKind::Strong => c.strong += 1,
                InlineKind::Emph => c.emph += 1,
                InlineKind::Code => c.code += 1,
                InlineKind::Link => c.link += 1,
                InlineKind::Image => c.image += 1,
                InlineKind::Strikethrough => c.del += 1,
                InlineKind::Math => {}
            }
            walk(&n.children, c);
        }
    }
    let mut c = Counts::default();
    for (id, _, _) in s.blocks() {
        if let Some(nodes) = s.block_inlines(id) {
            walk(&nodes, &mut c);
        }
    }
    c
}

// ===== Suite D：行内前序森林签名 (kind, depth) =====

// 期望侧签名由 `reference_inline` 一并产出（与 Suite C 同源），见上方。

/// Veil 行内树的前序签名；Math 不产生标签（与 C 口径一致）。
fn actual_inline_structure(s: &Session) -> Vec<(&'static str, usize)> {
    fn walk(nodes: &[InlineNode], depth: usize, out: &mut Vec<(&'static str, usize)>) {
        for n in nodes {
            let kind = match n.kind {
                InlineKind::Strong => "strong",
                InlineKind::Emph => "em",
                InlineKind::Code => "code",
                InlineKind::Link => "link",
                InlineKind::Image => "image",
                InlineKind::Strikethrough => "del",
                InlineKind::Math => continue,
            };
            out.push((kind, depth));
            walk(&n.children, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    for (id, _, _) in s.blocks() {
        if let Some(nodes) = s.block_inlines(id) {
            walk(&nodes, 0, &mut out);
        }
    }
    out
}

fn sig_short(sig: &[(&'static str, usize)]) -> String {
    let mut s = String::new();
    for (k, d) in sig.iter().take(12) {
        s.push_str(&format!("{k}@{d} "));
    }
    if sig.len() > 12 {
        s.push('…');
    }
    s.trim_end().to_string()
}

fn short(seq: &[B]) -> String {
    let mut s = String::new();
    for b in seq.iter().take(8) {
        s.push_str(&format!("{b:?} "));
    }
    if seq.len() > 8 {
        s.push('…');
    }
    s.trim_end().to_string()
}

#[test]
fn commonmark_0_31_2_conformance() {
    let dir = EXAMPLES;
    let index = fs::read_to_string(Path::new(dir).join("index.tsv")).expect("index.tsv");
    let mut total = 0usize;
    let mut a_pass = 0usize;
    let mut b_exact = 0usize;
    let mut b_canon = 0usize;
    let mut c_pass = 0usize;
    let mut b_exact_a = 0usize;
    let mut b_canon_a = 0usize;
    let mut c_pass_a = 0usize;
    let mut panics = 0usize;
    let mut d_pass = 0usize;
    let mut d_pass_a = 0usize;
    let mut by_section: BTreeMap<String, (usize, usize, usize, usize, usize)> = BTreeMap::new();
    let mut a_fail: Vec<String> = Vec::new();
    let mut c_fail: Vec<String> = Vec::new();
    let mut d_fail: Vec<String> = Vec::new();

    for line in index.lines() {
        let mut it = line.split('\t');
        let (Some(id), Some(section), Some(_num)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let md = fs::read_to_string(Path::new(dir).join(format!("{id}.md"))).expect("md");
        let expected = fs::read_to_string(Path::new(dir).join(format!("{id}.html"))).expect("html");
        total += 1;
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let s = Session::open(&md);
            let a = actual_blocks(&s);
            let body = export_body(&s);
            let counts = actual_inline(&s);
            let sig = actual_inline_structure(&s);
            (a, body, counts, sig)
        }));
        let entry = by_section.entry(section.to_string()).or_default();
        entry.0 += 1;
        match outcome {
            Err(_) => {
                panics += 1;
                a_fail.push(format!("#{id} {section}: PANIC"));
                continue;
            }
            Ok((actual, body, counts, asig)) => {
                let exp = expected_blocks(&expected);
                let a_ok = actual == exp;
                if a_ok {
                    a_pass += 1;
                    entry.1 += 1;
                } else if a_fail.len() < 60 {
                    a_fail.push(format!(
                        "#{id} {section}: expected=[{}] actual=[{}]",
                        short(&exp),
                        short(&actual)
                    ));
                }
                let exp_html = expected.replace("\r\n", "\n");
                let body_html = body.replace("\r\n", "\n");
                let be_ok = exp_html == body_html;
                let bc_ok = canonical(&exp_html) == canonical(&body_html);
                let (ec, esig) = reference_inline(&md);
                let c_ok = ec == counts;
                if be_ok {
                    b_exact += 1;
                    entry.2 += 1;
                }
                if bc_ok {
                    b_canon += 1;
                    entry.3 += 1;
                }
                if c_ok {
                    c_pass += 1;
                    entry.4 += 1;
                } else if c_fail.len() < 60 {
                    c_fail.push(format!("#{id} {section}: expected={ec:?} actual={counts:?}"));
                }
                let d_ok = esig == asig;
                if d_ok {
                    d_pass += 1;
                    if a_ok {
                        d_pass_a += 1;
                    }
                } else if d_fail.len() < 60 {
                    d_fail.push(format!(
                        "#{id} {section}: expected=[{}] actual=[{}]",
                        sig_short(&esig),
                        sig_short(&asig)
                    ));
                }
                if a_ok {
                    if be_ok {
                        b_exact_a += 1;
                    }
                    if bc_ok {
                        b_canon_a += 1;
                    }
                    if c_ok {
                        c_pass_a += 1;
                    }
                }
            }
        }
    }

    println!("TOTAL={total} PANICS={panics}");
    println!("A_BLOCK_SEQUENCE={a_pass}/{total} (baseline {BASELINE_A})");
    println!("B_EXPORT_EXACT={b_exact}/{total} (baseline {BASELINE_B_EXACT})");
    println!("B_EXPORT_CANONICAL={b_canon}/{total} (baseline {BASELINE_B_CANON})");
    println!("C_INLINE_COUNTS={c_pass}/{total} (baseline {BASELINE_C})");
    println!("B_EXACT_WHEN_A_PASS={b_exact_a}/{a_pass} (baseline {BASELINE_B_EXACT_WHEN_A})");
    println!("B_CANON_WHEN_A_PASS={b_canon_a}/{a_pass}");
    println!("C_INLINE_WHEN_A_PASS={c_pass_a}/{a_pass} (baseline {BASELINE_C_WHEN_A})");
    println!("D_INLINE_STRUCTURE={d_pass}/{total} (baseline {BASELINE_D})");
    println!("D_STRUCTURE_WHEN_A_PASS={d_pass_a}/{a_pass} (baseline {BASELINE_D_WHEN_A})");
    println!("SECTIONS (total, A, Bexact, Bcanon, C):");
    for (sec, (n, a, be, bc, c)) in &by_section {
        println!("  {sec}: {n}, {a}, {be}, {bc}, {c}");
    }
    println!("--- A failures (first 60) ---");
    for f in &a_fail {
        println!("{f}");
    }
    println!("--- C failures (first 60) ---");
    for f in &c_fail {
        println!("{f}");
    }
    println!("--- D failures (first 60) ---");
    for f in &d_fail {
        println!("{f}");
    }

    assert_eq!(total, 652, "语料规模应为 652 例");
    assert_eq!(panics, 0, "解析全链路不允许 panic");
    assert!(
        a_pass >= BASELINE_A,
        "Suite A 退化：{a_pass} < 基线 {BASELINE_A}"
    );
    assert!(
        b_exact >= BASELINE_B_EXACT,
        "Suite B 精确退化：{b_exact} < 基线 {BASELINE_B_EXACT}"
    );
    assert!(
        b_canon >= BASELINE_B_CANON,
        "Suite B 规范化退化：{b_canon} < 基线 {BASELINE_B_CANON}"
    );
    assert!(
        c_pass >= BASELINE_C,
        "Suite C 退化：{c_pass} < 基线 {BASELINE_C}"
    );
    assert!(
        b_exact_a >= BASELINE_B_EXACT_WHEN_A,
        "条件 B 精确退化：{b_exact_a} < 基线 {BASELINE_B_EXACT_WHEN_A}"
    );
    assert!(
        c_pass_a >= BASELINE_C_WHEN_A,
        "条件 C 退化：{c_pass_a} < 基线 {BASELINE_C_WHEN_A}"
    );
    assert!(
        d_pass >= BASELINE_D,
        "Suite D 退化：{d_pass} < 基线 {BASELINE_D}"
    );
    assert!(
        d_pass_a >= BASELINE_D_WHEN_A,
        "条件 D 退化：{d_pass_a} < 基线 {BASELINE_D_WHEN_A}"
    );
    assert_eq!(
        by_section.len(),
        BASELINE_C_SECTION.len(),
        "语料分节数变化：{} -> {}",
        BASELINE_C_SECTION.len(),
        by_section.len()
    );
    for &(sec, base) in BASELINE_C_SECTION {
        let got = by_section.get(sec).map_or(0, |e| e.4);
        assert!(got >= base, "Suite C 分段退化：{sec} {got} < 基线 {base}");
    }
}
