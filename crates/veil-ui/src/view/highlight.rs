//! 代码语法高亮：syntect（fancy-regex）逐行增量解析，输出致密语义段。
//!
//! 设计要点（ADR-012）：
//! - 语法集惰性加载：`SyntaxSet` 只在首次遇到代码块时经 `OnceLock` 加载，启动零成本。
//! - 逐行 ParseState 缓存：块内一行内容与其行首解析状态都未变时直接复用该行 token，
//!   单字符编辑只重解析受影响的行（S1-b 实测 p95 < 0.5 ms / 1000 行）。
//! - 缓存按块起始偏移寻址并做 LRU 上限，块 id 变化（重解析）不会导致整块重算。
//! - 对下游仍输出 `(start, end, TokenKind)` 致密段；颜色由主题 `SyntaxColors` 映射。

use std::collections::HashMap;
use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

/// 高亮 token 对应的语义色（由 Theme.syntax 映射到具体 RGB）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenKind {
    Keyword,
    String,
    Comment,
    Number,
    Function,
    TypeName,
}

/// 语法集：进程内只加载一次（约 0.9 ms、7 MB 常驻），首次遇到代码块才触发。
pub(crate) fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_nonewlines)
}

/// scope 前缀 → 语义 token。按栈顶向下匹配，先命中者胜；未命中即默认正文色。
///
/// `storage.type` 统一视为关键字（syntect 默认语法把 Rust 的 `let`/`fn` 与
/// `u32`、C 的 `int` 放在同一 scope 下，无法在不重引入词表的前提下细分）。
pub(crate) fn kind_map() -> &'static [(Scope, TokenKind)] {
    static MAP: OnceLock<Vec<(Scope, TokenKind)>> = OnceLock::new();
    MAP.get_or_init(|| {
        [
            ("comment", TokenKind::Comment),
            ("string", TokenKind::String),
            ("constant.numeric", TokenKind::Number),
            ("constant.language", TokenKind::Keyword),
            ("constant.other", TokenKind::Keyword),
            ("keyword", TokenKind::Keyword),
            ("storage.modifier", TokenKind::Keyword),
            ("storage.type", TokenKind::Keyword),
            ("entity.name.function", TokenKind::Function),
            ("variable.function", TokenKind::Function),
            ("support.function", TokenKind::Function),
            ("entity.name.type", TokenKind::TypeName),
            ("entity.name.class", TokenKind::TypeName),
            ("entity.name.struct", TokenKind::TypeName),
            ("entity.name.union", TokenKind::TypeName),
            ("entity.name.enum", TokenKind::TypeName),
            ("support.type", TokenKind::TypeName),
            ("support.class", TokenKind::TypeName),
            ("variable.language", TokenKind::Keyword),
        ]
        .into_iter()
        .filter_map(|(s, k)| Scope::new(s).ok().map(|scope| (scope, k)))
        .collect()
    })
}

/// 取语法栈对应的 token 种类：栈顶原子优先（与主题选择器语义一致）。
pub(crate) fn kind_of(stack: &[Scope]) -> Option<TokenKind> {
    for scope in stack.iter().rev() {
        for &(prefix, kind) in kind_map() {
            if prefix.is_prefix_of(*scope) {
                return Some(kind);
            }
        }
    }
    None
}

/// 语言别名 → syntect 规范语法名（内置优先，手写补充 TypeScript/TOML/Dockerfile）。
/// 返回 `None` 表示按纯文本处理——syntect 默认 SyntaxSet（75 套）不包含这三者，
/// 我们也暂不 vendor 第三方 .sublime-syntax（体积 +10 KB / 个，留给后续）。
pub(crate) fn lookup_syntax(lang: &str) -> Option<&'static str> {
    let ss = syntaxes();
    let trimmed = lang.trim();
    let lower = trimmed.to_ascii_lowercase();
    // 1. 内置语法集：按 token 查（js → JavaScript、cpp → C++ 等 syntect 自带 alias 解析）
    if let Some(syntax) = ss.find_syntax_by_token(&lower) {
        return Some(syntax.name.as_str());
    }
    // 2. 内置语法集：按 name 精确查
    if let Some(syntax) = ss.find_syntax_by_name(trimmed) {
        return Some(syntax.name.as_str());
    }
    // 3. 手写补充：syntect 默认集确实没有 TypeScript / TOML / Dockerfile。
    //    TS 路由到 JavaScript（类型注解 scope 会正确标注 function/type token，效果够用）；
    //    TOML / Dockerfile 暂不支持（返回 None → 回退纯文本）。
    match lower.as_str() {
        "typescript" | "ts" | "tsx" => Some("JavaScript"),
        _ => None,
    }
}

