//! 版本历史面板（N1/B8a）：左侧快照列表 + 右侧只读预览。
//!
//! 纯逻辑零 GUI：面板/行/按钮几何、命中、滚动、尺寸格式化与空/错态文案都是
//! 纯函数，headless 可测；绘制在根级 paint，交互在 interaction。
//! 预览复用正文渲染器：打开预览时把实时编辑器暂存到本状态，正文区显示快照文本
//! 对应的只读 Editor（输入在 History 模式下被吞掉）。

use crate::app::SnapshotMeta;
use crate::editor::Editor;
use crate::geometry::Rect;
use crate::i18n::Key;
use crate::tokens::typography;

/// 面板基准宽（乘 DPI scale）：占左侧，正文区右移。
pub const PANEL_W: f32 = 300.0;
/// 标题区基准高。
pub const HEAD_H: f32 = 38.0;
/// 底部操作区基准高（恢复一行 + 删除/清空一行 + 提示行）。
pub const FOOT_H: f32 = 108.0;
/// 快照行基准高（时间 + 大小两行）。
pub const ROW_H: f32 = 46.0;
/// 内边距基准值：与全部浮层同一 surface 令牌。
pub const PAD: f32 = crate::theme::surface::PADDING;
/// 操作按钮基准高。
pub const BUTTON_H: f32 = 28.0;

/// 面板动作（按钮/键盘共用同一业务标识）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HistoryAction {
    Restore,
    Delete,
    Clear,
    Close,
}

impl HistoryAction {
    pub(crate) fn label(self) -> Key {
        match self {
            HistoryAction::Restore => Key::HistoryRestore,
            HistoryAction::Delete => Key::HistoryDelete,
            HistoryAction::Clear => Key::HistoryClear,
            HistoryAction::Close => Key::HistoryClose,
        }
    }
}

/// 面板状态：开关、快照元数据、选中/悬停/滚动与预览暂存的实时编辑器。
#[derive(Default)]
pub(crate) struct History {
    pub(crate) open: bool,
    /// 快照元数据（时间倒序，平台层已排序）。
    pub(crate) entries: Vec<SnapshotMeta>,
    /// 与 `entries` 同序的本地时间标签（由注入 Clock 生成）。
    pub(crate) labels: Vec<String>,
    pub(crate) selected: Option<usize>,
    pub(crate) hover: Option<usize>,
    pub(crate) hover_action: Option<HistoryAction>,
    pub(crate) scroll: f32,
    /// 读取失败态文本（None = 正常）。空列表且无错误显示空态文案。
    pub(crate) error: Option<String>,
    /// 打开面板时实时文档内容哈希：用于「当前」标记。
    pub(crate) current_hash: u64,
    /// 预览期间的实时编辑器（Some = 正文区显示的是快照，不是当前文档）。
    pub(crate) saved_editor: Option<Editor>,
    pub(crate) saved_scroll: f32,
    pub(crate) saved_goal: f32,
}

impl History {
    /// 关闭时清空列表/预览暂存（实时编辑器的归还由调用方完成）。
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.labels.clear();
        self.selected = None;
        self.hover = None;
        self.hover_action = None;
        self.scroll = 0.0;
        self.error = None;
        self.current_hash = 0;
    }

    pub(crate) fn current_index(&self) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.hash == self.current_hash)
    }

    /// 上下移动选中项并保证可见；预览由调用方在同一路径重取。
    pub(crate) fn step(&mut self, delta: i32) {
        if self.entries.is_empty() {
            self.selected = None;
            return;
        }
        let last = self.entries.len() as i32 - 1;
        let cur = self.selected.map_or(0, |i| i as i32);
        self.selected = Some((cur + delta).clamp(0, last) as usize);
    }
}

/// 面板矩形：贴左侧、覆盖内容视口高度。
pub(crate) fn panel_rect(vp: Rect, scale: f32) -> Rect {
    Rect {
        x: vp.x,
        y: vp.y,
        w: PANEL_W * scale,
        h: vp.h.max(0.0),
    }
}

/// 列表区顶端（标题区之下）。
pub(crate) fn rows_top(panel: Rect, scale: f32) -> f32 {
    panel.y + HEAD_H * scale
}

/// 底部操作区顶端。
pub(crate) fn footer_top(panel: Rect, scale: f32) -> f32 {
    panel.y + panel.h - FOOT_H * scale
}

/// 列表区可用高度。
pub(crate) fn rows_height(panel: Rect, scale: f32) -> f32 {
    (footer_top(panel, scale) - rows_top(panel, scale)).max(0.0)
}

