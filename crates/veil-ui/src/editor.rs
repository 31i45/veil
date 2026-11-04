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
    /// Ctrl+←/→：按词移动（字母数字/下划线 + CJK 表意字为一个词，换行是硬边界）。
    WordLeft,
    WordRight,
    /// Ctrl+Shift+←/→：按词扩选（anchor 不动）。
    SelectWordLeft,
    SelectWordRight,
    SelectDocStart,
    SelectDocEnd,
    /// 全选（Ctrl+A）：anchor 落文首、caret 落文末，随后输入即整体替换。
    SelectAll,
    /// 缩进（Tab）：无选区插 4 空格；有选区对触及的每行加 4 空格（空行跳过）。
    Indent,
    /// 反缩进（Shift+Tab）：删除行首至多 4 个空格或 1 个制表符，绝不删非空白字符。
    Dedent,
    /// 行内格式切换（3c-1）：有选区包裹/解包裹 marker（`**`/`*` 等）；空选区插入
    /// 标记对、光标居中，光标恰在标记对中时删除标记对（连按两次取消）。
    ToggleWrap(&'static str),
    /// 插入链接（3c-1）：选区作链接文本 `[sel]()`、光标入括号；空选区 `[]()`、光标入方括号。
    InsertLink,
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
    /// 文档换行风格（首个换行序列决定）。编辑插入新行时沿用，避免在 CRLF
    /// 文档里混入 LF 而构成隐式规范化（阶段 2 源码真值不变量）。
    line_ending: &'static str,
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
            line_ending: detect_line_ending(text),
        }
    }

    /// 自上次 `mark_saved` 以来内容是否被修改（保存与标题脏标的判定依据）。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 恢复场景专用（0b）：崩溃恢复的内容相对磁盘基线是「未保存修改」，打开即标脏。
    /// 正常编辑路径不得调用（脏标只能由内容编辑置位、mark_saved 清除）。
    pub(crate) fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    /// 保存成功后由调用方（shell）置回干净态。
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// 断开撤销分组（最小门面）：鼠标落光标/划选前由交互层调用，
    /// 与键盘移动/扩选路径的断组对齐（VS Code 语义）。
    pub(crate) fn break_undo_group(&mut self) {
        self.session.break_undo_group();
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
        self.set_marks(offset, offset)
    }

    /// 同时重置 caret（Bias::Right）与 anchor（Bias::Left）。同一偏移两次 create
    /// 校验一致、同成同败，无半截状态；任一失败则旧标记原样保留。
    fn set_marks(&mut self, caret: usize, anchor: usize) -> bool {
        match (
            self.session.create_mark(caret, Bias::Right),
            self.session.create_mark(anchor, Bias::Left),
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

    /// 双击选词：选中 `offset` 处字符所属类别（词 / 空白 / 标点）的连续串。
    /// 换行是硬边界（空白串不跨行）；游标落在行尾换行处时按前一字符取词。
    /// 返回 false = 空文档 / 空行 / 无内容可选。
    pub fn select_word_at(&mut self, offset: usize) -> bool {
        let Some(text) = self.full_text() else {
            return false;
        };
        let Some((start, end)) = word_run(&text, offset) else {
            return false;
        };
        self.set_marks(end, start)
    }

    /// 三击选行：选中 `offset` 所在行（不含行尾换行）；空行无可选内容。
    pub fn select_line_at(&mut self, offset: usize) -> bool {
        let r = self.line_bounds(offset);
        if r.start == r.end {
            return false;
        }
        self.set_marks(r.end, r.start)
    }

    /// 全文文本（词边界扫描用）；空文档返回空串。
    fn full_text(&self) -> Option<String> {
        self.session
            .text_slice(Range::new(0, self.session.text_len()))
    }

    /// 单字节文本（CRLF 判定的 1 字节比较）；越界返回 None。
    fn byte_at(&self, i: usize) -> Option<u8> {
        self.session
            .text_slice(Range::new(i, i + 1))
            .and_then(|s| s.as_bytes().first().copied())
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
                | Intent::Indent
                | Intent::Dedent
                | Intent::ToggleWrap(_)
                | Intent::InsertLink
                | Intent::Undo
                | Intent::Redo
        );
        let r = self.apply_inner(intent);
        if mutating && r.is_some() {
            self.dirty = true;
        }
        r
    }

    /// 切换任务列表复选框（P3-5，点击方框）。`state_range` 为状态字符（` `/`x`）的
    /// 文档字节区间，`checked` 为目标态。光标保持原位（点击方框不移动光标，
    /// 与 GitHub 行为一致）；等长替换故各标记无需位移。成功即置脏标。
    pub(crate) fn toggle_task(
        &mut self,
        state_range: Range,
        checked: bool,
    ) -> Option<Vec<BlockId>> {
        let ev = self
            .session
            .apply(Edit::Replace {
                range: state_range,
                text: if checked { "x" } else { " " }.into(),
            })
            .ok()?;
        self.dirty = true;
        Some(ev.dirty_blocks)
    }

    /// 查找替换·单次：替换某个命中区间（一步事务），光标落在替换文本之后。
    pub(crate) fn replace_range(&mut self, range: Range, text: &str) -> Option<Vec<BlockId>> {
        let ev = self
            .session
            .apply(Edit::Replace {
                range,
                text: text.into(),
            })
            .ok()?;
        self.dirty = true;
        self.set_caret(range.start + text.len());
        Some(ev.dirty_blocks)
    }

    /// 查找替换·全部：`ranges` 按起点升序且互不重叠，首尾之间整段一次 `Replace`，
    /// 保证全部替换是单个撤销事务。返回 (脏块, 替换次数)；无命中 None。
    pub(crate) fn replace_all(
        &mut self,
        ranges: &[Range],
        text: &str,
    ) -> Option<(Vec<BlockId>, usize)> {
        let (first, last) = (ranges.first()?, ranges.last()?);
        let len = self.session.text_len();
        let hay = self.session.text_slice(Range::new(0, len))?;
        let mut out = String::with_capacity(len + ranges.len() * text.len());
        let mut at = first.start;
        for r in ranges {
            out.push_str(&hay[at..r.start]);
            out.push_str(text);
            at = r.end;
        }
        debug_assert_eq!(at, last.end, "命中区间升序且互不重叠");
        let ev = self
            .session
            .apply(Edit::Replace {
                range: Range::new(first.start, last.end),
                text: out,
            })
            .ok()?;
        self.dirty = true;
        self.set_caret(first.start);
        Some((ev.dirty_blocks, ranges.len()))
    }

    fn apply_inner(&mut self, intent: Intent) -> Option<Vec<BlockId>> {
        if self.preedit.is_some() {
            return None; // 组合期按键归 IME（Windows 下 IME 消费组合键，此乃兜底）
        }
        // 非编辑的光标移动/选区变化即断开撤销分组（VS Code 语义）。此处是 Intent 分派的
        // 唯一入口，故单点接线；编辑意图不在此列，否则连续输入会被拆成单字符撤销。
        if matches!(
            intent,
            Intent::MoveLeft
                | Intent::MoveRight
                | Intent::WordLeft
                | Intent::WordRight
                | Intent::MoveDocStart
                | Intent::MoveDocEnd
                | Intent::SelectLeft
                | Intent::SelectRight
                | Intent::SelectWordLeft
                | Intent::SelectWordRight
                | Intent::SelectDocStart
                | Intent::SelectDocEnd
                | Intent::SelectAll
        ) {
            self.session.break_undo_group();
        }
        // 行内格式意图自带选区语义，先于此处分流（下方两个 match 的同名臂仅为穷尽性）
        match intent {
            Intent::ToggleWrap(marker) => return self.apply_wrap(marker),
            Intent::InsertLink => return self.apply_link(),
            _ => {}
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
                    let text = self.line_ending;
                    let ev = self
                        .session
                        .apply(Edit::Replace {
                            range: sel,
                            text: text.into(),
                        })
                        .expect("选区恒合法");
                    return Some(self.collapse_and_return(ev.dirty_blocks, sel.start + text.len()));
                }
                Intent::DeleteBackward | Intent::DeleteForward => {
                    let ev = self
                        .session
                        .apply(Edit::Delete { range: sel })
                        .expect("选区恒合法");
                    return Some(self.collapse_and_return(ev.dirty_blocks, sel.start));
                }
                Intent::MoveLeft | Intent::WordLeft => return Some(self.collapse_to(sel.start)),
                Intent::MoveRight | Intent::WordRight => return Some(self.collapse_to(sel.end)),
                Intent::MoveDocStart => return Some(self.collapse_to(0)),
                Intent::MoveDocEnd => {
                    let end = self.session.text_len();
                    return Some(self.collapse_to(end));
                }
                Intent::Undo | Intent::Redo => {}
                // Select* 意图继续扩选（不走「编辑意图」优先通道）
                Intent::SelectLeft
                | Intent::SelectRight
                | Intent::SelectWordLeft
                | Intent::SelectWordRight
                | Intent::SelectDocStart
                | Intent::SelectDocEnd
                | Intent::SelectAll => {}
                // 缩进选中行：选区保留（落点随行首前缀平移），在主 match 中统一处理
                Intent::Indent | Intent::Dedent => {}
                // 已于入口分流（穷尽性占位）
                Intent::ToggleWrap(_) | Intent::InsertLink => {}
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
                // CRLF 视为一个换行单位：删到行尾 `\n` 时连同前面的 `\r` 一起删，
                // 否则会留下孤立 `\r`（破坏源码真值与后续行级判定）
                let mut start = prev;
                if prev > 0
                    && self.byte_at(prev) == Some(b'\n')
                    && self.byte_at(prev - 1) == Some(b'\r')
                {
                    start -= 1;
                }
                self.session
                    .apply(Edit::Delete {
                        range: Range::new(start, c),
                    })
                    .ok()
                    .map(|e| e.dirty_blocks)
            }
            Intent::DeleteForward => {
                let next = self.session.next_grapheme(c)?;
                // 同理：正向删到 `\r` 时把紧随的 `\n` 一并删掉
                let mut end = next;
                if self.byte_at(c) == Some(b'\r') && self.byte_at(next) == Some(b'\n') {
                    end += 1;
                }
                self.session
                    .apply(Edit::Delete {
                        range: Range::new(c, end),
                    })
                    .ok()
                    .map(|e| e.dirty_blocks)
            }
            Intent::Newline => Some(self.newline(c)),
            Intent::MoveLeft => self.session.prev_grapheme(c).map(|p| self.collapse_to(p)),
            Intent::MoveRight => self.session.next_grapheme(c).map(|p| self.collapse_to(p)),
            Intent::WordLeft => self.word_move(c, false, false),
            Intent::WordRight => self.word_move(c, true, false),
            Intent::SelectWordLeft => self.word_move(c, false, true),
            Intent::SelectWordRight => self.word_move(c, true, true),
            Intent::MoveDocStart => Some(self.collapse_to(0)),
            Intent::MoveDocEnd => Some(self.collapse_to(self.session.text_len())),
            Intent::SelectLeft => self.session.prev_grapheme(c).map(|p| self.extend_to(p)),
            Intent::SelectRight => self.session.next_grapheme(c).map(|p| self.extend_to(p)),
            Intent::SelectDocStart => (c > 0).then(|| self.extend_to(0)),
            Intent::SelectDocEnd => {
                let end = self.session.text_len();
                (c < end).then(|| self.extend_to(end))
            }
            Intent::SelectAll => {
                let end = self.session.text_len();
                if end == 0 {
                    return None; // 空文档无可选内容
                }
                // 先坍缩到文首（anchor=Bias::Left 落 0），再扩到文末（caret=Bias::Right 落 end）
                self.set_caret(0);
                Some(self.extend_to(end))
            }
            Intent::Indent => self.apply_indent(true),
            Intent::Dedent => self.apply_indent(false),
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
            // 已于入口分流（穷尽性占位）
            Intent::ToggleWrap(_) | Intent::InsertLink => unreachable!("入口已分流"),
        }
    }

    /// 行内格式切换（3c-1）：有选区包裹/解包裹 marker；空选区插标记对、光标居中，
    /// 光标恰在标记对中时删标记对。整段一次 Replace/Delete——单撤销步。
    /// 单字符 marker 遇到 `**...**` 双标记包裹时不误判解包裹，再包一层（加粗上叠斜体）。
    fn apply_wrap(&mut self, marker: &str) -> Option<Vec<BlockId>> {
        let m = marker.len();
        if let Some(sel) = self.selection() {
            let text = self.session.text_slice(sel)?;
            // 严格按字节切 UTF-8 会 panic（选区首/尾是 CJK 时 `text[1..]` 不在字符边界）：
            // 先判定「已包裹」，再取内部切片判双包裹，全程用 starts/ends_with。
            let enclosed =
                text.len() >= 2 * m && text.starts_with(marker) && text.ends_with(marker);
            let double_wrapped = enclosed && m == 1 && {
                let inner = &text[m..text.len() - m];
                inner.starts_with(marker) && inner.ends_with(marker)
            };
            let wrapped = enclosed && text.len() > 2 * m && !double_wrapped;
            let (new, at) = if wrapped {
                let inner = &text[m..text.len() - m];
                (inner.to_string(), sel.start + inner.len())
            } else {
                (format!("{marker}{text}{marker}"), sel.start + text.len() + 2 * m)
            };
            let ev = self
                .session
                .apply(Edit::Replace { range: sel, text: new })
                .expect("选区恒合法");
            return Some(self.collapse_and_return(ev.dirty_blocks, at));
        }
        let c = self.caret();
        // 光标恰在标记对中（两侧各 m 字节均为 marker）→ 删除标记对
        if c >= m
            && self.session.text_slice(Range::new(c - m, c)).as_deref() == Some(marker)
            && self.session.text_slice(Range::new(c, c + m)).as_deref() == Some(marker)
        {
            let ev = self
                .session
                .apply(Edit::Delete {
                    range: Range::new(c - m, c + m),
                })
                .ok()?;
            return Some(self.collapse_and_return(ev.dirty_blocks, c - m));
        }
        let ev = self
            .session
            .apply(Edit::Insert {
                at: c,
                text: format!("{marker}{marker}"),
            })
            .expect("caret 恒合法");
        Some(self.collapse_and_return(ev.dirty_blocks, c + m))
    }

    /// 插入链接（3c-1）：选区作链接文本、光标入 `()`；空选区插 `[]()`、光标入 `[]`。单撤销步。
    fn apply_link(&mut self) -> Option<Vec<BlockId>> {
        if let Some(sel) = self.selection() {
            let text = self.session.text_slice(sel)?;
            let at = sel.start + text.len() + 3; // `[` + 文本 + `](` 之后，光标落在 `()` 内
            let ev = self
                .session
                .apply(Edit::Replace {
                    range: sel,
                    text: format!("[{text}]()"),
                })
                .expect("选区恒合法");
            return Some(self.collapse_and_return(ev.dirty_blocks, at));
        }
        let c = self.caret();
        let ev = self
            .session
            .apply(Edit::Insert {
                at: c,
                text: "[]()".into(),
            })
            .expect("caret 恒合法");
        Some(self.collapse_and_return(ev.dirty_blocks, c + 1))
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

    /// 按词移动/扩选：目标与当前位置相同（文首向左/文末向右）视为无效果。
    fn word_move(&mut self, c: usize, forward: bool, extend: bool) -> Option<Vec<BlockId>> {
        let text = self.full_text()?;
        let target = if forward {
            word_right(&text, c)
        } else {
            word_left(&text, c)
        };
        if target == c {
            return None;
        }
        Some(if extend {
            self.extend_to(target)
        } else {
            self.collapse_to(target)
        })
    }

    /// 先坍缩光标再返回脏块（替换/删除/撤销共用）。
    fn collapse_and_return(&mut self, dirty: Vec<BlockId>, offset: usize) -> Vec<BlockId> {
        self.set_caret(offset);
        dirty
    }

    /// Tab / Shift+Tab（C3）：无选区时 Tab 在光标处插 4 空格、Shift+Tab 反缩进当前行；
    /// 有选区时对触及的每一行做行首前缀变换，整段一次 `Replace`——单撤销步可整体回退。
    /// 落点按行前缀增量平移：caret 含行首前缀（Bias::Right），anchor 不含
    /// （Bias::Left 恒留新行首之前，与 mark 偏置语义一致）。
    fn apply_indent(&mut self, indent: bool) -> Option<Vec<BlockId>> {
        let selection = self.selection();
        if selection.is_none() && indent {
            let at = self.caret();
            let dirty = self
                .session
                .apply(Edit::Insert {
                    at,
                    text: INDENT.into(),
                })
                .expect("caret 恒合法")
                .dirty_blocks;
            self.collapse_to(self.caret());
            return Some(dirty);
        }
        let sel = selection.unwrap_or_else(|| self.line_bounds(self.caret()));
        let plan = self.indent_plan(sel, !indent)?;
        let (caret, anchor) = (self.caret(), self.anchor_offset());
        let (new_caret, new_anchor) = (plan.map(caret, true), plan.map(anchor, false));
        let dirty = self
            .session
            .apply(Edit::Replace {
                range: plan.span,
                text: plan.text,
            })
            .ok()?
            .dirty_blocks;
        self.set_marks(new_caret, new_anchor);
        Some(dirty)
    }

    /// Enter（C4）：行尾且行首是列表/引用标记时续写下一项（编号递增、任务项给未勾选）；
    /// 空列表项则删去整行标记以收束列表；其余情形仍是普通换行。
    /// 行中间回车不做续行（与主流编辑器一致，避免拆词时凭空造出列表）。
    fn newline(&mut self, caret: usize) -> Vec<BlockId> {
        let at_line_end = match self.session.next_grapheme(caret) {
            None => true,
            // `\r\n` 按 UAX#29 是一个字素簇；单独 `\r` 或 `\n` 也属行尾
            Some(next) => matches!(
                self.session
                    .text_slice(Range::new(caret, next))
                    .as_deref(),
                Some("\n" | "\r" | "\r\n")
            ),
        };
        let marker = if at_line_end {
            let line = self.line_bounds(caret);
            self.session
                .text_slice(line)
                .as_deref()
                .and_then(list_marker)
        } else {
            None
        };
        match marker {
            Some(m) if m.empty => {
                let line = self.line_bounds(caret);
                let dirty = self
                    .session
                    .apply(Edit::Delete { range: line })
                    .expect("行区间恒合法")
                    .dirty_blocks;
                self.collapse_to(self.caret());
                dirty
            }
            Some(m) => {
                let dirty = self
                    .session
                    .apply(Edit::Insert {
                        at: caret,
                        text: format!("{}{}", self.line_ending, m.next),
                    })
                    .expect("caret 恒合法")
                    .dirty_blocks;
                self.collapse_to(self.caret()); // 同 InsertText：换行后 anchor 追齐 caret
                dirty
            }
            None => {
                let dirty = self
                    .session
                    .apply(Edit::Insert {
                        at: caret,
                        text: self.line_ending.into(),
                    })
                    .expect("caret 恒合法")
                    .dirty_blocks;
                self.collapse_to(self.caret());
                dirty
            }
        }
    }

    /// 光标所在行的字节区间（不含行尾换行；CRLF 的 `\r` 一并剥除）。
    fn line_bounds(&self, offset: usize) -> Range {
        let len = self.session.text_len();
        let text = self
            .session
            .text_slice(Range::new(0, len))
            .unwrap_or_default();
        let starts = line_starts(&text);
        let i = line_index(&starts, offset);
        let mut end = starts.get(i + 1).map(|&s| s - 1).unwrap_or(len);
        // 行尾 `\n` 前还挂着 `\r` 时再剥一字节：行内容不含 `\r`，list_marker/缩进
        // 与选中语义才不会把 CRLF 的 `\r` 当成正文
        if end > starts[i] && text.as_bytes().get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
        Range::new(starts[i], end)
    }

    /// 行级缩进计划：行区间 = 选区起点行 ..= 终点行；去掉换行后为空的行不做前缀
    /// 变换（Ctrl+A 后 Tab 不会给文末幻影行留下尾随空格）。全部变动为空时 None。
    fn indent_plan(&self, sel: Range, dedent: bool) -> Option<IndentPlan> {
        let len = self.session.text_len();
        let text = self.session.text_slice(Range::new(0, len))?;
        let starts = line_starts(&text);
        let first = line_index(&starts, sel.start);
        let last = line_index(&starts, sel.end);
        let mut out = String::with_capacity(text.len() + INDENT.len());
        // 行级缩进按上下文选前缀：列表项/段落用 2 空格，引用行加一层 `> `。
        // 2 空格既能让本解析器的列表 depth 加一（depth = 缩进字节/2），
        // 又低于 4 空格的缩进代码块阈值，不会把整段段落变成 IndentedCode。

        let mut lines = Vec::new();
        for i in first..=last {
            let s = starts[i];
            let e = starts.get(i + 1).copied().unwrap_or(len);
            let line = &text[s..e];
            let content = line.strip_suffix('\n').unwrap_or(line);
            if content.is_empty() {
                out.push_str(line); // 空行不缩进
                continue;
            }
            if dedent {
                let cut = dedent_prefix_len(content);
                out.push_str(&line[cut..]);
                if cut > 0 {
                    lines.push((s, -(cut as isize)));
                }
            } else {
                let prefix = indent_prefix(content);
                out.push_str(prefix);
                out.push_str(line);
                lines.push((s, prefix.len() as isize));
            }
        }
        if lines.is_empty() {
            return None;
        }
        let end = starts.get(last + 1).copied().unwrap_or(len);
        Some(IndentPlan {
            span: Range::new(starts[first], end),
            text: out,
            lines,
        })
    }
}

