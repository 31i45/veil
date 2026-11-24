//! AccessKit 无障碍支持：mixed handlers + chrome 最小语义树。
//!
//! 整体架构（参照 spike `accesskit-spike/src/main.rs`）：
//! - `ShellEvent::{Doc(Doc), AccessKit(AccessKitEvent)}` 统一用户事件类型
//! - 每个窗口在隐藏阶段创建 `accesskit_winit::Adapter`
//! - mixed handler：同步 `ActivationHandler` 返回线程安全快照；
//!   action / deactivation 由 accesskit_winit 内部自动走 proxy 回主线程
//! - `Shell::window_event` 在应用 match 前调用 `adapter.process_event(...)`
//!
//! Task 5 范围：窗口 / 菜单栏 / 菜单 / 模态 / 面板 / 右键菜单。
//! 正文 caret / selection / 文本动作归 Task 6。

#![cfg(feature = "accessibility")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::ThreadId;

use accesskit::{Action, ActivationHandler, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use accesskit_winit::Event as AccessKitEvent;
use winit::window::WindowId;

// —— Node ID 规划（Task 5 范围，u64）—— //
pub const ROOT_ID: NodeId = NodeId(1);
pub const MENU_BAR_ID: NodeId = NodeId(100);
// 子菜单基址：MENU_FILE=200, MENU_EDIT=300, MENU_VIEW=400, MENU_HELP=500
pub const MENU_FILE_ID: NodeId = NodeId(200);
pub const MENU_EDIT_ID: NodeId = NodeId(300);
pub const MENU_VIEW_ID: NodeId = NodeId(400);
pub const MENU_HELP_ID: NodeId = NodeId(500);

pub const MODAL_ID: NodeId = NodeId(10_000);
pub const CONTEXT_MENU_ID: NodeId = NodeId(20_000);

// 大纲/快速打开/查找面板基址
pub const OUTLINE_PANEL_ID: NodeId = NodeId(30_000);
pub const QUICK_OPEN_PANEL_ID: NodeId = NodeId(31_000);
pub const FIND_PANEL_ID: NodeId = NodeId(32_000);

/// Shell 用户事件枚举：把外部文档请求与 AccessKit 事件合并成单一事件循环类型。
pub enum ShellEvent {
    Doc(crate::app::Doc),
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

impl From<crate::app::Doc> for ShellEvent {
    fn from(doc: crate::app::Doc) -> Self {
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

/// 根据当前 App 状态构建最小语义树（Task 5 范围）。
pub(crate) fn build_semantic_tree(app: &crate::app::App) -> TreeUpdate {
    let mut nodes: Vec<(NodeId, Node)> = Vec::new();

    // 根节点 = Window
    let mut root = Node::new(Role::Window);
    root.set_label(app.title_text());
    root.add_action(Action::Focus);
    nodes.push((ROOT_ID, root));

    let mut children: Vec<NodeId> = Vec::new();

    // —— 菜单栏始终可见 —— //
    let mut menu_bar = Node::new(Role::MenuBar);
    menu_bar.set_label("主菜单栏");
    nodes.push((MENU_BAR_ID, menu_bar));
    children.push(MENU_BAR_ID);

    // —— 当前打开的顶层菜单 —— //
    if let Some(idx) = app.ui_state.chrome.open {
        if idx < crate::chrome::MENUS.len() {
            let menu_id = match idx {
                0 => MENU_FILE_ID,
                1 => MENU_EDIT_ID,
                2 => MENU_VIEW_ID,
                3 => MENU_HELP_ID,
                _ => MENU_FILE_ID,
            };
            children.push(menu_id);

            let def = &crate::chrome::MENUS[idx];
            let mut menu_node = Node::new(Role::Menu);
            menu_node.set_label(crate::i18n::t(def.title).to_string());
            let mut item_children: Vec<NodeId> = Vec::new();
            for (i, (label_key, shortcut_key, action)) in def.items.iter().enumerate() {
                if *action == crate::chrome::MenuAction::Separator {
                    continue;
                }
                let item_id = NodeId(menu_id.0 + 1 + i as u64);
                let mut item = Node::new(Role::MenuItem);
                let label_text = crate::i18n::t(*label_key).to_string();
                item.set_label(label_text);
                item.add_action(Action::Click);
                item.add_action(Action::Focus);
                // 禁用态：Task 5 先用 Enabled=true 占位，Task 6 细化状态查询。
                // 快捷键
                let sc = crate::i18n::t(*shortcut_key);
                if !sc.is_empty() && *shortcut_key != crate::i18n::Key::Empty {
                    item.set_keyboard_shortcut(sc.to_string());
                }
                nodes.push((item_id, item));
                item_children.push(item_id);
            }
            menu_node.set_children(item_children);
            nodes.push((menu_id, menu_node));
        }
    }

    // —— 模态框优先于其他面板 —— //
    if app.ui_state.settings.is_some() {
        let mut dialog = Node::new(Role::Dialog);
        dialog.set_label("设置");
        dialog.set_modal();
        nodes.push((MODAL_ID, dialog));
        children.push(MODAL_ID);
    } else if let Some(modal) = &app.ui_state.modal {
        let mut dialog = Node::new(Role::Dialog);
        dialog.set_label(modal.spec.title.clone());
        dialog.set_modal();
        let mut buttons: Vec<NodeId> = Vec::new();
        for (i, btn) in modal.spec.buttons.iter().enumerate() {
            let bid = NodeId(MODAL_ID.0 + 1 + i as u64);
            let mut bnode = Node::new(Role::Button);
            bnode.set_label(btn.label.clone());
            bnode.add_action(Action::Click);
            bnode.add_action(Action::Focus);
            nodes.push((bid, bnode));
            buttons.push(bid);
        }
        dialog.set_children(buttons);
        nodes.push((MODAL_ID, dialog));
        children.push(MODAL_ID);
    } else {
        // 非模态状态：面板（查找/大纲/快速打开）
        if app.ui_state.search.open {
            let mut panel = Node::new(Role::Group);
            panel.set_label("查找");
            panel.add_action(Action::Focus);
            nodes.push((FIND_PANEL_ID, panel));
            children.push(FIND_PANEL_ID);
        }
        if app.ui_state.outline.open {
            let mut panel = Node::new(Role::Group);
            panel.set_label("文档大纲");
            panel.add_action(Action::Focus);
            nodes.push((OUTLINE_PANEL_ID, panel));
            children.push(OUTLINE_PANEL_ID);
        }
        if app.ui_state.quick.open {
            let mut panel = Node::new(Role::Group);
            panel.set_label("快速打开");
            panel.add_action(Action::Focus);
            nodes.push((QUICK_OPEN_PANEL_ID, panel));
            children.push(QUICK_OPEN_PANEL_ID);
        }
    }

    // —— 右键菜单 —— //
    if app.ui_state.ctx.open {
        let mut menu = Node::new(Role::Menu);
        menu.set_label("上下文菜单");
        nodes.push((CONTEXT_MENU_ID, menu));
        children.push(CONTEXT_MENU_ID);
    }

    // 回填根节点 children（nodes[0] 是 root）
    if let Some((_, ref mut root_node)) = nodes.get_mut(0) {
        root_node.set_children(children);
    }

    // 焦点：模态 → 模态主按钮，菜单 → 菜单项，否则 root
    let focus = if let Some(modal) = &app.ui_state.modal {
        let primary = modal.spec.primary_id();
        primary
            .and_then(|pid| modal.spec.buttons.iter().position(|b| b.id == pid))
            .map(|i| NodeId(MODAL_ID.0 + 1 + i as u64))
            .unwrap_or(MODAL_ID)
    } else if app.ui_state.settings.is_some() {
        MODAL_ID
    } else {
        ROOT_ID
    };

    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(ROOT_ID)),
        tree_id: TreeId::ROOT,
        focus,
    }
}

// —— 确定性测试 —— //
// 测试放在 shell.rs 的 tests 模块里（已有完整 App fixture），受 `#[cfg(feature = "accessibility")]` 保护。
