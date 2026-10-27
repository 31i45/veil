//! CommonMark 0.31.2 一致性门禁（652 例官方规范语料，固化自 P1 评审 harness）。
//!
//! 三套判定（口径详见 docs/review/P1-commonmark-conformance.md §2）：
//! - Suite A：Veil 自有块解析器的顶层块序列 vs 规范期望 HTML 的顶层块标签序列。
//! - Suite B：产品导出路径复刻（块分段 + pulldown-cmark）的 HTML vs 规范 HTML。
//! - Suite C：Veil 自有行内节点计数 vs 规范 HTML 行内标签计数（弱判定，视为上限）。
//!
//! 门禁方向：三套指标只允许改善、不允许退化（`>=` 基线）。修复提升通过率后，
//! 应同步上调基线常量。已知 A 存在少量工具误判（规范 HTML 为裸行内标签时
//! 期望序列扫描为空），此类由基线吸收，不计产品缺陷。

use std::collections::BTreeMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use pulldown_cmark::{html, Options, Parser};
use veil_core::{BlockKind, InlineKind, InlineNode, Range, Session};

/// 一致性基线（0c P0-3 宽松/多段列表项 + 引用懒续行完成后，2026-10-27）。
/// 上一基线（P0-2 Setext 标题，2026-10-27）：A=518 B=565/566 C=505 条件 B=495 C=402。
/// P0-3 收益（A 518→560，+42）：Block quotes 段 23/25（懒续行）、List items 段 44/48、
/// Lists 段 23/26（区域扫描 + 内容列规则 + 宽松列表）。剩余 A 失败：0231/0236/0253/
/// 0257/0272/0289/0313 归 P1（缩进代码/围栏边界），0308/0309 归 P2（HTML 块）；
/// C 持平 505（行内强调/自动链接属 P1）。
const BASELINE_A: usize = 560;
const BASELINE_B_EXACT: usize = 606;
const BASELINE_B_CANON: usize = 607;
const BASELINE_C: usize = 505;
/// 条件基线（仅 A 通过时统计）。
const BASELINE_B_EXACT_WHEN_A: usize = 538;
const BASELINE_C_WHEN_A: usize = 444;

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
    Html,
    Other,
}

fn base_name(tag: &str) -> String {
    tag.trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn is_void(name: &str) -> bool {
    matches!(name, "br" | "img" | "hr" | "input" | "meta" | "link")
}

/// 规范期望 HTML 的顶层块序列（只取块容器，不进入嵌套）。
fn expected_blocks(html: &str) -> Vec<B> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < html.len() {
        let Some(off) = html[i..].find('<') else {
            break;
        };
        i += off;
        if html[i..].starts_with("<!--") {
            if depth == 0 {
                out.push(B::Html);
            }
            match html[i..].find("-->") {
                Some(end) => i += end + 3,
                None => break,
            }
            continue;
        }
        let Some(end) = html[i..].find('>') else {
            break;
        };
        let tag = &html[i + 1..i + end];
        let name = base_name(tag);
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        i += end + 1;
        if closing {
            depth = depth.saturating_sub(1);
            continue;
        }
        if self_closing || is_void(&name) {
            if depth == 0 && name == "hr" {
                out.push(B::Rule);
            }
            continue;
        }
        if depth == 0 {
            let b = match name.as_str() {
                "p" => Some(B::P),
                "h1" => Some(B::H(1)),
                "h2" => Some(B::H(2)),
                "h3" => Some(B::H(3)),
                "h4" => Some(B::H(4)),
                "h5" => Some(B::H(5)),
                "h6" => Some(B::H(6)),
                "pre" => Some(B::Pre),
                "blockquote" => Some(B::Quote),
                "ul" | "ol" => Some(B::List),
                // CommonMark 规范不含 GFM 表格：<table> 只会来自原始 HTML 块
                "table" | "div" | "section" | "article" | "script" | "style" | "details"
                | "summary" => Some(B::Html),
                _ => None,
            };
            if let Some(b) = b {
                out.push(b);
            }
        }
        depth += 1;
    }
    out
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
            BlockKind::FencedCode(_) => B::Pre,
            BlockKind::Math => B::Other,
            BlockKind::Table => B::Table,
            BlockKind::BlockQuote => B::Quote,
            BlockKind::ThematicBreak => B::Rule,
            BlockKind::LinkRefDef => continue,
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

fn expected_inline(html: &str) -> Counts {
    let mut c = Counts::default();
    let mut pre = 0i32;
    let mut i = 0usize;
    while i < html.len() {
        let Some(off) = html[i..].find('<') else {
            break;
        };
        i += off;
        if html[i..].starts_with("<!--") {
            match html[i..].find("-->") {
                Some(e) => i += e + 3,
                None => break,
            }
            continue;
        }
        let Some(end) = html[i..].find('>') else {
            break;
        };
        let tag = &html[i + 1..i + end];
        let name = base_name(tag);
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        i += end + 1;
        if closing {
            if name == "pre" {
                pre -= 1;
            }
            continue;
        }
        match name.as_str() {
            "strong" => c.strong += 1,
            "em" => c.emph += 1,
            "code" if pre == 0 => c.code += 1,
            "a" => c.link += 1,
            "img" => c.image += 1,
            "del" => c.del += 1,
            _ => {}
        }
        if name == "pre" {
            pre += 1;
        }
        let _ = self_closing;
    }
    c
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
    let mut by_section: BTreeMap<String, (usize, usize, usize, usize, usize)> = BTreeMap::new();
    let mut a_fail: Vec<String> = Vec::new();
    let mut c_fail: Vec<String> = Vec::new();

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
            (a, body, counts)
        }));
        let entry = by_section.entry(section.to_string()).or_default();
        entry.0 += 1;
        match outcome {
            Err(_) => {
                panics += 1;
                a_fail.push(format!("#{id} {section}: PANIC"));
                continue;
            }
            Ok((actual, body, counts)) => {
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
                let ec = expected_inline(&expected);
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
}
