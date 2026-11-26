//! inline 解析（v0 子集 + CommonMark delimiter-run 强调，0c P1 起）。
//
// 语义取舍（逐条钉在测试上）：
// - 两阶段强调：段内扫描产出原子节点（code/math/link/image/strikethrough）与
//   `*`/`_` 分隔符 run，随后按 CommonMark delimiter-run 规则匹配为 Emph/Strong
//   （左右 flanking、`_` intraword 限制、rule of 3、run 按 1/2 拆分自内容侧消费、
//   闭标记左→右就近回溯匹配即嵌套）。强调之外的构造仍递归下降。
// - 空内容不成节点（`****` 为纯文本）；未匹配分隔符落回纯文本。
// - Code/Math 内容原样不嵌套；Math 要求内容非空、首尾非空格、闭 `$` 后非数字
//   （pandoc 规则，避免 `$5 and $6` 误判为公式）。
// - Link 文本可嵌套除 Link 外的任意 inline；URL 扫至首个 `)`，不支持 title。
//   引用式（0c P0-1）：`[text][label]` / `[text][]` / `[text]` shortcut 查文档引用表，
//   节点的 url 指向定义处 dest 的绝对区间；查无定义则整体按纯文本（内部 inline 正常解析）。
//   优先级：inline 链接 > full/collapsed 引用 > shortcut（与 CommonMark 一致）。
// - Image 语法同 Link，alt 允许为空；URL 可空（`![]()` 合法，视图侧无加载键则只显
//   alt）。Image 可出现在 Link 文本内（徽章写法）；alt 为纯文本，不产生子节点
//   （CommonMark：alt = 纯文本内容）。引用式图片 `![alt][label]` / `![alt][]` / `![alt]`
//   同引用式链接（0c P0-1）。
// - Strikethrough（GFM `~~`）仍为朴素最近配对，不参与 delimiter-run（GFM 语料另计）。
// - 裸链接（GFM 自动链接扩展，W1-5）：`scheme://` / `www.` URL 与 `local@domain`
//   邮箱在解析后追加 Link 节点（url 指向源码本身，协议/`mailto:` 由消费侧补全）；
//   尾部标点、`&entity;`、未配对 `)` 按 GFM 规则排除，代码/显式链接/尖括号内不接管。
// - `\` 转义全部 ASCII 标点（CommonMark；0c P1 起由 v0 白名单扩展）。
// - Unicode 标点判定：ASCII 精确命中规范集合；非 ASCII 以「非字母数字且非空白」
//   近似 P/S 类（零依赖取舍：CJK 标点、货币符号均正确，仅 Mn/Cf 等记号类字符
//   邻接 run 时存在理论偏差）。flanking 的空白判定用 char::is_whitespace，
//   比规范的 Zs+\t\n\f\r 多出 \v/NEL，从宽。
// - 递归深度硬上限 MAX_INLINE_DEPTH（B5：超深嵌套不得栈溢出）；分隔符匹配为
//   迭代实现，不受递归上限影响。

use super::block::{normalize_label, RefMap};
use super::model::{HtmlAttrs, HtmlTag, ImageAlign, InlineKind, InlineNode};
use crate::range::Range;

/// inline 解析递归深度硬上限。超限后开标记按纯文本处理；
/// 16 层远超正常写作嵌套，仅作恶意语料保险丝。
const MAX_INLINE_DEPTH: usize = 16;

/// 解析一段文本的 inline 节点，偏移以 `base` 为基准（文档字节偏移）。
pub(super) fn parse_inlines(text: &str, base: usize, refs: &RefMap) -> Vec<InlineNode> {
    let nodes = InlineParser {
        s: text,
        base,
        refs,
        pos: 0,
        bracket_pairs: Vec::new(),
        resolved_prefix: Vec::new(),
        backtick_runs: Vec::new(),
    }
    .parse_until(None, true, 0);
    link_bare_urls(text, base, nodes)
}

/// 裸 URL / 裸邮箱自动成链（GFM 自动链接扩展，视图态增强）。
/// URL 前缀为任意 `scheme://`（scheme = `[A-Za-z][A-Za-z0-9+.-]{1,31}`）或 `www.`；
/// 邮箱为 `local@domain`（domain 至少一个点、末字符为字母数字）。
/// 扫描到空白/`<` 为止，再按 GFM 规则剥尾部标点（`&entity;` 整体排除、
/// 尾部 `)` 按括号收支剥离），跳过已由 Code/Link/Autolink 覆盖的区间。
/// 不修改源码字节（协议补全在消费侧完成）。
fn link_bare_urls(text: &str, base: usize, mut nodes: Vec<InlineNode>) -> Vec<InlineNode> {
    link_bare_urls_in(text, base, &mut nodes);
    nodes.sort_by_key(|n| n.range.start);
    nodes
}

/// 单层裸链接扫描；Emph/Strong/Strikethrough 先递归处理内容区
/// （其 `content` 为内层文本的绝对区间），容器自身区间在父层已计入覆盖。
fn link_bare_urls_in(text: &str, base: usize, nodes: &mut Vec<InlineNode>) {
    for n in nodes.iter_mut() {
        if matches!(
            n.kind,
            InlineKind::Emph | InlineKind::Strong | InlineKind::Strikethrough
        ) {
            let c = n.content;
            let sub = &text[(c.start - base)..(c.end - base)];
            link_bare_urls_in(sub, c.start, &mut n.children);
        }
    }
    // 本层已覆盖区间（[start, end) 绝对偏移）
    let mut covered: Vec<(usize, usize)> =
        nodes.iter().map(|n| (n.range.start, n.range.end)).collect();
    covered.sort_unstable();

    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            if let Some((s, e)) = scan_bare_email(bytes, i) {
                if !is_covered(&covered, base + s) && !inside_angle(bytes, s) {
                    push_bare_link(nodes, &mut covered, base, s, e);
                    i = e;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        let Some(prefix) = url_prefix(bytes, i) else {
            i += 1;
            continue;
        };
        // 词内不接管（`xhttp://` 不是自动链接）；CJK 等非 ASCII 前缀照常成链
        if !at_token_boundary(bytes, i) {
            i += 1;
            continue;
        }
        let abs_start = base + i;
        if is_covered(&covered, abs_start) || inside_angle(bytes, i) {
            i += 1;
            continue;
        }
        let j = url_end(text, i, prefix);
        if j > i + prefix {
            push_bare_link(nodes, &mut covered, base, i, j);
        }
        i = j.max(i + 1);
    }
}

/// 位置是否落在既有节点（Code/Link/Autolink 等）覆盖的区间内。
fn is_covered(covered: &[(usize, usize)], abs: usize) -> bool {
    covered.iter().any(|&(s, e)| s <= abs && abs < e)
}

/// 位置是否处于未闭合的 `<...>` 内（CommonMark autolink / 原始 HTML 标签）：
/// 向前扫描 ≤50 字节，跳过成对的 `>`…`<`，若存在未匹配的 `<` 说明仍在尖括号
/// 作用域。裸链接不应从这里抢链接——由 try_autolink/HTML 逻辑接管，可能因
/// 空格/非法字符拒绝，拒绝后该位置应保持纯文本。注意 `<` 本身允许出现在 URL
/// 中作为终止符，故仅在当前位置之前判定。
fn inside_angle(bytes: &[u8], pos: usize) -> bool {
    let mut depth = 0i32;
    let mut checked = 0usize;
    let mut k = pos;
    while k > 0 && checked < 50 {
        k -= 1;
        checked += 1;
        match bytes[k] {
            // 反向扫描：先遇到闭合 `>` 说明该尖括号区间在当前位置之前结束，
            // 跳过它与其开 `<`；只有孤立未配对的 `<` 才表示当前位置在尖括号内。
            b'>' => depth += 1,
            b'<' => {
                if depth == 0 {
                    return true;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    false
}

/// 裸 URL 前缀长度（`www.` 或 `scheme://`）。scheme 规则同 CommonMark：
/// `[A-Za-z][A-Za-z0-9+.-]{1,31}`，故总长 2..=32 后接 `://`。
fn url_prefix(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes[i..].starts_with(b"www.") {
        return Some(4);
    }
    if !bytes[i].is_ascii_alphabetic() {
        return None;
    }
    let mut k = i + 1;
    while k < bytes.len()
        && k - i < 32
        && (bytes[k].is_ascii_alphanumeric() || matches!(bytes[k], b'+' | b'.' | b'-'))
    {
        k += 1;
    }
    if !(2..=32).contains(&(k - i)) || !bytes[k..].starts_with(b"://") {
        return None;
    }
    Some(k - i + 3)
}

/// 自动链接的左边界：不能紧贴字母数字或 scheme 字符（否则只是普通单词的一部分）。
fn at_token_boundary(bytes: &[u8], i: usize) -> bool {
    i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || matches!(bytes[i - 1], b'+' | b'.' | b'-'))
}

/// URL 终点：扫到空白/控制符/`<` 或 CJK 标点（`。`、`、`、全角括号等）为止，
/// 随后按 GFM 规则剥除尾部：
/// - `?!.,:*_~'\"` 反复剥；
/// - 尾部 `&name;`（name 全字母）整体排除；
/// - 尾部 `)` 按全串括号收支剥离（`(business))+ok` 的 `)` 中段不受影响）。
fn url_end(text: &str, i: usize, prefix: usize) -> usize {
    let bytes = text.as_bytes();
    let mut j = i + prefix;
    while j < bytes.len() {
        let b = bytes[j];
        if b.is_ascii_whitespace() || b < 0x20 || b == b'<' {
            break;
        }
        if b < 0x80 {
            j += 1;
            continue;
        }
        // 非 ASCII 按字符推进：CJK 标点是句子边界，不属于裸 URL。
        let c = text[j..].chars().next().unwrap_or('\u{fffd}');
        if is_cjk_url_terminator(c) {
            break;
        }
        j += c.len_utf8();
    }
    while j > i + prefix {
        let lb = bytes[j - 1];
        if matches!(
            lb,
            b'.' | b',' | b':' | b'!' | b'?' | b'*' | b'_' | b'~' | b'\'' | b'"'
        ) {
            j -= 1;
            continue;
        }
        if lb == b';' {
            let mut k = j - 1;
            while k > i + prefix && bytes[k - 1].is_ascii_alphabetic() {
                k -= 1;
            }
            if k < j - 1 && bytes[k - 1] == b'&' {
                j = k - 1; // `&name;` 属实体样式，整体排除
            } else {
                j -= 1;
            }
            continue;
        }
        break;
    }
    let mut opens = 0i32;
    let mut closes = 0i32;
    for &b in &bytes[i..j] {
        match b {
            b'(' => opens += 1,
            b')' => closes += 1,
            _ => {}
        }
    }
    while closes > opens && j > i + prefix && bytes[j - 1] == b')' {
        j -= 1;
        closes -= 1;
    }
    j
}

/// CJK 标点（句子/短语边界）：裸 URL 在此终止，避免把后续中文吞进链接。
/// 覆盖 CJK 符号标点、全角 ASCII 标点、中文引号/省略号/破折号与中点。
fn is_cjk_url_terminator(c: char) -> bool {
    matches!(
        c,
        '\u{3001}'..='\u{3003}'
            | '\u{3008}'..='\u{3011}'
            | '\u{3014}'..='\u{301B}'
            | '\u{301D}'..='\u{301F}'
            | '\u{3030}'
            | '\u{FF01}'..='\u{FF0F}'
            | '\u{FF1A}'..='\u{FF20}'
            | '\u{FF3B}'..='\u{FF40}'
            | '\u{FF5B}'..='\u{FF65}'
            | '\u{2018}'..='\u{201F}'
            | '\u{2013}'..='\u{2014}'
            | '\u{2025}'..='\u{2026}'
            | '\u{00B7}'
            | '\u{30FB}'
    )
}

/// 裸邮箱候选：`at` 指向 `@`。local 反向取 `[alnum .!#$%&'*+/=?^_`{|}~-]`，
/// 首尾须为字母数字；domain 正向取 `[alnum ._-]`，剥句末句点后要求至少一个点、
/// 末字符为字母数字（因此 `a@b.c-` / `a@b.c_` 整体不成链）。
fn scan_bare_email(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    let mut s = at;
    while s > 0 && is_email_local_byte(bytes[s - 1]) {
        s -= 1;
    }
    if s == at || !bytes[s].is_ascii_alphanumeric() || !bytes[at - 1].is_ascii_alphanumeric() {
        return None;
    }
    // scheme 前缀（`mailto:foo@bar.baz`）或路径分隔符后的片段不当作裸邮箱
    if s > 0 && matches!(bytes[s - 1], b':' | b'/' | b'\\' | b'@') {
        return None;
    }
    let mut end = at + 1;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'.' | b'-' | b'_'))
    {
        end += 1;
    }
    while end > at + 1 && bytes[end - 1] == b'.' {
        end -= 1;
    }
    let domain = &bytes[at + 1..end];
    if !domain.contains(&b'.') || !domain[domain.len() - 1].is_ascii_alphanumeric() {
        return None;
    }
    let labels_ok = domain.split(|&b| b == b'.').all(|l| {
        !l.is_empty()
            && l.len() <= 63
            && l[0].is_ascii_alphanumeric()
            && l.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
    });
    labels_ok.then_some((s, end))
}

/// 裸邮箱 local 部分允许的字节（GFM atext + `.`）。
fn is_email_local_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&b)
}

/// 追加裸链接节点并登记覆盖区间（`url` 与 `content` 同指源码本身）。
fn push_bare_link(
    nodes: &mut Vec<InlineNode>,
    covered: &mut Vec<(usize, usize)>,
    base: usize,
    s: usize,
    e: usize,
) {
    let range = Range::new(base + s, base + e);
    nodes.push(InlineNode {
        kind: InlineKind::Link,
        range,
        content: range,
        url: Some(range),
        children: Vec::new(),
    });
    covered.push((range.start, range.end));
}

/// CommonMark：反斜杠可转义一切 ASCII 标点（`!` 为 Image 开标记首字符，亦在内）。
fn is_escapable(c: char) -> bool {
    c.is_ascii_punctuation()
}

/// `text` 中第一个未被反斜杠转义的 `$` 字节偏移。连续反斜杠按奇偶判定
/// （`\\$` 是字面反斜杠 + 定界符，`\$` 是转义美元）。
fn find_unescaped_dollar(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                let mut n = 0usize;
                while i + n < b.len() && b[i + n] == b'\\' {
                    n += 1;
                }
                i += n;
                if i < b.len() && b[i] == b'$' {
                    // 奇数个反斜杠：`$` 被转义，跳过；偶数个：未转义（留待外层循环识别）。
                    if n % 2 == 1 {
                        i += 1;
                        continue;
                    }
                    return Some(i);
                }
                continue;
            }
            b'$' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// URI 自动链接（`<scheme:...>`）：scheme = `[A-Za-z][A-Za-z0-9+.-]{1,31}` + `:`。
/// 例：`<https://../>` / `<made-up-scheme://foo,bar>` 合法，`<m:abc>`（scheme 1 字符）非法。
fn is_uri_autolink(body: &str) -> bool {
    let b = body.as_bytes();
    let Some(colon) = b.iter().position(|&c| c == b':') else {
        return false;
    };
    let scheme = &b[..colon];
    if !(2..=32).contains(&scheme.len()) || !scheme[0].is_ascii_alphabetic() {
        return false;
    }
    scheme[1..]
        .iter()
        .all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
}

