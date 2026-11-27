//! Task 5 性能探针（默认忽略）：5000 标题的打开与滚动、版本历史面板 5000 快照的
//! 打开/滚动/关闭。全部走 headless 真实路径（App 构造、render、历史面板逻辑与绘制），
//! 只有 release 优化级别下的数字有参考意义，由 `tools/verify/probe-gate.ps1` 覆盖。
//!
//! cargo test -p veil-ui --release --lib -- --ignored --nocapture perf_5000_headings
//! cargo test -p veil-ui --release --lib -- --ignored --nocapture perf_history_5000

use super::*;

/// 5000 标题文档（与 `docs/qa/w11-load-state-measurement.md` 的大纲来源同形）：
/// 解析 + 索引 + 首帧 + 300 步滚轮重绘。
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_5000_headings() {
    let mut text = String::with_capacity(5000 * 48);
    for i in 0..5000 {
        text.push_str("## 标题 ");
        text.push_str(&i.to_string());
        text.push_str("\n\n段落 ");
        text.push_str(&i.to_string());
        text.push_str("：用于五千标题的打开与滚动测量。\n\n");
    }
    let store = MemStore::default();
    // 打开是单次观测：测试内采 3 次取 p50 再断言；滚动本就有 300 步分布。
    const OPEN_SAMPLES: usize = 3;
    let mut opens = Vec::with_capacity(OPEN_SAMPLES);
    let mut keep = None;
    for _ in 0..OPEN_SAMPLES {
        let t = Instant::now();
        let mut candidate = app(&text, None, false, &store);
        candidate.paint();
        opens.push(t.elapsed().as_secs_f64() * 1e3);
        keep = Some(candidate);
    }
    let mut a = keep.expect("至少构造一次 App");
    opens.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let open_p50 = opens[OPEN_SAMPLES / 2];

    let t = Instant::now();
    let headings = a
        .index
        .blocks
        .iter()
        .filter(|g| matches!(g.kind, BlockKind::AtxHeading(_)))
        .count();
    let headings_us = t.elapsed().as_secs_f64() * 1e6;
    assert_eq!(headings, 5000, "构造出 5000 个 ATX 标题");

    let vp = a.viewport();
    a.input_state.mouse =
        PhysicalPosition::new((vp.x + vp.w / 2.0) as f64, (vp.y + vp.h / 2.0) as f64);
    let step = -3.0 * BASE_LINE_HEIGHT * a.input_state.scale;
    let mut samples = Vec::with_capacity(300);
    for _ in 0..300 {
        let t = Instant::now();
        a.on_wheel(step);
        let _ = a.render();
        samples.push(t.elapsed().as_secs_f64() * 1e3);
    }
    assert!(a.window_state.scroll_y > 0.0, "滚轮须推动文档");
    samples.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let pick = |q: f64| {
        let i = ((samples.len() as f64 * q).ceil() as usize)
            .saturating_sub(1)
            .min(samples.len() - 1);
        samples[i]
    };

    println!("perf_5000_headings open_ms_p50={open_p50:.3} headings_us={headings_us:.1}");
    println!("PROBE name=perf_5000_headings metric=headings value={headings} unit=count");
    println!("PROBE name=perf_5000_headings metric=open_ms value={open_p50:.3} unit=ms");
    println!("PROBE name=perf_5000_headings metric=headings_us value={headings_us:.1} unit=us");
    println!(
        "PROBE name=perf_5000_headings metric=scroll_p50_ms value={:.3} unit=ms",
        pick(0.5)
    );
    println!(
        "PROBE name=perf_5000_headings metric=scroll_p95_ms value={:.3} unit=ms",
        pick(0.95)
    );
    // 回归上限（regression ceiling，不是目标值）：本机 9 轮实测 open_ms 91.1–96.6 ms、
    // scroll_p95 0.845–1.145 ms；open 取 3 次 p50，上限取实测最大值上方 ≥20% 再加抖动余量：
    // open ≤200 ms、单步滚动 p95 ≤2.5 ms。只拦按标题数线性膨胀的回归。
    assert!(
        open_p50 <= 200.0,
        "5000 标题打开 p50 {open_p50:.1}ms 超回归上限 200 ms（regression ceiling，非目标值）"
    );
    assert!(
        pick(0.95) <= 2.5,
        "5000 标题滚动 p95 {:.3}ms 超回归上限 2.5 ms（regression ceiling，非目标值）",
        pick(0.95)
    );
}

