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

pub use block::{list_signature, task_span};
pub(crate) use model::Block;
pub use model::{BlockId, BlockKind, CellAlign, InlineKind, InlineNode, Table, TableCell};

use crate::buffer::Buffer;
use block::{
    fence_close, fence_open, heading_content_span, is_thematic_break, line_text, list_mark,
    normalize_label, parse_delim_row, parse_link_ref_def, parse_table_cells, quote_def_lines,
    quote_link_ref_defs, quote_marker_pos, scan_blocks, scan_next, visible_text, RefMap,
};
use inline::parse_inlines;

pub(crate) struct Syntax {
    blocks: Vec<Block>,
    next_id: u64,
    /// 文档级链接引用表（0c P0-1）：规范化 label → (dest, title) 绝对区间。
    /// 派生数据：new/reparse 后由 LinkRefDef 块全量重建，不持久化、不进撤销栈。
    refs: RefMap,
}

impl Syntax {
    pub fn new(buf: &Buffer) -> Self {
        let mut syntax = Syntax {
            blocks: Vec::new(),
            next_id: 0,
            refs: RefMap::new(),
        };
        let raw = scan_blocks(buf, 0, buf.line_count());
        syntax.blocks = raw
            .into_iter()
            .map(|(kind, s, e)| syntax.fresh_block(kind, s, e))
            .collect();
        syntax.build_refs(buf);
        syntax
    }

