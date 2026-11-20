//! 一致性 harness 共用口径（CommonMark 652 例与 GFM 扩展语料共用）。
//!
//! - Suite A：Veil 自有块解析器的顶层「Markdown 生成块」骨架 vs 期望 HTML 的同类骨架。
//!   原始 HTML 块两侧都不计（源码原样输出，输出侧无法可靠区分）；GFM 语料额外把
//!   生成 `<table>` 识别为表格块（CommonMark 语料里 `<table>` 只可能是原始 HTML）。
//! - Suite B：产品导出路径复刻（块分段 + pulldown-cmark）的 HTML vs 规范 HTML。
//! - Suite C：Veil 自有行内节点计数 vs pulldown-cmark 源码行内事件计数。
//! - Suite D：行内前序森林签名 (kind, depth) 逐序比对。
#![allow(dead_code)]

use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};
use veil_core::{BlockKind, InlineKind, InlineNode, Range, Session};

/// pulldown-cmark 解析器产出的顶层块序列（与 Veil `actual_blocks` 同口径的参照）。
///
/// 关键点：pulldown-cmark 的 Tag::Paragraph 既可以是顶层块，也可以是 Quote/List 内部的
/// 行内容器。Veil 的 actual_blocks 只产顶层块，所以这里必须**跟踪容器深度**——只有
/// 顶层（depth==0）才 push Paragraph/Rule/Heading/CodeBlock。
/// 容器 = BlockQuote / List / Table（List 内部的 ListItem 不增加顶层深度；pulldown
/// 把 List 作为整体容器）。
pub fn pulldown_blocks(md: &str) -> Vec<B> {
    use pulldown_cmark::TagEnd;
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    let mut out = Vec::new();
    let mut depth = 0i32;
    for ev in Parser::new_ext(md, opts) {
        match ev {
            Event::Start(tag) => {
                match tag {
                    Tag::Paragraph if depth == 0 => out.push(B::P),
                    Tag::Heading { level, .. } if depth == 0 => out.push(B::H(level as u8)),
                    Tag::CodeBlock(_) if depth == 0 => out.push(B::Pre),
                    // 容器：只有 depth==0 才作为顶层块 push；嵌套容器（Quote 内部的 List、
                    // List 内部的嵌套 List 等）不应出现在顶层序列里。depth 自增在外层。
                    Tag::BlockQuote(_) => {
                        if depth == 0 {
                            out.push(B::Quote);
                        }
                        depth += 1;
                    }
                    Tag::List(_) => {
                        if depth == 0 {
                            out.push(B::List);
                        }
                        depth += 1;
                    }
                    Tag::Table(_) => {
                        if depth == 0 {
                            out.push(B::Table);
                        }
                        depth += 1;
                    }
                    Tag::HtmlBlock => {} // 原始 HTML 块两侧都不计
                    _ => {}
                }
            }
            Event::Rule if depth == 0 => out.push(B::Rule),
            Event::End(TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Table) => depth -= 1,
            Event::End(_) => {}
            _ => {}
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum B {
    P,
    H(u8),
    Pre,
    Quote,
    List,
    Table,
    Rule,
    Other,
}

/// 期望 HTML 的顶层「Markdown 生成块」序列。
///
/// 口径：只看 Markdown 生成块，原始 HTML 块不计——源码原样输出，从输出侧无法与
/// 生成标签可靠区分（`<div>` 未闭合、`<pre>` 无 `<code>` 等）；其渲染正确性由
/// Suite B 导出比对覆盖。扫描按行，生成块总是在行首开始标签：
/// `<p>` / `<hN>` / `<pre><code`（原始 `<pre>` 不算）/ `<blockquote>` /
/// `<ul>` / `<ol>` / `<hr />`；`gfm_tables` 为真时另认生成 `<table>`。
/// 引用与列表整体折叠为一个顶层块，内部块不进入序列（与实际块序列口径一致）。
pub fn expected_blocks(html: &str, gfm_tables: bool) -> Vec<B> {
    let lines: Vec<&str> = html.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if l.trim_end() == "<hr />" {
            out.push(B::Rule);
            i += 1;
            continue;
        }
        let (b, close): (B, String) = if l.starts_with("<pre><code") {
            (B::Pre, "</pre>".to_string())
        } else if l.starts_with("<p>") {
            (B::P, "</p>".to_string())
        } else if l.starts_with("<blockquote>") {
            (B::Quote, "</blockquote>".to_string())
        } else if l.starts_with("<ul>") || l.starts_with("<ul ") {
            (B::List, "</ul>".to_string())
        } else if l.starts_with("<ol>") || l.starts_with("<ol ") {
            (B::List, "</ol>".to_string())
        } else if gfm_tables && (l.starts_with("<table>") || l.starts_with("<table ")) {
            (B::Table, "</table>".to_string())
        } else if let Some(level) = heading_level(l) {
            (B::H(level), format!("</h{level}>"))
        } else {
            i += 1;
            continue;
        };
        out.push(b);
        i = if matches!(b, B::Quote | B::List) {
            skip_container(&lines, i, &close)
        } else {
            skip_until(&lines, i, &close)
        };
    }
    out
}

/// 行首生成标题 `<h1>`–`<h6>`（排除 `<hr`、`<html`）。
fn heading_level(l: &str) -> Option<u8> {
    let rest = l.strip_prefix("<h")?.as_bytes();
    match (rest.first(), rest.get(1)) {
        (Some(d), Some(b'>')) if (b'1'..=b'6').contains(d) => Some(d - b'0'),
        _ => None,
    }
}

/// 单行容器（p/pre/hN/table）：跳至首个含闭合标签的行。
fn skip_until(lines: &[&str], mut i: usize, close: &str) -> usize {
    while i < lines.len() {
        let hit = lines[i].contains(close);
        i += 1;
        if hit {
            break;
        }
    }
    i
}

/// 引用/列表容器：按开关标签计深度，跳过整个容器（嵌套块不进入顶层序列）。
fn skip_container(lines: &[&str], mut i: usize, close: &str) -> usize {
    let mut depth = 0i32;
    while i < lines.len() {
        let l = lines[i];
        if close == "</blockquote>" {
            depth += l.matches("<blockquote").count() as i32;
            depth -= l.matches("</blockquote").count() as i32;
        } else {
            depth += l.matches("<ul").count() as i32 + l.matches("<ol").count() as i32;
            depth -= l.matches("</ul>").count() as i32 + l.matches("</ol>").count() as i32;
        }
        i += 1;
        if depth <= 0 {
            break;
        }
    }
    i
}

/// Veil 自有块解析器的顶层块序列；同签名的连续列表项折叠为一个 List（与规范
/// `<ul>`/`<ol>` 对齐；0c P0-3：标记变化如 `-`→`+` / `.`→`)` 起新列表，见
/// Example 301/302）。续块（首行无合法标记，签名为 None）静默随前项归并。
/// LinkRefDef 无 HTML 输出，跳过（规范期望序列中定义行不对应任何顶层块）。
/// 注：`BlockKind::ListItem.depth` 是「缩进列 / 2」的视觉深度，不是结构嵌套层级
/// （见 block.rs §v0 取舍）：缩进 1–3 列的顶层列表同样得到 depth ≥ 1，用
/// depth == 0 判定顶层会把 Example 4/108/271/287/288/290/291 的缩进顶层列表漏计。
/// 顶层/嵌套改按 CommonMark 几何判定——子项缩进 ≥ 父项内容列才属嵌套（Ex 297 的
/// `10) foo` 内容列 4，后继 3 空格 `- bar` 仍是顶层新列表）。
pub fn actual_blocks(s: &Session) -> Vec<B> {
    let mut out = Vec::new();
    // 当前顶层列表签名（None = 无活跃顶层列表）
    let mut cur_sig: Option<(bool, u8)> = None;
    // 开放列表项栈：(内容列, 签名)；仅用于判定后继项是嵌套还是同层/顶层
    let mut stack: Vec<(usize, (bool, u8))> = Vec::new();
    for (_, kind, range) in s.blocks() {
        if let BlockKind::ListItem { .. } = kind {
            let text = s.text_slice(range).unwrap_or_default();
            let first = text.split_inclusive('\n').next().unwrap_or(&text);
            let Some((indent, content_col, sig)) = list_geom(first) else {
                // 续块（首行无合法标记）随前项归并，不产生新块
                continue;
            };
            // 关闭缩进不足的开放项
            while let Some(&(col, _)) = stack.last() {
                if indent >= col {
                    break;
                }
                stack.pop();
            }
            if stack.is_empty() && cur_sig != Some(sig) {
                // 顶层项且标记签名变化（`-`→`+` / `.`→`)`）→ 开新 List（Ex 301/302）
                cur_sig = Some(sig);
                out.push(B::List);
            }
            // 嵌套子项（含不同标记）归属当前顶层 List，不产生新块
            stack.push((content_col, sig));
            continue;
        }
        stack.clear();
        cur_sig = None;
        let b = match kind {
            BlockKind::Paragraph => B::P,
            BlockKind::AtxHeading(n) | BlockKind::SetextHeading(n) => B::H(n),
            BlockKind::FencedCode(_) | BlockKind::IndentedCode | BlockKind::Mermaid => B::Pre,
            BlockKind::Math => B::Other,
            BlockKind::Table => B::Table,
            BlockKind::BlockQuote | BlockKind::Admonition(_) | BlockKind::Callout(_) => B::Quote,
            BlockKind::ThematicBreak => B::Rule,
            BlockKind::LinkRefDef
            | BlockKind::FootnoteDef
            | BlockKind::FrontMatter(_)
            | BlockKind::Details { .. } => continue,
            BlockKind::HtmlBlock => continue,
            BlockKind::ListItem { .. } => unreachable!("列表项已在上方处理"),
        };
        out.push(b);
    }
    out
}

/// 列表项首行几何（测试侧简化版 `block.rs::list_mark`）：
/// 返回 `(缩进列, 内容列, 签名)`；无合法标记返回 None。Tab 按 4 列制表位展开。
fn list_geom(line: &str) -> Option<(usize, usize, (bool, u8))> {
    let b = line.as_bytes();
    let mut i = 0usize;
    let mut col = 0usize;
    while i < b.len() {
        match b[i] {
            b' ' => {
                col += 1;
                i += 1;
            }
            b'\t' => {
                col += 4 - (col % 4);
                i += 1;
            }
            _ => break,
        }
    }
    let indent = col;
    let (ordered, marker, marker_len) = if b.get(i)?.is_ascii_digit() {
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() && (i - start) < 9 {
            i += 1;
        }
        let marker = match b.get(i)? {
            b'.' => b'.',
            b')' => b')',
            _ => return None,
        };
        i += 1;
        (true, marker, i - start)
    } else {
        let marker = match b.get(i)? {
            b'-' => b'-',
            b'*' => b'*',
            b'+' => b'+',
            _ => return None,
        };
        i += 1;
        (false, marker, 1usize)
    };
    // 标记后须为空白或行尾（`-tag` 非列表）
    let mut spaces = 0usize;
    while i < b.len() {
        match b[i] {
            b' ' => {
                spaces += 1;
                i += 1;
            }
            b'\t' => {
                spaces += 4 - ((indent + marker_len + spaces) % 4);
                i += 1;
            }
            _ => break,
        }
    }
    // 标记后须为空白或行尾（`-tag` 非列表）；`\r`/`\n` 视为行尾
    if i < b.len() && spaces == 0 && !matches!(b[i], b'\r' | b'\n') {
        return None;
    }
    let blank = line[i..].trim().is_empty();
    let content_col = if blank || spaces >= 5 {
        indent + marker_len + 1
    } else {
        indent + marker_len + spaces
    };
    Some((indent, content_col, (ordered, marker)))
}

/// 产品导出路径的分段渲染：连续列表项合并为一段源码，其余块逐块渲染。
/// 引用式链接跨块可见性：非 Math 块源码前拼接全部定义源码（与产品 export.rs 一致）。
pub fn export_body(s: &Session) -> String {
    let mut body = String::new();
    let blocks = s.blocks();
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut defs = String::new();
    for (_, kind, range) in &blocks {
        if *kind == BlockKind::LinkRefDef {
            if let Some(t) = s.text_slice(*range) {
                defs.push_str(&t);
                if !t.ends_with('\n') {
                    defs.push('\n');
                }
            }
        }
    }
    let mut i = 0;
    while i < blocks.len() {
        let (_, kind, range) = blocks[i].clone();
        if kind == BlockKind::Math
            || kind == BlockKind::LinkRefDef
            || kind == BlockKind::FootnoteDef
        {
            i += 1;
            continue;
        }
        if matches!(kind, BlockKind::ListItem { .. }) {
            let start = range.start;
            let mut end = range.end;
            let mut j = i + 1;
            while let Some((_, k, r)) = blocks.get(j) {
                if matches!(k, BlockKind::ListItem { .. }) {
                    end = r.end;
                    j += 1;
                } else {
                    break;
                }
            }
            if let Some(text) = s.text_slice(Range::new(start, end)) {
                let mut nodes = Vec::new();
                for (item_id, _, _) in &blocks[i..j] {
                    if let Some(ns) = s.block_inlines(*item_id) {
                        nodes.extend(ns);
                    }
                }
                let rendered = inject_bare_links(&text, start, &nodes);
                html::push_html(
                    &mut body,
                    Parser::new_ext(&format!("{defs}{rendered}"), opts),
                );
            }
            i = j;
            continue;
        }
        if let Some(text) = s.text_slice(range) {
            let nodes = if kind == BlockKind::Table {
                s.table(blocks[i].0).map_or_else(Vec::new, |t| {
                    t.header
                        .iter()
                        .chain(t.rows.iter().flatten())
                        .flat_map(|c| c.inlines.iter().cloned())
                        .collect()
                })
            } else {
                s.block_inlines(blocks[i].0).unwrap_or_default()
            };
            let rendered = inject_bare_links(&text, range.start, &nodes);
            html::push_html(
                &mut body,
                Parser::new_ext(&format!("{defs}{rendered}"), opts),
            );
        }
        i += 1;
    }
    body
}

/// 裸链接成链（GFM 自动链接扩展）的导出复刻：把 `url == range` 的 Link 节点替换为
/// raw `<a href>`。产品侧同逻辑见 `veil/src/export.rs` 的 `render_block_inlines`
/// （本文件刻意复刻产品路径，不依赖 veil crate）。
fn inject_bare_links(text: &str, base: usize, nodes: &[InlineNode]) -> String {
    fn collect(nodes: &[InlineNode], out: &mut Vec<InlineNode>) {
        for n in nodes {
            if n.kind == InlineKind::Link && n.url == Some(n.range) && n.content == n.range {
                out.push(n.clone());
            }
            collect(&n.children, out);
        }
    }
    let mut links = Vec::new();
    collect(nodes, &mut links);
    links.sort_by_key(|n| n.range.start);
    let mut out = String::with_capacity(text.len());
    let mut p = 0usize;
    for node in links {
        let Some(url) = node.url.and_then(|u| s_slice(text, base, u)) else {
            continue;
        };
        let (Some(st), Some(en)) = (
            node.range.start.checked_sub(base),
            node.range.end.checked_sub(base),
        ) else {
            continue;
        };
        if st < p || en > text.len() || !text.is_char_boundary(st) || !text.is_char_boundary(en) {
            continue;
        }
        out.push_str(&text[p..st]);
        out.push_str(&format!(
            "<a href=\"{}\">{}</a>",
            escape_attr(&link_href(url)),
            escape_link_text(url)
        ));
        p = en;
    }
    out.push_str(&text[p..]);
    out
}

/// 从块内文本按绝对区间取切片（越界/非字符边界返回 None）。
fn s_slice(text: &str, base: usize, r: Range) -> Option<&str> {
    let s = r.start.checked_sub(base)?;
    let e = r.end.checked_sub(base)?;
    if e > text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
        return None;
    }
    Some(&text[s..e])
}

/// 产品 `veil_ui::normalize_link_url` 的复刻：`www.` 补 http，裸邮箱补 mailto。
fn link_href(url: &str) -> String {
    if url.len() >= 4 && url.as_bytes()[..4].eq_ignore_ascii_case(b"www.") {
        return format!("http://{url}");
    }
    if url.contains("://") || url.chars().any(char::is_whitespace) {
        return url.to_string();
    }
    let Some((local, domain)) = url.split_once('@') else {
        return url.to_string();
    };
    let email = !local.is_empty()
        && !local.contains(':')
        && domain.contains('.')
        && domain
            .as_bytes()
            .last()
            .is_some_and(|b| b.is_ascii_alphanumeric());
    if email {
        format!("mailto:{url}")
    } else {
        url.to_string()
    }
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 裸链接可见文本：HTML 转义 + markdown 标记字符改数字实体（保持纯文本）。
fn escape_link_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '*' | '_' | '~' | '`' | '[' | ']' | '\\' => {
                out.push_str(&format!("&#{};", c as u32));
            }
            _ => out.push(c),
        }
    }
    out
}

