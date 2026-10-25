//! 块级扫描：ATX 标题 / 围栏代码块 / 数学块 / GFM 表格 / 列表项 / 引用块 /
//! 分隔线 / 段落（GFM 其余块留待后续增量）。

use super::inline::parse_inlines;
use super::model::{BlockKind, CellAlign, InlineKind, InlineNode, TableCell};
use crate::buffer::Buffer;
use crate::range::Range;

/// 扫描 `[from_line, total)` 内的全部块（跳过空行）。
pub(super) fn scan_blocks(
    buf: &Buffer,
    from_line: usize,
    total: usize,
) -> Vec<(BlockKind, usize, usize)> {
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
pub(super) fn scan_one(buf: &Buffer, line: usize, total: usize) -> (BlockKind, usize) {
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

pub(super) fn line_text(buf: &Buffer, line: usize) -> String {
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
pub(super) struct ListMark {
    pub ordered: bool,
    pub depth: u8,
    pub content_start: usize,
}

/// 识别行首列表标记；非列表返回 None。`text` 可为整行（含行尾换行）。
pub(super) fn list_mark(text: &str) -> Option<ListMark> {
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
pub(super) fn quote_marker_pos(line: &str) -> Option<usize> {
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
pub(super) fn parse_delim_row(line: &str) -> Option<Vec<CellAlign>> {
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
pub(super) fn parse_table_cells(line: &str, line_off: usize, ncols: usize) -> Vec<TableCell> {
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
}