    /// 重建引用表：遍历 LinkRefDef 块解析定义文本（first wins）；引用块内部经
    /// 容器剥离后同样扫描（CommonMark：容器内定义对全文档可见，Example 218）。
    /// O(块数) + O(定义数)，击键热路径可接受（定义块稀少；普通块不触碰文本）。
    fn build_refs(&mut self, buf: &Buffer) {
        self.refs.clear();
        for b in &self.blocks {
            if !matches!(b.kind, BlockKind::LinkRefDef | BlockKind::BlockQuote) {
                continue;
            }
            let start = buf.offset_of_line(b.start_line);
            let end = if b.end_line < buf.line_count() {
                buf.offset_of_line(b.end_line)
            } else {
                buf.len()
            };
            let text = buf.slice(start, end);
            if b.kind == BlockKind::BlockQuote {
                for (label, dest, title) in quote_link_ref_defs(&text, start) {
                    self.refs.entry(label).or_insert((dest, title));
                }
                continue;
            }
            let Some(ast) = parse_link_ref_def(&text, start) else {
                continue; // 防御：块种类与解析同源，不应失败
            };
            let label = normalize_label(&buf.slice(ast.label.start, ast.label.end));
            self.refs.entry(label).or_insert((ast.dest, ast.title));
        }
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// 某块的 inline 节点（绝对字节偏移）。派生数据：访问时按块当前文本即时解析，
    /// 不持久化、不进撤销栈——块区间由 reparse 维护，故编辑后查询恒正确。
    /// FencedCode/IndentedCode/Math/Table/ThematicBreak 恒为空（无 inline）；未知 id 返回 None。
    /// 列表/引用仅解析标记（`- ` / `> `）之后的内容，标记本身不得被 inline 误认。
    pub fn inlines(&self, buf: &Buffer, block: BlockId) -> Option<Vec<InlineNode>> {
        let b = self.blocks.iter().find(|b| b.id == block)?;
        if matches!(
            b.kind,
            BlockKind::FencedCode(_)
                | BlockKind::IndentedCode
                | BlockKind::Math
                | BlockKind::Table
                | BlockKind::ThematicBreak
                | BlockKind::LinkRefDef
                | BlockKind::HtmlBlock
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
                // 项内围栏代码不解析 inline（0c P0-3：多行项可含围栏，v0 规则同
                // 区域扫描器——闭合围栏任意缩进）；首行跳过标记，续行整行解析。
                let mut out = Vec::new();
                let mut in_fence: Option<(u8, usize)> = None;
                let mut off = 0usize;
                for (idx, line) in text.split_inclusive('\n').enumerate() {
                    let skip = if idx == 0 { cs.min(line.len()) } else { 0 };
                    let seg = &line[skip..];
                    let t = seg.trim_start();
                    if let Some((ch, len)) = in_fence {
                        if fence_close(t, ch, len) {
                            in_fence = None;
                        }
                    } else if let Some((ch, len, _)) = fence_open(t) {
                        in_fence = Some((ch, len));
                    } else if is_thematic_break(t) {
                        // 项内分隔线（Example 61）：无 inline
                    } else {
                        out.extend(parse_inlines(seg, start + off + skip, &self.refs));
                    }
                    off += line.len();
                }
                out
            }
            BlockKind::BlockQuote => quote_inlines(&text, start, &self.refs),
            // Setext：inline 仅解析内容行，下划线行不参与（0c P0-2）
            BlockKind::SetextHeading(_) => {
                let ul = buf.offset_of_line(b.end_line - 1);
                parse_inlines(&buf.slice(start, ul), start, &self.refs)
            }
            _ => parse_inlines(&text, start, &self.refs),
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
        let header = parse_table_cells(
            &line_text(buf, b.start_line),
            header_off,
            aligns.len(),
            &self.refs,
        );
        let mut rows = Vec::with_capacity(b.end_line.saturating_sub(b.start_line + 2));
        for line in b.start_line + 2..b.end_line {
            let off = buf.offset_of_line(line);
            rows.push(parse_table_cells(&line_text(buf, line), off, aligns.len(), &self.refs));
        }
        Some(Table {
            aligns,
            header,
            rows,
        })
    }

    /// 标题块派生数据：(级别, 可见文本)。ATX 标记/闭合序列与 inline 标记均剥除
    /// （`## **B**` → `B`）；Setext 取内容行（末行下划线除外）。非标题块/未知 id
    /// 返回 None（大纲面板的数据来源）。
    pub fn heading(&self, buf: &Buffer, block: BlockId) -> Option<(u8, String)> {
        let b = self.blocks.iter().find(|b| b.id == block)?;
        match b.kind {
            BlockKind::AtxHeading(level) => {
                let line = line_text(buf, b.start_line);
                let (cs, ce) = heading_content_span(&line)?;
                let line_off = buf.offset_of_line(b.start_line);
                let content = &line[cs..ce];
                let nodes = parse_inlines(content, line_off + cs, &self.refs);
                Some((
                    level,
                    visible_text(content, &nodes, line_off + cs)
                        .trim()
                        .to_string(),
                ))
            }
            BlockKind::SetextHeading(level) => {
                // 内容 = 除末行（下划线）外的全部行；行间换行保留（HTML 渲染为软换行）
                let start = buf.offset_of_line(b.start_line);
                let ul = buf.offset_of_line(b.end_line - 1);
                let content = buf.slice(start, ul);
                let nodes = parse_inlines(&content, start, &self.refs);
                Some((level, visible_text(&content, &nodes, start).trim().to_string()))
            }
            _ => None,
        }
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
        // 同时：编辑起点正好落在当前块首行时，前块也可能向下生长合并进
        // 当前块（删列表/引用标记、给段落首行加缩进）——旧条件 `<` 漏掉
        // 这一族反例（Commit 1-3 钉住 A/B/C 三类）。
        if bi == self.blocks.len() {
            bi = bi.saturating_sub(1);
        } else if edit_start_line <= self.blocks[bi].start_line && bi > 0 {
            bi -= 1;
        }
        // 列表区域回走（0c P0-3）：编辑点落入列表区域（前块同为列表项且间隔全是
        // 空行）时，区域扫描以整片列表为单元，必须回到区域起点重扫——否则空行
        // 增删导致的续行归属/嵌套关系变化对重扫不可见。
        while bi > 0
            && bi < self.blocks.len()
            && matches!(self.blocks[bi].kind, BlockKind::ListItem { .. })
            && matches!(self.blocks[bi - 1].kind, BlockKind::ListItem { .. })
            && (self.blocks[bi - 1].end_line..self.blocks[bi].start_line)
                .all(|l| line_text(buf, l).trim().is_empty())
        {
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
        // 扫描单元为 scan_next：列表区域整体产出（0c P0-3）。
        let mut produced: Vec<(BlockKind, usize, usize)> = Vec::new();
        let mut suffix_start: Option<usize> = None;
        let mut line = scan_from;
        while line < total {
            if line_text(buf, line).trim().is_empty() {
                line += 1;
                continue;
            }
            let (mut v, consumed) = scan_next(buf, line, total);
            // 先把块扫出来再比对，避免为校验额外扫一遍（旧写法在此处再调
            // scan_next 会退化为 O(n²)）。起始行 + 种类 + **结束行**三者都平移
            // 对齐才算「干净块」；只比起始行与种类时，块边界变化（内容已改，
            // 但首行恰好对齐）仍会复用旧 BlockId，派生缓存于是命中陈旧布局。
            if line > new_end_line {
                if let Some(k) = (bi..self.blocks.len()).find(|&k| {
                    let shifted = self.blocks[k].start_line as isize + delta;
                    shifted >= 0 && shifted as usize == line
                }) {
                    let (nk, _, ne) = &v[0];
                    let shifted_end = self.blocks[k].end_line as isize + delta;
                    if *nk == self.blocks[k].kind
                        && shifted_end >= 0
                        && shifted_end as usize == *ne
                    {
                        suffix_start = Some(k);
                        break; // 此后旧块整体平移保留，不再追加刚扫出的 v
                    }
                }
            }
            produced.append(&mut v);
            line += consumed;
        }

        let mut dirty = Vec::with_capacity(produced.len());
        let mut new_blocks: Vec<Block> = self.blocks[..bi].to_vec();
        for (i, (kind, s, e)) in produced.into_iter().enumerate() {
            // zip-reuse：与旧块同相对位次且种类一致时，区间不触编辑区
            // [edit_start_line, new_end_line) ⇒ 内容必然未变 ⇒ 保留 id 不脏
            // （0c P0-3：列表区域整体重扫时，编辑点之前的同区域项由此保 id）。
            // 编辑区之前的块坐标不平移，之后的块平移 delta。
            let reused = self.blocks.get(bi + i).and_then(|o| {
                if o.kind != kind {
                    return None;
                }
                let same = if e <= edit_start_line {
                    o.start_line == s && o.end_line == e
                } else if s > new_end_line {
                    (o.start_line as isize + delta) as usize == s
                        && (o.end_line as isize + delta) as usize == e
                } else {
                    false
                };
                if same {
                    Some(o.id)
                } else {
                    None
                }
            });
            let block = match reused {
                Some(id) => Block {
                    id,
                    kind,
                    start_line: s,
                    end_line: e,
                },
                None => {
                    let b = self.fresh_block(kind, s, e);
                    dirty.push(b.id);
                    b
                }
            };
            new_blocks.push(block);
        }
        if let Some(k) = suffix_start {
            for b in &self.blocks[k..] {
                // 已对齐块 shifted >= 0；其后块起始行更大，平移后必然非负
                new_blocks.push(Block {
                    id: b.id,
                    kind: b.kind.clone(),
                    start_line: (b.start_line as isize + delta) as usize,
                    end_line: (b.end_line as isize + delta) as usize,
                });
            }
        }
        self.blocks = new_blocks;
        self.build_refs(buf);
        dirty
    }
}

/// 引用块 inline：逐行解析 `>` 与至多一个随行空白之后的内容（base 为行绝对起点）。
/// 属链接引用定义的行跳过——定义被引用表消耗，不产生 inline（Example 218）。
fn quote_inlines(text: &str, base: usize, refs: &RefMap) -> Vec<InlineNode> {
    let defs = quote_def_lines(text);
    let mut out = Vec::new();
    let mut off = 0;
    for (idx, line) in text.split_inclusive('\n').enumerate() {
        if defs.iter().any(|&(s, e)| (s..e).contains(&idx)) {
            off += line.len();
            continue;
        }
        let cs = match quote_marker_pos(line) {
            Some(p) => {
                let mut e = p + 1;
                if matches!(line.as_bytes().get(e), Some(b' ') | Some(b'\t')) {
                    e += 1;
                }
                e
            }
            None => 0, // 懒续行（0c P0-3）：无 `>` 标记，从行首解析
        };
        out.extend(parse_inlines(&line[cs..], base + off + cs, refs));
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
    fn quote_link_ref_def_visible_document_wide() {
        // Example 218：引用块内的定义对全文档可见，且定义行自身不产生 inline
        let b = Buffer::new("[foo]\n\n> [foo]: /url\n");
        let s = Syntax::new(&b);
        assert_eq!(s.blocks().len(), 2);
        assert_eq!(
            sexp(&s.inlines(&b, s.blocks()[0].id).unwrap()),
            "Link[0..5 c=1..4 url=16..20]"
        );
        assert!(s.inlines(&b, s.blocks()[1].id).unwrap().is_empty());
        // 定义不能打断进行中段落：剥离视图为单段落时不注册（与顶层规则一致）
        let b2 = Buffer::new("[foo]\n\n> text\n> [foo]: /url\n");
        let s2 = Syntax::new(&b2);
        assert!(s2.inlines(&b2, s2.blocks()[0].id).unwrap().is_empty());
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
    fn heading_text_strips_atx_and_inline_markers() {
        let buf = Buffer::new("# Title ###\n\n## **B** and `c`\n\n###\n\n# Title#\n\npara\n");
        let s = Syntax::new(&buf);
        let heads: Vec<_> = s
            .blocks()
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::AtxHeading(_)))
            .collect();
        assert_eq!(heads.len(), 4);
        assert_eq!(s.heading(&buf, heads[0].id), Some((1, "Title".into())));
        assert_eq!(s.heading(&buf, heads[1].id), Some((2, "B and c".into())));
        assert_eq!(s.heading(&buf, heads[2].id), Some((3, String::new())));
        // 闭合序列须由空白引出：紧贴文本的 `#` 属内容
        assert_eq!(s.heading(&buf, heads[3].id), Some((1, "Title#".into())));
        // 非标题块返回 None
        let para = s.blocks().last().unwrap().id;
        assert_eq!(s.heading(&buf, para), None);
    }

    // ===== Setext 标题（0c P0-2）：派生数据与增量 =====

    #[test]
    fn setext_heading_text_and_level() {
        // 内容行 inline 标记剥除；多行内容合并（保留行间换行）
        let buf = Buffer::new("Foo *bar*\n=========\n\n多行\n内容\n---\n");
        let s = Syntax::new(&buf);
        let heads: Vec<_> = s
            .blocks()
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::SetextHeading(_)))
            .collect();
        assert_eq!(heads.len(), 2);
        assert_eq!(s.heading(&buf, heads[0].id), Some((1, "Foo bar".into())));
        assert_eq!(s.heading(&buf, heads[1].id), Some((2, "多行\n内容".into())));
    }

