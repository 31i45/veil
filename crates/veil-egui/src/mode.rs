//! 交互模式：按键路由的唯一实现点（ADR-009 决策二）。
//!
//! `Mode` 是 `chrome.open` / `search.open` 既有状态的派生投影，不新增真值；
//! 优先级只在这里表达一次，事件回调不再各自判断。
//! 大纲是常驻面板而非模态：面板打开时键盘仍归文档，故不进入 `Mode`。

use crate::app::App;
use crate::input::{translate, Action, Key, Mods};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Document,
    /// 菜单展开：只响应 Esc，其余按键吞掉（避免误编辑）。
    Menu,
    /// 快速打开浮层：文档只读，按键归过滤串与选中项。
    QuickOpen,
    /// 查找栏打开：文档只读，按键归查询串。
    Search,
}

/// IME 文本落点。组合串与提交文本共用同一判定，避免两处各写一遍。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TextSink {
    Doc,
    Query,
    /// 快速打开浮层的过滤串。
    Quick,
}

/// 路由产物：只描述「该做什么」，执行留在 `interaction`。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Cmd {
    /// 本模式下该键无效：不产生任何副作用。
    Ignore,
    /// 打开快速打开浮层（查找栏打开时也允许 Ctrl+P 抢占）。
    OpenQuick,
    CloseMenu,
    CloseSearch,
    QuickClose,
    /// 回车：打开当前选中项。
    QuickAccept,
    /// 上下移动选中项。
    QuickMove(i32),
    QuickBackspace,
    QuickInput(String),
    SearchInput(String),
    SearchBackspace,
    /// 替换模式下 Tab 在查询/替换输入框之间切换焦点。
    SearchToggleField,
    SearchNext,
    SearchPrev,
    /// 文档级动作（含纯视图动作），交 `perform_action` 执行。
    Edit(Action),
}

/// 按键路由：纯函数，无副作用、可穷举测试。
/// `composing` = IME 组合期：查找栏的全部按键让给 IME，文档路径不受影响
/// （winit 组合期的文本键不经 `commit_ime`，仍按既有路径处理）。
pub(crate) fn route(mode: Mode, key: Key, mods: Mods, composing: bool) -> Cmd {
    match mode {
        Mode::Menu => {
            if key == Key::Escape {
                Cmd::CloseMenu
            } else {
                Cmd::Ignore
            }
        }
        Mode::QuickOpen => {
            if composing {
                return Cmd::Ignore;
            }
            match key {
                Key::Escape => Cmd::QuickClose,
                Key::Enter => Cmd::QuickAccept,
                Key::Up => Cmd::QuickMove(-1),
                Key::Down => Cmd::QuickMove(1),
                Key::Backspace => Cmd::QuickBackspace,
                Key::Text(t) if !mods.ctrl => Cmd::QuickInput(t),
                _ => Cmd::Ignore,
            }
        }
        Mode::Search => {
            if composing {
                return Cmd::Ignore;
            }
            match key {
                Key::Escape => Cmd::CloseSearch,
                Key::Enter if mods.shift => Cmd::SearchPrev,
                Key::Enter => Cmd::SearchNext,
                Key::Backspace => Cmd::SearchBackspace,
                Key::Tab => Cmd::SearchToggleField,
                Key::P if mods.ctrl => Cmd::OpenQuick,
                Key::Text(t) if !mods.ctrl => Cmd::SearchInput(t),
                _ => Cmd::Ignore,
            }
        }
        Mode::Document => match translate(key, mods) {
            Some(action) => Cmd::Edit(action),
            None => Cmd::Ignore,
        },
    }
}

impl App {
    /// 当前模式：唯一优先级实现（菜单/右键菜单 > 快速打开 > 查找栏 > 文档）。
    pub(crate) fn mode(&self) -> Mode {
        if self.ui_state.chrome.is_open() || self.ui_state.ctx.open {
            Mode::Menu
        } else if self.ui_state.quick.open {
            Mode::QuickOpen
        } else if self.ui_state.search.open {
            Mode::Search
        } else {
            Mode::Document
        }
    }

