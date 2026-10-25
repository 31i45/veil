//! 编辑器交互策略核心：光标（mark）、IME preedit 视图态、文档级意图应用。
//! 纯逻辑、零 GUI 依赖；像素级命中与坐标换算属 layout 层（机制与策略分离，INSTANCE §0）。

use veil_core::{Bias, BlockId, Edit, MarkId, Range, Session};

/// 文档级意图（input 层翻译产出；视觉行内移动/点击由 shell 经 layout 直接换算偏移，
/// 不走这里）。preedit 激活期间一切意图被忽略——组合期按键归 IME 所有。
/// `Select*` 为扩选（anchor 不动、caret 移动），视觉行扩选由 shell 走 `set_caret_extend`。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Intent {
    InsertText(String),
    DeleteBackward,
    DeleteForward,
    Newline,
    MoveLeft,
    MoveRight,
    MoveDocStart,
    MoveDocEnd,
    SelectLeft,
    SelectRight,
    SelectDocStart,
    SelectDocEnd,
    Undo,
    Redo,
}

/// IME 组合中状态：纯视图装饰，不进入 Session 文本与撤销栈（INSTANCE §3 注解不持久化的同族约束）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Preedit {
    pub text: String,
    /// 组合串内光标字节区间（winit `Ime::Preedit` 原样透传）。
    pub cursor: Option<(usize, usize)>,
}

pub struct Editor {
    session: Session,
    /// bias Right：在光标处插入文本后光标停在插入物之后（ADR-003 规则表）。
    caret: MarkId,
    /// bias Left：选区锚点。与 caret 重合即坍缩（无选区）。
    anchor: MarkId,
    preedit: Option<Preedit>,
    /// 自上次 mark_saved 以来文档内容是否被修改（v0：光标移动不置脏；
    /// 撤销/重做总置脏，即使内容回到保存点——记事本同语义）。
    dirty: bool,
}

impl Editor {
    pub fn new(text: &str) -> Self {
        let mut session = Session::open(text);
        let caret = session
            .create_mark(0, Bias::Right)
            .expect("空文档 offset 0 恒合法");
        let anchor = session
            .create_mark(0, Bias::Left)
            .expect("空文档 offset 0 恒合法");
        Editor {
            session,
            caret,
            anchor,
            preedit: None,
            dirty: false,
        }
    }

    /// 自上次 `mark_saved` 以来内容是否被修改（保存与标题脏标的判定依据）。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 保存成功后由调用方（shell）置回干净态。
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn caret(&self) -> usize {
        // mark 在 Editor 生命周期内恒存在（仅 remove+create 瞬时不重叠）
        self.session
            .mark_offset(self.caret)
            .expect("caret mark 不被外部移除")
    }

    fn anchor_offset(&self) -> usize {
        self.session
            .mark_offset(self.anchor)
            .expect("anchor mark 不被外部移除")
    }

    /// 当前选区（文档坐标，`start <= caret/anchor` 的正规化区间）。坍缩时返回 None。
    pub fn selection(&self) -> Option<Range> {
        let (c, a) = (self.caret(), self.anchor_offset());
        if c == a {
            None
        } else {
            Some(Range::new(c.min(a), c.max(a)))
        }
    }

    /// 点击/视觉移动落点：移动光标并坍缩选区。非法偏移（越界/非字符边界）拒绝并返回 false。
    pub fn set_caret(&mut self, offset: usize) -> bool {
        // 同一偏移两次 create 校验一致、同成同败，无半截状态
        match (
            self.session.create_mark(offset, Bias::Right),
            self.session.create_mark(offset, Bias::Left),
        ) {
            (Ok(c), Ok(a)) => {
                self.session.remove_mark(self.caret);
                self.session.remove_mark(self.anchor);
                self.caret = c;
                self.anchor = a;
                true
            }
            _ => false,
        }
    }