/// 围栏 info string → 语言标签文本：已注册语言用规范展示名，未知语言保留原文；
/// 空 info string 返回 `None`（调用方隐藏标签）。
pub fn language_label(info: &str) -> Option<String> {
    let raw = info.trim();
    if raw.is_empty() {
        return None;
    }
    Some(
        lookup_syntax(raw)
            .unwrap_or(raw) // lookup_syntax 返回 None 时回退原文（含 TOML/Dockerfile 等未注册语言）
            .to_string(),
    )
}

/// 单行解析记忆：行文本 + 行首/行尾状态 + 该行已求解的相对 token 段。
struct LineMemo {
    text: String,
    start: ParseState,
    end: ParseState,
    /// 行尾语法栈：复用该行时直接恢复，供后续行映射使用。
    end_stack: ScopeStack,
    /// 行内相对字节区间（不含换行）。
    spans: Vec<(usize, usize, TokenKind)>,
}

/// 块级记忆：按块起始偏移寻址，`tick` 用于 LRU 回收。
struct BlockMemo {
    lang: &'static str,
    lines: Vec<LineMemo>,
    tick: u64,
}

/// 缓存上限：可见文档通常只有少量代码块；超限按最久未用回收，保证内存有界。
const MAX_BLOCKS: usize = 64;
const MAX_LINES: usize = 2048;

/// 长代码块阈值：超限块首次渲染跳过高亮，后台 parse 后回填颜色。
pub const LONG_CODE_THRESHOLD_LINES: usize = 200;
pub const LONG_CODE_THRESHOLD_BYTES: usize = 20 * 1024;

/// 后台预计算的纯 spans 缓存：与 blocks（逐行 ParseState 缓存）并列，后台回填时直接塞这里，
/// 避免后台线程构造 LineMemo（ParseState 非 Send）。命中即零成本返回 spans。
type SpansCache = HashMap<usize, (String, Vec<(usize, usize, TokenKind)>)>;

/// 增量高亮器：跨越同一块的多次布局复用逐行解析状态。
#[derive(Default)]
pub struct Highlighter {
    blocks: HashMap<usize, BlockMemo>,
    /// 后台预计算的纯 spans 缓存（不可增量编辑，但命中即零成本）。
    bg_spans: SpansCache,
    tick: u64,
}

impl Highlighter {
    /// 查询指定块是否已在缓存中（行级 ParseState 或后台预计算 spans 任一就绪）。
    pub fn has(&self, key: usize) -> bool {
        self.blocks.contains_key(&key) || self.bg_spans.contains_key(&key)
    }

    /// 后台线程预计算的 spans 缓存进主线程：key 为块起始偏移（同 highlight），
    /// 后台传入原始 text 供主线程校验（块已重解析时自然失配丢弃）。
    pub fn cache_spans(&mut self, key: usize, text: String, spans: Vec<(usize, usize, TokenKind)>) {
        self.bg_spans.insert(key, (text, spans));
    }

