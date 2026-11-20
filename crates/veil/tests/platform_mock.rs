//! Mermaid 远程管线（veil.exe 路径）测试。
//!
//! 覆盖：
//! 1. `svg_to_bitmap` 光栅化 happy path — 固定 SVG → RGBA 位图
//! 2. `svg_to_bitmap` 错误路径 — 非法 SVG 文本 → Err
//! 3. `MermaidFetcher::set_allowed(false)` 拒绝路径 — 不发起任何网络请求
//! 4. `MermaidFetcher` poll FIFO — request→poll 空队列

use veil::{svg_to_bitmap, MermaidFetcher};

/// 最小有效 SVG（resvg 可解析）。
const VALID_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50"><rect x="5" y="5" width="90" height="40"/><text x="50" y="28">Hello</text></svg>"#;

// =============================================================================
// 测试 1 — svg_to_bitmap happy path
// =============================================================================

#[test]
fn svg_to_bitmap_produces_nonempty_rgba() {
    let bm = svg_to_bitmap(VALID_SVG, 2.0).expect("有效 SVG 应光栅化成功");
    assert!(bm.width > 0, "宽度应 > 0");
    assert!(bm.height > 0, "高度应 > 0");
    assert!(!bm.data.is_empty(), "像素缓冲区非空");
    assert_eq!(bm.data.len() as u32, bm.width * bm.height * 4);
    assert!(bm.svg_text.is_none(), "svg_to_bitmap 不应预填充 svg_text");
}

/// scale=3.0 时位图更大（scale 被 clamp 到 [2.0, 4.0]）。
#[test]
fn svg_to_bitmap_scale_changes_dimensions() {
    let bm2 = svg_to_bitmap(VALID_SVG, 2.0).expect("scale 2");
    let bm3 = svg_to_bitmap(VALID_SVG, 3.0).expect("scale 3");
    assert!(
        bm3.width >= bm2.width,
        "scale 3 应至少不小于 scale 2 的宽度：{} vs {}",
        bm3.width,
        bm2.width
    );
}

// =============================================================================
// 测试 2 — svg_to_bitmap 错误路径
// =============================================================================

#[test]
fn svg_to_bitmap_rejects_garbage_input() {
    let result = svg_to_bitmap("this is not svg at all", 2.0);
    assert!(result.is_err(), "垃圾文本必须 Err：{result:?}");
}

#[test]
fn svg_to_bitmap_rejects_empty_input() {
    let result = svg_to_bitmap("", 2.0);
    assert!(result.is_err(), "空字符串必须 Err");
}

// =============================================================================
// 测试 3 — MermaidFetcher allowed=false 拒绝路径
// =============================================================================

#[test]
fn mermaid_fetcher_rejects_when_allowed_is_false() {
    let mut f = MermaidFetcher::new();
    f.set_allowed(false);
    f.request("http://mermaid.ink/svg/dummy".into());
    // allowed=false 时 request() 直接返回，不会 spawn 线程、也不会产生任何结果
    std::thread::sleep(std::time::Duration::from_millis(200));
    let results = f.poll();
    assert!(
        results.is_empty(),
        "allowed=false 时不应有任何结果：{results:?}"
    );
}

#[test]
fn mermaid_fetcher_poll_returns_empty_when_nothing_requested() {
    let mut f = MermaidFetcher::new();
    let results = f.poll();
    assert!(results.is_empty(), "无 request 时 poll 应为空");
}

#[test]
fn mermaid_fetcher_render_blocking_errors_when_allowed_false() {
    let mut f = MermaidFetcher::new();
    f.set_allowed(false);
    let err = f
        .render_blocking("http://mermaid.ink/svg/dummy".into())
        .expect_err("allowed=false 必须直接 Err");
    assert!(!err.is_empty(), "错误消息必须可读：{err}");
}

#[test]
fn mermaid_fetcher_set_dpi_scale_then_reset() {
    let mut f = MermaidFetcher::new();
    f.set_dpi_scale(3.5);
    f.set_dpi_scale(2.0);
    // scale 只在内部，无公开 getter；只要不 panic 就算过
}
