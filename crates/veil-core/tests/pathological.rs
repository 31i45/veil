//! P3 鲁棒性复测探针（G1 门禁）：病态输入矩阵。
//!
//! 判定标准：不 panic、不挂起、块序列可枚举、全文往返一致。
//! 运行（release，耗时数字才有意义）：
//!   cargo test -p veil-core --release --test pathological -- --ignored --nocapture
#![allow(dead_code)]

use std::time::Instant;
use veil_core::{Range, Session};

/// 一类病态语料：(名称, 构造)。
fn cases() -> Vec<(&'static str, String)> {
    let mut out = Vec::new();

    out.push(("empty", String::new()));

    // CJK 8000 段
    let mut s = String::new();
    for i in 0..8000 {
        s.push_str(&format!("第{i}段中文内容，用于检验长文档的解析与分块稳定性。\n\n"));
    }
    out.push(("cjk_8000_paragraphs", s));

    // 10MB 单行（一个巨型段落）
    out.push(("10mb_single_line", "a".repeat(10 * 1024 * 1024)));

    // 未闭合围栏 10 万行
    let mut s = String::from("```\n");
    for _ in 0..100_000 {
        s.push_str("code line\n");
    }
    out.push(("unclosed_fence_100k_lines", s));

    // 2 万 `*` 嵌套
    out.push(("20k_asterisk_runs", "*".repeat(20_000)));

    // 5 千层 `[` 嵌套
    out.push(("5k_bracket_nesting", "[".repeat(5_000)));

    // 1 万列表项
    let mut s = String::new();
    for i in 0..10_000 {
        s.push_str(&format!("- 列表项 {i}\n"));
    }
    out.push(("10k_list_items", s));

    // 1 万行引用
    let mut s = String::new();
    for _ in 0..10_000 {
        s.push_str("> quote line\n");
    }
    out.push(("10k_quote_lines", s));

    // 1000 行表格
    let mut s = String::from("| a | b |\n| --- | --- |\n");
    for _ in 0..1000 {
        s.push_str("| 1 | 2 |\n");
    }
    out.push(("1000_row_table", s));

    // 12 万块混合文档
    let mut s = String::new();
    for i in 0..30_000 {
        s.push_str(&format!(
            "## 标题 {i}\n\n段落 {i} 的正文内容，包含 **粗体** 与 `代码`。\n\n- 项目 {i}\n\n> 引用 {i}\n\n"
        ));
    }
    out.push(("120k_mixed_blocks", s));

    out
}

#[test]
#[ignore = "病态输入探针，release 下运行才有参考意义"]
fn pathological_inputs_matrix() {
    println!("{:<24} {:>10} {:>10} {:>9} {:>7}", "case", "bytes", "ms", "blocks", "roundtrip");
    for (name, text) in cases() {
        let bytes = text.len();
        let t = Instant::now();
        let result = std::panic::catch_unwind(|| {
            let s = Session::open(&text);
            let blocks = s.blocks().len();
            let rt = s.text_slice(Range::new(0, s.text_len()));
            (blocks, rt)
        });
        let ms = t.elapsed().as_secs_f64() * 1e3;
        match result {
            Ok((blocks, rt)) => {
                let ok = rt.as_deref() == Some(text.as_str()) || text.is_empty();
                println!("{name:<24} {bytes:>10} {ms:>10.2} {blocks:>9} {:>7}", if ok { "是" } else { "否" });
                assert!(ok, "{name}: 往返不一致");
            }
            Err(_) => panic!("{name}: 解析 panic"),
        }
    }
}