/// 可见行槽数（至少 1 槽用于空/错态提示）。
pub(crate) fn visible_slots(panel: Rect, scale: f32) -> usize {
    ((rows_height(panel, scale) / (ROW_H * scale)).floor() as usize).max(1)
}

/// 最大滚动量（条目不溢出时为 0）。
pub(crate) fn max_scroll(panel: Rect, count: usize, scale: f32) -> f32 {
    (count as f32 * ROW_H * scale - rows_height(panel, scale)).max(0.0)
}

/// 让第 `index` 行完整可见所需的滚动量。
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

/// 首个可见槽在 `entries` 中的位置。
pub(crate) fn first_visible(scroll: f32, scale: f32) -> usize {
    (scroll / (ROW_H * scale)).floor().max(0.0) as usize
}

/// 单行矩形（面板内缩进）。
pub(crate) fn row_rect(panel: Rect, index: usize, scroll: f32, scale: f32) -> Rect {
    let row_h = ROW_H * scale;
    Rect {
        x: panel.x + PAD * scale,
        y: rows_top(panel, scale) + index as f32 * row_h - scroll,
        w: (panel.w - 2.0 * PAD * scale).max(0.0),
        h: row_h,
    }
}

/// 操作按钮几何：恢复整行；删除/清空并排；关闭在标题区右侧。
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Layout {
    pub(crate) panel: Rect,
    pub(crate) close: Rect,
    pub(crate) restore: Rect,
    pub(crate) delete: Rect,
    pub(crate) clear: Rect,
    pub(crate) footer_note_y: f32,
}

pub(crate) fn layout(vp: Rect, scale: f32) -> Layout {
    let panel = panel_rect(vp, scale);
    let pad = PAD * scale;
    let bh = BUTTON_H * scale;
    let gap = crate::theme::surface::control_gap(scale);
    let footer = footer_top(panel, scale);
    let inner_w = (panel.w - 2.0 * pad).max(0.0);
    let half = ((inner_w - gap) / 2.0).max(0.0);
    let close_w = 58.0 * scale;
    let close_h = 24.0 * scale;
    Layout {
        panel,
        close: Rect {
            x: panel.x + panel.w - pad - close_w,
            y: panel.y + (HEAD_H * scale - close_h) / 2.0,
            w: close_w,
            h: close_h,
        },
        restore: Rect {
            x: panel.x + pad,
            y: footer + 8.0 * scale,
            w: inner_w,
            h: bh,
        },
        delete: Rect {
            x: panel.x + pad,
            y: footer + 8.0 * scale + bh + gap,
            w: half,
            h: bh,
        },
        clear: Rect {
            x: panel.x + pad + half + gap,
            y: footer + 8.0 * scale + bh + gap,
            w: half,
            h: bh,
        },
        footer_note_y: footer + 8.0 * scale + 2.0 * bh + gap + 8.0 * scale,
    }
}

/// 命中结果。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Hit {
    Row(usize),
    Action(HistoryAction),
    /// 面板内空白：吞掉点击，不落正文光标。
    Background,
    Outside,
}

pub(crate) fn hit(l: &Layout, top: usize, count: usize, x: f32, y: f32, scale: f32) -> Hit {
    if l.close.contains(x, y) {
        return Hit::Action(HistoryAction::Close);
    }
    if l.restore.contains(x, y) {
        return Hit::Action(HistoryAction::Restore);
    }
    if l.delete.contains(x, y) {
        return Hit::Action(HistoryAction::Delete);
    }
    if l.clear.contains(x, y) {
        return Hit::Action(HistoryAction::Clear);
    }
    if !l.panel.contains(x, y) {
        return Hit::Outside;
    }
    let row_h = ROW_H * scale;
    let top_y = rows_top(l.panel, scale);
    if y >= top_y && y < footer_top(l.panel, scale) {
        let slot = ((y - top_y) / row_h).floor() as usize;
        let index = top + slot;
        if index < count {
            return Hit::Row(index);
        }
    }
    Hit::Background
}

/// 字节数的人类可读文本（列表第二行）。
pub(crate) fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{:.1} MB", b / MB)
    }
}

/// 近似文本宽度：CJK 全角按 UI 字号、ASCII 按 0.6 字号（只服务截断/布局）。
pub(crate) fn text_width(text: &str, scale: f32) -> f32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                typography::UI_SIZE * 0.6 * scale
            } else {
                typography::UI_SIZE * scale
            }
        })
        .sum()
}

