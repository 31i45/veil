//! C10 探针：测 10 万字文档打开耗时与内存。
//! 运行：cargo run --release -p veil --example bench_c10

use std::hint::black_box;
use std::time::Instant;
use veil_core::Session;
use veil_ui::LayoutEngine;

fn main() {
    let path = format!("{}\\veil-walkthrough\\03-10万字.txt", std::env::var("TEMP").unwrap());
    let text = std::fs::read_to_string(&path).expect("读取 10 万字文件失败");
    let chars = text.chars().count();
    let bytes = text.len();

    println!("文档: {path}");
    println!("字符数: {chars}, 字节: {bytes}");

    // 测量 1：Session::open（纯解析）
    let t = Instant::now();
    let session = Session::open(&text);
    let open_ms = t.elapsed().as_secs_f64() * 1000.0;
    let block_count = session.blocks().len();
    println!("Session::open: {open_ms:.2} ms, 块数: {block_count}");

    // 测量 2：open + 全部块排版（模拟首帧渲染）
    let t = Instant::now();
    let session = Session::open(&text);
    let mut eng = LayoutEngine::new();
    let blocks = session.blocks();
    for (id, kind, range) in &blocks {
        let body = session.text_slice(*range).unwrap_or_default();
        let inlines = session.block_inlines(*id).unwrap_or_default();
        let l = eng.layout_block(kind.clone(), &body, *range, &inlines, 1200.0, usize::MAX, &[], false);
        black_box(l.height);
    }
    let full_layout_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("open + 全部排版: {full_layout_ms:.2} ms");

    // 门禁判定
    let open_pass = open_ms <= 2000.0;
    let layout_pass = full_layout_ms <= 2000.0;
    println!("---");
    println!("C10 门禁（打开 ≤2s）: open={open_ms:.0}ms {} | layout={full_layout_ms:.0}ms {}",
        if open_pass { "✅" } else { "❌" },
        if layout_pass { "✅" } else { "❌" });
    println!("内存: 需要任务管理器真机实测（私有工作集 ≤150MB）");
}
