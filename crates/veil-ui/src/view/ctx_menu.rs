//! 右键上下文菜单（2f）：选区/链接/块三类上下文的项集、几何与命中。
//! 纯模型与几何：不绘制（paint.rs 消费布局）、不执行（interaction.rs 分发动作）。

use crate::i18n::{t, Key};
use crate::tokens::{space, typography};
use veil_core::BlockId;

/// 上下文菜单动作：执行映射在 interaction.rs（复用既有剪贴板/插入路径）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CtxAction {
    /// 分组分隔线：无动作、无命中区、不绘制文字。
    Separator,
    Cut,
    Copy,
    Paste,
    SelectAll,
    /// 复制命中链接的 URL（inline 链接才有；引用式无 url 区间不出现本项）。
    CopyLink,
    /// 复制命中的围栏代码块源码（不含围栏行）。
    CopyCodeBlock,
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
    /// Mermaid v0：在 mermaid.live 在线预览当前源码。
    MermaidPreview,
    /// Mermaid：应用内预览在线渲染的 PNG（mermaid.ink，显式点击才联网）。
    MermaidPreviewImage,
    /// Mermaid：下载在线渲染的 PNG 到 `<doc>.assets/` 并插入图片引用。
    MermaidInsertImage,
    /// Mermaid v0：复制当前 mermaid 块的源码（不含围栏行）到剪贴板。
    MermaidCopySource,
    /// Mermaid v0：复制 mermaid.live 预览 URL 到剪贴板。
    MermaidCopyLink,
    /// Mermaid v0：插入预设模板（三种 diagram 类型）。
    MermaidInsertFlowchart,
    MermaidInsertSequence,
    MermaidInsertClass,
    /// Details 块折叠 / 展开（右键菜单入口，与左键点击 summary 行同源）。
    FoldDetails,
    UnfoldDetails,
    /// 图片专用动作（DEF-T67-E1-005）：复制源路径 / 清除失败缓存后重绘。
    CopyImagePath,
    ReloadImage,
    /// 缺失图片的可见提示项：不执行动作，仅说明当前为替代文本。
    ImageMissingHint,
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
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum CtxKind {
    /// 右键落在既有选区内（选区保留不动）。
    Selection,
    /// 光标落在 inline 链接节点内（含标记区）。
    Link,
    /// 其余正文位置。
    Block,
    /// 表格单元格/结构字符上。
    Table(TableCtx),
    /// Mermaid v0：mermaid 围栏代码块上。
    MermaidBlock,
    /// Mermaid 空块（内容为空或全为 `%%` 注释）：只给模板项。
    MermaidEmptyBlock,
    /// 普通围栏代码块：首项为「复制代码块」。
    CodeBlock { id: BlockId },
    /// Details 块：携带命中块的 BlockId 与打开时刻的展开态，
    /// 菜单只列出与当前状态匹配的折叠/展开项（DEF-T07-001）。
    Details { id: BlockId, expanded: bool },
    /// 图片（Markdown `![]()` 或 `<img>`）：携带源与缺失状态（DEF-T67-E1-005）。
    Image { path: String, missing: bool },
}

