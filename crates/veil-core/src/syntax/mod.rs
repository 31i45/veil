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

use std::hash::{Hash, Hasher};

pub use block::{list_signature, set_image_alt_fallback, split_row as split_table_row, task_span};
pub(crate) use model::Block;
pub use model::{
    sanitize_style, should_strip_html, AdmonitionType, BlockContainer, BlockId, BlockKind, Callout,
    CellAlign, HtmlAttrs, HtmlStyle, HtmlTag, ImageAlign, InlineKind, InlineNode, Table, TableCell,
    HTML_INLINE_WHITELIST,
};

use crate::buffer::Buffer;
use crate::callouts::CalloutRegistry;
use crate::range::Range;
use block::{
    fence_close, fence_open, heading_content_span, indent_info, is_thematic_break, line_text,
    list_mark, normalize_label, parse_delim_row, parse_footnote_def, parse_link_ref_def,
    parse_table_cells, quote_def_lines, quote_link_ref_defs, quote_marker_pos, scan_blocks_meta,
    scan_next_meta, strip_container_line, visible_text, RefMap, ScannedBlock,
};
use inline::parse_inlines;

pub(crate) struct Syntax {
    blocks: Vec<Block>,
    next_id: u64,
    /// 文档级链接引用表（0c P0-1）：规范化 label → (dest, title) 绝对区间。
    /// 派生数据：new/reparse 后由 LinkRefDef 块全量重建，不持久化、不进撤销栈。
    refs: RefMap,
    /// 派生脚注编号表：id → 编号（1 起）。重建时机与 `refs` 相同——new/reparse 后
    /// 全量重算，不持久化、不进撤销栈；视图侧以 `footnote_epoch` 摘要参与布局缓存键，
    /// 保证文首新增引用后文末脚注块编号失效重排。
    footnote_numbers: std::collections::HashMap<String, usize>,
    /// 编号顺序的 id 列表（下标 + 1 = 编号），供视图按序渲染脚注区。
    footnote_ids: Vec<String>,
    /// 脚注编号代次：`footnote_ids` 顺序与编号的确定性哈希，随 `build_footnotes`
    /// 一次算好。视图布局缓存键直接读它，避免每帧克隆/哈希整张 id 表。
    footnote_epoch: u64,
    /// 脚注定义表：id → 首个定义的首行内容区间（重复定义 first wins）。
    /// 仅用于视图取定义内容；编号与它无关（未被引用的定义不占号）。
    footnote_defs: std::collections::HashMap<String, Range>,
    /// BlockId → blocks 内索引（new/reparse 后重建，O(1) 查表替代 iter().find）。
    id_index: std::collections::HashMap<BlockId, usize>,
    /// 最近一次 `reparse` 实际扫描的行跨度（只读观测值，不参与解析语义）。
    last_rescan_lines: usize,
    /// 自定义 Callout 表（`callouts.json`）：解析期把 `> [!NAME]` 引用块
    /// 分类为 `BlockKind::Callout`。空表时仅五种内置 GFM Alerts 生效。
    callouts: CalloutRegistry,
}

impl Syntax {
    pub fn new(buf: &Buffer) -> Self {
        Self::new_with_callouts(buf, &CalloutRegistry::default())
    }

    /// 带自定义 Callout 表的构造入口（产品层从 `callouts.json` 注入）。
    pub fn new_with_callouts(buf: &Buffer, callouts: &CalloutRegistry) -> Self {
        let mut syntax = Syntax {
            blocks: Vec::new(),
            next_id: 0,
            refs: RefMap::new(),
            footnote_numbers: std::collections::HashMap::new(),
            footnote_ids: Vec::new(),
            footnote_epoch: 0,
            footnote_defs: std::collections::HashMap::new(),
            id_index: std::collections::HashMap::new(),
            last_rescan_lines: 0,
            callouts: callouts.clone(),
        };
        let raw = scan_blocks_meta(buf, 0, buf.line_count());
        syntax.blocks = raw
            .into_iter()
            .map(|b| {
                let kind = syntax.classify_callout(buf, b.kind, b.start);
                syntax.fresh_block(buf, kind, b.start, b.end, b.container, b.table_lines)
            })
            .collect();
        syntax.build_refs(buf);
        // 先建 id 索引：build_footnotes 经 `inlines` 查块，依赖索引已就绪。
        syntax.rebuild_id_index();
        syntax.build_footnotes(buf);
        syntax
    }

