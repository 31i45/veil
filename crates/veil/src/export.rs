//! HTML 导出（切片 7）：逐块翻译为自包含 HTML 文档。
//! Math 块经 latex-rust → 内嵌 SVG（字形为 path，无外部字体/脚本依赖）；
//! 其余块交 pulldown-cmark（CommonMark 参考实现，仅作导出辅助，不进编辑主链路）。
//! 公式渲染失败降级为转义源码 <pre>，绝不中断导出。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use veil_core::{heading_slugs, BlockKind, HtmlTag, InlineKind, InlineNode, Range, Session};
use veil_ui::{ExportStyle, Exporter, FileStore};

/// 文档外壳：最小自包含模板（无外部资源）。样式由当前主题快照生成，
/// 使导出产物与编辑器屏幕配色一致（深色主题导出深色页面）。
/// 为什么用 format! 而非固定常量：只有 6 个色值参与拼接，成本可忽略；
/// 这样无需维护第二份调色板，也避免「改了主题忘了改导出」的漂移。
fn head(style: &ExportStyle) -> String {
    let bg = hex(style.bg);
    let text = hex(style.text);
    let muted = hex(style.muted);
    let link = hex(style.link);
    let code_bg = hex(style.code_bg);
    let border = hex(style.border);
    // 深色页面须声明 color-scheme：否则浏览器默认的滚动条/表单控件仍是浅色，观感割裂
    let scheme = if style.dark {
        "color-scheme: dark;\n"
    } else {
        ""
    };
    format!(
        "<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>veil</title>\n<style>\n{scheme}\
         body{{max-width:44em;margin:2em auto;padding:0 1em;\
         font:16px/1.6 system-ui,sans-serif;background:{bg};color:{text}}}\n\
         a{{color:{link}}}\n\
         code,pre{{background:{code_bg};border:1px solid {border};\
         font-family:Consolas,Menlo,monospace}}\n\
         code{{padding:.1em .3em}}\n\
         pre{{padding:.8em;overflow-x:auto}}\n\
         pre code{{padding:0;border:none}}\n\
         kbd{{border:1px solid {border};border-radius:3px;padding:.05em .35em;\
         font-size:.9em;font-weight:600;background:{code_bg};\
         box-shadow:0 1px 0 {border}}}\n\
         sub,sup{{font-size:.75em;line-height:0}}\n\
         sub{{vertical-align:-0.25em}}\n\
         sup{{vertical-align:0.5em}}\n\
         mark{{background:#fff799;color:inherit;padding:0 .1em;border-radius:2px}}\n\
         abbr[title]{{text-decoration:underline dotted;cursor:help}}\n\
         details{{margin:.8em 0;padding:.2em .6em;border-left:3px solid {border}}}\n\
         summary{{cursor:pointer;font-weight:600}}\n\
         details[open] summary{{margin-bottom:.4em}}\n\
         img{{max-width:100%;height:auto;max-height:3.2em}}\n\
         p>img:only-child{{max-height:9.6em}}\n\
         td img,th img{{max-height:2.4em}}\n\
         blockquote{{border-left:4px solid {border};margin-left:0;padding-left:1em;color:{muted}}}\n\
         table{{border-collapse:collapse}}\n\
         th,td{{border:1px solid {border};padding:.4em .6em}}\n\
         hr{{border:none;border-top:1px solid {border}}}\n\
         div.math{{text-align:center;margin:1em 0}}\n\
         pre.math-source{{color:#a00;background:#fdf5f5}}\n\
         .fnref a{{text-decoration:none}}\n\
         .footnotes{{border-top:1px solid {border};margin-top:2em;\
         padding-top:.5em;color:{muted};font-size:.9em}}\n\
         </style>\n</head>\n<body>\n"
    )
}

/// 颜色 → CSS 十六进制（小写，`#rrggbb`）。
fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// 会话 → 自包含 HTML 文档（纯函数，headless 可测）。
/// `style` 为当前主题快照（值语义），由调用方（App）在主题切换点同步，本函数不感知主题。
pub fn html_of(session: &Session, style: &ExportStyle, font: &latex_rust::MathFont) -> String {
    let mut body = String::new();
    let blocks = session.blocks();
    // 表格依赖 GFM 扩展，须显式开启（Parser::new 默认关闭 tables）
    let opts = pulldown_cmark::Options::ENABLE_TABLES
        | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
        | pulldown_cmark::Options::ENABLE_TASKLISTS;
    // 引用式链接跨块可见性（0c P0-1）：逐块独立解析时定义对后续块不可见，
    // 故收集全部定义源码作为每个解析单元的前缀；定义无 HTML 输出，重复定义 first-wins 无害。
    let mut defs = String::new();
    for (_, kind, range) in &blocks {
        if *kind == BlockKind::LinkRefDef {
            if let Some(t) = session.text_slice(*range) {
                defs.push_str(&t);
                if !t.ends_with('\n') {
                    defs.push('\n');
                }
            }
        }
    }
    // 标题锚点：必须一次收集全文标题文本再交给 core 的 `heading_slugs` 去重——
    // 逐块生成时无法知道同名标题已占用到第几个后缀。文本取 `Session::heading`
    // 的可见文本（与大纲同一口径，已剥 ATX/Setext 标记与 inline 标记），
    // 下游渲染的行内内容也从同一内容区间切出，二者保证同源。
    let mut heading_positions = Vec::new();
    let mut heading_texts = Vec::new();
    for (pos, (id, kind, _)) in blocks.iter().enumerate() {
        if heading_level(kind).is_some() {
            heading_positions.push(pos);
            heading_texts.push(session.heading(*id).map(|(_, t)| t).unwrap_or_default());
        }
    }
    let slugs: HashMap<usize, String> = heading_positions
        .into_iter()
        .zip(heading_slugs(heading_texts.iter().map(String::as_str)))
        .collect();
    // 脚注定义一次收齐（重复定义 first-wins，与 core 一致）；引用替换与文末
    // 脚注区共用这张表。
    let footnote_defs = collect_footnote_defs(session, &blocks);
    // 同一 id 的引用计数：决定重复引用的 id 后缀（fnref-n / fnref-n-2 …）。
    let mut fn_occurrences: HashMap<String, usize> = HashMap::new();
    // B3：details 头块只输出开标签 + summary；内部子块（`details_depth` 更深）
    // 紧跟其后依次输出，遇到深度不大于开标签的块时补闭合标签（嵌套 details 用栈）。
    let mut details_stack: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        let (id, kind, range) = blocks[i].clone();
        let depth = session.block_container(id).details_depth;
        while let Some(&open_depth) = details_stack.last() {
            if depth <= open_depth {
                body.push_str("</details>\n");
                details_stack.pop();
            } else {
                break;
            }
        }
        if kind == BlockKind::Math {
            let tex = session.math_source(id).unwrap_or_default();
            body.push_str(&math_html(&tex, font));
            i += 1;
            continue;
        }
        // N3b：内置 GFM Alerts 与自定义 Callout 统一输出带类型的 `<div class="callout">`
        // （内联样式自包含；引用缩进语义由内层 blockquote 内容保留）。
        if matches!(kind, BlockKind::Admonition(_) | BlockKind::Callout(_)) {
            body.push_str(&callout_html(
                session,
                &mut fn_occurrences,
                id,
                range,
                &kind,
                &defs,
                opts,
            ));
            i += 1;
            continue;
        }
        // 定义块自身无输出（链接定义已并入 defs 前缀；脚注定义统一在文末输出）
        if matches!(kind, BlockKind::LinkRefDef | BlockKind::FootnoteDef) {
            i += 1;
            continue;
        }
        // Task 3-5：Details 块手动渲染，强制加 `open` 保证浏览器默认展开
        // （编辑器默认收起，但导出给读者应展开以避免隐藏内容）。summary 文本里
        // 若含内联格式（Html/HtmlLiteral）先经 render_block_inlines 处理。
        if let BlockKind::Details { summary, .. } = &kind {
            // summary 行里可能嵌 Html 标签（如 `<kbd>`），用空 base 简化处理
            body.push_str(&format!(
                "<details open><summary>{}</summary>\n",
                escape_html(summary)
            ));
            details_stack.push(depth);
            i += 1;
            continue;
        }
        // 标题不走通用的 Heading 事件：pulldown 的 Heading 不带 id，且 slug 需
        // 文档级去重。这里只手写 <hN id>，内部仍交 pulldown 渲染行内格式。
        if let Some(level) = heading_level(&kind) {
            let text = session.text_slice(range).unwrap_or_default();
            let (offset, content) = heading_content(&kind, &text);
            let nodes = session.block_inlines(id).unwrap_or_default();
            let rendered = render_block_inlines(
                session,
                &mut fn_occurrences,
                content,
                range.start + offset,
                &nodes,
            );
            let inner = inline_html(&defs, &rendered, opts);
            let slug = slugs.get(&i).map(String::as_str).unwrap_or("");
            body.push_str(&format!("<h{level} id=\"{slug}\">{inner}</h{level}>\n"));
            i += 1;
            continue;
        }
        // 连续列表项合并为一段源码再解析（切片 12）：逐项独立解析会碎裂成多个
        // <ul>/<ol>，嵌套（混合有序性）与紧凑/松散列表语义随之丢失；原文连续
        // 切片交 pulldown 恰好还原 GFM 行为。
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
            if let Some(text) = session.text_slice(Range::new(start, end)) {
                // 脚注引用节点分散在各列表项块里，合并源码前按各自绝对区间替换
                let mut nodes = Vec::new();
                for (item_id, _, _) in &blocks[i..j] {
                    if let Some(ns) = session.block_inlines(*item_id) {
                        nodes.extend(ns);
                    }
                }
                let rendered =
                    render_block_inlines(session, &mut fn_occurrences, &text, start, &nodes);
                pulldown_cmark::html::push_html(
                    &mut body,
                    pulldown_cmark::Parser::new_ext(&format!("{defs}{rendered}"), opts),
                );
            }
            i = j;
            continue;
        }
        // v0 其余块类型均自包含（段落/代码/表格/引用/分隔线），逐块独立解析无跨块上下文
        // B2：details 内表格的块区间可能吞入闭标签行，导出只取真实表格区间。
        let span = if kind == BlockKind::Table {
            session.table_span(id).unwrap_or(range)
        } else {
            range
        };
        let Some(text) = session.text_slice(span) else {
            i += 1;
            continue;
        };
        // 未知 Callout 类型回退普通引用块：把首行 `[!XXX]` 换成等长空格（保持
        // 行内节点偏移不变），不得把裸标记文本带进导出件。
        let text = if kind == BlockKind::BlockQuote {
            blank_callout_marker(&text)
        } else {
            std::borrow::Cow::Borrowed(text.as_str())
        };
        // 表格的行内节点不在 block_inlines 上（单元格内解析），单独收集，使表格内的
        // 裸链接与正文同样成链；其余块直接取块级行内节点。
        let nodes = if kind == BlockKind::Table {
            session.table(id).map_or_else(Vec::new, |t| {
                t.header
                    .iter()
                    .chain(t.rows.iter().flatten())
                    .flat_map(|c| c.inlines.iter().cloned())
                    .collect()
            })
        } else {
            session.block_inlines(id).unwrap_or_default()
        };
        let rendered =
            render_block_inlines(session, &mut fn_occurrences, &text, span.start, &nodes);
        pulldown_cmark::html::push_html(
            &mut body,
            pulldown_cmark::Parser::new_ext(&format!("{defs}{rendered}"), opts),
        );
        i += 1;
    }
    while !details_stack.is_empty() {
        body.push_str("</details>\n");
        details_stack.pop();
    }
    let notes = footnotes_html(session, &footnote_defs, opts);
    format!("{}{body}{notes}</body>\n</html>\n", head(style))
}

