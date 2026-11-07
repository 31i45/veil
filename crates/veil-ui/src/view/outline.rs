//! 大纲面板（P3-2）：标题条目的派生状态与面板几何。
//!
//! 纯逻辑零 GUI：条目由 App 从块索引 + `Session::heading` 派生，本文件只承载
//! 状态容器与「画在哪 / 点到谁」的纯函数；绘制在根级 paint，交互在 interaction。
//! 面板贴右侧、滚动条左侧：正文 x 原点不变，仅版心宽减去面板宽。

use veil_core::BlockId;

use crate::geometry::BASE_FONT_SIZE;

/// 面板基准宽（乘 DPI scale）。
pub const PANEL_W: f32 = 240.0;
/// 面板标题区基准高。
pub const HEAD_H: f32 = 34.0;
/// 条目行高基准值：与全部浮层同一 surface 令牌。
pub const ROW_H: f32 = crate::theme::surface::ROW_HEIGHT;
/// 面板左右内边距基准值：与全部浮层同一 surface 令牌。
pub const PAD: f32 = crate::theme::surface::PADDING;
/// 标题层级缩进步长基准值。
pub const INDENT_STEP: f32 = 14.0;

/// 标题条目：级别 + 可见文本 + 源位置（点击跳转）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct OutlineEntry {
    pub(crate) level: u8,
    pub(crate) text: String,
    pub(crate) block: BlockId,
    /// 标题块起点（绝对字节偏移）：跳转后光标落此。
    pub(crate) start: usize,
}