    /// 引用块首行 `> [!NAME]` → 配置里定义的自定义 Callout；未配置的名字
    /// 保持 `BlockQuote`（视图层负责隐藏裸标记文本）。
    fn classify_callout(&self, buf: &Buffer, kind: BlockKind, start_line: usize) -> BlockKind {
        if kind != BlockKind::BlockQuote {
            return kind;
        }
        let line = line_text(buf, start_line);
        let Some(name) = callout_marker_name(&line) else {
            return kind;
        };
        match self.callouts.get(&name) {
            Some(c) => BlockKind::Callout(c.clone()),
            None => kind,
        }
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

    /// 重建脚注派生表（与 `build_refs` 同点位、同生命周期）：
    /// - `footnote_numbers` / `footnote_ids`：按**文档顺序首次引用**从 1 递增；
    ///   同 id 重复引用同号，未被引用的定义不占号。
    /// - `footnote_defs`：id → 首个定义内容区间（重复定义 first wins）。
    ///
    /// 全量重算的代价是逐块 inline 解析；为守住击键热路径，块文本不含 `[^` 时
    /// 直接跳过（脚注在普通文档里稀疏），不改变语义。
    fn build_footnotes(&mut self, buf: &Buffer) {
        self.footnote_numbers.clear();
        self.footnote_ids.clear();
        self.footnote_defs.clear();
        for i in 0..self.blocks.len() {
            let (kind, start_line, end_line) = {
                let b = &self.blocks[i];
                (b.kind.clone(), b.start_line, b.end_line)
            };
            if kind == BlockKind::FootnoteDef {
                let line = line_text(buf, start_line);
                let base = buf.offset_of_line(start_line);
                if let Some((id_range, content)) = parse_footnote_def(&line, base) {
                    let id = buf.slice(id_range.start, id_range.end);
                    self.footnote_defs.entry(id).or_insert(content);
                }
                continue;
            }
            if kind.has_no_inlines() {
                continue;
            }
            let start = buf.offset_of_line(start_line);
            let end = if end_line < buf.line_count() {
                buf.offset_of_line(end_line)
            } else {
                buf.len()
            };
            if !buf.slice(start, end).contains("[^") {
                continue;
            }
            let block_id = self.blocks[i].id;
            let Some(nodes) = self.inlines(buf, block_id) else {
                continue;
            };
            collect_footnote_refs(
                &nodes,
                buf,
                &mut self.footnote_numbers,
                &mut self.footnote_ids,
            );
        }
        // 编号代次在这里一次算好：视图侧每帧都要它，不能每帧重扫整张 id 表。
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.footnote_ids.len().hash(&mut h);
        for (i, id) in self.footnote_ids.iter().enumerate() {
            id.hash(&mut h);
            (i + 1).hash(&mut h);
        }
        self.footnote_epoch = h.finish();
    }

    /// 脚注编号代次（顺序与编号的确定性哈希）；无脚注时为固定值。
    pub fn footnote_epoch(&self) -> u64 {
        self.footnote_epoch
    }

    /// 脚注编号（1 起）；未引用/未知 id 返回 None。
    pub fn footnote_number(&self, id: &str) -> Option<usize> {
        self.footnote_numbers.get(id).copied()
    }

    /// 已编号脚注 id（文档顺序，下标 + 1 = 编号）。
    pub fn footnote_ids(&self) -> &[String] {
        &self.footnote_ids
    }

    /// 脚注首个定义的首行内容区间（绝对字节偏移）；无定义返回 None。
    pub fn footnote_definition(&self, id: &str) -> Option<Range> {
        self.footnote_defs.get(id).copied()
    }

    /// 重建 BlockId → blocks 索引 HashMap（每次 new/reparse 后调用）。
    fn rebuild_id_index(&mut self) {
        self.id_index.clear();
        for (i, b) in self.blocks.iter().enumerate() {
            self.id_index.insert(b.id, i);
        }
    }

    /// O(1) 按 BlockId 查块引用（内部 helper，替代 iter().find）。
    fn get_block(&self, id: BlockId) -> Option<&Block> {
        let idx = self.id_index.get(&id)?;
        self.blocks.get(*idx)
    }

    /// 块 id 是否仍存活（O(1)）。
    pub fn contains_block(&self, id: BlockId) -> bool {
        self.id_index.contains_key(&id)
    }

    /// 块在文档中的序号（O(1)）。
    pub fn block_position(&self, id: BlockId) -> Option<usize> {
        self.id_index.get(&id).copied()
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// 最近一次 `reparse` 实际扫描的行跨度（只读观测值，不影响解析结果）。
    /// 口径：重扫起点（含向前扩展/列表区域回走后的 `scan_from`）到扫描终止行（不含）；
    /// 命中旧块对齐而提前收尾时，对齐行自身不计入。全量首扫（`new`）记 0。
    pub fn last_rescan_lines(&self) -> usize {
        self.last_rescan_lines
    }

    /// 某块的 inline 节点（绝对字节偏移）。派生数据：访问时按块当前文本即时解析，
    /// 不持久化、不进撤销栈——块区间由 reparse 维护，故编辑后查询恒正确。
    /// FencedCode/IndentedCode/Math/Table/ThematicBreak 恒为空（无 inline）；未知 id 返回 None。
    /// 列表/引用仅解析标记（`- ` / `> `）之后的内容，标记本身不得被 inline 误认。
    pub fn inlines(&self, buf: &Buffer, block: BlockId) -> Option<Vec<InlineNode>> {
        self.inlines_within(buf, block, usize::MAX)
    }

    /// `inlines` 的前缀版本：只解析块首起 `max_bytes` 以内的 inline 节点（绝对偏移）。
    /// 排版只消费可渲染前缀（超限块仅排前缀），调用方据此避免每次击键解析整块
    /// 10 MB 文本；命中范围之外的节点本就不会进入排版，语义不变。
    pub fn inlines_within(
        &self,
        buf: &Buffer,
        block: BlockId,
        max_bytes: usize,
    ) -> Option<Vec<InlineNode>> {
        let b = self.get_block(block)?;
        if b.kind.has_no_inlines() {
            return Some(Vec::new());
        }
        let start = buf.offset_of_line(b.start_line);
        let block_end = if b.end_line < buf.line_count() {
            buf.offset_of_line(b.end_line)
        } else {
            buf.len()
        };
        let mut end = start.saturating_add(max_bytes).min(block_end);
        while end > start && !buf.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = end < block_end;
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
            BlockKind::BlockQuote | BlockKind::Callout(_) => {
                quote_inlines(&text, start, &self.refs)
            }
            // Setext：inline 仅解析内容行，下划线行不参与（0c P0-2）
            BlockKind::SetextHeading(_) => {
                if truncated {
                    // 前缀不含下划线行：按普通行内容解析，避免为取末行再切整块。
                    parse_inlines(&text, start, &self.refs)
                } else {
                    let ul = buf.offset_of_line(b.end_line - 1);
                    parse_inlines(&buf.slice(start, ul), start, &self.refs)
                }
            }
            _ => parse_inlines(&text, start, &self.refs),
        })
    }

    fn fresh_block(
        &mut self,
        buf: &Buffer,
        kind: BlockKind,
        start_line: usize,
        end_line: usize,
        container: BlockContainer,
        table_lines: Option<usize>,
    ) -> Block {
        let id = BlockId(self.next_id);
        self.next_id += 1;
        Block {
            id,
            kind,
            start_line,
            end_line,
            range: block_byte_range(buf, start_line, end_line),
            container,
            table_lines,
        }
    }

