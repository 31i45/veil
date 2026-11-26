//! AccessKit 无障碍支持：mixed handlers + 可导航的 chrome/正文语义树。
//!
//! 整体架构（参照 spike `accesskit-spike/src/main.rs`）：
//! - `ShellEvent::{Doc(Doc), AccessKit(Event)}` 统一用户事件类型；
//! - 每个窗口在隐藏阶段创建 `accesskit_winit::Adapter`（严格早于首次显示）；
//! - mixed handler：同步 `ActivationHandler` 返回线程安全快照；action/deactivation
//!   由 accesskit_winit 内部经 proxy 回主线程；
//! - `Shell::route_event` 在应用 match 前调用 `adapter.process_event(...)`。
//!
//! 与旧实现的关键差异（解决「Narrator 只能读标题」）：
//! - 菜单栏**始终**包含四个顶层菜单，每个菜单挂全部非分隔线菜单项；不需要先打开
//!   菜单才能被屏幕阅读器发现；
//! - 增加正文节点（`Role::MultilineTextInput`，label「正文」）。值只暴露**有界快照**
//!   （前 [`BODY_SNAPSHOT_BYTES`] 字节，按字符边界截断），caret/selection 与逐行
//!   文本尚未完整映射——这是已知限制，不是「完整正文语义」；
//! - 模态打开时后台控件（菜单栏/正文/面板）从树中移除，只保留 Dialog 与按钮；
//! - 菜单项 Click 直接路由到 `App::perform_menu`，模态按钮 Click 路由到现有
//!   `resolve_modal` / `resolve_settings`，Focus 只更新语义焦点、不伪造视觉状态。

#![cfg(feature = "accessibility")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::ThreadId;

use accesskit::{Action, ActivationHandler, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use accesskit_winit::Event as AccessKitEvent;
use veil_core::Range;
use winit::window::WindowId;

use crate::app::{App, Doc};
use crate::chrome::{self, MenuAction};
use crate::i18n::{t, Key};

// —— Node ID 规划（u64）—— //
pub(crate) const ROOT_ID: NodeId = NodeId(1);
pub(crate) const BODY_ID: NodeId = NodeId(2);
pub(crate) const MENU_BAR_ID: NodeId = NodeId(100);
// 顶层菜单基址：文件=200、编辑=300、视图=400、帮助=500。
// 菜单项 id = 基址 + 1 + 该菜单在 `chrome::menu_rows` 中的行下标（含分隔线占位），
// 每个菜单可用 99 个槽位，远大于实际条目数（最近文件上限 10）。
pub(crate) const MENU_BASE: [NodeId; 4] = [NodeId(200), NodeId(300), NodeId(400), NodeId(500)];

pub(crate) const MODAL_ID: NodeId = NodeId(10_000);
pub(crate) const CONTEXT_MENU_ID: NodeId = NodeId(20_000);

pub(crate) const OUTLINE_PANEL_ID: NodeId = NodeId(30_000);
pub(crate) const QUICK_OPEN_PANEL_ID: NodeId = NodeId(31_000);
pub(crate) const FIND_PANEL_ID: NodeId = NodeId(32_000);

/// 正文快照上限：只暴露前 8000 字节，避免每次语义更新都全量复制/朗读整个文档。
/// 多字节字符的回退在 [`body_snapshot`] 中按字符边界处理（最多回退 3 字节）。
pub(crate) const BODY_SNAPSHOT_BYTES: usize = 8000;

/// Shell 用户事件枚举：把外部文档请求与 AccessKit 事件合并成单一事件循环类型。
pub enum ShellEvent {
    Doc(Doc),
    AccessKit(AccessKitEvent),
}

impl std::fmt::Debug for ShellEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Doc(_) => f.debug_tuple("ShellEvent::Doc").finish_non_exhaustive(),
            Self::AccessKit(e) => f
                .debug_tuple("ShellEvent::AccessKit")
                .field(&e.window_id)
                .finish(),
        }
    }
}

impl From<Doc> for ShellEvent {
    fn from(doc: Doc) -> Self {
        Self::Doc(doc)
    }
}

impl From<AccessKitEvent> for ShellEvent {
    fn from(event: AccessKitEvent) -> Self {
        Self::AccessKit(event)
    }
}

// —— 线程安全快照 + 同步 activation —— //

