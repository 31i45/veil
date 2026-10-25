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
//! 语义取舍逐条列于下方「inline 解析」注释块并钉在测试上。

use crate::buffer::Buffer;
use crate::range::Range;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Paragraph,
    /// ATX 标题，级别 1–6。
    AtxHeading(u8),
    FencedCode,
    /// 展示数学块（`$$...$$`）。块内不解析 inline，TeX 源码经 `math_source` 提取。
    Math,
    /// GFM 表格（表头行 + 分隔行 + 至多若干表体行）。块内不解析 inline，
    /// 单元格文本/对齐经 `table` 提取（视图只读渲染，切片 9）。
    Table,
    /// 列表项（v0：单行项，续行不并入）。`ordered` 区分有序/无序；`depth` = 缩进字节 / 2
    /// （上限 8，v0 简化：GFM 的 4 空格制与续行属后续增量）。inline 仅解析标记后内容。
    ListItem {
        ordered: bool,
        depth: u8,
    },
    /// 引用块：连续以 `>` 开头的行合为一块（v0 不支持惰性续行）。inline 逐行解析标记后内容。
    BlockQuote,
    /// 分隔线：`---` / `***` / `___`（同字符 ≥ 3，其间可夹空白）。无 inline。
    ThematicBreak,
}

/// 表格列对齐（GFM 分隔行 `:--` / `--:` / `:-:` 语法）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CellAlign {
    #[default]
    None,
    Left,
    Center,
    Right,
}

/// 单元格派生数据：可见文本 + 内容区间（文档字节偏移，已 trim）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TableCell {
    /// 剥除 inline 标记后的可见文本（`**b**` → `b`），供只读渲染直接使用。
    pub text: String,
    /// 内容区间；网格点击以此起点进入表格源码态。
    pub range: Range,
}

/// 表格派生数据（不持久化、不进撤销栈，随块结构自动正确）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Table {
    /// 每列对齐，长度 = 列数。
    pub aligns: Vec<CellAlign>,
    pub header: Vec<TableCell>,
    /// 表体行：每行长度恒等于列数（缺格补空，多格截断，GFM 行为）。
    pub rows: Vec<Vec<TableCell>>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Block {
    pub id: BlockId,
    pub kind: BlockKind,
    /// 行区间 `[start_line, end_line)`。
    pub start_line: usize,
    pub end_line: usize,
}

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

// ===== inline 解析（v0 子集） =====
//
// 语义取舍（逐条钉在测试上；CommonMark flanking 全量规则属后续增量）：
// - 递归下降、最左匹配：开标记找最近闭标记成节点，嵌套经递归实现
//   （`[link **bold**](url)` 合法嵌套；`**a **b** c**` 解析为两个并列 strong，
//   与 CommonMark 的嵌套结果不同——已知分歧，钉在 adjacent_strongs_leftmost_match）。
// - 空内容不成节点（`****` 为纯文本）；未闭合开标记为纯文本。
// - Code/Math 内容原样不嵌套；Math 要求内容非空、首尾非空格、闭 `$` 后非数字
//   （pandoc 规则，避免 `$5 and $6` 误判为公式）。
// - Link 文本可嵌套除 Link 外的任意 inline；URL 扫至首个 `)`，不支持 title。
// - Image 语法同 Link，alt 允许为空；URL 可空（`![]()` 合法，视图侧无加载键则只显
//   alt）。Image 可出现在 Link 文本内（徽章写法），alt 内亦可嵌套 inline（切片 13）。
// - `\` 转义 v0 标记字符；递归深度硬上限 MAX_INLINE_DEPTH（B5：超深嵌套不得栈溢出）。

/// inline 解析递归深度硬上限。超限后开标记按纯文本处理；
/// 16 层远超正常写作嵌套，仅作恶意语料保险丝。
const MAX_INLINE_DEPTH: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InlineKind {
    /// `**strong**`
    Strong,
    /// `*emph*`
    Emph,
    /// `` `code` ``，内容原样不嵌套
    Code,
    /// `[text](url)`
    Link,
    /// `![alt](url)`：`content` 为 alt 文本（可为空），`url` 为图片路径/URL。
    Image,
    /// `$math$`，内容原样不嵌套
    Math,
}

/// 一棵 inline 子树。所有区间为文档字节偏移、半开 `[start, end)`。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InlineNode {
    pub kind: InlineKind,
    /// 含开闭标记的整个节点区间。
    pub range: Range,
    /// 内容区间（不含标记；Link 为可见文本）。
    pub content: Range,
    /// 链接 URL 区间（仅 Link 为 Some）。
    pub url: Option<Range>,
    /// 嵌套子节点（Code/Math 恒为空）。
    pub children: Vec<InlineNode>,
}

/// 解析一段文本的 inline 节点，偏移以 `base` 为基准（文档字节偏移）。
fn parse_inlines(text: &str, base: usize) -> Vec<InlineNode> {
    InlineParser {
        s: text,
        base,
        pos: 0,
    }
    .parse_until(None, true, 0)
}

/// v0 可转义字符：inline 标记字符 + 反斜杠自身（`!` 为 Image 开标记首字符）。
fn is_escapable(c: char) -> bool {
    matches!(c, '\\' | '*' | '$' | '[' | ']' | '(' | ')' | '`' | '!')
}

struct InlineParser<'a> {
    s: &'a str,
    base: usize,
    pos: usize,
}

