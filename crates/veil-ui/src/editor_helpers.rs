use crate::editor::Editor;
use veil_core::{emphasis_spans, InlineKind, Range};

impl Editor {
    /// 选区边界落在 marker 串内部时外扩到整串：绝不把 `**` 切出半个 `*` 当成
    /// 完整标记（真机「粗体中部分字加斜 → 整行异常粗体与星号」的根因）。
    pub(crate) fn expand_over_runs(
        text: &str,
        mut s: usize,
        mut e: usize,
        ch: u8,
    ) -> (usize, usize) {
        let b = text.as_bytes();
        if s < b.len() && b[s] == ch {
            while s > 0 && b[s - 1] == ch {
                s -= 1;
            }
        }
        if e > 0 && e <= b.len() && b[e - 1] == ch {
            while e < b.len() && b[e] == ch {
                e += 1;
            }
        }
        (s, e)
    }

    /// 从 `at` 向右的连续 marker 字符数（`text[at]` 不是 marker 时为 0）。
    pub(crate) fn run_right(text: &str, at: usize, ch: u8) -> usize {
        let b = text.as_bytes();
        let mut i = at;
        while i < b.len() && b[i] == ch {
            i += 1;
        }
        i - at
    }

    /// 从 `at` 向左的连续 marker 字符数（`text[at-1]` 不是 marker 时为 0）。
    pub(crate) fn run_left(text: &str, at: usize, ch: u8) -> usize {
        let b = text.as_bytes();
        let mut i = at;
        while i > 0 && b[i - 1] == ch {
            i -= 1;
        }
        at - i
    }

    /// 选区恰好压在一个强调结构上时，返回 `(含标记起点, 内容起点, 内容终点, 含标记终点)`
    /// （文档偏移）。两种命中：
    /// 1. 选区 = 结构内容段（标记贴在选区外侧）；
    /// 2. 结构的开/闭标记段整体落在选区首/尾的 marker 串内（选整个 `***b***`
    ///    再按 Ctrl+B，摘掉内层 `**`，而不是再包一层）。
    ///
    /// 只认单行选区（调用方保证）；结构裁决交给解析器，`*a* *b*` 中间空白这类
    /// 「两侧都有星号但其实不是同一对」的形态因此不会被误判为已包裹。
    pub(crate) fn span_at_selection(
        doc: &str,
        s: usize,
        e: usize,
        kind: InlineKind,
        ch: u8,
    ) -> Option<(usize, usize, usize, usize)> {
        let line_start = doc[..s].rfind('\n').map_or(0, |i| i + 1);
        let line_end = doc[s..].find('\n').map_or(doc.len(), |i| s + i);
        let line = &doc[line_start..line_end];
        let lead = Self::run_right(line, s - line_start, ch);
        let trail = Self::run_left(line, e - line_start, ch);
        for sp in emphasis_spans(line) {
            if sp.kind != kind || !line[sp.open.start..sp.open.end].bytes().all(|b| b == ch) {
                continue;
            }
            let (rs, cs, ce, re) = (
                sp.open.start + line_start,
                sp.content.start + line_start,
                sp.content.end + line_start,
                sp.close.end + line_start,
            );
            if cs == s && ce == e {
                return Some((rs, cs, ce, re));
            }
            if rs >= s && cs <= s + lead && re <= e && ce >= e - trail {
                return Some((rs, cs, ce, re));
            }
        }
        None
    }