/// 引用块首行 `[!NAME]` 标记区间（字节，行内相对块起点）：缩进 + `>` + 空白 +
/// `[!` + name + `]`。与 core/预览同一识别口径。
fn callout_marker_span(text: &str) -> Option<(usize, usize)> {
    let line = text.split_inclusive('\n').next()?;
    let b = line.as_bytes();
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    if b.get(indent) != Some(&b'>') {
        return None;
    }
    let mut i = indent + 1;
    while matches!(b.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if b.get(i) != Some(&b'[') || b.get(i + 1) != Some(&b'!') {
        return None;
    }
    let close = line[i..].find(']')? + i;
    Some((i, close + 1))
}

/// 把首行 `[!NAME]` 替换为等长空格：未知类型回退普通引用块时不得露出裸标记，
/// 等长替换保证行内节点绝对偏移不变。
fn blank_callout_marker(text: &str) -> std::borrow::Cow<'_, str> {
    match callout_marker_span(text) {
        Some((s, e)) => {
            let mut out = String::with_capacity(text.len());
            out.push_str(&text[..s]);
            out.push_str(&" ".repeat(e - s));
            out.push_str(&text[e..]);
            std::borrow::Cow::Owned(out)
        }
        None => std::borrow::Cow::Borrowed(text),
    }
}

/// Callout 块 → 自包含 `<div class="callout callout-<name>">`（内联样式）。
/// 正文仍交 pulldown 渲染，去掉外层 `<blockquote>` 包裹（`>` 缩进语义由原源码保留）。
fn callout_html(
    session: &Session,
    occurrences: &mut HashMap<String, usize>,
    id: veil_core::BlockId,
    range: Range,
    kind: &BlockKind,
    defs: &str,
    opts: pulldown_cmark::Options,
) -> String {
    let (name, label, icon, color) = match kind {
        BlockKind::Admonition(ty) => (
            ty.name().to_string(),
            ty.label().to_string(),
            ty.icon().to_string(),
            ty.color(),
        ),
        BlockKind::Callout(c) => (c.name.clone(), c.label.clone(), c.icon.clone(), c.color),
        _ => return String::new(),
    };
    let Some(text) = session.text_slice(range) else {
        return String::new();
    };
    let blanked = blank_callout_marker(&text);
    let nodes = session.block_inlines(id).unwrap_or_default();
    let rendered = render_block_inlines(session, occurrences, &blanked, range.start, &nodes);
    let mut html = String::new();
    pulldown_cmark::html::push_html(
        &mut html,
        pulldown_cmark::Parser::new_ext(&format!("{defs}{rendered}"), opts),
    );
    let inner = html.strip_prefix("<blockquote>\n").unwrap_or(&html);
    let inner = inner.strip_suffix("</blockquote>\n").unwrap_or(inner);
    let rgb = format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]);
    let title = if icon.is_empty() {
        escape_html(&label)
    } else {
        format!("{} {}", escape_html(&icon), escape_html(&label))
    };
    format!(
        "<div class=\"callout callout-{name}\" style=\"border-left:4px solid {rgb};\
         background:rgba({},{},{},0.10);padding:.5em .9em;border-radius:4px;margin:1em 0\">\n\
         <p class=\"callout-title\" style=\"color:{rgb};font-weight:600;\
         margin:0 0 .3em;font-size:.92em\">{title}</p>\n{inner}</div>\n",
        color[0], color[1], color[2]
    )
}

/// 收集高亮节点（`==x==`）：导出为 `<mark>`。内容源文本按 HTML 转义后再交
/// pulldown（保留 `**粗**` 等行内语法，同时屏蔽裸 `<script>` 等非白名单标签）。
fn collect_highlights(nodes: &[InlineNode], out: &mut Vec<InlineNode>) {
    for n in nodes {
        if n.kind == InlineKind::Highlight {
            out.push(n.clone());
        }
        collect_highlights(&n.children, out);
    }
}

/// 标题级别（ATX / Setext 共用）。
fn heading_level(kind: &BlockKind) -> Option<u8> {
    match kind {
        BlockKind::AtxHeading(level) | BlockKind::SetextHeading(level) => Some(*level),
        _ => None,
    }
}

/// 标题内容区间（块内字节偏移，标题块起点即首行行首）。
/// ATX 的剥除规则与 core 私有 `heading_content_span` 逐条对应（前导 ≤3 空格 +
/// `#` 序列 + 至多一个随行空白；闭合 `#` 序列须由空白引出）；Setext 取除末行
/// （下划线）外的全部内容行。与 `Session::heading` 用同一区间，保证「slug 输入
/// 文本」与「实际渲染文本」不会出现一侧含标记另一侧不含的错位。
fn heading_content<'a>(kind: &BlockKind, text: &'a str) -> (usize, &'a str) {
    match kind {
        BlockKind::AtxHeading(_) => {
            let line = text.split('\n').next().unwrap_or("");
            let (cs, ce) = atx_content_span(line);
            (cs, &line[cs..ce])
        }
        BlockKind::SetextHeading(_) => {
            let end = text.trim_end_matches('\n').rfind('\n').unwrap_or(0);
            (0, &text[..end])
        }
        _ => (0, text),
    }
}

