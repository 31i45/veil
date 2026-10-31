//! 键盘事件 → 意图翻译。GUI 无关：`Key` 由 shell 从 winit 事件构造，此处不依赖 winit。
//!
//! Ctrl+←/→ 按词移动、Ctrl+Shift+←/→ 按词扩选（C6）；词边界规则在 editor 层。

use crate::editor::Intent;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Key {
    /// 可打印文本（winit `KeyEvent.text`；ctrl 组合键时 shell 不产生此变体）。
    Text(String),
    /// 逻辑键 z/y（无论是否带 ctrl；用于 Ctrl+Z/Y 与文本分流）。
    Z,
    Y,
    /// 逻辑键 c/x/v（无论是否带 ctrl；用于 Ctrl+C/X/V 与文本分流）。
    C,
    X,
    V,
    /// 逻辑键 s（无论是否带 ctrl；用于 Ctrl+S 保存与文本分流）。
    S,
    /// 逻辑键 o（无论是否带 ctrl；用于 Ctrl+O 打开）。
    O,
    /// 逻辑键 e（无论是否带 ctrl；用于 Ctrl+E 导出与文本分流）。
    E,
    /// 逻辑键 f（无论是否带 ctrl；用于 Ctrl+F 查找与文本分流）。
    F,
    /// 逻辑键 h（无论是否带 ctrl；用于 Ctrl+H 替换与文本分流）。
    H,
    /// 逻辑键 a（无论是否带 ctrl；用于 Ctrl+A 全选与文本分流）。
    A,
    /// 逻辑键 n（无论是否带 ctrl；用于 Ctrl+N 新建与文本分流）。
    N,
    /// 逻辑键 p（无论是否带 ctrl；用于 Ctrl+P 快速打开与文本分流）。
    P,
    /// 逻辑键 b/i/k（无论是否带 ctrl；用于 Ctrl+B/I/K 行内格式与文本分流，3c-1）。
    B,
    I,
    K,
    /// 逻辑键 0/-/=（无论是否带 ctrl；用于 Ctrl+0/± 视图缩放与文本分流，3c-2）。
    Digit0,
    Minus,
    Equal,
    /// Escape：关闭菜单（切片 15）。
    Escape,
    /// F11：全屏切换（3c-3；不需要修饰键）。
    F11,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    Enter,
    /// Tab：缩进（Shift 按住为反缩进）。
    Tab,
}

/// 翻译产物：文档级交 `Editor::apply`；视图级由 shell 经 layout 换算。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    Doc(Intent),
    /// 垂直移动 ±n 视觉行（goal-column 语义在 shell 维护）。
    MoveVertical(i32),
    /// 垂直扩选 ±n 视觉行（Shift+↑/↓；落点换算同 MoveVertical，仅保留 anchor）。
    SelectVertical(i32),
    MoveLineStart,
    MoveLineEnd,
    /// Shift+Home/End：视觉行首尾扩选（落点换算同 MoveLineStart/End，仅保留 anchor）。
    SelectLineStart,
    SelectLineEnd,
    /// 剪贴板：复制/剪切/粘贴。系统读写由 shell 经 `Clipboard` trait 完成（此处不产文档意图）。
    Copy,
    Cut,
    Paste,
    /// 保存（Ctrl+S）：文件读写由 shell 经 `FileStore` trait 完成（此处不产文档意图）。
    Save,
    /// 另存为（Ctrl+Shift+S）：路径由 shell 经 `FileDialog` 选择。
    SaveAs,
    /// 打开（Ctrl+O）：路径由 shell 经 `FileDialog` 选择。
    Open,
    /// 新建（Ctrl+N）：以空白草稿替换当前文档（脏文档拒绝）。
    New,
    /// 快速打开（Ctrl+P）：浮层过滤最近文件与当前目录 Markdown 文件。
    QuickOpen,
    /// 关闭菜单（Esc）：纯视图动作。
    CloseMenu,
    /// 切换大纲面板（Ctrl+Shift+O）：纯视图动作。
    ToggleOutline,
    /// 导出 HTML（Ctrl+E）：由 shell 经 `Exporter` trait 完成（此处不产文档意图）。
    Export,
    /// 打开文档内查找栏（Ctrl+F）：纯视图动作。
    Find,
    /// 打开查找栏并展开替换行（Ctrl+H）：纯视图动作。
    Replace,
    /// 视图缩放（3c-2）：放大/缩小一档（0.1）、还原 100%。纯视图动作，不动文档。
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// 全屏切换（3c-3，F11）：无边框全屏；窗口操作由 shell 完成。纯视图动作。
    ToggleFullscreen,
}