/// 版本历史面板 5000 快照：打开（读取列表 + 首帧重绘）、单步滚轮重绘、关闭。
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_history_5000() {
    let store = MemStore::default();
    let mut snap = FakeSnapshot::default();
    let mut a = app_history(
        "current\n",
        Some(r"C:\notes\perf\history5000.md"),
        &store,
        &snap,
    );
    let key = a.doc.history_key;
    for i in 0..5000 {
        snap.save_snapshot(key, &format!("v{i}\n")).unwrap();
    }

    // 打开/关闭是单次观测：各采 3 次取 p50 再断言；滚动仍有 300 步分布。
    const SAMPLES: usize = 3;
    let mut opens = Vec::with_capacity(SAMPLES);
    let mut closes = Vec::with_capacity(SAMPLES);
    let mut samples = Vec::with_capacity(300);
    let mut entries = 0usize;
    for round in 0..SAMPLES {
        let t = Instant::now();
        a.open_history();
        let _ = a.render();
        opens.push(t.elapsed().as_secs_f64() * 1e3);
        entries = a.ui_state.history.entries.len();

        if round == 0 {
            let l = a.history_layout();
            a.input_state.mouse =
                PhysicalPosition::new((l.panel.x + 40.0) as f64, (l.panel.y + 200.0) as f64);
            for _ in 0..300 {
                let t = Instant::now();
                a.on_wheel(-60.0);
                let _ = a.render();
                samples.push(t.elapsed().as_secs_f64() * 1e3);
            }
            assert!(a.ui_state.history.scroll > 0.0, "面板须实际滚动");
        }

        let t = Instant::now();
        a.close_history();
        let _ = a.render();
        closes.push(t.elapsed().as_secs_f64() * 1e3);
        assert!(!a.ui_state.history.open, "关闭后面板不再打开");
    }
    assert_eq!(entries, 5000, "历史面板须读到 5000 份快照");
    opens.sort_by(|x, y| x.partial_cmp(y).unwrap());
    closes.sort_by(|x, y| x.partial_cmp(y).unwrap());
    samples.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let open_p50 = opens[SAMPLES / 2];
    let close_p50 = closes[SAMPLES / 2];
    let pick = |q: f64| {
        let i = ((samples.len() as f64 * q).ceil() as usize)
            .saturating_sub(1)
            .min(samples.len() - 1);
        samples[i]
    };

    println!(
        "perf_history_5000 open_ms_p50={open_p50:.3} scroll_p50={:.3} scroll_p95={:.3} close_ms_p50={close_p50:.3}",
        pick(0.5),
        pick(0.95)
    );
    println!("PROBE name=perf_history_5000 metric=entries value={entries} unit=count");
    println!("PROBE name=perf_history_5000 metric=open_ms value={open_p50:.3} unit=ms");
    println!(
        "PROBE name=perf_history_5000 metric=scroll_p50_ms value={:.3} unit=ms",
        pick(0.5)
    );
    println!(
        "PROBE name=perf_history_5000 metric=scroll_p95_ms value={:.3} unit=ms",
        pick(0.95)
    );
    println!("PROBE name=perf_history_5000 metric=close_ms value={close_p50:.3} unit=ms");
    // 回归上限（regression ceiling，不是目标值）：本机 9 轮实测 open_ms 9.3–10.9 ms、
    // scroll_p95 3.28–3.64 ms、close_ms 0.37–0.43 ms；打开/关闭取 3 次 p50，
    // 上限取实测最大值上方 ≥20% 再加抖动余量：open ≤25 ms、滚动 p95 ≤7 ms、close ≤1.2 ms。
    assert!(
        open_p50 <= 25.0,
        "历史面板打开 p50 {open_p50:.1}ms 超回归上限 25 ms（regression ceiling，非目标值）"
    );
    assert!(
        pick(0.95) <= 7.0,
        "历史面板滚动 p95 {:.3}ms 超回归上限 7 ms（regression ceiling，非目标值）",
        pick(0.95)
    );
    assert!(
        close_p50 <= 1.2,
        "历史面板关闭 p50 {close_p50:.3}ms 超回归上限 1.2 ms（regression ceiling，非目标值）"
    );
}
