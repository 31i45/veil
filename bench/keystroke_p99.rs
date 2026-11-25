//! 击键 p99 补充基准（不新增依赖，仅 std::time::Instant）。
//!
//! 口径与 `crates/veil/examples/bench_typing.rs::measure_keystrokes` 一致：
//! 5005 块结构文档、光标停在段落块中部、连续 1000 次 `Session::apply(Edit::Insert)`
//! 单字符插入，最近秩百分位。原 example 只输出 p50/p95，这里补 p99 与 max。
//!
//! 构建与运行（由 bench/run_bench.ps1 调用，参数与 release profile 对齐）：
//!   rustc --edition 2021 -C opt-level=s -C codegen-units=1 -C panic=abort -C lto=fat \
//!     -L target/release/deps --extern veil_core=<libveil_core-*.rlib> \
//!     bench/keystroke_p99.rs -o <out.exe>
//!
//! 注意：这里只测内核 apply 路径；全路径（apply + refresh_index + 布局 + 光栅 + 帧合成）
//! 的 p50/p95/max 由既有 ignored 探针 `perf_keystroke_path` 输出，p99 不由本文件冒充。

use std::hint::black_box;
use std::time::Instant;

use veil_core::{BlockKind, Edit, Session};

/// 最近秩百分位（与 bench_typing 的 pctl 同义）。
fn pctl(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((p / 100.0) * s.len() as f64).ceil() as usize;
    s[idx.saturating_sub(1).min(s.len() - 1)]
}

/// 与 bench_typing 完全相同的文档构造：715 轮 × 7 块 ≈ 5005 块。
fn build_doc(iters: usize) -> String {
    let mut s = String::with_capacity(iters * 280);
    for i in 0..iters {
        s.push_str(&format!("## Section {i}\n\n"));
        s.push_str(&format!(
            "Paragraph {i} with **bold** text, `code`, and a [link](https://example.com/{i}).\n\n"
        ));
        s.push_str(&format!("- item A {i}\n- item B {i}\n\n"));
        s.push_str("```rust\n");
        s.push_str(&format!("fn f{i}() -> u32 {{ {i} }}\n"));
        s.push_str("```\n\n");
        s.push_str(&format!("> quote {i}\n\n"));
        s.push_str("---\n\n");
    }
    s
}

/// 文档中部段落块内的安全插入点（同 bench_typing::pick_caret）。
fn pick_caret(session: &Session) -> usize {
    let mid = session.text_len() / 2;
    let blocks = session.blocks();
    let pick = blocks
        .iter()
        .find(|(_, k, r)| *k == BlockKind::Paragraph && r.start > mid)
        .or_else(|| blocks.iter().find(|(_, k, _)| *k == BlockKind::Paragraph));
    let r = pick.expect("文档应含段落块").2;
    r.start + 8
}

fn main() {
    const ITERS: usize = 715;
    let doc = build_doc(ITERS);
    let block_count = Session::open(&doc).blocks().len();
    let caret = pick_caret(&Session::open(&doc));

    let mut session = Session::open(&doc);
    let mut samples = Vec::with_capacity(1000);
    let mut probes = Vec::with_capacity(1000);
    for at in (caret..).take(1000) {
        let t = Instant::now();
        let ev = session
            .apply(Edit::Insert {
                at,
                text: "a".into(),
            })
            .expect("插入点恒合法");
        samples.push(t.elapsed().as_secs_f64() * 1e6);
        probes.push(ev.dirty_blocks.len());
        black_box(ev.dirty_blocks.len());
    }

    println!("keystroke_core_samples={}", samples.len());
    println!("keystroke_core_p50_us={:.3}", pctl(&samples, 50.0));
    println!("keystroke_core_p95_us={:.3}", pctl(&samples, 95.0));
    println!("keystroke_core_p99_us={:.3}", pctl(&samples, 99.0));
    println!("keystroke_core_max_us={:.3}", pctl(&samples, 100.0));
    println!("keystroke_core_dirty_blocks={}", probes.iter().max().unwrap_or(&0));
    println!("keystroke_core_blocks={block_count}");
    println!("keystroke_core_doc_chars={}", doc.chars().count());
    println!("keystroke_core_doc_bytes={}", doc.len());

    assert!(block_count >= 4500, "文档块数不足 4500：{block_count}");
}
