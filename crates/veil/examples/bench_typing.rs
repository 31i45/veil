//! 纯度量示例（不接测试门禁）：冷启动、单次键入、12 次全量枚举、
//! 脏块重排、帧合成、全局分配计数。仅用现有公开 API，不引入新依赖。
//!
//! 运行：cargo run --release -p veil --example bench_typing

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use veil_core::{BlockKind, Edit, Session};
use veil_egui::{Frame, LayoutEngine};

// ---- 全局分配计数器：alloc/dealloc 次数与字节数，另记释放字节算存活堆 ----

struct CountingAlloc;

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static DEALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static FREED_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        FREED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        System.alloc_zeroed(layout)
    }
    // realloc 走 GlobalAlloc 默认实现（alloc + 拷贝 + dealloc），计数天然被覆盖
}

fn snapshot() -> (u64, u64) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
    )
}

fn live_bytes() -> u64 {
    let a = ALLOC_BYTES.load(Ordering::Relaxed);
    let f = FREED_BYTES.load(Ordering::Relaxed);
    a.saturating_sub(f)
}

// ---- 统计与排版辅助 ----

/// 最近秩百分位（会排序并复用切片）。
fn pctl(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v[idx.saturating_sub(1).min(v.len() - 1)]
}

/// 构造约 7 块/轮的文档；715 轮 → 5005 块。
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

/// 文档中部某个段落块内的安全插入点。
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

