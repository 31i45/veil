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
    let t = Instant::now();
    let mut a = app(&text, None, false, &store);
    a.paint();
    let open_ms = t.elapsed().as_secs_f64() * 1e3;

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

    println!("perf_5000_headings open_ms={open_ms:.3} headings_us={headings_us:.1}");
    println!("PROBE name=perf_5000_headings metric=headings value={headings} unit=count");
    println!("PROBE name=perf_5000_headings metric=open_ms value={open_ms:.3} unit=ms");
    println!("PROBE name=perf_5000_headings metric=headings_us value={headings_us:.1} unit=us");
    println!(
        "PROBE name=perf_5000_headings metric=scroll_p50_ms value={:.3} unit=ms",
        pick(0.5)
    );
    println!(
        "PROBE name=perf_5000_headings metric=scroll_p95_ms value={:.3} unit=ms",
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

    let t = Instant::now();
    a.open_history();
    let _ = a.render();
    let open_ms = t.elapsed().as_secs_f64() * 1e3;
    let entries = a.ui_state.history.entries.len();
    assert_eq!(entries, 5000, "历史面板须读到 5000 份快照");

    let l = a.history_layout();
    a.input_state.mouse =
        PhysicalPosition::new((l.panel.x + 40.0) as f64, (l.panel.y + 200.0) as f64);
    let mut samples = Vec::with_capacity(300);
    for _ in 0..300 {
        let t = Instant::now();
        a.on_wheel(-60.0);
        let _ = a.render();
        samples.push(t.elapsed().as_secs_f64() * 1e3);
    }
    assert!(a.ui_state.history.scroll > 0.0, "面板须实际滚动");
    samples.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let pick = |q: f64| {
        let i = ((samples.len() as f64 * q).ceil() as usize)
            .saturating_sub(1)
            .min(samples.len() - 1);
        samples[i]
    };

    let t = Instant::now();
    a.close_history();
    let _ = a.render();
    let close_ms = t.elapsed().as_secs_f64() * 1e3;
    assert!(!a.ui_state.history.open, "关闭后面板不再打开");

    println!(
        "perf_history_5000 open_ms={open_ms:.3} scroll_p50={:.3} scroll_p95={:.3} close_ms={close_ms:.3}",
        pick(0.5),
        pick(0.95)
    );
    println!("PROBE name=perf_history_5000 metric=entries value={entries} unit=count");
    println!("PROBE name=perf_history_5000 metric=open_ms value={open_ms:.3} unit=ms");
    println!(
        "PROBE name=perf_history_5000 metric=scroll_p50_ms value={:.3} unit=ms",
        pick(0.5)
    );
    println!(
        "PROBE name=perf_history_5000 metric=scroll_p95_ms value={:.3} unit=ms",
        pick(0.95)
    );
    println!("PROBE name=perf_history_5000 metric=close_ms value={close_ms:.3} unit=ms");
}