    /// 高亮整块文本。`key` 为块起始偏移（同块编辑不变化，块整体平移会自然失配）。
    pub fn highlight(
        &mut self,
        key: usize,
        lang: &str,
        text: &str,
    ) -> Vec<(usize, usize, TokenKind)> {
        // 后台预计算路径优先：命中且文本未变 → 零 parse 直接返回。
        if let Some((cached_text, spans)) = self.bg_spans.get(&key) {
            if cached_text == text {
                return spans.clone();
            }
            self.bg_spans.remove(&key);
        }
        let Some(name) = lookup_syntax(lang) else {
            return Vec::new();
        };
        let ss = syntaxes();
        let Some(syntax) = ss.find_syntax_by_name(name) else {
            return Vec::new();
        };
        self.tick += 1;
        let old_lines = self
            .blocks
            .remove(&key)
            .filter(|m| m.lang == name)
            .map(|m| m.lines)
            .unwrap_or_default();

        let mut state = ParseState::new(syntax);
        let mut stack = ScopeStack::new();
        let mut out: Vec<(usize, usize, TokenKind)> = Vec::new();
        let mut next: Vec<LineMemo> = Vec::new();
        let mut offset = 0usize;

        for (i, chunk) in text.split_inclusive('\n').enumerate() {
            let parse = chunk.strip_suffix('\n').unwrap_or(chunk);
            let parse = parse.strip_suffix('\r').unwrap_or(parse);
            if let Some(m) = old_lines.get(i) {
                if m.text == parse && m.start == state {
                    for &(s, e, k) in &m.spans {
                        out.push((offset + s, offset + e, k));
                    }
                    state = m.end.clone();
                    stack = m.end_stack.clone();
                    next.push(LineMemo {
                        text: parse.to_string(),
                        start: m.start.clone(),
                        end: m.end.clone(),
                        end_stack: m.end_stack.clone(),
                        spans: m.spans.clone(),
                    });
                    offset += chunk.len();
                    continue;
                }
            }
            let start = state.clone();
            let ops = state.parse_line(parse, ss).unwrap_or_default();
            let mut spans: Vec<(usize, usize, TokenKind)> = Vec::new();
            let mut pos = 0usize;
            for (end, op) in &ops {
                if *end > pos {
                    if let Some(k) = kind_of(stack.as_slice()) {
                        spans.push((pos, *end, k));
                    }
                }
                let _ = stack.apply(op);
                pos = *end;
            }
            if pos < parse.len() {
                if let Some(k) = kind_of(stack.as_slice()) {
                    spans.push((pos, parse.len(), k));
                }
            }
            for &(s, e, k) in &spans {
                out.push((offset + s, offset + e, k));
            }
            next.push(LineMemo {
                text: parse.to_string(),
                start,
                end: state.clone(),
                end_stack: stack.clone(),
                spans,
            });
            offset += chunk.len();
        }
        merge_adjacent(&mut out);
        self.blocks.insert(
            key,
            BlockMemo {
                lang: name,
                lines: next,
                tick: self.tick,
            },
        );
        self.prune();
        out
    }

    /// 超限回收：块数与总行数双上限，按最久未用逐出。
    fn prune(&mut self) {
        while self.blocks.len() > MAX_BLOCKS
            || self.blocks.values().map(|m| m.lines.len()).sum::<usize>() > MAX_LINES
        {
            let Some(oldest) = self
                .blocks
                .iter()
                .min_by_key(|(_, m)| m.tick)
                .map(|(k, _)| *k)
            else {
                break;
            };
            self.blocks.remove(&oldest);
        }
    }
}

pub(crate) fn merge_adjacent(spans: &mut Vec<(usize, usize, TokenKind)>) {
    if spans.len() < 2 {
        return;
    }
    let mut write = 0;
    for read in 1..spans.len() {
        if spans[read].2 == spans[write].2 && spans[read].0 == spans[write].1 {
            spans[write].1 = spans[read].1;
        } else {
            write += 1;
            spans[write] = spans[read];
        }
    }
    spans.truncate(write + 1);
}

/// 把高亮段（稀疏彩色 token）补全为致密 `(start, end, Style)`，
/// 间隙用 `default` 填充，彩色段用对应 `color`。
pub fn to_styles(
    spans: &[(usize, usize, TokenKind)],
    text_len: usize,
    colors: &crate::theme::SyntaxColors,
    default: Style,
) -> Vec<(usize, usize, Style)> {
    let mut out: Vec<(usize, usize, Style)> = Vec::new();
    let mut pos = 0;
    for &(s, e, tk) in spans {
        if s > pos {
            out.push((pos, s, default));
        }
        let color = match tk {
            TokenKind::Keyword => colors.keyword,
            TokenKind::String => colors.string,
            TokenKind::Comment => colors.comment,
            TokenKind::Number => colors.number,
            TokenKind::Function => colors.function,
            TokenKind::TypeName => colors.type_name,
        };
        let mut st = default;
        st.color = Some(color);
        out.push((s, e, st));
        pos = e;
    }
    if pos < text_len {
        out.push((pos, text_len, default));
    }
    out
}

