//! 块级增量解析 + inline 解析。
//!
//! 块级 v0 子集：ATX 标题 / 围栏代码块 / 数学块（pandoc 风格 `$$`）/ GFM 表格 /
//! 列表项 / 引用块 / 分隔线 / 段落（GFM 其余块留待后续增量）。
//! 脏块重同步规则（定义文档反例 1 的落地）：
//! 从脏区所在块起点重扫，向后扩展，直至扫描出的块边界与既有块（平移后）
//! 在「起始行 + 种类」上重新对齐；对齐点之后的旧块整体保留（id 不变）。
//! 围栏代码块吞咽、setext、表格头等向后/向前污染由此规则统一覆盖。
//!
//! inline v0 子集：`**`、`*`、`` ` ``、`[text](url)`、`![alt](url)`、`$math$`，
//! 语义取舍逐条列于 `inline` 模块注释块并钉在测试上。

mod block;
mod inline;
mod model;

pub use model::{BlockId, BlockKind, CellAlign, InlineKind, InlineNode, Table, TableCell};
pub(crate) use model::Block;

use block::{line_text, list_mark, parse_delim_row, parse_table_cells, quote_marker_pos, scan_blocks, scan_one};
use crate::buffer::Buffer;
use inline::parse_inlines;

pub(crate) struct Syntax {
    blocks: Vec<Block>,
    next_id: u64,
}