/// mixed handler 的同步 activation：只返回快照，不碰 UI 状态。
pub(crate) struct SnapshotActivation {
    pub(crate) snapshot: Arc<Mutex<TreeUpdate>>,
    pub(crate) activated: Arc<AtomicBool>,
    pub(crate) window_id: WindowId,
    pub(crate) main_thread: ThreadId,
}

impl ActivationHandler for SnapshotActivation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        let callback_thread = thread::current().id();
        let _ = (self.window_id, self.main_thread, callback_thread);
        let update = self
            .snapshot
            .lock()
            .expect("semantic snapshot lock poisoned")
            .clone();
        self.activated.store(true, Ordering::Release);
        Some(update)
    }
}

// —— 构建语义树 —— //

/// 有界正文快照：取前 `BODY_SNAPSHOT_BYTES` 字节，按 UTF-8 字符边界回退。
/// 只暴露纯文本；caret/selection/逐行结构尚未映射（见模块文档）。
fn body_snapshot(app: &App) -> String {
    let session = app.editor.session();
    let len = session.text_len();
    let mut end = len.min(BODY_SNAPSHOT_BYTES);
    while end > 0 {
        if let Some(text) = session.text_slice(Range::new(0, end)) {
            return text;
        }
        end -= 1;
    }
    String::new()
}

/// 某菜单当前行（含最近文件动态项）；标签/快捷键/动作与绘制、命中同源。
fn menu_rows(app: &App, menu: usize) -> Vec<(String, &'static str, MenuAction)> {
    chrome::menu_rows(menu, &app.recent_labels())
}

/// 按当前 App 状态构建语义树（无外部焦点覆盖）。
pub(crate) fn build_semantic_tree(app: &App) -> TreeUpdate {
    build_semantic_tree_with_focus(app, None)
}

