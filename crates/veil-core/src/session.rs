//! 门面：视图层的唯一入口（docs/01-define-design.md B2）。
//! 视图层每次交互 ≤3 行：翻译事件 → `Session::apply` → 按 `Events` 失效重绘。

use crate::buffer::Buffer;
use crate::history::{History, PrimEdit, Transaction};
use crate::range::{Bias, MarkId, Range};
use crate::syntax::{Block, BlockId, BlockKind, InlineNode, Syntax, Table};

/// 一次文本变更。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    Insert { at: usize, text: String },
    Delete { range: Range },
    Replace { range: Range, text: String },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditError {
    /// 区间越界或 start > end。
    OutOfRange,
    /// 偏移未落在 UTF-8 字符边界上。
    NotCharBoundary,
}

/// 一次变更后视图需要知道的一切。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Events {
    /// 需要重建的脏块；未列出的块保证未变（含 id 不变）。
    pub dirty_blocks: Vec<BlockId>,
}

pub struct Session {
    buf: Buffer,
    syntax: Syntax,
    history: History,
}

impl Session {
    pub fn open(text: &str) -> Self {
        let buf = Buffer::new(text);
        let syntax = Syntax::new(&buf);
        Session {
            buf,
            syntax,
            history: History::default(),
        }
    }

    /// 序列化回 Markdown 源文本。无编辑时恒等于输入（INSTANCE §3 幂等不变量）。
    pub fn serialize(&self) -> String {
        self.buf.text()
    }

    pub fn apply(&mut self, edit: Edit) -> Result<Events, EditError> {
        let (start, end, text) = match &edit {
            Edit::Insert { at, text } => (*at, *at, text.as_str()),
            Edit::Delete { range } => (range.start, range.end, ""),
            Edit::Replace { range, text } => (range.start, range.end, text.as_str()),
        };
        self.check_bounds(start, end)?;
        if start == end && text.is_empty() {
            // 空操作：不进撤销栈，不产生脏块
            return Ok(Events {
                dirty_blocks: Vec::new(),
            });
        }
        Ok(Events {
            dirty_blocks: self.apply_primitive(start, end, text),
        })
    }

    pub fn undo(&mut self) -> Option<Events> {
        self.revert(true)
    }

    pub fn redo(&mut self) -> Option<Events> {
        self.revert(false)
    }

    pub fn create_mark(&mut self, offset: usize, bias: Bias) -> Result<MarkId, EditError> {
        if offset > self.buf.len() {
            return Err(EditError::OutOfRange);
        }
        if !self.buf.is_char_boundary(offset) {
            return Err(EditError::NotCharBoundary);
        }
        Ok(self.buf.create_mark(offset, bias))
    }

    pub fn remove_mark(&mut self, id: MarkId) {
        self.buf.remove_mark(id);
    }

    pub fn mark_offset(&self, id: MarkId) -> Option<usize> {
        self.buf.offset_of(id)
    }

    pub fn text_len(&self) -> usize {
        self.buf.len()
    }

    /// offset 所在行号（0-based）。供视图层估算块高时避免 text_slice 全量拷贝。
    pub fn line_of_offset(&self, offset: usize) -> usize {
        self.buf.line_of_offset(offset.min(self.buf.len()))
    }

    /// 光标行列（1-based，列按字符计数而非字节）。offset 越界或非字符边界返回 None。
    pub fn line_col(&self, offset: usize) -> Option<(usize, usize)> {
        if offset > self.buf.len() || !self.buf.is_char_boundary(offset) {
            return None;
        }
        let line = self.buf.line_of_offset(offset);
        let start = self.buf.offset_of_line(line);
        let col = self.buf.slice(start, offset).chars().count();
        Some((line + 1, col + 1))
    }

    /// 按需取文本切片（视图按块取词，避免全量 serialize）。越界或非字符边界返回 None。
    pub fn text_slice(&self, range: Range) -> Option<String> {
        if range.start > range.end || range.end > self.buf.len() {
            return None;
        }
        if !self.buf.is_char_boundary(range.start) || !self.buf.is_char_boundary(range.end) {
            return None;
        }
        Some(self.buf.slice(range.start, range.end))
    }

    /// grapheme 光标导航（视图层左右移动）。offset 越界或非字符边界返回 None。
    pub fn prev_grapheme(&self, offset: usize) -> Option<usize> {
        if offset > self.buf.len() || !self.buf.is_char_boundary(offset) {
            return None;
        }
        self.buf.prev_grapheme(offset)
    }

