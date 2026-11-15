//! 打印预览覆盖层：与 PDF 导出同源分页的整页位图（由产品层离屏渲染），
//! 支持翻页与缩放。纯视图态：不改文档、不写历史、不进撤销栈。

use crate::app::App;
use crate::i18n::{t, tf, Key};
use crate::view::Frame;

/// 覆盖整窗绘制当前页：压暗背景 + 居中页面 + 顶部页码/缩放 + 底部操作提示。
pub(crate) fn paint(app: &mut App, frame: &mut Frame) {
    let (win_w, win_h) = app.win_size();
    let scale = app.input_state.scale;
    // 背景压暗（含菜单栏/状态栏）：预览期间它是唯一内容。
    frame.fill_rect(0, 0, win_w, win_h, [0, 0, 0, 200]);
    let Some(pv) = app.ui_state.print_preview.as_ref() else {
        return;
    };
    let total = pv.pages.len();
    if total == 0 {
        return;
    }
    let index = pv.index.min(total - 1);
    let page = &pv.pages[index];
    let pad = (28.0 * scale).max(8.0);
    let fit = ((win_w as f32 - 2.0 * pad) / page.width.max(1) as f32)
        .min((win_h as f32 - 2.0 * pad) / page.height.max(1) as f32)
        .max(0.05);
    let z = if pv.zoom <= 0.0 { 1.0 } else { pv.zoom };
    let pw = (page.width as f32 * fit * z).round().max(1.0);
    let ph = (page.height as f32 * fit * z).round().max(1.0);
    let px = ((win_w as f32 - pw) * 0.5).round() as i32;
    let py = ((win_h as f32 - ph) * 0.5).round() as i32;
    // 页影 + 纸面：缩放后的页面按最近邻贴图（页面本身已是最终版面）。
    frame.fill_rect(px - 3, py - 3, pw as u32 + 6, ph as u32 + 6, [0, 0, 0, 120]);
    frame.fill_rect(px, py, pw as u32, ph as u32, [255, 255, 255, 255]);
    frame.blit_scaled(
        &page.pixels,
        page.width,
        page.height,
        pw as u32,
        ph as u32,
        px,
        py,
    );
    // 顶部：页码 + 缩放百分比；底部：按键提示。
    let pct = format!("{}%", (z * 100.0).round() as i32);
    let title = tf(
        Key::PrintPreviewTitle,
        &[&(index + 1).to_string(), &total.to_string(), &pct],
    );
    let hint = t(Key::PrintPreviewHint).to_string();
    let (labels, engine) = (&mut app.ui_state.chrome.labels, &mut app.engine);
    if let Some(lab) = labels.get_or_insert(engine, &title, [240, 240, 240], scale) {
        frame.blit(&lab.pixels, lab.w, lab.h, pad as i32, (pad * 0.4) as i32);
    }
    if let Some(lab) = labels.get_or_insert(engine, &hint, [170, 170, 170], scale) {
        let x = ((win_w as f32 - lab.w as f32) * 0.5).max(0.0) as i32;
        let y = (win_h as f32 - pad * 0.4 - lab.h as f32).round() as i32;
        frame.blit(&lab.pixels, lab.w, lab.h, x, y);
    }
}
