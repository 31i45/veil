//! 右键上下文菜单（2f）：选区/链接/块三类上下文的项集、几何与命中。
//! 纯模型与几何：不绘制（paint.rs 消费布局）、不执行（interaction.rs 分发动作）。

use crate::chrome;
use crate::geometry::BASE_FONT_SIZE;

/// 上下文菜单动作：执行映射在 interaction.rs（复用既有剪贴板/插入路径）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CtxAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
    /// 复制命中链接的 URL（inline 链接才有；引用式无 url 区间不出现本项）。
    CopyLink,
    InsertImage,
}

/// 打开时刻定的上下文类别：决定项集，打开期间不随编辑变化（菜单是瞬态）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CtxKind {
    /// 右键落在既有选区内（选区保留不动）。
    Selection,
    /// 光标落在 inline 链接节点内（含标记区）。
    Link,
    /// 其余正文位置。
    Block,
}

/// 各类上下文的项集（单一来源：布局/命中/分发共用）。
pub(crate) fn items_for(kind: CtxKind) -> Vec<(&'static str, CtxAction)> {
    match kind {
        CtxKind::Selection => vec![
            ("剪切", CtxAction::Cut),
            ("复制", CtxAction::Copy),
            ("粘贴", CtxAction::Paste),
            ("全选", CtxAction::SelectAll),
        ],
        CtxKind::Link => vec![
            ("复制链接地址", CtxAction::CopyLink),
            ("复制", CtxAction::Copy),
            ("粘贴", CtxAction::Paste),
            ("全选", CtxAction::SelectAll),
        ],
        CtxKind::Block => vec![
            ("粘贴", CtxAction::Paste),
            ("全选", CtxAction::SelectAll),
            ("插入图片…", CtxAction::InsertImage),
        ],
    }
}

/// 右键菜单状态：打开位置（视口坐标）、项集与悬停；`link` 为 CopyLink 的载荷。
#[derive(Default)]
pub(crate) struct CtxMenu {
    pub(crate) open: bool,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) hover: Option<usize>,
    pub(crate) link: Option<String>,
    items: Vec<(&'static str, CtxAction)>,
}

impl CtxMenu {
    pub(crate) fn open_at(&mut self, x: f32, y: f32, kind: CtxKind, link: Option<String>) {
        self.open = true;
        self.x = x;
        self.y = y;
        self.hover = None;
        self.link = link;
        self.items = items_for(kind);
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.hover = None;
        self.link = None;
    }

    pub(crate) fn items(&self) -> &[(&'static str, CtxAction)] {
        &self.items
    }

    pub(crate) fn action(&self, i: usize) -> Option<CtxAction> {
        self.items.get(i).map(|&(_, a)| a)
    }
}

/// 与 chrome 菜单同源的命中宽度估算（命中框略宽无碍，不依赖上一帧绘制）。
fn text_width(text: &str, scale: f32) -> f32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                BASE_FONT_SIZE * 0.52 * scale
            } else {
                BASE_FONT_SIZE * scale
            }
        })
        .sum()
}

/// 布局：锚定右键点，钳位进内容视口（右/下缘不越界）。
pub(crate) struct Layout {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
    pub(crate) item_h: f32,
    pub(crate) pad: f32,
    /// 面板内边距（上下各一份），命中与绘制共用。
    pub(crate) top_pad: f32,
}

pub(crate) fn layout(
    items: &[(&'static str, CtxAction)],
    x: f32,
    y: f32,
    vp_right: f32,
    vp_bottom: f32,
    scale: f32,
) -> Layout {
    let pad = 12.0 * scale;
    let top_pad = 4.0 * scale;
    let item_h = chrome::ITEM_H * scale;
    let widest = items
        .iter()
        .map(|(l, _)| text_width(l, scale))
        .fold(0.0_f32, f32::max);
    let w = (widest + 2.0 * pad).max(120.0 * scale);
    let h = 2.0 * top_pad + item_h * items.len() as f32;
    let x = x.min((vp_right - w).max(0.0));
    let y = y.min((vp_bottom - h).max(0.0));
    Layout {
        x,
        y,
        w,
        h,
        item_h,
        pad,
        top_pad,
    }
}

/// 命中项下标（含上边距偏移；面板内空白为 None）。
pub(crate) fn hit(l: &Layout, count: usize, x: f32, y: f32) -> Option<usize> {
    if x < l.x || x >= l.x + l.w || y < l.y + l.top_pad {
        return None;
    }
    let i = ((y - l.y - l.top_pad) / l.item_h) as usize;
    (i < count).then_some(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_cover_three_contexts() {
        let sel = items_for(CtxKind::Selection);
        assert!(sel.iter().any(|&(_, a)| a == CtxAction::Cut));
        assert!(sel.iter().any(|&(_, a)| a == CtxAction::Copy));
        let link = items_for(CtxKind::Link);
        assert_eq!(link[0].1, CtxAction::CopyLink, "链接上下文首项为复制链接");
        let blk = items_for(CtxKind::Block);
        assert!(blk.iter().any(|&(_, a)| a == CtxAction::InsertImage));
        assert!(!blk.iter().any(|&(_, a)| a == CtxAction::Cut), "无选区不出现剪切");
    }

    #[test]
    fn layout_clamps_into_viewport() {
        let items = items_for(CtxKind::Block);
        let l = layout(&items, 790.0, 590.0, 800.0, 600.0, 1.0);
        assert!(l.x + l.w <= 800.0 && l.y + l.h <= 600.0, "右下缘不越界");
        let l2 = layout(&items, 10.0, 40.0, 800.0, 600.0, 1.0);
        assert_eq!((l2.x, l2.y), (10.0, 40.0), "空间充足时锚定原点");
    }

    #[test]
    fn hit_round_trips_items() {
        let items = items_for(CtxKind::Block);
        let l = layout(&items, 100.0, 100.0, 800.0, 600.0, 1.0);
        for i in 0..items.len() {
            let y = l.y + 4.0 + l.item_h * i as f32 + 1.0;
            assert_eq!(hit(&l, items.len(), l.x + 2.0, y), Some(i));
        }
        assert_eq!(hit(&l, items.len(), l.x + 2.0, l.y + 1.0), None, "上边距无命中");
        assert_eq!(
            hit(&l, items.len(), l.x - 1.0, l.y + l.top_pad + 1.0),
            None,
            "面板外无命中"
        );
    }
}