/// ATX 内容区间（与 core 同名规则一致；行不含换行）。
fn atx_content_span(line: &str) -> (usize, usize) {
    let t = line.trim_start();
    let indent = line.len() - t.len();
    let hashes = t.bytes().take_while(|&b| b == b'#').count();
    let mut cs = (indent + hashes).min(line.len());
    if matches!(line.as_bytes().get(cs), Some(b' ') | Some(b'\t')) {
        cs += 1;
    }
    let mut ce = line.trim_end().len();
    let hashed_end = line[..ce].trim_end_matches('#').len();
    // 闭合序列须由空白引出（`# Title#` 的 `#` 属于内容）
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
    (cs, ce.max(cs))
}

/// 标题块的文本按行内语义渲染：把内容逐行包成 ATX 标题再解析，最后剥掉 `<hN>`
/// 包裹。标题内容本应只有行内语义，直接按段落交 pulldown 会把 `1. 正文段落`
/// 当成有序列表、`# x` 当成嵌套标题等块结构。
/// `defs` 为文档级链接定义前缀，须放在包裹行之前（定义行自身不能进标题内容）。
fn inline_html(defs: &str, content: &str, opts: pulldown_cmark::Options) -> String {
    let mut src = String::with_capacity(defs.len() + content.len() + 16);
    src.push_str(defs);
    for line in content.split('\n') {
        src.push_str("# ");
        src.push_str(line);
        src.push('\n');
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new_ext(&src, opts));
    let mut inner = html;
    for level in 1..=6 {
        inner = inner
            .replace(&format!("<h{level}>"), "")
            .replace(&format!("</h{level}>"), "");
    }
    inner.trim_end_matches('\n').to_string()
}

/// 收集块内全部脚注引用（递归含嵌套节点）：(完整标记区间, id 文本)。
fn collect_footnote_refs(session: &Session, nodes: &[InlineNode], out: &mut Vec<(Range, String)>) {
    for n in nodes {
        if n.kind == InlineKind::FootnoteRef {
            if let Some(id) = session.text_slice(n.content) {
                out.push((n.range, id.to_string()));
            }
        }
        collect_footnote_refs(session, &n.children, out);
    }
}

/// 收集块内裸链接（GFM 自动链接扩展）：`url` 与 `content` 同指源码本身即为裸链接；
/// 显式 `[t](u)` / `<...>` / 引用式的 content 指向文本、与 range 不同，不会误判。
fn collect_bare_links(nodes: &[InlineNode], out: &mut Vec<InlineNode>) {
    for n in nodes {
        if n.kind == InlineKind::Link && n.url == Some(n.range) && n.content == n.range {
            out.push(n.clone());
        }
        collect_bare_links(&n.children, out);
    }
}

/// Task 3-5：收集并翻译行内 HTML 节点为安全的 HTML 片段。
///
/// - 白名单内 Html → `<tag attrs...>` 原样保留（attrs 已由解析层剥 on*= 并净化 style）
/// - HtmlSelfClosed::Br → `<br>`
/// - HtmlSelfClosed::Img → `<img src="..." alt="...">`（远程 src 已置空则降级为
///   `<img alt="..."> <!-- veil blocked remote -->`）
/// - HtmlLiteral / 非白名单 tag → escape 为 `&lt;…>`
/// - 危险 tag（script/style/iframe 等）→ 整块剥除（替换为空）
///
/// 返回 (块内起点, 块内终点, 替换 HTML 片段)，与脚注引用/裸链接共用 reps 列表。
fn collect_html_inlines(
    session: &Session,
    nodes: &[InlineNode],
    base: usize,
    text: &str,
    out: &mut Vec<(usize, usize, String)>,
) {
    // session 参数仅用于递归传递，clippy "parameter is only used in recursion" 允许继续
    #[allow(clippy::used_underscore_binding)]
    let _ = session;
    for n in nodes {
        let Some(s) = n.range.start.checked_sub(base) else {
            continue;
        };
        let Some(e) = n.range.end.checked_sub(base) else {
            continue;
        };
        if s >= e || e > text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
            collect_html_inlines(session, &n.children, base, text, out);
            continue;
        }
        match &n.kind {
            InlineKind::Html { tag, attrs } => {
                let tag_name = tag.to_str();
                // script/style/iframe 等强制剥除：整段替换为空
                if veil_core::should_strip_html(tag_name) {
                    out.push((s, e, String::new()));
                    collect_html_inlines(session, &n.children, base, text, out);
                    continue;
                }
                // 白名单内：产出 <tag attrs...>（attrs 按解析层给定，已过滤）
                let attrs_str = format_html_attrs(attrs);
                let open_tag = if attrs_str.is_empty() {
                    format!("<{tag_name}>")
                } else {
                    format!("<{tag_name} {attrs_str}>")
                };
                let close_tag = format!("</{tag_name}>");
                // 渲染子节点（递归处理子节点中的 Html 标签和其他 inline）
                let children_rendered = {
                    let mut sub_reps = Vec::new();
                    collect_html_inlines(session, &n.children, base, text, &mut sub_reps);
                    let sub_start = n.content.start.saturating_sub(base);
                    let sub_end = n.content.end.saturating_sub(base);
                    render_html_children(text, sub_start, sub_end, &sub_reps)
                };
                out.push((s, e, format!("{open_tag}{children_rendered}{close_tag}")));
            }
            InlineKind::HtmlSelfClosed { tag, attrs } => {
                let (Some(s), Some(e)) = (
                    n.range.start.checked_sub(base),
                    n.range.end.checked_sub(base),
                ) else {
                    continue;
                };
                if s >= e
                    || e > text.len()
                    || !text.is_char_boundary(s)
                    || !text.is_char_boundary(e)
                {
                    continue;
                }
                let rendered = match tag {
                    HtmlTag::Br => "<br>".to_string(),
                    HtmlTag::Img => {
                        // 远程 src 被 core 解析到 remote_src（v1.4）：HTML 导出须还原 URL，
                        // 让浏览器按需加载（与 Markdown 远程图片同口径、Typora/Obsidian 同行为）。
                        // src 与 remote_src 可能同时为空（非法/缺失），此时降级为 alt-only。
                        let src = attrs
                            .src
                            .as_deref()
                            .or(attrs.remote_src.as_deref())
                            .unwrap_or("");
                        let alt = attrs.alt.as_deref().unwrap_or("");
                        if src.is_empty() {
                            // src 与 remote_src 皆空：降级为带注释的 alt-only（HTML 解析层剥离或本地缺失）
                            format!(
                                "<img alt=\"{}\"> <!-- veil blocked remote -->",
                                escape_attr(alt)
                            )
                        } else {
                            let mut img = format!("<img src=\"{}\"", escape_attr(src));
                            if !alt.is_empty() {
                                img.push_str(&format!(" alt=\"{}\"", escape_attr(alt)));
                            }
                            img.push('>');
                            img
                        }
                    }
                    _ => String::new(), // 其他自闭合白名单（input/hr）暂替换为空
                };
                out.push((s, e, rendered));
            }
            InlineKind::HtmlLiteral => {
                let escaped = escape_html(&text[s..e]);
                out.push((s, e, escaped));
            }
            _ => {}
        }
        collect_html_inlines(session, &n.children, base, text, out);
    }
}

/// 把 attrs 结构体格式化为 `key="value"` 对（解析层已过滤危险键）。
fn format_html_attrs(attrs: &veil_core::HtmlAttrs) -> String {
    let mut parts = Vec::new();
    // `<a href>`：目标仅来自解析层白名单（javascript: 已在解析层置空）
    if let Some(href) = attrs.href.as_deref() {
        parts.push(format!("href=\"{}\"", escape_attr(href)));
    }
    if let Some(title) = attrs.title.as_deref() {
        parts.push(format!("title=\"{}\"", escape_attr(title)));
    }
    if let Some(src) = attrs.src.as_deref() {
        parts.push(format!("src=\"{}\"", escape_attr(src)));
    }
    if let Some(alt) = attrs.alt.as_deref() {
        parts.push(format!("alt=\"{}\"", escape_attr(alt)));
    }
    // style 已由解析层净化（仅白名单属性与合法值），导出原样保留。
    if let Some(style) = attrs.style.as_deref() {
        parts.push(format!("style=\"{}\"", escape_attr(style)));
    }
    parts.join(" ")
}