/// 缩进单位（4 空格）。
const INDENT: &str = "    ";

/// 一行前缀变换计划：`span` 为被整体替换的区间，`text` 为替换后文本，
/// `lines` 为 (行首旧偏移, 行前缀增量) 升序表，供 caret/anchor 落点映射。
struct IndentPlan {
    span: Range,
    text: String,
    lines: Vec<(usize, isize)>,
}

impl IndentPlan {
    /// 旧坐标 → 变换后坐标。`with_prefix_at_line_start` 为 true 时行首位置也随前缀
    /// 右移（caret）；false 时留在新行首之前（anchor，含缩进）。反缩进落点位于被删
    /// 空白内时夹到新行首。
    fn map(&self, offset: usize, with_prefix_at_line_start: bool) -> usize {
        let mut cum: isize = 0;
        for &(start, delta) in &self.lines {
            if offset < start {
                break;
            }
            if offset == start && !with_prefix_at_line_start {
                continue;
            }
            if delta < 0 && offset < start + (-delta) as usize {
                return (start as isize + cum) as usize;
            }
            cum += delta;
        }
        (offset as isize + cum) as usize
    }
}

/// 词字符（Ctrl+←/→ 判定）：字母/数字/下划线（含 CJK 表意字）；空白与换行不算。
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 左邻字符（`i` 前的字符及其起点）；`i` 为字符边界。
fn prev_char(text: &str, i: usize) -> Option<(usize, char)> {
    text[..i].char_indices().next_back()
}