/// 大纲面板状态：开关、滚动、悬停与条目。
#[derive(Default)]
pub(crate) struct Outline {
    pub(crate) open: bool,
    pub(crate) scroll: f32,
    pub(crate) hover: Option<usize>,
    pub(crate) entries: Vec<OutlineEntry>,
    /// 上次自动跟随的光标位置：仅光标变化时把当前条目滚入视野，
    /// 避免用户手动滚动大纲时被每帧强制拉回。打开面板时置 None 以重新同步。
    pub(crate) last_caret: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Rect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

/// 单行命中矩形。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Row {
    pub(crate) index: usize,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

/// 面板矩形（卡片式）：右缘距滚动条、上下距窗口边缘均为 surface 的 PANEL_MARGIN；
/// 左侧与版心相邻（投影提供分隔）。命中与绘制共用本矩形。
pub(crate) fn panel_rect(right: f32, top: f32, height: f32, scale: f32) -> Rect {
    let w = PANEL_W * scale;
    let m = crate::theme::surface::panel_margin(scale);
    Rect {
        x: right - w,
        y: top + m,
        w: w - m,
        h: (height - 2.0 * m).max(0.0),
    }
}

/// 条目区顶端（面板标题区之下）。
pub(crate) fn rows_top(panel: Rect, scale: f32) -> f32 {
    panel.y + HEAD_H * scale
}

/// 条目区可用高度。
pub(crate) fn rows_height(panel: Rect, scale: f32) -> f32 {
    (panel.h - HEAD_H * scale).max(0.0)
}

/// 面板最大滚动量（条目不溢出时为 0）。
pub(crate) fn max_scroll(panel: Rect, count: usize, scale: f32) -> f32 {
    (count as f32 * ROW_H * scale - rows_height(panel, scale)).max(0.0)
}

/// 当前光标所在条目：最后一个起点不晚于 `caret` 的标题下标。
/// 光标在首个标题之前（或没有标题）返回 `None`。
pub(crate) fn current_index(entries: &[OutlineEntry], caret: usize) -> Option<usize> {
    entries.partition_point(|e| e.start <= caret).checked_sub(1)
}

/// 让第 `index` 行完整可见所需的滚动量：行在条目区上方则上滚、下方则下滚；
/// 结果钳制到 `[0, max_scroll]`，条目不溢出时恒为 0。
pub(crate) fn ensure_visible(
    scroll: f32,
    index: usize,
    count: usize,
    panel: Rect,
    scale: f32,
) -> f32 {
    let row_h = ROW_H * scale;
    let view_h = rows_height(panel, scale);
    let top = index as f32 * row_h;
    let bottom = top + row_h;
    let mut out = scroll;
    if top < scroll {
        out = top;
    } else if bottom > scroll + view_h {
        out = bottom - view_h;
    }
    out.clamp(0.0, max_scroll(panel, count, scale))
}

/// 可见条目行（含与条目区相交的部分行；调用方按需裁剪绘制）。
pub(crate) fn visible_rows(panel: Rect, count: usize, scroll: f32, scale: f32) -> Vec<Row> {
    let row_h = ROW_H * scale;
    let top = rows_top(panel, scale);
    let bottom = panel.y + panel.h;
    let x = panel.x + PAD * scale;
    let w = (panel.w - 2.0 * PAD * scale).max(0.0);
    (0..count)
        .filter_map(|i| {
            let y = top + i as f32 * row_h - scroll;
            (y + row_h > top && y < bottom).then_some(Row {
                index: i,
                x,
                y,
                w,
                h: row_h,
            })
        })
        .collect()
}

/// 条目区命中：落在面板条目区内才有效（标题区与面板外恒 None）。
pub(crate) fn hit_row(
    panel: Rect,
    count: usize,
    scroll: f32,
    scale: f32,
    x: f32,
    y: f32,
) -> Option<usize> {
    if x < panel.x || x >= panel.x + panel.w {
        return None;
    }
    let top = rows_top(panel, scale);
    if y < top || y >= panel.y + panel.h {
        return None;
    }
    let i = ((y - top + scroll) / (ROW_H * scale)).floor();
    if i < 0.0 {
        return None;
    }
    let i = i as usize;
    (i < count).then_some(i)
}

/// 条目文字按可用宽度截断（超出补 `…`）；CJK 按全角、ASCII 按半角估算。
pub(crate) fn fit_label(text: &str, max_w: f32, scale: f32) -> String {
    let width_of = |c: char| {
        if c.is_ascii() {
            BASE_FONT_SIZE * 0.55 * scale
        } else {
            BASE_FONT_SIZE * scale
        }
    };
    if text.chars().map(width_of).sum::<f32>() <= max_w {
        return text.to_string();
    }
    let avail = max_w - width_of('…');
    let mut out = String::new();
    let mut w = 0.0;
    for c in text.chars() {
        let cw = width_of(c);
        if w + cw > avail {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> Rect {
        panel_rect(800.0, 30.0, 600.0, 1.0)
    }

    #[test]
    fn panel_rect_inset_card_margins() {
        let m = crate::theme::surface::panel_margin(1.0);
        let p = panel();
        assert_eq!(p.x, 800.0 - PANEL_W);
        assert_eq!(p.x + p.w, 800.0 - m);
        assert_eq!(p.y, 30.0 + m);
        assert_eq!(p.h, 600.0 - 2.0 * m);
        // 标题区之下才是条目区
        assert_eq!(rows_top(p, 1.0), 30.0 + m + HEAD_H);
        assert_eq!(rows_height(p, 1.0), 600.0 - 2.0 * m - HEAD_H);
    }

    #[test]
    fn scroll_only_when_entries_overflow() {
        let p = panel();
        assert_eq!(max_scroll(p, 1, 1.0), 0.0);
        let rows_h = rows_height(p, 1.0);
        let fits = (rows_h / ROW_H).floor() as usize;
        assert_eq!(max_scroll(p, fits, 1.0), 0.0);
        assert!(max_scroll(p, fits + 1, 1.0) > 0.0);
    }

    #[test]
    fn visible_rows_skip_scrolled_out_and_report_geometry() {
        let p = panel();
        let rows = visible_rows(p, 100, 0.0, 1.0);
        assert_eq!(rows[0].index, 0);
        assert_eq!(rows[0].y, rows_top(p, 1.0));
        assert!((rows[1].y - rows[0].y - ROW_H).abs() < 0.01);
        assert_eq!(rows[0].x, p.x + PAD);
        assert_eq!(rows[0].w, p.w - 2.0 * PAD);
        // 滚动一屏后首行下标前进，且不再包含第 0 行
        let scrolled = visible_rows(p, 100, 4.0 * ROW_H, 1.0);
        assert_eq!(scrolled[0].index, 4);
        assert!(scrolled.iter().all(|r| r.index > 0));
    }

    #[test]
    fn hit_row_matches_visible_row_centers_and_rejects_header() {
        let p = panel();
        let rows = visible_rows(p, 10, 0.0, 1.0);
        for r in &rows {
            assert_eq!(
                hit_row(p, 10, 0.0, 1.0, r.x + 1.0, r.y + r.h / 2.0),
                Some(r.index)
            );
        }
        // 面板标题区、面板外、行数之外均不命中
        assert_eq!(hit_row(p, 10, 0.0, 1.0, p.x + 1.0, p.y + 1.0), None);
        assert_eq!(hit_row(p, 10, 0.0, 1.0, p.x - 1.0, rows[0].y), None);
        assert_eq!(hit_row(p, 10, 0.0, 1.0, p.x + 1.0, p.y + p.h - 1.0), None);
        let far = hit_row(p, 1000, 0.0, 1.0, p.x + 1.0, p.y + p.h - 1.0);
        assert!(far.is_some_and(|i| i < 1000));
    }

    fn sample_entries() -> Vec<OutlineEntry> {
        [10usize, 40, 100]
            .into_iter()
            .enumerate()
            .map(|(i, start)| OutlineEntry {
                level: 1,
                text: format!("h{i}"),
                block: BlockId(i as u64),
                start,
            })
            .collect()
    }

    #[test]
    fn current_index_covers_before_exact_between_after() {
        let e = sample_entries();
        // 首个标题之前（含无标题）
        assert_eq!(current_index(&e, 0), None);
        assert_eq!(current_index(&e, 9), None);
        assert_eq!(current_index(&[], 5), None);
        // 恰好落在标题起点
        assert_eq!(current_index(&e, 10), Some(0));
        assert_eq!(current_index(&e, 40), Some(1));
        assert_eq!(current_index(&e, 100), Some(2));
        // 两个标题之间（取前一个）
        assert_eq!(current_index(&e, 25), Some(0));
        assert_eq!(current_index(&e, 99), Some(1));
        // 末个标题之后
        assert_eq!(current_index(&e, 1000), Some(2));
    }

    #[test]
    fn ensure_visible_keeps_visible_row_and_scrolls_both_ways() {
        let p = panel();
        let rows_h = rows_height(p, 1.0);
        let count = 100;
        // 已完整可见：滚动量不变
        assert!((ensure_visible(100.0, 10, count, p, 1.0) - 100.0).abs() < 0.01);
        // 下方越界：下滚到行底贴住条目区底
        let down = ensure_visible(0.0, 25, count, p, 1.0);
        assert!((down - (26.0 * ROW_H - rows_h)).abs() < 0.01);
        // 上方越界：上滚到行顶贴住条目区顶
        let up = ensure_visible(300.0, 2, count, p, 1.0);
        assert!((up - 2.0 * ROW_H).abs() < 0.01);
        // 溢出钳制：末行贴底、超量滚动回落到 max_scroll
        let max = max_scroll(p, count, 1.0);
        assert!((ensure_visible(0.0, count - 1, count, p, 1.0) - max).abs() < 0.01);
        assert!((ensure_visible(9999.0, count - 1, count, p, 1.0) - max).abs() < 0.01);
        // 不溢出：恒为 0
        assert_eq!(ensure_visible(0.0, 9, 10, p, 1.0), 0.0);
    }

    #[test]
    fn fit_label_truncates_with_ellipsis() {
        assert_eq!(fit_label("短标题", 200.0, 1.0), "短标题");
        let long = "一个很长很长很长很长很长很长的标题";
        let out = fit_label(long, 60.0, 1.0);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() < long.chars().count());
        // 可用宽度不足以放省略号时仍返回省略号（不 panic、不越界）
        assert_eq!(fit_label(long, 1.0, 1.0), "…");
    }
}
