//! 富文本粘贴（切片 11）：剪贴板 HTML → Markdown 子集。
//!
//! 纯函数、零 GUI 依赖。覆盖 Chrome/Word 纯文本复制出的常见结构：
//! 段落/标题/粗斜体/行内代码/链接/换行/分隔线/代码块/列表（嵌套）/引用块/常见实体。
//! 未知标签递归取子节点；未知实体原样保留（不猜测语义）。
//! 非目标：图片落盘、表格、样式属性（class/style 一律忽略）。

/// HTML → Markdown。空/纯空白输入返回空串；输出非空时保证以单个换行结尾。
pub fn html_to_markdown(html: &str) -> String {
    let mut out = render_blocks(&parse(&tokenize(html)));
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

// ---- 词法 ----

enum Tok {
    Text(String),
    Start {
        name: String,
        attrs: Vec<(String, String)>,
        self_close: bool,
    },
    End(String),
}

/// 空元素：不产生子树（`<br>`、`<hr>` 等在渲染期单独处理）。
fn is_void(name: &str) -> bool {
    matches!(
        name,
        "br" | "hr"
            | "img"
            | "input"
            | "meta"
            | "link"
            | "area"
            | "base"
            | "col"
            | "embed"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn tokenize(src: &str) -> Vec<Tok> {
    let b = src.as_bytes();
    let mut toks = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let start = i;
            while i < b.len() && b[i] != b'<' {
                i += 1;
            }
            text.push_str(&src[start..i]);
            continue;
        }
        if src[i..].starts_with("<!--") {
            i = src[i + 4..].find("-->").map_or(b.len(), |p| i + 4 + p + 3);
            continue;
        }
        if src[i..].starts_with("<!") || src[i..].starts_with("<?") {
            i = src[i..].find('>').map_or(b.len(), |p| i + p + 1);
            continue;
        }
        let Some((tok, next)) = parse_tag(src, i) else {
            // 「<」后不是标签名（如 `a < b`）：按字面文本处理
            text.push('<');
            i += 1;
            continue;
        };
        flush_text(&mut text, &mut toks);
        i = next;
        match tok {
            // 原始文本元素：内容对 Markdown 无价值，整体丢弃
            Tok::Start { ref name, .. } if name == "script" || name == "style" => {
                let close = format!("</{name}");
                let lower = src.to_ascii_lowercase();
                if let Some(p) = lower[i..].find(&close) {
                    let after = i + p + close.len();
                    i = src[after..].find('>').map_or(b.len(), |q| after + q + 1);
                } else {
                    i = b.len();
                }
            }
            t => toks.push(t),
        }
    }
    flush_text(&mut text, &mut toks);
    toks
}

fn flush_text(buf: &mut String, toks: &mut Vec<Tok>) {
    if buf.is_empty() {
        return;
    }
    let decoded = decode_entities(buf);
    // HTML 规范：\r\n 与 \r 均归一为 \n（pre 内亦如此，否则 Windows 剪贴板会带 \r）
    let norm = if decoded.contains('\r') {
        decoded.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        decoded
    };
    toks.push(Tok::Text(norm));
    buf.clear();
}

/// 解析 `<tag ...>` / `</tag>`。非标签返回 None（`<` 归文本）。
fn parse_tag(src: &str, i: usize) -> Option<(Tok, usize)> {
    let b = src.as_bytes();
    let mut j = i + 1;
    let closing = j < b.len() && b[j] == b'/';
    if closing {
        j += 1;
    }
    let name_start = j;
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b':') {
        j += 1;
    }
    if j == name_start {
        return None;
    }
    let name = src[name_start..j].to_ascii_lowercase();
    if closing {
        while j < b.len() && b[j] != b'>' {
            j += 1;
        }
        return (j < b.len()).then(|| (Tok::End(name), j + 1));
    }
    let mut attrs = Vec::new();
    let mut self_close = false;
    loop {
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= b.len() {
            return None; // 未闭合标签：整段按文本（容错优先）
        }
        match b[j] {
            b'>' => {
                j += 1;
                break;
            }
            b'/' => {
                self_close = true;
                j += 1;
                continue;
            }
            _ => {}
        }
        let attr_start = j;
        while j < b.len()
            && !b[j].is_ascii_whitespace()
            && b[j] != b'='
            && b[j] != b'>'
            && b[j] != b'/'
        {
            j += 1;
        }
        if j == attr_start {
            j += 1;
            continue;
        }
        let attr_name = src[attr_start..j].to_ascii_lowercase();
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut value = String::new();
        if j < b.len() && b[j] == b'=' {
            j += 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                let quote = b[j];
                j += 1;
                let v = j;
                while j < b.len() && b[j] != quote {
                    j += 1;
                }
                value = src[v..j].to_string();
                if j < b.len() {
                    j += 1;
                }
            } else {
                let v = j;
                while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                    j += 1;
                }
                value = src[v..j].to_string();
            }
        }
        attrs.push((attr_name, decode_entities(&value)));
    }
    Some((
        Tok::Start {
            name,
            attrs,
            self_close,
        },
        j,
    ))
}