/// 在父节点的子区间内，按子节点的替换列表渲染最终 HTML。
/// 子节点的 replace_range 是基于 block 起点的偏移，需要与 parent 的 content 区间对齐。
fn render_html_children(
    text: &str,
    start: usize,
    end: usize,
    sub_reps: &[(usize, usize, String)],
) -> String {
    let mut out = String::new();
    let mut p = start;
    for (s, e, rep) in sub_reps {
        if *s < p || *e > end {
            continue;
        }
        out.push_str(&text[p..*s]);
        out.push_str(rep);
        p = *e;
    }
    out.push_str(&text[p..end]);
    out
}

/// 把块源码里的脚注引用与裸链接替换为 raw HTML。
///
/// 脚注只按 core 给出的 FootnoteRef 节点区间改动；裸链接只按「url == range」的
/// Link 节点改动，代码跨度/围栏/图片语法不产出这类节点，天然不会误替换。
/// 两种替换同处一遍扫描（按起点排序，后到的重叠项丢弃），保证互不踩踏。
/// 同一脚注 id 重复引用时首个保持 `fnref-n`（文末返回链接指向它），后续加 `-{k}`
/// 后缀避免 id 重复——与 GitHub 等主流导出的锚点行为一致。
fn render_block_inlines(
    session: &Session,
    occurrences: &mut HashMap<String, usize>,
    text: &str,
    base: usize,
    nodes: &[InlineNode],
) -> String {
    // (块内起点, 块内终点, 替换文本)
    let mut reps: Vec<(usize, usize, String)> = Vec::new();
    let mut refs = Vec::new();
    collect_footnote_refs(session, nodes, &mut refs);
    refs.sort_by_key(|(range, _)| range.start);
    for (range, id) in refs {
        // 「已定义」才替换：core 会给仅被引用的 id 编号，但无定义时导出无法产出
        // 条目，保留原始标记比生成断链更诚实。
        let (Some(n), true) = (
            session.footnote_number(&id),
            session.footnote_definition(&id).is_some(),
        ) else {
            continue;
        };
        let (Some(s), Some(e)) = (range.start.checked_sub(base), range.end.checked_sub(base))
        else {
            continue;
        };
        if s >= e || e > text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
            continue;
        }
        let k = occurrences.entry(id).or_insert(0);
        *k += 1;
        let anchor = if *k == 1 {
            format!("fnref-{n}")
        } else {
            format!("fnref-{n}-{k}")
        };
        reps.push((
            s,
            e,
            format!("<sup class=\"fnref\"><a href=\"#fn-{n}\" id=\"{anchor}\">[{n}]</a></sup>"),
        ));
    }
    let mut links = Vec::new();
    collect_bare_links(nodes, &mut links);
    links.sort_by_key(|n| n.range.start);
    for node in links {
        let Some(url) = node.url.and_then(|u| session.text_slice(u)) else {
            continue;
        };
        let (Some(s), Some(e)) = (
            node.range.start.checked_sub(base),
            node.range.end.checked_sub(base),
        ) else {
            continue;
        };
        if s >= e || e > text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
            continue;
        }
        let href = veil_ui::normalize_link_url(&url);
        reps.push((
            s,
            e,
            format!(
                "<a href=\"{}\">{}</a>",
                escape_attr(&href),
                escape_link_text(&url)
            ),
        ));
    }
    // Task 3-5：翻译行内 HTML 节点为安全的 HTML 片段（白名单保留 / 危险标签剥除 /
    // 远程 src 降级 / 非白名单 escape）。
    let mut html_reps = Vec::new();
    collect_html_inlines(session, nodes, base, text, &mut html_reps);
    reps.extend(html_reps);
    // N3c：`==x==` → `<mark>x</mark>`（与预览同一底色；源码标记被整段替换）。
    let mut highlights = Vec::new();
    collect_highlights(nodes, &mut highlights);
    for node in highlights {
        let (Some(s), Some(e)) = (
            node.range.start.checked_sub(base),
            node.range.end.checked_sub(base),
        ) else {
            continue;
        };
        let (Some(cs), Some(ce)) = (
            node.content.start.checked_sub(base),
            node.content.end.checked_sub(base),
        ) else {
            continue;
        };
        if s >= e || e > text.len() || cs > ce || ce > text.len() {
            continue;
        }
        if !text.is_char_boundary(s)
            || !text.is_char_boundary(e)
            || !text.is_char_boundary(cs)
            || !text.is_char_boundary(ce)
        {
            continue;
        }
        reps.push((s, e, format!("<mark>{}</mark>", escape_html(&text[cs..ce]))));
    }
    reps.sort_by_key(|(s, _, _)| *s);
    let mut out = String::with_capacity(text.len() + 32);
    let mut p = 0usize;
    for (s, e, rep) in reps {
        if s < p {
            continue; // 区间重叠（理论上不出现）：保留先出现的替换
        }
        out.push_str(&text[p..s]);
        out.push_str(&rep);
        p = e;
    }
    out.push_str(&text[p..]);
    out
}

/// href 属性值转义（`&` 与引号必须转义，裸 URL 常见 `?a=1&b=2`）。
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 裸链接可见文本：HTML 转义后再把 markdown 行内标记字符改写为数字实体，
/// 使交回 pulldown 的源码保持纯文本（URL 里的 `_`/`*`/`[` 不触发强调/链接）。
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

/// 收集脚注定义正文：id → 剥除 `[^id]:` 前缀后的文本（含续行）。
/// 重复定义 first-wins；定义块在正文循环中被跳过，统一由文末脚注区输出。
fn collect_footnote_defs(
    session: &Session,
    blocks: &[(veil_core::BlockId, BlockKind, Range)],
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (_, kind, range) in blocks {
        if *kind != BlockKind::FootnoteDef {
            continue;
        }
        let Some(text) = session.text_slice(*range) else {
            continue;
        };
        if let Some((id, body)) = split_footnote_def(&text) {
            out.entry(id).or_insert(body);
        }
    }
    out
}

/// 脚注定义块源码 → (id, 正文)。首行剥 `[^id]:` 与紧随的一个空格；
/// 续行剥掉块要求的 4 列缩进，使其继续同一段落（否则会被 pulldown 当缩进代码）。
fn split_footnote_def(text: &str) -> Option<(String, String)> {
    let mut lines = text.split_inclusive('\n');
    let first = lines.next()?;
    let rest = first.trim_start().strip_prefix("[^")?;
    let (id, tail) = rest.split_once("]:")?;
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let mut body = String::new();
    body.push_str(
        tail.strip_prefix(' ')
            .unwrap_or(tail)
            .trim_end_matches(['\r', '\n']),
    );
    for line in lines {
        body.push('\n');
        body.push_str(strip_continuation_indent(line));
    }
    Some((id.to_string(), body))
}

/// 剥除续行至多 4 个前导空格（脚注续行的最小缩进），保留其余内容。
fn strip_continuation_indent(line: &str) -> &str {
    let n = line.bytes().take(4).take_while(|&b| b == b' ').count();
    &line[n..]
}

/// 文末脚注区：仅输出「被引用且已定义」的脚注，按编号升序。
fn footnotes_html(
    session: &Session,
    defs: &HashMap<String, String>,
    opts: pulldown_cmark::Options,
) -> String {
    let mut items = String::new();
    for (idx, id) in session.footnote_ids().iter().enumerate() {
        let Some(body) = defs.get(id) else {
            continue; // 引用无定义：引用处已保留原文，此处不产出断链条目
        };
        let n = session.footnote_number(id).unwrap_or(idx + 1);
        let mut inner = String::new();
        pulldown_cmark::html::push_html(&mut inner, pulldown_cmark::Parser::new_ext(body, opts));
        let inner = inner.trim_end_matches('\n');
        // 返回链接固定指向首次引用（fnref-n）；重复引用的 -2/-3 后缀不参与回跳。
        let back = format!("<a class=\"fnback\" href=\"#fnref-{n}\">↩</a>");
        let inner = match inner.rfind("</p>") {
            Some(pos) => format!("{} {back}{}", &inner[..pos], &inner[pos..]),
            None if inner.is_empty() => format!("<p>{back}</p>"),
            None => format!("<p>{inner} {back}</p>"),
        };
        items.push_str(&format!("<li id=\"fn-{n}\">{inner}</li>\n"));
    }
    if items.is_empty() {
        return String::new();
    }
    format!("<section class=\"footnotes\">\n<hr>\n<ol>\n{items}</ol>\n</section>\n")
}