/// GFM Suite B 的额外等价化：cmark-gfm 规范期望 HTML 与 pulldown-cmark（产品导出
/// 渲染器）在**序列化形式**上不同，语义相同。只抹平以下三类，不改结构：
/// - 表格对齐：cmark-gfm 写 `align="x"`，pulldown 写 `style="text-align: x"`；
/// - 任务列表复选框：属性顺序与自闭合斜杠（`<input checked="" type="checkbox">`
///   vs `<input type="checkbox" checked=""/>`）；
/// - 无表体行的表格：cmark-gfm 不输出 `<tbody>`，pulldown 输出空 `<tbody>`。
pub fn gfm_canonical(html: &str) -> String {
    let mut s = canonical(html)
        .replace("<tbody></tbody>", "")
        .replace("<tbody> </tbody>", "");
    s = normalize_align(&s);
    normalize_input(&s)
}

/// 在 `<th…>` / `<td…>` 标签内把 `align="x"` 改写为 `style="text-align: x"`。
fn normalize_align(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    while let Some(off) = html[i..].find('<') {
        let start = i + off;
        out.push_str(&html[i..start]);
        let Some(end) = html[start..].find('>') else {
            out.push_str(&html[start..]);
            return out;
        };
        let tag = &html[start..=start + end];
        let is_cell = tag.starts_with("<th") || tag.starts_with("<td");
        if !is_cell {
            out.push_str(tag);
        } else {
            let open = tag.find(' ').unwrap_or(tag.len() - 1);
            let (head, attrs) = tag.split_at(open);
            out.push_str(head);
            for attr in attrs[..attrs.len() - 1].split_whitespace() {
                let style = attr
                    .strip_prefix("align=\"")
                    .and_then(|v| v.strip_suffix('"'))
                    .map(|v| format!("style=\"text-align: {v}\""));
                out.push(' ');
                out.push_str(style.as_deref().unwrap_or(attr));
            }
            out.push('>');
        }
        i = start + end + 1;
    }
    out.push_str(&html[i..]);
    out
}