/// 右邻字符（`i` 处字符及其下一偏移）。
fn next_char(text: &str, i: usize) -> Option<(usize, char)> {
    let c = text[i..].chars().next()?;
    Some((i + c.len_utf8(), c))
}

/// Ctrl+← 落点：先向左跳过非词字符，再停在词首。
fn word_left(text: &str, offset: usize) -> usize {
    let mut i = offset;
    while let Some((j, c)) = prev_char(text, i) {
        if is_word_char(c) {
            break;
        }
        i = j;
    }
    while let Some((j, c)) = prev_char(text, i) {
        if !is_word_char(c) {
            break;
        }
        i = j;
    }
    i
}

/// Ctrl+→ 落点：先向右跳过非词字符，再停在词尾。
fn word_right(text: &str, offset: usize) -> usize {
    let mut i = offset;
    while let Some((j, c)) = next_char(text, i) {
        if is_word_char(c) {
            break;
        }
        i = j;
    }
    while let Some((j, c)) = next_char(text, i) {
        if !is_word_char(c) {
            break;
        }
        i = j;
    }
    i
}

/// 双击选词类别：词 / 空白 / 标点（换行是硬边界，不参与同类扩展）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Space,
    Other,
}

fn char_class(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Space
    } else if is_word_char(c) {
        CharClass::Word
    } else {
        CharClass::Other
    }
}