    /// 扩选落点（Shift+方向/拖选/Shift+点击）：仅移动 caret，anchor 不动。
    pub fn set_caret_extend(&mut self, offset: usize) -> bool {
        match self.session.create_mark(offset, Bias::Right) {
            Ok(new) => {
                self.session.remove_mark(self.caret);
                self.caret = new;
                true
            }
            Err(_) => false,
        }
    }

    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    /// 空串视为组合结束（部分 IME 以空 preedit 收尾）。
    pub fn set_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        self.preedit = if text.is_empty() {
            None
        } else {
            Some(Preedit { text, cursor })
        };
    }

    pub fn clear_preedit(&mut self) {
        self.preedit = None;
    }

    /// 复制：选中文本（无选区 None）。纯读取，不动文档与选区。
    pub fn copy(&self) -> Option<String> {
        self.session.text_slice(self.selection()?)
    }

    /// 剪切：返回选中文本并删除选区（复用 DeleteBackward 的选区语义，单撤销步）。
    /// 无选区/IME 组合期 None（Typora 行为：无选区剪切不动文档）。
    pub fn cut(&mut self) -> Option<(String, Vec<BlockId>)> {
        let text = self.copy()?;
        let dirty = self.apply(Intent::DeleteBackward)?;
        Some((text, dirty))
    }

    /// 粘贴：插入文本（有选区则替换，InsertText 语义）。空串/组合期 None。
    pub fn paste(&mut self, text: &str) -> Option<Vec<BlockId>> {
        if text.is_empty() {
            return None;
        }
        self.apply(Intent::InsertText(text.to_string()))
    }

    /// IME 提交：插文本 + 清 preedit，一次完成，保证 commit 永不落入组合态残留。
    pub fn commit_ime(&mut self, text: String) -> Vec<BlockId> {
        self.preedit = None;
        self.apply(Intent::InsertText(text)).unwrap_or_default()
    }

    /// 应用文档级意图。返回 None = 无意图效果（空撤销栈、文首退格等），视图无需重绘。
    /// 选区存在时：插入/换行/删除替换选区，移动键坍缩到选区端点，纯扩选意图继续扩选。
    /// 成功的内容编辑（插入/删除/换行/撤销/重做）置脏标记；移动与失败操作不置脏。
    pub fn apply(&mut self, intent: Intent) -> Option<Vec<BlockId>> {
        let mutating = matches!(
            intent,
            Intent::InsertText(_)
                | Intent::DeleteBackward
                | Intent::DeleteForward
                | Intent::Newline
                | Intent::Undo
                | Intent::Redo
        );
        let r = self.apply_inner(intent);
        if mutating && r.is_some() {
            self.dirty = true;
        }
        r
    }

    fn apply_inner(&mut self, intent: Intent) -> Option<Vec<BlockId>> {
        if self.preedit.is_some() {
            return None; // 组合期按键归 IME（Windows 下 IME 消费组合键，此乃兜底）
        }
        let c = self.caret();
        if let Some(sel) = self.selection() {
            match intent {
                Intent::InsertText(text) => {
                    let at = sel.start + text.len();
                    let ev = self
                        .session
                        .apply(Edit::Replace { range: sel, text })
                        .expect("选区来自 mark 恒合法");
                    return Some(self.collapse_and_return(ev.dirty_blocks, at));
                }
                Intent::Newline => {
                    let ev = self
                        .session
                        .apply(Edit::Replace {
                            range: sel,
                            text: "\n".into(),
                        })
                        .expect("选区恒合法");
                    return Some(self.collapse_and_return(ev.dirty_blocks, sel.start + 1));
                }
                Intent::DeleteBackward | Intent::DeleteForward => {
                    let ev = self
                        .session
                        .apply(Edit::Delete { range: sel })
                        .expect("选区恒合法");
                    return Some(self.collapse_and_return(ev.dirty_blocks, sel.start));
                }
                Intent::MoveLeft => return Some(self.collapse_to(sel.start)),
                Intent::MoveRight => return Some(self.collapse_to(sel.end)),
                Intent::MoveDocStart => return Some(self.collapse_to(0)),
                Intent::MoveDocEnd => {
                    let end = self.session.text_len();
                    return Some(self.collapse_to(end));
                }
                Intent::Undo | Intent::Redo => {}
                // Select* 意图继续扩选（不走「编辑意图」优先通道）
                Intent::SelectLeft
                | Intent::SelectRight
                | Intent::SelectDocStart
                | Intent::SelectDocEnd => {}
            }
        }
        match intent {
            Intent::InsertText(text) => {
                let dirty = self
                    .session
                    .apply(Edit::Insert { at: c, text })
                    .expect("caret 恒合法")
                    .dirty_blocks;
                // anchor 是 Bias::Left：纯插入时留在原处会与右移的 caret 构成伪选区
                // （下一字符将替换刚输入内容）。插入后 anchor 必须追齐 caret。
                self.collapse_to(self.caret());
                Some(dirty)
            }
            Intent::DeleteBackward => {
                let prev = self.session.prev_grapheme(c)?;
                self.session
                    .apply(Edit::Delete {
                        range: Range::new(prev, c),
                    })
                    .ok()
                    .map(|e| e.dirty_blocks)
            }
            Intent::DeleteForward => {
                let next = self.session.next_grapheme(c)?;
                self.session
                    .apply(Edit::Delete {
                        range: Range::new(c, next),
                    })
                    .ok()
                    .map(|e| e.dirty_blocks)
            }
            Intent::Newline => {
                let dirty = self
                    .session
                    .apply(Edit::Insert {
                        at: c,
                        text: "\n".into(),
                    })
                    .expect("caret 恒合法")
                    .dirty_blocks;
                self.collapse_to(self.caret()); // 同 InsertText：换行后 anchor 追齐 caret
                Some(dirty)
            }
            Intent::MoveLeft => self.session.prev_grapheme(c).map(|p| self.collapse_to(p)),
            Intent::MoveRight => self.session.next_grapheme(c).map(|p| self.collapse_to(p)),
            Intent::MoveDocStart => Some(self.collapse_to(0)),
            Intent::MoveDocEnd => Some(self.collapse_to(self.session.text_len())),
            Intent::SelectLeft => self.session.prev_grapheme(c).map(|p| self.extend_to(p)),
            Intent::SelectRight => self.session.next_grapheme(c).map(|p| self.extend_to(p)),
            Intent::SelectDocStart => (c > 0).then(|| self.extend_to(0)),
            Intent::SelectDocEnd => {
                let end = self.session.text_len();
                (c < end).then(|| self.extend_to(end))
            }
            Intent::Undo => {
                let r = self.session.undo().map(|e| e.dirty_blocks);
                if r.is_some() {
                    self.collapse_to(self.caret());
                }
                r
            }
            Intent::Redo => {
                let r = self.session.redo().map(|e| e.dirty_blocks);
                if r.is_some() {
                    self.collapse_to(self.caret());
                }
                r
            }
        }
    }

    /// 光标纯移动：同时坍缩选区（anchor 追齐 caret），不产脏块。
    fn collapse_to(&mut self, offset: usize) -> Vec<BlockId> {
        self.set_caret(offset);
        Vec::new()
    }

    /// 扩选移动：仅动 caret，不产脏块。
    fn extend_to(&mut self, offset: usize) -> Vec<BlockId> {
        self.set_caret_extend(offset);
        Vec::new()
    }

    /// 先坍缩光标再返回脏块（替换/删除/撤销共用）。
    fn collapse_and_return(&mut self, dirty: Vec<BlockId>, offset: usize) -> Vec<BlockId> {
        self.set_caret(offset);
        dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(text: &str, caret: usize) -> Editor {
        let mut e = Editor::new(text);
        assert!(e.set_caret(caret));
        e
    }

    #[test]
    fn insert_moves_caret_after_text() {
        let mut e = Editor::new("ab");
        assert!(e.apply(Intent::InsertText("中".into())).is_some());
        assert_eq!(e.session().serialize(), "中ab");
        assert_eq!(e.caret(), 3);
    }

    #[test]
    fn backspace_deletes_one_grapheme() {
        let mut e = ed("a中b", 4); // 「中」= 3 字节，光标在其后
        e.apply(Intent::DeleteBackward).unwrap();
        assert_eq!(e.session().serialize(), "ab");
        assert_eq!(e.caret(), 1);
    }

    #[test]
    fn backspace_at_doc_start_is_noop() {
        let mut e = Editor::new("ab");
        assert_eq!(e.apply(Intent::DeleteBackward), None);
        assert_eq!(e.session().serialize(), "ab");
    }

    #[test]
    fn delete_forward_at_end_is_noop() {
        let mut e = ed("ab", 2);
        assert_eq!(e.apply(Intent::DeleteForward), None);
    }

    #[test]
    fn delete_forward_removes_grapheme_caret_stays() {
        let mut e = ed("a中b", 1);
        e.apply(Intent::DeleteForward).unwrap();
        assert_eq!(e.session().serialize(), "ab");
        assert_eq!(e.caret(), 1);
    }

    #[test]
    fn newline_splits_and_returns_dirty() {
        let mut e = ed("ab", 1);
        let dirty = e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "a\nb");
        assert!(!dirty.is_empty());
        assert_eq!(e.caret(), 2);
    }

    #[test]
    fn move_left_right_crosses_multibyte() {
        let mut e = ed("中a", 0);
        e.apply(Intent::MoveRight).unwrap();
        assert_eq!(e.caret(), 3);
        e.apply(Intent::MoveRight).unwrap();
        assert_eq!(e.caret(), 4);
        assert_eq!(e.apply(Intent::MoveRight), None); // 文末
        e.apply(Intent::MoveLeft).unwrap();
        assert_eq!(e.caret(), 3);
    }

    #[test]
    fn move_doc_bounds() {
        let mut e = ed("ab\ncd", 2);
        e.apply(Intent::MoveDocEnd).unwrap();
        assert_eq!(e.caret(), 5);
        e.apply(Intent::MoveDocStart).unwrap();
        assert_eq!(e.caret(), 0);
    }

    #[test]
    fn undo_redo_roundtrip_and_caret() {
        let mut e = Editor::new("ab");
        e.apply(Intent::InsertText("x".into())).unwrap();
        assert_eq!(e.session().serialize(), "xab");
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), "ab");
        assert_eq!(e.caret(), 0); // 插入被撤销，光标坍缩回插入点
        e.apply(Intent::Redo).unwrap();
        assert_eq!(e.session().serialize(), "xab");
    }

    #[test]
    fn undo_empty_stack_is_none() {
        let mut e = Editor::new("ab");
        assert_eq!(e.apply(Intent::Undo), None);
    }

    #[test]
    fn set_caret_rejects_non_boundary() {
        let mut e = Editor::new("中");
        assert!(!e.set_caret(1)); // 「中」内部
        assert!(!e.set_caret(4)); // 越界
        assert_eq!(e.caret(), 0); // 原光标不动
    }

    #[test]
    fn preedit_blocks_intents_until_commit() {
        let mut e = Editor::new("ab");
        e.set_preedit("zhong".into(), Some((0, 5)));
        assert_eq!(e.apply(Intent::InsertText("x".into())), None); // 组合期忽略
        assert_eq!(e.apply(Intent::Newline), None);
        assert!(e.preedit().is_some()); // preedit 不被意图清掉
        e.commit_ime("中".into());
        assert!(e.preedit().is_none());
        assert_eq!(e.session().serialize(), "中ab");
        assert_eq!(e.caret(), 3);
    }

    #[test]
    fn empty_preedit_clears_state() {
        let mut e = Editor::new("ab");
        e.set_preedit("z".into(), None);
        e.set_preedit(String::new(), None);
        assert!(e.preedit().is_none());
    }

    #[test]
    fn serialize_never_contains_preedit() {
        // 不变量：preedit 是纯视图态，序列化产物恒无痕迹
        let mut e = Editor::new("ab");
        e.set_preedit("zhong".into(), None);
        assert_eq!(e.session().serialize(), "ab");
    }

    // ---- 选区（切片 3）：anchor(Bias::Left) 与 caret(Bias::Right) 构成选区 ----
    // 偏移逐字节手工核算：「a中b」a=[0,1) 中=[1,4) b=[4,5) 全长 5

    #[test]
    fn selection_collapsed_initially() {
        let e = Editor::new("ab");
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn select_right_extends_over_grapheme() {
        let mut e = Editor::new("a中b");
        e.apply(Intent::SelectRight).unwrap();
        assert_eq!(e.selection(), Some(Range::new(0, 1)));
        e.apply(Intent::SelectRight).unwrap(); // 跨过「中」：1 → 4
        assert_eq!(e.selection(), Some(Range::new(0, 4)));
        assert_eq!(e.caret(), 4);
    }

    #[test]
    fn select_left_extends_backward() {
        let mut e = ed("a中b", 5);
        e.apply(Intent::SelectLeft).unwrap();
        assert_eq!(e.selection(), Some(Range::new(4, 5)));
        e.apply(Intent::SelectLeft).unwrap(); // 4 → 1（跨过「中」）
        assert_eq!(e.selection(), Some(Range::new(1, 5)));
        assert_eq!(e.caret(), 1);
    }

    #[test]
    fn select_shrinks_then_collapses_on_reverse() {
        let mut e = Editor::new("abcd");
        e.apply(Intent::SelectRight).unwrap();
        e.apply(Intent::SelectRight).unwrap();
        assert_eq!(e.selection(), Some(Range::new(0, 2)));
        e.apply(Intent::SelectLeft).unwrap(); // 反向收缩 caret 2 → 1
        assert_eq!(e.selection(), Some(Range::new(0, 1)));
        e.apply(Intent::SelectLeft).unwrap(); // caret 与 anchor 重合于 0：坍缩
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn select_at_bounds_is_noop() {
        let mut e = Editor::new("ab");
        assert_eq!(e.apply(Intent::SelectLeft), None); // 文首
        let mut e = ed("ab", 2);
        assert_eq!(e.apply(Intent::SelectRight), None); // 文末
    }

    #[test]
    fn insert_replaces_selection() {
        let mut e = ed("a中b", 1);
        e.apply(Intent::SelectRight).unwrap(); // 选中「中」[1,4)
        e.apply(Intent::InsertText("X".into())).unwrap();
        assert_eq!(e.session().serialize(), "aXb");
        assert_eq!(e.caret(), 2); // X=[1,2)，光标停在插入物之后
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn replace_selection_is_single_undo_step() {
        let mut e = ed("a中b", 1);
        e.apply(Intent::SelectRight).unwrap();
        e.apply(Intent::InsertText("X".into())).unwrap();
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), "a中b"); // 一次撤销即还原，非「先撤插入再撤删除」
        assert_eq!(e.selection(), None); // 撤销后选区坍缩（v0 不恢复历史选区）
        e.apply(Intent::Redo).unwrap();
        assert_eq!(e.session().serialize(), "aXb");
    }

    #[test]
    fn backspace_deletes_selection() {
        let mut e = ed("a中b", 1);
        e.apply(Intent::SelectRight).unwrap();
        e.apply(Intent::DeleteBackward).unwrap();
        assert_eq!(e.session().serialize(), "ab");
        assert_eq!(e.caret(), 1);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn delete_forward_deletes_selection() {
        let mut e = ed("a中b", 1);
        e.apply(Intent::SelectRight).unwrap();
        e.apply(Intent::DeleteForward).unwrap();
        assert_eq!(e.session().serialize(), "ab");
        assert_eq!(e.caret(), 1);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn newline_replaces_selection() {
        let mut e = ed("abcd", 1);
        e.apply(Intent::SelectRight).unwrap(); // 选中 [1,2) = "b"
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "a\ncd"); // "b" 被 "\n" 替换
        assert_eq!(e.caret(), 2);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn plain_move_collapses_forward_selection() {
        let mut e = ed("abcd", 1);
        e.apply(Intent::SelectRight).unwrap(); // anchor=1 caret=2
        e.apply(Intent::MoveLeft).unwrap();
        assert_eq!(e.caret(), 1); // 坍缩到选区起点，不再左移
        assert_eq!(e.selection(), None);
        let mut e = ed("abcd", 1);
        e.apply(Intent::SelectRight).unwrap();
        e.apply(Intent::MoveRight).unwrap();
        assert_eq!(e.caret(), 2); // 坍缩到选区终点
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn plain_move_collapses_backward_selection() {
        let mut e = ed("abcd", 2);
        e.apply(Intent::SelectLeft).unwrap(); // anchor=2 caret=1，选区 [1,2)
        e.apply(Intent::MoveRight).unwrap();
        assert_eq!(e.caret(), 2);
        assert_eq!(e.selection(), None);
        let mut e = ed("abcd", 2);
        e.apply(Intent::SelectLeft).unwrap();
        e.apply(Intent::MoveLeft).unwrap();
        assert_eq!(e.caret(), 1);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn set_caret_collapses_selection() {
        let mut e = Editor::new("abcd");
        e.apply(Intent::SelectRight).unwrap();
        e.apply(Intent::SelectRight).unwrap();
        assert!(e.set_caret(3));
        assert_eq!(e.selection(), None);
        assert_eq!(e.caret(), 3);
    }

    #[test]
    fn set_caret_extend_keeps_anchor_and_validates() {
        let mut e = ed("abcd", 1);
        assert!(e.set_caret_extend(3));
        assert_eq!(e.selection(), Some(Range::new(1, 3)));
        assert!(!e.set_caret_extend(9)); // 越界拒绝，选区不变
        assert_eq!(e.selection(), Some(Range::new(1, 3)));
    }

    #[test]
    fn undo_collapses_selection() {
        let mut e = Editor::new("abcd");
        e.apply(Intent::InsertText("x".into())).unwrap(); // "xabcd"，caret=1
        e.apply(Intent::SelectRight).unwrap(); // 选区 [1,2) = "a"
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), "abcd");
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn select_doc_bounds() {
        let mut e = ed("ab\ncd", 1);
        e.apply(Intent::SelectDocEnd).unwrap();
        assert_eq!(e.selection(), Some(Range::new(1, 5)));
        e.apply(Intent::SelectDocStart).unwrap(); // caret 5 → 0，anchor 仍为 1
        assert_eq!(e.selection(), Some(Range::new(0, 1)));
    }

    #[test]
    fn commit_ime_replaces_selection() {
        let mut e = ed("abcd", 1);
        e.apply(Intent::SelectRight).unwrap(); // 选中 [1,2) = "b"
        e.commit_ime("中".into());
        assert_eq!(e.session().serialize(), "a中cd"); // 中=[1,4)
        assert_eq!(e.caret(), 4);
        assert_eq!(e.selection(), None);
    }

    // ---- 剪贴板纯逻辑（切片 4）：系统读写由 shell 经 Clipboard trait 注入 ----
    // 偏移逐字节手工核算：「abcdef」全长 6，选区 [1,4)="bcd"；「中」=3 字节

    fn ed_sel(text: &str, anchor: usize, caret: usize) -> Editor {
        let mut e = Editor::new(text);
        assert!(e.set_caret(anchor));
        assert!(e.set_caret_extend(caret));
        e
    }

    #[test]
    fn copy_returns_selected_text() {
        let e = ed_sel("abcdef", 1, 4);
        assert_eq!(e.copy().as_deref(), Some("bcd"));
        assert_eq!(e.selection(), Some(Range::new(1, 4))); // 复制不动选区
    }

    #[test]
    fn copy_collapsed_is_none() {
        let e = ed("abcdef", 2);
        assert_eq!(e.copy(), None);
    }

    #[test]
    fn cut_removes_selection_single_undo() {
        let mut e = ed_sel("abcdef", 1, 4);
        let (text, _dirty) = e.cut().expect("有选区必可剪");
        assert_eq!(text, "bcd");
        assert_eq!(e.session().serialize(), "aef");
        assert_eq!(e.caret(), 1);
        assert_eq!(e.selection(), None);
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), "abcdef"); // 单撤销步还原
    }

    #[test]
    fn cut_collapsed_is_none() {
        let mut e = ed("abcdef", 2);
        assert_eq!(e.cut(), None);
        assert_eq!(e.session().serialize(), "abcdef");
    }

    #[test]
    fn paste_replaces_selection_single_undo() {
        let mut e = ed_sel("abcdef", 1, 4);
        e.paste("XY").unwrap();
        assert_eq!(e.session().serialize(), "aXYef");
        assert_eq!(e.caret(), 3); // 1 + len("XY")
        assert_eq!(e.selection(), None);
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), "abcdef"); // 替换为单撤销步
    }

    #[test]
    fn paste_inserts_at_caret_multibyte() {
        let mut e = ed("abcdef", 2);
        e.paste("中").unwrap(); // 「中」=3 字节
        assert_eq!(e.session().serialize(), "ab中cdef");
        assert_eq!(e.caret(), 5);
    }

    #[test]
    fn paste_empty_is_noop() {
        let mut e = ed("abcdef", 2);
        assert_eq!(e.paste(""), None);
        assert_eq!(e.session().serialize(), "abcdef");
    }

    #[test]
    fn cut_paste_ignored_during_preedit() {
        let mut e = ed_sel("abcdef", 1, 4);
        e.set_preedit("pin".into(), None);
        assert_eq!(e.cut(), None);
        assert_eq!(e.paste("X"), None);
        assert_eq!(e.session().serialize(), "abcdef"); // 组合期文档不变
    }

    // ---- 连续键入（回归）：纯插入不得留下覆盖插入文本的伪选区 ----

    #[test]
    fn consecutive_inserts_do_not_replace_previous_char() {
        let mut e = Editor::new("");
        e.apply(Intent::InsertText("h".into())).unwrap();
        assert_eq!(e.selection(), None, "纯插入后 anchor 须追齐 caret");
        e.apply(Intent::InsertText("i".into())).unwrap();
        assert_eq!(e.session().serialize(), "hi");
        assert_eq!(e.caret(), 2);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn newline_does_not_leave_selection() {
        let mut e = ed("ab", 1);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "a\nb");
        assert_eq!(e.selection(), None);
    }

    // ---- 脏标记（切片 5）：保存语义的视图层状态，v0 不追踪「撤销回到保存点」----

    #[test]
    fn dirty_lifecycle() {
        let mut e = Editor::new("ab");
        assert!(!e.is_dirty()); // 打开即干净
        e.apply(Intent::MoveRight); // 光标移动不置脏
        assert!(!e.is_dirty());
        e.apply(Intent::InsertText("中".into())).unwrap();
        assert!(e.is_dirty()); // 内容编辑置脏
        e.mark_saved();
        assert!(!e.is_dirty()); // 保存后干净
        e.apply(Intent::Undo).unwrap(); // 撤销回到保存点：v0 仍置脏（记事本同语义）
        assert!(e.is_dirty());
    }

    #[test]
    fn failed_or_noop_edit_does_not_dirty() {
        let mut e = Editor::new("");
        assert_eq!(e.apply(Intent::DeleteBackward), None); // 文首退格无效果
        assert!(!e.is_dirty());
        assert_eq!(e.apply(Intent::Undo), None); // 空撤销栈
        assert!(!e.is_dirty());
    }

    #[test]
    fn clipboard_edits_dirty_but_copy_does_not() {
        let mut e = ed_sel("abcdef", 1, 4);
        assert!(e.copy().is_some());
        assert!(!e.is_dirty()); // 复制纯读取
        e.cut().unwrap();
        assert!(e.is_dirty()); // 剪切即删除
        e.mark_saved();
        e.paste("Z").unwrap();
        assert!(e.is_dirty()); // 粘贴即插入
    }
}
