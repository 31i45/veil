//! 字号比值与线宽语义的唯一来源（视觉契约 §3.2 / §四）。
//!
//! 标题以**纸面字号表**为唯一基准：纸面（PDF）全额使用 H1–H6 六档，
//! 屏幕（`view::layout::block_scale`）按级别上限夹取。两侧都从本模块派生，
//! 禁止再各写一套字面量。
//!
//! 线宽只做语义再导出，数值仍以 `tokens` 中的常量为唯一来源（此处不复制数字）。

/// 纸面正文字号（pt）：标题基准比值的除数。
pub const PAPER_BODY_PX: f32 = 11.0;

/// 纸面 H1–H6 字号（pt）：唯一基准表（契约 §3.2，冻结）。
pub const PAPER_HEADING_PX: [f32; 6] = [22.0, 18.0, 15.5, 14.0, 13.0, 12.5];

/// 屏幕 H1–H6 比值上限（契约 §3.2 的 cap 列，冻结）。
pub const SCREEN_HEADING_CAP: [f32; 6] = [1.6, 1.4, 1.2, 1.1, 1.1, 1.1];

/// 线宽 hairline（1.0）：分隔线、表格网格、边框、文字装饰线。
pub use crate::tokens::stroke::HAIRLINE as hairline;
/// 线宽 caret（2.0）：文本插入光标。
pub use crate::tokens::surface::CARET_WIDTH as caret;
/// 线宽 rail（3.0）：引用块左竖条、详情竖条。
pub use crate::tokens::doc::QUOTE_RAIL as rail;

/// 标题级别索引：1..=6 直接取；越界（含 0 与 7+）按 6 处理，
/// 与既有 `heading_size` 的 `_ => 12.5` 一致。
fn heading_index(level: u8) -> usize {
    match level {
        1..=6 => level as usize - 1,
        _ => 5,
    }
}

/// 纸面标题字号（pt）：H1–H6 = 22.0 / 18.0 / 15.5 / 14.0 / 13.0 / 12.5。
pub fn paper_heading_size(level: u8) -> f32 {
    PAPER_HEADING_PX[heading_index(level)]
}

/// 屏幕标题字号倍率（相对正文字号）：`min(纸面比值, 级别上限)`。
pub fn screen_heading_scale(level: u8) -> f32 {
    let idx = heading_index(level);
    (PAPER_HEADING_PX[idx] / PAPER_BODY_PX).min(SCREEN_HEADING_CAP[idx])
}

#[cfg(test)]
mod tests {
    use super::{paper_heading_size, screen_heading_scale, PAPER_BODY_PX, PAPER_HEADING_PX};
    use crate::tokens::typography::BODY_SIZE;

    /// 屏幕比值必须精确等于重构前的字面量（H1–H6）。
    #[test]
    fn screen_heading_scale_matches_legacy_literals() {
        let expected = [1.6f32, 1.4, 1.2, 1.1, 1.1, 1.1];
        for (i, want) in expected.iter().enumerate() {
            let level = i as u8 + 1;
            assert_eq!(
                screen_heading_scale(level),
                *want,
                "H{level} 屏幕比值必须保持 {want}"
            );
        }
    }

    /// 纸面字号必须精确等于重构前的字面量（H1–H6）。
    #[test]
    fn paper_heading_size_matches_legacy_literals() {
        let expected = [22.0f32, 18.0, 15.5, 14.0, 13.0, 12.5];
        assert_eq!(PAPER_HEADING_PX, expected);
        for (i, want) in expected.iter().enumerate() {
            let level = i as u8 + 1;
            assert_eq!(
                paper_heading_size(level),
                *want,
                "H{level} 纸面字号必须保持 {want}"
            );
        }
    }

    /// 越界级别按 6 处理（与既有 `_ => 12.5` 一致）。
    #[test]
    fn out_of_range_levels_fall_back_to_h6() {
        for level in [0u8, 7, 9, u8::MAX] {
            assert_eq!(paper_heading_size(level), paper_heading_size(6));
            assert_eq!(screen_heading_scale(level), screen_heading_scale(6));
        }
    }

    /// 屏幕实际像素（正文 16 × scale）与重构前逐级一致。
    #[test]
    fn screen_heading_pixels_match_legacy() {
        assert_eq!(BODY_SIZE, 16.0);
        assert_eq!(PAPER_BODY_PX, 11.0);
        let expected = [25.6f32, 22.4, 19.2, 17.6, 17.6, 17.6];
        for (i, want) in expected.iter().enumerate() {
            let level = i as u8 + 1;
            let px = BODY_SIZE * screen_heading_scale(level);
            assert_eq!(px, *want, "H{level} 屏幕像素必须保持 {want}");
        }
    }
}