/// 归一化 `<input …>`：属性按字典序重排，去掉自闭合 `/`。
fn normalize_input(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    while let Some(off) = html[i..].find("<input") {
        let start = i + off;
        out.push_str(&html[i..start]);
        let Some(end) = html[start..].find('>') else {
            out.push_str(&html[start..]);
            return out;
        };
        let inner = html[start + 6..start + end].trim_end_matches('/').trim();
        let mut attrs: Vec<&str> = inner.split_whitespace().collect();
        attrs.sort_unstable();
        out.push_str("<input");
        for a in attrs {
            out.push(' ');
            out.push_str(a);
        }
        out.push('>');
        i = start + end + 1;
    }
    out.push_str(&html[i..]);
    out
}

/// 语义近似：标签保留；`<pre>` 外文本折叠空白，标签间空白文本丢弃。
pub fn canonical(html: &str) -> String {
    let mut out = String::new();
    let mut pre = 0i32;
    let mut i = 0usize;
    while i < html.len() {
        let Some(off) = html[i..].find('<') else {
            push_text(&mut out, &html[i..], pre > 0);
            break;
        };
        if off > 0 {
            push_text(&mut out, &html[i..i + off], pre > 0);
            i += off;
        }
        let Some(end) = html[i..].find('>') else {
            push_text(&mut out, &html[i..], pre > 0);
            break;
        };
        let tag = &html[i..=i + end];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<pre") {
            pre += 1;
        } else if lower.starts_with("</pre") {
            pre -= 1;
        }
        out.push_str(tag);
        i += end + 1;
    }
    out
}