fn is_line_break(c: char) -> bool {
    c == '\n' || c == '\r'
}

/// 双击所选串的字节区间：以 `offset` 处字符（行尾换行则退到前一字符）为基准，
/// 向两侧扩展到同类字符，换行处截断。空文档/空行返回 None。
fn word_run(text: &str, offset: usize) -> Option<(usize, usize)> {
    let mut at = offset.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1; // 非字符边界（布局外调用）：退到所在字符起点
    }
    let mut ch = text[at..].chars().next();
    if ch.is_none() || ch.is_some_and(is_line_break) {
        let (i, c) = prev_char(text, at)?;
        if is_line_break(c) {
            return None; // 空行：无可选内容
        }
        at = i;
        ch = Some(c);
    }
    let cls = char_class(ch?);
    let same = |c: char| !is_line_break(c) && char_class(c) == cls;
    let mut start = at;
    for (i, c) in text[..at].char_indices().rev() {
        if !same(c) {
            break;
        }
        start = i;
    }
    let mut end = at;
    for c in text[at..].chars() {
        if !same(c) {
            break;
        }
        end += c.len_utf8();
    }
    Some((start, end))
}

/// 全文字节偏移的行首表：`[0, 每个 '\n' 之后的偏移]`；末尾换行会多出一个幻影空行。
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(
        text.bytes()
            .enumerate()
            .filter(|&(_, b)| b == b'\n')
            .map(|(i, _)| i + 1),
    );
    starts
}