    /// 数学块的 TeX 源码（视图渲染/导出的输入）：多行剥开闭标记行，单行剥两侧 `$$`；
    /// 中间行原样保留（含空行，`\r` 随 lines() 剥除）。非数学块/未知 id 返回 None。
    pub fn math_source(&self, buf: &Buffer, block: BlockId) -> Option<String> {
        let b = self.get_block(block)?;
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
            if t.len() <= 4 {
                // 文末裸 `$$`（无尾随换行）被识别为单行 Math：无内容可剥，返回空公式
                return Some(String::new());
            }
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

    /// 围栏代码块的 lang + body（剥掉开/闭围栏行）。非围栏块/未知 id 返回 None。
    /// body 不含开/闭 ``` 行，适合直接喂给语法高亮或 mermaid.live 预览。
    pub fn fenced_code_source(
        &self,
        buf: &Buffer,
        block: BlockId,
    ) -> Option<(Option<String>, String)> {
        let b = self.get_block(block)?;
        let lang = match &b.kind {
            BlockKind::FencedCode(l) => l.clone(),
            BlockKind::Mermaid => Some("mermaid".to_string()),
            _ => return None,
        };
        let start = buf.offset_of_line(b.start_line);
        let end = if b.end_line < buf.line_count() {
            buf.offset_of_line(b.end_line)
        } else {
            buf.len()
        };
        let text = buf.slice(start, end);
        let mut lines = text.lines();
        lines.next(); // 开围栏行（```mermaid）
                      // 容器内代码块的每行带容器前缀（`>` / 列表缩进）；逐行剥离后才是正文。
                      // 顶层块容器为默认值，剥离函数原样返回，不改变既有行为。
        let mut body: Vec<String> = lines
            .map(|l| strip_container_line(l, b.container, false).to_string())
            .collect();
        // 闭围栏行（```）只有 trim 后才等于 ```，未闭合围栏不会被吞进来
        if let Some(last) = body.last() {
            let trimmed = last.trim_start_matches([' ', '\t']);
            let is_fence = (trimmed.starts_with("```") && !trimmed[3..].contains('`'))
                || (trimmed.starts_with("~~~") && !trimmed[3..].contains('~'));
            if is_fence {
                body.pop();
            }
        }
        Some((lang, body.join("\n")))
    }

    /// 表格派生数据：表头/表体单元格与列对齐。非表格块/未知 id 返回 None。
    /// 单元格文本为「剥除 inline 标记后的可见文本」；区间为 trim 后的内容区间
    /// （绝对字节偏移）。表体行按列数补空/截断（GFM 行为）。
    /// 列数硬上限 128、总行数硬上限 512——超限截断降级不崩溃。
    const MAX_TABLE_COLS: usize = 128;
    const MAX_TABLE_ROWS: usize = 512;

    pub fn table(&self, buf: &Buffer, block: BlockId) -> Option<Table> {
        let b = self.get_block(block)?;
        if b.kind != BlockKind::Table {
            return None;
        }
        let table_lines = b.table_lines.unwrap_or(b.end_line - b.start_line);
        if table_lines < 2 {
            return None;
        }
        let delim_line = line_text(buf, b.start_line + 1);
        let (_, delim_src) = table_line_content(&delim_line, b.container);
        let mut aligns = parse_delim_row(delim_src)?;
        // 列数硬上限截断
        if aligns.len() > Self::MAX_TABLE_COLS {
            aligns.truncate(Self::MAX_TABLE_COLS);
        }
        let head_line = line_text(buf, b.start_line);
        let (head_off, head_src) = table_line_content(&head_line, b.container);
        let header_line_off = buf.offset_of_line(b.start_line);
        let header = parse_table_cells(
            head_src,
            header_line_off + head_off,
            aligns.len(),
            &self.refs,
        );
        let total_body = table_lines.saturating_sub(2);
        let capped_body = total_body.min(Self::MAX_TABLE_ROWS);
        let mut rows = Vec::with_capacity(capped_body);
        for line in b.start_line + 2..(b.start_line + 2 + capped_body) {
            let off = buf.offset_of_line(line);
            let line_text = line_text(buf, line);
            let (content_off, content) = table_line_content(&line_text, b.container);
            rows.push(parse_table_cells(
                content,
                off + content_off,
                aligns.len(),
                &self.refs,
            ));
        }
        Some(Table {
            aligns,
            header,
            rows,
        })
    }

    /// 表格实际字节区间（Details 内表格的块区间可能吞入尾部闭标签行）。
    pub fn table_span(&self, buf: &Buffer, block: BlockId) -> Option<Range> {
        let b = self.get_block(block)?;
        if b.kind != BlockKind::Table {
            return None;
        }
        let lines = b.table_lines.unwrap_or(b.end_line - b.start_line);
        let end_line = (b.start_line + lines).min(b.end_line);
        Some(block_byte_range(buf, b.start_line, end_line))
    }

    /// 块的容器上下文（仅表格块可能非默认）。
    pub fn block_container(&self, block: BlockId) -> BlockContainer {
        self.get_block(block)
            .map_or_else(BlockContainer::default, |b| b.container)
    }

    /// 标题块派生数据：(级别, 可见文本)。ATX 标记/闭合序列与 inline 标记均剥除
    /// （`## **B**` → `B`）；Setext 取内容行（末行下划线除外）。非标题块/未知 id
    /// 返回 None（大纲面板的数据来源）。
    pub fn heading(&self, buf: &Buffer, block: BlockId) -> Option<(u8, String)> {
        let b = self.get_block(block)?;
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
                Some((
                    level,
                    visible_text(&content, &nodes, start).trim().to_string(),
                ))
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
        // details 子块回走（B3）：子块是独立块，但其容器上下文由所属 Details 头块
        // 的扫描产物给出。编辑点落在子块 / 闭标签行 / 组内空行时，必须回到最外层
        // Details 头块重扫，否则子块会丢掉 `details_depth` 归属、闭标签行也会
        // 被当成正文重新解析。
        if bi > 0 && bi < self.blocks.len() {
            // 编辑点正好在闭标签行（不属于任何块）：回退到其前的末个子块。
            if self.blocks[bi - 1].container.details_depth > 0
                && edit_start_line >= self.blocks[bi - 1].end_line
                && edit_start_line <= self.blocks[bi].start_line
            {
                bi -= 1;
            }
        }
        bi = self.details_root_index(bi);
        let scan_from = if bi < self.blocks.len() {
            self.blocks[bi].start_line.min(edit_start_line)
        } else {
            edit_start_line
        };
        let total = buf.line_count();

        // 重扫：超过新区后，遇到与旧块（平移后）对齐且同种的边界即停，
        // 其后的旧块整体平移保留——这是「干净块不动」承诺的实现。
        // 扫描单元为 scan_next：列表区域整体产出（0c P0-3）。
        let mut produced: Vec<ScannedBlock> = Vec::new();
        let mut suffix_start: Option<usize> = None;
        let mut line = scan_from;
        while line < total {
            if line_text(buf, line).trim().is_empty() {
                line += 1;
                continue;
            }
            let (mut v, consumed) = scan_next_meta(buf, line, total);
            // 先把块扫出来再比对，避免为校验额外扫一遍（旧写法在此处再调
            // scan_next 会退化为 O(n²)）。起始行 + 种类 + **结束行**三者都平移
            // 对齐才算「干净块」；只比起始行与种类时，块边界变化（内容已改，
            // 但首行恰好对齐）仍会复用旧 BlockId，派生缓存于是命中陈旧布局。
            if line > new_end_line {
                if let Some(k) = (bi..self.blocks.len()).find(|&k| {
                    let shifted = self.blocks[k].start_line as isize + delta;
                    shifted >= 0 && shifted as usize == line
                }) {
                    let (nk, ne) = (&v[0].kind, v[0].end);
                    let shifted_end = self.blocks[k].end_line as isize + delta;
                    if *nk == self.blocks[k].kind && shifted_end >= 0 && shifted_end as usize == ne
                    {
                        suffix_start = Some(k);
                        break; // 此后旧块整体平移保留，不再追加刚扫出的 v
                    }
                }
            }
            produced.append(&mut v);
            line += consumed;
        }

        // 观测口径：`scan_from` 到扫描终止行（不含）的行数，即本次逐行检查的跨度；
        // 命中旧块对齐提前收尾时，对齐行未进入 produced，不计入。
        self.last_rescan_lines = line.saturating_sub(scan_from);

        let mut dirty = Vec::with_capacity(produced.len());
        // 前缀原样保留、后缀整片平移：旧块容器按值搬移，避免每次击键克隆
        // 整篇 BlockKind（列表/围栏语言字符串）。旧块只在此处被释放。
        let mut new_blocks = std::mem::take(&mut self.blocks);
        let rest = new_blocks.split_off(bi);
        let replaced_count = suffix_start.map_or(rest.len(), |k| k - bi);
        let replaced_ids: Vec<BlockId> = rest.iter().take(replaced_count).map(|b| b.id).collect();
        let produced_count = produced.len();
        let mut produced_footnote_marker = false;
        let mut produced_ref_definition = false;
        for (i, sb) in produced.into_iter().enumerate() {
            let kind = self.classify_callout(buf, sb.kind, sb.start);
            let (kind, s, e) = (kind, sb.start, sb.end);
            // zip-reuse：与旧块同相对位次且种类一致时，区间不触编辑区
            // [edit_start_line, new_end_line) ⇒ 内容必然未变 ⇒ 保留 id 不脏
            // （0c P0-3：列表区域整体重扫时，编辑点之前的同区域项由此保 id）。
            // 编辑区之前的块坐标不平移，之后的块平移 delta。
            let reused = rest.get(i).and_then(|o| {
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
                    // 重扫区内的块按新行号一次性换算字节区间（代价只与脏块数相关）。
                    range: block_byte_range(buf, s, e),
                    container: sb.container,
                    table_lines: sb.table_lines,
                },
                None => {
                    let b = self.fresh_block(buf, kind, s, e, sb.container, sb.table_lines);
                    dirty.push(b.id);
                    b
                }
            };
            new_blocks.push(block);
        }
        if let Some(k) = suffix_start {
            // 编辑点之后整片平移：用首个保留块对齐出字节 delta（新首字节 - 旧首字节），
            // 其后所有块只做一次算术加法，不逐块重新做行号换算。
            let rel = k - bi;
            let first = &rest[rel];
            let shifted_start_line = (first.start_line as isize + delta) as usize;
            let byte_delta =
                buf.offset_of_line(shifted_start_line) as isize - first.range.start as isize;
            for b in rest.into_iter().skip(rel) {
                // 已对齐块 shifted >= 0；其后块起始行更大，平移后必然非负
                new_blocks.push(Block {
                    id: b.id,
                    kind: b.kind,
                    start_line: (b.start_line as isize + delta) as usize,
                    end_line: (b.end_line as isize + delta) as usize,
                    range: Range::new(
                        (b.range.start as isize + byte_delta) as usize,
                        (b.range.end as isize + byte_delta) as usize,
                    ),
                    container: b.container,
                    table_lines: b.table_lines,
                });
            }
        }
        // 派生表按需重建：普通文档（无脚注/引用定义）不逐块切片全文扫描。
        for b in &new_blocks[bi..bi + produced_count] {
            if matches!(b.kind, BlockKind::LinkRefDef) {
                produced_ref_definition = true;
            }
            if b.kind == BlockKind::FootnoteDef {
                produced_footnote_marker = true;
            }
            if b.kind.has_no_inlines() {
                continue;
            }
            let text = buf.slice(b.range.start, b.range.end);
            if text.contains("[^") {
                produced_footnote_marker = true;
            }
            if b.kind == BlockKind::BlockQuote && text.contains('[') {
                produced_ref_definition = true;
            }
        }
        self.blocks = new_blocks;
        if produced_ref_definition || !self.refs.is_empty() {
            self.build_refs(buf);
        }
        // 同上：编号表经 `inlines` 查块，须在 id 索引重建之后。
        // 位次稳定（脏区新旧块数相同）时只替换脏区 id；块数变化才整表重建。
        if produced_count == replaced_count {
            for id in &replaced_ids {
                self.id_index.remove(id);
            }
            for (i, b) in self.blocks[bi..bi + produced_count].iter().enumerate() {
                self.id_index.insert(b.id, bi + i);
            }
        } else {
            self.rebuild_id_index();
        }
        if produced_footnote_marker
            || !self.footnote_numbers.is_empty()
            || !self.footnote_defs.is_empty()
        {
            self.build_footnotes(buf);
        }
        debug_assert!(self
            .blocks
            .iter()
            .all(|b| b.start_line <= b.end_line && b.range.end <= buf.len()));
        #[cfg(test)]
        self.debug_assert_ranges(buf);
        dirty
    }

    /// details 归属回走：`bi` 指向块级 details 子块（或内层 Details 头块）时，
    /// 返回其最外层 Details 头块索引；否则原样返回。
    fn details_root_index(&self, mut bi: usize) -> usize {
        if bi >= self.blocks.len() {
            bi = self.blocks.len().saturating_sub(1);
        }
        loop {
            let Some(b) = self.blocks.get(bi) else {
                return bi;
            };
            let depth = b.container.details_depth;
            if depth == 0 {
                return bi;
            }
            let want = depth - 1;
            match (0..bi).rev().find(|&k| {
                matches!(self.blocks[k].kind, BlockKind::Details { .. })
                    && self.blocks[k].container.details_depth == want
            }) {
                Some(parent) => bi = parent,
                None => return bi,
            }
        }
    }

    /// 单元测试专用的 O(块数) 不变量校验：字节区间必须等于行号的换算结果。
    /// 仅编译进 crate 单元测试（集成测试链接不带 `cfg(test)` 的库，生产/普通 debug 无成本）。
    #[cfg(test)]
    fn debug_assert_ranges(&self, buf: &Buffer) {
        let total = buf.line_count();
        for b in &self.blocks {
            assert_eq!(
                b.range.start,
                buf.offset_of_line(b.start_line),
                "Block.start 不匹配 start_line 首字节"
            );
            let end = if b.end_line >= total {
                buf.len()
            } else {
                buf.offset_of_line(b.end_line)
            };
            assert_eq!(b.range.end, end, "Block.end 不匹配 end_line 首字节");
        }
    }
}

/// 行区间 → 字节区间：`start` 为 `start_line` 首字节，`end` 为 `end_line` 行首；
/// `end_line` 越过末行时取文档长度。仅在新建/重扫块时调用。
fn block_byte_range(buf: &Buffer, start_line: usize, end_line: usize) -> Range {
    let end = if end_line >= buf.line_count() {
        buf.len()
    } else {
        buf.offset_of_line(end_line)
    };
    Range::new(buf.offset_of_line(start_line), end)
}

/// 容器内表格行的内容切片：剥除 `>` 前缀（按容器层数）与列表标记/续行缩进，
/// 返回 (内容字节偏移, 内容)。顶层表格原样返回。单元格解析与列对齐必须基于
/// 剥前缀后的内容，否则 `> | a |` 的首格会混入 `>`、`- | a |` 会混入标记。
fn table_line_content(line: &str, container: BlockContainer) -> (usize, &str) {
    let mut rest = line;
    let mut off = 0usize;
    for _ in 0..container.quote_depth {
        let Some(p) = quote_marker_pos(rest) else {
            break;
        };
        let mut e = p + 1;
        if matches!(rest.as_bytes().get(e), Some(b' ') | Some(b'\t')) {
            e += 1;
        }
        rest = &rest[e..];
        off += e;
    }
    if container.list_indent > 0 {
        if let Some(m) = list_mark(rest) {
            off += m.content_start;
            rest = &rest[m.content_start..];
        } else {
            let (_, start) = indent_info(rest);
            off += start;
            rest = &rest[start..];
        }
    }
    (off, rest)
}

/// 一个强调结构的标记/内容区间（`*` 系 Emph、`**` 系 Strong），半开区间。
/// 供编辑层做结构感知的 toggle 判定；渲染层仍直接消费 [`InlineNode`]。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EmphasisSpan {
    /// `Emph` 或 `Strong`。
    pub kind: InlineKind,
    /// 开标记区间。
    pub open: Range,
    /// 内容区间（不含标记）。
    pub content: Range,
    /// 闭标记区间。
    pub close: Range,
}