/// 各类上下文的项集（单一来源：布局/命中/分发共用）。
/// 分隔线按语义分组：编辑类 / 选择类 / 链接类 / 表格结构 / 对齐 / 剪贴板。
pub(crate) fn items_for(kind: CtxKind) -> Vec<(Key, CtxAction)> {
    match kind {
        CtxKind::Selection => vec![
            (Key::CtxCut, CtxAction::Cut),
            (Key::CtxCopy, CtxAction::Copy),
            (Key::CtxPaste, CtxAction::Paste),
            (Key::Empty, CtxAction::Separator),
            (Key::CtxSelectAll, CtxAction::SelectAll),
        ],
        CtxKind::Link => vec![
            // 「打开」在「复制」之前：链接上下文的首要意图是跳转
            (Key::CtxOpenInBrowser, CtxAction::OpenLink),
            (Key::CtxCopyLink, CtxAction::CopyLink),
            (Key::Empty, CtxAction::Separator),
            (Key::CtxCopy, CtxAction::Copy),
            (Key::CtxPaste, CtxAction::Paste),
            (Key::Empty, CtxAction::Separator),
            (Key::CtxSelectAll, CtxAction::SelectAll),
        ],
        CtxKind::Block => vec![
            (Key::CtxPaste, CtxAction::Paste),
            (Key::CtxSelectAll, CtxAction::SelectAll),
            (Key::Empty, CtxAction::Separator),
            (Key::CtxInsertImage, CtxAction::InsertImage),
        ],
        // 代码块：复制源码为唯一新增项；其余为标准块操作（粘贴/全选）。
        CtxKind::CodeBlock { .. } => vec![
            (Key::CtxCopyCode, CtxAction::CopyCodeBlock),
            (Key::Empty, CtxAction::Separator),
            (Key::CtxPaste, CtxAction::Paste),
            (Key::CtxSelectAll, CtxAction::SelectAll),
        ],
        CtxKind::MermaidBlock => vec![
            (Key::CtxMermaidPreview, CtxAction::MermaidPreview),
            (
                Key::CtxMermaidPreviewImage,
                CtxAction::MermaidPreviewImage,
            ),
            (Key::CtxMermaidInsertImage, CtxAction::MermaidInsertImage),
            (Key::Empty, CtxAction::Separator),
            (Key::CtxMermaidCopySource, CtxAction::MermaidCopySource),
            (Key::CtxMermaidCopyLink, CtxAction::MermaidCopyLink),
            (Key::Empty, CtxAction::Separator),
            (
                Key::CtxMermaidInsertFlowchart,
                CtxAction::MermaidInsertFlowchart,
            ),
            (
                Key::CtxMermaidInsertSequence,
                CtxAction::MermaidInsertSequence,
            ),
            (Key::CtxMermaidInsertClass, CtxAction::MermaidInsertClass),
            (Key::Empty, CtxAction::Separator),
            (Key::CtxPaste, CtxAction::Paste),
            (Key::CtxSelectAll, CtxAction::SelectAll),
        ],
        // 空块没有可预览/可复制的源码，只保留三条模板项（与提示行点击同源）。
        CtxKind::MermaidEmptyBlock => vec![
            (
                Key::CtxMermaidInsertFlowchart,
                CtxAction::MermaidInsertFlowchart,
            ),
            (
                Key::CtxMermaidInsertSequence,
                CtxAction::MermaidInsertSequence,
            ),
            (Key::CtxMermaidInsertClass, CtxAction::MermaidInsertClass),
        ],
        // Details 块：只列与当前展开态匹配的一项（错误项不再出现且不再是 no-op）
        CtxKind::Details { expanded, .. } => {
            let mut items = Vec::new();
            if expanded {
                items.push((Key::CtxFold, CtxAction::FoldDetails));
            } else {
                items.push((Key::CtxUnfold, CtxAction::UnfoldDetails));
            }
            items.push((Key::Empty, CtxAction::Separator));
            items.push((Key::CtxCopy, CtxAction::Copy));
            items.push((Key::CtxPaste, CtxAction::Paste));
            items.push((Key::Empty, CtxAction::Separator));
            items.push((Key::CtxSelectAll, CtxAction::SelectAll));
            items
        }
        // 图片：复制路径/重新加载；缺失时首项给出可见提示（DEF-T67-E1-005）。
        CtxKind::Image { missing, .. } => {
            let mut items = Vec::new();
            if missing {
                items.push((Key::CtxImageMissing, CtxAction::ImageMissingHint));
                items.push((Key::Empty, CtxAction::Separator));
            }
            items.push((Key::CtxCopyImagePath, CtxAction::CopyImagePath));
            items.push((Key::CtxReloadImage, CtxAction::ReloadImage));
            items.push((Key::Empty, CtxAction::Separator));
            items.push((Key::CtxPaste, CtxAction::Paste));
            items.push((Key::CtxSelectAll, CtxAction::SelectAll));
            items
        }
        CtxKind::Table(tc) => {
            let mut items = Vec::new();
            // 表头只给列操作；正文行才给行操作
            if tc.body_row.is_some() {
                items.push((Key::CtxTableInsertRowAbove, CtxAction::TableInsertRowAbove));
                items.push((Key::CtxTableInsertRowBelow, CtxAction::TableInsertRowBelow));
                items.push((Key::CtxTableDeleteRow, CtxAction::TableDeleteRow));
                items.push((Key::Empty, CtxAction::Separator));
            }
            items.push((
                Key::CtxTableInsertColumnLeft,
                CtxAction::TableInsertColumnLeft,
            ));
            items.push((
                Key::CtxTableInsertColumnRight,
                CtxAction::TableInsertColumnRight,
            ));
            if tc.ncols > 1 {
                items.push((Key::CtxTableDeleteColumn, CtxAction::TableDeleteColumn));
            }
            items.push((Key::Empty, CtxAction::Separator));
            // 任意列（含表头、单列）都可对齐
            items.push((Key::CtxTableAlignLeft, CtxAction::TableAlignLeft));
            items.push((Key::CtxTableAlignCenter, CtxAction::TableAlignCenter));
            items.push((Key::CtxTableAlignRight, CtxAction::TableAlignRight));
            items.push((Key::Empty, CtxAction::Separator));
            items.push((Key::CtxPaste, CtxAction::Paste));
            items.push((Key::CtxSelectAll, CtxAction::SelectAll));
            items
        }
    }
}

