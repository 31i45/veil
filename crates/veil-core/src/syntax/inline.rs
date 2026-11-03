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
// - `\` 转义全部 ASCII 标点（CommonMark；0c P1 起由 v0 白名单扩展）。
// - Unicode 标点判定：ASCII 精确命中规范集合；非 ASCII 以「非字母数字且非空白」
//   近似 P/S 类（零依赖取舍：CJK 标点、货币符号均正确，仅 Mn/Cf 等记号类字符
//   邻接 run 时存在理论偏差）。flanking 的空白判定用 char::is_whitespace，
//   比规范的 Zs+\t\n\f\r 多出 \v/NEL，从宽。
// - 递归深度硬上限 MAX_INLINE_DEPTH（B5：超深嵌套不得栈溢出）；分隔符匹配为
//   迭代实现，不受递归上限影响。

use super::block::{normalize_label, RefMap};
use super::model::{InlineKind, InlineNode};
use crate::range::Range;

/// inline 解析递归深度硬上限。超限后开标记按纯文本处理；
/// 16 层远超正常写作嵌套，仅作恶意语料保险丝。
const MAX_INLINE_DEPTH: usize = 16;

/// 解析一段文本的 inline 节点，偏移以 `base` 为基准（文档字节偏移）。
pub(super) fn parse_inlines(text: &str, base: usize, refs: &RefMap) -> Vec<InlineNode> {
    InlineParser {
        s: text,
        base,
        refs,
        pos: 0,
        bracket_pairs: Vec::new(),
        resolved_prefix: Vec::new(),
        backtick_runs: Vec::new(),
    }
    .parse_until(None, true, 0)
}

