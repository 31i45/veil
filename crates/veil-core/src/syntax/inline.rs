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
//   alt）。Image 可出现在 Link 文本内（徽章写法），alt 内亦可嵌套 inline（切片 13）；
//   引用式图片 `![alt][label]` / `![alt][]` / `![alt]` 同引用式链接（0c P0-1）。
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
    }
    .parse_until(None, true, 0)
}

/// CommonMark：反斜杠可转义一切 ASCII 标点（`!` 为 Image 开标记首字符，亦在内）。
fn is_escapable(c: char) -> bool {
    c.is_ascii_punctuation()
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

struct InlineParser<'a> {
    s: &'a str,
    base: usize,
    refs: &'a RefMap,
    pos: usize,
}

impl<'a> InlineParser<'a> {
    /// 解析直到 `stop` 分隔符（留在原位不消费）或 EOF。
    /// 段内产出 token 流（原子节点 + 强调分隔符 run），收尾经阶段二匹配为节点。
    fn parse_until(
        &mut self,
        stop: Option<&'static str>,
        allow_link: bool,
        depth: usize,
    ) -> Vec<InlineNode> {
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
                    .try_code_span()
                    .or_else(|| self.try_math())
                    .or_else(|| self.try_strikethrough(depth))
                    .or_else(|| self.try_image(depth))
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
            // （落为纯文本），循环顶部的 rest 已过期；run 在段尾时 pos 已至 EOF
            if self.pos < self.s.len() {
                self.pos += self.s[self.pos..].chars().next().unwrap().len_utf8();
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
    /// 文本可嵌套除 Link 外的任意 inline；inline 形式 URL 扫至首个 `)`。
    /// 引用式查文档引用表，url 指向定义处 dest 绝对区间；查无定义按纯文本。
    fn try_link(&mut self, depth: usize) -> Option<InlineNode> {
        if !self.s[self.pos..].starts_with('[') {
            return None;
        }
        let start = self.pos;
        self.pos = start + 1;
        let children = self.parse_until(Some("]"), false, depth + 1);
        let text_end = self.pos;
        if !self.s[text_end..].starts_with(']') {
            self.pos = start;
            return None;
        }
        // 分支 1：`](` inline 链接（优先于引用式）
        if self.s[text_end..].starts_with("](") {
            let url_start = text_end + 2;
            let rel = match self.s[url_start..].find(')') {
                Some(r) => r,
                None => {
                    self.pos = start;
                    return None;
                }
            };
            self.pos = url_start + rel + 1;
            return Some(self.node(
                InlineKind::Link,
                start,
                start + 1,
                text_end,
                Some((url_start, url_start + rel)),
                children,
            ));
        }
        // 分支 2：`][label]` full / `][]` collapsed（查无定义即纯文本，不再试 shortcut）
        if self.s[text_end..].starts_with("][") {
            let ls = text_end + 2;
            let Some(rel) = self.s[ls..].find(']') else {
                self.pos = start;
                return None;
            };
            let le = ls + rel;
            // label 不可含 `[`、长度 ≤999；空 label（collapsed）取链接文本
            if self.s[ls..le].contains('[') || le - ls > 999 {
                self.pos = start;
                return None;
            }
            let label_src = if le == ls {
                &self.s[start + 1..text_end]
            } else {
                &self.s[ls..le]
            };
            let Some(&(dest, _)) = self.refs.get(&normalize_label(label_src)) else {
                self.pos = start;
                return None;
            };
            self.pos = le + 1;
            let mut n = self.node(InlineKind::Link, start, start + 1, text_end, None, children);
            n.url = Some(dest); // 绝对区间：定义处 dest
            return Some(n);
        }
        // 分支 3：shortcut——以链接文本为 label 查表
        let Some(&(dest, _)) = self.refs.get(&normalize_label(&self.s[start + 1..text_end]))
        else {
            self.pos = start;
            return None;
        };
        self.pos = text_end + 1;
        let mut n = self.node(InlineKind::Link, start, start + 1, text_end, None, children);
        n.url = Some(dest);
        Some(n)
    }

    /// `![alt](url)` / `![alt][label]` / `![alt][]` / `![alt]`：alt 可空、可嵌套 inline。
    /// 未闭合、缺 `]` 或引用查无定义按纯文本处理。
    fn try_image(&mut self, depth: usize) -> Option<InlineNode> {
        if !self.s[self.pos..].starts_with("![") {
            return None;
        }
        let start = self.pos;
        self.pos = start + 2;
        let children = self.parse_until(Some("]"), false, depth + 1);
        let alt_end = self.pos;
        if !self.s[alt_end..].starts_with(']') {
            self.pos = start;
            return None;
        }
        // 分支 1：`](` inline 图片
        if self.s[alt_end..].starts_with("](") {
            let url_start = alt_end + 2;
            let rel = match self.s[url_start..].find(')') {
                Some(r) => r,
                None => {
                    self.pos = start;
                    return None;
                }
            };
            self.pos = url_start + rel + 1;
            return Some(self.node(
                InlineKind::Image,
                start,
                start + 2,
                alt_end,
                Some((url_start, url_start + rel)),
                children,
            ));
        }
        // 分支 2：`][label]` / `][]`
        if self.s[alt_end..].starts_with("][") {
            let ls = alt_end + 2;
            let Some(rel) = self.s[ls..].find(']') else {
                self.pos = start;
                return None;
            };
            let le = ls + rel;
            if self.s[ls..le].contains('[') || le - ls > 999 {
                self.pos = start;
                return None;
            }
            let label_src = if le == ls {
                &self.s[start + 2..alt_end]
            } else {
                &self.s[ls..le]
            };
            let Some(&(dest, _)) = self.refs.get(&normalize_label(label_src)) else {
                self.pos = start;
                return None;
            };
            self.pos = le + 1;
            let mut n = self.node(InlineKind::Image, start, start + 2, alt_end, None, children);
            n.url = Some(dest);
            return Some(n);
        }
        // 分支 3：shortcut 图片
        let Some(&(dest, _)) = self.refs.get(&normalize_label(&self.s[start + 2..alt_end])) else {
            self.pos = start;
            return None;
        };
        self.pos = alt_end + 1;
        let mut n = self.node(InlineKind::Image, start, start + 2, alt_end, None, children);
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
        // B5 保险丝：`![**` 交替制造 image↔strong 真嵌套（CommonMark 禁止链接嵌套，
        // 图片 alt 不受限）。0c P1 起强调不再消耗递归深度：16 层 image（MAX_INLINE_DEPTH）
        // + 其内 15 层 strong（delimiter-run 匹配）= 31；EMPH_MAX_DEPTH=32 下不触发
        let doc = format!("{}a{}", "![**".repeat(40), "**](u)".repeat(40));
        let nodes = parse(&doc);
        assert_eq!(depth(&nodes), 31);
    }

    #[test]
    fn emph_closed_by_first_star_of_double() {
        // 闭 run 长度 2 与开 run 长度 1 匹配：消费 1 个 `*` 成 Emph，余 1 个为纯文本
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
}