    /// IME 文本落点：菜单打开时提交被吞掉，故落点按 `Doc` 处理。
    pub(crate) fn text_sink(&self) -> TextSink {
        match self.mode() {
            Mode::QuickOpen => TextSink::Quick,
            Mode::Search => TextSink::Query,
            Mode::Menu | Mode::Document => TextSink::Doc,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
    };
    const CTRL: Mods = Mods {
        ctrl: true,
        shift: false,
    };
    const SHIFT: Mods = Mods {
        ctrl: false,
        shift: true,
    };
    const CTRL_SHIFT: Mods = Mods {
        ctrl: true,
        shift: true,
    };

    fn text(s: &str) -> Key {
        Key::Text(s.into())
    }

    #[test]
    fn document_mode_delegates_to_translate() {
        assert_eq!(
            route(Mode::Document, text("a"), NONE, false),
            Cmd::Edit(Action::Doc(crate::editor::Intent::InsertText("a".into())))
        );
        assert_eq!(
            route(Mode::Document, Key::F, CTRL, false),
            Cmd::Edit(Action::Find)
        );
        // translate 未绑定 → 无副作用
        assert_eq!(route(Mode::Document, text("v"), CTRL, false), Cmd::Ignore);
        assert_eq!(route(Mode::Document, Key::F, NONE, false), Cmd::Ignore);
        // 文档路径不受组合串影响（组合期文本键不走提交）
        assert_eq!(
            route(Mode::Document, text("a"), NONE, true),
            Cmd::Edit(Action::Doc(crate::editor::Intent::InsertText("a".into())))
        );
    }

    #[test]
    fn menu_mode_swallows_everything_but_escape() {
        assert_eq!(route(Mode::Menu, Key::Escape, NONE, false), Cmd::CloseMenu);
        for key in [
            Key::Enter,
            Key::Backspace,
            Key::Left,
            Key::F,
            text("a"),
            text("中"),
        ] {
            assert_eq!(route(Mode::Menu, key.clone(), NONE, false), Cmd::Ignore);
            assert_eq!(route(Mode::Menu, key, CTRL_SHIFT, false), Cmd::Ignore);
        }
    }

    #[test]
    fn search_mode_routing_table() {
        assert_eq!(
            route(Mode::Search, Key::Escape, NONE, false),
            Cmd::CloseSearch
        );
        assert_eq!(
            route(Mode::Search, Key::Enter, NONE, false),
            Cmd::SearchNext
        );
        assert_eq!(
            route(Mode::Search, Key::Enter, SHIFT, false),
            Cmd::SearchPrev
        );
        assert_eq!(
            route(Mode::Search, Key::Backspace, NONE, false),
            Cmd::SearchBackspace
        );
        assert_eq!(
            route(Mode::Search, Key::Tab, NONE, false),
            Cmd::SearchToggleField,
            "Tab 切查询/替换焦点"
        );
        assert_eq!(
            route(Mode::Search, text("查"), NONE, false),
            Cmd::SearchInput("查".into())
        );
        // Ctrl+文本 / 未绑定键：吞掉，不得落进查询串
        assert_eq!(route(Mode::Search, text("f"), CTRL, false), Cmd::Ignore);
        assert_eq!(route(Mode::Search, Key::F, CTRL, false), Cmd::Ignore);
        assert_eq!(route(Mode::Search, Key::Left, NONE, false), Cmd::Ignore);
    }

    #[test]
    fn quick_open_mode_routing_table() {
        assert_eq!(
            route(Mode::QuickOpen, Key::Escape, NONE, false),
            Cmd::QuickClose
        );
        assert_eq!(
            route(Mode::QuickOpen, Key::Enter, NONE, false),
            Cmd::QuickAccept
        );
        assert_eq!(
            route(Mode::QuickOpen, Key::Up, NONE, false),
            Cmd::QuickMove(-1)
        );
        assert_eq!(
            route(Mode::QuickOpen, Key::Down, NONE, false),
            Cmd::QuickMove(1)
        );
        assert_eq!(
            route(Mode::QuickOpen, Key::Backspace, NONE, false),
            Cmd::QuickBackspace
        );
        assert_eq!(
            route(Mode::QuickOpen, text("md"), NONE, false),
            Cmd::QuickInput("md".into())
        );
        // Ctrl+文本 / 方向键 / Tab：吞掉，不落过滤串
        assert_eq!(route(Mode::QuickOpen, text("p"), CTRL, false), Cmd::Ignore);
        assert_eq!(route(Mode::QuickOpen, Key::Left, NONE, false), Cmd::Ignore);
        assert_eq!(route(Mode::QuickOpen, Key::Tab, NONE, false), Cmd::Ignore);
        // 组合期让给 IME
        assert_eq!(route(Mode::QuickOpen, Key::Enter, NONE, true), Cmd::Ignore);
    }

    #[test]
    fn search_mode_yields_all_keys_while_composing() {
        for key in [Key::Escape, Key::Enter, Key::Backspace, text("a")] {
            assert_eq!(route(Mode::Search, key, NONE, true), Cmd::Ignore);
        }
    }

    #[test]
    fn three_modes_are_pairwise_distinct_on_same_key() {
        // 同一个 Esc 在三种模式下的产物不同：优先级只由 mode 决定
        assert_eq!(
            route(Mode::Document, Key::Escape, NONE, false),
            Cmd::Edit(Action::CloseMenu)
        );
        assert_eq!(route(Mode::Menu, Key::Escape, NONE, false), Cmd::CloseMenu);
        assert_eq!(
            route(Mode::Search, Key::Escape, NONE, false),
            Cmd::CloseSearch
        );
    }
}