/// 偏移所在行下标（`starts` 由 `line_starts` 生成，至少含 0）。
fn line_index(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|&s| s <= offset) - 1
}

/// 文档换行风格：首个 `\n` 前紧邻 `\r` 即 CRLF；无换行或首个即 LF 则为 LF。
/// 只检测一次，编辑插入新行沿用该风格，保证不引入混合换行。
fn detect_line_ending(text: &str) -> &'static str {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

/// 行级缩进前缀（与 `dedent_prefix_len` 对称）：引用行加一层 `> `（保持引用结构），
/// 其余行加 2 空格。2 空格是本解析器列表 depth 的单位，且 < 4 不触发缩进代码块。
fn indent_prefix(content: &str) -> &'static str {
    if content.trim_start_matches([' ', '\t']).starts_with('>') {
        "> "
    } else {
        "  "
    }
}

/// 反缩进要删除的行首长度：引用行移除一层 `>` 标记（含其后一个空格），
/// 其余行删除一个制表符或至多 4 个空格；非空白字符绝不动。
fn dedent_prefix_len(content: &str) -> usize {
    if content.starts_with('\t') {
        return 1;
    }
    let ws = content.bytes().take_while(|&b| b == b' ').count();
    if let Some(rest) = content.get(ws..) {
        if rest.starts_with('>') {
            let marker = if rest.as_bytes().get(1) == Some(&b' ') {
                2
            } else {
                1
            };
            return ws + marker;
        }
    }
    ws.min(4)
}

/// 行首列表/引用标记：`next` 为续行前缀（含原缩进），`empty` 表示标记后无正文。
struct ListMarker {
    next: String,
    empty: bool,
}