/// 解析一段文本的全部强调结构，按内容长度升序（最内层优先）。
///
/// 编辑层据此判定「光标是否落在某个已存在的强调结构内」「选区是否恰为某个
/// 结构的整段或内容段」——比按单个字符奇偶配对可靠：`***b***`、`*a* *b*`
/// 中间的空白、代码段内的 `*` 等歧义都交由解析器裁决（与渲染所见完全一致）。
/// 无强调/空文本返回空。
pub fn emphasis_spans(text: &str) -> Vec<EmphasisSpan> {
    let refs = RefMap::new();
    let nodes = parse_inlines(text, 0, &refs);
    let mut out = Vec::new();
    collect_emphasis(&nodes, &mut out);
    out.sort_by_key(|s| s.content.len());
    out
}

fn collect_emphasis(nodes: &[InlineNode], out: &mut Vec<EmphasisSpan>) {
    for n in nodes {
        if matches!(n.kind, InlineKind::Emph | InlineKind::Strong) {
            out.push(EmphasisSpan {
                kind: n.kind.clone(),
                open: Range::new(n.range.start, n.content.start),
                content: n.content,
                close: Range::new(n.content.end, n.range.end),
            });
        }
        collect_emphasis(&n.children, out);
    }
}

/// 块文本剥掉给定图片源码后是否只剩空白（独占一行图片的判定基础）。
/// `img_ranges` 为文档绝对区间；`text` 是 `block` 区间内的块文本。
pub fn block_is_image_only(text: &str, block: Range, img_ranges: &[Range]) -> bool {
    if img_ranges.is_empty() {
        return false;
    }
    let mut keep = String::new();
    let mut pos = 0usize;
    let mut ranges: Vec<Range> = img_ranges.to_vec();
    ranges.sort_by_key(|r| r.start);
    for r in ranges {
        let s = r.start.saturating_sub(block.start).min(text.len());
        let e = r.end.saturating_sub(block.start).min(text.len());
        if !(text.is_char_boundary(s) && text.is_char_boundary(e)) || s < pos || s >= e {
            continue;
        }
        keep.push_str(&text[pos..s]);
        pos = e;
    }
    if pos < text.len() {
        keep.push_str(&text[pos..]);
    }
    keep.trim().is_empty()
}