/// 右键菜单状态：打开位置（视口坐标）、项集与悬停；`link` 为 CopyLink 的载荷。
pub(crate) struct CtxMenu {
    pub(crate) open: bool,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) hover: Option<usize>,
    /// 悬停过渡（S6）：菜单项高亮淡入/淡出。
    pub(crate) fade: crate::anim::Fade,
    pub(crate) link: Option<String>,
    /// 表格上下文（打开时刻定；执行结构操作时读行列信息）。
    pub(crate) table: Option<TableCtx>,
    /// 远程图片总开关：关闭时隐藏 Mermaid 在线渲染两项（打开前由 interaction 写入）。
    pub(crate) remote_ok: bool,
    /// 右键 mermaid 块的内容结束偏移（「下载并插入」的插入锚点）。
    pub(crate) mermaid_end: Option<usize>,
    /// Mermaid v0：打开时刻捕获的块 body 源码（不含围栏行），供预览/复制链接动作读取。
    pub(crate) mermaid_body: Option<String>,
    /// Mermaid 空块：模板插入锚点（body 起点，绝对字节偏移）；非空块为 None，
    /// 此时沿用既有的「在光标处插入」语义。
    pub(crate) mermaid_insert_start: Option<usize>,
    /// Details 块右键菜单携带的 BlockId（供折叠/展开动作定位目标块）。
    pub(crate) details_id: Option<BlockId>,
    /// 图片右键菜单携带的源路径（供复制路径/重新加载动作）。
    pub(crate) image_path: Option<String>,
    /// 代码块右键菜单携带的块 id（供复制代码块动作读取源码）。
    pub(crate) code_id: Option<BlockId>,
    items: Vec<(Key, CtxAction)>,
}

impl Default for CtxMenu {
    fn default() -> Self {
        Self {
            open: false,
            x: 0.0,
            y: 0.0,
            hover: None,
            fade: crate::anim::Fade::default(),
            link: None,
            table: None,
            remote_ok: true,
            mermaid_end: None,
            mermaid_body: None,
            mermaid_insert_start: None,
            details_id: None,
            image_path: None,
            code_id: None,
            items: Vec::new(),
        }
    }
}

impl CtxMenu {
    pub(crate) fn open_at(
        &mut self,
        x: f32,
        y: f32,
        kind: CtxKind,
        link: Option<String>,
        mermaid_body: Option<String>,
    ) {
        self.open = true;
        self.x = x;
        self.y = y;
        self.hover = None;
        self.link = link;
        self.table = match &kind {
            CtxKind::Table(tc) => Some(*tc),
            _ => None,
        };
        self.mermaid_body = mermaid_body;
        self.mermaid_end = None;
        self.mermaid_insert_start = None;
        self.details_id = match &kind {
            CtxKind::Details { id, .. } => Some(*id),
            _ => None,
        };
        self.image_path = match &kind {
            CtxKind::Image { path, .. } => Some(path.clone()),
            _ => None,
        };
        self.code_id = match &kind {
            CtxKind::CodeBlock { id } => Some(*id),
            _ => None,
        };
        self.items = items_for(kind);
        if !self.remote_ok {
            self.items.retain(|&(_, a)| {
                !matches!(
                    a,
                    CtxAction::MermaidPreviewImage | CtxAction::MermaidInsertImage
                )
            });
        }
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.hover = None;
        self.link = None;
        self.table = None;
        self.mermaid_body = None;
        self.mermaid_end = None;
        self.mermaid_insert_start = None;
        self.code_id = None;
    }