    #[test]
    fn setext_inlines_exclude_underline() {
        // 「Foo *bar*\n---\n」：emph [4,9) 内容 [5,8)；下划线行不产生节点
        let buf = Buffer::new("Foo *bar*\n---\n");
        let s = Syntax::new(&buf);
        let b = &s.blocks()[0];
        assert_eq!(sexp(&s.inlines(&buf, b.id).unwrap()), "Emph[4..9 c=5..8]");
    }

    #[test]
    fn reparse_paragraph_becomes_setext_on_blank_delete() {
        // 「Foo\n\n===\n」删除空行（旧行 1）→ 段落转 Setext；向前扩展必须重扫段首
        let mut b = Buffer::new("Foo\n\n===\n");
        let mut s = Syntax::new(&b);
        assert_eq!(s.blocks().len(), 2);
        b.edit(4, 5, ""); // 删第二个 \n
        s.reparse(&b, 1, 2, 1);
        let got: Vec<_> = s
            .blocks()
            .iter()
            .map(|b| (b.kind.clone(), b.start_line, b.end_line))
            .collect();
        assert_eq!(got, [(BlockKind::SetextHeading(1), 0, 2)]);
    }

    #[test]
    fn reparse_setext_survives_content_edit() {
        // 内容行内编辑不改变块结构（「Foo\n===\n」Foo → Foox）
        let mut b = Buffer::new("Foo\n===\n");
        let mut s = Syntax::new(&b);
        b.edit(3, 3, "x");
        s.reparse(&b, 0, 1, 1);
        assert_eq!(s.blocks()[0].kind, BlockKind::SetextHeading(1));
        assert_eq!((s.blocks()[0].start_line, s.blocks()[0].end_line), (0, 2));
    }