/// 构建完整语义树。`focus_override` 来自 AccessKit `Action::Focus`——若目标节点
/// 不在本次树中则忽略，回退到按 UI 状态计算的焦点。
pub(crate) fn build_semantic_tree_with_focus(
    app: &App,
    focus_override: Option<NodeId>,
) -> TreeUpdate {
    let mut nodes: Vec<(NodeId, Node)> = Vec::new();
    let mut children: Vec<NodeId> = Vec::new();
    let modal_active = app.ui_state.settings.is_some() || app.ui_state.modal.is_some();

    if modal_active {
        // —— 模态：后台控件全部移除，只留 Dialog + 按钮 —— //
        if let Some(settings) = &app.ui_state.settings {
            let mut dialog = Node::new(Role::Dialog);
            dialog.set_label(t(Key::MenuViewSettings));
            dialog.set_modal();
            let mut buttons = Vec::new();
            for (i, btn) in settings.buttons.iter().enumerate() {
                let id = NodeId(MODAL_ID.0 + 1 + i as u64);
                let mut node = Node::new(Role::Button);
                node.set_label(btn.display_label());
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                nodes.push((id, node));
                buttons.push(id);
            }
            dialog.set_children(buttons);
            nodes.push((MODAL_ID, dialog));
        } else if let Some(modal) = &app.ui_state.modal {
            let mut dialog = Node::new(Role::Dialog);
            dialog.set_label(modal.spec.title.clone());
            dialog.set_modal();
            let mut buttons = Vec::new();
            for (i, btn) in modal.spec.buttons.iter().enumerate() {
                let id = NodeId(MODAL_ID.0 + 1 + i as u64);
                let mut node = Node::new(Role::Button);
                node.set_label(btn.display_label());
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                nodes.push((id, node));
                buttons.push(id);
            }
            dialog.set_children(buttons);
            nodes.push((MODAL_ID, dialog));
        }
        children.push(MODAL_ID);
    } else {
        // —— 菜单栏：始终包含四个菜单，每个菜单挂全部非分隔线项 —— //
        let mut menu_bar = Node::new(Role::MenuBar);
        menu_bar.set_label("主菜单栏");
        let mut menu_ids = Vec::with_capacity(chrome::MENUS.len());
        for (menu_idx, def) in chrome::MENUS.iter().enumerate() {
            let menu_id = MENU_BASE[menu_idx];
            let mut menu_node = Node::new(Role::Menu);
            menu_node.set_label(t(def.title));
            let flags = app.menu_flags();
            let mut item_ids = Vec::new();
            for (row_idx, (label, shortcut, action)) in
                menu_rows(app, menu_idx).into_iter().enumerate()
            {
                if matches!(action, MenuAction::Separator) {
                    continue;
                }
                let id = NodeId(menu_id.0 + 1 + row_idx as u64);
                let mut item = Node::new(Role::MenuItem);
                item.set_label(label);
                if !shortcut.is_empty() {
                    item.set_keyboard_shortcut(shortcut.to_string());
                }
                if !chrome::action_enabled(action, flags) {
                    item.set_disabled();
                }
                item.add_action(Action::Click);
                item.add_action(Action::Focus);
                nodes.push((id, item));
                item_ids.push(id);
            }
            menu_node.set_children(item_ids);
            nodes.push((menu_id, menu_node));
            menu_ids.push(menu_id);
        }
        menu_bar.set_children(menu_ids);
        nodes.push((MENU_BAR_ID, menu_bar));
        children.push(MENU_BAR_ID);

        // —— 正文：有界快照 —— //
        let mut body = Node::new(Role::MultilineTextInput);
        body.set_label("正文");
        let snapshot = body_snapshot(app);
        if !snapshot.is_empty() {
            body.set_value(snapshot);
        }
        body.add_action(Action::Focus);
        nodes.push((BODY_ID, body));
        children.push(BODY_ID);

        // —— 面板（查找/大纲/快速打开）与右键菜单：保持旧角色 —— //
        if app.ui_state.search.open {
            let mut panel = Node::new(Role::Group);
            panel.set_label(if app.ui_state.search.replace_open {
                format!("{} / {}", t(Key::MenuEditFind), t(Key::MenuEditReplace))
            } else {
                t(Key::MenuEditFind).to_string()
            });
            panel.add_action(Action::Focus);
            nodes.push((FIND_PANEL_ID, panel));
            children.push(FIND_PANEL_ID);
        }
        if app.ui_state.outline.open {
            let mut panel = Node::new(Role::Group);
            panel.set_label(t(Key::OutlineTitle));
            panel.add_action(Action::Focus);
            nodes.push((OUTLINE_PANEL_ID, panel));
            children.push(OUTLINE_PANEL_ID);
        }
        if app.ui_state.quick.open {
            let mut panel = Node::new(Role::Group);
            panel.set_label(match app.ui_state.quick.kind {
                crate::view::quick_open::QuickKind::Files => t(Key::MenuFileQuickOpen).to_string(),
                crate::view::quick_open::QuickKind::Commands => "命令面板".to_string(),
            });
            panel.add_action(Action::Focus);
            nodes.push((QUICK_OPEN_PANEL_ID, panel));
            children.push(QUICK_OPEN_PANEL_ID);
        }
        if app.ui_state.ctx.open {
            let mut menu = Node::new(Role::Menu);
            menu.set_label("上下文菜单");
            menu.add_action(Action::Focus);
            nodes.push((CONTEXT_MENU_ID, menu));
            children.push(CONTEXT_MENU_ID);
        }
    }

    // 根节点最后组装（节点顺序对 AccessKit 不重要）。
    let mut root = Node::new(Role::Window);
    root.set_label(app.title_text());
    root.set_children(children);
    nodes.push((ROOT_ID, root));

    let computed = computed_focus(app);
    let focus = focus_override
        .filter(|id| nodes.iter().any(|(node_id, _)| node_id == id))
        .unwrap_or(computed);

    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(ROOT_ID)),
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// 按 UI 状态计算焦点：模态 → 主按钮；菜单打开 → 对应菜单项；
/// 否则正文（无正文节点时回退根）。
fn computed_focus(app: &App) -> NodeId {
    if let Some(settings) = &app.ui_state.settings {
        let primary = settings
            .buttons
            .iter()
            .position(|b| b.role == crate::view::modal::ButtonRole::Primary)
            .unwrap_or(0);
        return NodeId(MODAL_ID.0 + 1 + primary as u64);
    }
    if let Some(modal) = &app.ui_state.modal {
        if let Some(primary) = modal.spec.primary_id() {
            if let Some(index) = modal.spec.buttons.iter().position(|b| b.id == primary) {
                return NodeId(MODAL_ID.0 + 1 + index as u64);
            }
        }
        return MODAL_ID;
    }
    if let Some(open) = app.ui_state.chrome.open {
        if open < chrome::MENUS.len() {
            return menu_focus_id(app, open);
        }
    }
    BODY_ID
}

