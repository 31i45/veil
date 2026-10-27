//! inline 解析（v0 子集）。
//
// 语义取舍（逐条钉在测试上；CommonMark flanking 全量规则属后续增量）：
// - 递归下降、最左匹配：开标记找最近闭标记成节点，嵌套经递归实现
//   （`[link **bold**](url)` 合法嵌套；`**a **b** c**` 解析为两个并列 strong，
//   与 CommonMark 的嵌套结果不同——已知分歧，钉在 adjacent_strongs_leftmost_match）。
// - 空内容不成节点（`****` 为纯文本）；未闭合开标记为纯文本。
// - Code/Math 内容原样不嵌套；Math 要求内容非空、首尾非空格、闭 `$` 后非数字
//   （pandoc 规则，避免 `$5 and $6` 误判为公式）。
// - Link 文本可嵌套除 Link 外的任意 inline；URL 扫至首个 `)`，不支持 title。
//   引用式（0c P0-1）：`[text][label]` / `[text][]` / `[text]` shortcut 查文档引用表，
//   节点的 url 指向定义处 dest 的绝对区间；查无定义则整体按纯文本（内部 inline 正常解析）。
//   优先级：inline 链接 > full/collapsed 引用 > shortcut（与 CommonMark 一致）。
// - Image 语法同 Link，alt 允许为空；URL 可空（`![]()` 合法，视图侧无加载键则只显
//   alt）。Image 可出现在 Link 文本内（徽章写法），alt 内亦可嵌套 inline（切片 13）；
//   引用式图片 `![alt][label]` / `![alt][]` / `![alt]` 同引用式链接（0c P0-1）。
// - `\` 转义 v0 标记字符；递归深度硬上限 MAX_INLINE_DEPTH（B5：超深嵌套不得栈溢出）。

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

/// v0 可转义字符：inline 标记字符 + 反斜杠自身（`!` 为 Image 开标记首字符）。
fn is_escapable(c: char) -> bool {
    matches!(
        c,
        '\\' | '*' | '$' | '[' | ']' | '(' | ')' | '`' | '!' | '~'
    )
}

struct InlineParser<'a> {
    s: &'a str,
    base: usize,
    refs: &'a RefMap,
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
                    .or_else(|| self.try_strikethrough(depth))
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
}
