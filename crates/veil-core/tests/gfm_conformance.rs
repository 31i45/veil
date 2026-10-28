//! GFM 扩展一致性门禁（cmark-gfm spec.txt 的四个扩展节，23 例）。
//!
//! 与 CommonMark 门禁共用 `tests/common/mod.rs` 的口径，差异只在两点：
//! - Suite A 额外把期望侧的生成 `<table>` 识别为表格块（CommonMark 语料里
//!   `<table>` 只可能是原始 HTML，两侧都不计）。
//! - Suite B/C/D 的参照仍为 pulldown-cmark 0.13：它支持表格/删除线/任务列表，
//!   但不支持 GFM 裸 URL/邮箱自动链接（`Options` 无对应开关），故自动链接节的
//!   C/D 只能验证「两侧都没有」；该能力缺口由 Suite B 逐字节暴露。
//! - Suite B 用 `gfm_canonical` 抹平 cmark-gfm 规范 HTML 与 pulldown 输出的序列化
//!   差异（表格 align/style、`<input>` 属性序与自闭合、空 `<tbody>`），只等价化
//!   不放松结构；原始逐字节 B 仅打印、不作门槛（渲染器不同源）。
//! - 0d 门槛范围 = §5.4「表格 / 任务列表 / 删除线」三节 12 例（A/C/D 全量 23 例）；
//!   自动链接扩展 11 例为已知缺口，仅作观测，待后续增量评估。
//!
//! 门禁方向与 CommonMark 一致：只允许改善、不允许退化（`>=` 基线）。

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use common::{
    actual_blocks, actual_inline, actual_inline_structure, export_body, expected_blocks,
    gfm_canonical, reference_inline, short, sig_short,
};
use veil_core::Session;

/// 基线（2026-10-28 实测；只允许改善）。
const BASELINE_A: usize = 23;
/// Suite B 规范化（仅抹平渲染器序列化差异，见 `common::gfm_canonical`）。
/// 23 例中 11 例为自动链接扩展：产品导出（pulldown-cmark）与 Veil 行内解析器
/// 均不支持裸 URL/邮箱自动链接，是 0d 计划（§5.4 表格/任务列表/删除线）之外的
/// 已知缺口，不计入门槛；计划内三节 12/12 全通过。
const BASELINE_B_CANON: usize = 12;
const BASELINE_C: usize = 23;
const BASELINE_D: usize = 23;
/// 0d 计划内三节的 B 规范化基线（§5.4 目标 ≥95%，实测 100%）。
const BASELINE_B_CANON_SECTION: &[(&str, usize)] = &[
    ("tables", 8),
    ("task_list_items", 2),
    ("strikethrough", 2),
];

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/conformance/gfm");

#[test]
fn gfm_extension_conformance() {
    let dir = EXAMPLES;
    let index = fs::read_to_string(Path::new(dir).join("index.tsv")).expect("index.tsv");
    let mut total = 0usize;
    let mut a_pass = 0usize;
    let mut b_exact = 0usize;
    let mut b_canon = 0usize;
    let mut c_pass = 0usize;
    let mut d_pass = 0usize;
    let mut panics = 0usize;
    let mut by_section: BTreeMap<String, (usize, usize, usize, usize, usize, usize)> =
        BTreeMap::new();
    let mut a_fail: Vec<String> = Vec::new();
    let mut b_fail: Vec<String> = Vec::new();
    let mut bc_fail: Vec<String> = Vec::new();
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
                let exp = expected_blocks(&expected, true);
                let a_ok = actual == exp;
                if a_ok {
                    a_pass += 1;
                    entry.1 += 1;
                } else {
                    a_fail.push(format!(
                        "#{id} {section}: expected=[{}] actual=[{}]",
                        short(&exp),
                        short(&actual)
                    ));
                }
                let exp_html = expected.replace("\r\n", "\n");
                let body_html = body.replace("\r\n", "\n");
                let be_ok = exp_html == body_html;
                let bc_ok = gfm_canonical(&exp_html) == gfm_canonical(&body_html);
                let (ec, esig) = reference_inline(&md);
                let c_ok = ec == counts;
                let d_ok = esig == asig;
                if be_ok {
                    b_exact += 1;
                    entry.2 += 1;
                } else {
                    b_fail.push(format!(
                        "#{id} {section}: expected=[{}] actual=[{}]",
                        short_text(&exp_html, 160),
                        short_text(&body_html, 160)
                    ));
                }
                if bc_ok {
                    b_canon += 1;
                    entry.3 += 1;
                } else {
                    bc_fail.push(format!(
                        "#{id} {section}: expected=[{}] actual=[{}]",
                        short_text(&gfm_canonical(&exp_html), 200),
                        short_text(&gfm_canonical(&body_html), 200)
                    ));
                }
                if c_ok {
                    c_pass += 1;
                    entry.4 += 1;
                } else {
                    c_fail.push(format!("#{id} {section}: expected={ec:?} actual={counts:?}"));
                }
                if d_ok {
                    d_pass += 1;
                    entry.5 += 1;
                } else {
                    d_fail.push(format!(
                        "#{id} {section}: expected=[{}] actual=[{}]",
                        sig_short(&esig),
                        sig_short(&asig)
                    ));
                }
            }
        }
    }

    println!("GFM TOTAL={total} PANICS={panics}");
    println!("GFM A_BLOCK_SEQUENCE={a_pass}/{total}");
    println!("GFM B_EXPORT_EXACT={b_exact}/{total}");
    println!("GFM B_EXPORT_CANONICAL={b_canon}/{total}");
    println!("GFM C_INLINE_COUNTS={c_pass}/{total}");
    println!("GFM D_INLINE_STRUCTURE={d_pass}/{total}");
    println!("GFM SECTIONS (total, A, Bexact, Bcanon, C, D):");
    for (sec, (n, a, be, bc, c, d)) in &by_section {
        println!("  {sec}: {n}, {a}, {be}, {bc}, {c}, {d}");
    }
    println!("--- A failures ---");
    for f in &a_fail {
        println!("{f}");
    }
    println!("--- B failures (first 60) ---");
    for f in &b_fail {
        println!("{f}");
    }
    println!("--- B canonical failures (first 60) ---");
    for f in &bc_fail {
        println!("{f}");
    }
    println!("--- C failures ---");
    for f in &c_fail {
        println!("{f}");
    }
    println!("--- D failures ---");
    for f in &d_fail {
        println!("{f}");
    }

    assert_eq!(total, 23, "GFM 语料规模应为 23 例");
    assert_eq!(panics, 0, "解析全链路不允许 panic");
    assert!(a_pass >= BASELINE_A, "GFM Suite A 退化：{a_pass}");
    assert!(b_canon >= BASELINE_B_CANON, "GFM 规范化 B 退化：{b_canon}");
    assert!(c_pass >= BASELINE_C, "GFM Suite C 退化：{c_pass}");
    assert!(d_pass >= BASELINE_D, "GFM Suite D 退化：{d_pass}");
    for &(sec, base) in BASELINE_B_CANON_SECTION {
        let got = by_section.get(sec).map_or(0, |e| e.3);
        assert!(got >= base, "GFM B 分段退化：{sec} {got} < 基线 {base}");
    }
}

/// 单行化摘要：失败信息里的 HTML 便于肉眼比对。
fn short_text(s: &str, max: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.len() > max {
        format!("{}…", &one[..max])
    } else {
        one
    }
}