/// 邮箱自动链接（`<local@domain>`）：local 允许 `[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]`，
/// domain 为 ≥2 个点分 label（alnum 起止，内部可含 `-`，长度 ≤63）。
fn is_email_autolink(body: &str) -> bool {
    let Some(at) = body.find('@') else {
        return false;
    };
    let (local, domain) = (&body[..at], &body[at + 1..]);
    if local.is_empty()
        || !local
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&c))
    {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| {
            let b = l.as_bytes();
            !b.is_empty()
                && b.len() <= 63
                && b[0].is_ascii_alphanumeric()
                && b[b.len() - 1].is_ascii_alphanumeric()
                && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
        })
}

/// Unicode 标点（flanking 判定用）：ASCII 精确；非 ASCII 近似 P/S 类（见模块头注）。
fn is_unicode_punctuation(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_punctuation()
    } else {
        !c.is_alphanumeric() && !c.is_whitespace()
    }
}

/// 强调分隔符 run（段内字节区间 [start, start+len) 的未消费余量）。
#[derive(Clone, Copy)]
struct Delim {
    chr: u8, // b'*' | b'_'
    start: usize,
    len: usize,
    can_open: bool,
    can_close: bool,
}

/// 段内 token：原子节点（连带本段已产生的强调嵌套深度）或待匹配分隔符 run。
/// 纯文本不产生 token（inline 节点只承载构造）。
// 解析期瞬态 token，避免为 HTML 属性装箱带来每 token 分配。
#[allow(clippy::large_enum_variant)]
enum Tok {
    Node(InlineNode, u8),
    Delim(Delim),
}

/// 单段内强调包裹深度上限。CommonMark 不设限，但视图层（style/layout）递归遍历
/// 行内树：恶意语料（百万 `*`）会产生超深树打爆视图栈。32 远超真实/规范语料
/// （最深 3 层），仅作保险丝；超限的配对按不匹配处理（分隔符落回纯文本）。
const EMPH_MAX_DEPTH: u8 = 32;

/// 阶段二：分隔符匹配为 Emph/Strong。闭标记左→右扫描，就近回溯同字符可开 run；
/// rule of 3：一方可开可闭时两 run 长度和为 3 的倍数则禁止配对（皆为 3 的倍数除外）。
/// 命中时开 run 自内容侧、闭 run 自内容侧各消费 1/2 个字符包裹中间 token。
/// 未匹配 run 收尾时落回纯文本（不产生节点）。
///
/// floor 防退化 O(n²)：按（字符 × 亦可开 × 长度%3）记录失败闭标记的字节位置下限，
/// 同类后续闭标记不回扫下限之前；每次成功匹配后清零（结构已变，旧结论失效）。
fn resolve_emphasis(toks: Vec<Tok>, base: usize) -> Vec<InlineNode> {
    let mut toks = toks;
    let mut floor = [[0usize; 3]; 4];
    let mut i = 0;
    while i < toks.len() {
        let Tok::Delim(c) = toks[i] else {
            i += 1;
            continue;
        };
        if !c.can_close {
            i += 1;
            continue;
        }
        let fidx = usize::from(c.chr == b'_') * 2 + usize::from(c.can_open);
        // 回溯最近同字符可开 run；rule of 3 不合者跳过，继续找更早的
        let mut j = i;
        let opener = loop {
            if j == 0 {
                break None;
            }
            j -= 1;
            let Tok::Delim(o) = toks[j] else {
                continue;
            };
            if o.start < floor[fidx][c.len % 3] {
                break None;
            }
            if o.chr != c.chr || !o.can_open {
                continue;
            }
            let rule3 = (o.can_close || c.can_open)
                && (o.len + c.len) % 3 == 0
                && (o.len % 3 != 0 || c.len % 3 != 0);
            if rule3 {
                continue;
            }
            break Some((j, o));
        };
        let Some((j, o)) = opener else {
            floor[fidx][c.len % 3] = c.start;
            if c.can_open {
                i += 1; // 亦可开：留作后续闭标记的开方候选
            } else {
                toks.remove(i); // 纯闭 run 永不可能匹配，移除防反复回扫
            }
            continue;
        };
        let use_n = if o.len >= 2 && c.len >= 2 { 2 } else { 1 };
        // 深度保险丝：最近开 run 产生最低嵌套深度；它超限则更早开 run（包裹更宽）
        // 必超限 → 直接等同无开标记处理
        let nest = 1 + toks[j + 1..=i]
            .iter()
            .map(|t| match t {
                Tok::Node(_, d) => *d,
                Tok::Delim(_) => 0,
            })
            .max()
            .unwrap_or(0);
        if nest > EMPH_MAX_DEPTH {
            floor[fidx][c.len % 3] = c.start;
            if c.can_open {
                i += 1;
            } else {
                toks.remove(i);
            }
            continue;
        }
        // 摘除 j..=i；children 为中间 token 的节点（分隔符 run 落为纯文本）
        let removed: Vec<Tok> = toks.splice(j..=i, []).collect();
        let mid = removed.len() - 2; // 去掉首尾的开/闭 run 自身
        let children = removed
            .into_iter()
            .skip(1)
            .take(mid)
            .filter_map(|t| match t {
                Tok::Node(n, _) => Some(n),
                Tok::Delim(_) => None,
            })
            .collect();
        let node = InlineNode {
            kind: if use_n == 2 {
                InlineKind::Strong
            } else {
                InlineKind::Emph
            },
            range: Range::new(base + o.start + o.len - use_n, base + c.start + use_n),
            content: Range::new(base + o.start + o.len, base + c.start),
            url: None,
            children,
        };
        let mut seg = Vec::with_capacity(3);
        if o.len > use_n {
            seg.push(Tok::Delim(Delim {
                len: o.len - use_n,
                ..o
            }));
        }
        seg.push(Tok::Node(node, nest));
        let closer_left = c.len - use_n;
        if closer_left > 0 {
            seg.push(Tok::Delim(Delim {
                start: c.start + use_n,
                len: closer_left,
                ..c
            }));
        }
        let seg_len = seg.len();
        toks.splice(j..j, seg);
        floor = [[0; 3]; 4];
        // 闭 run 有余量：继续以它为闭标记匹配更早开 run；否则越过插入片段
        i = if closer_left > 0 {
            j + seg_len - 1
        } else {
            j + seg_len
        };
    }
    toks.into_iter()
        .filter_map(|t| match t {
            Tok::Node(n, _) => Some(n),
            Tok::Delim(_) => None,
        })
        .collect()
}

/// 全角圆括号（Chinese IME 常把图片语法写成 `![a]（p.png）`）：按字节比较，
/// 任意位置切片都安全（不会切在多字节字符内部）。
const FW_OPEN: &[u8] = "（".as_bytes();
const FW_CLOSE: &[u8] = "）".as_bytes();

/// 解析 `](` 后的链接目标与可选 title（CommonMark §6.3）。
/// 返回 `(url_start, url_end, end)`：url 区间为内容（不含尖括号），end 为 `)` 之后。
/// 目标：`<...>`（无换行/未转义尖括号）或裸目标（无空格/换行/控制字符，括号须平衡）。
/// title：`"..."` / `'...'` / `(...)`（无换行，未转义闭标记结束）。
/// `image=true` 时闭定界符额外接受全角 `）`（可与半角 `)` 混用），目标内的
/// `（`/`）` 同样参与括号平衡；链接仍严格只认半角。
fn parse_link_dest(s: &str, start: usize, image: bool) -> Option<(usize, usize, usize)> {
    let b = s.as_bytes();
    let mut pos = start;
    // 跳过前导空白（含换行——CommonMark 允许组件间空白含换行）
    while pos < b.len() && matches!(b[pos], b' ' | b'\t' | b'\n' | b'\r') {
        pos += 1;
    }
    let (url_start, url_end);
    if pos < b.len() && b[pos] == b'<' {
        // 尖括号目标
        let cs = pos + 1;
        let mut i = cs;
        loop {
            if i >= b.len() {
                return None;
            }
            match b[i] {
                b'>' => {
                    let inner = &s[cs..i];
                    if inner.contains('\n') || inner.contains('\r') {
                        return None;
                    }
                    url_start = cs;
                    url_end = i;
                    pos = i + 1;
                    break;
                }
                b'\\' if i + 1 < b.len() => i += 2,
                b'<' | b'\n' | b'\r' => return None,
                _ => i += 1,
            }
        }
    } else {
        // 裸目标
        let mut depth: u32 = 0;
        let mut i = pos;
        loop {
            if i >= b.len() {
                return None; // 无 `)` 闭合
            }
            if b[i] == b')' {
                if depth == 0 {
                    url_start = pos;
                    url_end = i;
                    pos = i;
                    break;
                }
                depth -= 1;
                i += 1;
            } else if b[i] == b'(' {
                depth += 1;
                i += 1;
            } else if image && b[i..].starts_with(FW_CLOSE) {
                if depth == 0 {
                    url_start = pos;
                    url_end = i;
                    pos = i;
                    break;
                }
                depth -= 1;
                i += FW_CLOSE.len();
            } else if image && b[i..].starts_with(FW_OPEN) {
                depth += 1;
                i += FW_OPEN.len();
            } else if b[i] == b'\\' && i + 1 < b.len() {
                i += 2;
            } else if matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
                url_start = pos;
                url_end = i;
                pos = i;
                break;
            } else if b[i] <= 0x1f || b[i] == 0x7f {
                return None;
            } else {
                i += 1;
            }
        }
        if depth > 0 {
            return None; // 括号未平衡
        }
    }
    // 跳过目标后空白（含换行）
    while pos < b.len() && matches!(b[pos], b' ' | b'\t' | b'\n' | b'\r') {
        pos += 1;
    }
    // 可选 title
    if pos < b.len() && matches!(b[pos], b'"' | b'\'' | b'(') {
        let close = match b[pos] {
            b'"' => b'"',
            b'\'' => b'\'',
            _ => b')',
        };
        pos += 1;
        loop {
            if pos >= b.len() {
                return None;
            }
            match b[pos] {
                b'\\' if pos + 1 < b.len() => pos += 2,
                b'\n' | b'\r' => return None,
                c if c == close => {
                    pos += 1;
                    break;
                }
                _ => pos += 1,
            }
        }
        while pos < b.len() && matches!(b[pos], b' ' | b'\t' | b'\n' | b'\r') {
            pos += 1;
        }
    }
    if pos < b.len() && b[pos] == b')' {
        Some((url_start, url_end, pos + 1))
    } else if image && b[pos..].starts_with(FW_CLOSE) {
        Some((url_start, url_end, pos + FW_CLOSE.len()))
    } else {
        // 旧版 Veil 插入的目录锚点含空格（`(#第三章 - 结论)`、`(#11- 背景)`）：
        // 标准裸目标在空白处即终止，整条链接会退化成纯文本。仅对同文档锚点
        // （目标以 `#` 开头）放宽空白，其它目标/危险 scheme 保持严格解析。
        parse_fragment_dest(s, start, image)
    }
}

/// 图片目标的空白容错（仅全角定界符场景）：标准裸目标在空白处即止，
/// `![图]（未命名 图.png）` 会整段退化成文字。这里取到首个未嵌套的闭定界符
/// （`）` 或 `)`），允许目标含空格/制表符，首尾空白裁掉；跨行仍不识别。
fn parse_image_dest_spaces(s: &str, start: usize) -> Option<(usize, usize, usize)> {
    let b = s.as_bytes();
    let mut depth: u32 = 0;
    let mut i = start;
    while i < b.len() {
        if b[i] == b')' {
            if depth == 0 {
                let (a, z) = trim_dest_span(b, start, i);
                return Some((a, z, i + 1));
            }
            depth -= 1;
            i += 1;
        } else if b[i] == b'(' {
            depth += 1;
            i += 1;
        } else if b[i..].starts_with(FW_CLOSE) {
            if depth == 0 {
                let (a, z) = trim_dest_span(b, start, i);
                return Some((a, z, i + FW_CLOSE.len()));
            }
            depth -= 1;
            i += FW_CLOSE.len();
        } else if b[i..].starts_with(FW_OPEN) {
            depth += 1;
            i += FW_OPEN.len();
        } else if b[i] == b'\\' && i + 1 < b.len() {
            i += 2;
        } else if matches!(b[i], b'\n' | b'\r') || (b[i] < 0x20 && b[i] != b'\t') {
            return None;
        } else {
            i += 1;
        }
    }
    None
}

/// 目标区间首尾的空白/制表符裁除（字节偏移）。
fn trim_dest_span(b: &[u8], start: usize, end: usize) -> (usize, usize) {
    let mut a = start;
    let mut z = end;
    while a < z && matches!(b[a], b' ' | b'\t') {
        a += 1;
    }
    while z > a && matches!(b[z - 1], b' ' | b'\t') {
        z -= 1;
    }
    (a, z)
}

/// 同文档锚点容错目标：`#` 起头、允许空白、括号须平衡、不得跨行。
/// 只放宽空白与括号——`#` 片段不会交给系统 shell，安全面与标准解析一致。
fn parse_fragment_dest(s: &str, start: usize, image: bool) -> Option<(usize, usize, usize)> {
    let b = s.as_bytes();
    let mut pos = start;
    while pos < b.len() && matches!(b[pos], b' ' | b'\t') {
        pos += 1;
    }
    if b.get(pos) != Some(&b'#') {
        return None;
    }
    let url_start = pos;
    let mut depth: u32 = 0;
    let mut i = pos;
    loop {
        if i >= b.len() {
            return None; // 无 `)` 闭合
        }
        if b[i] == b')' {
            if depth == 0 {
                return Some((url_start, i, i + 1));
            }
            depth -= 1;
            i += 1;
        } else if b[i] == b'(' {
            depth += 1;
            i += 1;
        } else if image && b[i..].starts_with(FW_CLOSE) {
            if depth == 0 {
                return Some((url_start, i, i + FW_CLOSE.len()));
            }
            depth -= 1;
            i += FW_CLOSE.len();
        } else if image && b[i..].starts_with(FW_OPEN) {
            depth += 1;
            i += FW_OPEN.len();
        } else if b[i] == b'\\' && i + 1 < b.len() {
            i += 2;
        } else if matches!(b[i], b'\n' | b'\r') || (b[i] < 0x20 && b[i] != b'\t') {
            return None;
        } else {
            i += 1;
        }
    }
}