impl<'a> InlineParser<'a> {
    /// 解析直到 `stop` 分隔符（留在原位不消费）或 EOF。
    fn parse_until(
        &mut self,
        stop: Option<&'static str>,
        allow_link: bool,
        depth: usize,
    ) -> Vec<InlineNode> {
        let mut out = Vec::new();
        while self.pos < self.s.len() {
            let rest = &self.s[self.pos..];
            if stop.is_some_and(|st| rest.starts_with(st)) {
                break;
            }
            if let Some(after) = rest.strip_prefix('\\') {
                // 转义标记字符则两字符皆为纯文本；否则 `\` 自身为普通字符
                match after.chars().next() {
                    Some(c) if is_escapable(c) => self.pos += 1 + c.len_utf8(),
                    _ => self.pos += 1,
                }
                continue;
            }
            if depth < MAX_INLINE_DEPTH {
                let node = self
                    .try_code_span()
                    .or_else(|| self.try_math())
                    .or_else(|| self.try_strong(depth))
                    .or_else(|| self.try_emph(depth))
                    .or_else(|| self.try_image(depth))
                    .or_else(|| {
                        if allow_link {
                            self.try_link(depth)
                        } else {
                            None
                        }
                    });
                if let Some(node) = node {
                    out.push(node);
                    continue;
                }
            }
            // 普通字符：按 UTF-8 字符边界前进，保证不慢于线性、不切碎多字节字符
            self.pos += rest.chars().next().unwrap().len_utf8();
        }
        out
    }

    fn node(
        &self,
        kind: InlineKind,
        start: usize,
        content_start: usize,
        content_end: usize,
        url: Option<(usize, usize)>,
        children: Vec<InlineNode>,
    ) -> InlineNode {
        InlineNode {
            kind,
            range: Range::new(self.base + start, self.base + self.pos),
            content: Range::new(self.base + content_start, self.base + content_end),
            url: url.map(|(s, e)| Range::new(self.base + s, self.base + e)),
            children,
        }
    }

    /// `` `code` ``：下一个反引号闭合；内容非空、原样（不嵌套）。
    fn try_code_span(&mut self) -> Option<InlineNode> {
        let rest = &self.s[self.pos..];
        if !rest.starts_with('`') {
            return None;
        }
        let rel = rest[1..].find('`')?;
        if rel == 0 {
            return None;
        }
        let start = self.pos;
        let content_end = start + 1 + rel;
        self.pos = content_end + 1;
        Some(self.node(
            InlineKind::Code,
            start,
            start + 1,
            content_end,
            None,
            Vec::new(),
        ))
    }

    /// `$math$`：内容非空、首尾非空格、闭 `$` 后不随数字（pandoc 规则）。
    fn try_math(&mut self) -> Option<InlineNode> {
        let rest = &self.s[self.pos..];
        if !rest.starts_with('$') {
            return None;
        }
        let inner = &rest[1..];
        let rel = inner.find('$')?;
        let content = &inner[..rel];
        let ok = !content.is_empty()
            && !content.starts_with(char::is_whitespace)
            && !content.ends_with(char::is_whitespace)
            && !inner[rel + 1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit());
        if !ok {
            return None;
        }
        let start = self.pos;
        let content_end = start + 1 + rel;
        self.pos = content_end + 1;
        Some(self.node(
            InlineKind::Math,
            start,
            start + 1,
            content_end,
            None,
            Vec::new(),
        ))
    }

    /// `**strong**`：最近闭标记成节点，内容非空，子节点递归（最左匹配）。
    fn try_strong(&mut self, depth: usize) -> Option<InlineNode> {
        if !self.s[self.pos..].starts_with("**") {
            return None;
        }
        let start = self.pos;
        self.pos = start + 2;
        let children = self.parse_until(Some("**"), true, depth + 1);
        let content_end = self.pos;
        if !self.s[content_end..].starts_with("**") || content_end == start + 2 {
            self.pos = start; // 未闭合或空内容：还原，开标记按纯文本
            return None;
        }
        self.pos = content_end + 2;
        Some(self.node(
            InlineKind::Strong,
            start,
            start + 2,
            content_end,
            None,
            children,
        ))
    }

    /// `*emph*`：`**` 开头归 strong（调用顺序保证）；闭标记为单个 `*`。
    fn try_emph(&mut self, depth: usize) -> Option<InlineNode> {
        let rest = &self.s[self.pos..];
        if !rest.starts_with('*') || rest.starts_with("**") {
            return None;
        }
        let start = self.pos;
        self.pos = start + 1;
        let children = self.parse_until(Some("*"), true, depth + 1);
        let content_end = self.pos;
        if !self.s[content_end..].starts_with('*') || content_end == start + 1 {
            self.pos = start;
            return None;
        }
        self.pos = content_end + 1;
        Some(self.node(
            InlineKind::Emph,
            start,
            start + 1,
            content_end,
            None,
            children,
        ))
    }

    /// `[text](url)`：文本可嵌套除 Link 外的任意 inline；URL 扫至首个 `)`。
    fn try_link(&mut self, depth: usize) -> Option<InlineNode> {
        if !self.s[self.pos..].starts_with('[') {
            return None;
        }
        let start = self.pos;
        self.pos = start + 1;
        let children = self.parse_until(Some("]("), false, depth + 1);
        let text_end = self.pos;
        if !self.s[text_end..].starts_with("](") {
            self.pos = start;
            return None;
        }
        let url_start = text_end + 2;
        let rel = match self.s[url_start..].find(')') {
            Some(r) => r,
            None => {
                self.pos = start;
                return None;
            }
        };
        self.pos = url_start + rel + 1;
        Some(self.node(
            InlineKind::Link,
            start,
            start + 1,
            text_end,
            Some((url_start, url_start + rel)),
            children,
        ))
    }