fn push_text(out: &mut String, text: &str, keep: bool) {
    if keep {
        out.push_str(text);
    } else {
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push_str(&collapsed);
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub strong: usize,
    pub emph: usize,
    pub code: usize,
    pub link: usize,
    pub image: usize,
    pub del: usize,
}

/// 行内参照：pulldown-cmark（CommonMark 参考实现）在源码上的行内事件。
/// 原始 HTML 以 `Event::Html` 透传，不产生 Link/Image/Del 事件，因此天然避开
/// 「期望 HTML 中源码原样输出的 `<a>`/`<img>`/`<del>` 被误计」的问题。
/// 返回 (计数, 前序签名)；Math 不计（与 Veil 口径一致）。
pub fn reference_inline(md: &str) -> (Counts, Vec<(&'static str, usize)>) {
    fn start_kind(tag: &Tag) -> Option<&'static str> {
        match tag {
            Tag::Emphasis => Some("em"),
            Tag::Strong => Some("strong"),
            Tag::Link { .. } => Some("link"),
            Tag::Image { .. } => Some("image"),
            Tag::Strikethrough => Some("del"),
            _ => None,
        }
    }
    fn end_kind(end: &TagEnd) -> Option<&'static str> {
        match end {
            TagEnd::Emphasis => Some("em"),
            TagEnd::Strong => Some("strong"),
            TagEnd::Link => Some("link"),
            TagEnd::Image => Some("image"),
            TagEnd::Strikethrough => Some("del"),
            _ => None,
        }
    }
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut c = Counts::default();
    let mut sig = Vec::new();
    let mut stack: Vec<&'static str> = Vec::new();
    // Veil 模型：图片 alt 为纯文本、不产生子节点（CommonMark 描述内可含行内标记，
    // 渲染时并入 alt 文本）。参照侧同口径：图片子树整体视为叶子。
    let mut image_depth = 0usize;
    for ev in Parser::new_ext(md, opts) {
        if image_depth > 0 {
            match ev {
                Event::Start(Tag::Image { .. }) => image_depth += 1,
                Event::End(TagEnd::Image) => image_depth -= 1,
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(Tag::Image { .. }) => {
                c.image += 1;
                sig.push(("image", stack.len()));
                image_depth = 1;
            }
            Event::Start(tag) => {
                if let Some(k) = start_kind(&tag) {
                    match k {
                        "em" => c.emph += 1,
                        "strong" => c.strong += 1,
                        "link" => c.link += 1,
                        _ => c.del += 1,
                    }
                    sig.push((k, stack.len()));
                    stack.push(k);
                }
            }
            Event::End(end) => {
                if let Some(k) = end_kind(&end) {
                    if stack.last() == Some(&k) {
                        stack.pop();
                    }
                }
            }
            Event::Code(_) => {
                c.code += 1;
                sig.push(("code", stack.len()));
            }
            _ => {}
        }
    }
    (c, sig)
}

pub fn actual_inline(s: &Session) -> Counts {
    fn walk(nodes: &[InlineNode], c: &mut Counts) {
        for n in nodes {
            match n.kind {
                InlineKind::Strong => c.strong += 1,
                InlineKind::Emph => c.emph += 1,
                InlineKind::Code => c.code += 1,
                InlineKind::Link => c.link += 1,
                InlineKind::Image => c.image += 1,
                InlineKind::Strikethrough => c.del += 1,
                // 高亮在 CommonMark/GFM 参照侧无对应事件：不计数，子节点照常递归。
                InlineKind::Highlight => {}
                InlineKind::Math => {}
                InlineKind::FootnoteRef => {}
                // Task 3-5：Html 标签在既有 inline 计数中不单独统计，子节点由递归覆盖
                InlineKind::Html { .. }
                | InlineKind::HtmlSelfClosed { .. }
                | InlineKind::HtmlLiteral => {}
            }
            walk(&n.children, c);
        }
    }
    let mut c = Counts::default();
    for (id, kind, _) in s.blocks() {
        // 表格单元格是 GFM 的 inline 解析位置（`table()` 暴露节点；块级恒为空）。
        if kind == BlockKind::Table {
            if let Some(t) = s.table(id) {
                for cell in t.header.iter().chain(t.rows.iter().flatten()) {
                    walk(&cell.inlines, &mut c);
                }
            }
        } else if let Some(nodes) = s.block_inlines(id) {
            walk(&nodes, &mut c);
        }
    }
    c
}

// ===== Suite D：行内前序森林签名 (kind, depth) =====

// 期望侧签名由 `reference_inline` 一并产出（与 Suite C 同源），见上方。

/// Veil 行内树的前序签名；Math 不产生标签（与 C 口径一致）。
pub fn actual_inline_structure(s: &Session) -> Vec<(&'static str, usize)> {
    fn walk(nodes: &[InlineNode], depth: usize, out: &mut Vec<(&'static str, usize)>) {
        for n in nodes {
            let kind = match n.kind {
                InlineKind::Strong => Some("strong"),
                InlineKind::Emph => Some("em"),
                InlineKind::Code => Some("code"),
                InlineKind::Link => Some("link"),
                InlineKind::Image => Some("image"),
                InlineKind::Strikethrough => Some("del"),
                // 高亮是 Veil 扩展，参照侧无标签：作为透明容器（子节点同深度）。
                InlineKind::Highlight => None,
                InlineKind::Math | InlineKind::FootnoteRef => None,
                // 行内 HTML 在参照侧是 raw HTML 事件：标签自身不计数，子节点按原深度透视
                // （与 Suite C 的 `actual_inline` 递归口径一致，否则 `<del>*foo*</del>`
                // 的子 Emphasis 会被整棵跳过）。
                InlineKind::Html { .. }
                | InlineKind::HtmlSelfClosed { .. }
                | InlineKind::HtmlLiteral => None,
            };
            match kind {
                Some(k) => {
                    out.push((k, depth));
                    walk(&n.children, depth + 1, out);
                }
                None => walk(&n.children, depth, out),
            }
        }
    }
    let mut out = Vec::new();
    for (id, kind, _) in s.blocks() {
        if kind == BlockKind::Table {
            if let Some(t) = s.table(id) {
                for cell in t.header.iter().chain(t.rows.iter().flatten()) {
                    walk(&cell.inlines, 0, &mut out);
                }
            }
        } else if let Some(nodes) = s.block_inlines(id) {
            walk(&nodes, 0, &mut out);
        }
    }
    out
}

pub fn sig_short(sig: &[(&'static str, usize)]) -> String {
    let mut s = String::new();
    for (k, d) in sig.iter().take(12) {
        s.push_str(&format!("{k}@{d} "));
    }
    if sig.len() > 12 {
        s.push('…');
    }
    s.trim_end().to_string()
}

pub fn short(seq: &[B]) -> String {
    let mut s = String::new();
    for b in seq.iter().take(8) {
        s.push_str(&format!("{b:?} "));
    }
    if seq.len() > 8 {
        s.push('…');
    }
    s.trim_end().to_string()
}