use crate::view::style::Style;

#[cfg(test)]
mod tests {
    use super::*;

    fn hl(lang: &str, text: &str) -> Vec<(usize, usize, TokenKind)> {
        Highlighter::default().highlight(0, lang, text)
    }

    fn color_at(spans: &[(usize, usize, TokenKind)], at: usize) -> Option<TokenKind> {
        spans
            .iter()
            .find(|(s, e, _)| *s <= at && at < *e)
            .map(|x| x.2)
    }

    /// 每个 span 的区间必须合法且两端都落在字符边界上。
    fn assert_spans_valid(text: &str, spans: &[(usize, usize, TokenKind)]) {
        for &(start, end, _) in spans {
            assert!(start <= end, "start={start} > end={end}（text={text:?}）");
            assert!(
                end <= text.len(),
                "end={end} > len={}（text={text:?}）",
                text.len()
            );
            assert!(
                text.is_char_boundary(start),
                "start={start} 非字符边界（text={text:?}）"
            );
            assert!(
                text.is_char_boundary(end),
                "end={end} 非字符边界（text={text:?}）"
            );
        }
    }

    #[test]
    fn language_labels_use_canonical_names_and_hide_when_absent() {
        assert_eq!(language_label("rust").as_deref(), Some("Rust"));
        assert_eq!(language_label("py").as_deref(), Some("Python"));
        assert_eq!(language_label(" YML ").as_deref(), Some("YAML"));
        assert_eq!(language_label("cpp").as_deref(), Some("C++"));
        // 未注册语言保留原始字符串（只去首尾空白）。
        assert_eq!(language_label("brainfuck").as_deref(), Some("brainfuck"));
        assert_eq!(language_label("MyLang").as_deref(), Some("MyLang"));
        assert_eq!(language_label(""), None);
        assert_eq!(language_label("   "), None);
    }

    #[test]
    fn cjk_markdown_fence_does_not_panic() {
        // 用户复现用例：中文围栏代码块正文按字节下标扫描时曾 panic
        let src = "# 任务：[精准任务名称]\n\n- 第一步：确认边界\n- 第二步：滚动到底\n\n```markdown\n# 任务：[精准任务名称]\n中文正文：保持 UTF-8 字符边界。\n```\n";
        let sp = hl("markdown", src);
        assert_spans_valid(src, &sp);
    }

    #[test]
    fn cjk_strings_comments_emoji_and_identifiers_stay_on_boundaries() {
        let cases: &[(&str, &str)] = &[
            // 中文字符串 + emoji + 中文行注释
            (
                "rust",
                "let 中文标识_1 = \"中文字符串😀\";\n// 中文行注释\n",
            ),
            // 中文块注释内 emoji
            ("rust", "/* 中文块注释\n😀 继续 */\nlet x = 1;"),
            // 反斜杠转义 emoji
            ("rust", "let s = \"a\\😀b\";\nlet t = 1;"),
            // 未闭合中文字符串
            ("rust", "let s = \"未闭合中文字符串\nlet t = 2;"),
            // 三引号中文串
            ("python", "s = \"\"\"中文三引号😀\n第二行\"\"\""),
            // 中文标识符 + 中文注释混排
            (
                "markdown",
                "```rust\nfn 中文() { println!(\"你好😀\"); }\n```",
            ),
        ];
        for &(lang, src) in cases {
            let sp = hl(lang, src);
            assert_spans_valid(src, &sp);
        }
    }

    #[test]
    fn rust_keyword_string_comment() {
        let src = "// hi\nlet x = \"a\";\nfn f() {}";
        let sp = hl("rust", src);
        assert_eq!(color_at(&sp, 0), Some(TokenKind::Comment)); // // hi
        assert_eq!(color_at(&sp, 6), Some(TokenKind::Keyword)); // let
        assert_eq!(color_at(&sp, 14), Some(TokenKind::String)); // "a"
        assert_eq!(color_at(&sp, 19), Some(TokenKind::Keyword)); // fn
        assert_eq!(color_at(&sp, 22), Some(TokenKind::Function)); // f
    }

