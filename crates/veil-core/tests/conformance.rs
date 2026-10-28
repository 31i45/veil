//! CommonMark 0.31.2 一致性门禁（652 例官方规范语料，固化自 P1 评审 harness）。
//!
//! 四套判定（口径详见 docs/review/P1-commonmark-conformance.md §2，实现见
//! `tests/common/mod.rs`）：
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

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use common::{
    actual_blocks, actual_inline, actual_inline_structure, canonical, export_body, expected_blocks,
    reference_inline, short, sig_short,
};
use veil_core::Session;

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
                let exp = expected_blocks(&expected, false);
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