pub fn translate(key: Key, mods: Mods) -> Option<Action> {
    match key {
        Key::Text(t) => {
            if mods.ctrl {
                None // Ctrl+可打印键：未绑定的快捷键，不插入文本
            } else {
                Some(Action::Doc(Intent::InsertText(t)))
            }
        }
        Key::Z if mods.ctrl && mods.shift => Some(Action::Doc(Intent::Redo)),
        Key::Z if mods.ctrl => Some(Action::Doc(Intent::Undo)),
        Key::Y if mods.ctrl => Some(Action::Doc(Intent::Redo)),
        Key::Z | Key::Y => None, // 无 ctrl 的 z/y 文本由 Text 变体承载
        Key::C if mods.ctrl => Some(Action::Copy),
        Key::X if mods.ctrl => Some(Action::Cut),
        Key::V if mods.ctrl => Some(Action::Paste),
        Key::C | Key::X | Key::V => None, // 无 ctrl 的 c/x/v 文本由 Text 变体承载
        Key::S if mods.ctrl && mods.shift => Some(Action::SaveAs),
        Key::S if mods.ctrl => Some(Action::Save),
        Key::S => None, // 无 ctrl 的 s 文本由 Text 变体承载
        Key::O if mods.ctrl && mods.shift => Some(Action::ToggleOutline),
        Key::O if mods.ctrl => Some(Action::Open),
        Key::O => None, // 无 ctrl 的 o 文本由 Text 变体承载
        Key::Escape => Some(Action::CloseMenu),
        Key::F11 => Some(Action::ToggleFullscreen), // 全屏切换不需要修饰键（3c-3）
        Key::E if mods.ctrl => Some(Action::Export),
        Key::E => None, // 无 ctrl 的 e 文本由 Text 变体承载
        Key::F if mods.ctrl => Some(Action::Find),
        Key::F => None, // 无 ctrl 的 f 文本由 Text 变体承载
        Key::H if mods.ctrl => Some(Action::Replace),
        Key::H => None, // 无 ctrl 的 h 文本由 Text 变体承载
        Key::A if mods.ctrl => Some(Action::Doc(Intent::SelectAll)),
        Key::A => None, // 无 ctrl 的 a 文本由 Text 变体承载
        Key::N if mods.ctrl => Some(Action::New),
        Key::N => None, // 无 ctrl 的 n 文本由 Text 变体承载
        Key::P if mods.ctrl => Some(Action::QuickOpen),
        Key::P => None, // 无 ctrl 的 p 文本由 Text 变体承载
        Key::B if mods.ctrl => Some(Action::Doc(Intent::ToggleWrap("**"))),
        Key::B => None, // 无 ctrl 的 b 文本由 Text 变体承载
        Key::I if mods.ctrl => Some(Action::Doc(Intent::ToggleWrap("*"))),
        Key::I => None, // 无 ctrl 的 i 文本由 Text 变体承载
        Key::K if mods.ctrl => Some(Action::Doc(Intent::InsertLink)),
        Key::K => None, // 无 ctrl 的 k 文本由 Text 变体承载
        Key::Equal if mods.ctrl => Some(Action::ZoomIn),
        Key::Minus if mods.ctrl => Some(Action::ZoomOut),
        Key::Digit0 if mods.ctrl => Some(Action::ZoomReset),
        Key::Digit0 | Key::Minus | Key::Equal => None, // 无 ctrl 的符号文本由 Text 变体承载
        // Ctrl+Shift+←/→ 按词扩选；Ctrl+←/→ 按词移动；Shift+方向为字符级扩选。
        Key::Left if mods.ctrl && mods.shift => Some(Action::Doc(Intent::SelectWordLeft)),
        Key::Right if mods.ctrl && mods.shift => Some(Action::Doc(Intent::SelectWordRight)),
        Key::Left if mods.ctrl => Some(Action::Doc(Intent::WordLeft)),
        Key::Right if mods.ctrl => Some(Action::Doc(Intent::WordRight)),
        Key::Left if mods.shift => Some(Action::Doc(Intent::SelectLeft)),
        Key::Right if mods.shift => Some(Action::Doc(Intent::SelectRight)),
        Key::Left => Some(Action::Doc(Intent::MoveLeft)),
        Key::Right => Some(Action::Doc(Intent::MoveRight)),
        Key::Up if mods.shift => Some(Action::SelectVertical(-1)),
        Key::Down if mods.shift => Some(Action::SelectVertical(1)),
        Key::Up => Some(Action::MoveVertical(-1)),
        Key::Down => Some(Action::MoveVertical(1)),
        Key::Home if mods.ctrl && mods.shift => Some(Action::Doc(Intent::SelectDocStart)),
        Key::End if mods.ctrl && mods.shift => Some(Action::Doc(Intent::SelectDocEnd)),
        Key::Home if mods.ctrl => Some(Action::Doc(Intent::MoveDocStart)),
        Key::End if mods.ctrl => Some(Action::Doc(Intent::MoveDocEnd)),
        Key::Home if mods.shift => Some(Action::SelectLineStart),
        Key::End if mods.shift => Some(Action::SelectLineEnd),
        Key::Home => Some(Action::MoveLineStart),
        Key::End => Some(Action::MoveLineEnd),
        Key::Backspace => Some(Action::Doc(Intent::DeleteBackward)),
        Key::Delete => Some(Action::Doc(Intent::DeleteForward)),
        Key::Enter => Some(Action::Doc(Intent::Newline)),
        Key::Tab if mods.shift => Some(Action::Doc(Intent::Dedent)),
        Key::Tab => Some(Action::Doc(Intent::Indent)),
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
    const CTRL_SHIFT: Mods = Mods {
        ctrl: true,
        shift: true,
    };

    fn text(s: &str) -> Key {
        Key::Text(s.into())
    }

    #[test]
    fn plain_text_inserts() {
        assert_eq!(
            translate(text("a"), NONE),
            Some(Action::Doc(Intent::InsertText("a".into())))
        );
        assert_eq!(
            translate(text("中"), NONE),
            Some(Action::Doc(Intent::InsertText("中".into())))
        );
    }

    #[test]
    fn ctrl_text_is_swallowed_not_inserted() {
        // 未知快捷键不得降级为文本插入（防 Ctrl+V 等入文档，粘贴属后续切片）
        assert_eq!(translate(text("v"), CTRL), None);
    }

    #[test]
    fn zoom_bindings() {
        // 3c-2：Ctrl+=/- 缩放档位、Ctrl+0 还原；无 ctrl 时不吞键（文本走 Text 变体）
        assert_eq!(translate(Key::Equal, CTRL), Some(Action::ZoomIn));
        assert_eq!(translate(Key::Minus, CTRL), Some(Action::ZoomOut));
        assert_eq!(translate(Key::Digit0, CTRL), Some(Action::ZoomReset));
        assert_eq!(translate(Key::Equal, NONE), None);
        assert_eq!(translate(Key::Minus, NONE), None);
        assert_eq!(translate(Key::Digit0, NONE), None);
        // Ctrl+Shift+=（部分键盘「+」的主键 =）仍按放大处理：shift 不影响
        assert_eq!(translate(Key::Equal, CTRL_SHIFT), Some(Action::ZoomIn));
    }

    #[test]
    fn fullscreen_binding_needs_no_mods() {
        // 3c-3：F11 全屏切换不依赖修饰键；Esc 仍是关菜单而非退全屏
        assert_eq!(translate(Key::F11, NONE), Some(Action::ToggleFullscreen));
        assert_eq!(translate(Key::F11, CTRL), Some(Action::ToggleFullscreen));
        assert_eq!(translate(Key::Escape, NONE), Some(Action::CloseMenu));
    }

    #[test]
    fn undo_redo_bindings() {
        assert_eq!(translate(Key::Z, CTRL), Some(Action::Doc(Intent::Undo)));
        assert_eq!(
            translate(Key::Z, CTRL_SHIFT),
            Some(Action::Doc(Intent::Redo))
        );
        assert_eq!(translate(Key::Y, CTRL), Some(Action::Doc(Intent::Redo)));
        assert_eq!(translate(Key::Z, NONE), None);
        assert_eq!(translate(Key::Y, NONE), None);
    }

    #[test]
    fn arrows_and_editing_keys() {
        assert_eq!(
            translate(Key::Left, NONE),
            Some(Action::Doc(Intent::MoveLeft))
        );
        assert_eq!(
            translate(Key::Right, NONE),
            Some(Action::Doc(Intent::MoveRight))
        );
        assert_eq!(translate(Key::Up, NONE), Some(Action::MoveVertical(-1)));
        assert_eq!(translate(Key::Down, NONE), Some(Action::MoveVertical(1)));
        assert_eq!(
            translate(Key::Backspace, NONE),
            Some(Action::Doc(Intent::DeleteBackward))
        );
        assert_eq!(
            translate(Key::Delete, NONE),
            Some(Action::Doc(Intent::DeleteForward))
        );
        assert_eq!(
            translate(Key::Enter, NONE),
            Some(Action::Doc(Intent::Newline))
        );
    }

    #[test]
    fn home_end_split_view_and_doc() {
        assert_eq!(translate(Key::Home, NONE), Some(Action::MoveLineStart));
        assert_eq!(translate(Key::End, NONE), Some(Action::MoveLineEnd));
        assert_eq!(
            translate(Key::Home, CTRL),
            Some(Action::Doc(Intent::MoveDocStart))
        );
        assert_eq!(
            translate(Key::End, CTRL),
            Some(Action::Doc(Intent::MoveDocEnd))
        );
    }

    #[test]
    fn shift_arrows_select() {
        let shift = Mods {
            ctrl: false,
            shift: true,
        };
        assert_eq!(
            translate(Key::Left, shift),
            Some(Action::Doc(Intent::SelectLeft))
        );
        assert_eq!(
            translate(Key::Right, shift),
            Some(Action::Doc(Intent::SelectRight))
        );
    }

    #[test]
    fn ctrl_arrows_move_and_select_by_word() {
        assert_eq!(
            translate(Key::Left, CTRL),
            Some(Action::Doc(Intent::WordLeft))
        );
        assert_eq!(
            translate(Key::Right, CTRL),
            Some(Action::Doc(Intent::WordRight))
        );
        assert_eq!(
            translate(Key::Left, CTRL_SHIFT),
            Some(Action::Doc(Intent::SelectWordLeft))
        );
        assert_eq!(
            translate(Key::Right, CTRL_SHIFT),
            Some(Action::Doc(Intent::SelectWordRight))
        );
        // Ctrl 优先于 Shift：不带 Ctrl 时仍是字符级扩选
        assert_eq!(
            translate(
                Key::Left,
                Mods {
                    ctrl: false,
                    shift: true
                }
            ),
            Some(Action::Doc(Intent::SelectLeft))
        );
    }

    #[test]
    fn shift_vertical_selects() {
        let shift = Mods {
            ctrl: false,
            shift: true,
        };
        assert_eq!(translate(Key::Up, shift), Some(Action::SelectVertical(-1)));
        assert_eq!(translate(Key::Down, shift), Some(Action::SelectVertical(1)));
    }

    #[test]
    fn ctrl_shift_home_end_selects_doc_bounds() {
        let cs = Mods {
            ctrl: true,
            shift: true,
        };
        assert_eq!(
            translate(Key::Home, cs),
            Some(Action::Doc(Intent::SelectDocStart))
        );
        assert_eq!(
            translate(Key::End, cs),
            Some(Action::Doc(Intent::SelectDocEnd))
        );
    }

    #[test]
    fn shift_home_end_selects_visual_line_edges() {
        let shift = Mods {
            ctrl: false,
            shift: true,
        };
        assert_eq!(translate(Key::Home, shift), Some(Action::SelectLineStart));
        assert_eq!(translate(Key::End, shift), Some(Action::SelectLineEnd));
    }

    #[test]
    fn clipboard_bindings() {
        assert_eq!(translate(Key::C, CTRL), Some(Action::Copy));
        assert_eq!(translate(Key::X, CTRL), Some(Action::Cut));
        assert_eq!(translate(Key::V, CTRL), Some(Action::Paste));
        // 无 ctrl 的 c/x/v 文本由 Text 变体承载，字母变体不产意图
        assert_eq!(translate(Key::C, NONE), None);
        assert_eq!(translate(Key::X, NONE), None);
        assert_eq!(translate(Key::V, NONE), None);
    }

    #[test]
    fn save_binding() {
        assert_eq!(translate(Key::S, CTRL), Some(Action::Save));
        assert_eq!(translate(Key::S, NONE), None); // 无 ctrl 的 s 由 Text 变体承载
    }

    #[test]
    fn save_as_and_open_bindings() {
        assert_eq!(translate(Key::S, CTRL_SHIFT), Some(Action::SaveAs));
        assert_eq!(translate(Key::O, CTRL), Some(Action::Open));
        assert_eq!(translate(Key::O, NONE), None); // 无 ctrl 的 o 由 Text 变体承载
        assert_eq!(translate(Key::Escape, NONE), Some(Action::CloseMenu));
    }

    #[test]
    fn outline_toggle_binding() {
        assert_eq!(translate(Key::O, CTRL_SHIFT), Some(Action::ToggleOutline));
        assert_eq!(translate(Key::O, NONE), None);
    }

    #[test]
    fn export_binding() {
        assert_eq!(translate(Key::E, CTRL), Some(Action::Export));
        assert_eq!(translate(Key::E, NONE), None); // 无 ctrl 的 e 由 Text 变体承载
    }

    #[test]
    fn find_binding() {
        assert_eq!(translate(Key::F, CTRL), Some(Action::Find));
        assert_eq!(translate(Key::F, NONE), None); // 无 ctrl 的 f 由 Text 变体承载
    }

    #[test]
    fn replace_binding() {
        assert_eq!(translate(Key::H, CTRL), Some(Action::Replace));
        assert_eq!(translate(Key::H, NONE), None); // 无 ctrl 的 h 由 Text 变体承载
    }

    #[test]
    fn select_all_binding() {
        assert_eq!(
            translate(Key::A, CTRL),
            Some(Action::Doc(Intent::SelectAll))
        );
        assert_eq!(translate(Key::A, NONE), None); // 无 ctrl 的 a 由 Text 变体承载
    }

    #[test]
    fn new_binding() {
        assert_eq!(translate(Key::N, CTRL), Some(Action::New));
        assert_eq!(translate(Key::N, NONE), None); // 无 ctrl 的 n 由 Text 变体承载
    }

    #[test]
    fn quick_open_binding() {
        assert_eq!(translate(Key::P, CTRL), Some(Action::QuickOpen));
        assert_eq!(translate(Key::P, NONE), None); // 无 ctrl 的 p 由 Text 变体承载
    }

    #[test]
    fn inline_format_bindings() {
        // 3c-1：Ctrl+B/I/K → 加粗/斜体切换、插入链接
        assert_eq!(
            translate(Key::B, CTRL),
            Some(Action::Doc(Intent::ToggleWrap("**")))
        );
        assert_eq!(
            translate(Key::I, CTRL),
            Some(Action::Doc(Intent::ToggleWrap("*")))
        );
        assert_eq!(
            translate(Key::K, CTRL),
            Some(Action::Doc(Intent::InsertLink))
        );
        assert_eq!(translate(Key::B, NONE), None); // 无 ctrl 的 b/i/k 由 Text 变体承载
        assert_eq!(translate(Key::I, NONE), None);
        assert_eq!(translate(Key::K, NONE), None);
    }

    #[test]
    fn tab_bindings() {
        let shift = Mods {
            ctrl: false,
            shift: true,
        };
        assert_eq!(translate(Key::Tab, NONE), Some(Action::Doc(Intent::Indent)));
        assert_eq!(
            translate(Key::Tab, shift),
            Some(Action::Doc(Intent::Dedent))
        );
    }
}