/// 单个数学块 → 内嵌 SVG；渲染失败降级为转义源码（导出绝不中断）。
fn math_html(tex: &str, font: &latex_rust::MathFont) -> String {
    let mut opt = latex_rust::SvgOptions::new();
    opt.display = true; // 块级 $$ 恒为 display（与编辑器渲染一致）
    match latex_rust::latex_to_svg(tex, font, &opt) {
        Ok(svg) => {
            // 内嵌进 HTML 须剥 XML 声明（<?xml ?> 仅允许在独立文档开头），并去掉首尾空白
            let inline = svg.split_once("?>").map(|(_, rest)| rest).unwrap_or(&svg);
            format!("<div class=\"math\">{}</div>\n", inline.trim())
        }
        Err(_) => format!("<pre class=\"math-source\">{}</pre>\n", escape_html(tex)),
    }
}

/// 降级路径的 HTML 转义（SVG 成功路径不需要：latex-rust 产物为受控标记）。
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 产品层导出器：HTML 写到源文件同名的 .html（经 FileStore 原子写）。
pub struct HtmlExporter {
    pub font: latex_rust::MathFont,
    pub store: Box<dyn FileStore>,
}

impl Exporter for HtmlExporter {
    fn export_html(
        &mut self,
        session: &Session,
        style: ExportStyle,
        source_path: Option<&Path>,
        target: Option<&Path>,
    ) -> Result<PathBuf, String> {
        let out = match target {
            Some(t) => t.to_path_buf(),
            None => source_path
                .ok_or_else(|| {
                    veil_ui::i18n::t(veil_ui::i18n::Key::ErrExportDraftNoPath).to_string()
                })?
                .with_extension("html"),
        };
        self.store
            .save(&out, html_of(session, &style, &self.font).as_bytes())?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    fn font() -> latex_rust::MathFont {
        latex_rust::MathFont::stix_two_math().unwrap()
    }

    /// 从完整文档中切出 body 段（模板外壳另有钉死测试）。
    fn body_of(md: &str) -> String {
        let s = Session::open(md);
        let html = html_of(&s, &ExportStyle::default(), &font());
        let body = html.split_once("<body>\n").unwrap().1;
        body.strip_suffix("</body>\n</html>\n").unwrap().to_string()
    }

    /// B3：details 子块导出时仍包在 <details open> 内，嵌套结构闭合正确。
    #[test]
    fn details_children_are_wrapped_in_details() {
        let body = body_of(
            "<details>\n<summary>S</summary>\n## H\n\npara\n\n<details>\n<summary>I</summary>\ninner\n</details>\n</details>\n",
        );
        let open = body.matches("<details open>").count();
        let close = body.matches("</details>").count();
        assert_eq!(open, 2, "嵌套 details 两个开标签：{body}");
        assert_eq!(close, 2, "闭合标签数量与开标签一致：{body}");
        assert!(body.contains("<h2 id=\"h\">H</h2>"), "标题子块：{body}");
        assert!(body.contains("<p>para</p>"), "段落子块：{body}");
        let outer_end = body.rfind("</details>").unwrap();
        assert!(
            body.find("<p>inner</p>").unwrap() < outer_end,
            "嵌套正文应在外层闭合前输出：{body}"
        );
    }

    #[test]
    fn heading_and_paragraph_inlines() {
        // 逐字节期望按 pulldown-cmark 0.13 默认输出手工核算：ATX → <h1>…</h1>\n；
        // 段落 → <p>…</p>\n；** → <strong>；` → <code>
        // 标题改为手写输出并带 slug 锚点（id 由标题文本去重得到，见 heading_slugs）
        assert_eq!(
            body_of("# Hi\n\nplain **b** and `c`\n"),
            "<h1 id=\"hi\">Hi</h1>\n<p>plain <strong>b</strong> and <code>c</code></p>\n"
        );
    }

    #[test]
    fn fenced_code_escapes_html() {
        // 代码内容 HTML 转义：< → &lt;；语言信息串 v0 不存在（veil-core 不解析 info）
        assert_eq!(
            body_of("```\nif a < b {}\n```\n"),
            "<pre><code>if a &lt; b {}\n</code></pre>\n"
        );
    }

    #[test]
    fn strikethrough_exports_as_del() {
        // GFM 扩展显式开启：~~ → <del>
        assert_eq!(
            body_of("done ~~gone~~ keep\n"),
            "<p>done <del>gone</del> keep</p>\n"
        );
    }

    // ===== N3c：`==高亮==` 三通道 =====

    #[test]
    fn highlight_exports_as_mark() {
        assert_eq!(body_of("a ==hi== b\n"), "<p>a <mark>hi</mark> b</p>\n");
        // 行内语法仍生效：粗体/代码在 mark 内继续解析
        assert_eq!(
            body_of("==**粗** `c`==\n"),
            "<p><mark><strong>粗</strong> <code>c</code></mark></p>\n"
        );
    }

    #[test]
    fn highlight_in_heading_and_table() {
        assert_eq!(body_of("# ==T==\n"), "<h1 id=\"t\"><mark>T</mark></h1>\n");
        let table = body_of("| a |\n| --- |\n| ==x== |\n");
        assert!(
            table.contains("<td><mark>x</mark></td>"),
            "实际输出: {table}"
        );
    }

    #[test]
    fn highlight_escapes_non_whitelisted_html() {
        let body = body_of("==<script>alert(1)</script>==\n");
        assert!(
            body.contains("<mark>&lt;script&gt;alert(1)&lt;/script&gt;</mark>"),
            "实际输出: {body}"
        );
        assert!(!body.contains("<script>"), "禁止裸脚本进导出: {body}");
    }

    #[test]
    fn highlight_roundtrip_is_byte_identical() {
        let src = "# ==标题==\n\n> ==引用==\n\n- ==项==\n\n| a |\n| --- |\n| ==格== |\n";
        let s = Session::open(src);
        assert_eq!(s.serialize(), src, "往返字节差必须为 0");
    }

    // ===== N3b：Callout 导出 =====

    #[test]
    fn gfm_alert_exports_typed_callout_div() {
        let body = body_of("> [!NOTE]\n> 注意事项\n");
        assert!(
            body.starts_with("<div class=\"callout callout-note\" style=\""),
            "实际输出: {body}"
        );
        assert!(body.contains("border-left:4px solid #0969da"), "{body}");
        assert!(body.contains("background:rgba(9,105,218,0.10)"), "{body}");
        assert!(
            body.contains("<p class=\"callout-title\" style=\"color:#0969da;"),
            "{body}"
        );
        assert!(body.contains("i Note"), "图标 + 标签: {body}");
        assert!(body.contains("注意事项"), "{body}");
        assert!(!body.contains("[!NOTE]"), "不得露出标记: {body}");
        assert!(
            !body.contains("<blockquote>"),
            "callout 不输出 blockquote: {body}"
        );
    }

    #[test]
    fn all_five_alert_types_have_own_class_and_color() {
        for (ty, class, color) in [
            ("NOTE", "note", "#0969da"),
            ("TIP", "tip", "#1a7f37"),
            ("IMPORTANT", "important", "#8250df"),
            ("WARNING", "warning", "#9a6700"),
            ("CAUTION", "caution", "#cf222e"),
        ] {
            let body = body_of(&format!("> [!{ty}]\n> x\n"));
            assert!(body.contains(&format!("callout-{class}\"")), "{ty}: {body}");
            assert!(
                body.contains(&format!("border-left:4px solid {color}")),
                "{ty}: {body}"
            );
        }
    }

    #[test]
    fn custom_callout_uses_config_label_color_and_class() {
        let reg = veil_core::CalloutRegistry::from_json(
            r##"[{"name":"FIXME","label":"待修复","color":"#c00","icon":"!"}]"##,
        );
        let s = Session::open_with_callouts("> [!FIXME]\n> 内容\n", &reg);
        let html = html_of(&s, &ExportStyle::default(), &font());
        let body = html.split_once("<body>\n").unwrap().1;
        assert!(
            body.contains("<div class=\"callout callout-fixme\""),
            "{body}"
        );
        assert!(body.contains("border-left:4px solid #cc0000"), "{body}");
        assert!(body.contains("! 待修复"), "{body}");
        assert!(body.contains("内容"), "{body}");
        assert!(!body.contains("[!FIXME]"), "{body}");
    }

    #[test]
    fn unknown_callout_falls_back_to_blockquote_without_marker() {
        let body = body_of("> [!BOGUS]\n> body\n");
        assert!(body.starts_with("<blockquote>"), "回退普通引用: {body}");
        assert!(body.contains("body"), "{body}");
        assert!(!body.contains("[!BOGUS]"), "不得显示裸标记: {body}");
        assert!(!body.contains("callout"), "{body}");
    }

    #[test]
    fn callout_roundtrip_is_byte_identical() {
        let src = "> [!NOTE]\n> n\n\n> [!WARNING]\n> w\n";
        let s = Session::open(src);
        assert_eq!(s.serialize(), src);
    }

    #[test]
    fn bare_links_export_as_anchors() {
        // GFM 自动链接扩展：`www.` 补 http、任意 scheme 原样、裸邮箱补 mailto；
        // URL 中的 `&` 在 href 与可见文本里都必须转义
        assert_eq!(
            body_of("Visit www.a.com/x, or ftp://a.b/c\n"),
            "<p>Visit <a href=\"http://www.a.com/x\">www.a.com/x</a>, or \
             <a href=\"ftp://a.b/c\">ftp://a.b/c</a></p>\n"
        );
        assert_eq!(
            body_of("mail foo@bar.baz\n"),
            "<p>mail <a href=\"mailto:foo@bar.baz\">foo@bar.baz</a></p>\n"
        );
        assert_eq!(
            body_of("www.a.com/?a=1&b=2\n"),
            "<p><a href=\"http://www.a.com/?a=1&amp;b=2\">www.a.com/?a=1&amp;b=2</a></p>\n"
        );
        // 代码段与显式链接不受影响（后者链接文本照旧、目标不被改写）
        assert_eq!(
            body_of("`www.a.com` and [t](u)\n"),
            "<p><code>www.a.com</code> and <a href=\"u\">t</a></p>\n"
        );
    }

    #[test]
    fn bare_link_in_table_cell_exported() {
        // 表格单元格的行内节点不在 block_inlines 上，导出侧需单独收集
        assert_eq!(
            body_of("| a |\n| - |\n| www.a.com |\n"),
            "<table><thead><tr><th>a</th></tr></thead><tbody>\n\
             <tr><td><a href=\"http://www.a.com\">www.a.com</a></td></tr>\n\
             </tbody></table>\n"
        );
    }

    #[test]
    fn task_list_exports_as_checkbox_input() {
        // GFM 任务列表：`[ ]` / `[x]` → checkbox input（pulldown 原生输出带换行）
        assert_eq!(
            body_of("- [ ] todo\n- [x] done\n"),
            "<ul>\n<li><input disabled=\"\" type=\"checkbox\"/>\ntodo</li>\n<li><input disabled=\"\" type=\"checkbox\" checked=\"\"/>\ndone</li>\n</ul>\n"
        );
    }

    #[test]
    fn math_block_becomes_svg_in_div() {
        // 校准依据（2026-10-25 探针实测）：latex_to_svg 产物含 <?xml ?> 声明且尾带 \n，
        // math_html 剥声明+trim 后单行包装；SVG 内容由 latex-rust 自身测试钉死，此处只钉结构
        let body = body_of("$$\n\\frac{1}{2}\n$$\n");
        assert!(
            body.starts_with("<div class=\"math\"><svg"),
            "实际输出: {body}"
        );
        assert!(body.ends_with("</svg></div>\n"), "实际输出: {body}");
        assert!(!body.contains("math-source"));
    }

    #[test]
    fn gfm_table_renders_as_html_table() {
        // pulldown-cmark 0.13.5 表格输出逐字节核算：对齐写 style，thead/tbody 分行位置固定
        assert_eq!(
            body_of("| a | b |\n| :-- | --: |\n| 1 | 2 |\n"),
            "<table><thead><tr><th style=\"text-align: left\">a</th><th style=\"text-align: right\">b</th></tr></thead><tbody>\n<tr><td style=\"text-align: left\">1</td><td style=\"text-align: right\">2</td></tr>\n</tbody></table>\n"
        );
    }

    #[test]
    fn quoted_gfm_table_nests_inside_blockquote() {
        let body = body_of("> | a | b |\n> | :-- | --: |\n> | 1 | 2 |\n");
        assert!(body.starts_with("<blockquote>"), "实际输出: {body}");
        let open = body.find("<table>").expect("引用内应输出表格");
        let close = body.find("</blockquote>").expect("应有引用闭合");
        assert!(open < close, "<table> 必须在 <blockquote> 内：{body}");
        assert!(body.contains("text-align: left") && body.contains("text-align: right"));
        assert!(!body.contains("| ---"), "分隔行不得出现在导出中：{body}");
    }

    #[test]
    fn list_item_table_exports_as_table() {
        let body = body_of("- | a | b |\n  | --- | --- |\n  | 1 | 2 |\n");
        assert!(
            body.contains("<table>"),
            "列表项内表格应输出 <table>：{body}"
        );
        assert!(body.contains("<li>"), "应保留列表项包裹：{body}");
    }

    #[test]
    fn details_child_table_stays_inside_details() {
        let body = body_of("<details>\n<summary>S</summary>\n| a |\n| --- |\n| 1 |\n</details>\n");
        assert!(
            body.contains("<details open><summary>S</summary>"),
            "{body}"
        );
        let open = body.find("<table>").expect("details 内应输出表格");
        let close = body
            .rfind("</details>")
            .expect("B2：子表格后应补 details 闭合");
        assert!(open < close, "<table> 必须在 <details> 内：{body}");
    }

    #[test]
    fn adjacent_list_items_merge_into_single_list() {
        // 逐项独立解析会碎裂为多个 <ul>；合并连续列表项后为单一列表（切片 12）
        assert_eq!(
            body_of("- a\n- b\n"),
            "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n"
        );
        // 空行分隔仍属同一（松散）列表：不得拆成两个 <ul>
        assert_eq!(
            body_of("- a\n\n- b\n"),
            "<ul>\n<li>\n<p>a</p>\n</li>\n<li>\n<p>b</p>\n</li>\n</ul>\n"
        );
        // 有序列表编号连续呈现（拆分将重置为 1）
        assert_eq!(
            body_of("1. a\n2. b\n"),
            "<ol>\n<li>a</li>\n<li>b</li>\n</ol>\n"
        );
    }

    #[test]
    fn image_renders_as_img_tag() {
        // 切片 13：`![alt](path)` 由 pulldown-cmark 输出 <img>；相对路径原样保留
        // （导出 HTML 与源文档同目录，相对引用继续有效）
        assert_eq!(
            body_of("![示例](img/a.png)\n"),
            "<p><img src=\"img/a.png\" alt=\"示例\" /></p>\n"
        );
    }

    #[test]
    fn blockquote_and_thematic_break_export() {
        assert_eq!(
            body_of("> a\n> b\n"),
            "<blockquote>\n<p>a\nb</p>\n</blockquote>\n"
        );
        assert_eq!(body_of("---\n"), "<hr />\n");
    }

    #[test]
    fn math_failure_degrades_to_escaped_source() {
        // 非法命令触发渲染失败 → 转义源码兜底（< > 必须转义）
        let body = body_of("$$\na < b \\notacmd\n$$\n");
        assert_eq!(
            body,
            "<pre class=\"math-source\">a &lt; b \\notacmd</pre>\n"
        );
    }

    #[test]
    fn document_shell_wraps_body() {
        let s = Session::open("x\n");
        let html = html_of(&s, &ExportStyle::default(), &font());
        assert!(
            html.starts_with("<!DOCTYPE html>\n<html"),
            "实际输出: {html}"
        );
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("<body>\n<p>x</p>\n</body>"));
        assert!(html.ends_with("</html>\n"));
    }