// ---- 语法树 ----

enum Node {
    Text(String),
    Elem {
        name: String,
        attrs: Vec<(String, String)>,
        children: Vec<Node>,
    },
}

struct Frame {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

fn parse(toks: &[Tok]) -> Vec<Node> {
    let mut stack = vec![Frame {
        name: String::new(),
        attrs: Vec::new(),
        children: Vec::new(),
    }];
    for t in toks {
        match t {
            Tok::Text(s) => {
                if !s.is_empty() {
                    stack
                        .last_mut()
                        .expect("根帧恒在")
                        .children
                        .push(Node::Text(s.clone()));
                }
            }
            Tok::Start {
                name,
                attrs,
                self_close,
            } => {
                if *self_close || is_void(name) {
                    stack
                        .last_mut()
                        .expect("根帧恒在")
                        .children
                        .push(Node::Elem {
                            name: name.clone(),
                            attrs: attrs.clone(),
                            children: Vec::new(),
                        });
                } else {
                    stack.push(Frame {
                        name: name.clone(),
                        attrs: attrs.clone(),
                        children: Vec::new(),
                    });
                }
            }
            Tok::End(name) => {
                // 容错：只闭合最近的同名帧，缺闭合的中间帧一并隐式闭合
                let Some(pos) = stack.iter().rposition(|f| &f.name == name) else {
                    continue;
                };
                if pos == 0 {
                    continue;
                }
                while stack.len() > pos {
                    let f = stack.pop().expect("非空栈");
                    stack
                        .last_mut()
                        .expect("根帧恒在")
                        .children
                        .push(Node::Elem {
                            name: f.name,
                            attrs: f.attrs,
                            children: f.children,
                        });
                }
            }
        }
    }
    while stack.len() > 1 {
        let f = stack.pop().expect("非空栈");
        stack
            .last_mut()
            .expect("根帧恒在")
            .children
            .push(Node::Elem {
                name: f.name,
                attrs: f.attrs,
                children: f.children,
            });
    }
    stack.pop().expect("根帧恒在").children
}

// ---- 渲染 ----

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "pre"
            | "hr"
            | "ul"
            | "ol"
            | "blockquote"
            | "li"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "aside"
            | "nav"
            | "figure"
    )
}

fn is_block_node(n: &Node) -> bool {
    matches!(n, Node::Elem { name, .. } if is_block(name))
}

/// 块序列 → Markdown（块间恰一空行，无末尾换行——由 `html_to_markdown` 补）。
fn render_blocks(nodes: &[Node]) -> String {
    let mut out = String::new();
    let mut para = String::new();
    for n in nodes {
        match n {
            Node::Text(t) => collapse_into(&mut para, t),
            Node::Elem { name, children, .. } => {
                if is_block(name) || children.iter().any(is_block_node) {
                    flush_para(&mut out, &mut para);
                    if let Some(b) = render_block(n) {
                        push_block(&mut out, &b);
                    }
                } else {
                    render_inline(n, &mut para);
                }
            }
        }
    }
    flush_para(&mut out, &mut para);
    out
}

fn flush_para(out: &mut String, para: &mut String) {
    let t = para.trim();
    if !t.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(t);
    }
    para.clear();
}

fn push_block(out: &mut String, block: &str) {
    let b = block.trim_matches('\n');
    if b.trim().is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(b);
}