/// 代码段闭 run：从 `open`（开 run 起点，长 `len`）之后寻找等长反引号 run，
/// 返回其结束位置；空内容（闭 run 紧贴开 run）与无等长 run 均返回 None。
/// 供链接括号配对扫描跳过代码段（段内方括号不参与配对）。
fn code_span_close(s: &str, open: usize, len: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = open + len;
    while i < b.len() {
        if b[i] == b'`' {
            let run = s[i..].bytes().take_while(|&c| c == b'`').count();
            if run == len {
                // 与 try_code_span 一致：空内容不成代码段，且不再向后找
                return (i > open + len).then_some(i + run);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// 行内 HTML 标签/自动链接的结束位置（`>` 之后）：`lt` 处 `<` 后须接字母或 /!?，
/// 且其后存在 `>`；否则返回 None（按普通字符处理）。
fn html_tag_end(s: &str, lt: usize) -> Option<usize> {
    let b = s.as_bytes();
    let tag_like = matches!(b.get(lt + 1), Some(c) if c.is_ascii_alphabetic()
        || *c == b'/' || *c == b'!' || *c == b'?');
    if !tag_like {
        return None;
    }
    // 引号内的 `>`（如 `style="font-family:'a>b'"`）不是标签结束；找不到合法结束
    // 则回退到第一个 `>`（保持历史容错行为，不把整段吞掉）。
    let mut quote = 0u8;
    let mut i = lt + 1;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return Some(i + 1);
        }
        i += 1;
    }
    s[lt..].find('>').map(|rel| lt + rel + 1)
}

/// 显式尺寸属性值 → 像素数：只接受纯十进制数字（可带首尾空白），且必须 > 0。
/// `0` / 空串 / `100px` / `-5` / 溢出 u32 的长数字都视为非法（返回 None，属性忽略）。
fn parse_px(v: Option<&str>) -> Option<u32> {
    let t = v?.trim();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse::<u32>().ok().filter(|n| *n > 0)
}

/// 从一段开标签源码（如 `<abbr title="x">`）中抽取标签名、是否自闭合、白名单属性。
/// 标签名必须以字母开头、后接 alnum/-；不满足返回 None（不是合法标签）。
/// 属性名必须以字母开头；白名单收集 `title`/`src`/`alt`（`<a>` 另收 `href`、
/// `<img>` 另收 `width`/`height`）与净化后的 `style`；`on*` 一律忽略；
/// `src`/`href` 统一经 [`crate::link_url_is_safe`] 过滤，非白名单 scheme 置空。
/// 属性值支持 `"..."` / `'...'` / 无引号。
fn parse_open_tag(src: &str) -> Option<(String, bool, HtmlAttrs)> {
    // src 形如 "kbd>" 或 "img src=\"x\" />"（不含 `<`）。须以字母开头。
    let bytes = src.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    // 标签名：字母 + alnum + -
    let mut tn = 1;
    while tn < bytes.len() && (bytes[tn].is_ascii_alphanumeric() || bytes[tn] == b'-') {
        tn += 1;
    }
    let tag_name = src[..tn].to_ascii_lowercase();
    let rest = &src[tn..];
    let rb = rest.as_bytes();
    let mut i = 0;
    let mut attrs = HtmlAttrs::default();
    let mut self_closed = false;
    'outer: while i < rb.len() {
        // 跳过前导空白
        while i < rb.len() && matches!(rb[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        if i >= rb.len() {
            break;
        }
        if rb[i] == b'/' {
            // 自闭合标记：`/>`
            if rb.get(i + 1) == Some(&b'>') {
                self_closed = true;
                break;
            }
            i += 1;
            continue;
        }
        if rb[i] == b'>' {
            break;
        }
        // 属性名：字母 + alnum + - + _ + . + :
        if !rb[i].is_ascii_alphabetic() && rb[i] != b'_' {
            // 非合法属性字符——跳过整段直到下一个空白或 >
            while i < rb.len() && !matches!(rb[i], b' ' | b'\t' | b'>' | b'/') {
                i += 1;
            }
            continue;
        }
        let an_start = i;
        i += 1;
        while i < rb.len()
            && (rb[i].is_ascii_alphanumeric() || matches!(rb[i], b'_' | b'.' | b':' | b'-'))
        {
            i += 1;
        }
        let an = &rest[an_start..i].to_ascii_lowercase();
        // 跳 `=` 与可选引号
        let mut val: Option<String> = None;
        if i < rb.len() && rb[i] == b'=' {
            i += 1;
            while i < rb.len() && matches!(rb[i], b' ' | b'\t') {
                i += 1;
            }
            let quote = if i < rb.len() && (rb[i] == b'"' || rb[i] == b'\'') {
                let q = rb[i];
                i += 1;
                q
            } else {
                0 // 无引号
            };
            let vs = i;
            if quote != 0 {
                while i < rb.len() && rb[i] != quote {
                    i += 1;
                }
                if i < rb.len() {
                    // i 当前指向闭引号；[vs..i) 是纯值
                    val = Some(rest[vs..i].to_string());
                    i += 1; // 跳过闭引号
                }
            } else {
                while i < rb.len() && !matches!(rb[i], b' ' | b'\t' | b'>' | b'/') {
                    i += 1;
                }
                val = Some(rest[vs..i.min(rb.len())].to_string());
            }
            if quote != 0 && i <= vs {
                // 没找到闭引号——放弃这个属性
                continue 'outer;
            }
        }
        // 白名单属性收集
        let on_event = an.starts_with("on");
        if on_event {
            continue; // 事件属性一律忽略
        }
        if an == "style" {
            // 只保留白名单属性与合法值；非法声明逐条丢弃，标签本身仍生效。
            if let Some(v) = val {
                attrs.style = crate::syntax::model::sanitize_style(&v);
            }
            continue;
        }
        match an.as_str() {
            "title" => attrs.title = val,
            "src" => {
                if let Some(ref v) = val {
                    // 非白名单 scheme（javascript:/data:/file:/vbscript: 等）连 src 都不写。
                    if !crate::link_url_is_safe(v) {
                        attrs.src = None;
                        attrs.remote_src = None;
                        continue;
                    }
                    let lower = v.to_ascii_lowercase();
                    if lower.starts_with("http://") || lower.starts_with("https://") {
                        // 远程地址不写入 src（导出/PDF 依赖 src 为空），仅存 remote_src 供 UI 加载。
                        attrs.src = None;
                        attrs.remote_src = Some(v.trim().to_string());
                    } else {
                        attrs.src = val;
                    }
                }
            }
            "alt" => attrs.alt = val,
            // `<details open>` 是布尔属性：只保留属性存在性，导出时写回 `open`
            // （值本身无安全语义，任何写法都等价于 true）。
            "open" if tag_name == "details" => attrs.open = true,
            // 显式尺寸只对 `<img>` 有意义（其它标签渲染/导出都不读这两个字段）。
            // 只接受纯十进制数字：`100px` / `50%` / 负数等一律当非法值忽略，
            // 避免把非像素语义的单位误当像素放大版面。
            "width" if tag_name == "img" => attrs.width = parse_px(val.as_deref()),
            "height" if tag_name == "img" => attrs.height = parse_px(val.as_deref()),
            // 对齐只收集 left/center/right；其它值（含 CSS 单位/表达式）一律忽略。
            "align" if tag_name == "img" => {
                attrs.align = val.as_deref().and_then(ImageAlign::parse);
            }
            // 只有 `<a>` 的 href 参与渲染/导出；非白名单 scheme 置空（保留标签与文字）。
            "href" if tag_name == "a" => {
                if let Some(ref v) = val {
                    attrs.href = crate::link_url_is_safe(v).then(|| v.clone());
                }
            }
            _ => {}
        }
    }
    Some((tag_name, self_closed, attrs))
}

/// 交叉标签尾部：`close_gt` 处紧跟一个闭标签 `</name>`，而 name 在 `[start, close_gt)`
/// 内有开标签却无配对闭标签 → 返回该闭标签之后的位置。返回 None 表示配对正常。
fn cross_tag_tail(src: &str, start: usize, close_gt: usize) -> Option<usize> {
    let rest = src.get(close_gt..)?.strip_prefix("</")?;
    let name_end = rest.find(['>', '/', ' ', '\t', '\n', '\r'])?;
    let name = rest[..name_end].to_ascii_lowercase();
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    let inner = src[start..close_gt].to_ascii_lowercase();
    let needle = format!("<{name}");
    let opened = inner.match_indices(&needle).any(|(i, _)| {
        matches!(
            inner.as_bytes().get(i + needle.len()),
            None | Some(b'>') | Some(b'/') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
        )
    });
    if !opened {
        return None;
    }
    let gt = rest.find('>')?;
    Some(close_gt + 2 + gt + 1)
}

/// 在开标签结束 `open_end`（`>` 之后）处向后扫描，找同一 `tag_name` 的合法闭标签。
/// 扫描时跳过 `<!-- -->` / `<![CDATA[...]]>` / `<?...?>` / `<!...>` / 自闭合标签，
/// 并正确处理同类嵌套（如 `<div>a<div>b</div></div>`）。找到返回闭标签 `<` 的位置；
/// 到 EOF 仍未找到返回 None。`void_tags` 为自闭合标签集（不参与配对计数）。
fn find_matching_close(src: &str, open_end: usize, tag_name: &str) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut i = open_end;
    let mut depth = 0i32;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // 跳过注释/CDATA/PI/DOCTYPE——整段吞掉
        if src[i..].starts_with("<!--") {
            let end = src[i..]
                .find("-->")
                .map(|r| i + r + 3)
                .unwrap_or(bytes.len());
            i = end;
            continue;
        }
        if src[i..].starts_with("<![CDATA[") {
            let end = src[i..]
                .find("]]>")
                .map(|r| i + r + 3)
                .unwrap_or(bytes.len());
            i = end;
            continue;
        }
        if bytes.get(i + 1) == Some(&b'?') {
            let end = src[i..].find('>').map(|r| i + r + 1).unwrap_or(bytes.len());
            i = end;
            continue;
        }
        if bytes.get(i + 1) == Some(&b'!')
            && bytes.get(i + 2).is_some_and(|c| c.is_ascii_uppercase())
        {
            let end = src[i..].find('>').map(|r| i + r + 1).unwrap_or(bytes.len());
            i = end;
            continue;
        }
        // 解析标签名
        let after_lt = &src[i + 1..];
        let (closing, name_start) = if after_lt.starts_with('/') {
            (true, 1)
        } else {
            (false, 0)
        };
        let nb = after_lt.as_bytes();
        if nb.get(name_start).is_none_or(|c| !c.is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        let mut nl = 1usize;
        while nl + name_start < nb.len()
            && (nb[nl + name_start].is_ascii_alphanumeric() || nb[nl + name_start] == b'-')
        {
            nl += 1;
        }
        let name_here = after_lt[name_start..name_start + nl].to_ascii_lowercase();
        // 找 `>` 看是否自闭合（引号内的 `>` 不算结束）
        let tag_end = html_tag_end(src, i);
        let self_closed = tag_end.is_some_and(|te| src[i..te - 1].trim_end().ends_with('/'));
        if self_closed && !closing {
            // 自闭合开标签：不计入 depth，直接跳过
            i = tag_end.unwrap_or(bytes.len());
            continue;
        }
        if closing {
            if name_here == tag_name {
                if depth == 0 {
                    return Some(i); // 匹配成功
                } else {
                    depth -= 1;
                }
            }
        } else {
            if name_here == tag_name {
                depth += 1;
            }
        }
        i = tag_end.unwrap_or(bytes.len());
    }
    None
}

struct InlineParser<'a> {
    s: &'a str,
    base: usize,
    refs: &'a RefMap,
    pos: usize,
    /// 一次性括号配对索引（parse_until 入口处建）。未配对位置存 None。
    bracket_pairs: Vec<Option<usize>>,
    /// 位置 i 之前（不含 i）「能成链且非图片开括号」的 `[` 累计数。
    /// O(1) 查询区间 (lo, hi) 内是否有内层成链：prefix[hi] - prefix[lo+1] > 0
    resolved_prefix: Vec<usize>,
    /// 反引号 run 索引（位置, 长度），按位置递增。后续批次 2-2 扩展使用。
    /// **不可按本层 stop 裁剪**：代码段优先级高于删除线（`` ~~a `b~~` c~~ `` 的首个
    /// `~~` 属于代码段内容），裁剪后闭 run 找不到，会误判删除线闭标记。
    backtick_runs: Vec<(usize, usize)>,
}

impl<'a> InlineParser<'a> {
    /// 一次性预扫 self.s：建括号配对索引 + 内层淘汰判定前缀和 + 反引号 run 索引。
    /// O(n)：栈扫描，跳过转义/代码段/HTML（和 link_close 原跳过逻辑完全同源）。
    /// `limit` 为本层上界（`layer_limit` 求得的 stop 起点）：只为 `pos < limit` 的
    /// 位置建配对，越界的开/闭括号一律无配对，使 `try_link` 不会造出横跨 stop 的链接。
    fn scan_bracket_pairs(&mut self, limit: usize) {
        let b = self.s.as_bytes();
        let n = b.len();

        // 第一遍：简单登记所有反引号 run（不跳过任何内容）
        let mut backticks: Vec<(usize, usize)> = Vec::new();
        {
            let mut pos = 0;
            while pos < n {
                if b[pos] == b'`' {
                    let run = b[pos..].iter().take_while(|&&x| x == b'`').count();
                    backticks.push((pos, run));
                    pos += run;
                } else {
                    pos += 1;
                }
            }
        }

        // 第二遍：括号配对栈扫描（正确跳过代码段/HTML/转义内部的方括号）
        let mut pairs: Vec<Option<usize>> = vec![None; n];
        let mut open_stack: Vec<usize> = Vec::new();
        let mut pos = 0;

        while pos < limit {
            match b[pos] {
                b'\\' if pos + 1 < n => {
                    pos += 2;
                }
                b'[' => {
                    open_stack.push(pos);
                    pos += 1;
                }
                b']' => {
                    if let Some(o) = open_stack.pop() {
                        pairs[o] = Some(pos);
                        pairs[pos] = Some(o);
                    }
                    pos += 1;
                }
                b'`' => {
                    let run = b[pos..].iter().take_while(|&&x| x == b'`').count();
                    // 跳过完整代码段（包括内部的方括号和反引号）
                    pos = code_span_close(self.s, pos, run).unwrap_or(pos + run);
                }
                b'<' => {
                    pos = html_tag_end(self.s, pos).unwrap_or(pos + 1);
                }
                _ => {
                    pos += 1;
                }
            }
        }

        // 前缀和：只统计「能成链 + 前邻无 !（非图片开括号）」的 [
        let mut prefix: Vec<usize> = vec![0; n + 1];
        for i in 0..n {
            prefix[i + 1] = prefix[i];
            if b[i] == b'[' {
                if let Some(close_pos) = pairs[i] {
                    let prev_is_bang = i >= 1 && b[i - 1] == b'!' && (i < 2 || b[i - 2] != b'\\');
                    if !prev_is_bang && self.pair_resolves(i, close_pos) {
                        prefix[i + 1] += 1;
                    }
                }
            }
        }

        self.bracket_pairs = pairs;
        self.resolved_prefix = prefix;
        self.backtick_runs = backticks;
    }

    /// 本层可控的 `stop` 起点（绝对索引）：从 `self.pos` 起按 parse_until 主循环同一套
    /// 跳过规则（转义跳 2、代码段、行内 HTML）找第一个 loop 级 `~~`；无 stop 或找不到
    /// 则为 `self.s.len()`。括号配对不得越过此界，否则 `[` 会与 stop 之后的 `]` 配对，
    /// 造出横跨删除线闭标记的 Link。
    fn layer_limit(&self, stop: Option<&'static str>) -> usize {
        let Some(st) = stop else {
            return self.s.len();
        };
        let n = self.s.len();
        let mut pos = self.pos;
        while pos < n {
            if self.s[pos..].starts_with(st) {
                return pos;
            }
            match self.s.as_bytes()[pos] {
                b'\\' => {
                    let adv = match self.s[pos + 1..].chars().next() {
                        Some(c) if is_escapable(c) => 1 + c.len_utf8(),
                        _ => 1,
                    };
                    pos += adv;
                }
                b'`' => {
                    let run = self.s[pos..].bytes().take_while(|&x| x == b'`').count();
                    // 与主循环 try_code_span 同源：完整代码段整段跳过，闭 run 不存在则跳过 run
                    pos = code_span_close(self.s, pos, run).unwrap_or(pos + run);
                }
                b'<' => {
                    pos = html_tag_end(self.s, pos).unwrap_or(pos + 1);
                }
                _ => {
                    pos += self.s[pos..].chars().next().map_or(1, char::len_utf8);
                }
            }
        }
        n
    }

    /// 解析直到 `stop` 分隔符（留在原位不消费）或 EOF。
    /// 段内产出 token 流（原子节点 + 强调分隔符 run），收尾经阶段二匹配为节点。
    fn parse_until(
        &mut self,
        stop: Option<&'static str>,
        allow_link: bool,
        depth: usize,
    ) -> Vec<InlineNode> {
        let limit = self.layer_limit(stop);
        self.scan_bracket_pairs(limit);
        let mut toks: Vec<Tok> = Vec::new();
        while self.pos < self.s.len() {
            let rest = &self.s[self.pos..];
            if stop.is_some_and(|st| rest.starts_with(st)) {
                break;
            }
            if let Some(after) = rest.strip_prefix('\\') {
                // 转义 ASCII 标点则两字符皆为纯文本；否则 `\` 自身为普通字符
                match after.chars().next() {
                    Some(c) if is_escapable(c) => self.pos += 1 + c.len_utf8(),
                    _ => self.pos += 1,
                }
                continue;
            }
            if depth < MAX_INLINE_DEPTH {
                if let Some(node) = self
                    .try_html(depth)
                    .or_else(|| self.try_autolink())
                    .or_else(|| self.try_code_span())
                    .or_else(|| self.try_math())
                    .or_else(|| self.try_strikethrough(depth))
                    .or_else(|| self.try_highlight(depth))
                    .or_else(|| self.try_footnote_ref())
                    .or_else(|| self.try_image())
                    .or_else(|| {
                        if allow_link {
                            self.try_link(depth)
                        } else {
                            None
                        }
                    })
                {
                    // 原子子树内的强调深度已由其内层 resolve 各自钳制（≤ MAX×EMPH 总深）
                    toks.push(Tok::Node(node, 0));
                    continue;
                }
                if let Some(d) = self.scan_delim_run() {
                    toks.push(d);
                    continue;
                }
            }
            // 普通字符：按 UTF-8 字符边界前进，保证不慢于线性、不切碎多字节字符。
            // 注意须按当前 pos 重新切片：scan_delim_run 对不可开闭的 run 已整段消费
            // （落为纯文本），循环顶部的 rest 已过期；run 在段尾时 pos 已至 EOF。
            // 反引号 run 同样整段消费：开 run 匹配失败时，run 内部任一位置都不能再
            // 作为开标记（CommonMark 的 run 是极大连续段，Example 347）。
            if self.pos < self.s.len() {
                let c = self.s[self.pos..].chars().next().unwrap();
                let adv = if c == '`' {
                    self.s[self.pos..]
                        .bytes()
                        .take_while(|&b| b == b'`')
                        .count()
                } else if c == '<' {
                    // 行内 HTML/自动链接形态整段跳过：标签内的 `[`/`]`/`*` 等不参与
                    // 行内解析（#0536：标签属性里的 `[ref]` 不成链）
                    match html_tag_end(self.s, self.pos) {
                        Some(end) => end - self.pos,
                        None => 1,
                    }
                } else {
                    c.len_utf8()
                };
                self.pos += adv;
            }
        }
        resolve_emphasis(toks, self.base)
    }

    /// `*`/`_` 分隔符 run：按 flanking 规则判定开/闭资格，整 run 记录为一个 token，
    /// 匹配时拆分消费。两侧皆不可开闭的 run 不落 token（直接是纯文本）。
    fn scan_delim_run(&mut self) -> Option<Tok> {
        let rest = &self.s[self.pos..];
        let chr = rest.as_bytes()[0];
        if chr != b'*' && chr != b'_' {
            return None;
        }
        let len = rest.bytes().take_while(|&b| b == chr).count();
        let start = self.pos;
        // flanking 取源文本原始相邻字符（转义消费不影响判定）；段首/段尾视为空白
        let prev = self.s[..start].chars().next_back();
        let next = rest[len..].chars().next();
        let prev_ws = prev.is_none_or(|c| c.is_whitespace());
        let prev_punct = prev.is_some_and(is_unicode_punctuation);
        let next_ws = next.is_none_or(|c| c.is_whitespace());
        let next_punct = next.is_some_and(is_unicode_punctuation);
        let left = !next_ws && (!next_punct || prev_ws || prev_punct);
        let right = !prev_ws && (!prev_punct || next_ws || next_punct);
        // `_` 追加 intraword 限制：词内（两侧非空白非标点）不可开、不可闭
        let (can_open, can_close) = if chr == b'_' {
            (
                left && (!right || prev_punct),
                right && (!left || next_punct),
            )
        } else {
            (left, right)
        };
        self.pos += len;
        (can_open || can_close).then_some(Tok::Delim(Delim {
            chr,
            start,
            len,
            can_open,
            can_close,
        }))
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

    /// 行内白名单 HTML 解析：`<kbd>Ctrl</kbd>` / `<br>` / `<img src="...">` 等。
    /// 分派位置在 try_autolink 之前（`<kbd>` 不是自动链接）。
    /// - 白名单 void 自闭合 → HtmlSelfClosed
    /// - 白名单 pair 配对成功 → Html + 子节点递归
    /// - 白名单 pair 配对失败 → HtmlLiteral（整段源码原样，吞至 EOF）
    /// - 白名单外 / 非合法开标签 → 返回 None（后续逻辑继续）
    fn try_html(&mut self, depth: usize) -> Option<InlineNode> {
        let rest = &self.s[self.pos..];
        if !rest.starts_with('<') {
            return None;
        }
        let start = self.pos;
        let end_tag = html_tag_end(self.s, start)?;
        let open_src = &self.s[start + 1..end_tag - 1];
        // 自动链接先行：`<scheme:...>` / `<local@domain>` 不含空白、不含 `<`、标签名后
        // 直接接 `:` 或 `@`。这些形态不是合法 HTML 属性语法（属性名须以字母/`_` 开头、
        // 前须有空白）。open_src 不含空白 → 大概率是 autolink，放给 try_autolink。
        if !open_src
            .as_bytes()
            .iter()
            .any(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'>'))
            && open_src.find([':', '@']).is_some()
        {
            return None;
        }
        let (tag_name, self_closed, attrs) = parse_open_tag(open_src)?;
        let tag = HtmlTag::from_str(&tag_name)?; // 白名单外 → None，后续处理
                                                 // void 标签（<br> / <img>）：HTML5 允许省略 `/`，只要标签名合法就算自闭合
        if tag.is_void() {
            self.pos = end_tag;
            return Some(self.node(
                InlineKind::HtmlSelfClosed { tag, attrs },
                start,
                end_tag,
                end_tag,
                None,
                Vec::new(),
            ));
        }
        // pair 标签——自闭合写法非法（如 `<a />`）
        if self_closed {
            return None;
        }
        let close_lt = find_matching_close(self.s, end_tag, &tag_name);
        let Some(close_lt) = close_lt else {
            // 未闭合 → 整段 HtmlLiteral 吞到 EOF
            self.pos = self.s.len();
            let literal_end = self.s.len();
            let lit_kind = InlineKind::HtmlLiteral;
            return Some(self.node(lit_kind, start, start, literal_end, None, Vec::new()));
        };
        // 找闭标签的 `>` 位置
        let close_gt = html_tag_end(self.s, close_lt)?;
        // 交叉标签（`<kbd><mark>x</kbd></mark>`）：外层闭标签之后紧跟着内层未闭合
        // 标签的闭标签。整段源码按字面（HtmlLiteral）处理并吞掉两段闭标签，
        // 否则内层开标签已被降级为字面、外层闭标签后却漏出孤立的 `</mark>`，
        // 导出件会产出半开半闭的畸形 HTML。
        if let Some(end) = cross_tag_tail(self.s, start, close_gt) {
            self.pos = end;
            return Some(self.node(InlineKind::HtmlLiteral, start, start, end, None, Vec::new()));
        }
        // 递归解析内部
        let inner_start = end_tag;
        let inner_end = close_lt;
        let mut sub = InlineParser {
            s: &self.s[inner_start..inner_end],
            base: self.base + inner_start,
            refs: self.refs,
            pos: 0,
            bracket_pairs: Vec::new(),
            resolved_prefix: Vec::new(),
            backtick_runs: Vec::new(),
        };
        let children = sub.parse_until(None, true, depth + 1);
        self.pos = close_gt;
        Some(self.node(
            InlineKind::Html { tag, attrs },
            start,
            inner_start,
            inner_end,
            None,
            children,
        ))
    }

    /// 自动链接（CommonMark §6.5）：`<scheme:...>` 或 `<local@domain>`。
    /// 内容不含空白/控制字符/`<`/`>`；URI 不做反斜杠转义（Example 603 的 `\[\` 原样）。
    fn try_autolink(&mut self) -> Option<InlineNode> {
        let rest = &self.s[self.pos..];
        let inner = rest.strip_prefix('<')?;
        let rel = inner
            .bytes()
            .position(|c| c == b'>' || c == b'<' || c <= b' ' || c == 0x7f)?;
        if inner.as_bytes()[rel] != b'>' {
            return None;
        }
        let body = &inner[..rel];
        if !is_uri_autolink(body) && !is_email_autolink(body) {
            return None;
        }
        let start = self.pos;
        let content_end = start + 1 + rel;
        self.pos = content_end + 1;
        Some(self.node(
            InlineKind::Link,
            start,
            start + 1,
            content_end,
            Some((start + 1, content_end)),
            Vec::new(),
        ))
    }

    /// `` `code` ``：开/闭反引号 run 长度必须相等（CommonMark §6.1）；内容非空、原样
    /// （不嵌套）。`` ```foo`` `` 的闭 run 只有 2 个反引号 → 不成代码段（Example 347）。
    /// 用 parse_until 入口预扫的 backtick_runs 索引，O(runs) 直接找同长度闭 run。
    fn try_code_span(&mut self) -> Option<InlineNode> {
        let rest = &self.s[self.pos..];
        let open = rest.bytes().take_while(|&b| b == b'`').count();
        if open == 0 {
            return None;
        }
        let search_start = self.pos + open;
        // 从预扫索引中找 search_start 之后、长度 == open 的第一个 run
        let close_pos = self
            .backtick_runs
            .iter()
            .find(|&&(rp, rl)| rp >= search_start && rl == open)
            .map(|&(rp, _)| rp)?;
        if close_pos == search_start {
            return None; // 空内容（` `` `）不成节点
        }
        let start = self.pos;
        let content_end = close_pos;
        self.pos = close_pos + open;
        Some(self.node(
            InlineKind::Code,
            start,
            start + open,
            content_end,
            None,
            Vec::new(),
        ))
    }

    /// `$math$`：内容非空、首尾非空格、闭 `$` 后不随数字（pandoc 规则）。
    /// 反斜杠转义的 `\$` 不是闭定界符（`$6 \$` 保持纯文本）。
    fn try_math(&mut self) -> Option<InlineNode> {
        let rest = &self.s[self.pos..];
        if !rest.starts_with('$') {
            return None;
        }
        let inner = &rest[1..];
        let rel = find_unescaped_dollar(inner)?;
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

    /// `~~del~~`（GFM 删除线）：最近闭标记成节点，内容非空，子节点递归。
    /// 单个 `~` 为纯文本（下标语法不在 v0 范围）。
    fn try_strikethrough(&mut self, depth: usize) -> Option<InlineNode> {
        if !self.s[self.pos..].starts_with("~~") {
            return None;
        }
        let start = self.pos;
        self.pos = start + 2;
        // 内层 parse_until 会在本实例上重建（更小 limit 的）括号索引；先移出外层索引，
        // 返回后恢复，否则本层 stop 之后的 `[...]` 会查不到配对而丢失链接。
        let saved_pairs = std::mem::take(&mut self.bracket_pairs);
        let saved_prefix = std::mem::take(&mut self.resolved_prefix);
        let saved_runs = std::mem::take(&mut self.backtick_runs);
        let children = self.parse_until(Some("~~"), true, depth + 1);
        self.bracket_pairs = saved_pairs;
        self.resolved_prefix = saved_prefix;
        self.backtick_runs = saved_runs;
        let content_end = self.pos;
        if !self.s[content_end..].starts_with("~~") || content_end == start + 2 {
            self.pos = start; // 未闭合或空内容：还原，开标记按纯文本
            return None;
        }
        self.pos = content_end + 2;
        Some(self.node(
            InlineKind::Strikethrough,
            start,
            start + 2,
            content_end,
            None,
            children,
        ))
    }

    /// `==text==`（高亮，等价 `<mark>`）：最近闭标记成节点，内容非空，子节点递归。
    /// `===` 不作为开标记；闭标记若属于更长的 `=` run（`===`）同样不承认，
    /// 避免把 `===x===` 的边界错切。转义 `\==` 由通用转义扫描消费。
    fn try_highlight(&mut self, depth: usize) -> Option<InlineNode> {
        let rest = &self.s[self.pos..];
        if !rest.starts_with("==") {
            return None;
        }
        // 开标记必须是长度恰为 2 的 `=` run（`===` 的任一位置都不开）。
        let prev_eq = self.pos > 0 && self.s.as_bytes()[self.pos - 1] == b'=';
        if prev_eq || rest.as_bytes().get(2) == Some(&b'=') {
            return None;
        }
        let start = self.pos;
        self.pos = start + 2;
        let saved_pairs = std::mem::take(&mut self.bracket_pairs);
        let saved_prefix = std::mem::take(&mut self.resolved_prefix);
        let saved_runs = std::mem::take(&mut self.backtick_runs);
        let children = self.parse_until(Some("=="), true, depth + 1);
        self.bracket_pairs = saved_pairs;
        self.resolved_prefix = saved_prefix;
        self.backtick_runs = saved_runs;
        let content_end = self.pos;
        let content = &self.s[start + 2..content_end];
        // 闭标记同为长度恰为 2 的 run；跨行（含 `\n`）不识别。
        let close_prev_eq = content_end > 0 && self.s.as_bytes()[content_end - 1] == b'=';
        let closed = self.s[content_end..].starts_with("==")
            && !close_prev_eq
            && self.s.as_bytes().get(content_end + 2) != Some(&b'=')
            && content_end > start + 2
            && !content.contains('\n');
        if !closed {
            self.pos = start; // 未闭合/空内容/跨行/`===` 边界：开标记按纯文本
            return None;
        }
        self.pos = content_end + 2;
        Some(self.node(
            InlineKind::Highlight,
            start,
            start + 2,
            content_end,
            None,
            children,
        ))
    }

    /// `[^id]`（GFM 脚注引用）：节点 `content` 为 id 文本区间，不查定义——缺失定义
    /// 仍产出节点，由视图按编号表降级。id 字符集与块级 `parse_footnote_def` 同源
    /// （字母/数字/`-`/`_`/中文），故两处不会对同一个 `[^…]` 给出不同边界。
    /// `![^id]` 是图片语法：紧邻的未转义 `!` 使本分支让位，避免把图片 alt 拆成
    /// 一个 `!` 文本 + 脚注引用。
    fn try_footnote_ref(&mut self) -> Option<InlineNode> {
        let rest = &self.s[self.pos..];
        let inner = rest.strip_prefix("[^")?;
        if self.pos > 0 && self.s.as_bytes()[self.pos - 1] == b'!' && !self.is_escaped(self.pos - 1)
        {
            return None;
        }
        let mut id_len = 0usize;
        for (off, c) in inner.char_indices() {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                id_len = off + c.len_utf8();
            } else {
                break;
            }
        }
        if id_len == 0 || inner.as_bytes().get(id_len) != Some(&b']') {
            return None;
        }
        let start = self.pos;
        let content_start = start + 2;
        let content_end = content_start + id_len;
        self.pos = content_end + 1;
        Some(self.node(
            InlineKind::FootnoteRef,
            start,
            content_start,
            content_end,
            None,
            Vec::new(),
        ))
    }

    /// 位置 `i` 的字符是否被反斜杠转义（前导连续反斜杠数为奇数）。
    fn is_escaped(&self, i: usize) -> bool {
        let mut n = 0;
        let mut k = i;
        while k > 0 && self.s.as_bytes()[k - 1] == b'\\' {
            n += 1;
            k -= 1;
        }
        n % 2 == 1
    }

    /// `[text](url)` / `[text][label]` / `[text][]` / `[text]`（shortcut）。
    /// 闭合 `]` 由括号栈配对定位（cmark 语义）：文本可含成对嵌套方括号（#0512）；
    /// 内层 `[...]` 先配对，内层成链则本开括号失效——链接不可嵌套链接（#0519/#0533）。
    /// 文本可嵌套除 Link 外的任意 inline。引用式查文档引用表，url 指向定义处 dest
    /// 绝对区间；查无定义按纯文本。
    fn try_link(&mut self, depth: usize) -> Option<InlineNode> {
        if !self.s[self.pos..].starts_with('[') {
            return None;
        }
        let start = self.pos;
        let text_end = self.bracket_pairs.get(start).cloned().flatten()?;
        // 内层淘汰：(start+1, text_end) 区间内有成链且非图片开括号的内层 [ → 淘汰本链接开括号
        let inner_resolved = self.resolved_prefix[text_end] - self.resolved_prefix[start + 1];
        if inner_resolved > 0 {
            return None;
        }
        // 分支 1：`](` inline 链接（优先于引用式）
        if self.s[text_end..].starts_with("](") {
            let after_open = text_end + 2;
            if let Some((url_s, url_e, end)) = parse_link_dest(self.s, after_open, false) {
                let children = self.link_children(start, text_end, depth);
                self.pos = end;
                return Some(self.node(
                    InlineKind::Link,
                    start,
                    start + 1,
                    text_end,
                    Some((url_s, url_e)),
                    children,
                ));
            }
            // 目标/title 不合法：回退到 shortcut 引用（#0568）
        }
        // 分支 2：`][label]` full / `][]` collapsed（查无定义即纯文本，不再试 shortcut）
        if self.s[text_end..].starts_with("][") {
            let ls = text_end + 2;
            let rel = self.s[ls..].find(']')?;
            let le = ls + rel;
            // label 不可含 `[`、长度 ≤999；空 label（collapsed）取链接文本
            if self.s[ls..le].contains('[') || le - ls > 999 {
                return None;
            }
            let label_src = if le == ls {
                &self.s[start + 1..text_end]
            } else {
                &self.s[ls..le]
            };
            let &(dest, _) = self.refs.get(&normalize_label(label_src))?;
            let children = self.link_children(start, text_end, depth);
            self.pos = le + 1;
            let mut n = self.node(InlineKind::Link, start, start + 1, text_end, None, children);
            n.url = Some(dest); // 绝对区间：定义处 dest
            return Some(n);
        }
        // 分支 3：shortcut——以链接文本为 label 查表
        let &(dest, _) = self
            .refs
            .get(&normalize_label(&self.s[start + 1..text_end]))?;
        let children = self.link_children(start, text_end, depth);
        self.pos = text_end + 1;
        let mut n = self.node(InlineKind::Link, start, start + 1, text_end, None, children);
        n.url = Some(dest);
        Some(n)
    }

    /// 链接文本子节点：文本区间已知为 (start, text_end)，子解析不再被 `]` 截断
    /// （嵌套方括号已由 link_close 平衡）；文本内不允许 Link（allow_link=false）。
    fn link_children(&self, start: usize, text_end: usize, depth: usize) -> Vec<InlineNode> {
        let mut sub = InlineParser {
            s: &self.s[start + 1..text_end],
            base: self.base + start + 1,
            refs: self.refs,
            pos: 0,
            bracket_pairs: Vec::new(),
            resolved_prefix: Vec::new(),
            backtick_runs: Vec::new(),
        };
        sub.parse_until(None, false, depth + 1)
    }

    /// 配对 (o, p) 能否成链/成图：`](`+合法目标、`][label]`/`][]`+定义、shortcut+定义。
    /// 只服务链接开括号的淘汰判定（`!` 前的图片开括号已排除），故目标严格半角。
    fn pair_resolves(&self, o: usize, p: usize) -> bool {
        let after = &self.s[p + 1..];
        if after.starts_with('(') && parse_link_dest(self.s, p + 2, false).is_some() {
            return true;
        }
        if after.starts_with('[') {
            let ls = p + 2;
            let Some(rel) = self.s[ls..].find(']') else {
                return false;
            };
            let le = ls + rel;
            if self.s[ls..le].contains('[') || le - ls > 999 {
                return false;
            }
            let src = if le == ls {
                &self.s[o + 1..p]
            } else {
                &self.s[ls..le]
            };
            return self.refs.contains_key(&normalize_label(src));
        }
        self.refs.contains_key(&normalize_label(&self.s[o + 1..p]))
    }

    /// `![alt](url)` / `![alt][label]` / `![alt][]` / `![alt]`：alt 可空、为纯文本
    /// （不产生子节点）；闭合 `]` 与链接同一套括号配对。未闭合或查无定义按纯文本。
    /// 目标定界符同时接受半角 `(` `)` 与全角 `（` `）`（允许混用），但只在
    /// `![alt]` 之后**紧跟**定界符时生效，且必须找到配对闭合符。
    fn try_image(&mut self) -> Option<InlineNode> {
        if !self.s[self.pos..].starts_with("![") {
            return None;
        }
        let start = self.pos;
        let alt_end = self.bracket_pairs.get(start + 1).cloned().flatten()?;
        // 图片开括号 ours_is_image=true，内层成链不淘汰（inner_pair_kills_opener 恒 false）
        // 分支 1：`](` / `]（` inline 图片
        let open_len = if self.s[alt_end..].starts_with("](") {
            2
        } else if self.s[alt_end..].starts_with("]（") {
            1 + FW_OPEN.len()
        } else {
            0
        };
        if open_len > 0 {
            let after_open = alt_end + open_len;
            let dest = parse_link_dest(self.s, after_open, true).or_else(|| {
                // 全角定界符容错：目标含空格时标准解析在空白处即止，无法闭合成图。
                // 仅当定界符确实用了全角（开标记全角，或同一行内出现全角闭括号）才放宽，
                // 纯 ASCII 图片（`![a](x y)`）保持 CommonMark 语义按纯文本。
                let used_fullwidth = open_len > 2
                    || self.s[after_open..]
                        .split(['\n', '\r'])
                        .next()
                        .is_some_and(|line| line.contains('（') || line.contains('）'));
                if used_fullwidth {
                    parse_image_dest_spaces(self.s, after_open)
                } else {
                    None
                }
            });
            if let Some((url_s, url_e, end)) = dest {
                self.pos = end;
                return Some(self.node(
                    InlineKind::Image,
                    start,
                    start + 2,
                    alt_end,
                    Some((url_s, url_e)),
                    Vec::new(),
                ));
            }
            // 目标/title 不合法：回退到 shortcut 引用
        }
        // 分支 2：`][label]` / `][]`
        if self.s[alt_end..].starts_with("][") {
            let ls = alt_end + 2;
            let rel = self.s[ls..].find(']')?;
            let le = ls + rel;
            if self.s[ls..le].contains('[') || le - ls > 999 {
                return None;
            }
            let label_src = if le == ls {
                &self.s[start + 2..alt_end]
            } else {
                &self.s[ls..le]
            };
            let &(dest, _) = self.refs.get(&normalize_label(label_src))?;
            self.pos = le + 1;
            let mut n = self.node(
                InlineKind::Image,
                start,
                start + 2,
                alt_end,
                None,
                Vec::new(),
            );
            n.url = Some(dest);
            return Some(n);
        }
        // 分支 3：shortcut 图片
        let &(dest, _) = self
            .refs
            .get(&normalize_label(&self.s[start + 2..alt_end]))?;
        self.pos = alt_end + 1;
        let mut n = self.node(
            InlineKind::Image,
            start,
            start + 2,
            alt_end,
            None,
            Vec::new(),
        );
        n.url = Some(dest);
        Some(n)
    }
}

/// 紧凑 S-表达式快照：种类 + 各区间 + 递归子节点，一图钉住整棵树。
/// 供本模块与 syntax 状态层（`super`）的测试共用。
#[cfg(test)]
pub(super) fn sexp(ns: &[InlineNode]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<InlineNode> {
        parse_inlines(text, 0, &RefMap::new())
    }

    /// 首个 Link 节点（含嵌套在强调/删除线内的）。
    fn find_link(ns: &[InlineNode]) -> Option<&InlineNode> {
        for n in ns {
            if n.kind == InlineKind::Link {
                return Some(n);
            }
            if let Some(f) = find_link(&n.children) {
                return Some(f);
            }
        }
        None
    }

    /// 首个 Image 节点（含嵌套）。用于断言「不得误判为图片」。
    fn find_image(ns: &[InlineNode]) -> Option<&InlineNode> {
        for n in ns {
            if n.kind == InlineKind::Image {
                return Some(n);
            }
            if let Some(f) = find_image(&n.children) {
                return Some(f);
            }
        }
        None
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
    fn code_span_marker_runs_must_match_length() {
        // Example 347：开 run 3 个、闭 run 2 个 → 不成代码段
        assert_eq!(sexp(&parse("```foo``")), "");
        // 等长 run：``a``
        assert_eq!(sexp(&parse("``a``")), "Code[0..5 c=2..3]");
        // Example 340：内层 `` 不是闭标记，等长的末尾单反引号闭合
        assert_eq!(sexp(&parse("` foo `` bar `")), "Code[0..14 c=1..13]");
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
    fn autolink_uri_and_email() {
        assert_eq!(sexp(&parse("<a+b+c:d>")), "Link[0..9 c=1..8 url=1..8]");
        assert_eq!(sexp(&parse("<a@b.c>")), "Link[0..7 c=1..6 url=1..6]");
        // scheme 至少 2 字符：`<m:abc>` 不是自动链接
        assert_eq!(sexp(&parse("<m:abc>")), "");
        // 内容含空格 / 无 scheme / 空：均非自动链接
        assert_eq!(sexp(&parse("<https://foo.bar/baz bim>")), "");
        assert_eq!(sexp(&parse("<foo.bar.baz>")), "");
        assert_eq!(sexp(&parse("<>")), "");
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
    fn adjacent_strong_runs_nest_per_delimiter_run() {
        // delimiter-run（0c P1 起）：`**a **b** c**` 闭标记就近匹配 → 嵌套 strong，
        // 与 CommonMark 一致（旧「最左匹配得两个并列 strong」分歧已消除）
        assert_eq!(
            sexp(&parse("**a **b** c**")),
            "Strong[0..13 c=2..11 Strong[4..9 c=6..7]]"
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
        // B5 保险丝：超深嵌套不得栈溢出（MAX_INLINE_DEPTH=16）。
        // image alt 为纯文本（无子节点），嵌套链靠 link→image→link 交替
        let doc = format!("{}a{}", "[![".repeat(40), "](i)](u)".repeat(40));
        let _ = parse(&doc); // 不 panic 即通过
    }

    #[test]
    fn emph_closed_by_first_star_of_double() {
        // 闭 run 长度 2 与开 run 长度 1 匹配：消费 1 个 `*` 成 Emph，余 1 个为纯文本
        assert_eq!(sexp(&parse("*a**")), "Emph[0..3 c=1..2]");
    }

    #[test]
    fn links_do_not_nest() {
        // 内层优先（cmark 括号栈语义）：`[b](u)` 成链，外层开括号失效为纯文本
        assert_eq!(sexp(&parse("[a [b](u)](v)")), "Link[3..9 c=4..5 url=7..8]");
        // 文本可含成对嵌套方括号（#0512）：内层不成链时外层成链
        assert_eq!(
            sexp(&parse("[link [foo [bar]]](/uri)")),
            "Link[0..24 c=1..17 url=19..23]"
        );
        // 内层 `]` 在代码段/行内 HTML 内不参与配对（#0524/#0536）
        assert!(parse("[foo <bar attr=\"](baz)\">")
            .iter()
            .all(|n| n.kind != InlineKind::Link));
        assert_eq!(
            sexp(&parse("[a `]` b](u)")),
            "Link[0..12 c=1..8 url=10..11 Code[3..6 c=4..5]]"
        );
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
        // CommonMark：非尖括号 URL 允许平衡括号
        assert_eq!(sexp(&parse("[t](a(b)c)")), "Link[0..10 c=1..2 url=4..9]");
        assert_eq!(sexp(&parse("[t](a(b))")), "Link[0..9 c=1..2 url=4..8]");
        // 未平衡：末尾 `)` 消费后 depth>0，整段非链接
        assert_eq!(sexp(&parse("[t](a(b)")), "");
    }

    #[test]
    fn link_dest_rejects_space() {
        // #0488：`/my uri` 含空格，非合法目标
        assert_eq!(sexp(&parse("[link](/my uri)")), "");
    }

    #[test]
    fn link_dest_rejects_newline() {
        // #0490：裸目标不可含换行
        assert_eq!(sexp(&parse("[link](foo\nbar)")), "");
        // #0491：尖括号目标同样不可含换行
        assert_eq!(sexp(&parse("[link](<foo\nbar>)")), "");
    }

    #[test]
    fn link_dest_angle_bracket() {
        // 尖括号目标：内容不含换行/未转义尖括号
        assert_eq!(
            sexp(&parse("[t](</my uri>)")),
            "Link[0..14 c=1..2 url=5..12]"
        );
        // #0493：转义 `>` 后无闭合尖括号 → 非链接
        assert_eq!(sexp(&parse("[link](<foo\\>)")), "");
        // #0494：`<b)c` 缺闭合尖括号
        assert_eq!(sexp(&parse("[a](<b)c")), "");
    }

    #[test]
    fn link_dest_balanced_parens() {
        // #0497：`foo(and(bar)` 缺一层闭括号
        assert_eq!(sexp(&parse("[link](foo(and(bar))")), "");
        // 平衡括号合法
        assert_eq!(
            sexp(&parse("[link](foo(and(bar)))")),
            "Link[0..21 c=1..5 url=7..20]"
        );
    }

    #[test]
    fn bare_url_basic_prefixes() {
        // W1-5：三种前缀（url 指向源码本身，协议补全在消费侧完成）
        assert_eq!(sexp(&parse("www.a.com")), "Link[0..9 c=0..9 url=0..9]");
        assert_eq!(
            sexp(&parse("http://a.com")),
            "Link[0..12 c=0..12 url=0..12]"
        );
        assert_eq!(
            sexp(&parse("https://a.com")),
            "Link[0..13 c=0..13 url=0..13]"
        );
        assert_eq!(
            sexp(&parse("Visit www.a.com now")),
            "Link[6..15 c=6..15 url=6..15]"
        );
    }

    #[test]
    fn bare_url_trailing_punctuation_stripped() {
        // GFM：`.` `,` `;` `:` `!` `?` `*` `_` `~` 不属于 URL 尾
        for (src, end) in [
            ("www.a.com.", 9usize),
            ("www.a.com,", 9),
            ("www.a.com;", 9),
            ("www.a.com:", 9),
            ("www.a.com!", 9),
            ("www.a.com?", 9),
            ("**www.a.com**", 11),
            ("_www.a.com_", 10),
            ("~~www.a.com~~", 11),
        ] {
            let ns = parse(src);
            let link = find_link(&ns).expect(src);
            assert_eq!(link.url.unwrap().end, end, "{src} 尾部标点应剥离");
        }
    }

    #[test]
    fn legacy_spaced_anchor_destination_is_parsed() {
        // 旧版 Veil 目录锚点含空格：仍须识别为链接，目标原文保持完整（含空白）。
        let src = "[第三章 结论](#第三章 - 结论)";
        let ns = parse(src);
        let link = find_link(&ns).expect("含空格锚点应成链");
        let url = link.url.expect("应有目标区间");
        assert_eq!(&src[url.start..url.end], "#第三章 - 结论");
        assert_eq!(link.range.end, src.len());
        let src = "[1.1 背景](#11- 背景)";
        let ns = parse(src);
        let link = find_link(&ns).expect("带编号的含空格锚点应成链");
        let url = link.url.unwrap();
        assert_eq!(&src[url.start..url.end], "#11- 背景");
        assert_eq!(link.range.end, src.len());
        // 目标前允许空白、尾随空白：`(# 第一章 - 介绍 )`
        let src = "[第一章 介绍](# 第一章 - 介绍 )";
        let link = parse(src)
            .into_iter()
            .find(|n| n.kind == InlineKind::Link)
            .expect("含前导/尾随空格的锚点应成链");
        let url = link.url.unwrap();
        assert_eq!(&src[url.start..url.end], "# 第一章 - 介绍 ");
        // 括号平衡不跨越 `)`；未闭合仍不是链接。
        let src = "[x](#a (b) c)";
        let ns = parse(src);
        let link = find_link(&ns).expect("平衡括号内的空格可容错");
        let url = link.url.unwrap();
        assert_eq!(&src[url.start..url.end], "#a (b) c");
        assert_eq!(sexp(&parse("[x](#a")), "");
    }

    #[test]
    fn spaced_non_anchor_destination_stays_invalid() {
        // 只放宽 `#` 锚点：外部 URL / 危险 scheme 含空格仍按 CommonMark 判非法。
        for src in [
            "[x](https://a.example/a b)",
            "[x](javascript:alert(1) x)",
            "[x](a b)",
        ] {
            // 裸 URL 自动识别可能另建链接节点，但 `[x](...)` 整体不得成链。
            let ns = parse(src);
            assert!(
                ns.iter()
                    .filter(|n| n.kind == InlineKind::Link)
                    .all(|n| n.range.start != 0),
                "{src} 不得因锚点容错整体成链"
            );
        }
        // 标准 `(#a \"title\")` 仍走 title 分支：目标不含 title。
        let src = "[x](#a \"t\")";
        let link = parse(src)
            .into_iter()
            .find(|n| n.kind == InlineKind::Link)
            .expect("带 title 的锚点链接");
        let url = link.url.unwrap();
        assert_eq!(&src[url.start..url.end], "#a");
    }

    #[test]
    fn bare_url_balanced_parens_kept() {
        // GFM 规则：成对括号属 URL，未配对的 `)` 终止
        let ns = parse("www.a.com/x+(b)");
        let link = ns.iter().find(|n| n.kind == InlineKind::Link).unwrap();
        assert_eq!(link.url.unwrap().end, 15);
        let ns = parse("(www.a.com/x)");
        let link = ns.iter().find(|n| n.kind == InlineKind::Link).unwrap();
        assert_eq!(link.url.unwrap().end, 12, "外层未配对 `)` 不计入");
    }

    #[test]
    fn bare_url_not_inside_code_or_autolink_or_markup() {
        // 代码段内不成链（已被 Code 节点覆盖）
        assert_eq!(sexp(&parse("`www.a.com`")), "Code[0..11 c=1..10]");
        // CommonMark 显式自动链接（`<...>`）由原逻辑处理，裸 URL 不抢
        assert_eq!(
            sexp(&parse("<http://a.com>")),
            "Link[0..14 c=1..13 url=1..13]"
        );
        // `<...>` 内含空格 → 显式自动链接非法，裸 URL 也不应接管
        assert_eq!(sexp(&parse("<http://a.com x>")), "");
        // 已有链接文本内不成链
        assert_eq!(
            sexp(&parse("[www.a.com](u)")),
            "Link[0..14 c=1..10 url=12..13]"
        );
    }

    #[test]
    fn bare_url_unbalanced_parens_stripped_after_suffix() {
        // GFM：扫到空白后才按括号收支剥尾部 `)`；中段未配对 `)` 不影响
        let ns = parse("www.a.com/x+(b))+ok");
        let link = ns.iter().find(|n| n.kind == InlineKind::Link).unwrap();
        assert_eq!(link.url.unwrap().end, 19, "尾部 `ok` 属 URL，中段 `)` 保留");
        let ns = parse("www.a.com/x+(b)))");
        let link = ns.iter().find(|n| n.kind == InlineKind::Link).unwrap();
        assert_eq!(link.url.unwrap().end, 15, "多出的两个 `)` 应剔除");
    }

    #[test]
    fn bare_url_entity_tail_excluded() {
        // `&hl;` 是实体样式，整体排除；正常 `&x=y` 保留
        let ns = parse("www.a.com/?q=a&hl;");
        let link = ns.iter().find(|n| n.kind == InlineKind::Link).unwrap();
        assert_eq!(
            &"www.a.com/?q=a&hl;"[..link.url.unwrap().end],
            "www.a.com/?q=a"
        );
        let ns = parse("www.a.com/?q=a&hl=en");
        let link = ns.iter().find(|n| n.kind == InlineKind::Link).unwrap();
        assert_eq!(link.url.unwrap().end, 20);
    }

    #[test]
    fn bare_url_any_scheme() {
        // 任意 `scheme://`（含 ftp）都成链；scheme 至少 2 字符
        let ns = parse("ftp://foo.bar.baz");
        let link = ns.iter().find(|n| n.kind == InlineKind::Link).unwrap();
        assert_eq!(link.url.unwrap().end, 17);
        assert_eq!(sexp(&parse("m://x.y")), "");
        // `www.` 紧贴单词内部不接管
        assert_eq!(sexp(&parse("awww.a.com")), "");
    }

    #[test]
    fn bare_url_stops_at_cjk_punctuation() {
        // TASK04-D003：`。下` 不得成为链接的一部分。
        let src = "见 https://example.com。下";
        let ns = parse(src);
        let link = find_link(&ns).expect("裸 URL 应成链");
        assert_eq!(
            &src[link.url.unwrap().start..link.url.unwrap().end],
            "https://example.com"
        );
        // 全角逗号/括号同样终止；括号后的中文保留。
        let src = "看 https://a.example/x，后再看";
        let ns = parse(src);
        let link = find_link(&ns).expect("裸 URL 应成链");
        assert_eq!(
            &src[link.url.unwrap().start..link.url.unwrap().end],
            "https://a.example/x"
        );
        // 对照：URL 内的非 ASCII 字母（中文路径）仍被保留。
        let src = "https://例子.example/路径";
        let ns = parse(src);
        let link = find_link(&ns).expect("裸 URL 应成链");
        assert_eq!(link.url.unwrap().end, src.len());
    }

    #[test]
    fn bare_url_after_closed_autolink_is_linked() {
        // TASK04-D010：闭合的 `<...>` 不能再把后续裸 URL 判为“尖括号内”而抑制。
        let src = "<https://a.example> https://b.example www.example.com";
        let ns = parse(src);
        let links: Vec<&InlineNode> = ns.iter().filter(|n| n.kind == InlineKind::Link).collect();
        assert_eq!(links.len(), 3, "自动链接 + 两个裸链接: {ns:?}");
        assert_eq!(links[0].range.end, 19);
        assert_eq!(
            &src[links[1].range.start..links[1].range.end],
            "https://b.example"
        );
        assert_eq!(
            &src[links[2].range.start..links[2].range.end],
            "www.example.com"
        );
        // 对照：普通文本中的 `>` 不影响其后裸 URL。
        let ns = parse("x > y https://b.example");
        assert_eq!(ns.iter().filter(|n| n.kind == InlineKind::Link).count(), 1);
    }

    #[test]
    fn escaped_dollar_is_not_math_closer() {
        // TASK04-D005：`\$` 是转义美元，不是闭定界符。
        let ns = parse("$6 \\$");
        assert!(
            ns.iter().all(|n| n.kind != InlineKind::Math),
            "$6 \\$ 不应形成 Math: {ns:?}"
        );
        // 对照：未转义 `$` 仍闭合。
        assert_eq!(sexp(&parse("$x$")), "Math[0..3 c=1..2]");
        // 两个反斜杠是字面反斜杠 + 真定界符。
        assert_eq!(sexp(&parse("$x\\\\$")), "Math[0..5 c=1..4]");
    }

    #[test]
    fn inline_details_open_attribute_is_preserved() {
        // TASK04-D008：布尔属性 `open` 必须记录（导出层写回 `<details open>`）。
        let ns = parse("<details open><summary>S</summary>body</details>");
        match &ns[0].kind {
            InlineKind::Html { tag, attrs } => {
                assert_eq!(*tag, HtmlTag::Details);
                assert!(attrs.open, "open 属性应保留");
            }
            other => panic!("expected Html Details, got {other:?}"),
        }
        let ns = parse("<details><summary>S</summary>body</details>");
        match &ns[0].kind {
            InlineKind::Html { attrs, .. } => assert!(!attrs.open),
            other => panic!("expected Html Details, got {other:?}"),
        }
    }

    #[test]
    fn bare_email_autolink() {
        for src in ["foo@bar.baz", "hello+xyz@mail.example", "a.b-c_d@a.b"] {
            let ns = parse(src);
            let link = ns.iter().find(|n| n.kind == InlineKind::Link).unwrap();
            assert_eq!(link.url.unwrap().end, src.len(), "{src} 应整体成链");
        }
        // 域名含 `+` 非法；尾部 `-` / `_` 非法；句末 `.` 不属于链接
        for src in [
            "hello@mail+xyz.example",
            "a.b-c_d@a.b-",
            "a.b-c_d@a.b_",
            "foo@bar",
        ] {
            assert_eq!(sexp(&parse(src)), "", "{src} 不应成链");
        }
        let ns = parse("a.b-c_d@a.b.");
        let link = ns.iter().find(|n| n.kind == InlineKind::Link).unwrap();
        assert_eq!(link.url.unwrap().end, 11);
        // 代码段/尖括号内不接管
        assert_eq!(sexp(&parse("`foo@bar.baz`")), "Code[0..13 c=1..12]");
        assert_eq!(
            sexp(&parse("<foo@bar.baz>")),
            "Link[0..13 c=1..12 url=1..12]"
        );
        // 已有 scheme 前缀（`mailto:`）不当作裸邮箱
        assert_eq!(sexp(&parse("mailto:foo@bar.baz")), "");
    }

    #[test]
    fn link_dest_with_title() {
        assert_eq!(
            sexp(&parse("[t](/url \"title\")")),
            "Link[0..17 c=1..2 url=4..8]"
        );
        assert_eq!(
            sexp(&parse("[t](/url 'title')")),
            "Link[0..17 c=1..2 url=4..8]"
        );
        assert_eq!(
            sexp(&parse("[t](/url (title))")),
            "Link[0..17 c=1..2 url=4..8]"
        );
        // #0508：双引号 title 内含未转义 `"`
        assert_eq!(sexp(&parse("[t](/url \"title \"and\" title\")")), "");
    }

    #[test]
    fn link_dest_empty_allowed() {
        assert_eq!(sexp(&parse("[t]()")), "Link[0..5 c=1..2 url=4..4]");
    }

    #[test]
    fn link_dest_multiline_whitespace() {
        // #0510：`](` 与目标之间、目标与 title 之间允许换行
        assert_eq!(
            sexp(&parse("[link](   /uri\n  \"title\"  )")),
            "Link[0..27 c=1..5 url=10..14]"
        );
    }

    #[test]
    fn link_dest_invalid_falls_back_to_shortcut() {
        // #0568：`[foo](not a link)` URL 含空格 → inline 链接失败，shortcut 查表
        let mut refs = RefMap::new();
        refs.insert("foo".into(), (crate::range::Range::new(21, 26), None));
        let nodes = parse_inlines("[foo](not a link)", 0, &refs);
        assert_eq!(sexp(&nodes), "Link[0..5 c=1..4 url=21..26]");
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
        // CommonMark：image alt 为纯文本，不产生子节点
        assert_eq!(sexp(&parse("![**b**](u)")), "Image[0..11 c=2..7 url=9..10]");
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
    fn image_fullwidth_delimiters() {
        // 全角开闭：`!` `[` 图(3B) `]` `（`(3B) `a.png` `）`(3B)
        assert_eq!(
            sexp(&parse("![图]（a.png）")),
            "Image[0..17 c=2..5 url=9..14]"
        );
    }

    #[test]
    fn image_fullwidth_mixed_with_halfwidth() {
        // 只换开定界符：`（` 3B 与半角 `)` 混用
        assert_eq!(
            sexp(&parse("![图]（a.png)")),
            "Image[0..15 c=2..5 url=9..14]"
        );
        // 只换闭定界符：半角 `(` 与 `）` 3B 混用
        assert_eq!(
            sexp(&parse("![图](a.png）")),
            "Image[0..15 c=2..5 url=7..12]"
        );
    }

    #[test]
    fn image_fullwidth_requires_paired_closer() {
        // 只有开定界符全角、没有闭合符：仍是普通文字（不吞后续内容）
        assert_eq!(sexp(&parse("![图]（a.png")), "");
        assert!(find_image(&parse("![图]（a.png")).is_none());
    }

    #[test]
    fn plain_fullwidth_parentheses_are_not_images() {
        // 普通正文里的全角括号不得被当成图片定界符
        assert_eq!(sexp(&parse("这是（说明）文字")), "");
        assert!(find_image(&parse("这是（说明）文字")).is_none());
        // 强调内的全角括号同理
        assert!(find_image(&parse("**（强调）**")).is_none());
        // 纯 ASCII 图片目标含空格仍按 CommonMark 语义降级为文字（不受本扩展影响）
        assert!(find_image(&parse("![a](x y)")).is_none());
    }

    #[test]
    fn image_fullwidth_alt_and_dest_with_spaces() {
        // alt 含空格、目标含空格：alt/dest 区间须各自正确切出
        let nodes = parse("![未命名 图]（a b.png）");
        assert_eq!(sexp(&nodes), "Image[0..29 c=2..15 url=19..26]");
        let img = nodes.first().expect("image node");
        assert_eq!(&"![未命名 图]（a b.png）"[2..15], "未命名 图");
        assert_eq!(&"![未命名 图]（a b.png）"[19..26], "a b.png");
        assert_eq!(img.kind, InlineKind::Image);
    }

    #[test]
    fn escaped_bang_is_literal() {
        // `\!` 转义后余下 `[a](u)` 按普通 Link 解析
        assert_eq!(sexp(&parse("\\![a](u)")), "Link[2..8 c=3..4 url=6..7]");
    }

    // ===== Highlight（N3c）：`==text==` =====

    #[test]
    fn highlight_basic() {
        assert_eq!(sexp(&parse("a ==hi== c")), "Highlight[2..8 c=4..6]");
    }

    #[test]
    fn highlight_nests_strong_and_code() {
        assert_eq!(
            sexp(&parse("==**b** `c`==")),
            "Highlight[0..13 c=2..11 Strong[2..7 c=4..5] Code[8..11 c=9..10]]"
        );
    }

    #[test]
    fn highlight_empty_single_or_triple_equals_is_text() {
        assert_eq!(sexp(&parse("==a")), ""); // 未闭合
        assert_eq!(sexp(&parse("====")), ""); // `===` 不是开标记（空内容同理）
        assert_eq!(sexp(&parse("a =x= b")), ""); // 单 `=` 为纯文本
        assert_eq!(sexp(&parse("===x===")), ""); // 三连 `=` 边界不成节点
                                                 // 三连边界不吞掉后续正常高亮
        assert_eq!(sexp(&parse("=== a ==b==")), "Highlight[6..11 c=8..9]");
    }

    #[test]
    fn escaped_equals_is_literal() {
        assert_eq!(sexp(&parse("\\==x==")), "");
        assert_eq!(sexp(&parse("\\=\\=x\\=\\=")), "");
    }

    #[test]
    fn highlight_multibyte_offsets_are_bytes() {
        assert_eq!(sexp(&parse("中文 ==亮==")), "Highlight[7..14 c=9..12]");
    }

    #[test]
    fn highlight_adjacent_leftmost() {
        assert_eq!(sexp(&parse("==a ==b==")), "Highlight[0..6 c=2..4]");
    }

    #[test]
    fn highlight_stop_bounds_bracket_pairs() {
        assert_eq!(
            sexp(&parse("==a [b c== d](url)==")),
            "Highlight[0..10 c=2..8]"
        );
    }

    // ===== Strikethrough（P3-4）：`~~del~~` =====

    #[test]
    fn strikethrough_basic() {
        assert_eq!(sexp(&parse("a ~~del~~ c")), "Strikethrough[2..9 c=4..7]");
    }

    #[test]
    fn strikethrough_nests_strong() {
        assert_eq!(
            sexp(&parse("~~a **b** c~~")),
            "Strikethrough[0..13 c=2..11 Strong[4..9 c=6..7]]"
        );
    }

    #[test]
    fn strikethrough_unclosed_empty_or_single_tilde_is_text() {
        assert_eq!(sexp(&parse("~~a")), ""); // 未闭合
        assert_eq!(sexp(&parse("~~~~")), ""); // 空内容
        assert_eq!(sexp(&parse("a ~x~ b")), ""); // 单 `~` 为纯文本
        assert_eq!(sexp(&parse("a ~1 b ~2")), ""); // 价格风单波浪线不误判
    }

    #[test]
    fn escaped_tilde_is_literal() {
        assert_eq!(sexp(&parse("\\~\\~x")), ""); // `\~` 两处均为字面文本
    }

    #[test]
    fn strikethrough_adjacent_leftmost() {
        // 最左匹配：`~~a ~~b~~` 先在内部 `~~` 闭合（content "a "），
        // 余下 `b~~` 的开标记未闭合为纯文本——与 strong 的已知分歧一致
        assert_eq!(sexp(&parse("~~a ~~b~~")), "Strikethrough[0..6 c=2..4]");
    }

    #[test]
    fn strikethrough_multibyte_offsets_are_bytes() {
        // `中文 ` 7 字节；删 = 3 字节
        assert_eq!(sexp(&parse("中文 ~~删~~")), "Strikethrough[7..14 c=9..12]");
    }

    #[test]
    fn strikethrough_stop_bounds_bracket_pairs() {
        // 括号配对不得跨越本层 stop：`[b` 的 `]` 在 `~~` 之后 → 不成对、不成链，
        // 删除线在第一个 `~~` 闭合（内容 "a [b c"），其后 ` d](url)~~` 全为纯文本
        assert_eq!(
            sexp(&parse("~~a [b c~~ d](url)~~")),
            "Strikethrough[0..10 c=2..8]"
        );
    }

    #[test]
    fn strikethrough_still_allows_link_inside() {
        // 现状不得回退：stop 之内的 `[x](u)` 仍正常成链
        assert_eq!(
            sexp(&parse("~~[x](u)~~")),
            "Strikethrough[0..10 c=2..8 Link[2..8 c=3..4 url=6..7]]"
        );
    }

    #[test]
    fn code_span_wins_over_strikethrough_stop() {
        // 代码段优先级高于删除线：首个 `~~` 是代码段内容，闭标记须取最后的 `~~`
        assert_eq!(
            sexp(&parse("~~a `b~~` c~~")),
            "Strikethrough[0..13 c=2..11 Code[4..9 c=5..8]]"
        );
    }

    #[test]
    fn nested_strikethrough_limits_are_per_layer() {
        // 每层各求 limit：外层先在自己的 stop 处闭合，其后 `[a](u)` 在顶层正常成链
        assert_eq!(
            sexp(&parse("~~x ~~y [a](u)~~")),
            "Strikethrough[0..6 c=2..4] Link[8..14 c=9..10 url=12..13]"
        );
    }

    // ===== CommonMark delimiter-run 语义（0c P1，语料 Emphasis 段锚点） =====

    #[test]
    fn flanking_open_followed_by_whitespace_is_text() {
        // #0351/#0379：开标记后随空白 → 不可开
        assert_eq!(sexp(&parse("a * foo bar*")), "");
        assert_eq!(sexp(&parse("** foo bar**")), "");
        assert_eq!(sexp(&parse("*foo bar\n*")), ""); // #0367：闭标记前换行（空白）
    }

    #[test]
    fn flanking_close_preceded_by_whitespace_is_text() {
        // #0366/#0391：闭标记前为空白 → 不可闭
        assert_eq!(sexp(&parse("*foo bar *")), "");
        assert_eq!(sexp(&parse("**foo bar **")), "");
    }

    #[test]
    fn flanking_punctuation_rules() {
        // #0352/#0380：开标记后随标点且前为字母（非空白非标点）→ 不可开
        assert_eq!(sexp(&parse("a*\"foo\"*")), "");
        assert_eq!(sexp(&parse("a**\"foo\"**")), "");
        // #0368/#0392：第二个 run 前为 `(` 标点、后为字母 → 不可开不可闭
        assert_eq!(sexp(&parse("*(*foo)")), "");
        assert_eq!(sexp(&parse("**(**foo)")), "");
        // #0354：`$`/`£`/`€` 属标点（Unicode S 类）→ 不可开/不可闭
        assert_eq!(sexp(&parse("*$*alpha.")), "");
        assert_eq!(sexp(&parse("*£*bravo.")), "");
        assert_eq!(sexp(&parse("*€*charlie.")), "");
    }

    #[test]
    fn underscore_basic_and_intraword_boundary() {
        // #0357：单词边界 `_` 正常成 Emph
        assert_eq!(sexp(&parse("_foo bar_")), "Emph[0..9 c=1..8]");
        // #0364：开 `_` 前为 `-` 标点 → 允许；闭 `_` 后为行尾
        assert_eq!(sexp(&parse("foo-_(bar)_")), "Emph[4..11 c=5..10]");
        // #0376：词内 `_`（两侧皆字母）不可开不可闭，外圈成对
        assert_eq!(sexp(&parse("_foo_bar_baz_")), "Emph[0..13 c=1..12]");
    }

    #[test]
    fn intraword_underscore_neither_opens_nor_closes() {
        assert_eq!(sexp(&parse("foo_bar_baz_")), "");
        assert_eq!(sexp(&parse("a_b_c")), "");
    }

    #[test]
    fn double_underscore_strong() {
        // #0382/#0402：词内 `__`（双侧皆字母）不可开闭 → 仅最外圈成对
        assert_eq!(sexp(&parse("__foo__")), "Strong[0..7 c=2..5]");
        assert_eq!(sexp(&parse("__foo__bar__baz__")), "Strong[0..17 c=2..15]");
    }

    #[test]
    fn mixed_runs_split_emph_over_strong() {
        // #0350 族：`***foo***` 就近消费 → 外层 Emph（run 首字符）套内层 Strong
        assert_eq!(
            sexp(&parse("***foo***")),
            "Emph[0..9 c=1..8 Strong[1..8 c=3..6]]"
        );
        // #0413：开 run 长 3 拆为 2（strong，靠内容侧）+ 1（emph，靠外侧）；串长 13
        assert_eq!(
            sexp(&parse("***foo** bar*")),
            "Emph[0..13 c=1..12 Strong[1..8 c=3..6]]"
        );
    }

    #[test]
    fn run_of_four_nests_strong() {
        // #0464：`****foo****` → 两层嵌套 Strong
        assert_eq!(
            sexp(&parse("****foo****")),
            "Strong[0..11 c=2..9 Strong[2..9 c=4..7]]"
        );
    }

    #[test]
    fn rule_of_three_blocks_match() {
        // #0412：`*foo**bar*` 内 `**`（可开可闭）与外 `*` 长度和为 3 的倍数 → 不匹配
        assert_eq!(sexp(&parse("*foo**bar*")), "Emph[0..10 c=1..9]");
        // 开 run 余量消费自内容侧：`**a*` → `*` + Emph
        assert_eq!(sexp(&parse("**a*")), "Emph[1..4 c=2..3]");
    }

    #[test]
    fn nested_emph_star() {
        // #0409：闭 run 长度 2 拆两次，就近匹配 → 嵌套 Emph
        assert_eq!(
            sexp(&parse("*foo *bar**")),
            "Emph[0..11 c=1..10 Emph[5..10 c=6..9]]"
        );
    }

    #[test]
    fn emph_wraps_link_atom_and_link_text_resolves_emph() {
        // 强调可跨链接原子成对；链接文本内部的强调在递归段内解析
        assert_eq!(
            sexp(&parse("*[a](u)*")),
            "Emph[0..8 c=1..7 Link[1..7 c=2..3 url=5..6]]"
        );
        assert_eq!(
            sexp(&parse("[*a*](u)")),
            "Link[0..8 c=1..4 url=6..7 Emph[1..4 c=2..3]]"
        );
    }

    #[test]
    fn escaped_underscore_is_literal() {
        // 可转义集扩展至全部 ASCII 标点（CommonMark）：`\_` 不成标记
        assert_eq!(sexp(&parse("\\_a\\_")), "");
        // #0449：转义符自身为内容，外圈 `_` 成对
        assert_eq!(sexp(&parse("foo _\\__")), "Emph[4..8 c=5..7]");
    }

    #[test]
    fn cjk_punctuation_flanking() {
        // CJK 标点按 Unicode P 类参与 flanking：`*强调。*` 闭标记前为 `。`（标点）、
        // 后为行尾（空白）→ 可闭
        assert_eq!(sexp(&parse("*强调。*")), "Emph[0..11 c=1..10]");
        // `「`（Ps）前导不影响开标记判定
        assert_eq!(sexp(&parse("「*a*」")), "Emph[3..6 c=4..5]");
    }

    #[test]
    fn intraword_star_allowed() {
        // `*` 无 intraword 限制：`a*b*c*` 首个配对成 Emph
        assert_eq!(sexp(&parse("a*b*c*")), "Emph[1..4 c=2..3]");
    }

    // ---- 批次 2-1b：括号索引预扫回归测试 ----

    #[test]
    fn bracket_index_matched_link_and_image() {
        // 合法链接/图片——括号索引能正确配对，pair_resolves 能正确识别
        assert_eq!(sexp(&parse("[a](b)")), "Link[0..6 c=1..2 url=4..5]");
        assert_eq!(sexp(&parse("![a](b)")), "Image[0..7 c=2..3 url=5..6]");
        // 未配对 [ —— bracket_pairs 返回 None → try_link 返回 None → 按裸文本
        assert_eq!(sexp(&parse("[[[")), "");
        assert_eq!(sexp(&parse("a [b c")), "");
    }

    #[test]
    fn bracket_index_image_inner_not_killed() {
        // 图片开括号（内层）永不淘汰外层链接开括号
        // "[![a](b)]" —— 外层 [ 配 ]，内层 ![a](b) 成图（内层是图片开括号，不淘汰外层）
        assert_eq!(
            sexp(&parse("[![a](b)]")),
            "Image[1..8 c=3..4 url=6..7]" // 内层图片命中后淘汰了外层链接（内层是链接？不对）
        );
    }

    #[test]
    fn bracket_index_unpaired_perf() {
        // 性能断言：10000 个未配对 [ —— 括号索引让 O(N·n) 变成 O(n)
        let s = "[".repeat(10_000);
        let start = std::time::Instant::now();
        let nodes = parse_inlines(&s, 0, &RefMap::new());
        let elapsed = start.elapsed();
        // 全是裸文本，不应产生任何节点
        assert!(nodes.is_empty(), "未配对 [ 不应产生节点");
        // ≤ 50ms：预扫一次 O(n) 就能解决所有未配对括号
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "10k 未配对 [ inline 解析 {}ms 超过 50ms 预算，O(n²) 仍存在",
            elapsed.as_millis()
        );
    }

    // ---- 批次 2-2：反引号 run 索引回归测试 ----

    #[test]
    fn code_span_basic_and_nested() {
        assert_eq!(sexp(&parse("`code`")), "Code[0..6 c=1..5]");
        assert_eq!(sexp(&parse("``")), ""); // 空代码段
        assert_eq!(sexp(&parse("```foo``")), ""); // 不等长 run（Example 347）
                                                  // 独立代码段 + 后续文本；node.end = close_pos + open = 3
        assert_eq!(sexp(&parse("`a`b`")), "Code[0..3 c=1..2]");
    }

    #[test]
    fn code_span_unmatched_perf() {
        // 性能断言：512KB 含不匹配反引号 run
        // O(n) 预扫 + O(runs) 查找，73ms 是线性水平（原 O(n²) 是秒级）
        let s = format!("`{}", "x".repeat(256 * 1024));
        let start = std::time::Instant::now();
        let _ = parse_inlines(&s, 0, &RefMap::new());
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "512KB 不匹配反引号 inline 解析 {}ms 超过 500ms 预算",
            elapsed.as_millis()
        );
    }

    // ===== 批次 3：行内 HTML 白名单 =====

    #[test]
    fn html_whitelist_pair_basic() {
        // 两个独立的 <kbd> —— 都被解析成 Html(Kbd)
        let ns = parse("<kbd>Ctrl</kbd>+<kbd>A</kbd>");
        assert_eq!(ns.len(), 2);
        assert!(matches!(&ns[0].kind, InlineKind::Html { tag, .. } if *tag == HtmlTag::Kbd));
        assert!(matches!(&ns[1].kind, InlineKind::Html { tag, .. } if *tag == HtmlTag::Kbd));
    }

    #[test]
    fn html_whitelist_void_self_closed() {
        // <br> 硬换行 —— HtmlSelfClosed(Br)
        let ns = parse("foo<br>bar");
        assert_eq!(ns.len(), 1);
        assert!(
            matches!(&ns[0].kind, InlineKind::HtmlSelfClosed { tag, .. } if *tag == HtmlTag::Br)
        );
        // 显式自闭合写法也接受
        let ns = parse("a<br/>b");
        assert_eq!(ns.len(), 1);
        assert!(
            matches!(&ns[0].kind, InlineKind::HtmlSelfClosed { tag, .. } if *tag == HtmlTag::Br)
        );
    }

    #[test]
    fn html_whitelist_nested_sub_inside_kbd() {
        // <kbd>a<sub>b</sub>c</kbd> —— 子节点里嵌 InlineKind::Html(Sub)
        let ns = parse("<kbd>a<sub>b</sub>c</kbd>");
        assert_eq!(ns.len(), 1);
        let kbd = &ns[0];
        assert!(matches!(&kbd.kind, InlineKind::Html { tag, .. } if *tag == HtmlTag::Kbd));
        // 子节点里有 InlineKind::Html(Sub)
        assert!(kbd
            .children
            .iter()
            .any(|c| matches!(&c.kind, InlineKind::Html { tag, .. } if *tag == HtmlTag::Sub)));
    }

    #[test]
    fn html_unclosed_pair_falls_back_to_literal() {
        // 未闭合 <kbd> —— 整段退成 HtmlLiteral
        let ns = parse("<kbd>未闭合文本");
        assert_eq!(ns.len(), 1);
        assert!(matches!(&ns[0].kind, InlineKind::HtmlLiteral));
    }

    #[test]
    fn html_crossed_tags_falls_back_to_literal() {
        // <kbd>交叉</sub> —— 找不到 </kbd>，整段退成 HtmlLiteral
        let ns = parse("<kbd>交叉</sub>x");
        assert_eq!(ns.len(), 1);
        assert!(matches!(&ns[0].kind, InlineKind::HtmlLiteral));
    }

    #[test]
    fn html_off_whitelist_passthrough_autolink() {
        // <div>不在白名单 —— try_html 返回 None，标签被跳过，整体按纯文本
        // 但我们无法直接断言 sexp（因为没有节点产出），而是断言没有 InlineKind::Html
        let ns = parse("<div>x</div>");
        assert!(ns
            .iter()
            .all(|n| !matches!(&n.kind, InlineKind::Html { .. })));
        // <script>alert(1)</script> —— 脚本白名单外，不是行内 Html
        let ns = parse("<script>alert(1)</script>");
        assert!(ns
            .iter()
            .all(|n| !matches!(&n.kind, InlineKind::Html { .. } | InlineKind::HtmlLiteral)));
    }

    #[test]
    fn html_remote_img_src_stripped() {
        // 远程 src 不写入 src（保持为空），但保留在 remote_src 供 UI 加载
        let ns = parse(r#"<img src="http://evil/x.png" alt="bad">"#);
        assert_eq!(ns.len(), 1);
        match &ns[0].kind {
            InlineKind::HtmlSelfClosed { tag, attrs } => {
                assert_eq!(*tag, HtmlTag::Img);
                assert!(attrs.src.is_none(), "远程 src 应被清空");
                assert_eq!(
                    attrs.remote_src.as_deref(),
                    Some("http://evil/x.png"),
                    "远程地址应保留在 remote_src"
                );
                assert_eq!(attrs.alt.as_deref(), Some("bad"));
            }
            _ => panic!("expected HtmlSelfClosed Img"),
        }
        // https 大小写不敏感
        let ns = parse(r#"<img src="HTTPS://Example.COM/a.png">"#);
        match &ns[0].kind {
            InlineKind::HtmlSelfClosed { attrs, .. } => {
                assert!(attrs.src.is_none(), "HTTPS 大写也应视为远程");
                assert_eq!(
                    attrs.remote_src.as_deref(),
                    Some("HTTPS://Example.COM/a.png")
                );
            }
            _ => panic!(),
        }
        // 本地 src 保留，remote_src 不设置
        let ns = parse(r#"<img src="local.png" alt="ok">"#);
        match &ns[0].kind {
            InlineKind::HtmlSelfClosed { attrs, .. } => {
                assert_eq!(attrs.src.as_deref(), Some("local.png"));
                assert!(attrs.remote_src.is_none(), "本地地址不应进 remote_src");
                assert_eq!(attrs.alt.as_deref(), Some("ok"));
            }
            _ => panic!(),
        }
        // 危险/相对 scheme 均不视为远程
        for input in [
            r#"<img src="javascript:alert(1)">"#,
            r#"<img src="data:image/png;base64,AA">"#,
            r#"<img src="file:///C:/a.png">"#,
            r#"<img src="./a.png">"#,
        ] {
            let ns = parse(input);
            match &ns[0].kind {
                InlineKind::HtmlSelfClosed { attrs, .. } => {
                    assert!(attrs.remote_src.is_none(), "{input} 不应进 remote_src");
                }
                _ => panic!("expected HtmlSelfClosed Img for {input}"),
            }
        }
    }

    #[test]
    fn html_img_explicit_size_parsed() {
        // 合法：纯十进制数字（含无引号与首尾空白）
        let ns = parse(r#"<img src="a.png" width="120" height=80>"#);
        match &ns[0].kind {
            InlineKind::HtmlSelfClosed { attrs, .. } => {
                assert_eq!(attrs.width, Some(120));
                assert_eq!(attrs.height, Some(80));
            }
            _ => panic!("expected HtmlSelfClosed Img"),
        }
        // 只给宽：高保持 None（由 UI 按原图比例推高）
        let ns = parse(r#"<img src="a.png" width="120">"#);
        match &ns[0].kind {
            InlineKind::HtmlSelfClosed { attrs, .. } => {
                assert_eq!(attrs.width, Some(120));
                assert_eq!(attrs.height, None);
            }
            _ => panic!("expected HtmlSelfClosed Img"),
        }
        // 非法值：单位/百分比/负数/零/非数字/溢出 → 忽略该属性
        for (input, want_w, want_h) in [
            (r#"<img src="a.png" width="100px">"#, None, None),
            (r#"<img src="a.png" width="50%">"#, None, None),
            (r#"<img src="a.png" width="-5">"#, None, None),
            (r#"<img src="a.png" width="0" height="0">"#, None, None),
            (r#"<img src="a.png" width="abc">"#, None, None),
            (r#"<img src="a.png" height="99999999999">"#, None, None),
            (r#"<img src="a.png" width="">"#, None, None),
            // 一个合法一个非法：只保留合法那个
            (
                r#"<img src="a.png" width="64" height="4x">"#,
                Some(64),
                None,
            ),
        ] {
            match &parse(input)[0].kind {
                InlineKind::HtmlSelfClosed { attrs, .. } => {
                    assert_eq!(attrs.width, want_w, "{input} width");
                    assert_eq!(attrs.height, want_h, "{input} height");
                }
                _ => panic!("expected HtmlSelfClosed Img for {input}"),
            }
        }
        // 非 img 标签不收集 width/height（白名单按标签收窄）
        let ns = parse(r#"<abbr width="120">x</abbr>"#);
        match &ns[0].kind {
            InlineKind::Html { attrs, .. } => assert_eq!(attrs.width, None),
            _ => panic!("expected Html Abbr"),
        }
    }

    #[test]
    fn html_img_align_collected_only_for_whitelisted_values() {
        for (input, want) in [
            (r#"<img src="a.png" align="left">"#, Some(ImageAlign::Left)),
            (
                r#"<img src="a.png" align="CENTER">"#,
                Some(ImageAlign::Center),
            ),
            (
                r#"<img src="a.png" align="centre">"#,
                Some(ImageAlign::Center),
            ),
            (
                r#"<img src="a.png" align=" right ">"#,
                Some(ImageAlign::Right),
            ),
        ] {
            match &parse(input)[0].kind {
                InlineKind::HtmlSelfClosed { attrs, .. } => {
                    assert_eq!(attrs.align, want, "{input}");
                }
                _ => panic!("expected HtmlSelfClosed Img for {input}"),
            }
        }
        // 未知值/空值不收集，标签本身仍生效（不破坏白名单）。
        for input in [
            r#"<img src="a.png" align="justify">"#,
            r#"<img src="a.png" align="center; color:red">"#,
            r#"<img src="a.png" align="">"#,
        ] {
            match &parse(input)[0].kind {
                InlineKind::HtmlSelfClosed { attrs, .. } => {
                    assert_eq!(attrs.align, None, "{input}")
                }
                _ => panic!("expected HtmlSelfClosed Img for {input}"),
            }
        }
        // 非 img 标签不收集 align。
        match &parse(r#"<span align="center">x</span>"#)[0].kind {
            InlineKind::Html { attrs, .. } => assert_eq!(attrs.align, None),
            _ => panic!("expected Html Span"),
        }
    }

    #[test]
    fn html_abbr_title_collected() {
        let ns = parse(r#"<abbr title="World Wide Web">WWW</abbr>"#);
        assert_eq!(ns.len(), 1);
        match &ns[0].kind {
            InlineKind::Html { tag, attrs } => {
                assert_eq!(*tag, HtmlTag::Abbr);
                assert_eq!(attrs.title.as_deref(), Some("World Wide Web"));
            }
            _ => panic!("expected Html Abbr"),
        }
    }

    #[test]
    fn html_event_and_style_attrs_ignored() {
        // onclick / style 属性应被忽略；abbr 正常 title 仍被收集
        let input = r#"<abbr onclick="x" style="y" title="T">a</abbr>"#;
        let ns = parse(input);
        eprintln!("ns.len() = {}", ns.len());
        for n in &ns {
            eprintln!("kind = {:?}", n.kind);
        }
        assert_eq!(ns.len(), 1);
        match &ns[0].kind {
            InlineKind::Html { attrs, .. } => {
                eprintln!("attrs.title = {:?}", attrs.title);
                assert_eq!(attrs.title.as_deref(), Some("T"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn html_semantic_tags_whitelisted() {
        // 用户真机截图暴露的常用语义标签：<strong style="color:red">VISUAL</strong>
        let ns = parse(r#"<strong style="color: red">VISUAL</strong>"#);
        match &ns[0].kind {
            InlineKind::Html { tag, attrs } => {
                assert_eq!(*tag, HtmlTag::Strong);
                assert_eq!(attrs.style.as_deref(), Some("color:red"));
            }
            _ => panic!("expected Html Strong"),
        }
        for (src, want) in [
            ("<b>x</b>", HtmlTag::B),
            ("<em>x</em>", HtmlTag::Em),
            ("<i>x</i>", HtmlTag::I),
            ("<u>x</u>", HtmlTag::U),
            ("<s>x</s>", HtmlTag::S),
            ("<del>x</del>", HtmlTag::Del),
            ("<ins>x</ins>", HtmlTag::Ins),
            ("<q>x</q>", HtmlTag::Q),
            ("<cite>x</cite>", HtmlTag::Cite),
            ("<var>x</var>", HtmlTag::Var),
            ("<samp>x</samp>", HtmlTag::Samp),
            ("<small>x</small>", HtmlTag::Small),
            ("<span>x</span>", HtmlTag::Span),
        ] {
            let ns = parse(src);
            assert_eq!(ns.len(), 1, "{src} 应解析为单个节点");
            assert!(
                matches!(&ns[0].kind, InlineKind::Html { tag, .. } if *tag == want),
                "{src} 应解析为 {want:?}"
            );
        }
    }

    #[test]
    fn html_style_legal_values_kept() {
        let cases = [
            ("#f00", "#f00"),
            ("#ff0000", "#ff0000"),
            ("rgb(1,2,3)", "rgb(1,2,3)"),
            ("rgba(1,2,3,0.5)", "rgba(1,2,3,0.5)"),
            ("hsl(120,50%,50%)", "hsl(120,50%,50%)"),
            ("red", "red"),
        ];
        for (v, want) in cases {
            let src = format!(r#"<span style="color:{v}">x</span>"#);
            let ns = parse(&src);
            match &ns[0].kind {
                InlineKind::Html { attrs, .. } => {
                    assert_eq!(attrs.style.as_deref(), Some(&*format!("color:{want}")))
                }
                _ => panic!("expected Html Span"),
            }
        }
        let ns = parse(
            r#"<span style="font-weight:bold;font-style:italic;text-decoration:underline line-through;font-size:14px;font-family:'Segoe UI', Arial">x</span>"#,
        );
        match &ns[0].kind {
            InlineKind::Html { attrs, .. } => {
                let st = veil_core_style(attrs);
                assert!(st.bold && st.italic && st.underline && st.strike);
                assert_eq!(st.size_px, Some(14.0));
            }
            _ => panic!("expected Html Span"),
        }
    }

    #[test]
    fn html_style_injections_and_unknown_dropped() {
        // 危险片段/未知属性逐条丢弃，合法声明保留，不整段降级为字面量。
        let ns = parse(
            r#"<strong style="color:red;position:fixed;background-image:url(javascript:x);expression(y);z-index:9">V</strong>"#,
        );
        match &ns[0].kind {
            InlineKind::Html { tag, attrs } => {
                assert_eq!(*tag, HtmlTag::Strong, "标签本身必须保留");
                assert_eq!(attrs.style.as_deref(), Some("color:red"));
                let st = veil_core_style(attrs);
                assert_eq!(st.color, Some([255, 0, 0]));
            }
            _ => panic!("expected Html Strong"),
        }
        // 全部非法 → 无 style 属性
        let ns = parse(r#"<span style="position:fixed;content:'x'">x</span>"#);
        match &ns[0].kind {
            InlineKind::Html { attrs, .. } => assert!(attrs.style.is_none()),
            _ => panic!("expected Html Span"),
        }
    }

    #[test]
    fn html_style_gt_inside_quotes_does_not_truncate() {
        // style 值内含 `>`、分号、引号：标签边界必须按引号状态判定。
        let src = r#"<span style="font-family:'a>b';color:red">X</span>Y"#;
        let ns = parse(src);
        assert_eq!(ns.len(), 1, "应只产出 span 节点（Y 是普通文本）");
        match &ns[0].kind {
            InlineKind::Html { tag, attrs } => {
                assert_eq!(*tag, HtmlTag::Span);
                assert_eq!(ns[0].range.end, src.find("</span>").unwrap() + 7);
                // `>` 出现在引号内不得截断标签；含 `>` 的 font-family 非法被丢弃，
                // 同一条 style 里的合法 color 声明必须保留。
                assert_eq!(
                    attrs.style.as_deref(),
                    Some("color:red"),
                    "style 解析不得被引号内 `>` 截断: {:?}",
                    attrs.style
                );
            }
            _ => panic!("expected Html Span"),
        }
    }

    #[test]
    fn html_semantic_tags_nest() {
        let ns = parse("<strong>a<em>b<u>c</u></em></strong>");
        assert_eq!(ns.len(), 1);
        let strong = &ns[0];
        assert!(matches!(&strong.kind, InlineKind::Html { tag, .. } if *tag == HtmlTag::Strong));
        let em = strong
            .children
            .iter()
            .find(|c| matches!(&c.kind, InlineKind::Html { tag, .. } if *tag == HtmlTag::Em))
            .expect("strong 内应有 em");
        assert!(em
            .children
            .iter()
            .any(|c| matches!(&c.kind, InlineKind::Html { tag, .. } if *tag == HtmlTag::U)));
    }

    fn veil_core_style(attrs: &HtmlAttrs) -> super::super::model::HtmlStyle {
        super::super::model::HtmlStyle::parse(attrs.style.as_deref().unwrap_or(""))
    }

    #[test]
    fn html_autolink_not_affected() {
        // try_html 在 autolink 形态（无空白 + 含 :/@）时应返回 None，交给 try_autolink
        assert_eq!(sexp(&parse("<a+b+c:d>")), "Link[0..9 c=1..8 url=1..8]");
        assert_eq!(sexp(&parse("<a@b.c>")), "Link[0..7 c=1..6 url=1..6]");
    }
}