impl Syntax {
    pub fn new(buf: &Buffer) -> Self {
        let mut syntax = Syntax {
            blocks: Vec::new(),
            next_id: 0,
        };
        let raw = scan_blocks(buf, 0, buf.line_count());
        syntax.blocks = raw
            .into_iter()
            .map(|(kind, s, e)| syntax.fresh_block(kind, s, e))
            .collect();
        syntax
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// 某块的 inline 节点（绝对字节偏移）。派生数据：访问时按块当前文本即时解析，
    /// 不持久化、不进撤销栈——块区间由 reparse 维护，故编辑后查询恒正确。
    /// FencedCode/Math/Table/ThematicBreak 恒为空（无 inline）；未知 id 返回 None。
    /// 列表/引用仅解析标记（`- ` / `> `）之后的内容，标记本身不得被 inline 误认。
    pub fn inlines(&self, buf: &Buffer, block: BlockId) -> Option<Vec<InlineNode>> {
        let b = self.blocks.iter().find(|b| b.id == block)?;
        if matches!(
            b.kind,
            BlockKind::FencedCode | BlockKind::Math | BlockKind::Table | BlockKind::ThematicBreak
        ) {
            return Some(Vec::new());
        }
        let start = buf.offset_of_line(b.start_line);
        let end = if b.end_line < buf.line_count() {
            buf.offset_of_line(b.end_line)
        } else {
            buf.len()
        };
        let text = buf.slice(start, end);
        Some(match b.kind {
            BlockKind::ListItem { .. } => {
                let cs = list_mark(&text).map(|m| m.content_start).unwrap_or(0);
                parse_inlines(&text[cs..], start + cs)
            }
            BlockKind::BlockQuote => quote_inlines(&text, start),
            _ => parse_inlines(&text, start),
        })
    }

    fn fresh_block(&mut self, kind: BlockKind, start_line: usize, end_line: usize) -> Block {
        let id = BlockId(self.next_id);
        self.next_id += 1;
        Block {
            id,
            kind,
            start_line,
            end_line,
        }
    }

    /// 数学块的 TeX 源码（视图渲染/导出的输入）：多行剥开闭标记行，单行剥两侧 `$$`；
    /// 中间行原样保留（含空行，`\r` 随 lines() 剥除）。非数学块/未知 id 返回 None。
    pub fn math_source(&self, buf: &Buffer, block: BlockId) -> Option<String> {
        let b = self.blocks.iter().find(|b| b.id == block)?;
        if b.kind != BlockKind::Math {
            return None;
        }
        let start = buf.offset_of_line(b.start_line);
        let end = if b.end_line < buf.line_count() {
            buf.offset_of_line(b.end_line)
        } else {
            buf.len()
        };
        let text = buf.slice(start, end);
        if b.end_line == b.start_line + 1 {
            // 单行 `$$x$$`：trim 去行尾空白/换行后剥两侧标记
            let t = text.trim();
            return Some(t[2..t.len() - 2].to_string());
        }
        let mut lines = text.lines();
        lines.next(); // 开标记行
        let mut body: Vec<&str> = lines.collect();
        if body.last().is_some_and(|l| l.trim() == "$$") {
            body.pop(); // 闭标记行（未闭合时无此行，全为内容）
        }
        Some(body.join("\n"))
    }

    /// 表格派生数据：表头/表体单元格与列对齐。非表格块/未知 id 返回 None。
    /// 单元格文本为「剥除 inline 标记后的可见文本」；区间为 trim 后的内容区间
    /// （绝对字节偏移）。表体行按列数补空/截断（GFM 行为）。
    pub fn table(&self, buf: &Buffer, block: BlockId) -> Option<Table> {
        let b = self.blocks.iter().find(|b| b.id == block)?;
        if b.kind != BlockKind::Table {
            return None;
        }
        let delim = line_text(buf, b.start_line + 1);
        let aligns = parse_delim_row(&delim)?;
        let header_off = buf.offset_of_line(b.start_line);
        let header = parse_table_cells(&line_text(buf, b.start_line), header_off, aligns.len());
        let mut rows = Vec::with_capacity(b.end_line.saturating_sub(b.start_line + 2));
        for line in b.start_line + 2..b.end_line {
            let off = buf.offset_of_line(line);
            rows.push(parse_table_cells(&line_text(buf, line), off, aligns.len()));
        }
        Some(Table {
            aligns,
            header,
            rows,
        })
    }

    /// 编辑后重解析。`edit_start_line`/`old_end_line` 基于旧文档行号，
    /// `new_end_line` 基于新文档。返回脏块 id（视图据此失效缓存）。
    pub fn reparse(
        &mut self,
        buf: &Buffer,
        edit_start_line: usize,
        old_end_line: usize,
        new_end_line: usize,
    ) -> Vec<BlockId> {
        let delta = new_end_line as isize - old_end_line as isize;
        // 前缀：完全在脏区之前的块不动（含 id）
        let mut bi = self
            .blocks
            .iter()
            .position(|b| edit_start_line < b.end_line)
            .unwrap_or(self.blocks.len());
        // 向前扩展：编辑起点落在块间空白或全文末尾时，前块可能向下生长
        // （段落合并），且空白区新增内容本身必须进入重扫——否则新内容
        // 对块结构不可见（tests/core.rs「reparse 向前扩展边界」组钉死）。
        if bi == self.blocks.len() {
            bi = bi.saturating_sub(1);
        } else if edit_start_line < self.blocks[bi].start_line && bi > 0 {
            bi -= 1;
        }
        let scan_from = if bi < self.blocks.len() {
            self.blocks[bi].start_line.min(edit_start_line)
        } else {
            edit_start_line
        };
        let total = buf.line_count();

        // 重扫：超过新区后，遇到与旧块（平移后）对齐且同种的边界即停，
        // 其后的旧块整体平移保留——这是「干净块不动」承诺的实现。
        let mut produced: Vec<(BlockKind, usize, usize)> = Vec::new();
        let mut suffix_start: Option<usize> = None;
        let mut line = scan_from;
        while line < total {
            if line_text(buf, line).trim().is_empty() {
                line += 1;
                continue;
            }
            if line > new_end_line {
                let aligned = (bi..self.blocks.len()).find(|&k| {
                    let shifted = self.blocks[k].start_line as isize + delta;
                    shifted >= 0 && shifted as usize == line
                });
                if let Some(k) = aligned {
                    let (kind, _) = scan_one(buf, line, total);
                    if kind == self.blocks[k].kind {
                        suffix_start = Some(k);
                        break;
                    }
                }
            }
            let (kind, consumed) = scan_one(buf, line, total);
            produced.push((kind, line, line + consumed));
            line += consumed;
        }

        let mut dirty = Vec::with_capacity(produced.len());
        let mut new_blocks: Vec<Block> = self.blocks[..bi].to_vec();
        for (kind, s, e) in produced {
            let block = self.fresh_block(kind, s, e);
            dirty.push(block.id);
            new_blocks.push(block);
        }
        if let Some(k) = suffix_start {
            for b in &self.blocks[k..] {
                // 已对齐块 shifted >= 0；其后块起始行更大，平移后必然非负
                new_blocks.push(Block {
                    id: b.id,
                    kind: b.kind,
                    start_line: (b.start_line as isize + delta) as usize,
                    end_line: (b.end_line as isize + delta) as usize,
                });
            }
        }
        self.blocks = new_blocks;
        dirty
    }
}

/// 引用块 inline：逐行解析 `>` 与至多一个随行空白之后的内容（base 为行绝对起点）。
fn quote_inlines(text: &str, base: usize) -> Vec<InlineNode> {
    let mut out = Vec::new();
    let mut off = 0;
    for line in text.split_inclusive('\n') {
        let cs = match quote_marker_pos(line) {
            Some(p) => {
                let mut e = p + 1;
                if matches!(line.as_bytes().get(e), Some(b' ') | Some(b'\t')) {
                    e += 1;
                }
                e
            }
            None => 0, // 防御：块内行应均以 > 开头
        };
        out.extend(parse_inlines(&line[cs..], base + off + cs));
        off += line.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::inline::sexp;
    use super::*;
    use crate::range::Range;

    // ===== 列表/引用 inline 范围：标记本身不得被误认（切片 12） =====

    #[test]
    fn list_item_inline_starts_after_marker() {
        // 「- **a**」：标记占 [0,2)，strong 从 2 起（逐字节：-=0 ' '=1 *=2 *=3 a=4 *=5 *=6）
        let b = Buffer::new("- **a**\n");
        let s = Syntax::new(&b);
        let id = s.blocks()[0].id;
        assert_eq!(sexp(&s.inlines(&b, id).unwrap()), "Strong[2..7 c=4..5]");
        // 无序标记 `*` 自身不得被解析为 emph
        let b2 = Buffer::new("* a\n");
        let s2 = Syntax::new(&b2);
        let id2 = s2.blocks()[0].id;
        assert!(s2.inlines(&b2, id2).unwrap().is_empty());
    }

    #[test]
    fn blockquote_inlines_per_line_after_marker() {
        // 「> **a**\n> b *c*」：行 1 strong [2,7)，行 2 emph [12,15)
        let b = Buffer::new("> **a**\n> b *c*\n");
        let s = Syntax::new(&b);
        let id = s.blocks()[0].id;
        assert_eq!(
            sexp(&s.inlines(&b, id).unwrap()),
            "Strong[2..7 c=4..5] Emph[12..15 c=13..14]"
        );
    }

    #[test]
    fn thematic_break_has_no_inlines() {
        let b = Buffer::new("---\n");
        let s = Syntax::new(&b);
        let id = s.blocks()[0].id;
        assert_eq!(s.inlines(&b, id), Some(Vec::new()));
    }

    #[test]
    fn math_block_has_no_inlines() {
        // 与 FencedCode 一致：数学块不解析 inline
        let buf = Buffer::new("$$\n$a$ and **b**\n$$\n");
        let s = Syntax::new(&buf);
        let b = &s.blocks()[0];
        assert_eq!(b.kind, BlockKind::Math);
        assert_eq!(s.inlines(&buf, b.id), Some(Vec::new()));
    }

    #[test]
    fn math_source_strips_markers() {
        let cases = [
            ("$$\nx + y\n$$\n", "x + y"),
            ("$$x+y$$\n", "x+y"),
            ("$$\nx\n\ny\n$$\n", "x\n\ny"), // 内部空行保留
            ("$$\nx\n", "x"),               // 未闭合：余下全是内容
            ("$$\n$$\n", ""),               // 空公式
            ("$$\r\nx\r\n$$\r\n", "x"),     // CRLF：\r 随 lines() 剥除
        ];
        for (doc, want) in cases {
            let buf = Buffer::new(doc);
            let s = Syntax::new(&buf);
            let b = &s.blocks()[0];
            assert_eq!(
                s.math_source(&buf, b.id).as_deref(),
                Some(want),
                "doc={doc:?}"
            );
        }
    }

    #[test]
    fn math_source_none_for_non_math() {
        let buf = Buffer::new("para\n");
        let s = Syntax::new(&buf);
        assert_eq!(s.math_source(&buf, s.blocks()[0].id), None);
    }

    #[test]
    fn table_delimiter_aligns_all_kinds() {
        let b = Buffer::new("| a | b | c | d |\n| :-- | --: | :-: | --- |\n");
        let s = Syntax::new(&b);
        let t = s.table(&b, s.blocks()[0].id).unwrap();
        assert_eq!(
            t.aligns,
            [
                CellAlign::Left,
                CellAlign::Right,
                CellAlign::Center,
                CellAlign::None
            ]
        );
    }

    #[test]
    fn table_escaped_pipe_is_not_a_separator() {
        // `| a \| b | c |`：首格内容含转义竖线，切分仍为两格
        let b = Buffer::new("| a \\| b | c |\n| --- | --- |\n");
        let s = Syntax::new(&b);
        let t = s.table(&b, s.blocks()[0].id).unwrap();
        assert_eq!(t.header.len(), 2);
        assert_eq!(t.header[0].text, "a | b");
        assert_eq!(t.header[1].text, "c");
    }

    #[test]
    fn table_cell_text_visible_and_range_trimmed() {
        // 「| **a** | `c` |」：内容区间各自 trim，可见文本剥标记
        let b = Buffer::new("| **a** | `c` |\n| --- | --- |\n");
        let s = Syntax::new(&b);
        let t = s.table(&b, s.blocks()[0].id).unwrap();
        assert_eq!(t.header[0].text, "a");
        assert_eq!(t.header[0].range, Range::new(2, 7));
        assert_eq!(t.header[1].text, "c");
        assert_eq!(t.header[1].range, Range::new(10, 13));
    }

    #[test]
    fn table_body_cells_pad_and_truncate() {
        let b = Buffer::new("| a | b |\n| --- | --- |\n| c |\n| e | f | g |\n");
        let s = Syntax::new(&b);
        let t = s.table(&b, s.blocks()[0].id).unwrap();
        assert_eq!(
            t.rows[0]
                .iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>(),
            ["c", ""]
        );
        assert_eq!(
            t.rows[1]
                .iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>(),
            ["e", "f"]
        );
    }

    #[test]
    fn table_cell_image_shows_alt_text() {
        // 单元格内 Image 的可见文本 = alt（切片 13：导出/网格共用 visible_text）
        let b = Buffer::new("| ![a](p) |\n| --- |\n");
        let s = Syntax::new(&b);
        let t = s.table(&b, s.blocks()[0].id).unwrap();
        assert_eq!(t.header[0].text, "a");
    }

    #[test]
    fn table_block_has_no_inlines_and_non_table_returns_none() {
        let buf = Buffer::new("| **a** |\n| --- |\n");
        let s = Syntax::new(&buf);
        let b = &s.blocks()[0];
        assert_eq!(b.kind, BlockKind::Table);
        assert_eq!(s.inlines(&buf, b.id), Some(Vec::new()));
        let p = Buffer::new("para\n");
        let s2 = Syntax::new(&p);
        assert_eq!(s2.table(&p, s2.blocks()[0].id), None);
    }
}