    pub(crate) fn items(&self) -> &[(Key, CtxAction)] {
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
                typography::UI_SIZE * 0.52 * scale
            } else {
                typography::UI_SIZE * scale
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
    /// (项下标, 相对面板顶部的 y 偏移)；分隔符不在此列。
    pub(crate) rows: Vec<(usize, f32)>,
}

pub(crate) fn layout(
    items: &[(Key, CtxAction)],
    x: f32,
    y: f32,
    vp_right: f32,
    vp_bottom: f32,
    scale: f32,
) -> Layout {
    let pad = crate::theme::surface::padding(scale);
    let top_pad = 4.0 * scale;
    let item_h = crate::theme::surface::row_height(scale);
    // 分组只用间距，不画分隔线（S3）。
    let sep_h = space::S * scale;
    let widest = items
        .iter()
        .filter(|(_, a)| *a != CtxAction::Separator)
        .map(|(l, _)| text_width(t(*l), scale))
        .fold(0.0_f32, f32::max);
    let w = (widest + 2.0 * pad).max(120.0 * scale);
    let mut cur = top_pad;
    let mut rows = Vec::with_capacity(items.len());
    for (i, (_, a)) in items.iter().enumerate() {
        if *a == CtxAction::Separator {
            cur += sep_h;
        } else {
            rows.push((i, cur));
            cur += item_h;
        }
    }
    let h = cur + top_pad;
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
        rows,
    }
}