/// 菜单打开时的焦点：优先悬停项，其次首个可用项，最后菜单节点本身。
fn menu_focus_id(app: &App, menu: usize) -> NodeId {
    let menu_id = MENU_BASE[menu];
    let flags = app.menu_flags();
    let rows = menu_rows(app, menu);
    if let Some((hover_menu, row)) = app.ui_state.chrome.hover_item {
        if hover_menu == menu {
            if let Some((_, _, action)) = rows.get(row) {
                if !matches!(action, MenuAction::Separator) {
                    return NodeId(menu_id.0 + 1 + row as u64);
                }
            }
        }
    }
    for (row, (_, _, action)) in rows.iter().enumerate() {
        if !matches!(action, MenuAction::Separator) && chrome::action_enabled(*action, flags) {
            return NodeId(menu_id.0 + 1 + row as u64);
        }
    }
    menu_id
}

// —— 动作路由 —— //

/// 解析菜单项节点 id → (菜单下标, `menu_rows` 行下标)。
fn menu_item_from_node(node: NodeId) -> Option<(usize, usize)> {
    for (menu, base) in MENU_BASE.iter().enumerate() {
        if node.0 > base.0 && node.0 <= base.0 + 99 {
            return Some((menu, (node.0 - base.0 - 1) as usize));
        }
    }
    None
}

/// 解析模态按钮节点 id → 按钮下标。
fn modal_button_index(node: NodeId) -> Option<usize> {
    let offset = node.0.checked_sub(MODAL_ID.0 + 1)?;
    (offset < 32).then_some(offset as usize)
}

fn modal_active(app: &App) -> bool {
    app.ui_state.settings.is_some() || app.ui_state.modal.is_some()
}