    #[test]
    fn reparse_underline_edit_drops_to_paragraph() {
        // 「Foo\n===\n」下划线改为文本（旧行 1）→ 整体退化为单段落
        let mut b = Buffer::new("Foo\n===\n");
        let mut s = Syntax::new(&b);
        b.edit(4, 7, "x"); // === → x
        s.reparse(&b, 1, 2, 2);
        let got: Vec<_> = s
            .blocks()
            .iter()
            .map(|b| (b.kind.clone(), b.start_line, b.end_line))
            .collect();
        assert_eq!(got, [(BlockKind::Paragraph, 0, 2)]);
    }

    // ===== 多行列表项与懒续行（0c P0-3）：派生数据与增量 =====

    #[test]
    fn inlines_multiline_list_item() {
        // 「- **a**\n  *b*\n」：strong 在首行 [2,7)，emph 在续行 [10,13)
        let b = Buffer::new("- **a**\n  *b*\n");
        let s = Syntax::new(&b);
        let id = s.blocks()[0].id;
        assert_eq!(
            sexp(&s.inlines(&b, id).unwrap()),
            "Strong[2..7 c=4..5] Emph[10..13 c=11..12]"
        );
    }

    #[test]
    fn inlines_outer_continuation_chunk() {
        // 外层项续块（首行无标记）：inline 从块起点解析（行 3 `  *c*`，块起 11）
        let b = Buffer::new("* a\n  * b\n\n  *c*\n");
        let s = Syntax::new(&b);
        assert_eq!(s.blocks().len(), 3);
        let chunk = &s.blocks()[2];
        assert!(matches!(chunk.kind, BlockKind::ListItem { .. }));
        assert_eq!((chunk.start_line, chunk.end_line), (3, 4));
        assert_eq!(sexp(&s.inlines(&b, chunk.id).unwrap()), "Emph[13..16 c=14..15]");
    }