    /// `![alt](url)`：alt 可空、可嵌套 inline；URL 扫至首个 `)`（可空）。
    /// 未闭合或缺 `](` 按纯文本处理。
    fn try_image(&mut self, depth: usize) -> Option<InlineNode> {
        if !self.s[self.pos..].starts_with("![") {
            return None;
        }
        let start = self.pos;
        self.pos = start + 2;
        let children = self.parse_until(Some("]("), false, depth + 1);
        let alt_end = self.pos;
        if !self.s[alt_end..].starts_with("](") {
            self.pos = start;
            return None;
        }
        let url_start = alt_end + 2;
        let rel = match self.s[url_start..].find(')') {
            Some(r) => r,
            None => {
                self.pos = start;
                return None;
            }
        };
        self.pos = url_start + rel + 1;
        Some(self.node(
            InlineKind::Image,
            start,
            start + 2,
            alt_end,
            Some((url_start, url_start + rel)),
            children,
        ))
    }
}

/// 扫描 `[from_line, total)` 内的全部块（跳过空行）。
fn scan_blocks(buf: &Buffer, from_line: usize, total: usize) -> Vec<(BlockKind, usize, usize)> {
    let mut out = Vec::new();
    let mut line = from_line;
    while line < total {
        if line_text(buf, line).trim().is_empty() {
            line += 1;
            continue;
        }
        let (kind, consumed) = scan_one(buf, line, total);
        out.push((kind, line, line + consumed));
        line += consumed;
    }
    out
}

/// 扫描单个块，返回 (种类, 消耗行数)。
fn scan_one(buf: &Buffer, line: usize, total: usize) -> (BlockKind, usize) {
    let text = line_text(buf, line);
    let t = text.trim_start();
    if let Some(level) = atx_level(t) {
        return (BlockKind::AtxHeading(level), 1);
    }
    if is_fence(t) {
        let mut n = 1;
        while line + n < total {
            if is_fence(line_text(buf, line + n).trim_start()) {
                return (BlockKind::FencedCode, n + 1);
            }
            n += 1;
        }
        return (BlockKind::FencedCode, n); // 未闭合围栏吞咽至 EOF（GFM 行为）
    }
    match math_marker(text.trim()) {
        Some(true) => return (BlockKind::Math, 1), // 单行 `$$x$$`
        Some(false) => {
            let mut n = 1;
            while line + n < total {
                if line_text(buf, line + n).trim() == "$$" {
                    return (BlockKind::Math, n + 1);
                }
                n += 1;
            }
            return (BlockKind::Math, n); // 未闭合吞咽至 EOF（与围栏一致）
        }
        None => {}
    }
    // 分隔线先于列表项判定：`---` 不是列表项（列表标记后须为空白或行尾）。
    // setext 标题（段落下一行 `---`）v0 不支持，按分隔线处理——已知分歧。
    if is_thematic_break(t) {
        return (BlockKind::ThematicBreak, 1);
    }
    if t.starts_with('>') {
        return (BlockKind::BlockQuote, quote_lines(buf, line, total));
    }
    if let Some(m) = list_mark(&text) {
        return (
            BlockKind::ListItem {
                ordered: m.ordered,
                depth: m.depth,
            },
            1,
        );
    }
    if let Some(n) = table_rows(buf, line, total) {
        return (BlockKind::Table, n);
    }
    // 段落：直到空行 / 围栏 / ATX / 数学标记 / 分隔线 / 引用 / 列表项打断
    // （均可直接打断段落；表格 v0 不打断段落——同 GFM，表头须位于块起始）
    let mut n = 1;
    while line + n < total {
        let l = line_text(buf, line + n);
        let lt = l.trim_start();
        if lt.trim().is_empty()
            || is_fence(lt)
            || atx_level(lt).is_some()
            || math_marker(l.trim()).is_some()
            || is_thematic_break(lt)
            || lt.starts_with('>')
            || list_mark(&l).is_some()
        {
            break;
        }
        n += 1;
    }
    (BlockKind::Paragraph, n)
}

fn line_text(buf: &Buffer, line: usize) -> String {
    let start = buf.offset_of_line(line);
    let end = if line + 1 < buf.line_count() {
        buf.offset_of_line(line + 1)
    } else {
        buf.len()
    };
    buf.slice(start, end)
}

fn atx_level(t: &str) -> Option<u8> {
    let hashes = t.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    match t.as_bytes().get(hashes) {
        None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') => Some(hashes as u8),
        _ => None,
    }
}

/// v0 仅识别 ``` 围栏；~~~ 与 info string 属于 GFM 全量子集，后续增量补。
fn is_fence(t: &str) -> bool {
    t.starts_with("```")
}

/// v0 数学块标记（pandoc 风格 `$$`）：`$$` 独占行 = 开/闭标记（Some(false)）；
/// `$$x$$` 单行自足（Some(true)，要求内容非空即标记长 > 4）。
/// `$$x`（开标记后随同行内容且不闭合）不算标记——独占行约束收紧误判面。
fn math_marker(t: &str) -> Option<bool> {
    if !t.starts_with("$$") {
        return None;
    }
    if t == "$$" {
        return Some(false);
    }
    if t.len() > 4 && t.ends_with("$$") {
        return Some(true);
    }
    None
}

// ===== 列表项 / 引用块 / 分隔线（切片 12） =====
//
// v0 取舍（逐条钉在测试上）：
// - 列表项单行成块；标记后须为空白或行尾（`-tag` 非列表，`---` 归分隔线）。
//   有序标记为 1–9 位数字 + `.`/`)`；depth = 缩进字节 / 2 上限 8，续行不入项。
// - 引用块：连续 `>` 行合为一块，空行终止；不支持惰性续行与嵌套渲染（v0）。
// - 分隔线：`-` / `*` / `_` 同字符 ≥ 3，其间只允许空白；不认 setext 标题。

/// 列表标记几何（缩进字节、有序性、内容起点）。
struct ListMark {
    ordered: bool,
    depth: u8,
    content_start: usize,
}