    /// 深色样式：取值与 `ExportStyle::from_theme(&Theme::dark())` 的映射一致
    /// （映射本身由 veil-ui 的 theme 测试钉死，此处只消费公开快照）。
    fn dark_style() -> ExportStyle {
        ExportStyle {
            dark: true,
            bg: [30, 30, 30],
            text: [220, 220, 220],
            muted: [160, 160, 160],
            link: [127, 180, 245],
            code_bg: [40, 40, 40],
            border: [60, 64, 70],
        }
    }

    #[test]
    fn shell_colors_follow_style_and_dark_declares_scheme() {
        let s = Session::open("x\n");
        let light = html_of(&s, &ExportStyle::default(), &font());
        // 默认浅色：与改造前硬编码色一致（视觉不回退）
        assert!(light.contains("background:#ffffff"), "实际输出: {light}");
        assert!(light.contains("color:#1e1e1e"), "实际输出: {light}");
        assert!(light.contains("background:#f5f5f5"), "实际输出: {light}");
        assert!(light.contains("a{color:#2b6cb0}"), "实际输出: {light}");
        assert!(
            !light.contains("color-scheme: dark"),
            "浅色不得声明深色 color-scheme"
        );
        let dark = html_of(&s, &dark_style(), &font());
        assert!(dark.contains("background:#1e1e1e"), "实际输出: {dark}");
        assert!(dark.contains("color:#dcdcdc"), "实际输出: {dark}");
        assert!(dark.contains("a{color:#7fb4f5}"), "实际输出: {dark}");
        assert!(dark.contains("background:#282828"), "实际输出: {dark}");
        assert!(
            dark.contains("border:1px solid #3c4046"),
            "实际输出: {dark}"
        );
        assert!(dark.contains("color-scheme: dark"), "实际输出: {dark}");
    }

