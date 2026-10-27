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
        let (mut v, consumed) = scan_next(buf, line, total);
        out.append(&mut v);
        line += consumed;
    }
    out
}

/// 扫描以 `line` 起始的一个扫描单元：普通块为单块；列表标记行触发区域扫描，
/// 整片列表（嵌套项/续块，0c P0-3）一次产出。返回 (块序列, 消耗行数)。
pub(super) fn scan_next(
    buf: &Buffer,
    line: usize,
    total: usize,
) -> (Vec<(BlockKind, usize, usize)>, usize) {
    let text = line_text(buf, line);
    // 缩进 ≥4 列（tab 展开后）属缩进代码领地：先于一切标记判定
    if indent_info(&text).0 >= 4 {
        let (kind, n) = scan_one(buf, line, total);
        return (vec![(kind, line, line + n)], n);
    }
    let t = text.trim_start();
    // 判定序与 scan_one 一致：ATX/围栏/数学/分隔线/引用 优先于列表
    let earlier = atx_level(t).is_some()
        || is_fence(&text)
        || math_marker(text.trim()).is_some()
        || is_thematic_break(t)
        || quote_marker_pos(&text).is_some();
    if !earlier && list_mark(&text).is_some_and(|m| m.indent_cols <= 3) {
        return scan_list_region(buf, line, total);
    }
    let (kind, n) = scan_one(buf, line, total);
    (vec![(kind, line, line + n)], n)
}