/// CommonMark：反斜杠可转义一切 ASCII 标点（`!` 为 Image 开标记首字符，亦在内）。
fn is_escapable(c: char) -> bool {
    c.is_ascii_punctuation()
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

/// 解析 `](` 后的链接目标与可选 title（CommonMark §6.3）。
/// 返回 `(url_start, url_end, end)`：url 区间为内容（不含尖括号），end 为 `)` 之后。
/// 目标：`<...>`（无换行/未转义尖括号）或裸目标（无空格/换行/控制字符，括号须平衡）。
/// title：`"..."` / `'...'` / `(...)`（无换行，未转义闭标记结束）。
fn parse_link_dest(s: &str, start: usize) -> Option<(usize, usize, usize)> {
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
            match b[i] {
                b')' => {
                    if depth == 0 {
                        url_start = pos;
                        url_end = i;
                        pos = i;
                        break;
                    }
                    depth -= 1;
                    i += 1;
                }
                b'(' => {
                    depth += 1;
                    i += 1;
                }
                b'\\' if i + 1 < b.len() => i += 2,
                b' ' | b'\t' | b'\n' | b'\r' => {
                    url_start = pos;
                    url_end = i;
                    pos = i;
                    break;
                }
                c if c <= 0x1f || c == 0x7f => return None,
                _ => i += 1,
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
    } else {
        None
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
    s[lt..].find('>').map(|rel| lt + rel + 1)
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
    backtick_runs: Vec<(usize, usize)>,
}

impl<'a> InlineParser<'a> {
    /// 一次性预扫 self.s：建括号配对索引 + 内层淘汰判定前缀和 + 反引号 run 索引。
    /// O(n)：栈扫描，跳过转义/代码段/HTML（和 link_close 原跳过逻辑完全同源）。
    fn scan_bracket_pairs(&mut self) {
        let b = self.s.as_bytes();
        let n = b.len();
        let mut pairs: Vec<Option<usize>> = vec![None; n];
        let mut open_stack: Vec<usize> = Vec::new();
        let mut backticks: Vec<(usize, usize)> = Vec::new();
        let mut pos = 0;

        while pos < n {
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
                    backticks.push((pos, run));
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
                    let prev_is_bang =
                        i >= 1 && b[i - 1] == b'!' && (i < 2 || b[i - 2] != b'\\');
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

    /// 解析直到 `stop` 分隔符（留在原位不消费）或 EOF。
    /// 段内产出 token 流（原子节点 + 强调分隔符 run），收尾经阶段二匹配为节点。
    fn parse_until(
        &mut self,
        stop: Option<&'static str>,
        allow_link: bool,
        depth: usize,
    ) -> Vec<InlineNode> {
        self.scan_bracket_pairs();
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
                    .try_autolink()
                    .or_else(|| self.try_code_span())
                    .or_else(|| self.try_math())
                    .or_else(|| self.try_strikethrough(depth))
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
                    self.s[self.pos..].bytes().take_while(|&b| b == b'`').count()
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
            (left && (!right || prev_punct), right && (!left || next_punct))
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
    fn try_code_span(&mut self) -> Option<InlineNode> {
        let rest = &self.s[self.pos..];
        let open = rest.bytes().take_while(|&b| b == b'`').count();
        if open == 0 {
            return None;
        }
        let mut i = open;
        loop {
            let off = rest[i..].find('`')?;
            i += off;
            let run = rest[i..].bytes().take_while(|&b| b == b'`').count();
            if run == open {
                if i == open {
                    return None; // 空内容（` `` `）不成节点
                }
                let start = self.pos;
                let content_end = start + i;
                self.pos = content_end + run;
                return Some(self.node(
                    InlineKind::Code,
                    start,
                    start + open,
                    content_end,
                    None,
                    Vec::new(),
                ));
            }
            i += run;
        }
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

    /// `~~del~~`（GFM 删除线）：最近闭标记成节点，内容非空，子节点递归。
    /// 单个 `~` 为纯文本（下标语法不在 v0 范围）。
    fn try_strikethrough(&mut self, depth: usize) -> Option<InlineNode> {
        if !self.s[self.pos..].starts_with("~~") {
            return None;
        }
        let start = self.pos;
        self.pos = start + 2;
        let children = self.parse_until(Some("~~"), true, depth + 1);
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
            if let Some((url_s, url_e, end)) = parse_link_dest(self.s, after_open) {
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
    fn pair_resolves(&self, o: usize, p: usize) -> bool {
        let after = &self.s[p + 1..];
        if after.starts_with('(') && parse_link_dest(self.s, p + 2).is_some() {
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
            let src = if le == ls { &self.s[o + 1..p] } else { &self.s[ls..le] };
            return self.refs.contains_key(&normalize_label(src));
        }
        self.refs.contains_key(&normalize_label(&self.s[o + 1..p]))
    }

    /// `![alt](url)` / `![alt][label]` / `![alt][]` / `![alt]`：alt 可空、为纯文本
    /// （不产生子节点）；闭合 `]` 与链接同一套括号配对。未闭合或查无定义按纯文本。
    fn try_image(&mut self) -> Option<InlineNode> {
        if !self.s[self.pos..].starts_with("![") {
            return None;
        }
        let start = self.pos;
        let alt_end = self.bracket_pairs.get(start + 1).cloned().flatten()?;
        // 图片开括号 ours_is_image=true，内层成链不淘汰（inner_pair_kills_opener 恒 false）
        // 分支 1：`](` inline 图片
        if self.s[alt_end..].starts_with("](") {
            let after_open = alt_end + 2;
            if let Some((url_s, url_e, end)) = parse_link_dest(self.s, after_open) {
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
            let mut n = self.node(InlineKind::Image, start, start + 2, alt_end, None, Vec::new());
            n.url = Some(dest);
            return Some(n);
        }
        // 分支 3：shortcut 图片
        let &(dest, _) = self
            .refs
            .get(&normalize_label(&self.s[start + 2..alt_end]))?;
        self.pos = alt_end + 1;
        let mut n = self.node(InlineKind::Image, start, start + 2, alt_end, None, Vec::new());
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
        refs.insert(
            "foo".into(),
            (crate::range::Range::new(21, 26), None),
        );
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
        assert_eq!(
            sexp(&parse("![**b**](u)")),
            "Image[0..11 c=2..7 url=9..10]"
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
            "Image[1..8 c=3..4 url=6..7]"  // 内层图片命中后淘汰了外层链接（内层是链接？不对）
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
}