/// 冷启动：Session::open + LayoutEngine::new + 全部块各排版一次。重复 5 次。
fn measure_cold_start(text: &str) -> (f64, f64) {
    let mut runs = Vec::with_capacity(5);
    for _ in 0..5 {
        let t = Instant::now();
        let session = Session::open(text);
        let mut eng = LayoutEngine::new();
        let blocks = session.blocks();
        for (id, kind, range) in &blocks {
            let body = session.text_slice(*range).unwrap_or_default();
            let inlines = session.block_inlines(*id).unwrap_or_default();
            let l = eng.layout_block(
                kind.clone(),
                &body,
                *range,
                &inlines,
                1200.0,
                usize::MAX,
                &[],
            );
            black_box(l.height);
        }
        runs.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    runs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (pctl(&mut runs, 50.0), runs[runs.len() - 1])
}

/// 连续 1000 次单字符插入的 apply 耗时（µs），返回 (p50, p95)。
fn measure_keystrokes(text: &str, caret0: usize) -> (f64, f64) {
    let mut session = Session::open(text);
    let mut samples = Vec::with_capacity(1000);
    for at in (caret0..).take(1000) {
        let t = Instant::now();
        let ev = session
            .apply(Edit::Insert {
                at,
                text: "a".into(),
            })
            .expect("插入点恒合法");
        samples.push(t.elapsed().as_secs_f64() * 1e6);
        black_box(ev.dirty_blocks.len());
    }
    (pctl(&mut samples, 50.0), pctl(&mut samples, 95.0))
}

/// 病理场景：每次编辑后 12 次 blocks() 并遍历求和（只计 12 次枚举，不含 apply）。
/// 返回 ((p50, p95), (alloc_count, alloc_bytes))，分配计数覆盖「1000 次编辑 × 12 次枚举」。
fn measure_full_scan(text: &str, caret0: usize) -> ((f64, f64), (u64, u64)) {
    let mut session = Session::open(text);
    let mut samples = Vec::with_capacity(1000);
    let (a0, b0) = snapshot();
    for at in (caret0..).take(1000) {
        session
            .apply(Edit::Insert {
                at,
                text: "a".into(),
            })
            .expect("插入点恒合法");
        let t = Instant::now();
        let mut sum = 0usize;
        for _ in 0..12 {
            let blocks = session.blocks();
            for (_, _, r) in &blocks {
                sum = sum.wrapping_add(r.end - r.start);
            }
            black_box(blocks.len());
        }
        samples.push(t.elapsed().as_secs_f64() * 1e6);
        black_box(sum);
    }
    let (a1, b1) = snapshot();
    (
        (pctl(&mut samples, 50.0), pctl(&mut samples, 95.0)),
        (a1 - a0, b1 - b0),
    )
}

/// 脏块路径：每次编辑后只重排 apply 返回的脏块。块区间解析用一次 blocks()（不计时）。
/// 返回 (p50, p95)（µs）。
fn measure_dirty_layout(text: &str, caret0: usize) -> (f64, f64) {
    let mut session = Session::open(text);
    let mut eng = LayoutEngine::new();
    let mut samples = Vec::with_capacity(1000);
    for at in (caret0..).take(1000) {
        let ev = session
            .apply(Edit::Insert {
                at,
                text: "a".into(),
            })
            .expect("插入点恒合法");
        // 解析脏块 id → 种类/区间（不并入计时，见最终报告说明）
        let all = session.blocks();
        let t = Instant::now();
        for id in &ev.dirty_blocks {
            if let Some((_, kind, range)) = all.iter().find(|(bid, _, _)| bid == id) {
                let body = session.text_slice(*range).unwrap_or_default();
                let inlines = session.block_inlines(*id).unwrap_or_default();
                let l = eng.layout_block(
                    kind.clone(),
                    &body,
                    *range,
                    &inlines,
                    1200.0,
                    usize::MAX,
                    &[],
                );
                black_box(l.height);
            }
        }
        samples.push(t.elapsed().as_secs_f64() * 1e6);
    }
    (pctl(&mut samples, 50.0), pctl(&mut samples, 95.0))
}

/// 1920×1080 帧：构造 + 铺满背景（含分配），100 次，(p50, p95) 毫秒。
fn measure_frame_alloc() -> (f64, f64) {
    const W: u32 = 1920;
    const H: u32 = 1080;
    let mut samples = Vec::with_capacity(100);
    for _ in 0..100 {
        let t = Instant::now();
        let mut f = Frame::new(W, H, [0, 0, 0, 0]);
        f.fill_rect(0, 0, W, H, [250, 250, 245, 255]);
        black_box(f.pixels.len());
        samples.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    (pctl(&mut samples, 50.0), pctl(&mut samples, 95.0))
}

/// 热路径：复用同一缓冲做 reset（尺寸不变 → 零分配）+ 铺满背景，100 次。
fn measure_frame_reuse() -> (f64, f64) {
    const W: u32 = 1920;
    const H: u32 = 1080;
    let mut f = Frame::new(W, H, [0, 0, 0, 0]);
    let mut samples = Vec::with_capacity(100);
    for _ in 0..100 {
        let t = Instant::now();
        f.reset(W, H, [250, 250, 245, 255]);
        black_box(f.pixels.len());
        samples.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    (pctl(&mut samples, 50.0), pctl(&mut samples, 95.0))
}

fn main() {
    const ITERS: usize = 715; // 7 块/轮 → 5005 块
    let doc = build_doc(ITERS);

    let block_count = Session::open(&doc).blocks().len();
    let caret = pick_caret(&Session::open(&doc));

    let (cold_med, cold_max) = measure_cold_start(&doc);
    let (key_p50, key_p95) = measure_keystrokes(&doc, caret);
    let ((scan_p50, scan_p95), (alloc_count, alloc_bytes)) = measure_full_scan(&doc, caret);
    let (dirty_p50, dirty_p95) = measure_dirty_layout(&doc, caret);
    let (frame_p50, frame_p95) = measure_frame_alloc();
    let (frame_reuse_p50, frame_reuse_p95) = measure_frame_reuse();

    println!("cold_start_ms={cold_med:.2}/{cold_max:.2}");
    println!("keystroke_core_us={key_p50:.2}/{key_p95:.2}");
    println!("full_scan_cost_us={scan_p50:.2}/{scan_p95:.2}");
    println!("layout_dirty_us={dirty_p50:.2}/{dirty_p95:.2}");
    println!("frame_alloc_ms={frame_p50:.2}/{frame_p95:.2}");
    println!("frame_reuse_ms={frame_reuse_p50:.2}/{frame_reuse_p95:.2}");
    println!("alloc_count={alloc_count}");
    println!("alloc_bytes={alloc_bytes}");
    println!(
        "peak_heap_mb={:.2}",
        live_bytes() as f64 / (1024.0 * 1024.0)
    );
    println!("block_count={block_count}");

    assert!(block_count >= 4500, "文档块数不足 4500：{block_count}");
}