fn render_block(node: &Node) -> Option<String> {
    let Node::Elem { name, children, .. } = node else {
        return None;
    };
    match name.as_str() {
        "hr" => Some("---".into()),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = (name.as_bytes()[1] - b'0') as usize;
            let t = inline_trim(children.iter());
            (!t.is_empty()).then(|| format!("{} {t}", "#".repeat(level)))
        }
        "pre" => render_pre(children),
        "blockquote" => {
            let inner = render_blocks(children);
            if inner.is_empty() {
                return None;
            }
            let quoted: Vec<String> = inner
                .lines()
                .map(|l| {
                    if l.is_empty() {
                        ">".to_string()
                    } else {
                        format!("> {l}")
                    }
                })
                .collect();
            Some(quoted.join("\n"))
        }
        "ul" | "ol" => {
            let s = render_list(node, 0);
            (!s.trim().is_empty()).then_some(s)
        }
        _ => {
            // p/div/li 及未知块：含块级子节点（如 div>p）走块渲染，否则按段落
            if children.iter().any(is_block_node) {
                let s = render_blocks(children);
                (!s.is_empty()).then_some(s)
            } else {
                let t = inline_trim(children.iter());
                (!t.is_empty()).then_some(t)
            }
        }
    }
}

fn render_pre(children: &[Node]) -> Option<String> {
    let mut raw = String::new();
    for c in children {
        raw_text(c, &mut raw);
    }
    let t = raw.trim_matches('\n');
    (!t.trim().is_empty()).then(|| format!("```\n{t}\n```"))
}

/// 原始文本（保留空白）：pre 内不做折叠，`<br>` 换算为换行。
fn raw_text(node: &Node, out: &mut String) {
    match node {
        Node::Text(t) => out.push_str(t),
        Node::Elem { name, children, .. } => {
            if name == "br" {
                out.push('\n');
            } else {
                for c in children {
                    raw_text(c, out);
                }
            }
        }
    }
}