/// 解析行首标记（C4）。任务项优先于普通无序项；编号项取原数字 +1；
/// 续行前缀保留原缩进，任务项一律续成未勾选。不支持的写法（无空格分隔、
/// `*`/`+` 项目符号、`1)` 编号等）返回 None，走普通换行。
fn list_marker(content: &str) -> Option<ListMarker> {
    let ws_len = content
        .bytes()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count();
    let ws = &content[..ws_len];
    let rest = &content[ws_len..];
    let task = rest
        .strip_prefix("- [ ] ")
        .or_else(|| rest.strip_prefix("- [x] "))
        .or_else(|| rest.strip_prefix("- [X] "));
    if let Some(after) = task {
        return Some(ListMarker {
            next: format!("{ws}- [ ] "),
            empty: after.trim().is_empty(),
        });
    }
    if let Some(after) = rest.strip_prefix("- ") {
        return Some(ListMarker {
            next: format!("{ws}- "),
            empty: after.trim().is_empty(),
        });
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        if let Some(after) = rest[digits..].strip_prefix(". ") {
            let n: u64 = rest[..digits].parse().ok()?;
            return Some(ListMarker {
                next: format!("{ws}{}. ", n.saturating_add(1)),
                empty: after.trim().is_empty(),
            });
        }
    }
    if let Some(after) = rest.strip_prefix("> ") {
        return Some(ListMarker {
            next: format!("{ws}> "),
            empty: after.trim().is_empty(),
        });
    }
    if rest == ">" {
        return Some(ListMarker {
            next: format!("{ws}> "),
            empty: true,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(text: &str, caret: usize) -> Editor {
        let mut e = Editor::new(text);
        assert!(e.set_caret(caret));
        e
    }

    // ---- CRLF 编辑一致性（阶段 2 源码真值：不引入隐式规范化）----

    #[test]
    fn newline_uses_crlf_in_crlf_document() {
        // 行尾回车：沿用文档 CRLF，不在 `\r` 前插入 LF 造出空行或混合换行
        let mut e = ed("a\r\nb", 1); // a 之后、`\r` 之前（视觉行尾）
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "a\r\n\r\nb");
        // 下一行行首回车同样给出 CRLF
        let mut e = ed("a\r\nb", 3);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "a\r\n\r\nb");
    }

    #[test]
    fn newline_keeps_lf_in_lf_document() {
        let mut e = ed("a\nb", 1);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "a\n\nb");
    }

    #[test]
    fn newline_replaces_selection_with_document_line_ending() {
        let mut e = ed("a\r\nb", 0);
        e.apply(Intent::SelectRight).unwrap(); // 选中 "a"
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "\r\n\r\nb");
    }

    #[test]
    fn newline_continues_crlf_list() {
        let mut e = ed("- a", 3);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "- a\n- "); // LF 文档照旧
        let mut e = ed("- a\r\n", 3);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "- a\r\n- \r\n");
    }

    #[test]
    fn backspace_at_crlf_line_start_removes_whole_break() {
        // 行首退格必须吃掉整个 `\r\n`，否则留下孤立 `\r`
        let mut e = ed("a\r\nb", 3);
        e.apply(Intent::DeleteBackward).unwrap();
        assert_eq!(e.session().serialize(), "ab");
    }

    #[test]
    fn delete_forward_at_crlf_removes_whole_break() {
        let mut e = ed("a\r\nb", 1);
        e.apply(Intent::DeleteForward).unwrap();
        assert_eq!(e.session().serialize(), "ab");
    }

    #[test]
    fn line_bounds_excludes_crlf() {
        // 行内容不含 `\r`：三击选行只含正文
        let mut e = ed("ab\r\ncd", 0);
        assert!(e.select_line_at(0));
        assert_eq!(
            e.session().text_slice(Range::new(0, 2)).as_deref(),
            Some("ab")
        );
        assert_eq!(e.selection(), Some(Range::new(0, 2)));
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

    // ---- 全选（Ctrl+A）----

    #[test]
    fn select_all_covers_whole_document() {
        let mut e = ed("ab\ncd", 3); // 光标在中部
        e.apply(Intent::SelectAll).unwrap();
        assert_eq!(e.selection(), Some(Range::new(0, 5)));
        assert_eq!(e.caret(), 5, "caret 落文末，Shift+← 可自尾回收");
    }

    #[test]
    fn select_all_then_insert_replaces_document() {
        let mut e = ed("ab\ncd", 0);
        e.apply(Intent::SelectAll).unwrap();
        e.apply(Intent::InsertText("x".into())).unwrap();
        assert_eq!(e.session().serialize(), "x", "全选后输入整体替换");
        assert_eq!(e.selection(), None);
        assert_eq!(e.caret(), 1);
    }

    #[test]
    fn select_all_then_undo_restores_document() {
        let mut e = ed("ab\ncd", 0);
        e.apply(Intent::SelectAll).unwrap();
        e.apply(Intent::InsertText("x".into())).unwrap();
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), "ab\ncd", "整体替换是单撤销步");
    }

    #[test]
    fn select_all_on_empty_document_is_noop() {
        let mut e = Editor::new("");
        assert_eq!(e.apply(Intent::SelectAll), None);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn select_all_does_not_dirty() {
        let mut e = ed("ab", 1);
        e.apply(Intent::SelectAll);
        assert!(!e.is_dirty(), "全选不是内容编辑，不置脏");
    }

    // ---- 按词移动 / 扩选（C6）----

    #[test]
    fn ctrl_arrows_stop_at_word_boundaries() {
        let mut e = ed("foo bar_baz 中文字", 0);
        e.apply(Intent::WordRight).unwrap();
        assert_eq!(e.caret(), 3);
        e.apply(Intent::WordRight).unwrap();
        assert_eq!(e.caret(), 11);
        e.apply(Intent::WordRight).unwrap();
        assert_eq!(e.caret(), 21, "CJK 连续串按一个词");
        assert_eq!(e.apply(Intent::WordRight), None, "文末向右无效果");
        e.apply(Intent::WordLeft).unwrap();
        assert_eq!(e.caret(), 12);
        e.apply(Intent::WordLeft).unwrap();
        assert_eq!(e.caret(), 4);
        e.apply(Intent::WordLeft).unwrap();
        assert_eq!(e.caret(), 0);
        assert_eq!(e.apply(Intent::WordLeft), None, "文首向左无效果");
    }

    #[test]
    fn ctrl_shift_arrows_extend_by_word() {
        let mut e = ed("foo bar", 5);
        e.apply(Intent::SelectWordRight).unwrap();
        assert_eq!(e.selection(), Some(Range::new(5, 7)));
        assert_eq!(
            e.apply(Intent::SelectWordRight),
            None,
            "已在词尾：目标不动即无效果"
        );
        e.apply(Intent::SelectWordLeft).unwrap();
        assert_eq!(e.selection(), Some(Range::new(4, 5)), "反向扩选收缩");
        e.apply(Intent::SelectWordLeft).unwrap();
        assert_eq!(e.selection(), Some(Range::new(0, 5)));
    }

    #[test]
    fn word_move_with_selection_collapses_to_selection_edge() {
        let mut e = ed("foo bar", 0);
        e.apply(Intent::SelectAll).unwrap();
        e.apply(Intent::WordRight).unwrap();
        assert_eq!(e.selection(), None);
        assert_eq!(e.caret(), 7, "有选区时先坍缩到选区末端");
    }

    #[test]
    fn double_click_selects_word_space_and_handles_line_end() {
        let mut e = ed("foo bar  baz\nqux", 0);
        assert!(e.select_word_at(5));
        assert_eq!(e.selection(), Some(Range::new(4, 7)));
        assert!(e.select_word_at(8));
        assert_eq!(e.selection(), Some(Range::new(7, 9)), "空白串不跨行");
        assert!(e.select_word_at(12));
        assert_eq!(
            e.selection(),
            Some(Range::new(9, 12)),
            "行尾换行处按前一字符取词"
        );
        assert!(e.select_word_at(15));
        assert_eq!(e.selection(), Some(Range::new(13, 16)));
    }

    #[test]
    fn double_click_cjk_run_is_one_word() {
        let mut e = ed("中文字 abc", 0);
        assert!(e.select_word_at(4)); // 「文」内（字节 3..6）
        assert_eq!(e.selection(), Some(Range::new(0, 9)));
        assert!(e.select_word_at(1), "非字符边界退到所在字符起点");
        assert_eq!(e.selection(), Some(Range::new(0, 9)));
    }

    #[test]
    fn double_click_on_empty_line_selects_nothing() {
        let mut e = ed("a\n\nb", 0);
        assert!(!e.select_word_at(2), "空行无可选内容");
    }

    #[test]
    fn triple_click_selects_line_without_newline() {
        let mut e = ed("ab\ncd\n", 0);
        assert!(e.select_line_at(1));
        assert_eq!(e.selection(), Some(Range::new(0, 2)));
        assert!(e.select_line_at(4));
        assert_eq!(e.selection(), Some(Range::new(3, 5)));
        assert!(!e.select_line_at(6), "文末幻影空行无可选内容");
    }

    // ---- 缩进 / 反缩进（C3）----

    #[test]
    fn tab_without_selection_inserts_four_spaces() {
        let mut e = ed("ab", 1);
        assert!(e.apply(Intent::Indent).is_some());
        assert_eq!(e.session().serialize(), "a    b");
        assert_eq!(e.caret(), 5, "光标停在插入空白之后");
        assert!(e.is_dirty());
    }

    #[test]
    fn tab_indents_touched_lines_in_one_undo_step() {
        let mut e = ed("ab\ncd", 0);
        e.apply(Intent::SelectAll).unwrap();
        assert!(e.apply(Intent::Indent).is_some());
        assert_eq!(
            e.session().serialize(),
            "  ab\n  cd",
            "段落整行缩进用 2 空格，不触发 4 空格缩进代码块"
        );
        assert_eq!(e.caret(), 9, "caret 含行首缩进（Bias::Right）");
        assert_eq!(
            e.selection(),
            Some(Range::new(0, 9)),
            "anchor 留在新行首之前，缩进仍在选区内"
        );
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), "ab\ncd", "多行缩进是单撤销步");
    }

    #[test]
    fn indent_skips_empty_lines_and_trailing_phantom_line() {
        let mut e = ed("a\n\nb", 0);
        e.apply(Intent::SelectAll).unwrap();
        e.apply(Intent::Indent).unwrap();
        assert_eq!(e.session().serialize(), "  a\n\n  b", "空行不缩进");

        let mut e = ed("ab\n", 0);
        e.apply(Intent::SelectAll).unwrap();
        e.apply(Intent::Indent).unwrap();
        assert_eq!(
            e.session().serialize(),
            "  ab\n",
            "文末换行后的幻影行不产生尾随空格"
        );
    }

    #[test]
    fn tab_indents_only_the_selected_line() {
        let mut e = ed("ab\ncd", 3);
        e.set_caret(3);
        assert!(e.set_caret_extend(5));
        assert!(e.apply(Intent::Indent).is_some());
        assert_eq!(e.session().serialize(), "ab\n  cd");
        assert_eq!(e.selection(), Some(Range::new(3, 7)));
    }

    #[test]
    fn dedent_strips_up_to_four_spaces_or_one_tab() {
        let mut e = ed("    a\n  b\n\tc\nd", 0);
        e.apply(Intent::SelectAll).unwrap();
        assert!(e.apply(Intent::Dedent).is_some());
        assert_eq!(e.session().serialize(), "a\nb\nc\nd");
    }

    #[test]
    fn dedent_without_leading_whitespace_is_noop() {
        let mut e = ed("abcd", 0);
        assert_eq!(e.apply(Intent::Dedent), None);
        assert_eq!(e.session().serialize(), "abcd");
        assert!(!e.is_dirty(), "无变动不置脏");
    }

    #[test]
    fn dedent_collapsed_caret_keeps_relative_position() {
        let mut e = ed("  ab", 3);
        assert!(e.apply(Intent::Dedent).is_some());
        assert_eq!(e.session().serialize(), "ab");
        assert_eq!(e.caret(), 1, "光标随被删前缀左移");
    }

    // ---- 列表续行（C4）----

    #[test]
    fn newline_continues_unordered_list() {
        let mut e = ed("- a", 3);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "- a\n- ");
        assert_eq!(e.caret(), 6);
    }

    #[test]
    fn newline_increments_ordered_list() {
        let mut e = ed("9. a", 4);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "9. a\n10. ");
    }

    #[test]
    fn newline_continues_blockquote_and_task_list() {
        let mut e = ed("> q", 3);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "> q\n> ");

        let mut e = ed("- [x] done", 10);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(
            e.session().serialize(),
            "- [x] done\n- [ ] ",
            "任务项续行一律未勾选"
        );
    }

    #[test]
    fn newline_preserves_list_indentation() {
        let mut e = ed("  - a", 5);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "  - a\n  - ");
    }

    #[test]
    fn newline_on_empty_item_ends_list() {
        let mut e = ed("- ", 2);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "", "空项回车删标记收束列表");
        assert_eq!(e.caret(), 0);

        let mut e = ed("1. \ntext", 3);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "\ntext");
        assert_eq!(e.caret(), 0);
    }

    #[test]
    fn newline_mid_line_does_not_continue_list() {
        let mut e = ed("- ab", 2);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "- \nab");
    }

    #[test]
    fn newline_on_plain_text_is_plain() {
        let mut e = ed("abc", 3);
        e.apply(Intent::Newline).unwrap();
        assert_eq!(e.session().serialize(), "abc\n");
        assert_eq!(e.caret(), 4);
    }

    // ---- 行内格式（3c-1）：ToggleWrap 包裹切换 + InsertLink ----

    /// 造带选区的编辑器：anchor 定锚、caret 扩选到 caret。
    fn sel_ed(text: &str, anchor: usize, caret: usize) -> Editor {
        let mut e = Editor::new(text);
        assert!(e.set_caret(anchor));
        assert!(e.set_caret_extend(caret));
        e
    }

    #[test]
    fn bold_wraps_selection_single_undo_step() {
        let mut e = sel_ed("hello world", 0, 5);
        e.apply(Intent::ToggleWrap("**")).unwrap();
        assert_eq!(e.session().serialize(), "**hello** world");
        assert_eq!(e.caret(), 9, "光标停在包裹文本之后");
        assert!(e.selection().is_none());
        assert!(e.is_dirty());
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), "hello world", "包裹是单撤销步");
    }

    #[test]
    fn bold_unwraps_already_wrapped_selection() {
        let mut e = sel_ed("**hello** rest", 0, 9);
        e.apply(Intent::ToggleWrap("**")).unwrap();
        assert_eq!(e.session().serialize(), "hello rest");
        assert_eq!(e.caret(), 5);
    }

    #[test]
    fn italic_on_bold_stacks_instead_of_unwrapping() {
        // 选区恰为 `**b**`：单星 marker 不得误拆双星包裹，再包一层成加粗斜体
        let mut e = sel_ed("**b**", 0, 5);
        e.apply(Intent::ToggleWrap("*")).unwrap();
        assert_eq!(e.session().serialize(), "***b***");
    }

    #[test]
    fn italic_on_cjk_selection_does_not_panic() {
        // 回归：旧实现用 text[1..] / text[..len-1] 判双包裹，选区首尾是 CJK 时
        // 字节下标不在字符边界 → panic。Ctrl+I 选中中文即崩溃。
        let src = "混合元素";
        let mut e = sel_ed(src, 0, src.len());
        e.apply(Intent::ToggleWrap("*")).unwrap();
        assert_eq!(e.session().serialize(), format!("*{src}*"));
        assert_eq!(e.caret(), src.len() + 2);
    }

    #[test]
    fn italic_on_cjk_bold_stacks_instead_of_unwrapping() {
        let src = "**加粗**";
        let mut e = sel_ed(src, 0, src.len());
        e.apply(Intent::ToggleWrap("*")).unwrap();
        assert_eq!(e.session().serialize(), format!("*{src}*"));
    }

    #[test]
    fn empty_selection_inserts_pair_second_press_removes() {
        let mut e = ed("ab", 1);
        e.apply(Intent::ToggleWrap("**")).unwrap();
        assert_eq!(e.session().serialize(), "a****b");
        assert_eq!(e.caret(), 3, "光标居中于标记对");
        e.apply(Intent::ToggleWrap("**")).unwrap();
        assert_eq!(e.session().serialize(), "ab", "光标在标记对中再按一次删除标记对");
        assert_eq!(e.caret(), 1);
    }

    /// I3 验证：表格单元格即普通源码字节区间（cell 特判仅影响渲染与 Tab 导航），
    /// ToggleWrap 在 cell 内必须在源码层面生效且为单撤销步；「疑似无效」是渲染层
    /// 不解析行内样式造成的视觉错觉，而非编辑路径失败。
    #[test]
    fn toggle_wrap_inside_table_cell_edits_source() {
        let src = "| a | b |\n|---|---|\n| c | d |";
        let c = src.find("| c |").unwrap() + 2; // 数据行 cell 文本 'c' 的字节起点

        // 选中 cell 文本：包裹为 *c*，undo 一次还原
        let mut e = sel_ed(src, c, c + 1);
        assert!(
            e.apply(Intent::ToggleWrap("*")).is_some(),
            "cell 内 ToggleWrap 须在源码层面生效"
        );
        assert_eq!(e.session().serialize(), "| a | b |\n|---|---|\n| *c* | d |");
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), src, "cell 内包裹是单撤销步");

        // 空选区 caret 落在 cell 内（'c' 之后）：插入标记对、光标居中
        let mut e = ed(src, c + 1);
        assert!(e.apply(Intent::ToggleWrap("*")).is_some());
        assert_eq!(e.session().serialize(), "| a | b |\n|---|---|\n| c** | d |");
        assert_eq!(e.caret(), c + 2, "光标居中于标记对");
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), src);
    }

    #[test]
    fn link_wraps_selection_caret_in_parens() {
        let mut e = sel_ed("see here", 4, 8);
        e.apply(Intent::InsertLink).unwrap();
        assert_eq!(e.session().serialize(), "see [here]()");
        assert_eq!(e.caret(), 11, "光标入 `()` 便于直接输 URL");
        assert!(e.is_dirty());
        e.apply(Intent::Undo).unwrap();
        assert_eq!(e.session().serialize(), "see here", "插链接是单撤销步");
    }

    #[test]
    fn link_empty_selection_inserts_skeleton_caret_in_brackets() {
        let mut e = ed("ab", 1);
        e.apply(Intent::InsertLink).unwrap();
        assert_eq!(e.session().serialize(), "a[]()b");
        assert_eq!(e.caret(), 2, "光标入 `[]` 便于直接输链接文本");
    }
}