    pub fn next_grapheme(&self, offset: usize) -> Option<usize> {
        if offset > self.buf.len() || !self.buf.is_char_boundary(offset) {
            return None;
        }
        self.buf.next_grapheme(offset)
    }

    /// 块结构快照：(id, 种类, 字节区间)。
    pub fn blocks(&self) -> Vec<(BlockId, BlockKind, Range)> {
        let total = self.buf.line_count();
        self.syntax
            .blocks()
            .iter()
            .map(|b: &Block| {
                let start = self.buf.offset_of_line(b.start_line);
                let end = if b.end_line >= total {
                    self.buf.len()
                } else {
                    self.buf.offset_of_line(b.end_line)
                };
                (b.id, b.kind.clone(), Range::new(start, end))
            })
            .collect()
    }

    /// 某块的 inline 节点快照（绝对字节偏移）。派生数据，随块结构自动正确；
    /// FencedCode/Math 恒为空，未知 id 返回 None。
    pub fn block_inlines(&self, block: BlockId) -> Option<Vec<InlineNode>> {
        self.syntax.inlines(&self.buf, block)
    }

    /// 数学块的 TeX 源码（剥开闭 `$$` 标记）。非数学块/未知 id 返回 None。
    pub fn math_source(&self, block: BlockId) -> Option<String> {
        self.syntax.math_source(&self.buf, block)
    }

    /// 标题块派生数据：(级别, 可见文本)。非标题块/未知 id 返回 None。
    pub fn heading(&self, block: BlockId) -> Option<(u8, String)> {
        self.syntax.heading(&self.buf, block)
    }

    /// GFM 表格派生数据（列对齐 + 单元格文本/区间）。非表格块/未知 id 返回 None。
    pub fn table(&self, block: BlockId) -> Option<Table> {
        self.syntax.table(&self.buf, block)
    }

    fn check_bounds(&self, start: usize, end: usize) -> Result<(), EditError> {
        if start > end || end > self.buf.len() {
            return Err(EditError::OutOfRange);
        }
        if !self.buf.is_char_boundary(start) || !self.buf.is_char_boundary(end) {
            return Err(EditError::NotCharBoundary);
        }
        Ok(())
    }

    /// 应用一次原语编辑：文本 → mark 变换 → 增量重解析 → 记录历史。返回脏块。
    fn apply_primitive(&mut self, start: usize, end: usize, text: &str) -> Vec<BlockId> {
        let old_start_line = self.buf.line_of_offset(start);
        let old_end_line = self.buf.line_of_offset(end);
        let deleted = self.buf.edit(start, end, text);
        let new_end_line = self.buf.line_of_offset(start + text.len());
        let dirty = self
            .syntax
            .reparse(&self.buf, old_start_line, old_end_line, new_end_line);
        self.history.record(Transaction {
            prims: vec![PrimEdit {
                start,
                deleted,
                inserted: text.to_owned(),
            }],
        });
        dirty
    }

    /// 撤销/重做共享路径：逆序应用事务内各原语的逆操作，
    /// 逆操作序列本身构成反向事务压入另一栈（互换 deleted/inserted 即自逆）。
    fn revert(&mut self, undoing: bool) -> Option<Events> {
        let tx = if undoing {
            self.history.pop_undo()
        } else {
            self.history.pop_redo()
        }?;
        let mut inverse = Transaction::default();
        let mut dirty = Vec::new();
        for prim in tx.prims.iter().rev() {
            let end = prim.start + prim.inserted.len();
            let old_start_line = self.buf.line_of_offset(prim.start);
            let old_end_line = self.buf.line_of_offset(end);
            let deleted_now = self.buf.edit(prim.start, end, &prim.deleted);
            let new_end_line = self.buf.line_of_offset(prim.start + prim.deleted.len());
            dirty.extend(self.syntax.reparse(
                &self.buf,
                old_start_line,
                old_end_line,
                new_end_line,
            ));
            inverse.prims.push(PrimEdit {
                start: prim.start,
                deleted: deleted_now,
                inserted: prim.deleted.clone(),
            });
        }
        if undoing {
            self.history.push_redo(inverse);
        } else {
            self.history.push_undo(inverse);
        }
        Some(Events {
            dirty_blocks: dirty,
        })
    }
}