/// 命中项下标（分隔线与面板空白均返回 None）。
pub(crate) fn hit(l: &Layout, x: f32, y: f32) -> Option<usize> {
    if x < l.x || x >= l.x + l.w || y < l.y + l.top_pad {
        return None;
    }
    let dy = y - l.y;
    l.rows
        .iter()
        .find(|&&(_, off)| dy >= off && dy < off + l.item_h)
        .map(|&(i, _)| i)
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
        assert_eq!(link[1].1, CtxAction::CopyLink, "复制链接地址紧随其后");
        let blk = items_for(CtxKind::Block);
        assert!(blk.iter().any(|&(_, a)| a == CtxAction::InsertImage));
        assert!(
            !blk.iter().any(|&(_, a)| a == CtxAction::Cut),
            "无选区不出现剪切"
        );
    }

    /// DEF-T07-001：Details 菜单只出现与当前展开态匹配的一项。
    #[test]
    fn details_menu_matches_expanded_state() {
        let expanded = items_for(CtxKind::Details {
            id: BlockId(7),
            expanded: true,
        });
        assert!(expanded.iter().any(|&(_, a)| a == CtxAction::FoldDetails));
        assert!(
            !expanded
                .iter()
                .any(|&(_, a)| a == CtxAction::UnfoldDetails),
            "已展开时不得再列展开"
        );

        let collapsed = items_for(CtxKind::Details {
            id: BlockId(7),
            expanded: false,
        });
        assert!(collapsed
            .iter()
            .any(|&(_, a)| a == CtxAction::UnfoldDetails));
        assert!(
            !collapsed.iter().any(|&(_, a)| a == CtxAction::FoldDetails),
            "已折叠时不得再列折叠"
        );
    }

    /// DEF-T67-E1-005：图片菜单有专用动作；缺失图片额外给出可见提示项。
    #[test]
    fn image_menu_has_dedicated_actions_and_missing_hint() {
        let normal = items_for(CtxKind::Image {
            path: "img/a.png".into(),
            missing: false,
        });
        assert!(normal
            .iter()
            .any(|&(_, a)| a == CtxAction::CopyImagePath));
        assert!(normal.iter().any(|&(_, a)| a == CtxAction::ReloadImage));
        assert!(!normal
            .iter()
            .any(|&(_, a)| a == CtxAction::ImageMissingHint));

        let missing = items_for(CtxKind::Image {
            path: "img/gone.png".into(),
            missing: true,
        });
        assert_eq!(missing[0].1, CtxAction::ImageMissingHint);
        assert!(missing
            .iter()
            .any(|&(_, a)| a == CtxAction::CopyImagePath));
        assert!(missing.iter().any(|&(_, a)| a == CtxAction::ReloadImage));
    }

    /// 代码块上下文：首项为复制代码块，且不混入无关项（无剪切/插入图片）。
    #[test]
    fn code_block_menu_offers_copy_code_first() {
        let items = items_for(CtxKind::CodeBlock { id: BlockId(1) });
        assert_eq!(
            items[0],
            (Key::CtxCopyCode, CtxAction::CopyCodeBlock),
            "复制代码块为首项"
        );
        assert!(!items
            .iter()
            .any(|&(_, a)| matches!(a, CtxAction::Cut | CtxAction::InsertImage | CtxAction::Copy)));
        assert!(items.iter().any(|&(_, a)| a == CtxAction::Paste));
        assert!(items.iter().any(|&(_, a)| a == CtxAction::SelectAll));
    }

    #[test]
    fn opening_image_context_caches_source_path() {
        let mut m = CtxMenu::default();
        m.open_at(
            0.0,
            0.0,
            CtxKind::Image {
                path: "img/a.png".into(),
                missing: false,
            },
            None,
            None,
        );
        assert_eq!(m.image_path.as_deref(), Some("img/a.png"));
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
        assert!(items
            .iter()
            .any(|&(_, a)| a == CtxAction::TableInsertColumnLeft));
        assert!(items
            .iter()
            .any(|&(_, a)| a == CtxAction::TableInsertColumnRight));
        assert!(items
            .iter()
            .any(|&(_, a)| a == CtxAction::TableDeleteColumn));
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
        assert!(items
            .iter()
            .any(|&(_, a)| a == CtxAction::TableInsertRowAbove));
        assert!(items.iter().any(|&(_, a)| a == CtxAction::TableDeleteRow));
        assert!(!items
            .iter()
            .any(|&(_, a)| a == CtxAction::TableDeleteColumn));
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
        assert_eq!(
            items.len(),
            14,
            "6 项结构操作 + 3 项对齐 + 粘贴 + 全选 + 3 条分隔线"
        );
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
                assert!(
                    items.iter().any(|&(_, a)| a == want),
                    "{tc:?} 缺少 {want:?}"
                );
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
        m.open_at(0.0, 0.0, CtxKind::Table(tc), None, None);
        assert_eq!(m.table, Some(tc));
        m.close();
        assert_eq!(m.table, None);
    }

    #[test]
    fn every_context_groups_items_with_separators() {
        for kind in [
            CtxKind::Selection,
            CtxKind::Link,
            CtxKind::Block,
            CtxKind::CodeBlock { id: BlockId(1) },
            CtxKind::Table(TableCtx {
                body_row: Some(0),
                col: 0,
                ncols: 3,
            }),
            CtxKind::Table(TableCtx {
                body_row: None,
                col: 0,
                ncols: 1,
            }),
        ] {
            let items = items_for(kind.clone());
            assert!(
                items.iter().any(|&(_, a)| a == CtxAction::Separator),
                "{kind:?} 应有分组分隔线"
            );
            assert_ne!(items[0].1, CtxAction::Separator, "首项不得是分隔线");
            assert_ne!(
                items.last().unwrap().1,
                CtxAction::Separator,
                "末项不得是分隔线"
            );
            assert!(
                !items
                    .windows(2)
                    .any(|w| w[0].1 == CtxAction::Separator && w[1].1 == CtxAction::Separator),
                "{kind:?} 不允许连续分隔线"
            );
        }
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
        // 每个可点项的中心都能命中自身（分隔线不占命中区）
        for &(i, off) in &l.rows {
            let y = l.y + off + l.item_h / 2.0;
            assert_eq!(hit(&l, l.x + 2.0, y), Some(i));
        }
        assert_eq!(hit(&l, l.x + 2.0, l.y + 1.0), None, "上边距无命中");
        assert_eq!(
            hit(&l, l.x - 1.0, l.y + l.top_pad + 1.0),
            None,
            "面板外无命中"
        );
        // 分组只用 8px 间距：分隔符两侧的行间空档为 8px
        let seps = items
            .iter()
            .filter(|(_, a)| *a == CtxAction::Separator)
            .count();
        let gaps = l
            .rows
            .windows(2)
            .filter(|w| (w[1].1 - (w[0].1 + l.item_h) - 8.0).abs() < 0.01)
            .count();
        assert_eq!(gaps, seps, "每个分隔符对应一个 8px 分组间距");
        // 面板高度 = 上内边距 + 可点项行高 + 分组间距 + 下内边距
        let expect = 2.0 * l.top_pad + l.item_h * l.rows.len() as f32 + 8.0 * seps as f32;
        assert!((l.h - expect).abs() < 0.01, "高度包含分组间距");
    }
}