/// 扫描单个非列表块，返回 (种类, 消耗行数)。列表项由 `scan_next` 的区域扫描接管。
pub(super) fn scan_one(buf: &Buffer, line: usize, total: usize) -> (BlockKind, usize) {
    let text = line_text(buf, line);
    // 缩进代码块（0c P1）：前导缩进 ≥4 列优先于一切块标记（`    # foo` 是代码）。
    // 空行暂存、非空行须 ≥4 列续块；末尾空行不计入块区间。
    if indent_info(&text).0 >= 4 {
        return (BlockKind::IndentedCode, indented_code_lines(buf, line, total));
    }
    let t = text.trim_start();
    if let Some(level) = atx_level(t) {
        return (BlockKind::AtxHeading(level), 1);
    }
    if let Some((ch, len, lang)) = fence_open(&text) {
        let mut n = 1;
        while line + n < total {
            if fence_close(&line_text(buf, line + n), ch, len) {
                return (BlockKind::FencedCode(lang), n + 1);
            }
            n += 1;
        }
        return (BlockKind::FencedCode(lang), n); // 未闭合围栏吞咽至 EOF（GFM 行为）
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
    if is_thematic_break(t) {
        return (BlockKind::ThematicBreak, 1);
    }
    if quote_marker_pos(&text).is_some() {
        return (BlockKind::BlockQuote, quote_lines(buf, line, total));
    }
    // 列表项由 scan_next 的区域扫描接管，此处不再处理
    if let Some(n) = table_rows(buf, line, total) {
        return (BlockKind::Table, n);
    }
    // 链接引用定义：块起始识别（不打断进行中段落，段落循环不检查此项）
    if let Some(n) = link_ref_def(buf, line, total) {
        return (BlockKind::LinkRefDef, n);
    }
    // 段落：直到空行 / 可打断段落的块起始（打断规则集中于 `interrupts_paragraph`）。
    // 表格 v0 不打断段落——同 GFM，表头须位于块起始。
    // Setext 下划线（0c P0-2）优先于全部打断判定：`Foo\n---` 整体成 2 级标题。
    let mut n = 1;
    while line + n < total {
        let l = line_text(buf, line + n);
        if l.trim().is_empty() {
            break;
        }
        if let Some(level) = setext_underline(&l) {
            return (BlockKind::SetextHeading(level), n + 1);
        }
        if interrupts_paragraph(&l) {
            break;
        }
        n += 1;
    }
    (BlockKind::Paragraph, n)
}

/// 段落打断判定（CommonMark §5.2/§5.3 的「可打断段落」集合）：围栏 / ATX / 数学标记 /
/// 分隔线（缩进 ≤3，4+ 属缩进代码领地——Example 87）/ 引用标记（≤3）/ 列表标记
/// （空内容标记与起始号非 1 的有序标记不打断——Example 285/304）。
fn interrupts_paragraph(l: &str) -> bool {
    // 缩进 ≥4 列（tab 展开后）是缩进代码块，而缩进代码块不能打断段落
    // （Example 70：`foo\n    # bar` 是同一段落）。
    if indent_info(l).0 >= 4 {
        return false;
    }
    let lt = l.trim_start();
    is_fence(l)
        || atx_level(lt).is_some()
        || math_marker(l.trim()).is_some()
        || (indent_le3(l) && is_thematic_break(lt))
        || quote_marker_pos(l).is_some()
        || list_mark(l).is_some_and(|m| marker_interrupts_paragraph(&m))
}

/// 列表标记打断段落的附加规则：缩进 ≤3 列、内容非空、有序起始号为 1。
fn marker_interrupts_paragraph(m: &ListMark) -> bool {
    m.indent_cols <= 3 && !m.blank_content && (!m.ordered || m.start == 1)
}

/// Setext 下划线（0c P0-2）：`=+` → 1 级、`-+` → 2 级；前导 ≤3 空格（\t 不合法，
/// 制表位列宽语义属 Tabs 增量），字符间不允许空白（区别于分隔线：`--- -` 归后者），
/// 尾随空白随意。仅在段落续行位置调用；块起始的 `---`/`===` 永不进入本函数。
fn setext_underline(line: &str) -> Option<u8> {
    let t = line.trim_end_matches(['\r', '\n']);
    let b = t.as_bytes();
    let mut i = 0;
    while b.get(i) == Some(&b' ') {
        i += 1;
    }
    if i > 3 {
        return None;
    }
    let marker = *b.get(i)?;
    let level = match marker {
        b'=' => 1u8,
        b'-' => 2u8,
        _ => return None,
    };
    let mut j = i;
    while b.get(j) == Some(&marker) {
        j += 1;
    }
    if t[j..].bytes().all(|c| matches!(c, b' ' | b'\t')) {
        Some(level)
    } else {
        None
    }
}

/// 前导缩进 ≤3 空格（遇 \t 直接判负：v0 不展开制表位，保守处理）。
fn indent_le3(line: &str) -> bool {
    let spaces = line.bytes().take_while(|&c| c == b' ').count();
    spaces <= 3 && line.as_bytes().get(spaces) != Some(&b'\t')
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

/// ATX 标题内容区间（块相对字节）：剥除前导缩进/`#` 标记与可选闭合 `#` 序列。
/// 闭合序列须由空白引出（CommonMark：`# Title ###` → `Title`，`# Title#` → `Title#`）；
/// 内容为空时返回 `(cs, cs)`。
pub(super) fn heading_content_span(line: &str) -> Option<(usize, usize)> {
    let t = line.trim_start();
    let indent = line.len() - t.len();
    let hashes = t.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let mut cs = indent + hashes;
    if matches!(line.as_bytes().get(cs), Some(b' ') | Some(b'\t')) {
        cs += 1;
    }
    let mut ce = line.trim_end().len();
    let hashed_end = line[..ce].trim_end_matches('#').len();
    if hashed_end < ce
        && (hashed_end == cs
            || (hashed_end > 0
                && matches!(
                    line.as_bytes().get(hashed_end - 1),
                    Some(b' ') | Some(b'\t')
                )))
    {
        ce = line[..hashed_end].trim_end().len();
    }
    Some((cs, ce.max(cs)))
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

// ===== 制表位 / 缩进（0c P1） =====
//
// CommonMark 以 4 列为一个制表位：`\t` 推进到下一个 4 的倍数。块级缩进判定必须
// 按列而非字节（`\tfoo` 与 `    foo` 等价），内容切片再由列映射回字节。

/// 前导空白列宽（tab 展开到 4 列制表位）+ 首个非空白字节偏移。
pub(super) fn indent_info(line: &str) -> (usize, usize) {
    let mut col = 0usize;
    for (idx, ch) in line.char_indices() {
        match ch {
            ' ' => col += 1,
            '\t' => col += 4 - col % 4,
            _ => return (col, idx),
        }
    }
    (col, line.len())
}

/// 列位置 → 字节偏移（跨越制表位的列取该 tab 之后的字节）。
fn col_to_byte(line: &str, target: usize) -> usize {
    let mut col = 0usize;
    for (idx, ch) in line.char_indices() {
        if col >= target {
            return idx;
        }
        match ch {
            ' ' => col += 1,
            '\t' => col += 4 - col % 4,
            _ => return idx,
        }
    }
    line.len()
}

/// 缩进代码块消耗行数：空行暂存（可含尾随空白），非空行须缩进 ≥4 列才续块；
/// 末尾空行不计入块区间（块在最后一个非空行结束）。
fn indented_code_lines(buf: &Buffer, line: usize, total: usize) -> usize {
    let mut n = 1usize;
    let mut last_nonblank = 1usize;
    while line + n < total {
        let l = line_text(buf, line + n);
        if l.trim().is_empty() {
            n += 1;
            continue;
        }
        if indent_info(&l).0 >= 4 {
            n += 1;
            last_nonblank = n;
            continue;
        }
        break;
    }
    last_nonblank
}

/// 开围栏判定：缩进 ≤3 列，≥3 个连续 ``` 或 ~~~；反引号围栏的 info string
/// 不得含反引号（Example 138/145：```` ``` ```` 是段落而非围栏）。
/// 返回 (围栏字符, 长度, info 首词)。
pub(super) fn fence_open(line: &str) -> Option<(u8, usize, Option<String>)> {
    let (indent, start) = indent_info(line);
    if indent > 3 {
        return None;
    }
    let b = line.as_bytes();
    let ch = *b.get(start)?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let mut n = 0usize;
    while b.get(start + n) == Some(&ch) {
        n += 1;
    }
    if n < 3 {
        return None;
    }
    let info = line[start + n..].trim_end_matches(['\r', '\n']);
    if ch == b'`' && info.contains('`') {
        return None;
    }
    let lang = info.split_whitespace().next().map(str::to_string);
    Some((ch, n, lang))
}

/// 闭围栏判定：同字符、长度 ≥ 开围栏长度，标记后只允许空白；缩进 ≤3 列
/// （Example 137：4 空格缩进的 ``` 只是内容）。
pub(super) fn fence_close(line: &str, ch: u8, len: usize) -> bool {
    let (indent, start) = indent_info(line);
    if indent > 3 {
        return false;
    }
    let b = line.as_bytes();
    let mut n = 0usize;
    while b.get(start + n) == Some(&ch) {
        n += 1;
    }
    n >= len
        && line[start + n..]
            .trim_end_matches(['\r', '\n'])
            .bytes()
            .all(|c| c == b' ' || c == b'\t')
}

/// 是否为围栏开标记行（含 `~~~`）。
pub(super) fn is_fence(line: &str) -> bool {
    fence_open(line).is_some()
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

// ===== 列表项 / 引用块 / 分隔线 =====
//
// v0 取舍（逐条钉在测试上）：
// - 列表项（0c P0-3 区域扫描）：标记后须为空白或行尾（`-tag` 非列表，`---` 归分隔线）。
//   有序标记为 1–9 位数字 + `.`/`)`。块起始标记要求缩进 ≤3（Ex 289，4+ 属缩进代码）。
//   内容列 = 缩进 + 标记宽 W + 空白数 N（N ≥ 5 或空内容时取 W+1）；续行缩进 ≥ 内容列
//   并入项；项间/项内空行不终止列表（宽松列表）；段落可无缩进懒续行（不可跨空行、
//   不可遇可打断段落者）。`depth` = 缩进字节 / 2 上限 8（视觉深度；结构嵌套由栈判定）。
// - 引用块：连续 `>` 行 + 段落懒续行合为一块，空行终止；`>` 标记要求缩进 ≤3（Ex 231）。
// - 分隔线：`-` / `*` / `_` 同字符 ≥ 3，其间只允许空白；段落续行位置的
//   `=+`/`-+` 归 setext 下划线（见 `setext_underline`），块起始恒为分隔线。

/// 列表标记几何。
pub(super) struct ListMark {
    pub ordered: bool,
    /// 视觉深度 = 缩进列 / 2（上限 8）；结构嵌套由区域扫描器的栈判定。
    pub depth: u8,
    /// 首行内容实际起点（inline 解析 / 任务复选框定位用）。
    pub content_start: usize,
    /// 前导空白列宽（tab 按 4 列制表位展开；缩进合法性/嵌套判定用）。
    pub indent_cols: usize,
    /// 标记后空白字节数 N。
    pub n_spaces: usize,
    /// 内容列 = 缩进列 + 标记宽 W + N 列；N ≥ 5 或空内容时取 W+1（CommonMark §5.2）。
    pub content_col: usize,
    /// 标记字节：`-`/`*`/`+` 或有序分隔符 `.`/`)`（列表分组签名用）。
    pub marker: u8,
    /// 有序标记起始号（段落打断要求 == 1，Example 304）。
    pub start: u32,
    /// 标记后仅有空白：块起始合法（Ex 278），但不打断段落（Ex 285）。
    pub blank_content: bool,
}

/// 识别行首列表标记；非列表返回 None。`text` 可为整行（含行尾换行）。
pub(super) fn list_mark(text: &str) -> Option<ListMark> {
    let (indent_cols, indent) = indent_info(text);
    let b = text.as_bytes();
    let (ordered, marker, marker_len, start) = match b.get(indent)? {
        &c @ (b'-' | b'*' | b'+') => (false, c, 1, 0),
        d if d.is_ascii_digit() => {
            let digits = b[indent..]
                .iter()
                .take_while(|x| x.is_ascii_digit())
                .count();
            if digits > 9 {
                return None;
            }
            match b.get(indent + digits) {
                Some(&delim @ (b'.' | b')')) => {
                    let start = text[indent..indent + digits].parse().ok()?;
                    (true, delim, digits + 1, start)
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    let after = indent + marker_len;
    // 标记后空白：字节数与列数分开记（tab 按 4 列制表位推进）
    let mut n_spaces = 0usize;
    let mut n_cols = 0usize;
    let mut col = indent_cols + marker_len;
    while let Some(&c) = b.get(after + n_spaces) {
        match c {
            b' ' => {
                col += 1;
                n_cols += 1;
            }
            b'\t' => {
                let adv = 4 - col % 4;
                col += adv;
                n_cols += adv;
            }
            _ => break,
        }
        n_spaces += 1;
    }
    let blank_content = matches!(b.get(after + n_spaces), None | Some(b'\n') | Some(b'\r'));
    if n_spaces == 0 && !blank_content {
        return None; // 标记后非空白：`-tag` / `1.x`
    }
    let content_col = if blank_content || n_cols >= 5 {
        indent_cols + marker_len + 1
    } else {
        indent_cols + marker_len + n_cols
    };
    Some(ListMark {
        ordered,
        depth: (indent_cols / 2).min(8) as u8,
        content_start: after + n_spaces,
        indent_cols,
        n_spaces,
        content_col,
        marker,
        start,
        blank_content,
    })
}

/// 列表分组签名（0c P0-3）：(是否有序, 标记字节)。仅块起始标记（缩进 ≤3）有签名；
/// 续块首行（无合法标记）返回 None——调用方据此令续块随前项归并。
pub fn list_signature(line: &str) -> Option<(bool, u8)> {
    let m = list_mark(line)?;
    (m.indent_cols <= 3).then_some((m.ordered, m.marker))
}

/// GFM 任务列表复选框（P3-5）：`- [ ]` / `- [x]` / `- [X]`（有序标记同）。
/// 复选框须紧随列表标记空白（内容起点处），其后须为空白或行尾（`- [x]tag` 不算）。
/// 返回 `(标记起点 = 缩进起, 状态字符偏移, 复选框区间终点 = 内容起点, 勾选态)`。
pub fn task_span(text: &str) -> Option<(usize, usize, usize, bool)> {
    let m = list_mark(text)?;
    // 复选框须紧随标记后的单个空白（v0 严格：`-  [x]` 双空格不算）
    if m.n_spaces != 1 {
        return None;
    }
    let b = text.as_bytes();
    let i = m.content_start;
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let done = match b.get(i + 1)? {
        b' ' => false,
        b'x' | b'X' => true,
        _ => return None,
    };
    if b.get(i + 2) != Some(&b']') {
        return None;
    }
    // 复选框后须为空白或行尾；空白并入复选框区间（视图折叠为 ☐/☑ 后不留双空格）
    let end = match b.get(i + 3) {
        Some(b' ') | Some(b'\t') => i + 4,
        None | Some(b'\n') | Some(b'\r') => i + 3,
        _ => return None,
    };
    let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
    Some((indent, i + 1, end, done))
}

/// 引用行首 `>` 的位置；非引用行返回 None。块起始/懒续行的 `>` 均要求缩进 ≤3
/// （Example 231：4 空格缩进属缩进代码领地；\t 按 v0 保守判负）。
pub(super) fn quote_marker_pos(line: &str) -> Option<usize> {
    let spaces = line.bytes().take_while(|&c| c == b' ').count();
    if spaces > 3 || line.as_bytes().get(spaces) == Some(&b'\t') {
        return None;
    }
    (line.as_bytes().get(spaces) == Some(&b'>')).then_some(spaces)
}

/// 引用块行数（0c P0-3）：连续 `>` 行 + 段落懒续行，空行终止。
/// 懒续行仅当引用内部当前块为段落（剥除全部容器标记后判定），且本行不可打断段落
/// （`---` 归分隔线可打断——Ex 94/101；`===` 可懒续为 setext——Ex 93）。
fn quote_lines(buf: &Buffer, line: usize, total: usize) -> usize {
    let mut n = 1;
    let mut para = quote_inner_para(&line_text(buf, line));
    while line + n < total {
        let l = line_text(buf, line + n);
        if l.trim().is_empty() {
            break;
        }
        if quote_marker_pos(&l).is_some() {
            para = quote_inner_para(&l);
            n += 1;
            continue;
        }
        if para && !interrupts_paragraph(&l) {
            n += 1; // 懒续行：仍为段落文本
            continue;
        }
        break;
    }
    n
}

/// 引用行内部（剥除全部容器标记后）是否为段落。
fn quote_inner_para(line: &str) -> bool {
    let inner = strip_containers(line);
    let t = inner.trim_start();
    if t.trim().is_empty() {
        return false;
    }
    !(is_fence(t)
        || atx_level(t).is_some()
        || math_marker(inner.trim()).is_some()
        || (indent_le3(inner) && is_thematic_break(t))
        || list_mark(inner).is_some_and(|m| marker_interrupts_paragraph(&m)))
}

/// 反复剥除行首容器标记（`>` 与列表标记），返回内部内容（Ex 292：引用套列表套引用）。
fn strip_containers(mut s: &str) -> &str {
    loop {
        if let Some(p) = quote_marker_pos(s) {
            let mut e = p + 1;
            if matches!(s.as_bytes().get(e), Some(b' ') | Some(b'\t')) {
                e += 1;
            }
            s = &s[e..];
            continue;
        }
        if let Some(m) = list_mark(s) {
            s = &s[m.content_start..];
            continue;
        }
        return s;
    }
}

/// 列表区域扫描（0c P0-3）：从首个标记行起，产出整片列表的块序列。
/// 栈存开放项的内容列；标记行弹栈至内容列 ≤ 缩进，再验 indent-col ≤ 3（栈空则
/// indent ≤ 3）决定归属层级；内容行并入归属项当前块（跨内层闭合的外层续行产新续块，
/// 继承归属项签名）；空行暂存不终止，但永久闭合尚无内容的项（Ex 280）；懒续行仅续
/// 段落且不跨空行；项内围栏吞咽含空行（v0 规则：闭合围栏任意缩进）。
fn scan_list_region(
    buf: &Buffer,
    start: usize,
    total: usize,
) -> (Vec<(BlockKind, usize, usize)>, usize) {
    /// 开放项：内容列 + 项身份 + 续块继承所需的签名信息
    struct OpenItem {
        col: usize,
        id: usize,
        ordered: bool,
        depth: u8,
        /// 是否已有内容（非空首行内容 / 子项 / 续行内容）。
        /// 空行永久闭合无内容的项（cmark：空行仅匹配已有子块的项，Ex 280）。
        has_content: bool,
    }
    let mut blocks: Vec<(BlockKind, usize, usize)> = Vec::new();
    let mut stack: Vec<OpenItem> = Vec::new();
    let mut next_id = 0usize;
    let mut cur = 0usize; // 当前生长块下标（首行必开项，恒定有效）
    let mut cur_item = usize::MAX;
    let mut cur_para = false;
    // 项内围栏：(字符, 长度)；闭合须同字符且长度 ≥ 开围栏。
    let mut in_fence: Option<(u8, usize)> = None;
    let mut pending_blanks = 0usize;
    let mut i = start;

    while i < total {
        let l = line_text(buf, i);
        if let Some((ch, len)) = in_fence {
            blocks[cur].2 = i + 1;
            if fence_close(l.trim_start(), ch, len) {
                in_fence = None;
            }
            i += 1;
            continue;
        }
        if l.trim().is_empty() {
            pending_blanks += 1;
            cur_para = false; // 空行闭合段落：懒续行不跨空行
            // 空行永久闭合无内容的项（Ex 280：`-\n\n  foo` → 列表外段落）
            while stack.last().is_some_and(|t| !t.has_content) {
                stack.pop();
            }
            i += 1;
            continue;
        }
        let (indent, _) = indent_info(&l);
        // 试弹栈：内容列 > 本行缩进的项对本行闭合（懒续行命中时不提交）
        let mut popped_len = stack.len();
        while popped_len > 0 && stack[popped_len - 1].col > indent {
            popped_len -= 1;
        }
        // 分隔线优先于列表标记（cmark 判定序）：够不着任何开放项且缩进 ≤3 →
        // 顶层分隔线，区域终止（Example 60）；达到某开放项 → 落入内容分支
        // （hr 入项，Example 61 同型）。
        let thematic = is_thematic_break(l.trim_start());
        if thematic && popped_len == 0 && indent <= 3 {
            break;
        }
        if !thematic {
            if let Some(m) = list_mark(&l) {
            let valid = match stack.get(popped_len.wrapping_sub(1)) {
                _ if popped_len == 0 => indent <= 3,
                _ => indent - stack[popped_len - 1].col <= 3,
            };
            // 接受规则（对齐 cmark 的「last matched ≠ 段落」判定，Ex 302/312 实测）：
            // - 弹栈（缩进够不着当前项）：当前项及其段落已被反缩进闭合，标记自由
            //   开项——起始号非 1 / 空内容亦可（Ex 302 `3) baz` 开新列表）；
            // - 未弹栈且当前块为开放段落：适用段落打断规则——内容非空且（无序或
            //   起始号为 1）（Ex 285/304 的区域内形态；缩进合法性已由 valid 保证，
            //   此处不再要求绝对缩进 ≤3——嵌套标记缩进可 >3，Ex 307）。
            let accept = valid
                && (popped_len < stack.len()
                    || !cur_para
                    || (!m.blank_content && (!m.ordered || m.start == 1)));
            if accept {
                stack.truncate(popped_len);
                // 子项入栈即父项内容（父项不再是「无内容项」，空行不再闭合它）
                if let Some(parent) = stack.last_mut() {
                    parent.has_content = true;
                }
                let kind = BlockKind::ListItem {
                    ordered: m.ordered,
                    depth: m.depth,
                    task: task_span(&l).map(|(_, _, _, done)| done),
                };
                blocks.push((kind, i, i + 1));
                let id = next_id;
                next_id += 1;
                stack.push(OpenItem {
                    col: m.content_col,
                    id,
                    ordered: m.ordered,
                    depth: m.depth,
                    has_content: !m.blank_content,
                });
                cur = blocks.len() - 1;
                cur_item = id;
                pending_blanks = 0;
                let (para, fence) = classify_content(&l[m.content_start..]);
                cur_para = para;
                in_fence = fence;
                i += 1;
                continue;
            }
            // 不能打断段落的标记行 → 懒续为段落文本（落入下方懒续分支）
            }
        }
        if popped_len > 0 {
            // 内容行：并入归属项（indent ≥ 内容列由弹栈不变量保证）
            let (owner_id, owner_ordered, owner_depth, owner_col) = {
                let top = &stack[popped_len - 1];
                (top.id, top.ordered, top.depth, top.col)
            };
            stack.truncate(popped_len);
            stack[popped_len - 1].has_content = true; // 内容行即归属项内容
            if cur_item == owner_id {
                blocks[cur].2 = i + 1; // 延伸覆盖暂存空行
            } else {
                // 外层续块：内层已闭合，归属外层项（无标记无任务框）
                blocks.push((
                    BlockKind::ListItem {
                        ordered: owner_ordered,
                        depth: owner_depth,
                        task: None,
                    },
                    i,
                    i + 1,
                ));
                cur = blocks.len() - 1;
                cur_item = owner_id;
            }
            pending_blanks = 0;
            // 内容列按列计，切片前映射回字节（tab 跨越内容列时取 tab 之后）
            let (para, fence) = classify_content(&l[col_to_byte(&l, owner_col)..]);
            cur_para = para;
            in_fence = fence;
            i += 1;
            continue;
        }
        // 缩进够不着任何开放项：懒续行（不弹栈，项保持开放）
        if pending_blanks == 0 && cur_para && !interrupts_paragraph(&l) {
            blocks[cur].2 = i + 1;
            i += 1;
            continue;
        }
        break; // 区域终止（试弹栈作废）
    }
    (blocks, i - start)
}

/// 项内容分类（懒续行/围栏吞咽判定）：(是否段落, 开启的围栏 (字符, 长度))。
/// 容器性内容（ATX/围栏/数学/分隔线/引用/嵌套标记）均非段落。
fn classify_content(content: &str) -> (bool, Option<(u8, usize)>) {
    let t = content.trim_start();
    if t.trim().is_empty() {
        return (false, None);
    }
    if let Some((ch, len, _)) = fence_open(t) {
        return (false, Some((ch, len)));
    }
    if atx_level(t).is_some()
        || math_marker(content.trim()).is_some()
        || is_thematic_break(t)
        || t.starts_with('>')
        || list_mark(content).is_some()
    {
        return (false, None);
    }
    (true, None)
}

/// 分隔线：同字符 `-` / `*` / `_` ≥ 3，其间只允许空白。
pub(super) fn is_thematic_break(t: &str) -> bool {
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

// ===== 链接引用定义（0c P0-1） =====
//
// CommonMark 0.31.2：`[label]: dest "title"`。逐条取舍（钉在测试上）：
// - 缩进 ≤3 空格；label 1–999 字节、无未转义 `[`/`]`、至少一个非空白字符、不跨行。
// - `:` 与 dest、dest 与 title 间允许空白 + 至多一个换行；dest 为 `<…>`（内无 `<`/换行）
//   或裸串（无空白/控制字符/`<`，未转义括号须平衡；空 dest 仅 `<>` 合法）。
// - title 可选：`"…"`/`'…'`/`(…)`，不跨行；换行后若以引号/`(` 开头则必须是纯 title 行，
//   否则整个定义非法（Example 193）；定义尾部只允许空白至行尾（Example 190）。
// - 定义只识别于块起始，不打断进行中的段落；引用块内定义 v0 不支持（已知分歧）。
// - label 匹配规范化：trim + 内部空白折叠 + Unicode lowercase；label 内反斜杠转义
//   不解析（v0 简化）。同 label 重复定义 first wins。
// - 定义行产出独立块 LinkRefDef（编辑器可见、不背叛源码），注册进文档引用表；
//   HTML 导出为空输出，inline 为空。

/// 引用表：规范化 label → (dest 区间, title 区间)，文档绝对字节偏移。
pub(super) type RefMap = std::collections::HashMap<String, (Range, Option<Range>)>;

/// 规范化 label：折叠内部空白为单空格 + lowercase（不解反斜杠转义，v0 简化）。
pub(super) fn normalize_label(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// 定义解析结果；区间为 `base` 基准的绝对偏移。
pub(super) struct RefDefAst {
    pub label: Range,
    pub dest: Range,
    pub title: Option<Range>,
    /// 定义终点（dest/title 末尾字节的相对偏移）；其后至行尾已保证纯空白。
    pub len: usize,
}

/// 跳过行内空白（空格/制表符，不含 `\r`——其仅出现于行尾，与 `\n` 一并处理）。
fn skip_inline_ws(b: &[u8], i: &mut usize) {
    while matches!(b.get(*i), Some(b' ') | Some(b'\t') | Some(b'\r')) {
        *i += 1;
    }
}

/// 跳过空白 + 至多一个换行（`:` 与 dest、dest 与 title 间的合法间隔）。
fn skip_ws_one_newline(b: &[u8], i: &mut usize) {
    skip_inline_ws(b, i);
    if b.get(*i) == Some(&b'\n') {
        *i += 1;
        skip_inline_ws(b, i);
    }
}

/// 在 `text` 开头解析一个链接引用定义（`base` 为 text 起点的文档绝对偏移）。
/// text 可含后续行（dest/title 允许换行）；失败返回 None（调用方按段落处理）。
pub(super) fn parse_link_ref_def(text: &str, base: usize) -> Option<RefDefAst> {
    let b = text.as_bytes();
    let mut i = 0;
    while b.get(i) == Some(&b' ') {
        i += 1;
    }
    if i > 3 {
        return None; // 缩进超限：属缩进代码块领地
    }
    // label：`[...]`，无未转义方括号，不跨行，至少一个非空白
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let ls = i + 1;
    let mut j = ls;
    let mut has_non_ws = false;
    let le = loop {
        match b.get(j) {
            None | Some(b'\n') => return None,
            Some(b'\\') => j += 2,
            Some(b'[') => return None,
            Some(b']') => break j,
            Some(&c) => {
                if !c.is_ascii_whitespace() {
                    has_non_ws = true;
                }
                j += 1;
            }
        }
    };
    if le - ls > 999 || !has_non_ws {
        return None;
    }
    j = le + 1;
    if b.get(j) != Some(&b':') {
        return None;
    }
    j += 1;
    skip_ws_one_newline(b, &mut j);
    // dest：`<…>` 或裸串
    let (ds, de, after) = match b.get(j) {
        Some(b'<') => {
            let s = j + 1;
            let mut k = s;
            loop {
                match b.get(k) {
                    None | Some(b'\n') | Some(b'<') => return None,
                    Some(b'>') => break (s, k, k + 1),
                    _ => k += 1,
                }
            }
        }
        Some(&c) if !c.is_ascii_whitespace() => {
            let s = j;
            let mut k = j;
            let mut depth = 0i32;
            loop {
                match b.get(k) {
                    Some(b'\\') => k += 2,
                    Some(b'(') => {
                        depth += 1;
                        k += 1;
                    }
                    Some(b')') => {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                        k += 1;
                    }
                    Some(&c2) if c2.is_ascii_whitespace() || c2 == b'<' || c2 < 0x20 => break,
                    None => break,
                    Some(_) => k += 1,
                }
            }
            if k == s || depth != 0 {
                return None;
            }
            (s, k, k)
        }
        _ => return None, // 缺 dest（裸 `[foo]:` 非法）
    };
    j = after;
    // 终点默认为 dest 尾；其后行内空白跳过，再判 title / 行尾
    let mut end = de;
    let ws_start = j;
    skip_inline_ws(b, &mut j);
    // title 起点候选：同行剩余非空白，或（换行后）下一行行首
    let mut t = j;
    let crossed_newline = if b.get(t) == Some(&b'\n') {
        t += 1;
        skip_inline_ws(b, &mut t);
        true
    } else {
        false
    };
    // title 与 dest 间必须至少一个空白/换行：`<bar>(baz)` 中 title 紧贴 dest 整体非法
    // （Example 201）；换行本身即合法间隔，不受此限。
    if !crossed_newline
        && j == ws_start
        && matches!(b.get(t), Some(b'"') | Some(b'\'') | Some(b'('))
    {
        return None;
    }
    match b.get(t).copied() {
        // 行尾/EOF：无 title，定义结束于 dest
        None | Some(b'\n') => {}
        Some(o @ (b'"' | b'\'' | b'(')) => {
            let closer = match o {
                b'"' => b'"',
                b'\'' => b'\'',
                _ => b')',
            };
            let ts = t + 1;
            let mut m = ts;
            let te = loop {
                match b.get(m) {
                    Some(b'\\') => m += 2,
                    Some(&c) if c == closer => break m,
                    // `(…)` 内不允许未转义 `(`；title 不跨行
                    Some(b'(') if closer == b')' => return None,
                    None | Some(b'\n') => return None,
                    Some(_) => m += 1,
                }
            };
            // title 尾部：至行尾/EOF 只允许空白
            let mut k = te + 1;
            skip_inline_ws(b, &mut k);
            if b.get(k).is_some_and(|&c| c != b'\n') {
                return None; // `"title" ok` → 整体非法（Example 190/193）
            }
            let _ = crossed_newline;
            end = te;
            return Some(RefDefAst {
                label: Range::new(base + ls, base + le),
                dest: Range::new(base + ds, base + de),
                title: Some(Range::new(base + ts, base + te)),
                len: end + 1, // 含闭引号
            });
        }
        Some(_) => {
            // dest 后同行有非空白且非 title → 非法；换行后非引号开头 → 无 title 定义
            if !crossed_newline {
                return None;
            }
        }
    }
    Some(RefDefAst {
        label: Range::new(base + ls, base + le),
        dest: Range::new(base + ds, base + de),
        title: None,
        len: end,
    })
}

/// 块起始处识别链接引用定义，返回消耗行数（1–3）。非定义返回 None。
fn link_ref_def(buf: &Buffer, line: usize, total: usize) -> Option<usize> {
    let mut text = line_text(buf, line);
    let mut starts = vec![0usize];
    for k in 1..3usize {
        if line + k >= total {
            break;
        }
        starts.push(text.len());
        text.push_str(&line_text(buf, line + k));
    }
    let ast = parse_link_ref_def(&text, 0)?;
    // 消耗行数 = 终点覆盖到的行数（行起点 ≤ 终点位置）
    Some(starts.iter().filter(|&&s| s <= ast.len).count().max(1))
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
pub(super) fn parse_table_cells(
    line: &str,
    line_off: usize,
    ncols: usize,
    refs: &RefMap,
) -> Vec<TableCell> {
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
        let nodes = parse_inlines(content, line_off + cs, refs);
        let text = unescape_cell_pipes(&visible_text(content, &nodes, line_off + cs));
        out.push(TableCell {
            text,
            range: Range::new(line_off + cs, line_off + ce),
        });
    }
    out
}

/// inline 节点 → 可见文本：保留内容、剥除标记；Code/Math 内容原样。
pub(super) fn visible_text(src: &str, nodes: &[InlineNode], base: usize) -> String {
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
            InlineKind::Strong
            | InlineKind::Emph
            | InlineKind::Strikethrough
            | InlineKind::Link
            | InlineKind::Image => {
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
            [
                (BlockKind::Paragraph, 0, 1),
                (BlockKind::FencedCode(None), 1, 5)
            ]
        );
    }

    #[test]
    fn closed_fence_then_paragraph() {
        let b = Buffer::new("```\nx\n```\nb\n");
        assert_eq!(
            kinds(&b),
            [
                (BlockKind::FencedCode(None), 0, 3),
                (BlockKind::Paragraph, 3, 4)
            ]
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
            [
                (BlockKind::Paragraph, 0, 1),
                (BlockKind::FencedCode(None), 1, 4)
            ]
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
        // 缩进 0/2 → depth 0/1；有序标记保留种类。
        // 「  2) d」（0c P0-3）：缩进 2 < 「1. c」内容列 3 → 反缩进闭合该项，弹栈后
        // 顶层自由开项（起始号非 1 亦可，Ex 302 同型）→ 顶层兄弟项。
        let b = Buffer::new("- a\n  - b\n1. c\n  2) d\n");
        assert_eq!(
            kinds(&b),
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
                        ordered: true,
                        depth: 0,
                        task: None
                    },
                    2,
                    3
                ),
                (
                    BlockKind::ListItem {
                        ordered: true,
                        depth: 1,
                        task: None
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
                    depth: 0,
                    task: None
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
        // 深层嵌套：每层 `  - ` 深入一级，depth = 层数、上限 8
        let mut doc = String::new();
        for k in 0..10usize {
            doc.push_str(&" ".repeat(2 * k));
            doc.push_str("- x\n");
        }
        let b = Buffer::new(&doc);
        let got = kinds(&b);
        assert_eq!(got.len(), 10);
        for (k, (kind, s, e)) in got.iter().enumerate() {
            let BlockKind::ListItem { depth, .. } = kind else {
                panic!("line {k} 应为列表项");
            };
            assert_eq!(*depth, k.min(8) as u8, "line {k}");
            assert_eq!((*s, *e), (k, k + 1));
        }
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
                        depth: 0,
                        task: None
                    },
                    1,
                    2
                )
            ]
        );
    }

    #[test]
    fn task_span_parses_checkbox_variants() {
        // (缩进起, 状态字符, 区间终点含尾空格, 勾选态)；边角逐条钉死
        assert_eq!(task_span("- [ ] a\n"), Some((0, 3, 6, false)));
        assert_eq!(task_span("- [x] a\n"), Some((0, 3, 6, true)));
        assert_eq!(task_span("- [X] a\n"), Some((0, 3, 6, true)));
        assert_eq!(task_span("+ [x] a\n"), Some((0, 3, 6, true)));
        assert_eq!(task_span("* [x] a\n"), Some((0, 3, 6, true)));
        assert_eq!(task_span("1. [x] a\n"), Some((0, 4, 7, true)));
        assert_eq!(task_span("  - [ ] a\n"), Some((2, 5, 8, false)));
        assert_eq!(task_span("- [x]\n"), Some((0, 3, 5, true))); // 行尾项
        assert_eq!(task_span("- [x]  a\n"), Some((0, 3, 6, true))); // 只并入一个空白
                                                                    // 非任务
        assert_eq!(task_span("- [ ]a\n"), None); // 后无空白
        assert_eq!(task_span("- a [ ] b\n"), None); // 不在内容起点
        assert_eq!(task_span("-  [x] a\n"), None); // 标记后双空格（v0 严格）
        assert_eq!(task_span("- [] a\n"), None); // 空复选框
        assert_eq!(task_span("- [y] a\n"), None); // 非法状态
        assert_eq!(task_span("- a\n"), None); // 普通列表项
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
        // 0c P0-3：无 `>` 的段落文本行按懒续行并入引用块
        assert_eq!(
            kinds(&Buffer::new("> a\nb\n")),
            [(BlockKind::BlockQuote, 0, 2)]
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
    fn thematic_break_wins_over_list_marker_at_block_start() {
        // `---` 不是列表项（列表标记后须为空白或行尾）；块起始恒为分隔线
        // （setext 下划线只识别于段落续行位置，见下方 Setext 组）
        let b = Buffer::new("---\n");
        assert_eq!(kinds(&b), [(BlockKind::ThematicBreak, 0, 1)]);
    }

    // ===== 多行列表项 / 宽松列表 / 懒续行（0c P0-3）=====
    //
    // CommonMark 0.31.2 §5.2/§5.3：列表项以「内容列」吸纳后续行——续行缩进 ≥ 内容列
    // （内容列 = 标记缩进 + 标记宽 W + 空白数 N；N ≥ 5 或内容为空时取 W+1），项间/项内
    // 空行不终止列表（宽松列表）；段落可直接懒续行（无需缩进），可打断段落的块
    // （ATX/围栏/分隔线/非空列表标记/引用）不可懒续。块起始的列表标记要求缩进 ≤3。
    // 已知分歧（钉死）：制表位按字节计（Tabs 增量）；项内围栏吞咽按 v0 规则（闭合
    // 围栏任意缩进）；项内缩进代码/HTML 块的内部结构不解析（归 P1/P2）。

    fn li(ordered: bool, depth: u8, s: usize, e: usize) -> (BlockKind, usize, usize) {
        (BlockKind::ListItem { ordered, depth, task: None }, s, e)
    }

    #[test]
    fn list_item_continuation_lines_join_same_block() {
        // 续行缩进 ≥ 内容列（2）即并入项
        let b = Buffer::new("- foo\n  bar\n  baz\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 3)]);
        // 空行不终止列表：两个空行后仍是项内容（Example 262）
        let b = Buffer::new("- foo\n\n\n  bar\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 4)]);
        // Example 256：项间空行 + 宽松
        let b = Buffer::new("- one\n\n  two\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 3)]);
    }

    #[test]
    fn list_item_content_column_rules() {
        // Example 277：`-  foo` → 内容列 3；`   bar` 并入
        let b = Buffer::new("-  foo\n\n   bar\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 3)]);
        // Example 271：`  10.  foo` → 内容列 8；`           bar`（11 ≥ 8）并入
        let b = Buffer::new("  10.  foo\n\n           bar\n");
        assert_eq!(kinds(&b), [li(true, 1, 0, 3)]);
        // Example 273：N=5 → 内容列 = W+1 = 3；`   paragraph` 并入
        let b = Buffer::new("1.     indented code\n\n   paragraph\n\n       more code\n");
        assert_eq!(kinds(&b), [li(true, 0, 0, 5)]);
        // Example 257/258 边界：` -    one` → 内容列 6；缩进 5 不入项（≥4 列 → 缩进代码）、
        // 缩进 6 入项
        let b = Buffer::new(" -    one\n\n     two\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), (BlockKind::IndentedCode, 2, 3)]
        );
        let b = Buffer::new(" -    one\n\n      two\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 3)]);
    }

    #[test]
    fn list_marker_at_block_start_max_indent_3() {
        // Example 289：4 空格缩进的标记不是列表（缩进代码块）
        let b = Buffer::new("    1.  A paragraph\n        with two lines.\n");
        assert_eq!(kinds(&b), [(BlockKind::IndentedCode, 0, 2)]);
        // 3 空格合法
        let b = Buffer::new("   1.  A paragraph\n       with two lines.\n");
        assert_eq!(kinds(&b), [li(true, 1, 0, 2)]);
    }

    #[test]
    fn list_item_empty_marker_and_fence_inside() {
        // Example 279：标记后仅空白 → 内容列 = 缩进+W+1
        let b = Buffer::new("-   \n  foo\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 2)]);
        // Example 278：空内容项 + 项内围栏（围栏吞咽含空行）
        let b = Buffer::new("-\n  foo\n-\n  ```\n  bar\n  ```\n-\n      baz\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 2), li(false, 0, 2, 6), li(false, 0, 6, 8)]
        );
        // Example 318：项内围栏吞噬其间的空行
        let b = Buffer::new("- a\n- ```\n  b\n\n\n  ```\n- c\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), li(false, 0, 1, 6), li(false, 0, 6, 7)]
        );
    }

    #[test]
    fn list_item_empty_first_line_closed_by_blank() {
        // Example 280：空首行项遇空行永久闭合，后续缩进文本是列表外段落
        let b = Buffer::new("-\n\n  foo\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), (BlockKind::Paragraph, 2, 3)]
        );
        // 无空行时后续内容行正常入项（Example 279 同型）
        let b = Buffer::new("-\n  foo\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 2)]);
        // 已有内容的项不受空行影响（宽松列表项内空行后续块仍入项）
        let b = Buffer::new("- a\n\n  b\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 3)]);
        // 父项因子项而有内容：空行不闭合它，外层续行仍入项（产外层续块）
        let b = Buffer::new("-\n  - a\n\n  b\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), li(false, 1, 1, 2), li(false, 0, 3, 4)]
        );
    }

    #[test]
    fn list_item_lazy_continuation() {
        // Example 290/291：项内段落可无缩进懒续行
        let b = Buffer::new("  1.  A paragraph\nwith two lines.\n");
        assert_eq!(kinds(&b), [li(true, 1, 0, 2)]);
        let b = Buffer::new("  1.  A paragraph\n    with two lines.\n");
        assert_eq!(kinds(&b), [li(true, 1, 0, 2)]);
        // 懒续行不跨空行：空行后无缩进文本终止列表
        let b = Buffer::new("- a\n\nb\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), (BlockKind::Paragraph, 2, 3)]
        );
        // 懒续行遇可打断段落者即止
        let b = Buffer::new("- a\n# h\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), (BlockKind::AtxHeading(1), 1, 2)]
        );
        let b = Buffer::new("- a\n> q\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), (BlockKind::BlockQuote, 1, 2)]
        );
        // 项后的 `---` 是分隔线（不打断为项内 setext：下划线须达内容列，Example 94）
        let b = Buffer::new("- a\n---\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), (BlockKind::ThematicBreak, 1, 2)]
        );
    }

    #[test]
    fn list_item_paragraph_interruption_rules() {
        // Example 304：起始号非 1 的有序标记不打断段落
        let b = Buffer::new("The number of windows in my house is\n14.  The number of doors is 6.\n");
        assert_eq!(kinds(&b), [(BlockKind::Paragraph, 0, 2)]);
        // 起始号 1 可以打断
        let b = Buffer::new("a\n1. b\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), li(true, 0, 1, 2)]
        );
        // Example 285：空内容标记（`*` / `1.` 独占行）不打断段落
        let b = Buffer::new("foo\n*\n\nfoo\n1.\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 2), (BlockKind::Paragraph, 3, 5)]
        );
        // 块起始处空内容标记仍是合法项（Example 278）
        let b = Buffer::new("-\n  foo\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 2)]);
    }

    #[test]
    fn list_item_nested_and_outer_continuation() {
        // Example 307：嵌套项 + 两个空行 + 内容列续行并入最内项
        let b = Buffer::new("- foo\n  - bar\n    - baz\n\n\n      bim\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), li(false, 1, 1, 2), li(false, 2, 2, 6)]
        );
        // Example 325：外层项续行（缩进 2 = 外层内容列）在内层项闭合后成为续块，
        // 归属外层项签名（无序、depth 0）
        let b = Buffer::new("* foo\n  * bar\n\n  baz\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), li(false, 1, 1, 2), li(false, 0, 3, 4)]
        );
    }

    #[test]
    fn list_item_containers_inside_item() {
        // Example 254：项内容列内的续行/缩进代码/引用全部并入
        let b = Buffer::new(
            "1.  A paragraph\n    with two lines.\n\n        indented code\n\n    > A block quote.\n",
        );
        assert_eq!(kinds(&b), [li(true, 0, 0, 6)]);
        // Example 320：项内引用（含空引用行）
        let b = Buffer::new("* a\n  > b\n  >\n* c\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 3), li(false, 0, 3, 4)]);
        // Example 321：项内引用后接项内围栏
        let b = Buffer::new("- a\n  > b\n  ```\n  c\n  ```\n- d\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 5), li(false, 0, 5, 6)]);
        // Example 300：项内 ATX 与 setext 下划线（达内容列）
        let b = Buffer::new("- # Foo\n- Bar\n  ---\n  baz\n");
        assert_eq!(kinds(&b), [li(false, 0, 0, 1), li(false, 0, 1, 4)]);
    }

    #[test]
    fn list_marker_change_produces_sibling_blocks() {
        // Example 301/302：标记变化在块层各自成块（列表分组由上层按签名归并）
        let b = Buffer::new("- foo\n- bar\n+ baz\n");
        assert_eq!(
            kinds(&b),
            [li(false, 0, 0, 1), li(false, 0, 1, 2), li(false, 0, 2, 3)]
        );
        let b = Buffer::new("1. foo\n2. bar\n3) baz\n");
        assert_eq!(
            kinds(&b),
            [li(true, 0, 0, 1), li(true, 0, 1, 2), li(true, 0, 2, 3)]
        );
        // Example 297：有序项后以内容列内缩进的 bullet 起新项
        let b = Buffer::new("10) foo\n   - bar\n");
        assert_eq!(kinds(&b), [li(true, 0, 0, 1), li(false, 1, 1, 2)]);
    }

    #[test]
    fn blockquote_lazy_continuation() {
        // Example 247：基本懒续行
        let b = Buffer::new("> bar\nbaz\n");
        assert_eq!(kinds(&b), [(BlockKind::BlockQuote, 0, 2)]);
        // Example 233：懒续行后 `>` 行再接续，整段一块
        let b = Buffer::new("> bar\nbaz\n> foo\n");
        assert_eq!(kinds(&b), [(BlockKind::BlockQuote, 0, 3)]);
        // Example 250：嵌套引用的懒续行并入最外层引用块
        let b = Buffer::new("> > > foo\nbar\n");
        assert_eq!(kinds(&b), [(BlockKind::BlockQuote, 0, 2)]);
        // Example 292：项内引用的懒续行
        let b = Buffer::new("> 1. > Blockquote\ncontinued here.\n");
        assert_eq!(kinds(&b), [(BlockKind::BlockQuote, 0, 2)]);
        // Example 238：缩进 ≥4 的行不构成列表标记，懒续行并入
        let b = Buffer::new("> foo\n    - bar\n");
        assert_eq!(kinds(&b), [(BlockKind::BlockQuote, 0, 2)]);
    }

    #[test]
    fn blockquote_lazy_stops_at_interrupters() {
        // ATX / 列表 / 围栏可打断段落 → 不懒续
        let b = Buffer::new("> # foo\nbar\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::BlockQuote, 0, 1), (BlockKind::Paragraph, 1, 2)]
        );
        let b = Buffer::new("> a\n- b\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::BlockQuote, 0, 1), li(false, 0, 1, 2)]
        );
        let b = Buffer::new("> a\n```\nx\n```\n");
        assert_eq!(
            kinds(&b),
            [
                (BlockKind::BlockQuote, 0, 1),
                (BlockKind::FencedCode(None), 1, 4)
            ]
        );
        // Example 93：setext 下划线可懒续（不打断段落），并入引用块
        let b = Buffer::new("> foo\nbar\n===\n");
        assert_eq!(kinds(&b), [(BlockKind::BlockQuote, 0, 3)]);
        // 空行终止引用（懒续行不跨空行）
        let b = Buffer::new("> a\n\nb\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::BlockQuote, 0, 1), (BlockKind::Paragraph, 2, 3)]
        );
    }

    #[test]
    fn blockquote_marker_max_indent_3() {
        // Example 231：4 空格缩进的 `>` 不是引用（缩进代码块）
        let b = Buffer::new("    > # Foo\n    > bar\n");
        assert_eq!(kinds(&b), [(BlockKind::IndentedCode, 0, 2)]);
        // 3 空格合法
        let b = Buffer::new("   > foo\n");
        assert_eq!(kinds(&b), [(BlockKind::BlockQuote, 0, 1)]);
    }

    // ===== Setext 标题（0c P0-2）：识别边界 =====
    //
    // CommonMark 0.31.2 §Setext headings：内容行（段落）+ 下划线行（`=+` → 1 级、
    // `-+` → 2 级，≥1 字符，前导 ≤3 空格，字符间无空白，允许尾随空白）。
    // 关键优先级：段落续行位置 `---` 是下划线而非分隔线；块起始/列表项/引用块后
    // 仍是分隔线；`***`/`___` 无 setext 含义。

    #[test]
    fn setext_basic_levels() {
        let b = Buffer::new("Foo\n===\n");
        assert_eq!(kinds(&b), [(BlockKind::SetextHeading(1), 0, 2)]);
        let b = Buffer::new("Foo\n---\n");
        assert_eq!(kinds(&b), [(BlockKind::SetextHeading(2), 0, 2)]);
        // 下划线 ≥1 字符即合法（Example 83）
        let b = Buffer::new("Foo\n=\n");
        assert_eq!(kinds(&b), [(BlockKind::SetextHeading(1), 0, 2)]);
        // 单个 `-`：空列表项不打断段落，下划线胜出
        let b = Buffer::new("Foo\n-\n");
        assert_eq!(kinds(&b), [(BlockKind::SetextHeading(2), 0, 2)]);
    }

    #[test]
    fn setext_multiline_content() {
        // Example 95：多行内容整体成标题
        let b = Buffer::new("Foo\nBar\n---\n");
        assert_eq!(kinds(&b), [(BlockKind::SetextHeading(2), 0, 3)]);
    }

    #[test]
    fn setext_underline_indent_and_trailing_ws() {
        // Example 86：≤3 空格前导 + 尾随空白合法
        let b = Buffer::new("Foo\n   ----   \n");
        assert_eq!(kinds(&b), [(BlockKind::SetextHeading(2), 0, 2)]);
        // Example 84：内容行/下划线各带 ≤3 缩进
        let b = Buffer::new("  Foo\n  ===\n");
        assert_eq!(kinds(&b), [(BlockKind::SetextHeading(1), 0, 2)]);
    }

    #[test]
    fn setext_underline_four_indent_is_continuation() {
        // Example 87：4 空格缩进的 `---` 既非下划线也非分隔线（缩进代码不打断段落）
        let b = Buffer::new("Foo\n    ---\n");
        assert_eq!(kinds(&b), [(BlockKind::Paragraph, 0, 2)]);
    }

    #[test]
    fn setext_underline_no_internal_blanks() {
        // Example 88：`= =` 非法下划线 → 段落续行
        let b = Buffer::new("Foo\n= =\n");
        assert_eq!(kinds(&b), [(BlockKind::Paragraph, 0, 2)]);
        // `--- -` 非法下划线但合法分隔线（字符间夹空白只放行分隔线规则）
        let b = Buffer::new("Foo\n--- -\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), (BlockKind::ThematicBreak, 1, 2)]
        );
    }

    #[test]
    fn setext_needs_paragraph_immediately_above() {
        // Example 97/104：空行隔开后 `===` 是段落、`---` 是分隔线
        let b = Buffer::new("Foo\n\n===\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), (BlockKind::Paragraph, 2, 3)]
        );
        let b = Buffer::new("Foo\n\n---\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), (BlockKind::ThematicBreak, 2, 3)]
        );
    }

    #[test]
    fn setext_not_after_list_or_quote() {
        // Example 94/99：列表项后的 `---` 是分隔线
        let b = Buffer::new("- Foo\n---\n");
        assert_eq!(
            kinds(&b),
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
                (BlockKind::ThematicBreak, 1, 2)
            ]
        );
        // Example 92/101：引用块后的 `---` 是分隔线
        let b = Buffer::new("> Foo\n---\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::BlockQuote, 0, 1), (BlockKind::ThematicBreak, 1, 2)]
        );
    }

    #[test]
    fn setext_escaped_underline_is_paragraph() {
        // Example 106：`\---` 是转义文本行
        let b = Buffer::new("Foo\nbar\n\\---\nbaz\n");
        assert_eq!(kinds(&b), [(BlockKind::Paragraph, 0, 4)]);
    }

    #[test]
    fn setext_crlf_and_trailing_content_ws() {
        let b = Buffer::new("Foo\r\n---\r\n");
        assert_eq!(kinds(&b), [(BlockKind::SetextHeading(2), 0, 2)]);
        // Example 89：内容行尾随两空格无碍
        let b = Buffer::new("Foo  \n-----\n");
        assert_eq!(kinds(&b), [(BlockKind::SetextHeading(2), 0, 2)]);
    }

    #[test]
    fn setext_stars_and_underscores_are_thematic_breaks() {
        // Example 105：`* * *`/`***`/`___` 无 setext 含义，仍是分隔线
        let b = Buffer::new("Foo\n* * *\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::Paragraph, 0, 1), (BlockKind::ThematicBreak, 1, 2)]
        );
    }

    #[test]
    fn setext_sequence_after_initial_rule() {
        // Example 96：`---` 块起始为分隔线，其后 Foo/Bar 各自成 2 级标题
        let b = Buffer::new("---\nFoo\n---\nBar\n---\nBaz\n");
        assert_eq!(
            kinds(&b),
            [
                (BlockKind::ThematicBreak, 0, 1),
                (BlockKind::SetextHeading(2), 1, 3),
                (BlockKind::SetextHeading(2), 3, 5),
                (BlockKind::Paragraph, 5, 6)
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
        assert_eq!(kinds(&b), [(BlockKind::FencedCode(None), 0, 3)]);
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

    // ===== 链接引用定义（0c P0-1）：识别边界 =====

    #[test]
    fn link_ref_def_basic_forms() {
        // 基本形态：单行 1 块 1 行
        let b = Buffer::new("[foo]: /url\n");
        assert_eq!(kinds(&b), [(BlockKind::LinkRefDef, 0, 1)]);
        // title 三种引号同行
        for doc in [
            "[foo]: /url \"t\"\n",
            "[foo]: /url 't'\n",
            "[foo]: /url (t)\n",
        ] {
            let b = Buffer::new(doc);
            assert_eq!(kinds(&b), [(BlockKind::LinkRefDef, 0, 1)], "doc={doc:?}");
        }
        // `<…>` dest、可空；`:` 后零空白
        let b = Buffer::new("[foo]:<bar baz>\n");
        assert_eq!(kinds(&b), [(BlockKind::LinkRefDef, 0, 1)]);
        let b = Buffer::new("[foo]: <>\n");
        assert_eq!(kinds(&b), [(BlockKind::LinkRefDef, 0, 1)]);
    }

    #[test]
    fn link_ref_def_multiline() {
        // dest/title 各允许一个换行：`:` 后换行 + title 独占行
        let b = Buffer::new("[foo]:\n/url\n\"title\"\nafter\n");
        assert_eq!(
            kinds(&b),
            [
                (BlockKind::LinkRefDef, 0, 3),
                (BlockKind::Paragraph, 3, 4)
            ]
        );
    }

    #[test]
    fn link_ref_def_invalid_cases_fall_back_to_paragraph() {
        for doc in [
            "[foo]: /url \"title\" ok\n", // title 后尾随垃圾（Example 190）
            "[foo]: /url\n\"title\" ok\n", // 换行后引号开头但非纯 title 行（Example 193）
            "[foo]: <bar>(baz)\n",      // title 与 dest 间无空白（Example 201）
            "[foo]:\n",                 // 缺 dest
            "[foo]: /url \"unclosed\n", // title 未闭合
            "[]: /url\n",               // 空 label
            "[foo]: /url(baz\n",        // 裸 dest 括号不平衡
        ] {
            let b = Buffer::new(doc);
            assert!(
                matches!(kinds(&b)[0].0, BlockKind::Paragraph),
                "doc={doc:?} kinds={:?}",
                kinds(&b)
            );
        }
        // 缩进 ≥4 列：定义行落入缩进代码块，不注册定义
        assert_eq!(
            kinds(&Buffer::new("    [foo]: /url\n")),
            [(BlockKind::IndentedCode, 0, 1)]
        );
    }

    #[test]
    fn link_ref_def_then_text_becomes_paragraph() {
        // Example 192：定义被吸收，后续文本行成段落
        let b = Buffer::new("[foo]: /url\nbar\n");
        assert_eq!(
            kinds(&b),
            [(BlockKind::LinkRefDef, 0, 1), (BlockKind::Paragraph, 1, 2)]
        );
        // 连续定义各自成块
        let b = Buffer::new("[a]: /1\n[b]: /2\ntext\n");
        assert_eq!(
            kinds(&b),
            [
                (BlockKind::LinkRefDef, 0, 1),
                (BlockKind::LinkRefDef, 1, 2),
                (BlockKind::Paragraph, 2, 3)
            ]
        );
    }

    #[test]
    fn link_ref_def_does_not_interrupt_paragraph() {
        // 段落中的定义行是普通文本（定义只识别于块起始）
        let b = Buffer::new("foo\n[bar]: /url\n");
        assert_eq!(kinds(&b), [(BlockKind::Paragraph, 0, 2)]);
    }

    // ===== 缩进代码块 / 围栏边界（0c P1） =====

    #[test]
    fn indented_code_basic_block() {
        let b = Buffer::new("    a simple\n      indented code block\n");
        assert_eq!(kinds(&b), [(BlockKind::IndentedCode, 0, 2)]);
    }

    #[test]
    fn indented_code_blank_lines_continue_and_trailing_trimmed() {
        // 空行暂存：后续 ≥4 列行续块；末尾空行不计入块区间
        let b = Buffer::new("    chunk1\n\n    chunk2\n\n");
        assert_eq!(kinds(&b), [(BlockKind::IndentedCode, 0, 3)]);
        // 纯空白行同样视为空行
        let b = Buffer::new("    chunk1\n  \n    chunk2\n");
        assert_eq!(kinds(&b), [(BlockKind::IndentedCode, 0, 3)]);
    }

    #[test]
    fn indented_code_cannot_interrupt_paragraph() {
        // Example 70：4 空格缩进的行是段落续行，而非 ATX 标题
        let b = Buffer::new("foo\n    # bar\n");
        assert_eq!(kinds(&b), [(BlockKind::Paragraph, 0, 2)]);
    }

    #[test]
    fn indented_code_tab_stops() {
        // \t 从列 0 推进到列 4：与 4 空格等价
        assert_eq!(kinds(&Buffer::new("\tfoo\n\tbar\n")), [(BlockKind::IndentedCode, 0, 2)]);
        // 2 空格 + \t 同样到列 4
        assert_eq!(kinds(&Buffer::new("  \tfoo\n")), [(BlockKind::IndentedCode, 0, 1)]);
        // 4 空格 + \t 的续行
        assert_eq!(kinds(&Buffer::new("    foo\n\tbar\n")), [(BlockKind::IndentedCode, 0, 2)]);
    }

    #[test]
    fn indented_code_precedes_block_markers() {
        // Example 48/69：缩进的分隔线/ATX 是代码，不是块标记
        assert_eq!(kinds(&Buffer::new("    ***\n")), [(BlockKind::IndentedCode, 0, 1)]);
        assert_eq!(kinds(&Buffer::new("    # foo\n")), [(BlockKind::IndentedCode, 0, 1)]);
    }

    #[test]
    fn tilde_fence_supported() {
        assert_eq!(
            kinds(&Buffer::new("~~~\naaa\n~~~\n")),
            [(BlockKind::FencedCode(None), 0, 3)]
        );
        // 闭合围栏须同字符：``` 不闭合 ~~~ 围栏
        assert_eq!(
            kinds(&Buffer::new("~~~\naaa\n```\n~~~\n")),
            [(BlockKind::FencedCode(None), 0, 4)]
        );
    }

    #[test]
    fn fence_close_requires_length_and_whitespace_tail() {
        // 闭合 ≥ 开围栏长度（3 个 ~~~ 不闭合 4 个 ~~~~）
        assert_eq!(
            kinds(&Buffer::new("~~~~\naaa\n~~~\n~~~~\n")),
            [(BlockKind::FencedCode(None), 0, 4)]
        );
        // 闭合标记后只允许空白：` ``` aaa ` 是内容（Example 147）
        assert_eq!(
            kinds(&Buffer::new("```\n``` aaa\n```\n")),
            [(BlockKind::FencedCode(None), 0, 3)]
        );
        // 闭合缩进 >3 列不合法（Example 137）：围栏未闭合，吞咽至 EOF（含尾空行）
        assert_eq!(
            kinds(&Buffer::new("```\naaa\n    ```\n")),
            [(BlockKind::FencedCode(None), 0, 4)]
        );
        // 闭合缩进 ≤3 列合法（Example 135）
        assert_eq!(
            kinds(&Buffer::new("```\naaa\n  ```\n")),
            [(BlockKind::FencedCode(None), 0, 3)]
        );
    }

    #[test]
    fn fence_info_backticks_invalid_for_backtick_fence() {
        // Example 138：``` ``` 的 info 含反引号 → 段落
        assert_eq!(kinds(&Buffer::new("``` ```\naaa\n")), [(BlockKind::Paragraph, 0, 2)]);
        // Example 145：``` aa ``` 同理
        assert_eq!(kinds(&Buffer::new("``` aa ```\nfoo\n")), [(BlockKind::Paragraph, 0, 2)]);
        // ~~~ 围栏的 info 可含反引号（Example 146）
        assert_eq!(
            kinds(&Buffer::new("~~~ aa ``` ~~~\nfoo\n~~~\n")),
            [(BlockKind::FencedCode(Some("aa".to_string())), 0, 3)]
        );
    }

    #[test]
    fn fence_requires_three_markers() {
        // `` 只有两个反引号：不是围栏
        assert!(!is_fence("``"));
        assert_eq!(kinds(&Buffer::new("``\nfoo\n``\n")), [(BlockKind::Paragraph, 0, 3)]);
    }
}