/// 单张图片是否「独占一行」：图片所在的**视觉行**（两个换行之间）除该图源码外只剩空白。
/// 多行段落里图片之后另起一行的正文（如 `![a](p.png)\nX`）不应把这张图降级为
/// 行内小图——它自己的那一行仍是纯图片；同一行内有任意正文时返回 false。
/// 视图、HTML 与 PDF 导出共用此判定。
pub fn image_standalone(text: &str, block: Range, img: Range) -> bool {
    if img.start < block.start || img.end > block.end || img.start >= img.end {
        return false;
    }
    let s = img.start - block.start;
    let e = img.end - block.start;
    if e > text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
        return false;
    }
    let line_start = text[..s].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[e..].find('\n').map_or(text.len(), |i| e + i);
    let line = &text[line_start..line_end];
    let base = block.start + line_start;
    block_is_image_only(
        line,
        Range::new(base, block.start + line_end),
        &[Range::new(base + (s - line_start), base + (e - line_start))],
    )
}

/// 段落只含一张独立图片时返回 `(图片区间, alt 题注)`；行内、多图与空 alt 均不派生题注。
/// 屏幕题注、HTML `<figcaption>` 与 PDF 题注共用本函数，保证三端语义一致。
pub fn standalone_image_caption(
    nodes: &[InlineNode],
    text: &str,
    base: usize,
) -> Option<(Range, String)> {
    let mut images = nodes.iter().filter(|n| n.is_image());
    let node = images.next()?;
    if images.next().is_some() {
        return None;
    }
    let block = Range::new(base, base + text.len());
    if !image_standalone(text, block, node.range) {
        return None;
    }
    let alt = match &node.kind {
        InlineKind::Image => {
            text.get(node.content.start.checked_sub(base)?..node.content.end.checked_sub(base)?)?
        }
        InlineKind::HtmlSelfClosed { attrs, .. } => attrs.alt.as_deref()?,
        _ => return None,
    };
    let caption = alt.trim().replace(['\r', '\n'], " ");
    (!caption.is_empty()).then_some((node.range, caption))
}

#[cfg(test)]
mod image_tests {
    use super::*;

    fn inlines(text: &str) -> Vec<InlineNode> {
        parse_inlines(text, 0, &RefMap::new())
    }

    #[test]
    fn image_only_block_ignores_surrounding_whitespace() {
        let r = Range::new(0, 11);
        assert!(
            block_is_image_only("![a](p.png)", r, &[r]),
            "整块只有图片应判定为独占一行"
        );
        let r = Range::new(2, 13);
        assert!(
            block_is_image_only("  ![a](p.png)  ", Range::new(0, 15), &[r]),
            "图片前后空白不影响判定"
        );
        // 块文本常带尾换行：仅空白残留仍应算独占一行（否则会误走行内 4 行限高）
        let r = Range::new(0, 17);
        assert!(
            block_is_image_only("<img src=\"a.png\">\n", Range::new(0, 18), &[r]),
            "仅剩换行也算独占一行"
        );
        assert!(
            !block_is_image_only("x ![a](p.png)", Range::new(0, 13), &[Range::new(2, 13)]),
            "有正文时按行内图处理"
        );
    }

    #[test]
    fn image_standalone_by_visual_line() {
        let r = Range::new(0, 11);
        assert!(image_standalone("![a](p.png)\n", Range::new(0, 12), r));
        // 图片后另起一行的正文：图片仍独占自己的视觉行
        assert!(
            image_standalone("![a](p.png)\nX", Range::new(0, 13), r),
            "下一行正文不应把图片降级为行内"
        );
        assert!(!image_standalone(
            "x ![a](p.png)",
            Range::new(0, 13),
            Range::new(2, 13)
        ));
        assert!(!image_standalone(
            "![a](p.png) x",
            Range::new(0, 13),
            Range::new(0, 11)
        ));
        // 图片前一行有正文不影响本行
        assert!(image_standalone(
            "Y\n![a](p.png)\nX",
            Range::new(0, 15),
            Range::new(2, 13)
        ));
        // 区间越界/空区间防御
        assert!(!image_standalone("abc", Range::new(0, 3), Range::new(0, 2)));
        assert!(!image_standalone("abc", Range::new(0, 3), Range::new(5, 6)));
    }

    #[test]
    fn standalone_caption_rule_is_shared_and_stable() {
        // 单张独立图片：出题注（Markdown alt）
        let text = "![a](p.png)\n";
        let nodes = inlines(text);
        assert_eq!(
            standalone_image_caption(&nodes, text, 0),
            Some((Range::new(0, 11), "a".to_string()))
        );
        // `<img>` 的 alt 同样出题注
        let text = "<img src=\"p.png\" alt=\"图\">\n";
        let nodes = inlines(text);
        assert_eq!(
            standalone_image_caption(&nodes, text, 0).unwrap().1,
            "图".to_string()
        );
        // 同段落两张图（换行或同一行）都不出题注
        for text in ["![a](p.png)\n![b](q.png)\n", "![a](p.png) ![b](q.png)\n"] {
            let nodes = inlines(text);
            assert_eq!(standalone_image_caption(&nodes, text, 0), None, "{text}");
        }
        // 行内图片（同行有正文）不出题注
        let text = "x ![a](p.png)\n";
        let nodes = inlines(text);
        assert_eq!(standalone_image_caption(&nodes, text, 0), None);
        // 空 alt 不出题注
        let text = "![](p.png)\n";
        let nodes = inlines(text);
        assert_eq!(standalone_image_caption(&nodes, text, 0), None);
    }
}

