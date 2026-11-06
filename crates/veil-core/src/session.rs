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

    /// 显式断开撤销分组：光标移动/选区变化后，下一次编辑另起一步（VS Code 语义）。
    pub fn break_undo_group(&mut self) {
        self.history.break_group();
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

    /// 行首字节偏移（Buffer 直接转发）。供视图层快速定位行边界。
    pub fn offset_of_line(&self, line: usize) -> usize {
        self.buf.offset_of_line(line)
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

    /// 块结构快照：(id, 种类, 字节区间)。区间由语法层增量维护，此处只做结构拷贝。
    pub fn blocks(&self) -> Vec<(BlockId, BlockKind, Range)> {
        self.syntax
            .blocks()
            .iter()
            .map(|b: &Block| (b.id, b.kind.clone(), b.range))
            .collect()
    }

    /// 当前块总数（只读观测值，供性能探针/测试；不构建快照）。
    pub fn block_count(&self) -> usize {
        self.syntax.blocks().len()
    }

    /// 最近一次编辑的增量重扫行数（只读观测值）。口径见 `Syntax::last_rescan_lines`。
    pub fn last_rescan_lines(&self) -> usize {
        self.syntax.last_rescan_lines()
    }

    /// 当前可撤销步数（只读观测值，不改变撤销分组/裁剪语义）。
    pub fn undo_depth(&self) -> usize {
        self.history.undo_len()
    }

    /// 当前可重做步数（只读观测值）：菜单「重做」禁用态的唯一数据源。
    pub fn redo_depth(&self) -> usize {
        self.history.redo_len()
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

    /// 脚注编号（1 起，按文档顺序首次引用）。未被引用或未知 id 返回 None。
    pub fn footnote_number(&self, id: &str) -> Option<usize> {
        self.syntax.footnote_number(id)
    }

    /// 已编号脚注 id（文档顺序，下标 + 1 = 编号）。
    pub fn footnote_ids(&self) -> Vec<String> {
        self.syntax.footnote_ids().to_vec()
    }

    /// 脚注首个定义的首行内容区间（重复定义 first wins）；无定义返回 None。
    pub fn footnote_definition(&self, id: &str) -> Option<Range> {
        self.syntax.footnote_definition(id)
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
            prim: PrimEdit {
                start,
                deleted,
                inserted: text.to_owned(),
            },
        });
        dirty
    }

    /// 撤销/重做共享路径：应用事务的逆操作，
    /// 逆操作本身构成反向事务压入另一栈（互换 deleted/inserted 即自逆）。
    fn revert(&mut self, undoing: bool) -> Option<Events> {
        let tx = if undoing {
            self.history.pop_undo()
        } else {
            self.history.pop_redo()
        }?;
        let prim = tx.prim;
        let end = prim.start + prim.inserted.len();
        let old_start_line = self.buf.line_of_offset(prim.start);
        let old_end_line = self.buf.line_of_offset(end);
        let deleted_now = self.buf.edit(prim.start, end, &prim.deleted);
        let new_end_line = self.buf.line_of_offset(prim.start + prim.deleted.len());
        let dirty = self.syntax.reparse(
            &self.buf,
            old_start_line,
            old_end_line,
            new_end_line,
        );
        let inverse = Transaction {
            prim: PrimEdit {
                start: prim.start,
                deleted: deleted_now,
                inserted: prim.deleted,
            },
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(at: usize, text: &str) -> Edit {
        Edit::Insert {
            at,
            text: text.into(),
        }
    }

    #[test]
    fn break_undo_group_splits_typing() {
        let mut s = Session::open("");
        s.apply(insert(0, "a")).unwrap();
        s.apply(insert(1, "b")).unwrap();
        s.break_undo_group();
        s.apply(insert(2, "c")).unwrap();
        assert_eq!(s.serialize(), "abc");
        s.undo().unwrap();
        assert_eq!(s.serialize(), "ab", "断组后仅回退 c");
        s.undo().unwrap();
        assert_eq!(s.serialize(), "");
        assert!(s.undo().is_none());
    }

    #[test]
    fn typing_without_break_stays_single_step() {
        let mut s = Session::open("");
        for (i, ch) in "abc".chars().enumerate() {
            s.apply(insert(i, &ch.to_string())).unwrap();
        }
        s.undo().unwrap();
        assert_eq!(s.serialize(), "", "未断组时连续输入仍合并为一步");
        assert!(s.undo().is_none());
    }

    // ===== 块字节区间：基本自洽 / 编辑序列 / 开头平移 =====

    /// 含中文多字节、列表、引用、表格、公式、围栏代码的多块文档。
    const RICH: &str = "# 标题\n\n中文 **粗体** 段落\n\n- 项目一\n  - 嵌套\n\n> 引用 *斜体*\n\n\
                        | a | b |\n| --- | --- |\n| 中 | 文 |\n\n$$\nx + y\n$$\n\n\
                        ```rust\nfn f() {}\n```\n\n末尾\n";

    /// 全量校验：公开快照的 Range 与内部行号换算一致、边界是字符边界、
    /// `text_slice` 可用、块间不重叠且除块间空行外覆盖全文。
    fn assert_ranges_consistent(s: &Session) {
        let text = s.serialize();
        let total = s.buf.line_count();
        let raw = s.syntax.blocks();
        let public = s.blocks();
        assert_eq!(raw.len(), public.len(), "公开块数与内部块数不一致");
        let mut prev_end = 0usize;
        for (b, (_, _, r)) in raw.iter().zip(public.iter()) {
            assert_eq!(*r, b.range, "公开 Range 未直接取自 Block.range");
            assert_eq!(
                b.range.start,
                s.buf.offset_of_line(b.start_line),
                "Block.start 不是 start_line 首字节"
            );
            let want_end = if b.end_line >= total {
                text.len()
            } else {
                s.buf.offset_of_line(b.end_line)
            };
            assert_eq!(b.range.end, want_end, "Block.end 不是 end_line 首字节");
            assert!(b.range.start <= b.range.end && b.range.end <= text.len());
            assert!(
                text.is_char_boundary(b.range.start) && text.is_char_boundary(b.range.end),
                "块边界不在字符边界上"
            );
            assert!(
                s.text_slice(b.range).is_some(),
                "text_slice 拒绝块区间 {:?}",
                b.range
            );
            assert!(b.range.start >= prev_end, "块区间重叠：{b:?}");
            assert!(
                text[prev_end..b.range.start]
                    .chars()
                    .all(|c| c.is_whitespace()),
                "块间空隙含非空白内容"
            );
            prev_end = b.range.end;
        }
        assert!(
            text[prev_end..].chars().all(|c| c.is_whitespace()),
            "文末空隙含非空白内容"
        );
    }

    fn replace_first(s: &mut Session, needle: &str, repl: &str) {
        let pos = s
            .serialize()
            .find(needle)
            .unwrap_or_else(|| panic!("语料缺少 {needle:?}"));
        s.apply(Edit::Replace {
            range: Range::new(pos, pos + needle.len()),
            text: repl.to_owned(),
        })
        .unwrap();
    }

    #[test]
    fn blocks_ranges_self_consistent_across_multi_block_doc() {
        let s = Session::open(RICH);
        assert!(s.blocks().len() >= 6, "语料应产生多块");
        assert_ranges_consistent(&s);
    }

    #[test]
    fn blocks_ranges_stay_consistent_through_edit_sequence() {
        for _round in 0..3 {
            let mut s = Session::open(RICH);
            assert_ranges_consistent(&s);

            // 1 多字节插入
            let pos = s.serialize().find("中文").unwrap();
            s.apply(insert(pos, "字")).unwrap();
            assert_ranges_consistent(&s);

            // 2 多字节删除
            let pos = s.serialize().find("字中文").unwrap();
            s.apply(Edit::Delete {
                range: Range::new(pos, pos + 3),
            })
            .unwrap();
            assert_ranges_consistent(&s);

            // 3 同长替换
            replace_first(&mut s, "粗体", "斜体");
            assert_ranges_consistent(&s);

            // 4 段内插入换行
            let pos = s.serialize().find("段落").unwrap() + "段落".len();
            s.apply(insert(pos, "\n")).unwrap();
            assert_ranges_consistent(&s);

            // 5 删除换行（段落合回）
            s.apply(Edit::Delete {
                range: Range::new(pos, pos + 1),
            })
            .unwrap();
            assert_ranges_consistent(&s);

            // 6 段落 → 列表
            let pos = s.serialize().find("中文").unwrap();
            s.apply(insert(pos, "- ")).unwrap();
            assert_ranges_consistent(&s);

            // 7 列表 → 段落
            let pos = s.serialize().find("- 中文").unwrap();
            s.apply(Edit::Delete {
                range: Range::new(pos, pos + 2),
            })
            .unwrap();
            assert_ranges_consistent(&s);

            // 8 列表缩进变化
            replace_first(&mut s, "  - 嵌套", "- 嵌套");
            assert_ranges_consistent(&s);

            // 9 删列表与引用间空行（触发列表区域回走重扫）
            replace_first(&mut s, "- 嵌套\n\n>", "- 嵌套\n>");
            assert_ranges_consistent(&s);

            // 10 引用内容编辑
            replace_first(&mut s, "引用", "引用x");
            assert_ranges_consistent(&s);

            // 11 表格单元格
            replace_first(&mut s, "| 中 |", "| 文 |");
            assert_ranges_consistent(&s);

            // 12 公式内容
            replace_first(&mut s, "x + y", "x + y + z");
            assert_ranges_consistent(&s);

            // 13 围栏代码
            replace_first(&mut s, "fn f() {}", "fn g() {}");
            assert_ranges_consistent(&s);

            // 14 整块删除
            replace_first(&mut s, "> 引用x *斜体*\n\n", "");
            assert_ranges_consistent(&s);
        }
    }

    #[test]
    fn blocks_ranges_shift_uniformly_on_prefix_insert() {
        let mut s = Session::open(RICH);
        let before: Vec<(usize, usize, String)> = s
            .blocks()
            .into_iter()
            .map(|(_, _, r)| (r.start, r.end, s.text_slice(r).unwrap()))
            .collect();

        // 文档开头插 1 字节：插入点所在的第 0 块起点不变、终点 +1；
        // 其后所有块的 start/end 必须整体 +1（它们完全在插入点之后）。
        s.apply(insert(0, "X")).unwrap();
        let after = s.blocks();
        assert_eq!(after.len(), before.len(), "开头插 1 字节不应改变块数");
        assert_eq!(
            (after[0].2.start, after[0].2.end),
            (before[0].0, before[0].1 + 1),
            "含插入点的首块终点未按 delta=+1 平移"
        );
        assert_eq!(
            s.text_slice(after[0].2).as_deref(),
            Some(format!("X{}", before[0].2).as_str()),
            "首块内容应等于插入物后接原文"
        );
        for (i, (_, _, r)) in after.iter().enumerate().skip(1) {
            assert_eq!(
                (r.start, r.end),
                (before[i].0 + 1, before[i].1 + 1),
                "第 {i} 块未按 delta=+1 整体平移"
            );
            assert_eq!(
                s.text_slice(*r).as_deref(),
                Some(before[i].2.as_str()),
                "第 {i} 块平移后内容不一致"
            );
        }
        assert_ranges_consistent(&s);
    }

    // ===== P0 观测值：block_count / last_rescan_lines / undo_depth =====

    #[test]
    fn observability_values_track_edits() {
        let mut s = Session::open("# 标题\n\n第一段\n\n第二段\n");
        let blocks_before = s.block_count();
        assert!(blocks_before >= 2, "多块文档应产生多个块");
        assert_eq!(s.last_rescan_lines(), 0, "首扫不计增量重扫行数");
        assert_eq!(s.undo_depth(), 0, "打开文档时无撤销步");

        s.apply(insert(0, "x")).unwrap();
        assert!(s.last_rescan_lines() > 0, "编辑后应记录本次重扫行数");
        assert_eq!(s.undo_depth(), 1, "首次编辑产生一步");
        assert_eq!(s.block_count(), blocks_before, "行内编辑不改变块数");
        assert_eq!(s.serialize(), "x# 标题\n\n第一段\n\n第二段\n");

        // 光标移动断组后，下一次编辑单独成步（不被单字符合并吞掉）
        s.break_undo_group();
        let pos = s.serialize().find("第二段").unwrap();
        s.apply(insert(pos, "y")).unwrap();
        assert!(s.last_rescan_lines() > 0);
        assert_eq!(s.undo_depth(), 2, "断组后应新增一步");
        assert_eq!(s.block_count(), blocks_before);

        s.undo().unwrap();
        assert_eq!(s.undo_depth(), 1, "撤销后步数回落");
        assert_eq!(s.serialize(), "x# 标题\n\n第一段\n\n第二段\n");
    }

    // ===== 脚注编号表：增量重扫必须与全量重建一致 =====

    /// 编号表是派生数据：增量路径与 `Session::open` 全量重建必须给出同一结果。
    fn assert_footnotes_match_fresh(s: &Session) {
        let fresh = Session::open(&s.serialize());
        assert_eq!(s.footnote_ids(), fresh.footnote_ids(), "编号顺序不一致");
        for id in fresh.footnote_ids() {
            assert_eq!(
                s.footnote_number(&id),
                fresh.footnote_number(&id),
                "id={id:?} 编号不一致"
            );
        }
    }

    #[test]
    fn footnote_numbering_consistent_after_incremental_reparse() {
        let mut s = Session::open("[^a] [^b]\n\n[^a]: A\n[^b]: B\n");
        assert_eq!(s.footnote_number("a"), Some(1));
        assert_eq!(s.footnote_number("b"), Some(2));
        assert_footnotes_match_fresh(&s);

        // 开头插入对 b 的引用：b 变成首次引用 → b=1、a=2
        s.apply(insert(0, "[^b] ")).unwrap();
        assert_eq!(s.footnote_number("b"), Some(1));
        assert_eq!(s.footnote_number("a"), Some(2));
        assert_footnotes_match_fresh(&s);

        // 删除 a 的引用后它不再占号（定义仍在也不算引用）
        let pos = s.serialize().find("[^a]").unwrap();
        s.apply(Edit::Delete {
            range: Range::new(pos, pos + 4),
        })
        .unwrap();
        assert_eq!(s.footnote_ids(), vec!["b".to_string()]);
        assert_eq!(s.footnote_number("a"), None);
        assert_eq!(s.footnote_number("b"), Some(1));
        assert_footnotes_match_fresh(&s);

        // 撤销后回到插入态，编号仍须一致
        s.undo().unwrap();
        assert_footnotes_match_fresh(&s);
    }

    #[test]
    fn footnote_definition_visible_through_session() {
        let s = Session::open("[^x]: 内容\n");
        let r = s.footnote_definition("x").expect("脚注定义");
        assert_eq!(s.text_slice(r).as_deref(), Some(" 内容"));
        // 未被引用：不占编号
        assert_eq!(s.footnote_number("x"), None);
        assert!(s.footnote_ids().is_empty());
    }

    /// release 编辑耗时 bench（不引入 criterion）：
    /// `cargo test -p veil-core --release bench_edit_reparse_history -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_edit_reparse_history_100k() {
        use std::time::Instant;

        // 约 10 万字节：2041 个「48 字符 + 空行」段落。
        let line = format!("{}\n\n", "x".repeat(48));
        let doc: String = line.repeat(2041);
        let mut s = Session::open(&doc);
        let mid_line = 1020;
        const ITERS: usize = 50;

        let (mut edit_ns, mut reparse_ns, mut history_ns) = (0u128, 0u128, 0u128);
        for _ in 0..ITERS {
            let start = s.offset_of_line(mid_line) + 10;
            let old_start_line = s.buf.line_of_offset(start);
            let old_end_line = s.buf.line_of_offset(start);

            let t = Instant::now();
            let deleted = s.buf.edit(start, start, "z");
            edit_ns += t.elapsed().as_nanos();

            let new_end_line = s.buf.line_of_offset(start + 1);
            let t = Instant::now();
            let _dirty = s
                .syntax
                .reparse(&s.buf, old_start_line, old_end_line, new_end_line);
            reparse_ns += t.elapsed().as_nanos();

            let t = Instant::now();
            s.history.record(Transaction {
                prim: PrimEdit {
                    start,
                    deleted,
                    inserted: "z".to_owned(),
                },
            });
            history_ns += t.elapsed().as_nanos();
        }

        let per_iter = |total: u128| total as f64 / ITERS as f64 / 1000.0;
        println!(
            "bench 10万字 {ITERS} 次编辑：edit={:.2} µs/次  reparse={:.2} µs/次  history={:.2} µs/次（末次重扫 {} 行）",
            per_iter(edit_ns),
            per_iter(reparse_ns),
            per_iter(history_ns),
            s.last_rescan_lines()
        );
        assert!(s.block_count() >= 2000, "bench 文档应产生大量块");
    }
}