    #[test]
    fn blockquote_inlines_with_lazy_line() {
        // 「> **a**\n*b*\n」：懒续行无 `>`，inline 从行首解析
        let b = Buffer::new("> **a**\n*b*\n");
        let s = Syntax::new(&b);
        let id = s.blocks()[0].id;
        assert_eq!(
            sexp(&s.inlines(&b, id).unwrap()),
            "Strong[2..7 c=4..5] Emph[8..11 c=9..10]"
        );
    }

    #[test]
    fn reparse_edit_inside_list_region_rescans_from_region_start() {
        // 「* a\n  * b\n\n  c\n」删空行（旧行 2）→ 续行归属须回走至区域起点重扫
        let mut b = Buffer::new("* a\n  * b\n\n  c\n");
        let mut s = Syntax::new(&b);
        b.edit(10, 11, ""); // 删空行的 \n
        s.reparse(&b, 2, 3, 2);
        let got: Vec<_> = s
            .blocks()
            .iter()
            .map(|b| (b.kind.clone(), b.start_line, b.end_line))
            .collect();
        assert_eq!(
            got,
            [
                (BlockKind::ListItem { ordered: false, depth: 0, task: None }, 0, 1),
                (BlockKind::ListItem { ordered: false, depth: 1, task: None }, 1, 2),
                (BlockKind::ListItem { ordered: false, depth: 0, task: None }, 2, 3),
            ]
        );
    }

    #[test]
    fn reparse_list_prefix_items_keep_ids() {
        // 「- a\n- b\n- c\n」编辑第二项内容：首项区间未触编辑，id 保留（干净块不动）
        let mut b = Buffer::new("- a\n- b\n- c\n");
        let mut s = Syntax::new(&b);
        let first_id = s.blocks()[0].id;
        b.edit(6, 7, "B"); // b → B
        s.reparse(&b, 1, 2, 2);
        assert_eq!(s.blocks().len(), 3);
        assert_eq!(s.blocks()[0].id, first_id);
        assert_ne!(s.blocks()[1].id, first_id);
    }

    #[test]
    fn reparse_typing_continuation_extends_list_item() {
        // 「- a\n」末尾追加续行 → 项区间向下生长为 [0,2)
        let mut b = Buffer::new("- a\n");
        let mut s = Syntax::new(&b);
        b.edit(4, 4, "  b\n");
        s.reparse(&b, 1, 1, 2);
        assert_eq!(s.blocks().len(), 1);
        assert_eq!(
            (s.blocks()[0].start_line, s.blocks()[0].end_line),
            (0, 2)
        );
    }