/// 识别行首列表标记；非列表返回 None。`text` 可为整行（含行尾换行）。
fn list_mark(text: &str) -> Option<ListMark> {
    let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
    let b = text.as_bytes();
    let (ordered, marker_len) = match b.get(indent)? {
        b'-' | b'*' | b'+' => (false, 1),
        d if d.is_ascii_digit() => {
            let digits = b[indent..]
                .iter()
                .take_while(|x| x.is_ascii_digit())
                .count();
            if digits > 9 {
                return None;
            }
            match b.get(indent + digits) {
                Some(b'.') | Some(b')') => (true, digits + 1),
                _ => return None,
            }
        }
        _ => return None,
    };
    let after = indent + marker_len;
    let content_start = match b.get(after) {
        None | Some(b'\n') | Some(b'\r') => after, // 行尾：空项
        Some(b' ') | Some(b'\t') => after + 1,
        _ => return None, // 标记后非空白：`-tag` / `1.x`
    };
    Some(ListMark {
        ordered,
        depth: (indent / 2).min(8) as u8,
        content_start,
    })
}

/// 引用行首 `>` 的位置（缩进之后）；非引用行返回 None。
fn quote_marker_pos(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    (line.as_bytes().get(indent) == Some(&b'>')).then_some(indent)
}