/// 按可用宽度截断（超出补尾部 `…`）。
pub(crate) fn fit_label(text: &str, max_w: f32, scale: f32) -> String {
    if text_width(text, scale) <= max_w {
        return text.to_string();
    }
    let avail = (max_w - typography::UI_SIZE * scale).max(0.0);
    let mut out = String::new();
    let mut w = 0.0;
    for c in text.chars() {
        let cw = if c.is_ascii() {
            typography::UI_SIZE * 0.6 * scale
        } else {
            typography::UI_SIZE * scale
        };
        if w + cw > avail {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.push('…');
    out
}

/// 空态文案（无任何快照）。
pub(crate) const EMPTY_TEXT: Key = Key::HistoryEmpty;
/// 空态补充说明：让用户知道历史从何而来。
pub(crate) const EMPTY_HINT: Key = Key::HistoryEmptyHint;

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Rect {
        Rect {
            x: 0.0,
            y: 30.0,
            w: 1000.0,
            h: 700.0,
        }
    }

    #[test]
    fn panel_is_left_side_and_buttons_inside() {
        let l = layout(vp(), 1.0);
        assert_eq!(l.panel.x, 0.0);
        assert_eq!(l.panel.y, 30.0);
        assert_eq!(l.panel.w, PANEL_W);
        for r in [l.close, l.restore, l.delete, l.clear] {
            assert!(r.x >= l.panel.x && r.x + r.w <= l.panel.x + l.panel.w + 0.01);
            assert!(r.y >= l.panel.y && r.y + r.h <= l.panel.y + l.panel.h + 0.01);
        }
        assert!(l.delete.w > 0.0 && l.clear.w > 0.0);
    }

    #[test]
    fn hit_maps_rows_and_actions() {
        let l = layout(vp(), 1.0);
        let scale = 1.0;
        let first = row_rect(l.panel, 0, 0.0, scale);
        assert_eq!(
            hit(&l, 0, 5, first.x + 2.0, first.y + 2.0, scale),
            Hit::Row(0)
        );
        assert_eq!(
            hit(&l, 2, 5, first.x + 2.0, first.y + 2.0, scale),
            Hit::Row(2)
        );
        assert_eq!(
            hit(&l, 0, 0, first.x + 2.0, first.y + 2.0, scale),
            Hit::Background
        );
        assert_eq!(
            hit(&l, 0, 5, l.restore.x + 2.0, l.restore.y + 2.0, scale),
            Hit::Action(HistoryAction::Restore)
        );
        assert_eq!(
            hit(&l, 0, 5, l.close.x + 2.0, l.close.y + 2.0, scale),
            Hit::Action(HistoryAction::Close)
        );
        assert_eq!(hit(&l, 0, 5, 900.0, 400.0, scale), Hit::Outside);
    }

    #[test]
    fn scroll_helpers_clamp_and_keep_selection_visible() {
        let l = layout(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 900.0,
                h: 400.0,
            },
            1.0,
        );
        let count = 50;
        let max = max_scroll(l.panel, count, 1.0);
        assert!(max > 0.0);
        let s = ensure_visible(0.0, 40, count, l.panel, 1.0);
        assert!(s > 0.0 && s <= max);
        assert_eq!(ensure_visible(0.0, 0, count, l.panel, 1.0), 0.0);
        assert!(first_visible(s, 1.0) > 0);
    }

    #[test]
    fn format_size_uses_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(2 * 1024 * 1024), "2.0 MB");
    }

    #[test]
    fn fit_label_truncates_with_ellipsis() {
        assert_eq!(fit_label("短", 200.0, 1.0), "短");
        let long = "2026-11-12 20:31:05 一个非常非常非常长的标签";
        let out = fit_label(long, 60.0, 1.0);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() < long.chars().count());
    }

    #[test]
    fn current_index_matches_hash() {
        let mut h = History {
            entries: vec![
                SnapshotMeta {
                    ts_ms: 2,
                    hash: 22,
                    bytes: 2,
                },
                SnapshotMeta {
                    ts_ms: 1,
                    hash: 11,
                    bytes: 1,
                },
            ],
            current_hash: 11,
            ..Default::default()
        };
        assert_eq!(h.current_index(), Some(1));
        h.current_hash = 99;
        assert_eq!(h.current_index(), None);
    }

    #[test]
    fn step_clamps_to_bounds() {
        let mut h = History {
            entries: vec![
                SnapshotMeta {
                    ts_ms: 3,
                    hash: 3,
                    bytes: 3,
                },
                SnapshotMeta {
                    ts_ms: 2,
                    hash: 2,
                    bytes: 2,
                },
            ],
            ..Default::default()
        };
        h.step(-1);
        assert_eq!(h.selected, Some(0));
        h.step(1);
        assert_eq!(h.selected, Some(1));
        h.step(1);
        assert_eq!(h.selected, Some(1));
    }
}