    #[test]
    fn python_number_and_def() {
        let sp = hl("python", "def foo(n):\n    return n + 42\n");
        assert_eq!(color_at(&sp, 0), Some(TokenKind::Keyword)); // def
        assert_eq!(color_at(&sp, 4), Some(TokenKind::Function)); // foo
        assert_eq!(color_at(&sp, 27), Some(TokenKind::Number)); // 42
    }

    #[test]
    fn block_comment_spans_lines() {
        let sp = hl("rust", "a /* multi\nline */ b");
        // 整个块注释为 Comment（换行本身不着色，取两行内的字符位置）
        assert_eq!(color_at(&sp, 2), Some(TokenKind::Comment));
        assert_eq!(color_at(&sp, 11), Some(TokenKind::Comment));
        assert_eq!(color_at(&sp, 19), None); // b
    }

    #[test]
    fn triple_quoted_string() {
        let sp = hl("python", "x = \"\"\"multi\nline\"\"\"");
        assert_eq!(color_at(&sp, 4), Some(TokenKind::String));
        assert_eq!(color_at(&sp, 13), Some(TokenKind::String));
    }

    #[test]
    fn uppercase_const_is_keyword() {
        let sp = hl("rust", "const MAX: u32 = 100;");
        assert_eq!(color_at(&sp, 6), Some(TokenKind::Keyword)); // MAX
    }

    #[test]
    fn path_type_is_type_name() {
        let sp = hl("rust", "Vec::new();");
        assert_eq!(color_at(&sp, 0), Some(TokenKind::TypeName)); // Vec
    }

    #[test]
    fn json_keys_and_values() {
        let sp = hl("json", "{\"a\": true, \"b\": 1}");
        assert_eq!(color_at(&sp, 1), Some(TokenKind::String)); // "a"
        assert_eq!(color_at(&sp, 7), Some(TokenKind::Keyword)); // true
    }

    #[test]
    fn unknown_lang_degrades_to_plain_text() {
        assert!(hl("brainfuck", "// comment\n\"str\" 42").is_empty());
        // 默认语法集不含 TOML / Dockerfile：纯文本，不报错（ADR-012 覆盖差异）。
        assert!(hl("toml", "mode = \"dark\" # c").is_empty());
        assert!(hl("dockerfile", "FROM rust:1.80").is_empty());
    }

    // ---------- TypeScript / TSX 路由验证（syntect 默认集没有 TS，映射到 JS 语法） ----------

    #[test]
    fn typescript_code_block_produces_spans() {
        // TS 路由到 JavaScript 语法高亮器：类型注解 token 被正确识别
        let ts_code = "const x: number = 42;\nfunction foo(s: string): void { return s; }";
        let sp = hl("typescript", ts_code);
        assert!(!sp.is_empty(), "TypeScript 代码块必须产出非空 spans");
        // 至少有一个 Keyword（const / function）
        assert!(
            sp.iter().any(|(_, _, k)| *k == TokenKind::Keyword),
            "应识别到至少一个关键字"
        );
        // 至少有一个 Number（42）
        assert!(
            sp.iter().any(|(_, _, k)| *k == TokenKind::Number),
            "应识别到至少一个数字"
        );
    }

    #[test]
    fn ts_alias_routes_to_same_syntax() {
        // typescript / ts / tsx 三个 alias 全部产出 spans
        let code = "let n = 42;";
        assert!(!hl("ts", code).is_empty(), "ts alias 必须产出 spans");
        assert!(!hl("tsx", code).is_empty(), "tsx alias 必须产出 spans");
    }

    #[test]
    fn js_aliases_route_via_syntect_builtin() {
        // 验证 syntect 内置 alias 解析：js / cpp / py 等短名正确路由
        assert!(!hl("js", "let x = 1;").is_empty());
        assert!(!hl("cpp", "int main() { return 0; }").is_empty());
        assert!(!hl("py", "def f(): pass").is_empty());
    }