/// 引用块行数：连续 `>` 行，空行终止（v0 无惰性续行）。
fn quote_lines(buf: &Buffer, line: usize, total: usize) -> usize {
    let mut n = 1;
    while line + n < total {
        let l = line_text(buf, line + n);
        if l.trim().is_empty() || !l.trim_start().starts_with('>') {
            break;
        }
        n += 1;
    }
    n
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

/// 分隔线：同字符 `-` / `*` / `_` ≥ 3，其间只允许空白。
fn is_thematic_break(t: &str) -> bool {
    let s = t.trim_end();
    let mut chars = s.chars();
    let Some(marker) = chars.next() else {
        return false;
    };
    if !matches!(marker, '-' | '*' | '_') {
        return false;
    }
    let mut count = 1;
    for c in chars {
        if c == marker {
            count += 1;
        } else if c != ' ' && c != '\t' {
            return false;
        }
    }
    count >= 3
}

// ===== GFM 表格（切片 9） =====
//
// v0 取舍（逐条钉在测试上）：
// - 只在块起始识别（不打断段落，与 GFM 一致）；表头/分隔行均须含未转义 `|`，
//   列数一致，分隔格为 `:?-+:?`（至少一个连字符）。
// - 表体行：非空且含未转义 `|`，遇空行/无竖线行/围栏/ATX/数学标记即止（GFM 的
//   「另一个块级结构打断表格」v0 仅覆盖本内核已识别的块种类）。
// - 行切分尊重 `\|` 转义；单元格 trim 后取内容区间；表体缺格补空、多格截断。
// - 单元格可见文本剥除 inline 标记（`**b**` → `b`），只读渲染用；样式暂不下发。

/// 扫描表格块消耗行数（表头 + 分隔 + 表体），非表格返回 None。
fn table_rows(buf: &Buffer, line: usize, total: usize) -> Option<usize> {
    if line + 1 >= total {
        return None;
    }
    let head = line_text(buf, line);
    if !has_unescaped_pipe(&head) {
        return None;
    }
    let ncols = parse_delim_row(&line_text(buf, line + 1))?.len();
    if split_row(&head).len() != ncols {
        return None;
    }
    let mut n = 2;
    while line + n < total {
        let l = line_text(buf, line + n);
        let lt = l.trim_start();
        let interrupted = is_fence(lt)
            || atx_level(lt).is_some()
            || math_marker(l.trim()).is_some()
            || !has_unescaped_pipe(&l);
        if lt.trim().is_empty() || interrupted {
            break;
        }
        n += 1;
    }
    Some(n)
}

/// 分隔行 → 列对齐；非法（无竖线、空格内容、列数为零）返回 None。
fn parse_delim_row(line: &str) -> Option<Vec<CellAlign>> {
    if !has_unescaped_pipe(line) {
        return None;
    }
    let segs = split_row(line);
    let mut out = Vec::with_capacity(segs.len());
    for (s, e) in segs {
        out.push(delim_align(&line[s..e])?);
    }
    (!out.is_empty()).then_some(out)
}

/// 单个分隔格：`:?-+:?`。无连字符（含单个 `:`）非法。
fn delim_align(raw: &str) -> Option<CellAlign> {
    let t = raw.trim();
    let (left, right) = (t.starts_with(':'), t.ends_with(':'));
    let body = t.trim_start_matches(':').trim_end_matches(':');
    if body.is_empty() || !body.bytes().all(|b| b == b'-') {
        return None;
    }
    Some(match (left, right) {
        (true, true) => CellAlign::Center,
        (true, false) => CellAlign::Left,
        (false, true) => CellAlign::Right,
        (false, false) => CellAlign::None,
    })
}

/// 按未转义 `|` 切分表格行；剥除首尾空白与可选外层竖线。
/// 返回行内字节区间（含单元格两侧空白；trim 由调用方完成）。
fn split_row(line: &str) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let (mut s, mut e) = (0, b.len());
    while s < e && b[s].is_ascii_whitespace() {
        s += 1;
    }
    while e > s && b[e - 1].is_ascii_whitespace() {
        e -= 1;
    }
    if s < e && b[s] == b'|' {
        s += 1;
    }
    if e > s && b[e - 1] == b'|' && b[e - 2] != b'\\' {
        e -= 1;
    }
    let mut cells = Vec::new();
    let mut start = s;
    let mut i = s;
    while i < e {
        match b[i] {
            b'\\' => i += 2, // 转义字符不参与切分（`\|` 留在格内）
            b'|' => {
                cells.push((start, i));
                start = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    cells.push((start, e));
    cells
}

/// 行内是否有未转义竖线（UTF-8 下 `|` 只会以单字节出现，区间恒为字符边界）。
fn has_unescaped_pipe(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'|' => return true,
            _ => i += 1,
        }
    }
    false
}

/// 一行 → 恰好 `ncols` 个单元格（缺格补空、多格截断），`line_off` 为该行绝对起点。
fn parse_table_cells(line: &str, line_off: usize, ncols: usize) -> Vec<TableCell> {
    let segs = split_row(line);
    let line_end = line.trim_end().len();
    let mut out = Vec::with_capacity(ncols);
    for i in 0..ncols {
        let (s, e) = segs.get(i).copied().unwrap_or((line_end, line_end));
        let raw = &line[s..e];
        let lead = raw.len() - raw.trim_start().len();
        let cs = s + lead;
        let ce = cs + raw.trim().len();
        let content = &line[cs..ce];
        let nodes = parse_inlines(content, line_off + cs);
        let text = unescape_cell_pipes(&visible_text(content, &nodes, line_off + cs));
        out.push(TableCell {
            text,
            range: Range::new(line_off + cs, line_off + ce),
        });
    }
    out
}

/// inline 节点 → 可见文本：保留内容、剥除标记；Code/Math 内容原样。
fn visible_text(src: &str, nodes: &[InlineNode], base: usize) -> String {
    let mut out = String::with_capacity(src.len());
    push_visible(src, nodes, base, 0, src.len(), &mut out);
    out
}

fn push_visible(
    src: &str,
    nodes: &[InlineNode],
    base: usize,
    start: usize,
    end: usize,
    out: &mut String,
) {
    let mut p = start;
    for n in nodes {
        let ns = n.range.start.saturating_sub(base);
        let ne = n.range.end.saturating_sub(base);
        if ns < p || ne > end {
            continue; // 防御：节点须完整落在当前区间内
        }
        out.push_str(&src[p..ns]);
        let cs = n.content.start.saturating_sub(base);
        let ce = n.content.end.saturating_sub(base);
        match n.kind {
            InlineKind::Code | InlineKind::Math => out.push_str(&src[cs..ce]),
            InlineKind::Strong | InlineKind::Emph | InlineKind::Link | InlineKind::Image => {
                push_visible(src, &n.children, base, cs, ce, out);
            }
        }
        p = ne;
    }
    out.push_str(&src[p..end]);
}

/// GFM：表格单元格内 `\|` 表示字面竖线。仅解转义竖线（其余转义保留以兼容 inline 语义）。
fn unescape_cell_pipes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            chars.next();
            out.push('|');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(buf: &Buffer) -> Vec<(BlockKind, usize, usize)> {
        scan_blocks(buf, 0, buf.line_count())
    }

    #[test]
    fn parse_headings_and_paragraphs() {
        let b = Buffer::new("# T\n\npara\n\n## H2\n");
        assert_eq!(
            kinds(&b),
            vec![
                (BlockKind::AtxHeading(1), 0, 1),
                (BlockKind::Paragraph, 2, 3),
                (BlockKind::AtxHeading(2), 4, 5),
            ]
        );
    }

    #[test]
    fn unclosed_fence_swallows_to_eof() {
        let b = Buffer::new("a\n```\nb\nc\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), (BlockKind::FencedCode, 1, 5)]
        );
    }

    #[test]
    fn closed_fence_then_paragraph() {
        let b = Buffer::new("```\nx\n```\nb\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::FencedCode, 0, 3), (BlockKind::Paragraph, 3, 4)]
        );
    }

    #[test]
    fn atx_requires_space_or_eol() {
        assert_eq!(atx_level("#tag"), None); // GFM：`#tag` 不是标题
        assert_eq!(atx_level("# t"), Some(1));
        assert_eq!(atx_level("#"), Some(1));
        assert_eq!(atx_level("####### x"), None); // 7 个 # 超出 1–6
    }

    #[test]
    fn empty_doc_has_no_blocks() {
        assert!(kinds(&Buffer::new("")).is_empty());
    }

    #[test]
    fn atx_levels_one_to_six() {
        for n in 1..=6usize {
            let t = format!("{} h", "#".repeat(n));
            assert_eq!(atx_level(&t), Some(n as u8));
        }
    }

    #[test]
    fn paragraph_interrupted_by_fence_without_blank() {
        // GFM：围栏可直接打断段落，无需空行
        let b = Buffer::new("a\n```\nb\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), (BlockKind::FencedCode, 1, 4)]
        );
    }

    #[test]
    fn whitespace_only_line_is_blank() {
        let b = Buffer::new("a\n \nb\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), (BlockKind::Paragraph, 2, 3)]
        );
    }

    #[test]
    fn indented_atx_recognized() {
        // v0 简化：trim_start 后判定，不限缩进量（GFM 仅允许 ≤3 空格）
        let b = Buffer::new("  ## h\n");
        assert_eq!(kinds(&b), [(BlockKind::AtxHeading(2), 0, 1)]);
    }

    #[test]
    fn crlf_doc_block_structure() {
        // CRLF：\r 残留在行尾，trim 视为空白；块结构不受影响
        let b = Buffer::new("# T\r\n\r\npara\r\n");
        assert_eq!(
            kinds(&b),
            [
                (BlockKind::AtxHeading(1), 0, 1),
                (BlockKind::Paragraph, 2, 3)
            ]
        );
    }

    // ===== 列表项 / 引用块 / 分隔线（切片 12）：识别与边界 =====

    #[test]
    fn list_items_unordered_and_ordered() {
        // 缩进 0/2 → depth 0/1；有序标记保留种类
        let b = Buffer::new("- a\n  - b\n1. c\n  2) d\n");
        assert_eq!(
            kinds(&b),
            [
                (
                    BlockKind::ListItem {
                        ordered: false,
                        depth: 0
                    },
                    0,
                    1
                ),
                (
                    BlockKind::ListItem {
                        ordered: false,
                        depth: 1
                    },
                    1,
                    2
                ),
                (
                    BlockKind::ListItem {
                        ordered: true,
                        depth: 0
                    },
                    2,
                    3
                ),
                (
                    BlockKind::ListItem {
                        ordered: true,
                        depth: 1
                    },
                    3,
                    4
                ),
            ]
        );
    }

    #[test]
    fn list_marker_requires_blank_or_eol() {
        // GFM：`-tag` 与 `1.x` 均非列表；裸标记（空项）合法
        assert_eq!(
            kinds(&Buffer::new("-tag\n")),
            [(BlockKind::Paragraph, 0, 1)]
        );
        assert_eq!(kinds(&Buffer::new("1.x\n")), [(BlockKind::Paragraph, 0, 1)]);
        assert_eq!(
            kinds(&Buffer::new("-\n")),
            [(
                BlockKind::ListItem {
                    ordered: false,
                    depth: 0
                },
                0,
                1
            )]
        );
        // 10 位数字超出 v0 有序标记上限
        assert_eq!(
            kinds(&Buffer::new("1234567890. x\n")),
            [(BlockKind::Paragraph, 0, 1)]
        );
    }

    #[test]
    fn list_depth_caps_at_eight() {
        let b = Buffer::new(&format!("{}- x\n", " ".repeat(40)));
        assert_eq!(
            kinds(&b),
            [(
                BlockKind::ListItem {
                    ordered: false,
                    depth: 8
                },
                0,
                1
            )]
        );
    }

    #[test]
    fn list_interrupts_paragraph() {
        // GFM：列表项可打断段落（无需空行）
        let b = Buffer::new("a\n- b\n");
        assert_eq!(
            kinds(&b),
            [
                (BlockKind::Paragraph, 0, 1),
                (
                    BlockKind::ListItem {
                        ordered: false,
                        depth: 0
                    },
                    1,
                    2
                )
            ]
        );
    }

    #[test]
    fn blockquote_groups_consecutive_marker_lines() {
        let b = Buffer::new("> a\n> b\n\n> c\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::BlockQuote, 0, 2), (BlockKind::BlockQuote, 3, 4)]
        );
    }

    #[test]
    fn blockquote_requires_gt_and_interrupts_paragraph() {
        let b = Buffer::new("a\n> b\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), (BlockKind::BlockQuote, 1, 2)]
        );
        // 无 `>` 的惰性续行 v0 不支持：单独成段
        assert_eq!(
            kinds(&Buffer::new("> a\nb\n")),
            [(BlockKind::BlockQuote, 0, 1), (BlockKind::Paragraph, 1, 2)]
        );
    }

    #[test]
    fn thematic_break_variants() {
        for line in ["---", "***", "___", "- - -", "* * *", "_ _ _", "----"] {
            let b = Buffer::new(&format!("{line}\n"));
            assert_eq!(
                kinds(&b),
                [(BlockKind::ThematicBreak, 0, 1)],
                "line={line:?}"
            );
        }
        // 不足 3 或混入异字符 → 非分隔线
        for line in ["--", "-*-", "**", "__"] {
            let b = Buffer::new(&format!("{line}\n"));
            assert_ne!(
                kinds(&b),
                [(BlockKind::ThematicBreak, 0, 1)],
                "line={line:?}"
            );
        }
    }

    #[test]
    fn thematic_break_wins_over_list_and_interrupts_paragraph() {
        // `---` 不是列表项；且可打断段落（setext v0 不支持 → 已知分歧）
        let b = Buffer::new("a\n---\nb\n");
        assert_eq!(
            kinds(&b),
            [
                (BlockKind::Paragraph, 0, 1),
                (BlockKind::ThematicBreak, 1, 2),
                (BlockKind::Paragraph, 2, 3)
            ]
        );
    }

    // ===== inline 解析测试（v0 语义逐条钉住） =====

    fn parse(text: &str) -> Vec<InlineNode> {
        parse_inlines(text, 0)
    }

    /// 紧凑 S-表达式快照：种类 + 各区间 + 递归子节点，一图钉住整棵树。
    fn sexp(ns: &[InlineNode]) -> String {
        ns.iter()
            .map(|n| {
                let mut s = format!(
                    "{:?}[{}..{} c={}..{}",
                    n.kind, n.range.start, n.range.end, n.content.start, n.content.end
                );
                if let Some(u) = n.url {
                    s.push_str(&format!(" url={}..{}", u.start, u.end));
                }
                for c in &n.children {
                    s.push(' ');
                    s.push_str(&sexp(std::slice::from_ref(c)));
                }
                s.push(']');
                s
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn depth(ns: &[InlineNode]) -> usize {
        ns.iter().map(|n| 1 + depth(&n.children)).max().unwrap_or(0)
    }

    #[test]
    fn strong_basic() {
        assert_eq!(sexp(&parse("a **bold** c")), "Strong[2..10 c=4..8]");
    }

    #[test]
    fn emph_basic() {
        assert_eq!(sexp(&parse("*a*")), "Emph[0..3 c=1..2]");
    }

    #[test]
    fn code_is_literal_no_nesting() {
        assert_eq!(sexp(&parse("`**x**`")), "Code[0..7 c=1..6]");
    }

    #[test]
    fn math_basic() {
        assert_eq!(sexp(&parse("$x+y$")), "Math[0..5 c=1..4]");
    }

    #[test]
    fn math_rejects_price_and_inner_space() {
        assert_eq!(sexp(&parse("$5 and $6")), ""); // 价格：内容尾空格 + 闭 $ 后随数字
        assert_eq!(sexp(&parse("$x$5")), ""); // 闭 $ 后随数字
        assert_eq!(sexp(&parse("$ x$")), ""); // 首空格
        assert_eq!(sexp(&parse("$x $")), ""); // 尾空格
    }

    #[test]
    fn link_basic() {
        assert_eq!(
            sexp(&parse("[t](http://a)")),
            "Link[0..13 c=1..2 url=4..12]"
        );
    }

    #[test]
    fn link_with_strong_child() {
        // ADR-003 嵌套用例：链接文本内可含 strong
        assert_eq!(
            sexp(&parse("[link **bold**](url)")),
            "Link[0..20 c=1..14 url=16..19 Strong[6..14 c=8..12]]"
        );
    }

    #[test]
    fn strong_with_emph_child() {
        assert_eq!(
            sexp(&parse("**a *b* c**")),
            "Strong[0..11 c=2..9 Emph[4..7 c=5..6]]"
        );
    }

    #[test]
    fn adjacent_strongs_leftmost_match() {
        // v0 最左匹配：两个并列 strong；CommonMark 解析为嵌套——已知分歧
        assert_eq!(
            sexp(&parse("**a **b** c**")),
            "Strong[0..6 c=2..4] Strong[7..13 c=9..11]"
        );
    }

    #[test]
    fn unclosed_or_empty_markers_are_text() {
        assert_eq!(sexp(&parse("**a")), ""); // 未闭合
        assert_eq!(sexp(&parse("****")), ""); // 空内容
        assert_eq!(sexp(&parse("*")), "");
        assert_eq!(sexp(&parse("`")), "");
        assert_eq!(sexp(&parse("``")), ""); // 空代码
    }

    #[test]
    fn escaped_markers_are_text() {
        assert_eq!(sexp(&parse("\\*a*")), ""); // `\*a*`
        assert_eq!(sexp(&parse("\\\\**a**")), "Strong[2..7 c=4..5]"); // `\\` 转义反斜杠，其后 strong 仍成立
    }

    #[test]
    fn nesting_depth_capped_at_max() {
        // B5 保险丝：`[**` 交替制造 link↔strong 真嵌套（单层标记无法嵌套：
        // 闭标记检查优先，相邻 `**` 恒先被当作闭标记），80 层语料钳在 MAX，不栈溢出
        let doc = format!("{}a{}", "[**".repeat(40), "**](u)".repeat(40));
        let nodes = parse(&doc);
        assert_eq!(depth(&nodes), MAX_INLINE_DEPTH);
    }

    #[test]
    fn emph_closed_by_first_star_of_double() {
        // 闭标记只需一个 `*`：`*a**` 的 emph 吃掉 `**` 的第一半
        assert_eq!(sexp(&parse("*a**")), "Emph[0..3 c=1..2]");
    }

    #[test]
    fn links_do_not_nest() {
        // v0 外层贪婪：文本含 `[b`；CommonMark 内层优先——已知分歧
        assert_eq!(sexp(&parse("[a [b](u)](v)")), "Link[0..9 c=1..5 url=7..8]");
    }

    #[test]
    fn multibyte_offsets_are_bytes() {
        // `中文 ` 7 字节；粗 = 3 字节。一切区间为字节偏移
        assert_eq!(sexp(&parse("中文 **粗**")), "Strong[7..14 c=9..12]");
    }

    #[test]
    fn inline_crosses_soft_break() {
        // 段落内软换行可穿过 inline 内容
        assert_eq!(sexp(&parse("a *b\nc* d")), "Emph[2..7 c=3..6]");
    }

    #[test]
    fn two_code_spans_in_one_line() {
        assert_eq!(
            sexp(&parse("`a` `b`")),
            "Code[0..3 c=1..2] Code[4..7 c=5..6]"
        );
    }

    #[test]
    fn two_math_nodes_in_one_line() {
        assert_eq!(
            sexp(&parse("$a$ b $c$")),
            "Math[0..3 c=1..2] Math[6..9 c=7..8]"
        );
    }

    #[test]
    fn link_url_stops_at_first_paren() {
        // v0：URL 扫至首个 `)`，不支持括号转义/嵌套
        assert_eq!(sexp(&parse("[t](a(b)c)")), "Link[0..8 c=1..2 url=4..7]");
    }

    // ===== Image（切片 13）：`![alt](url)` =====

    #[test]
    fn image_basic() {
        // 逐字节：!=0 [=1 a=2 ]=3 (=4 p=5 . . n g = 5..10 )=10
        assert_eq!(sexp(&parse("![a](p.png)")), "Image[0..11 c=2..3 url=5..10]");
    }

    #[test]
    fn image_empty_alt_allowed() {
        // ![]()：空 alt 与空 URL 均合法（CommonMark 图片语法）；视图侧无加载键时显 alt
        assert_eq!(sexp(&parse("![](u)")), "Image[0..6 c=2..2 url=4..5]");
        assert_eq!(sexp(&parse("![]()")), "Image[0..5 c=2..2 url=4..4]");
    }

    #[test]
    fn image_unclosed_or_missing_url_is_text() {
        assert_eq!(sexp(&parse("![a](u")), ""); // 未闭合 `)`
        assert_eq!(sexp(&parse("![a]")), ""); // 缺 `](`
                                              // 空 URL 仍是 Image（不是退化成 Link 的 `[a]()`）
        assert_eq!(sexp(&parse("![a]()")), "Image[0..6 c=2..3 url=5..5]");
    }

    #[test]
    fn image_alt_may_nest_inline() {
        // alt 按 inline 解析：`![**b**](u)` 的 alt 内为 Strong
        assert_eq!(
            sexp(&parse("![**b**](u)")),
            "Image[0..11 c=2..7 url=9..10 Strong[2..7 c=4..5]]"
        );
    }

    #[test]
    fn image_inside_link_text() {
        // 徽章写法：Link 文本内嵌 Image，两者区间各自独立
        assert_eq!(
            sexp(&parse("[![a](i)](u)")),
            "Link[0..12 c=1..8 url=10..11 Image[1..8 c=3..4 url=6..7]]"
        );
    }

    #[test]
    fn escaped_bang_is_literal() {
        // `\!` 转义后余下 `[a](u)` 按普通 Link 解析
        assert_eq!(sexp(&parse("\\![a](u)")), "Link[2..8 c=3..4 url=6..7]");
    }

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

    // ===== 数学块（切片 6，pandoc 风格 `$$`）：独占行开闭 + 单行 `$$x$$` =====

    #[test]
    fn math_block_multiline() {
        let b = Buffer::new("$$\nx+y\n$$\n");
        assert_eq!(kinds(&b), [(BlockKind::Math, 0, 3)]);
    }

    #[test]
    fn math_block_single_line() {
        let b = Buffer::new("$$x+y$$\n");
        assert_eq!(kinds(&b), [(BlockKind::Math, 0, 1)]);
    }

    #[test]
    fn math_block_unclosed_swallows_to_eof() {
        // 与围栏一致的吞咽行为
        let b = Buffer::new("a\n$$\nx\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), (BlockKind::Math, 1, 4)]
        );
    }

    #[test]
    fn math_block_interrupts_paragraph_without_blank() {
        // 与围栏/ATX 一致：`$$` 标记行可直接打断段落
        let b = Buffer::new("a\n$$\nx\n$$\nb\n");
        assert_eq!(
            kinds(&b),
            [
                (BlockKind::Paragraph, 0, 1),
                (BlockKind::Math, 1, 4),
                (BlockKind::Paragraph, 4, 5)
            ]
        );
    }

    #[test]
    fn math_single_line_interrupts_paragraph() {
        let b = Buffer::new("a\n$$x$$\nb\n");
        assert_eq!(
            kinds(&b),
            [
                (BlockKind::Paragraph, 0, 1),
                (BlockKind::Math, 1, 2),
                (BlockKind::Paragraph, 2, 3)
            ]
        );
    }

    #[test]
    fn math_marker_edge_cases() {
        // `$$$$`：单行形态要求内容非空（标记长 > 4），否则按段落
        let b = Buffer::new("$$$$\n");
        assert_eq!(kinds(&b), [(BlockKind::Paragraph, 0, 1)]);
        // `$$x`：开标记非独占行，v0 不认；其后 `$$` 独占行开块并吞至 EOF
        let b = Buffer::new("$$x\n$$\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), (BlockKind::Math, 1, 3)]
        );
    }

    #[test]
    fn fence_swallows_math_markers() {
        // 围栏优先级高于数学块：`$$` 行在围栏内只是代码内容
        let b = Buffer::new("```\n$$\n```\n");
        assert_eq!(kinds(&b), [(BlockKind::FencedCode, 0, 3)]);
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

    // ===== GFM 表格（切片 9）：块识别边界 =====

    #[test]
    fn table_basic_header_delimiter_body() {
        let b = Buffer::new("| a | b |\n| :- | -: |\n| 1 | 2 |\n3 | 4\n");
        assert_eq!(kinds(&b), [(BlockKind::Table, 0, 4)]);
    }

    #[test]
    fn table_requires_delimiter_row_and_matching_columns() {
        // 无分隔行 → 段落
        assert_eq!(
            kinds(&Buffer::new("| a | b |\n| 1 | 2 |\n")),
            [(BlockKind::Paragraph, 0, 2)]
        );
        // 列数不一致 → 非表格
        assert_eq!(
            kinds(&Buffer::new("| a | b |\n| --- |\n")),
            [(BlockKind::Paragraph, 0, 2)]
        );
        // 分隔格非法（无连字符）→ 非表格
        assert_eq!(
            kinds(&Buffer::new("| a |\n| x |\n")),
            [(BlockKind::Paragraph, 0, 2)]
        );
        // 仅表头（无分隔行）→ 段落
        assert_eq!(
            kinds(&Buffer::new("| a | b |\n")),
            [(BlockKind::Paragraph, 0, 1)]
        );
    }

    #[test]
    fn table_outer_pipes_optional_and_single_column() {
        // 无外层竖线：GFM 允许
        let b = Buffer::new("a | b\n--- | ---\n1 | 2\n");
        assert_eq!(kinds(&b), [(BlockKind::Table, 0, 3)]);
        // 单列表格
        let b = Buffer::new("| a |\n| --- |\n| b |\n");
        assert_eq!(kinds(&b), [(BlockKind::Table, 0, 3)]);
    }

    #[test]
    fn table_body_stops_at_blank_no_pipe_or_block_start() {
        // 空行终止表格；其后新表头缺分隔行 → 段落
        let b = Buffer::new("| a |\n| --- |\n| b |\n\n| c |\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Table, 0, 3), (BlockKind::Paragraph, 4, 5)]
        );
        // 无竖线的非空行终止表格（GFM 例 200）
        let b = Buffer::new("| a |\n| --- |\n| b |\ntail\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Table, 0, 3), (BlockKind::Paragraph, 3, 4)]
        );
        // 下一块起始（ATX）打断表格
        let b = Buffer::new("| a |\n| --- |\n| b |\n## h | x\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Table, 0, 3), (BlockKind::AtxHeading(2), 3, 4)]
        );
    }

    #[test]
    fn table_does_not_interrupt_paragraph() {
        // GFM：表头须位于块起始；段落后的表格行被并入段落
        let b = Buffer::new("para\n| a |\n| --- |\n");
        assert_eq!(kinds(&b), [(BlockKind::Paragraph, 0, 3)]);
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