    #[test]
    fn export_head_styles_inline_html_whitelist() {
        // 行内白名单 HTML 的预览样式（kbd/sub/sup/mark/abbr/details/img）必须在导出件
        // 里逐条落地，否则「导出件与预览不一致」（AC-3 三通道一致性）。
        let style = ExportStyle::default();
        let css = head(&style);
        // kbd：描边圆角矩形 + 浅灰底（code_bg）+ 下沉阴影（border），与预览层一致
        assert!(
            css.contains(
                "kbd{border:1px solid #dddddd;border-radius:3px;padding:.05em .35em;\
                 font-size:.9em;font-weight:600;background:#f5f5f5;\
                 box-shadow:0 1px 0 #dddddd}"
            ),
            "实际输出: {css}"
        );
        // sub/sup：0.75 倍字号 + 基线偏移
        assert!(
            css.contains("sub,sup{font-size:.75em;line-height:0}"),
            "实际输出: {css}"
        );
        assert!(
            css.contains("sub{vertical-align:-0.25em}"),
            "实际输出: {css}"
        );
        assert!(css.contains("sup{vertical-align:0.5em}"), "实际输出: {css}");
        // mark：预览层同一高亮色 rgb(255,247,153)，文字保持正文色
        assert!(
            css.contains("mark{background:#fff799;color:inherit;padding:0 .1em;border-radius:2px}"),
            "实际输出: {css}"
        );
        // abbr：虚线下划线
        assert!(
            css.contains("abbr[title]{text-decoration:underline dotted;cursor:help}"),
            "实际输出: {css}"
        );
        // details/summary：折叠块 + 左侧竖线（border 色随主题）
        assert!(
            css.contains("details{margin:.8em 0;padding:.2em .6em;border-left:3px solid #dddddd}"),
            "实际输出: {css}"
        );
        assert!(
            css.contains("summary{cursor:pointer;font-weight:600}"),
            "实际输出: {css}"
        );
        assert!(
            css.contains("details[open] summary{margin-bottom:.4em}"),
            "实际输出: {css}"
        );
        // img：不溢出正文栏宽；行内限高 2 行（3.2em），独占一行放宽到 6 行（9.6em），
        // 表格单元格 1.5 行（2.4em）——与编辑视图 B4 策略一致（body 1.6 行距基准）。
        assert!(
            css.contains("img{max-width:100%;height:auto;max-height:3.2em}"),
            "实际输出: {css}"
        );
        assert!(
            css.contains("p>img:only-child{max-height:9.6em}"),
            "实际输出: {css}"
        );
        assert!(
            css.contains("td img,th img{max-height:2.4em}"),
            "实际输出: {css}"
        );
        // 颜色经 format! 插值：深色主题下变量必须换成主题色而非写死浅色
        let dark = head(&dark_style());
        assert!(
            dark.contains("details{margin:.8em 0;padding:.2em .6em;border-left:3px solid #3c4046}"),
            "实际输出: {dark}"
        );
        assert!(
            dark.contains("background:#282828;box-shadow:0 1px 0 #3c4046}"),
            "实际输出: {dark}"
        );
    }

    #[test]
    fn export_has_no_remote_resources() {
        // 单文件自包含是离线编辑器的硬约束：不得引入任何 http(s) 资源或 @import
        for style in [ExportStyle::default(), dark_style()] {
            let html = html_of(&Session::open("# T\n\n[a](b.md)\n"), &style, &font());
            assert!(!html.contains("src=\"http"), "不得引用远程图片/脚本");
            assert!(!html.contains("href=\"http"), "不得引用远程样式/链接");
            assert!(!html.contains("@import"), "不得 @import 远程样式表");
            assert!(!html.contains("http"), "实际输出: {html}");
        }
    }

    // ===== 标题锚点 =====

    #[test]
    fn heading_slugs_are_document_scoped_and_deduped() {
        // 英文小写、中文原样、重名追加 -1、标点丢弃；闭合 `#` 序列不进入内容
        assert_eq!(
            body_of("# Hello World\n\n## 安装 ##\n\n# Hello World\n\n### a.b:c;d\n"),
            "<h1 id=\"hello-world\">Hello World</h1>\n<h2 id=\"安装\">安装</h2>\n<h1 id=\"hello-world-1\">Hello World</h1>\n<h3 id=\"abcd\">a.b:c;d</h3>\n"
        );
    }

    #[test]
    fn heading_inlines_render_inside_anchor() {
        // slug 取自可见文本（Bold c → bold-c），行内格式仍由 pulldown 渲染
        assert_eq!(
            body_of("# **Bold** `c`\n"),
            "<h1 id=\"bold-c\"><strong>Bold</strong> <code>c</code></h1>\n"
        );
    }

    #[test]
    fn setext_heading_gets_slug() {
        // 内容行与下划线行同属一块：渲染只取内容行，id 与 ATX 同一 slug 规则
        assert_eq!(body_of("标题\n===\n"), "<h1 id=\"标题\">标题</h1>\n");
    }

    #[test]
    fn ordered_list_marker_in_heading_stays_literal_text() {
        // 标题内容只有行内语义：`1. 正文段落` 不得被 pulldown 当成有序列表
        assert_eq!(
            body_of("## 1. 正文段落\n"),
            "<h2 id=\"1-正文段落\">1. 正文段落</h2>\n"
        );
        // 多行 Setext（中置 FrontMatter 场景）同样逐行按行内渲染，不产生列表
        let body = body_of("标题: 中文标题\n作者: 张三\n---\n");
        assert!(
            body.contains("中文标题") && body.contains("张三"),
            "实际输出: {body}"
        );
        assert!(!body.contains("<ol>"), "实际输出: {body}");
    }

    #[test]
    fn anchor_href_is_preserved_and_javascript_rejected() {
        // 白名单 `<a>` 的 href 必须保留（预览/导出同源）
        assert_eq!(
            body_of("<a href=\"https://example.com\">链接</a>\n"),
            "<p><a href=\"https://example.com\">链接</a></p>\n"
        );
        // javascript: 仍被解析层置空：标签保留、目标不输出
        let body = body_of("<a href=\"javascript:alert(1)\">x</a>\n");
        assert_eq!(body, "<p><a>x</a></p>\n");
        assert!(!body.contains("javascript"), "实际输出: {body}");
    }

    #[test]
    fn crossed_inline_tags_export_as_escaped_literal() {
        // 交叉标签（<kbd><mark>…</kbd></mark>）整段按字面显示，不得漏出孤立闭标签
        let body = body_of("交叉标签：<kbd><mark>交叉</kbd></mark> 应原样显示。\n");
        assert!(
            body.contains("&lt;kbd&gt;&lt;mark&gt;交叉&lt;/kbd&gt;&lt;/mark&gt;"),
            "实际输出: {body}"
        );
        assert!(!body.contains("<kbd>"), "实际输出: {body}");
        assert!(!body.contains("</mark>"), "实际输出: {body}");
    }