fn render_list(node: &Node, depth: usize) -> String {
    let Node::Elem { name, children, .. } = node else {
        return String::new();
    };
    let ordered = name == "ol";
    let mut out = String::new();
    let mut n = 1;
    for c in children {
        let Node::Elem {
            name: child_name,
            children: grandchildren,
            ..
        } = c
        else {
            continue;
        };
        if child_name != "li" {
            continue;
        }
        let mut nested = Vec::new();
        let mut inline_nodes = Vec::new();
        for x in grandchildren {
            if matches!(x, Node::Elem { name, .. } if name == "ul" || name == "ol") {
                nested.push(x);
            } else {
                inline_nodes.push(x);
            }
        }
        let text = inline_trim(inline_nodes.iter().copied());
        let indent = "  ".repeat(depth);
        let marker = if ordered {
            format!("{n}. ")
        } else {
            "- ".to_string()
        };
        let pad = " ".repeat(depth * 2 + marker.chars().count());
        if !text.is_empty() {
            out.push_str(&indent);
            out.push_str(&marker);
            out.push_str(&text.replace('\n', &format!("\n{pad}")));
            out.push('\n');
        } else if nested.is_empty() {
            // 空列表项仍占一行，保证编号连续可读
            out.push_str(&indent);
            out.push_str(marker.trim_end());
            out.push('\n');
        }
        for nn in nested {
            out.push_str(&render_list(nn, depth + 1));
        }
        n += 1;
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

fn render_inline(node: &Node, out: &mut String) {
    match node {
        Node::Text(t) => collapse_into(out, t),
        Node::Elem {
            name,
            attrs,
            children,
        } => match name.as_str() {
            "br" => out.push_str("  \n"),
            "strong" | "b" => wrap(out, "**", children),
            "em" | "i" => wrap(out, "*", children),
            "code" => render_code(children, out),
            "a" => {
                let mut inner = String::new();
                for c in children {
                    render_inline(c, &mut inner);
                }
                let text = inner.trim();
                let href = attr(attrs, "href").map(str::trim).unwrap_or("");
                if href.is_empty() {
                    out.push_str(text);
                } else if text.is_empty() {
                    out.push_str(href);
                } else {
                    out.push('[');
                    out.push_str(text);
                    out.push_str("](");
                    out.push_str(href);
                    out.push(')');
                }
            }
            _ => {
                for c in children {
                    render_inline(c, out);
                }
            }
        },
    }
}

fn render_code(children: &[Node], out: &mut String) {
    let mut raw = String::new();
    for c in children {
        raw_text(c, &mut raw);
    }
    let mut collapsed = String::new();
    collapse_into(&mut collapsed, &raw);
    let t = collapsed.trim();
    if t.is_empty() {
        return;
    }
    // 内容含反引号时用更长的围栏，避免提前闭合
    let fence = "`".repeat(longest_backtick_run(t) + 1);
    let pad = if t.starts_with('`') || t.ends_with('`') {
        " "
    } else {
        ""
    };
    out.push_str(&fence);
    out.push_str(pad);
    out.push_str(t);
    out.push_str(pad);
    out.push_str(&fence);
}

fn wrap(out: &mut String, marker: &str, children: &[Node]) {
    let mut inner = String::new();
    for c in children {
        render_inline(c, &mut inner);
    }
    let t = inner.trim();
    if t.is_empty() {
        return;
    }
    out.push_str(marker);
    out.push_str(t);
    out.push_str(marker);
}

fn inline_trim<'a>(children: impl IntoIterator<Item = &'a Node>) -> String {
    let mut s = String::new();
    for c in children {
        render_inline(c, &mut s);
    }
    s.trim().to_string()
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn longest_backtick_run(s: &str) -> usize {
    let mut max = 0;
    let mut cur = 0;
    for ch in s.chars() {
        if ch == '`' {
            cur += 1;
            max = max.max(cur);
        } else {
            cur = 0;
        }
    }
    max
}

// ---- 空白与实体 ----

/// HTML 空白折叠：任意空白（含 &nbsp; 解码出的 U+00A0）折叠为单空格。
fn collapse_into(out: &mut String, text: &str) {
    let mut prev_space = out.ends_with(' ') || out.ends_with('\n');
    for ch in text.chars() {
        if ch.is_whitespace() || ch == '\u{a0}' {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
}

fn entity_char(ent: &str) -> Option<char> {
    match ent {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = ent.strip_prefix('#') {
                dec.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                None
            }
        }
    }
}

/// 解码常见实体与数字实体；未知实体原样保留（不猜测）。
fn decode_entities(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'&' {
            let start = i;
            while i < b.len() && b[i] != b'&' {
                i += 1;
            }
            out.push_str(&s[start..i]);
            continue;
        }
        // 实体名过长视为非实体（避免为无分号的「&」扫描整段）
        let limit = (i + 1 + 32).min(b.len());
        let semi = (i + 1..limit).find(|&j| b[j] == b';');
        match semi.and_then(|j| entity_char(&s[i + 1..j]).map(|c| (c, j))) {
            Some((c, j)) => {
                out.push(c);
                i = j + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::html_to_markdown;

    #[test]
    fn paragraph_collapses_whitespace() {
        assert_eq!(html_to_markdown("<p>Hello   world</p>"), "Hello world\n");
        assert_eq!(html_to_markdown("<p>\n  a\n  b\t</p>"), "a b\n");
    }

    #[test]
    fn headings() {
        assert_eq!(
            html_to_markdown("<h1>标题</h1><h2>小节</h2>"),
            "# 标题\n\n## 小节\n"
        );
        assert_eq!(
            html_to_markdown("<h3>三</h3><h4>四</h4><h5>五</h5><h6>六</h6>"),
            "### 三\n\n#### 四\n\n##### 五\n\n###### 六\n"
        );
    }

    #[test]
    fn strong_and_emphasis() {
        assert_eq!(
            html_to_markdown("<p>a <b>粗</b> <i>斜</i></p>"),
            "a **粗** *斜*\n"
        );
        assert_eq!(html_to_markdown("<p><strong>x</strong></p>"), "**x**\n");
        assert_eq!(html_to_markdown("<p><em>y</em></p>"), "*y*\n");
    }

    #[test]
    fn inline_code() {
        assert_eq!(
            html_to_markdown("<p>run <code>cargo test</code> now</p>"),
            "run `cargo test` now\n"
        );
        // 内容含反引号：围栏加长
        assert_eq!(html_to_markdown("<p><code>a`b</code></p>"), "``a`b``\n");
    }

    #[test]
    fn links() {
        assert_eq!(
            html_to_markdown("<p><a href=\"https://example.com/x\">示例</a></p>"),
            "[示例](https://example.com/x)\n"
        );
        assert_eq!(
            html_to_markdown("<p><a href=\"https://a.b\"></a></p>"),
            "https://a.b\n"
        );
        assert_eq!(html_to_markdown("<p><a>裸文本</a></p>"), "裸文本\n");
    }

    #[test]
    fn br_becomes_hard_break() {
        assert_eq!(html_to_markdown("<p>a<br>b</p>"), "a  \nb\n");
    }

    #[test]
    fn pre_fenced_block_preserves_whitespace() {
        assert_eq!(
            html_to_markdown("<pre>fn main() {\n    x\n}</pre>"),
            "```\nfn main() {\n    x\n}\n```\n"
        );
        // 首尾换行剥除（Chrome 常在 <pre> 内包一层换行）
        assert_eq!(
            html_to_markdown("<pre>\nlet x = 1;\n</pre>"),
            "```\nlet x = 1;\n```\n"
        );
        // 剪贴板 HTML 的 CRLF 归一
        assert_eq!(html_to_markdown("<pre>a\r\nb</pre>"), "```\na\nb\n```\n");
    }

    #[test]
    fn horizontal_rule() {
        assert_eq!(html_to_markdown("<p>a</p><hr><p>b</p>"), "a\n\n---\n\nb\n");
    }

    #[test]
    fn unordered_list() {
        assert_eq!(
            html_to_markdown("<ul><li>一</li><li>二</li></ul>"),
            "- 一\n- 二\n"
        );
    }

    #[test]
    fn ordered_list_numbers_increment() {
        assert_eq!(
            html_to_markdown("<ol><li>a</li><li>b</li><li>c</li></ol>"),
            "1. a\n2. b\n3. c\n"
        );
    }

    #[test]
    fn nested_list_indents_two_spaces() {
        assert_eq!(
            html_to_markdown("<ul><li>a<ul><li>b</li><li>c</li></ul></li></ul>"),
            "- a\n  - b\n  - c\n"
        );
        assert_eq!(
            html_to_markdown("<ol><li>x<ol><li>y</li></ol></li></ol>"),
            "1. x\n  1. y\n"
        );
    }

    #[test]
    fn blockquote_prefixes_each_line() {
        assert_eq!(
            html_to_markdown("<blockquote><p>a</p><p>b</p></blockquote>"),
            "> a\n>\n> b\n"
        );
    }

    #[test]
    fn entities_decoded() {
        assert_eq!(
            html_to_markdown("<p>&amp; &lt; &#65; &#x42; &nbsp; &quot;</p>"),
            "& < A B \"\n"
        );
        // 未知实体原样保留
        assert_eq!(html_to_markdown("<p>&bogus; &amp;</p>"), "&bogus; &\n");
    }

    #[test]
    fn unknown_tags_recurse_to_children() {
        assert_eq!(html_to_markdown("<p><span class=\"x\">a</span></p>"), "a\n");
        assert_eq!(html_to_markdown("<div><p>a</p><p>b</p></div>"), "a\n\nb\n");
    }

    #[test]
    fn script_and_style_are_dropped() {
        assert_eq!(
            html_to_markdown("<style>p{color:red}</style><p>a</p><script>x<1</script>"),
            "a\n"
        );
    }

    #[test]
    fn empty_inputs_yield_empty_output() {
        assert_eq!(html_to_markdown(""), "");
        assert_eq!(html_to_markdown("   \n\t"), "");
        assert_eq!(html_to_markdown("<p></p><p> </p>"), "");
        assert_eq!(html_to_markdown("<ul></ul>"), "");
    }

    #[test]
    fn unclosed_tags_tolerated() {
        // 缺 </b>：闭合 p 时隐式收拢
        assert_eq!(html_to_markdown("<p>a<b>b</p>"), "a**b**\n");
        assert_eq!(html_to_markdown("<p>a"), "a\n");
    }
}