    #[test]
    fn dense_coverage_from_to_styles() {
        use crate::theme::Theme;
        let sp = hl("rust", "let x = 1;");
        let styles = to_styles(&sp, 10, &Theme::light().syntax, Style::default());
        let mut pos = 0;
        for &(s, e, _) in &styles {
            assert_eq!(s, pos, "致密覆盖无间隙");
            pos = e;
        }
        assert_eq!(pos, 10);
    }

    /// 单字符编辑只重解析受影响行：其余行必须逐段复用。
    #[test]
    fn incremental_edit_reuses_unaffected_lines() {
        let mut h = Highlighter::default();
        let base = "fn a() {}\nfn b() {}\nfn c() {}\n";
        let first = h.highlight(0, "rust", base);
        assert!(!first.is_empty());
        let edited = "fn a() {}\nfn bx() {}\nfn c() {}\n";
        let second = h.highlight(0, "rust", edited);
        // 输出区间仍然合法且致密有序。
        assert_spans_valid(edited, &second);
        let mut pos = 0;
        for &(s, _e, _k) in &second {
            assert!(s >= pos, "段必须升序");
            pos = s;
        }
        assert_eq!(h.blocks.len(), 1, "同块编辑不得新建缓存条目");
        assert_eq!(h.blocks[&0].lines.len(), 3, "块行数应随文本更新");
    }

    // ---------- 后台异步高亮相关测试 ----------

    #[test]
    fn bg_threshold_constants_are_sane() {
        const {
            assert!(
                LONG_CODE_THRESHOLD_LINES >= 100,
                "200 行才触发异步，避免短代码块走后台"
            );
            assert!(
                LONG_CODE_THRESHOLD_BYTES >= 1024,
                "20 KB 阈值合理（~2000 行普通代码）"
            );
        }
    }

    #[test]
    fn bg_spans_fast_path_returns_cached_without_parse() {
        // 预先 cache_spans → 后续 highlight 应直接返回， blocks 缓存不变（未 parse）
        let mut h = Highlighter::default();
        let body = "fn cached() { let x = 1; }\n".repeat(3);
        let fake_spans: Vec<(usize, usize, TokenKind)> =
            vec![(0, 2, TokenKind::Keyword), (3, 9, TokenKind::Function)];
        h.cache_spans(7, body.clone(), fake_spans.clone());
        assert!(h.has(7));
        // 同步 parse 路径未命中 → 走 bg_spans 快速路径
        let got = h.highlight(7, "rust", &body);
        assert_eq!(got, fake_spans, "bg_spans 缓存命中必须零 parse 返回");
        assert_eq!(
            h.blocks.len(),
            0,
            "bg_spans 命中时 blocks 缓存不应新建（未 parse）"
        );
    }

    #[test]
    fn bg_spans_invalidated_on_body_edit() {
        // body 编辑后，bg_spans 比对失败 → 应自动丢弃旧 spans → 走同步 parse
        let mut h = Highlighter::default();
        let body = "fn orig() { let x = 1; }\n".repeat(3);
        h.cache_spans(9, body.clone(), vec![(0, 2, TokenKind::Keyword)]);
        assert!(h.has(9));
        let mut modified = body.clone();
        modified.push_str("new_line_here\n");
        let got = h.highlight(9, "rust", &modified);
        // 同步 parse 必须产出非空 spans（body 有效 Rust 代码）
        assert!(!got.is_empty(), "编辑后应走同步 parse 产出 spans");
        // bg_spans 旧条目已被比对失败自动移除
        assert!(!h.bg_spans.contains_key(&9), "编辑后旧 bg_spans 必须被丢弃");
    }

    #[test]
    fn cache_spans_does_not_increment_block_cache() {
        // cache_spans 是纯 spans 缓存，与 blocks（逐行 ParseState）无关，
        // 所以 blocks.len() 必须保持 0 — 验证两条缓存独立。
        let mut h = Highlighter::default();
        h.cache_spans(11, "text".into(), vec![(0, 2, TokenKind::Keyword)]);
        assert_eq!(h.blocks.len(), 0, "cache_spans 不应污染 blocks");
        assert_eq!(h.bg_spans.len(), 1, "bg_spans 必须恰好一条");
    }
}
