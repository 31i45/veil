//! 图片放大预览覆盖层：压暗背景 + 居中缩放位图 + 标题/操作提示。
//! 纯视图态：不写文档、不动光标、不进撤销栈；位图直接取自行内图片缓存。

use crate::app::App;
use crate::geometry::Rect;
use crate::i18n::{t, tf, Key};
use crate::view::Frame;

/// 预览图片在窗口内的屏幕矩形（无预览或位图不可用时 None）。
/// 绘制与「点击遮罩关闭」的命中判定共用同一几何，避免两处漂移。
pub(crate) fn image_rect(app: &App) -> Option<Rect> {
    let pv = app.ui_state.image_preview.as_ref()?;
    let img = app
        .cache
        .image_or_remote(&pv.key, app.input_state.revs.image_base)?;
    let (win_w, win_h) = app.win_size();
    let scale = app.input_state.scale;
    let pad = (40.0 * scale).max(8.0);
    let w = img.width.max(1) as f32;
    let h = img.height.max(1) as f32;
    // 1.0 = 适应窗口（留出标题/提示的边距）；作者缩放以 fit 为基准。
    let fit = ((win_w as f32 - 2.0 * pad) / w)
        .min((win_h as f32 - 2.0 * pad) / h)
        .max(0.01);
    let z = if pv.zoom <= 0.0 { 1.0 } else { pv.zoom };
    let dw = (w * fit * z).round().max(1.0);
    let dh = (h * fit * z).round().max(1.0);
    let cx = win_w as f32 * 0.5 + pv.pan.0;
    let cy = win_h as f32 * 0.5 + pv.pan.1;
    Some(Rect {
        x: (cx - dw * 0.5).round(),
        y: (cy - dh * 0.5).round(),
        w: dw,
        h: dh,
    })
}

/// 位图缺失时的提示色：缺失不是错误操作，取中性空态色（`state::empty_fg`），
/// 不使用危险色；绘制与测试共用同一取色点。
pub(crate) fn unavailable_hint_color(theme: &crate::theme::Theme) -> [u8; 3] {
    crate::tokens::state::empty_fg(theme)
}

/// 覆盖整窗绘制：背景压暗 + 图影 + 位图 + 顶部标题/缩放 + 底部操作提示。
pub(crate) fn paint(app: &mut App, frame: &mut Frame) {
    let (win_w, win_h) = app.win_size();
    let scale = app.input_state.scale;
    let theme = app.input_state.theme;
    frame.fill_rect(0, 0, win_w, win_h, [0, 0, 0, 220]);
    let Some(r) = image_rect(app) else {
        // 位图缺失：保留遮罩，居中显示中性提示（Esc/点击遮罩仍可关闭）。
        let message = t(Key::ImagePreviewUnavailable).to_string();
        let fg = unavailable_hint_color(&theme);
        if let Some(lab) = app
            .ui_state
            .chrome
            .labels
            .get_or_insert(&mut app.engine, &message, fg, scale)
        {
            let x = ((win_w as f32 - lab.w as f32) * 0.5).max(0.0) as i32;
            let y = ((win_h as f32 - lab.h as f32) * 0.5).max(0.0) as i32;
            frame.blit(&lab.pixels, lab.w, lab.h, x, y);
        }
        return;
    };
    let Some(pv) = app.ui_state.image_preview.as_ref() else {
        return;
    };
    let (key, title, z) = (pv.key.clone(), pv.title.clone(), pv.zoom.max(1.0));
    {
        let Some(img) = app
            .cache
            .image_or_remote(&key, app.input_state.revs.image_base)
        else {
            return;
        };
        frame.fill_rect(
            r.x as i32 - 3,
            r.y as i32 - 3,
            r.w as u32 + 6,
            r.h as u32 + 6,
            [0, 0, 0, 160],
        );
        frame.blit_scaled(
            &img.pixels,
            img.width,
            img.height,
            r.w as u32,
            r.h as u32,
            r.x as i32,
            r.y as i32,
        );
    }
    let pad = (28.0 * scale).max(8.0);
    let pct = format!("{}%", (z * 100.0).round() as i32);
    let title = tf(Key::ImagePreviewTitle, &[&title, &pct]);
    let hint = t(Key::ImagePreviewHint).to_string();
    let (labels, engine) = (&mut app.ui_state.chrome.labels, &mut app.engine);
    if let Some(lab) = labels.get_or_insert(engine, &title, [240, 240, 240], scale) {
        let x = ((win_w as f32 - lab.w as f32) * 0.5).max(0.0) as i32;
        frame.blit(&lab.pixels, lab.w, lab.h, x, (pad * 0.4) as i32);
    }
    if let Some(lab) = labels.get_or_insert(engine, &hint, [170, 170, 170], scale) {
        let x = ((win_w as f32 - lab.w as f32) * 0.5).max(0.0) as i32;
        let y = (win_h as f32 - pad * 0.4 - lab.h as f32).round() as i32;
        frame.blit(&lab.pixels, lab.w, lab.h, x, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    /// 位图缺失提示必须存在且取中性色（缺失不是错误操作，不得用危险色）。
    #[test]
    fn missing_bitmap_hint_is_neutral_and_localized() {
        for theme in [Theme::light(), Theme::dark(), Theme::high_contrast()] {
            assert_eq!(
                unavailable_hint_color(&theme),
                theme.muted,
                "缺失提示取中性空态色"
            );
            assert_ne!(
                unavailable_hint_color(&theme),
                theme.danger,
                "缺失不是错误操作，不得使用危险色"
            );
        }
        assert_eq!(t(Key::ImagePreviewUnavailable), "图片无法显示");
    }
}