    /// 在 caret 周围查找 `[text](url)` inline link 结构，返回 `(lbracket_start, close_paren_end)`
    /// 即整个 link 的半开字节范围。只认 inline link，不认 ref link `[text][ref]`。
    /// 简化：不支持嵌套 `[]` 或含 `]` 的 URL（返回 None 则退化为插新骨架）。
    pub(crate) fn find_link_pair(text: &str, caret: usize) -> Option<(usize, usize)> {
        // 向左扫找最近的 `[`
        for open_bracket in (0..caret).rev() {
            if text.as_bytes()[open_bracket] != b'[' {
                continue;
            }
            // 跳过 ref link 定义 `[ref]:`
            if text.as_bytes().get(open_bracket + 1) == Some(&b']')
                && text.as_bytes().get(open_bracket + 2) == Some(&b':')
            {
                continue;
            }
            // 向右找对应的 `]`
            let mut close_bracket = None;
            let mut i = open_bracket + 1;
            while i < text.len() {
                let b = text.as_bytes()[i];
                if b == b']' {
                    close_bracket = Some(i);
                    break;
                }
                if b == b'\n' {
                    break; // link 不跨多行
                }
                i += 1;
            }
            let cb = match close_bracket {
                Some(x) => x,
                None => continue, // 这个 `[` 没有闭合，试更早的
            };
            // 验证 `]` 后紧跟 `(`
            if cb + 1 >= text.len() || text.as_bytes()[cb + 1] != b'(' {
                continue;
            }
            // 找配对的 `)`（处理 URL 内嵌括号）
            let mut depth = 1;
            let mut j = cb + 2; // cb = close_bracket，从 `(` 之后第一个字符开始
            let mut closed = false;
            while j < text.len() {
                match text.as_bytes()[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            j += 1; // 跨过闭合 `)`，j = 闭合 `)` 之后的半开位置
                            closed = true;
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if !closed {
                continue;
            }
            let link_end = j;
            // 验证 caret 在 link 区间内
            if caret >= open_bracket && caret <= link_end {
                // 内部不含未配对 `[` 或 `]`
                let inner = &text[open_bracket + 1..cb];
                if !inner.contains('[') && !inner.contains(']') {
                    return Some((open_bracket, link_end));
                }
            }
            break; // 只看最近一个 `[`
        }
        None
    }
}

/// 缩进单位（4 空格）。
pub(crate) const INDENT: &str = "    ";

/// 一行前缀变换计划：`span` 为被整体替换的区间，`text` 为替换后文本，
/// `lines` 为 (行首旧偏移, 行前缀增量) 升序表，供 caret/anchor 落点映射。
pub(crate) struct IndentPlan {
    pub(crate) span: Range,
    pub(crate) text: String,
    pub(crate) lines: Vec<(usize, isize)>,
}

impl IndentPlan {
    /// 旧坐标 → 变换后坐标。`with_prefix_at_line_start` 为 true 时行首位置也随前缀
    /// 右移（caret）；false 时留在新行首之前（anchor，含缩进）。反缩进落点位于被删
    /// 空白内时夹到新行首。
    pub(crate) fn map(&self, offset: usize, with_prefix_at_line_start: bool) -> usize {
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
pub(crate) fn word_left(text: &str, offset: usize) -> usize {
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
pub(crate) fn word_right(text: &str, offset: usize) -> usize {
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
pub(crate) fn word_run(text: &str, offset: usize) -> Option<(usize, usize)> {
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
pub(crate) fn line_starts(text: &str) -> Vec<usize> {
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
pub(crate) fn line_index(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|&s| s <= offset) - 1
}

/// 文档换行风格：首个 `\n` 前紧邻 `\r` 即 CRLF；无换行或首个即 LF 则为 LF。
/// 只检测一次，编辑插入新行沿用该风格，保证不引入混合换行。
pub(crate) fn detect_line_ending(text: &str) -> &'static str {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

/// 行级缩进前缀（与 `dedent_prefix_len` 对称）：引用行加一层 `> `（保持引用结构），
/// 其余行加 2 空格。2 空格是本解析器列表 depth 的单位，且 < 4 不触发缩进代码块。
pub(crate) fn indent_prefix(content: &str) -> &'static str {
    if content.trim_start_matches([' ', '\t']).starts_with('>') {
        "> "
    } else {
        "  "
    }
}

/// 反缩进要删除的行首长度：引用行移除一层 `>` 标记（含其后一个空格），
/// 其余行删除一个制表符或至多 4 个空格；非空白字符绝不动。
pub(crate) fn dedent_prefix_len(content: &str) -> usize {
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
pub(crate) struct ListMarker {
    pub(crate) next: String,
    pub(crate) empty: bool,
}

/// 解析行首标记（C4）。任务项优先于普通无序项；编号项取原数字 +1；
/// 续行前缀保留原缩进，任务项一律续成未勾选。不支持的写法（无空格分隔、
/// `*`/`+` 项目符号、`1)` 编号等）返回 None，走普通换行。
pub(crate) fn list_marker(content: &str) -> Option<ListMarker> {
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