/// 按文档顺序（前序）收集脚注引用并编号：首次出现的 id 取下一个递增编号。
/// 编号只由引用决定，定义块不参与；这也保证了「未被引用的定义不占号」。
fn collect_footnote_refs(
    nodes: &[InlineNode],
    buf: &Buffer,
    numbers: &mut std::collections::HashMap<String, usize>,
    ids: &mut Vec<String>,
) {
    for n in nodes {
        if n.kind == InlineKind::FootnoteRef {
            let id = buf.slice(n.content.start, n.content.end);
            if let std::collections::hash_map::Entry::Vacant(e) = numbers.entry(id) {
                ids.push(e.key().clone());
                e.insert(ids.len());
            }
        }
        collect_footnote_refs(&n.children, buf, numbers, ids);
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

/// 引用块首行 `[!NAME]` 的名字（大小写原样）。仅用于自定义 Callout 分类，
/// 与 `block::scan_one` 的内置识别口径一致（行首 `>` + `[!` + `]`）。
fn callout_marker_name(line: &str) -> Option<String> {
    let rest = line.trim_start_matches('>').trim_start();
    let inner = rest.strip_prefix("[!")?;
    let close = inner.find(']')?;
    let name = inner[..close].trim();
    (!name.is_empty()).then(|| name.to_string())
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
    fn emphasis_spans_expose_triple_star_structure() {
        // `***b***` → Emph 在外、Strong 在内；最内层优先（内容长度升序）
        let spans = emphasis_spans("***b***");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].kind, InlineKind::Strong);
        assert_eq!(spans[0].open, Range::new(1, 3));
        assert_eq!(spans[0].content, Range::new(3, 4));
        assert_eq!(spans[0].close, Range::new(4, 6));
        assert_eq!(spans[1].kind, InlineKind::Emph);
        assert_eq!(spans[1].open, Range::new(0, 1));
        assert_eq!(spans[1].content, Range::new(1, 6));
        assert_eq!(spans[1].close, Range::new(6, 7));
    }

    #[test]
    fn emphasis_spans_skip_code_and_unmatched_markers() {
        // 代码段内的 `*` 不产生强调；未闭合的 `**` 同理
        assert!(emphasis_spans("`*a*`").is_empty());
        assert!(emphasis_spans("**未闭合").is_empty());
        // 两段独立斜体各为一对，中间的空白不属于任何结构
        let spans = emphasis_spans("*a* *b*");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, Range::new(1, 2));
        assert_eq!(spans[1].content, Range::new(5, 6));
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
    fn math_source_single_line_edge_cases() {
        // 文末裸 `$$`（无尾随换行）：单行分支不得下溢 panic
        let buf = Buffer::new("$$");
        let s = Syntax::new(&buf);
        assert_eq!(s.math_source(&buf, s.blocks()[0].id).as_deref(), Some(""));
        // 有尾随换行：`$$` 走多行分支（空标记对），同为空公式
        let buf = Buffer::new("$$\n");
        let s = Syntax::new(&buf);
        assert_eq!(s.math_source(&buf, s.blocks()[0].id).as_deref(), Some(""));
        // `$$$$`：标记长 = 4，按段落（非 Math），故 None
        let buf = Buffer::new("$$$$");
        let s = Syntax::new(&buf);
        assert_eq!(s.math_source(&buf, s.blocks()[0].id), None);
        // 正常单行公式剥两侧标记
        let buf = Buffer::new("$$x$$");
        let s = Syntax::new(&buf);
        assert_eq!(s.math_source(&buf, s.blocks()[0].id).as_deref(), Some("x"));
        // 标记内空白：trim 只剥行首尾，内容空白原样保留
        let buf = Buffer::new("$$  $$");
        let s = Syntax::new(&buf);
        assert_eq!(s.math_source(&buf, s.blocks()[0].id).as_deref(), Some("  "));
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
    fn table_derivation_strips_container_prefixes() {
        // 引用 / 列表 / details 内表格：单元格文本与区间必须基于剥前缀后的内容。
        let quote = Buffer::new("> | a | b |\n> | --- | --- |\n> | 1 | 2 |\n");
        let s = Syntax::new(&quote);
        assert_eq!(s.blocks()[0].kind, BlockKind::Table);
        let t = s.table(&quote, s.blocks()[0].id).unwrap();
        assert_eq!(t.header[0].text, "a");
        assert_eq!(t.header[1].text, "b");
        assert_eq!(t.rows[0][0].text, "1");
        let r = t.header[0].range;
        assert_eq!(&quote.slice(r.start, r.end), "a");

        let list = Buffer::new("- | k | v |\n  | --- | --- |\n  | 1 | 2 |\n");
        let s = Syntax::new(&list);
        assert_eq!(s.blocks()[0].kind, BlockKind::Table);
        let t = s.table(&list, s.blocks()[0].id).unwrap();
        assert_eq!(t.header[0].text, "k");
        assert_eq!(t.header[1].text, "v");
        let r = t.rows[0][0].range;
        assert_eq!(&list.slice(r.start, r.end), "1");

        let details =
            Buffer::new("<details>\n<summary>S</summary>\n| a |\n| --- |\n| 1 |\n</details>\n");
        let s = Syntax::new(&details);
        let block = &s.blocks()[1];
        assert_eq!(block.kind, BlockKind::Table);
        let t = s.table(&details, block.id).unwrap();
        assert_eq!(t.header[0].text, "a");
        assert_eq!(t.rows[0][0].text, "1");
        let span = s.table_span(&details, block.id).unwrap();
        assert_eq!(
            &details.slice(span.start, span.end),
            "| a |\n| --- |\n| 1 |\n"
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
    fn table_cell_br_and_html_img_visible_text() {
        let b = Buffer::new("| a<br>b | <img src=\"p.png\" alt=\"图\"> |\n| --- | --- |\n");
        let s = Syntax::new(&b);
        let t = s.table(&b, s.blocks()[0].id).unwrap();
        assert_eq!(
            t.header[0].text, "a\nb",
            "<br> 在单元格可见文本中应为硬换行"
        );
        assert_eq!(t.header[1].text, "图", "HTML img 的可见文本应为 alt");
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
        assert_eq!(
            sexp(&s.inlines(&b, chunk.id).unwrap()),
            "Emph[13..16 c=14..15]"
        );
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
                (
                    BlockKind::ListItem {
                        ordered: false,
                        depth: 0,
                        task: None
                    },
                    0,
                    1
                ),
                (
                    BlockKind::ListItem {
                        ordered: false,
                        depth: 1,
                        task: None
                    },
                    1,
                    2
                ),
                (
                    BlockKind::ListItem {
                        ordered: false,
                        depth: 0,
                        task: None
                    },
                    2,
                    3
                ),
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
        assert_eq!((s.blocks()[0].start_line, s.blocks()[0].end_line), (0, 2));
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

    // ===== GFM 脚注（定义块 / 引用 inline / 派生编号表） =====

    #[test]
    fn footnote_def_block_and_indented_continuation() {
        // 续行须缩进 ≥4 列（与缩进代码块同口径）；末尾空行不计入块区间
        let b = Buffer::new("正文\n\n[^1]: 第一行\n    续行\n\n尾段\n");
        let s = Syntax::new(&b);
        let kinds: Vec<_> = s.blocks().iter().map(|b| b.kind.clone()).collect();
        assert_eq!(
            kinds,
            [
                BlockKind::Paragraph,
                BlockKind::FootnoteDef,
                BlockKind::Paragraph
            ]
        );
        let def = &s.blocks()[1];
        assert_eq!((def.start_line, def.end_line), (2, 4));
        // 定义块按源码渲染：无 inline（不背叛源码，与 LinkRefDef 一致）
        assert_eq!(s.inlines(&b, def.id), Some(Vec::new()));
    }

    #[test]
    fn footnote_def_never_becomes_link_ref_def() {
        // 判定优先 + parse_link_ref_def 显式排除 `^`：`[^1]: …` 只能是脚注定义
        let b = Buffer::new("[^1]: /url\n");
        let s = Syntax::new(&b);
        assert_eq!(s.blocks().len(), 1);
        assert_eq!(s.blocks()[0].kind, BlockKind::FootnoteDef);
        // 文档内 `[^1]` 仍是脚注引用，不是 shortcut 链接
        let b2 = Buffer::new("[^1]\n\n[^1]: /url\n");
        let s2 = Syntax::new(&b2);
        assert_eq!(
            sexp(&s2.inlines(&b2, s2.blocks()[0].id).unwrap()),
            "FootnoteRef[0..4 c=2..3]"
        );
    }

    #[test]
    fn footnote_def_indentation_and_non_continuation() {
        // ≤3 空格合法
        let b = Buffer::new("   [^a]: x\n");
        let s = Syntax::new(&b);
        assert_eq!(s.blocks()[0].kind, BlockKind::FootnoteDef);
        // 4 空格属缩进代码领地
        let b = Buffer::new("    [^a]: x\n");
        let s = Syntax::new(&b);
        assert_eq!(s.blocks()[0].kind, BlockKind::IndentedCode);
        // 非缩进续行另起段落
        let b = Buffer::new("[^a]: x\nnot continuation\n");
        let s = Syntax::new(&b);
        assert_eq!(s.blocks().len(), 2);
        assert_eq!(s.blocks()[1].kind, BlockKind::Paragraph);
        // 定义不打断进行中段落（与 LinkRefDef 同构）
        let b = Buffer::new("文本\n[^a]: x\n");
        let s = Syntax::new(&b);
        assert_eq!(s.blocks().len(), 1);
        assert_eq!(s.blocks()[0].kind, BlockKind::Paragraph);
    }

    #[test]
    fn footnote_ref_skipped_in_code_and_fenced_blocks() {
        // 代码跨度内不解析
        let b = Buffer::new("`[^1]`\n");
        let s = Syntax::new(&b);
        assert_eq!(
            sexp(&s.inlines(&b, s.blocks()[0].id).unwrap()),
            "Code[0..6 c=1..5]"
        );
        // 围栏代码块无 inline
        let b = Buffer::new("```\n[^1]\n```\n");
        let s = Syntax::new(&b);
        assert_eq!(s.inlines(&b, s.blocks()[0].id), Some(Vec::new()));
    }

    #[test]
    fn footnote_ref_not_parsed_in_image_syntax() {
        let b = Buffer::new("![^1]\n");
        let s = Syntax::new(&b);
        assert!(s.inlines(&b, s.blocks()[0].id).unwrap().is_empty());
        // 即使存在同名定义，`![^id]` 仍属图片语法，不得拆出脚注引用
        let b = Buffer::new("![^1]\n\n[^1]: x\n");
        let s = Syntax::new(&b);
        assert!(s.inlines(&b, s.blocks()[0].id).unwrap().is_empty());
    }

    #[test]
    fn footnote_ref_without_definition_still_parsed() {
        // 解析层不依赖定义：缺失定义也产出节点（视图按编号表缺失降级纯文本）
        let b = Buffer::new("无定义 [^ghost]\n");
        let s = Syntax::new(&b);
        assert_eq!(
            sexp(&s.inlines(&b, s.blocks()[0].id).unwrap()),
            "FootnoteRef[10..18 c=12..17]"
        );
        assert_eq!(s.footnote_number("ghost"), Some(1));
        assert_eq!(s.footnote_definition("ghost"), None);
    }

    #[test]
    fn footnote_numbers_follow_first_reference_order() {
        let b = Buffer::new("[^b] [^a] [^b]\n\n[^a]: A\n[^b]: B\n[^c]: C\n");
        let s = Syntax::new(&b);
        // 首次引用顺序：b 先于 a；同 id 重复引用同号
        assert_eq!(s.footnote_number("b"), Some(1));
        assert_eq!(s.footnote_number("a"), Some(2));
        // 未被引用的定义不占号
        assert_eq!(s.footnote_number("c"), None);
        assert_eq!(s.footnote_ids(), ["b", "a"]);
    }

    #[test]
    fn footnote_ref_inside_details_body_is_numbered() {
        // details 正文拆成兄弟块，脚注按文档顺序编号、引用与定义一一对应
        let b = Buffer::new(
            "<details>\n<summary>更多</summary>\n\n正文 [^d]\n\n</details>\n\n[^d]: 定义\n",
        );
        let s = Syntax::new(&b);
        assert_eq!(s.footnote_number("d"), Some(1));
        assert_eq!(s.footnote_ids(), ["d"]);
        assert!(s.footnote_definition("d").is_some());
    }

    #[test]
    fn no_inline_block_kinds_single_source() {
        // 共享判定：代码/数学/表格/分隔线/引用定义/脚注定义/HTML/FrontMatter 无 inline
        for kind in [
            BlockKind::FencedCode(None),
            BlockKind::IndentedCode,
            BlockKind::Math,
            BlockKind::Table,
            BlockKind::ThematicBreak,
            BlockKind::LinkRefDef,
            BlockKind::FootnoteDef,
            BlockKind::HtmlBlock,
            BlockKind::FrontMatter(String::new()),
        ] {
            assert!(kind.has_no_inlines(), "{kind:?}");
        }
        // Details/段落/标题/列表/引用含 inline
        for kind in [
            BlockKind::Details {
                summary: "s".into(),
                open_default: false,
            },
            BlockKind::Paragraph,
            BlockKind::AtxHeading(1),
            BlockKind::ListItem {
                ordered: false,
                depth: 0,
                task: None,
            },
            BlockKind::BlockQuote,
        ] {
            assert!(!kind.has_no_inlines(), "{kind:?}");
        }
    }

    #[test]
    fn footnote_duplicate_definition_first_wins() {
        let b = Buffer::new("[^a]: first\n[^a]: second\n");
        let s = Syntax::new(&b);
        assert_eq!(s.blocks().len(), 2);
        let r = s.footnote_definition("a").unwrap();
        assert_eq!(&b.slice(0, b.len())[r.start..r.end], " first");
    }

    #[test]
    fn footnote_chinese_id() {
        let b = Buffer::new("脚注[^中文-id_2]。\n\n[^中文-id_2]: 定义\n");
        let s = Syntax::new(&b);
        assert_eq!(s.footnote_number("中文-id_2"), Some(1));
        assert_eq!(s.footnote_ids(), ["中文-id_2"]);
        let r = s.footnote_definition("中文-id_2").unwrap();
        assert_eq!(&b.slice(0, b.len())[r.start..r.end], " 定义");
    }

    #[test]
    fn footnote_def_inside_blockquote_uses_container_stripping() {
        // 引用块内定义走 LinkRefDef 同一容器剥离路径：定义行不产生 inline，
        // 引用块本身仍为 BlockQuote（不是段落，也不误判成顶层 FootnoteDef）
        let b = Buffer::new("> 引用\n>\n> [^n]: 定义\n");
        let s = Syntax::new(&b);
        assert_eq!(s.blocks().len(), 1);
        assert_eq!(s.blocks()[0].kind, BlockKind::BlockQuote);
        assert!(s.inlines(&b, s.blocks()[0].id).unwrap().is_empty());
        // 定义行紧跟正文（无空行）时按段落续行处理，不能拆成定义
        let b = Buffer::new("> text\n> [^n]: 定义\n");
        let s = Syntax::new(&b);
        assert_eq!(
            sexp(&s.inlines(&b, s.blocks()[0].id).unwrap()),
            "FootnoteRef[9..13 c=11..12]"
        );
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
        let p = s
            .blocks()
            .iter()
            .find(|x| x.kind == BlockKind::Paragraph)
            .unwrap();
        let nodes = s.inlines(&b, p.id).unwrap();
        let url = nodes[0].url.unwrap();
        assert_eq!(&b.slice(0, b.len())[url.start..url.end], "/new");
    }

    // ===== N3c：`==高亮==` 的跨行/表格边界 =====

    #[test]
    fn highlight_is_confined_to_one_line() {
        let b = Buffer::new("==a\nb==\n");
        let s = Syntax::new(&b);
        let p = s.blocks()[0].id;
        let nodes = s.inlines(&b, p).unwrap();
        assert!(
            nodes.iter().all(|n| n.kind != InlineKind::Highlight),
            "跨行不得识别为高亮"
        );
    }

    #[test]
    fn highlight_inside_table_cell() {
        let b = Buffer::new("| a |\n| --- |\n| ==x== |\n");
        let s = Syntax::new(&b);
        let id = s.blocks()[0].id;
        let t = s.table(&b, id).unwrap();
        let kinds: Vec<InlineKind> = t.rows[0][0]
            .inlines
            .iter()
            .map(|n| n.kind.clone())
            .collect();
        assert_eq!(kinds, vec![InlineKind::Highlight], "表格单元格内同样生效");
    }

    // ===== N3b：自定义 Callout 分类 =====

    fn registry() -> CalloutRegistry {
        CalloutRegistry::from_json(
            r##"[{"name":"FIXME","label":"待修复","color":"#c00","icon":"!"}]"##,
        )
    }

    #[test]
    fn custom_callout_is_classified_when_configured() {
        let b = Buffer::new("> [!FIXME] 内容\n> 第二行\n");
        let s = Syntax::new_with_callouts(&b, &registry());
        match &s.blocks()[0].kind {
            BlockKind::Callout(c) => {
                assert_eq!(c.name, "fixme");
                assert_eq!(c.label, "待修复");
                assert_eq!(c.color, [204, 0, 0]);
            }
            other => panic!("应分类为 Callout: {other:?}"),
        }
        // inline 仍按引用块口径（剥 `>`）：`[!FIXME]` 不是行内节点，内容文本照常解析
        let nodes = s.inlines(&b, s.blocks()[0].id).unwrap();
        assert!(
            nodes.iter().all(|n| !n.range.is_empty()),
            "inline 区间不得为空"
        );
    }

    #[test]
    fn custom_callout_name_is_case_insensitive() {
        let b = Buffer::new("> [!fixme] x\n");
        let s = Syntax::new_with_callouts(&b, &registry());
        assert!(matches!(&s.blocks()[0].kind, BlockKind::Callout(c) if c.name == "fixme"));
    }

    #[test]
    fn unknown_callout_falls_back_to_blockquote() {
        for src in ["> [!BOGUS] x\n", "> [!NOTE2] x\n"] {
            let s = Syntax::new_with_callouts(&Buffer::new(src), &registry());
            assert_eq!(
                s.blocks()[0].kind,
                BlockKind::BlockQuote,
                "未知类型回退普通引用块: {src}"
            );
        }
    }

    #[test]
    fn builtin_alerts_win_over_custom_registry() {
        let b = Buffer::new("> [!NOTE] x\n");
        let s = Syntax::new_with_callouts(&b, &registry());
        assert_eq!(
            s.blocks()[0].kind,
            BlockKind::Admonition(AdmonitionType::Note)
        );
    }

    #[test]
    fn five_builtin_alert_types_and_warn_alias() {
        for (src, ty) in [
            ("> [!NOTE] x\n", AdmonitionType::Note),
            ("> [!TIP] x\n", AdmonitionType::Tip),
            ("> [!IMPORTANT] x\n", AdmonitionType::Important),
            ("> [!WARNING] x\n", AdmonitionType::Warning),
            ("> [!CAUTION] x\n", AdmonitionType::Caution),
            ("> [!warn] x\n", AdmonitionType::Warning),
            ("> [!caution] x\n", AdmonitionType::Caution),
        ] {
            let s = Syntax::new(&Buffer::new(src));
            assert_eq!(
                s.blocks()[0].kind,
                BlockKind::Admonition(ty),
                "内置类型识别（大小写不敏感 + WARN 别名）: {src}"
            );
        }
    }

    #[test]
    fn builtin_callout_visual_tokens_are_distinct() {
        // 每种都有统一图标 / 强调色；互不重复，便于真机区分。
        let all = [
            AdmonitionType::Note,
            AdmonitionType::Tip,
            AdmonitionType::Important,
            AdmonitionType::Warning,
            AdmonitionType::Caution,
        ];
        let mut names: Vec<&str> = all.iter().map(|t| t.name()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "类型名唯一（CSS 类名不冲突）");
        let mut colors: Vec<[u8; 3]> = all.iter().map(|t| t.color()).collect();
        colors.sort_unstable();
        colors.dedup();
        assert_eq!(colors.len(), n, "五种强调色唯一");
        assert!(all
            .iter()
            .all(|t| !t.icon().is_empty() && !t.label().is_empty()));
    }

    #[test]
    fn custom_callout_survives_incremental_reparse() {
        let mut b = Buffer::new("> [!FIXME] x\n");
        let mut s = Syntax::new_with_callouts(&b, &registry());
        // 在正文末尾追加字符（同块内编辑，首行标记不变）
        let at = b.len() - 2;
        b.edit(at, at, "!");
        s.reparse(&b, 0, 1, 1);
        assert!(
            matches!(&s.blocks()[0].kind, BlockKind::Callout(c) if c.name == "fixme"),
            "重解析后仍为自定义 Callout"
        );
    }

    // ===== G7：容器内 Mermaid 围栏提升 + 源码前缀剥离 =====

    /// 取文档中唯一的 Mermaid 块源码正文（`fenced_code_source` 的 body）。
    fn mermaid_body(b: &Buffer, s: &Syntax) -> String {
        let block = s
            .blocks()
            .iter()
            .find(|blk| blk.kind == BlockKind::Mermaid)
            .expect("应提升出独立 Mermaid 块");
        s.fenced_code_source(b, block.id).expect("围栏源码").1
    }

    #[test]
    fn quote_mermaid_source_strips_quote_layers() {
        let b = Buffer::new("> ```mermaid\n> flowchart TD\n>     A[x] --> B[y]\n> ```\n");
        let s = Syntax::new(&b);
        assert_eq!(mermaid_body(&b, &s), "flowchart TD\n    A[x] --> B[y]");
    }

    #[test]
    fn nested_quote_mermaid_source_strips_all_layers() {
        let b = Buffer::new("> > ```mermaid\n> > flowchart TD\n> >     A[x] --> B[y]\n> > ```\n");
        let s = Syntax::new(&b);
        assert_eq!(mermaid_body(&b, &s), "flowchart TD\n    A[x] --> B[y]");
    }

    #[test]
    fn list_mermaid_source_strips_only_container_indent() {
        let b = Buffer::new("- 前文\n\n  ```mermaid\n  flowchart TD\n      A[x] --> B[y]\n  ```\n");
        let s = Syntax::new(&b);
        assert_eq!(mermaid_body(&b, &s), "flowchart TD\n    A[x] --> B[y]");
    }

    #[test]
    fn nested_quote_list_mermaid_source_strips_both_prefixes() {
        let b = Buffer::new(
            "> - 项目\n>   ```mermaid\n>   flowchart TD\n>     A[x] --> B[y]\n>   ```\n",
        );
        let s = Syntax::new(&b);
        assert_eq!(mermaid_body(&b, &s), "flowchart TD\n  A[x] --> B[y]");
    }

    #[test]
    fn admonition_mermaid_source_strips_quote_prefix() {
        let b = Buffer::new(
            "> [!NOTE] 标题\n> ```mermaid\n> flowchart TD\n>     A[x] --> B[y]\n> ```\n",
        );
        let s = Syntax::new(&b);
        assert_eq!(mermaid_body(&b, &s), "flowchart TD\n    A[x] --> B[y]");
    }

    #[test]
    fn callout_mermaid_source_and_block_kind() {
        let b = Buffer::new(
            "> [!FIXME] 标题\n> ```mermaid\n> flowchart TD\n>     A[x] --> B[y]\n> ```\n",
        );
        let s = Syntax::new_with_callouts(&b, &registry());
        let kinds: Vec<BlockKind> = s.blocks().iter().map(|blk| blk.kind.clone()).collect();
        assert!(matches!(kinds[0], BlockKind::Callout(_)), "{kinds:?}");
        assert_eq!(kinds[1], BlockKind::Mermaid);
        assert_eq!(mermaid_body(&b, &s), "flowchart TD\n    A[x] --> B[y]");
        assert_eq!(s.blocks()[1].container.quote_depth, 1);
    }

    #[test]
    fn details_mermaid_source_keeps_relative_indent() {
        let b = Buffer::new(
            "<details>\n<summary>S</summary>\n\n```mermaid\nflowchart TD\n    A[x] --> B[y]\n```\n\n</details>\n",
        );
        let s = Syntax::new(&b);
        assert!(matches!(s.blocks()[0].kind, BlockKind::Details { .. }));
        assert_eq!(mermaid_body(&b, &s), "flowchart TD\n    A[x] --> B[y]");
    }

    #[test]
    fn top_level_fenced_code_source_is_unchanged() {
        // 顶层围栏代码块的现有行为不得被容器前缀剥离改变。
        let b = Buffer::new("```mermaid\nflowchart TD\n    A[x] --> B[y]\n```\n");
        let s = Syntax::new(&b);
        assert_eq!(mermaid_body(&b, &s), "flowchart TD\n    A[x] --> B[y]");
        let b = Buffer::new("```rust\n    fn f() {}\n```\n");
        let s = Syntax::new(&b);
        let block = &s.blocks()[0];
        assert_eq!(
            s.fenced_code_source(&b, block.id).unwrap().1,
            "    fn f() {}"
        );
    }
}