/// 把 AccessKit 动作路由到现有产品路径。返回 `true` 表示已处理（调用方应刷新语义树）。
///
/// 规则：
/// - 模态打开时只接受当前 Dialog 内按钮的 Click；后台命令一律拒绝（不执行）。
/// - 菜单项 Click → 与 `menu_accept` 同源：关闭菜单后调用 `App::perform_menu`；
///   禁用项不执行。
/// - Focus 只报告已处理，由调用方写入焦点覆盖，不伪造视觉状态。
pub(crate) fn route_action(app: &mut App, action: Action, target: NodeId) -> bool {
    if modal_active(app) {
        if !matches!(action, Action::Click | Action::Focus) {
            return false;
        }
        if target == MODAL_ID {
            return action == Action::Focus; // 对话本身可获焦，但不能被“点击”执行
        }
        let Some(index) = modal_button_index(target) else {
            return false; // 后台命令拒绝
        };
        if action == Action::Focus {
            return true;
        }
        if let Some(settings) = &app.ui_state.settings {
            let Some(button) = settings.buttons.get(index) else {
                return false;
            };
            let id = button.id;
            app.resolve_settings(id);
            return true;
        }
        if let Some(modal) = &app.ui_state.modal {
            let Some(button) = modal.spec.buttons.get(index) else {
                return false;
            };
            let id = button.id;
            app.resolve_modal(id);
            return true;
        }
        return false;
    }

    match action {
        Action::Focus => {
            // 只接受本次树里存在的焦点目标；范围外的 stale id 忽略。
            target == ROOT_ID
                || target == BODY_ID
                || target == MENU_BAR_ID
                || target == OUTLINE_PANEL_ID
                || target == QUICK_OPEN_PANEL_ID
                || target == FIND_PANEL_ID
                || target == CONTEXT_MENU_ID
                || menu_item_from_node(target).is_some()
        }
        Action::Click => {
            let Some((menu, row)) = menu_item_from_node(target) else {
                // 正文/面板/菜单栏的 Click 无产品语义，消费掉即可。
                return true;
            };
            let Some((_, _, menu_action)) = menu_rows(app, menu).get(row).cloned() else {
                return false;
            };
            if matches!(menu_action, MenuAction::Separator) {
                return false;
            }
            if !chrome::action_enabled(menu_action, app.menu_flags()) {
                return true; // 禁用项：不执行
            }
            app.ui_state.chrome.close();
            app.perform_menu(menu_action);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app(text: &str) -> App {
        let store = crate::app_tests::MemStore::default();
        App::new(
            Doc {
                text: text.into(),
                path: None,
                bom: false,
                resume: false,
                open_error: None,
            },
            crate::app_tests::ports(&store, crate::app_tests::no_url()),
        )
    }

    fn node<'a>(tree: &'a TreeUpdate, id: NodeId) -> &'a Node {
        tree.nodes
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .map(|(_, node)| node)
            .unwrap_or_else(|| panic!("语义树缺少节点 {id:?}"))
    }

    fn children(tree: &TreeUpdate, id: NodeId) -> Vec<NodeId> {
        node(tree, id).children().to_vec()
    }

    /// 在菜单表中找到某动作对应的语义节点 id（同时验证 id 规划可逆）。
    fn node_for_action(app: &App, target: MenuAction) -> Option<NodeId> {
        for menu in 0..chrome::MENUS.len() {
            for (row, (_, _, action)) in menu_rows(app, menu).into_iter().enumerate() {
                if action == target {
                    return Some(NodeId(MENU_BASE[menu].0 + 1 + row as u64));
                }
            }
        }
        None
    }

    /// 菜单节点全部可达：四个顶层菜单常驻，每个菜单挂其全部非分隔线项；正文节点存在。
    #[test]
    fn tree_exposes_menu_bar_all_menus_items_and_body() {
        let app = test_app("# 标题\n\n正文内容。");
        let tree = build_semantic_tree(&app);
        let title = app.title_text();

        assert_eq!(node(&tree, ROOT_ID).role(), Role::Window);
        assert_eq!(node(&tree, ROOT_ID).label(), Some(title.as_str()));
        assert_eq!(children(&tree, ROOT_ID), vec![MENU_BAR_ID, BODY_ID]);

        assert_eq!(node(&tree, MENU_BAR_ID).role(), Role::MenuBar);
        let menus = children(&tree, MENU_BAR_ID);
        assert_eq!(menus, MENU_BASE.to_vec(), "四个顶层菜单必须常驻");

        for (menu_idx, menu_id) in MENU_BASE.iter().enumerate() {
            assert_eq!(node(&tree, *menu_id).role(), Role::Menu);
            assert_eq!(
                node(&tree, *menu_id).label(),
                Some(t(chrome::MENUS[menu_idx].title)),
                "菜单名必须与产品菜单表同源"
            );
            let items = children(&tree, *menu_id);
            assert!(!items.is_empty(), "菜单 {menu_idx} 必须有可导航项");
            let expected = menu_rows(&app, menu_idx)
                .iter()
                .filter(|(_, _, action)| !matches!(action, MenuAction::Separator))
                .count();
            assert_eq!(items.len(), expected, "所有非分隔线菜单项都必须暴露");
            for item in items {
                assert_eq!(node(&tree, item).role(), Role::MenuItem);
                assert!(node(&tree, item).supports_action(Action::Click));
                assert!(node(&tree, item).supports_action(Action::Focus));
            }
        }

        let body = node(&tree, BODY_ID);
        assert_eq!(body.role(), Role::MultilineTextInput);
        assert_eq!(body.label(), Some("正文"));
        assert!(
            body.value().is_some_and(|v| v.contains("正文内容")),
            "正文节点必须暴露有界文本快照"
        );
        assert_eq!(tree.focus, BODY_ID, "无菜单/模态时焦点落在正文");
    }

    /// 禁用态与 `chrome::action_enabled` 同源（空草稿 vs 有内容草稿）。
    #[test]
    fn disabled_state_matches_menu_flags() {
        for text in ["", "有内容的段落。"] {
            let app = test_app(text);
            let flags = app.menu_flags();
            let tree = build_semantic_tree(&app);
            let mut saw_enabled = false;
            let mut saw_disabled = false;
            for menu in 0..chrome::MENUS.len() {
                for (row, (_, _, action)) in menu_rows(&app, menu).into_iter().enumerate() {
                    if matches!(action, MenuAction::Separator) {
                        continue;
                    }
                    let id = NodeId(MENU_BASE[menu].0 + 1 + row as u64);
                    let expected = chrome::action_enabled(action, flags);
                    assert_eq!(
                        node(&tree, id).is_disabled(),
                        !expected,
                        "动作 {action:?} 的禁用态必须与 action_enabled 一致"
                    );
                    saw_enabled |= expected;
                    saw_disabled |= !expected;
                }
            }
            assert!(saw_enabled && saw_disabled, "样本必须同时覆盖启用与禁用");
        }
    }

    /// 模态打开：后台控件从树中移除，只保留 Dialog + 按钮，焦点在主按钮。
    #[test]
    fn modal_removes_background_controls_and_focuses_primary_button() {
        let mut app = test_app("有内容的段落。");
        app.show_about_modal();
        let tree = build_semantic_tree(&app);

        assert_eq!(children(&tree, ROOT_ID), vec![MODAL_ID]);
        let dialog = node(&tree, MODAL_ID);
        assert_eq!(dialog.role(), Role::Dialog);
        assert!(dialog.is_modal());
        assert!(
            !tree
                .nodes
                .iter()
                .any(|(id, _)| *id == MENU_BAR_ID || *id == BODY_ID),
            "模态期间菜单栏/正文不得留在语义树中"
        );

        let buttons = children(&tree, MODAL_ID);
        assert!(!buttons.is_empty());
        let modal = app.ui_state.modal.as_ref().expect("关于模态仍打开");
        let primary = modal.spec.primary_id().expect("模态必有主按钮");
        let index = modal
            .spec
            .buttons
            .iter()
            .position(|b| b.id == primary)
            .unwrap();
        assert_eq!(tree.focus, NodeId(MODAL_ID.0 + 1 + index as u64));
    }

    /// 菜单项 Click → 对应 `MenuAction`（关闭菜单后走现有 perform_menu）。
    #[test]
    fn menu_click_routes_to_menu_action() {
        let mut app = test_app("有内容的段落。");
        assert!(!app.ui_state.writing.focus);
        let target = node_for_action(&app, MenuAction::ToggleFocus).expect("菜单动作必须可定位");
        assert!(route_action(&mut app, Action::Click, target));
        assert!(app.ui_state.writing.focus, "Click 必须执行对应菜单动作");
    }

    /// 禁用项 Click 不执行产品动作。
    #[test]
    fn disabled_menu_click_is_not_executed() {
        let mut app = test_app(""); // 空文档：导出禁用
        let before = app.ui_state.chrome.notice.clone();
        let target = node_for_action(&app, MenuAction::Export).expect("导出项存在");
        let tree = build_semantic_tree(&app);
        assert!(node(&tree, target).is_disabled());
        assert!(route_action(&mut app, Action::Click, target));
        assert_eq!(
            app.ui_state.chrome.notice, before,
            "禁用项不得执行（执行会走空文档提示路径）"
        );
    }

    /// 模态按钮 Click → 现有 resolve_modal 路径（关闭模态）。
    #[test]
    fn modal_button_click_uses_resolve_modal() {
        let mut app = test_app("有内容的段落。");
        app.show_about_modal();
        let modal = app.ui_state.modal.as_ref().expect("关于模态打开");
        let index = modal
            .spec
            .buttons
            .iter()
            .position(|b| b.id == 2)
            .expect("关于模态有关闭按钮");
        let target = NodeId(MODAL_ID.0 + 1 + index as u64);
        assert!(route_action(&mut app, Action::Click, target));
        assert!(app.ui_state.modal.is_none(), "关闭按钮必须走 resolve_modal");
    }

    /// 模态打开时拒绝后台菜单命令。
    #[test]
    fn modal_rejects_background_menu_command() {
        let mut app = test_app("有内容的段落。");
        app.show_about_modal();
        let target = node_for_action(&app, MenuAction::ToggleFocus).expect("菜单动作存在");
        assert!(!route_action(&mut app, Action::Click, target));
        assert!(!app.ui_state.writing.focus, "后台命令不得执行");
        assert!(app.ui_state.modal.is_some(), "模态必须保持打开");
    }

    /// Focus 只更新语义焦点：合法目标覆盖生效，stale id 回退计算焦点。
    #[test]
    fn focus_action_updates_tree_focus_only() {
        let app = test_app("有内容的段落。");
        let tree = build_semantic_tree_with_focus(&app, Some(MENU_BAR_ID));
        assert_eq!(tree.focus, MENU_BAR_ID);
        let stale = build_semantic_tree_with_focus(&app, Some(NodeId(999_999)));
        assert_eq!(stale.focus, BODY_ID);
    }

    /// 快速打开与命令面板使用不同 name。
    #[test]
    fn quick_open_and_command_palette_are_distinguishable() {
        let mut app = test_app("x");
        app.ui_state.quick.open = true;
        app.ui_state.quick.kind = crate::view::quick_open::QuickKind::Files;
        let files = build_semantic_tree(&app);
        assert_eq!(
            node(&files, QUICK_OPEN_PANEL_ID).label(),
            Some(t(Key::MenuFileQuickOpen))
        );

        app.ui_state.quick.kind = crate::view::quick_open::QuickKind::Commands;
        let commands = build_semantic_tree(&app);
        assert_eq!(
            node(&commands, QUICK_OPEN_PANEL_ID).label(),
            Some("命令面板")
        );
    }
}