    #[test]
    fn inline_semantic_tags_keep_sanitized_style() {
        // 用户真机缺陷：<strong style="color:red">VISUAL</strong> 必须原样保留语义标签，
        // 且 style 只保留白名单声明。
        let body = body_of("<strong style=\" color: red\">VISUAL</strong>\n");
        assert!(
            body.contains("<strong style=\"color:red\">VISUAL</strong>"),
            "实际输出: {body}"
        );
        assert!(
            !body.contains("position") && !body.contains("url("),
            "实际输出: {body}"
        );
        // 注入声明逐条丢弃，合法声明保留；不得出现 javascript:/position/url(
        let body = body_of(
            "<span style=\"color:red;position:fixed;background-image:url(javascript:x)\">x</span>\n",
        );
        assert!(
            body.contains("<span style=\"color:red\">x</span>"),
            "实际输出: {body}"
        );
        for bad in ["javascript:", "position", "url(", "expression"] {
            assert!(!body.contains(bad), "输出不应含 {bad}: {body}");
        }
    }

    #[test]
    fn anchor_href_matches_heading_id() {
        // ASCII 目标可与 id 逐字对照；中文 id 保持原样（pulldown 会对 URL 做
        // 百分号编码，浏览器按 URL 规则解码后仍命中同一锚点）。
        let body = body_of("[go](#install)\n\n## Install\n");
        assert!(body.contains("href=\"#install\""), "实际输出: {body}");
        assert!(body.contains("id=\"install\""), "实际输出: {body}");
        let cn = body_of("## 安装\n");
        assert!(cn.contains("id=\"安装\""), "实际输出: {cn}");
    }

    // ===== 脚注导出 =====

    #[test]
    fn single_footnote_renders_ref_and_section() {
        let body = body_of("文本[^1]\n\n[^1]: 说明\n");
        assert!(
            body.contains("<sup class=\"fnref\"><a href=\"#fn-1\" id=\"fnref-1\">[1]</a></sup>"),
            "实际输出: {body}"
        );
        assert!(
            body.contains(
                "<section class=\"footnotes\">\n<hr>\n<ol>\n<li id=\"fn-1\"><p>说明 <a class=\"fnback\" href=\"#fnref-1\">↩</a></p></li>\n</ol>\n</section>\n"
            ),
            "实际输出: {body}"
        );
        // 引用与定义前缀都被消耗，正文中不得再有原始标记
        assert!(!body.contains("[^1]"), "实际输出: {body}");
    }

    #[test]
    fn footnotes_number_by_first_reference_and_dedupe_anchors() {
        // 首次引用顺序：b=1、a=2；b 的第二次引用 id 加 -2 后缀，回跳仍指首次
        let body = body_of("a[^b] b[^a] c[^b]\n\n[^a]: A\n[^b]: B\n");
        assert_eq!(
            body.matches("id=\"fnref-1\"").count(),
            1,
            "实际输出: {body}"
        );
        assert!(body.contains("id=\"fnref-1-2\""), "实际输出: {body}");
        assert!(body.contains("href=\"#fnref-1\""), "实际输出: {body}");
        assert!(
            body.contains(
                "<li id=\"fn-1\"><p>B <a class=\"fnback\" href=\"#fnref-1\">↩</a></p></li>"
            ),
            "实际输出: {body}"
        );
        assert!(
            body.contains(
                "<li id=\"fn-2\"><p>A <a class=\"fnback\" href=\"#fnref-2\">↩</a></p></li>"
            ),
            "实际输出: {body}"
        );
        let first = body.find("<li id=\"fn-1\">").unwrap();
        let second = body.find("<li id=\"fn-2\">").unwrap();
        assert!(first < second, "脚注须按编号升序: {body}");
        assert!(!body.contains("[^"), "实际输出: {body}");
    }

    #[test]
    fn undefined_footnote_ref_stays_literal() {
        // core 会给引用编号，但没有定义就无法产出条目：保留原文、不产生断链
        let body = body_of("无定义 [^ghost]\n");
        assert!(body.contains("[^ghost]"), "实际输出: {body}");
        assert!(!body.contains("footnotes"), "实际输出: {body}");
    }

    #[test]
    fn footnote_ref_inside_code_is_not_replaced() {
        // core 不在代码跨度/围栏内产出 FootnoteRef，源码中的同名文本必须原样保留
        let body = body_of("```\n[^1]\n```\n\n[^1]: x\n");
        assert!(body.contains("[^1]"), "实际输出: {body}");
        assert!(!body.contains("footnotes"), "实际输出: {body}");
        // 代码跨度同理：core 只产出 Code 节点，不得按区间误替换
        let inline = body_of("`[^1]`\n\n[^1]: x\n");
        assert!(inline.contains("<code>[^1]</code>"), "实际输出: {inline}");
        assert!(!inline.contains("footnotes"), "实际输出: {inline}");
    }

    #[test]
    fn unreferenced_footnote_definition_is_not_exported() {
        let body = body_of("[^unused]: 未引用\n\n普通段落\n");
        assert!(!body.contains("未引用"), "实际输出: {body}");
        assert!(!body.contains("footnotes"), "实际输出: {body}");
        assert!(body.contains("<p>普通段落</p>"), "实际输出: {body}");
    }

    #[test]
    fn footnote_definition_renders_inline_formatting() {
        // 定义正文仍经 pulldown 解析：**粗** → <strong>，返回链接落在段落末尾
        let body = body_of("x[^1]\n\n[^1]: **粗** 体\n");
        assert!(
            body.contains(
                "<p><strong>粗</strong> 体 <a class=\"fnback\" href=\"#fnref-1\">↩</a></p>"
            ),
            "实际输出: {body}"
        );
    }

    #[test]
    fn multiline_footnote_definition_continues_paragraph() {
        // 续行在源码里须缩进 4 列；导出剥掉该缩进后继续同一段落，
        // 否则 pulldown 会把它当作缩进代码块输出 <pre>。
        let body = body_of("x[^1]\n\n[^1]: 第一行\n    第二行\n");
        assert!(
            body.contains("<p>第一行\n第二行 <a class=\"fnback\" href=\"#fnref-1\">↩</a></p>"),
            "实际输出: {body}"
        );
        assert!(!body.contains("<pre>"), "实际输出: {body}");
    }

    #[test]
    fn footnote_export_stays_self_contained() {
        let s = Session::open("x[^1]\n\n[^1]: **粗**\n");
        let html = html_of(&s, &ExportStyle::default(), &font());
        assert!(!html.contains("http"), "实际输出: {html}");
        assert!(
            html.contains(".fnref a{text-decoration:none}"),
            "实际输出: {html}"
        );
        assert!(html.contains(".footnotes{"), "实际输出: {html}");
    }

    #[derive(Default)]
    struct MemStore {
        files: Rc<RefCell<HashMap<PathBuf, Vec<u8>>>>,
    }
    impl FileStore for MemStore {
        fn load(&mut self, path: &Path) -> Result<Vec<u8>, String> {
            self.files
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| "文件不存在".to_string())
        }
        fn save(&mut self, path: &Path, data: &[u8]) -> Result<(), String> {
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), data.to_vec());
            Ok(())
        }
    }

    #[test]
    fn exporter_writes_sibling_html_via_atomic_store() {
        let store = MemStore::default();
        let files = store.files.clone();
        let mut e = HtmlExporter {
            font: font(),
            store: Box::new(store),
        };
        let s = Session::open("# T\n");
        let style = ExportStyle::default();
        let got = e
            .export_html(&s, style, Some(Path::new("dir/a.md")), None)
            .unwrap();
        assert_eq!(got, PathBuf::from("dir/a.html")); // 同名换扩展名
        let bytes = files.borrow()[Path::new("dir/a.html")].clone();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            html_of(&s, &style, &font())
        ); // 落盘内容 = html_of 输出
    }

    #[test]
    fn exporter_rejects_draft_without_writing() {
        let store = MemStore::default();
        let files = store.files.clone();
        let mut e = HtmlExporter {
            font: font(),
            store: Box::new(store),
        };
        let s = Session::open("x\n");
        assert!(e
            .export_html(&s, ExportStyle::default(), None, None)
            .is_err()); // 草稿：可读错误
        assert!(files.borrow().is_empty()); // 无半截产物
    }

    #[test]
    fn exporter_writes_to_explicit_target_for_draft() {
        let store = MemStore::default();
        let files = store.files.clone();
        let mut e = HtmlExporter {
            font: font(),
            store: Box::new(store),
        };
        let s = Session::open("x\n");
        // 草稿无源路径，但用户已在对话框选定目标：按目标落盘
        let got = e
            .export_html(
                &s,
                ExportStyle::default(),
                None,
                Some(Path::new("out/draft.html")),
            )
            .unwrap();
        assert_eq!(got, PathBuf::from("out/draft.html"));
        assert_eq!(files.borrow().len(), 1);
    }
}
