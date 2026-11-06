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
    /// 用系统默认程序打开命中链接（协议白名单在 interaction 侧统一校验）。
    OpenLink,
    InsertImage,
    /// 表格结构操作（B2）：6 项均经 `Editor::replace_table_block` 落入源码。
    TableInsertRowAbove,
    TableInsertRowBelow,
    TableDeleteRow,
    TableInsertColumnLeft,
    TableInsertColumnRight,
    TableDeleteColumn,
    /// 列对齐（F3）：只改分隔行该列标记。
    TableAlignLeft,
    TableAlignCenter,
    TableAlignRight,
}

/// 表格右键上下文：打开时刻的命中位置（表头命中时 `body_row` 为 None）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TableCtx {
    /// 命中的正文行下标（0-based）；表头/分隔行为 None（不提供行操作）。
    pub(crate) body_row: Option<usize>,
    /// 命中的列下标（0-based）。
    pub(crate) col: usize,
    /// 表格列数（单列不提供删除列）。
    pub(crate) ncols: usize,
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
    /// 表格单元格/结构字符上。
    Table(TableCtx),
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
            // 「打开」在「复制」之前：链接上下文的首要意图是跳转
            ("在浏览器中打开", CtxAction::OpenLink),
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
        CtxKind::Table(tc) => {
            let mut items = Vec::new();
            // 表头只给列操作；正文行才给行操作
            if tc.body_row.is_some() {
                items.push(("上方插入行", CtxAction::TableInsertRowAbove));
                items.push(("下方插入行", CtxAction::TableInsertRowBelow));
                items.push(("删除行", CtxAction::TableDeleteRow));
            }
            items.push(("左侧插入列", CtxAction::TableInsertColumnLeft));
            items.push(("右侧插入列", CtxAction::TableInsertColumnRight));
            if tc.ncols > 1 {
                items.push(("删除列", CtxAction::TableDeleteColumn));
            }
            // 任意列（含表头、单列）都可对齐
            items.push(("左对齐", CtxAction::TableAlignLeft));
            items.push(("居中", CtxAction::TableAlignCenter));
            items.push(("右对齐", CtxAction::TableAlignRight));
            items.push(("粘贴", CtxAction::Paste));
            items.push(("全选", CtxAction::SelectAll));
            items
        }
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
    /// 表格上下文（打开时刻定；执行结构操作时读行列信息）。
    pub(crate) table: Option<TableCtx>,
    items: Vec<(&'static str, CtxAction)>,
}

impl CtxMenu {
    pub(crate) fn open_at(&mut self, x: f32, y: f32, kind: CtxKind, link: Option<String>) {
        self.open = true;
        self.x = x;
        self.y = y;
        self.hover = None;
        self.link = link;
        self.table = match kind {
            CtxKind::Table(tc) => Some(tc),
            _ => None,
        };
        self.items = items_for(kind);
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.hover = None;
        self.link = None;
        self.table = None;
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
        assert_eq!(
            link[0].1,
            CtxAction::OpenLink,
            "链接上下文首项为在浏览器中打开"
        );
        assert_eq!(
            link[1].1,
            CtxAction::CopyLink,
            "复制链接地址紧随其后"
        );
        let blk = items_for(CtxKind::Block);
        assert!(blk.iter().any(|&(_, a)| a == CtxAction::InsertImage));
        assert!(!blk.iter().any(|&(_, a)| a == CtxAction::Cut), "无选区不出现剪切");
    }

    #[test]
    fn table_header_only_offers_column_actions() {
        let tc = TableCtx {
            body_row: None,
            col: 0,
            ncols: 2,
        };
        let items = items_for(CtxKind::Table(tc));
        assert!(!items.iter().any(|&(_, a)| matches!(
            a,
            CtxAction::TableInsertRowAbove
                | CtxAction::TableInsertRowBelow
                | CtxAction::TableDeleteRow
        )));
        assert!(items.iter().any(|&(_, a)| a == CtxAction::TableInsertColumnLeft));
        assert!(items.iter().any(|&(_, a)| a == CtxAction::TableInsertColumnRight));
        assert!(items.iter().any(|&(_, a)| a == CtxAction::TableDeleteColumn));
        assert!(items.iter().any(|&(_, a)| a == CtxAction::Paste));
        assert!(items.iter().any(|&(_, a)| a == CtxAction::SelectAll));
    }

    #[test]
    fn table_single_column_hides_delete_column() {
        let tc = TableCtx {
            body_row: Some(0),
            col: 0,
            ncols: 1,
        };
        let items = items_for(CtxKind::Table(tc));
        assert!(items.iter().any(|&(_, a)| a == CtxAction::TableInsertRowAbove));
        assert!(items.iter().any(|&(_, a)| a == CtxAction::TableDeleteRow));
        assert!(!items.iter().any(|&(_, a)| a == CtxAction::TableDeleteColumn));
    }

    #[test]
    fn table_body_row_offers_all_six_actions() {
        let tc = TableCtx {
            body_row: Some(1),
            col: 0,
            ncols: 3,
        };
        let items = items_for(CtxKind::Table(tc));
        for want in [
            CtxAction::TableInsertRowAbove,
            CtxAction::TableInsertRowBelow,
            CtxAction::TableDeleteRow,
            CtxAction::TableInsertColumnLeft,
            CtxAction::TableInsertColumnRight,
            CtxAction::TableDeleteColumn,
        ] {
            assert!(items.iter().any(|&(_, a)| a == want), "缺少 {want:?}");
        }
        assert_eq!(items.len(), 11, "6 项结构操作 + 3 项对齐 + 粘贴 + 全选");
    }

    #[test]
    fn table_align_items_present_for_any_column() {
        for tc in [
            TableCtx {
                body_row: None,
                col: 0,
                ncols: 1,
            },
            TableCtx {
                body_row: Some(0),
                col: 2,
                ncols: 3,
            },
        ] {
            let items = items_for(CtxKind::Table(tc));
            for want in [
                CtxAction::TableAlignLeft,
                CtxAction::TableAlignCenter,
                CtxAction::TableAlignRight,
            ] {
                assert!(items.iter().any(|&(_, a)| a == want), "{tc:?} 缺少 {want:?}");
            }
        }
    }

    #[test]
    fn opening_table_context_caches_table_and_closes_clears() {
        let mut m = CtxMenu::default();
        let tc = TableCtx {
            body_row: Some(0),
            col: 1,
            ncols: 2,
        };
        m.open_at(0.0, 0.0, CtxKind::Table(tc), None);
        assert_eq!(m.table, Some(tc));
        m.close();
        assert_eq!(m.table, None);
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