    #[test]
    fn reparse_lazy_line_joins_blockquote() {
        // 「> a\n\n」在空行（行 1）填入文本 → 懒续行并入引用块
        let mut b = Buffer::new("> a\n\n");
        let mut s = Syntax::new(&b);
        b.edit(4, 4, "b");
        s.reparse(&b, 1, 1, 1);
        let got: Vec<_> = s
            .blocks()
            .iter()
            .map(|b| (b.kind.clone(), b.start_line, b.end_line))
            .collect();
        assert_eq!(got, [(BlockKind::BlockQuote, 0, 2)]);
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

    // ===== 引用式链接/图片（0c P0-1）：引用表 + 三种形式 =====

    fn para_inlines(doc: &str) -> (Syntax, Buffer, Vec<InlineNode>) {
        let b = Buffer::new(doc);
        let s = Syntax::new(&b);
        let p = s
            .blocks()
            .iter()
            .find(|b| b.kind == BlockKind::Paragraph)
            .expect("段落块");
        let nodes = s.inlines(&b, p.id).unwrap();
        (s, b, nodes)
    }

    #[test]
    fn shortcut_link_resolves_via_ref_table() {
        // `[foo]\n\n[foo]: /url "t"`：shortcut 链接；url 指向定义处 dest 绝对区间
        let (_s, b, nodes) = para_inlines("[foo]\n\n[foo]: /url \"t\"\n");
        assert_eq!(sexp(&nodes), "Link[0..5 c=1..4 url=14..18]");
        // url 区间取自定义处源码
        assert_eq!(&b.slice(0, b.len())[14..18], "/url");
    }

    #[test]
    fn full_and_collapsed_reference_links() {
        let (_s, _b, nodes) = para_inlines("[a][x] [b][]\n\n[x]: /u1\n[b]: /u2\n");
        assert_eq!(
            sexp(&nodes),
            "Link[0..6 c=1..2 url=19..22] Link[7..12 c=8..9 url=28..31]"
        );
    }

    #[test]
    fn reference_link_unknown_label_is_text() {
        // 查无定义：无节点；内部 inline 正常解析（`[*foo*]` 的 emph 仍在）
        let (_s, _b, nodes) = para_inlines("[foo] [*bar*]\n");
        assert_eq!(sexp(&nodes), "Emph[7..12 c=8..11]");
    }

    #[test]
    fn reference_label_normalization_case_and_ws() {
        // label 匹配：大小写不敏感 + 内部空白折叠
        let (_s, _b, nodes) = para_inlines("[FOO  bar]\n\n[foo bar]: /u\n");
        assert_eq!(sexp(&nodes), "Link[0..10 c=1..9 url=23..25]");
    }

    #[test]
    fn reference_first_definition_wins() {
        let (_s, b, nodes) = para_inlines("[a]\n\n[a]: /first\n[a]: /second\n");
        let url = nodes[0].url.unwrap();
        assert_eq!(&b.slice(0, b.len())[url.start..url.end], "/first");
    }

    #[test]
    fn reference_image_all_forms() {
        let (_s, _b, nodes) = para_inlines("![a] ![b][x] ![c][]\n\n[a]: /1\n[x]: /2\n[c]: /3\n");
        assert_eq!(
            sexp(&nodes),
            "Image[0..4 c=2..3 url=26..28] Image[5..12 c=7..8 url=34..36] Image[13..19 c=15..16 url=42..44]"
        );
    }

    #[test]
    fn inline_link_wins_over_reference() {
        // `[a](/inline)` 优先于引用式（同 label 有定义也用 inline URL）
        let (_s, _b, nodes) = para_inlines("[a](/inline)\n\n[a]: /ref\n");
        assert_eq!(sexp(&nodes), "Link[0..12 c=1..2 url=4..11]");
    }

    #[test]
    fn reparse_rebuilds_ref_table() {
        // 编辑定义行后引用表同步：改 dest → url 指向新位置
        let mut b = Buffer::new("[a]\n\n[a]: /old\n");
        let mut s = Syntax::new(&b);
        // 把 `/old` 改为 `/new`（同长度，简化行号不变）
        let off = b.len() - 5; // `/old` 起点
        b.edit(off, off + 4, "/new");
        s.reparse(&b, 2, 3, 3);
        let p = s.blocks().iter().find(|x| x.kind == BlockKind::Paragraph).unwrap();
        let nodes = s.inlines(&b, p.id).unwrap();
        let url = nodes[0].url.unwrap();
        assert_eq!(&b.slice(0, b.len())[url.start..url.end], "/new");
    }
}
