//! HTML 导出（切片 7）：逐块翻译为自包含 HTML 文档。
//! Math 块经 latex-rust → 内嵌 SVG（字形为 path，无外部字体/脚本依赖）；
//! 其余块交 pulldown-cmark（CommonMark 参考实现，仅作导出辅助，不进编辑主链路）。
//! 公式渲染失败降级为转义源码 <pre>，绝不中断导出。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use veil_core::{
    heading_slugs, percent_decode_path, BlockKind, HtmlTag, InlineKind, InlineNode, Range, Session,
};
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
         font:16px/1.6 system-ui,sans-serif;background:{bg};color:{text};\
         overflow-wrap:anywhere;word-break:break-word}}\n\
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
         p>img:only-child,figure img{{max-height:9.6em}}\n\
         figure{{margin:1em 0;text-align:left}}\n\
         figure.img-center{{text-align:center}}\n\
         figure.img-right{{text-align:right}}\n\
         figcaption{{margin-top:.4em;color:{muted};font-size:.9em;\
         overflow-wrap:anywhere;word-break:break-word}}\n\
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
    html_of_doc(session, style, font, None)
}

/// 同 [`html_of`]，但可用 `base_dir` 把文档内相对图片内嵌为 data URI。
/// `None` = 调用方无文档目录（纯内存测试/草稿），相对图片保持原样。
pub fn html_of_doc(
    session: &Session,
    style: &ExportStyle,
    font: &latex_rust::MathFont,
    base_dir: Option<&Path>,
) -> String {
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
        // 定义块自身无输出（链接定义已并入 defs 前缀；脚注定义统一在文末输出）；
        // Front Matter 是元数据，按产品语义隐藏，正文从元数据之后开始。
        if matches!(
            kind,
            BlockKind::LinkRefDef | BlockKind::FootnoteDef | BlockKind::FrontMatter(_)
        ) {
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
        // 块级原始 HTML（`HtmlBlock`）：pulldown 的 Html 事件会原样输出源码，
        // 直接交 `push_html` 等于把 `<script>`/`onerror` 直出到导出件。
        // 这里单独渲染：危险标签整段剥除，白名单标签过滤属性后保留，其余转义。
        if kind == BlockKind::HtmlBlock {
            if let Some(text) = session.text_slice(range) {
                let safe = sanitize_block_html(&text);
                body.push_str(&safe);
                if !safe.is_empty() && !safe.ends_with('\n') {
                    body.push('\n');
                }
            }
            i += 1;
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
        // 独立图片题注与屏幕/PDF 共用内核判定，避免三端语义漂移。
        let caption = (kind == BlockKind::Paragraph)
            .then(|| veil_core::standalone_image_caption(&nodes, &text, span.start))
            .flatten()
            .map(|(_, caption)| caption);
        // 独占行图片的水平对齐（`<img align>`）：与屏幕/PDF 同一白名单值；
        // 只有整段仅含该图片时才生效，行内图片忽略。
        let standalone_align = (kind == BlockKind::Paragraph)
            .then(|| {
                nodes.iter().find_map(|n| {
                    let align = match &n.kind {
                        InlineKind::HtmlSelfClosed {
                            tag: HtmlTag::Img,
                            attrs,
                        } => attrs.align?,
                        _ => return None,
                    };
                    veil_core::image_standalone(
                        &text,
                        Range::new(span.start, span.start + text.len()),
                        n.range,
                    )
                    .then_some(align)
                })
            })
            .flatten();
        let mut html = String::new();
        pulldown_cmark::html::push_html(
            &mut html,
            pulldown_cmark::Parser::new_ext(&format!("{defs}{rendered}"), opts),
        );
        if caption.is_some() || standalone_align.is_some() {
            let inner = html
                .strip_prefix("<p>")
                .and_then(|h| h.strip_suffix("</p>\n"))
                .unwrap_or(&html);
            let class = match standalone_align {
                Some(a @ (veil_core::ImageAlign::Center | veil_core::ImageAlign::Right)) => {
                    format!(" class=\"img-{}\"", a.as_str())
                }
                _ => String::new(),
            };
            body.push_str(&format!("<figure{class}>\n{inner}\n"));
            if let Some(caption) = caption {
                body.push_str(&format!(
                    "<figcaption>{}</figcaption>\n",
                    escape_html(&caption)
                ));
            }
            body.push_str("</figure>\n");
        } else {
            body.push_str(&html);
        }
        i += 1;
    }
    while !details_stack.is_empty() {
        body.push_str("</details>\n");
        details_stack.pop();
    }
    let notes = footnotes_html(session, &footnote_defs, opts);
    // 图片自包含：远程 src 一律删除（导出件不得隐式联网），本地图片按文档目录
    // 内嵌为 data URI；无法读取/超限/格式不支持时降级为 alt-only，不保留断链。
    let localized = localize_images(&format!("{body}{notes}"), base_dir);
    // 最后一道 scheme 白名单：Markdown 链接（含引用式定义的 dest）由 pulldown 直接写
    // `href`，只能在成品标签上复核；非白名单目标整条属性删除，文字保留。
    let content = sanitize_html_urls(&localized);
    format!("{}{content}</body>\n</html>\n", head(style))
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
/// - HtmlSelfClosed::Img → `<img src="..." alt="..." width= height=>`（远程 src 已置空则降级为
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
                        // 远程 URL 解析层放进 remote_src（不写 src）：导出件不得引用任何
                        // 远程资源。只保留本地 src，后续 localize_images 再按需内嵌；
                        // 两者皆空时降级为 alt-only（缺图提示，不留断链）。
                        let src = attrs.src.as_deref().unwrap_or("");
                        let alt = attrs.alt.as_deref().unwrap_or("");
                        if src.is_empty() {
                            format!(
                                "<img alt=\"{}\"> <!-- veil image unavailable -->",
                                escape_attr(alt)
                            )
                        } else {
                            let mut img = format!("<img src=\"{}\"", escape_attr(src));
                            if !alt.is_empty() {
                                img.push_str(&format!(" alt=\"{}\"", escape_attr(alt)));
                            }
                            // 显式尺寸与 UI 排版同源：作者写了 width/height 就保留，
                            // 否则导出图会退回浏览器原生尺寸、与编辑器内观感不一致。
                            if let Some(width) = attrs.width {
                                img.push_str(&format!(" width=\"{width}\""));
                            }
                            if let Some(height) = attrs.height {
                                img.push_str(&format!(" height=\"{height}\""));
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
    // `<details open>`：布尔属性只需写出属性名。
    if attrs.open {
        parts.push("open".to_string());
    }
    // `<a href>`：目标已由解析层按统一 scheme 白名单过滤（非白名单置空）。
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
    // 显式像素宽高：解析层只收纯十进制数字，导出原样写回即可（浏览器按 CSS 像素解释）。
    if let Some(width) = attrs.width {
        parts.push(format!("width=\"{width}\""));
    }
    if let Some(height) = attrs.height {
        parts.push(format!("height=\"{height}\""));
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

/// 标签名长度（字母开头，后续 alnum/-）；不是合法标签返回 `None`。
fn tag_head_len(inner: &str) -> Option<usize> {
    let b = inner.as_bytes();
    if b.first().is_none_or(|c| !c.is_ascii_alphabetic()) {
        return None;
    }
    let mut i = 1;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') {
        i += 1;
    }
    Some(i)
}

/// `<` 之后找标签结束 `>`（跳过引号内的 `>`），返回 `>` 之后的位置。
fn tag_end(b: &[u8], start: usize) -> Option<usize> {
    let mut quote = 0u8;
    let mut i = start;
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
    None
}

fn tag_boundary(b: Option<&u8>) -> bool {
    matches!(
        b,
        None | Some(b'>') | Some(b'/') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
    )
}

/// 跳过危险标签的内容，返回其闭标签之后的位置；无闭标签则吞到末尾。
/// `script`/`style`/`textarea` 的内容是原始文本，不参与嵌套计数。
fn skip_element(src: &str, from: usize, name: &str) -> usize {
    let lower = src.to_ascii_lowercase();
    let close = format!("</{name}");
    let raw_text = matches!(name, "script" | "style" | "textarea");
    let mut depth = 1usize;
    let mut pos = from;
    while let Some(rel) = lower[pos..].find('<') {
        let at = pos + rel;
        if lower[at..].starts_with(&close) && tag_boundary(lower.as_bytes().get(at + close.len())) {
            depth -= 1;
            if depth == 0 {
                return lower[at..].find('>').map_or(src.len(), |r| at + r + 1);
            }
        } else if !raw_text {
            let open = format!("<{name}");
            if lower[at..].starts_with(&open) && tag_boundary(lower.as_bytes().get(at + open.len()))
            {
                depth += 1;
            }
        }
        pos = at + 1;
    }
    src.len()
}

/// 块级 HTML 标签白名单：只保留 core 认定的行内安全标签。
fn block_tag_allowed(tag: &str) -> bool {
    veil_core::HTML_INLINE_WHITELIST.contains(&tag)
}

/// 属性名是否承载 URL（需要 scheme 白名单校验）。
fn is_url_attr(name: &str) -> bool {
    matches!(
        name,
        "href"
            | "src"
            | "action"
            | "formaction"
            | "poster"
            | "background"
            | "cite"
            | "data"
            | "longdesc"
            | "usemap"
            | "manifest"
            | "xlink:href"
    )
}

/// URL 属性是否可写出：普通链接走统一 scheme 白名单；`<img src>` 额外允许
/// 位图 data URI（自包含导出产物；SVG/文本 data URI 仍拒绝）。
fn url_attr_is_safe(tag: &str, attr: &str, value: &str) -> bool {
    veil_core::link_url_is_safe(value)
        || (tag.eq_ignore_ascii_case("img") && attr == "src" && is_data_image(value))
}

/// 块级 HTML 属性过滤：剥 `on*`、危险 URL 属性、`srcdoc`/`srcset`/`ping`，
/// `style` 经 core 净化，其余属性保留并转义值。
/// `<img src>` 额外允许位图 data URI（导出内嵌图片的唯一合法 `data:` 用法）。
fn sanitize_block_attrs(attr_src: &str, tag: &str) -> String {
    let mut out = String::new();
    let b = attr_src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'/') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let ns = i;
        while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'=' | b'/') {
            i += 1;
        }
        let name = attr_src[ns..i].to_ascii_lowercase();
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        let mut val: Option<&str> = None;
        if b.get(i) == Some(&b'=') {
            i += 1;
            while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            let quote = b.get(i).copied().filter(|c| *c == b'"' || *c == b'\'');
            if let Some(q) = quote {
                i += 1;
                let vs = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                val = Some(&attr_src[vs..i.min(attr_src.len())]);
                i = (i + 1).min(b.len());
            } else {
                let vs = i;
                while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
                    i += 1;
                }
                val = Some(&attr_src[vs..i]);
            }
        }
        if name.is_empty()
            || name.starts_with("on")
            || matches!(name.as_str(), "srcdoc" | "srcset" | "ping")
        {
            continue;
        }
        if name == "style" {
            if let Some(style) = val.and_then(veil_core::sanitize_style) {
                out.push_str(&format!(" style=\"{}\"", escape_attr(&style)));
            }
            continue;
        }
        if is_url_attr(&name) {
            if let Some(v) = val {
                if url_attr_is_safe(tag, &name, v) {
                    out.push_str(&format!(" {name}=\"{}\"", escape_attr(v)));
                }
            }
            continue;
        }
        match val {
            Some(v) => out.push_str(&format!(" {name}=\"{}\"", escape_attr(v))),
            None => {
                out.push(' ');
                out.push_str(&name);
            }
        }
    }
    out
}

/// 块级原始 HTML 的安全化渲染。
///
/// pulldown 会把块级 HTML 原样写进导出件，因此这里必须自行过滤：
/// - `script`/`style`/`iframe`/`object`/`embed`/`form`/`textarea`：整段剥除（含内容）；
/// - 白名单标签：保留标签语义，属性经 [`sanitize_block_attrs`] 过滤；
/// - 其余标签与正文：HTML 转义为可见文本；注释/CDATA/声明整体丢弃。
fn sanitize_block_html(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let start = i;
            while i < b.len() && b[i] != b'<' {
                i += 1;
            }
            out.push_str(&escape_html(&src[start..i]));
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map_or(b.len(), |rel| i + rel + 3);
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i = rest.find("]]>").map_or(b.len(), |rel| i + rel + 3);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            i = rest.find('>').map_or(b.len(), |rel| i + rel + 1);
            continue;
        }
        // `<` 后不是标签起始字符（如正文里的 `a < b`）：按文本转义，
        // 否则会把下一段真实标签一起吞进“标签”里。
        if !matches!(b.get(i + 1), Some(c) if c.is_ascii_alphabetic() || *c == b'/') {
            out.push_str("&lt;");
            i += 1;
            continue;
        }
        let Some(end) = tag_end(b, i + 1) else {
            out.push_str("&lt;");
            i += 1;
            continue;
        };
        let inner = &src[i + 1..end - 1];
        if let Some(close) = inner.strip_prefix('/') {
            let Some(len) = tag_head_len(close) else {
                out.push_str(&escape_html(&src[i..end]));
                i = end;
                continue;
            };
            let raw = &close[..len];
            let name = raw.to_ascii_lowercase();
            if veil_core::should_strip_html(&name) {
                // 孤立闭标签：丢弃
            } else if block_tag_allowed(&name) {
                out.push_str("</");
                out.push_str(raw);
                out.push('>');
            } else {
                out.push_str(&escape_html(&src[i..end]));
            }
            i = end;
            continue;
        }
        let Some(len) = tag_head_len(inner) else {
            out.push_str(&escape_html(&src[i..end]));
            i = end;
            continue;
        };
        let raw = &inner[..len];
        let name = raw.to_ascii_lowercase();
        let self_closed = inner.trim_end().ends_with('/');
        if veil_core::should_strip_html(&name) {
            i = if self_closed || name == "embed" {
                end
            } else {
                skip_element(src, end, &name)
            };
            continue;
        }
        if block_tag_allowed(&name) {
            out.push('<');
            out.push_str(raw);
            out.push_str(&sanitize_block_attrs(&inner[len..], &name));
            if self_closed {
                out.push_str(" />");
            } else {
                out.push('>');
            }
        } else {
            out.push_str(&escape_html(&src[i..end]));
        }
        i = end;
    }
    out
}

/// 成品 HTML 的 scheme 白名单复核：逐个标签删除不安全的 URL 属性与 `on*`。
/// 只处理标签本身，正文/代码块里的字面 `href="…"` 不受影响。
fn sanitize_html_urls(html: &str) -> String {
    let b = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let start = i;
            while i < b.len() && b[i] != b'<' {
                i += 1;
            }
            out.push_str(&html[start..i]);
            continue;
        }
        let Some(end) = tag_end(b, i + 1) else {
            out.push_str(&html[i..]);
            break;
        };
        // `<` 后不是标签起始字符：原样输出该字符，继续扫描后续真实标签。
        if !matches!(b.get(i + 1), Some(c) if c.is_ascii_alphabetic() || *c == b'/') {
            out.push_str(&html[i..i + 1]);
            i += 1;
            continue;
        }
        let inner = &html[i + 1..end - 1];
        if let Some(close) = inner.strip_prefix('/') {
            // 闭标签：危险标签整体丢弃（开标签处已吞掉内容），其余原样保留。
            if let Some(len) = tag_head_len(close) {
                if veil_core::should_strip_html(&close[..len].to_ascii_lowercase()) {
                    i = end;
                    continue;
                }
            }
            out.push_str(&html[i..end]);
            i = end;
            continue;
        }
        let Some(len) = tag_head_len(inner) else {
            // 注释/声明/处理指令：原样保留
            out.push_str(&html[i..end]);
            i = end;
            continue;
        };
        let raw = &inner[..len];
        let name = raw.to_ascii_lowercase();
        let self_closed = inner.trim_end().ends_with('/');
        if veil_core::should_strip_html(&name) {
            // 段落里漏出的 `<object>`/`<iframe>` 等：整段剥除（含内容）。
            i = if self_closed || name == "embed" {
                end
            } else {
                skip_element(html, end, &name)
            };
            continue;
        }
        match strip_unsafe_url_attrs(&inner[len..], &name) {
            // 无属性被删：原样保留（保住 `<input .../>` 之类的既有字节形态）。
            None => out.push_str(&html[i..end]),
            Some(attrs) => {
                out.push('<');
                out.push_str(raw);
                out.push_str(&attrs);
                if self_closed {
                    out.push_str(" />");
                } else {
                    out.push('>');
                }
            }
        }
        i = end;
    }
    out
}

/// 保留属性原文，只删除 `on*`、`srcdoc`/`srcset`/`ping` 与非白名单 URL 目标。
/// 没有任何属性被删时返回 `None`（调用方据此原样输出标签）。
fn strip_unsafe_url_attrs(attr_src: &str, tag: &str) -> Option<String> {
    let mut out = String::new();
    let mut changed = false;
    let b = attr_src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let ws = i;
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'/') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let ns = i;
        while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'=' | b'/') {
            i += 1;
        }
        let name = attr_src[ns..i].to_ascii_lowercase();
        let mut val: Option<&str> = None;
        let mut j = i;
        while j < b.len() && matches!(b[j], b' ' | b'\t' | b'\n' | b'\r') {
            j += 1;
        }
        if b.get(j) == Some(&b'=') {
            j += 1;
            while j < b.len() && matches!(b[j], b' ' | b'\t' | b'\n' | b'\r') {
                j += 1;
            }
            let quote = b.get(j).copied().filter(|c| *c == b'"' || *c == b'\'');
            if let Some(q) = quote {
                j += 1;
                let vs = j;
                while j < b.len() && b[j] != q {
                    j += 1;
                }
                val = Some(&attr_src[vs..j.min(attr_src.len())]);
                j = (j + 1).min(b.len());
            } else {
                let vs = j;
                while j < b.len() && !matches!(b[j], b' ' | b'\t' | b'\n' | b'\r') {
                    j += 1;
                }
                val = Some(&attr_src[vs..j]);
            }
            i = j;
        }
        let drop = name.is_empty()
            || name.starts_with("on")
            || matches!(name.as_str(), "srcdoc" | "srcset" | "ping")
            || (is_url_attr(&name)
                && val.is_none_or(|v| !url_attr_is_safe(tag, &name, v)));
        if drop {
            changed = true;
        } else {
            out.push_str(&attr_src[ws..i]);
        }
    }
    changed.then_some(out)
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
        // 裸链接的可见文本就是 URL 本身：危险 scheme 不产出 <a>，只保留文本
        // （成品 HTML 还有 sanitize_html_urls 兜底，这里提前降级以免留下空链）。
        if !veil_core::link_url_is_safe(&href) {
            reps.push((s, e, escape_link_text(&url)));
            continue;
        }
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
/// 已是完整字符引用（`&amp;` / `&#38;` / `&#x26;`）的 `&` 保持原样，
/// 输入 `A &amp; B` 不会被二次转义成 `A &amp;amp; B`。
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find(['&', '"', '<', '>']) {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let c = tail.chars().next().unwrap_or('&');
        match c {
            '&' => {
                if let Some(len) = html_entity_len(tail) {
                    out.push_str(&tail[..len]);
                    rest = &tail[len..];
                    continue;
                }
                out.push_str("&amp;");
            }
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => {}
        }
        rest = &tail[c.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// `s`（以 `&` 开头）为完整字符引用时返回其字节长度：`&name;` / `&#123;` /
/// `&#x1F;`。非法/未闭合返回 None（调用方按裸 `&` 转义）。
fn html_entity_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.first() != Some(&b'&') {
        return None;
    }
    let mut i = 1usize;
    if b.get(i) == Some(&b'#') {
        i += 1;
        let hex = matches!(b.get(i), Some(b'x' | b'X'));
        if hex {
            i += 1;
        }
        let start = i;
        while i < b.len()
            && if hex {
                b[i].is_ascii_hexdigit()
            } else {
                b[i].is_ascii_digit()
            }
        {
            i += 1;
        }
        if i == start || i - start > 8 {
            return None;
        }
    } else {
        if !b.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_alphanumeric() && i < 32 {
            i += 1;
        }
    }
    (b.get(i) == Some(&b';')).then_some(i + 1)
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

// =============================================================================
// 图片自包含（DEF-T09-001 / DEF-T09-002）
// =============================================================================

/// 单张图片内嵌上限：超过则不内嵌（避免导出件因单张大图失控）。
const MAX_INLINE_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
/// 全部内嵌图片总量上限：超出的图片降级为 alt-only。
const MAX_INLINE_IMAGE_TOTAL_BYTES: u64 = 24 * 1024 * 1024;

/// 图片 `src` 的处置结果。
enum ImageSrc {
    /// 原标签保持不变（无 src、已是安全 data URI、或调用方无基准目录）。
    Keep,
    /// 改写为内嵌 data URI。
    Data(String),
    /// 远程地址：删除 src，绝不产生远程请求。
    Blocked,
    /// 本地不可用/超限/格式不支持：删除 src，保留 alt。
    Unavailable,
}

/// 成品 HTML 的 `<img>` 自包含化：远程 src 删除，本地图片按 `base_dir` 内嵌。
/// 只处理标签本身；正文/代码块里的字面 `<img ...>` 文本不受影响。
fn localize_images(html: &str, base_dir: Option<&Path>) -> String {
    let b = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut budget = MAX_INLINE_IMAGE_TOTAL_BYTES;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let start = i;
            while i < b.len() && b[i] != b'<' {
                i += 1;
            }
            out.push_str(&html[start..i]);
            continue;
        }
        let Some(end) = tag_end(b, i + 1) else {
            out.push_str(&html[i..]);
            break;
        };
        let inner = &html[i + 1..end - 1];
        let Some(len) = tag_head_len(inner) else {
            out.push_str(&html[i..end]);
            i = end;
            continue;
        };
        if !inner[..len].eq_ignore_ascii_case("img") {
            out.push_str(&html[i..end]);
            i = end;
            continue;
        }
        let attrs = parse_tag_attrs(&inner[len..]);
        let src = attrs
            .iter()
            .find(|(name, _)| name == "src")
            .and_then(|(_, value)| value.as_deref());
        match decide_image_src(src, base_dir, &mut budget) {
            ImageSrc::Keep => out.push_str(&html[i..end]),
            ImageSrc::Data(uri) => out.push_str(&render_img_tag(&attrs, Some(&uri))),
            ImageSrc::Blocked => {
                out.push_str(&render_img_tag(&attrs, None));
                out.push_str(" <!-- veil blocked remote -->");
            }
            ImageSrc::Unavailable => {
                out.push_str(&render_img_tag(&attrs, None));
                out.push_str(" <!-- veil image unavailable -->");
            }
        }
        i = end;
    }
    out
}

/// 判定单个图片 src 的处置方式（`budget` 为剩余可内嵌字节数）。
fn decide_image_src(src: Option<&str>, base_dir: Option<&Path>, budget: &mut u64) -> ImageSrc {
    let Some(raw) = src else {
        return ImageSrc::Keep;
    };
    let value = raw.trim();
    if value.is_empty() {
        return ImageSrc::Keep;
    }
    match veil_core::link_scheme(value).as_deref() {
        // 远程资源：导出件不得隐式联网（离线打开也不该产生第三方请求）。
        Some("http" | "https") => ImageSrc::Blocked,
        // 作者已写成 data URI：只接受位图 MIME，SVG 等可携带脚本的一律删除。
        Some("data") => {
            if is_data_image(value) {
                ImageSrc::Keep
            } else {
                ImageSrc::Blocked
            }
        }
        // file:/javascript:/盘符绝对路径等：不读取、不写出，降级为 alt。
        Some(_) => ImageSrc::Unavailable,
        None => {
            let Some(dir) = base_dir else {
                return ImageSrc::Keep;
            };
            let path = dir.join(percent_decode_path(strip_query_fragment(value)));
            let Ok(meta) = std::fs::metadata(&path) else {
                return ImageSrc::Unavailable;
            };
            if !meta.is_file() || meta.len() > MAX_INLINE_IMAGE_BYTES || meta.len() > *budget {
                return ImageSrc::Unavailable;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                return ImageSrc::Unavailable;
            };
            let Some(mime) = image_mime(&bytes) else {
                return ImageSrc::Unavailable;
            };
            *budget -= bytes.len() as u64;
            ImageSrc::Data(format!("data:{mime};base64,{}", base64_encode(&bytes)))
        }
    }
}

/// 按原属性顺序重建 `<img>`：`src` 有值时写内嵌 URI，否则整条省略（alt 仍在）。
fn render_img_tag(attrs: &[(String, Option<String>)], src: Option<&str>) -> String {
    let mut out = String::from("<img");
    for (name, value) in attrs {
        if name == "src" {
            if let Some(uri) = src {
                out.push_str(&format!(" src=\"{}\"", escape_attr(uri)));
            }
            continue;
        }
        match value {
            Some(v) => out.push_str(&format!(" {name}=\"{}\"", escape_attr(v))),
            None => {
                out.push(' ');
                out.push_str(name);
            }
        }
    }
    out.push('>');
    out
}

/// 解析标签属性为 `(小写名, 原始值)`；值已做 HTML 实体还原，重建时统一转义。
fn parse_tag_attrs(attr_src: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let b = attr_src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'/') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let ns = i;
        while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b'=' | b'/') {
            i += 1;
        }
        let name = attr_src[ns..i].to_ascii_lowercase();
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        let mut value = None;
        if b.get(i) == Some(&b'=') {
            i += 1;
            while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            let quote = b.get(i).copied().filter(|c| *c == b'"' || *c == b'\'');
            if let Some(q) = quote {
                i += 1;
                let vs = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                value = Some(decode_attr(&attr_src[vs..i.min(attr_src.len())]));
                i = (i + 1).min(b.len());
            } else {
                let vs = i;
                while i < b.len() && !matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
                    i += 1;
                }
                value = Some(decode_attr(&attr_src[vs..i]));
            }
        }
        if !name.is_empty() {
            out.push((name, value));
        }
    }
    out
}

/// 还原常见 HTML 实体（属性值解析用；重建时由 `escape_attr` 统一再转义）。
fn decode_attr(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            if let Some((ch, used)) = decode_entity(&s[i..]) {
                out.push(ch);
                i += used;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap_or('\u{0}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// 解码 `&#NN;` / `&#xNN;` / `&amp;` 等实体；无法识别返回 None。
fn decode_entity(s: &str) -> Option<(char, usize)> {
    let semi = s[1..].find(';').map(|p| p + 1)?;
    let body = &s[1..semi];
    let ch = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
        char::from_u32(u32::from_str_radix(hex, 16).ok()?)?
    } else if let Some(dec) = body.strip_prefix('#') {
        char::from_u32(dec.parse::<u32>().ok()?)?
    } else {
        match body {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => return None,
        }
    };
    Some((ch, semi + 1))
}

/// 去掉本地路径上的 `?query` 与 `#fragment`（导出只关心文件本身）。
fn strip_query_fragment(value: &str) -> &str {
    let cut = value
        .find(['?', '#'])
        .unwrap_or(value.len());
    &value[..cut]
}

/// 按魔数判定位图 MIME；SVG 等文本格式不内嵌（可携带脚本）。
fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") && bytes.len() > 26 {
        Some("image/bmp")
    } else {
        None
    }
}

/// `data:image/<位图>;base64,` 判定（大小写不敏感；payload 须为纯 base64）。
fn is_data_image(value: &str) -> bool {
    let Some((head, payload)) = value.split_once(',') else {
        return false;
    };
    let lower = head.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("data:") else {
        return false;
    };
    let Some(mime) = rest.strip_suffix(";base64") else {
        return false;
    };
    let known = ["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp"];
    if !known.contains(&mime) {
        return false;
    }
    !payload.is_empty()
        && payload
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'='))
}

/// base64 编码（标准字母表，带 `=` 填充）：`base64` crate 的 STANDARD 引擎。
fn base64_encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// 重名导出递增命名：`name.ext` 已存在时改用 `name (2).ext`、`name (3).ext`……
/// 只用于导出目标（不替换既有文件）；任何 IO 异常按「可用」处理，交由写入报错。
pub(crate) fn unique_file_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    for n in 2..1000u32 {
        let name = if ext.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let candidate = path.with_file_name(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
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
        // DEF-T09-004：重名导出递增命名，绝不替换既有文件。
        let out = unique_file_path(&out);
        // 本地图片按文档所在目录解析并内嵌（草稿无源路径时保留相对外链）。
        let base_dir = source_path.and_then(|p| p.parent());
        let html = match base_dir {
            Some(dir) => html_of_doc(session, &style, &self.font, Some(dir)),
            None => html_of(session, &style, &self.font),
        };
        self.store.save(&out, html.as_bytes())?;
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

    #[test]
    fn base64_encode_matches_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
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
    fn inline_abbr_title_entity_is_not_double_escaped() {        // TASK04-D007：作者已写好的实体只做一次处理，不得变成 `&amp;amp;`。
        let body = body_of("<abbr title=\"A &amp; B\">AB</abbr>\n");
        assert_eq!(body, "<p><abbr title=\"A &amp; B\">AB</abbr></p>\n");
        // 裸 `&` 仍必须转义为实体。
        let body = body_of("<abbr title=\"A & B\">AB</abbr>\n");
        assert_eq!(body, "<p><abbr title=\"A &amp; B\">AB</abbr></p>\n");
    }

    #[test]
    fn inline_details_open_attribute_is_exported() {
        // TASK04-D008：行内 `<details open>` 的布尔属性在导出件中保留。
        let body = body_of("before <details open><summary>Sum</summary>body</details> after\n");
        assert!(
            body.contains("<details open><summary>Sum</summary>"),
            "实际输出: {body}"
        );
        assert!(!body.contains("<details >"), "不得产生空属性: {body}");
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
        assert_eq!(
            body_of("```\nif a < b {}\n```\n"),
            "<pre><code>if a &lt; b {}\n</code></pre>\n"
        );
    }

    #[test]
    fn fenced_code_language_class_preserves_rust_alias_and_unknown_language() {
        for (lang, marker) in [
            ("rust", "let rust = 1;"),
            ("rs", "let alias = 1;"),
            ("not-a-known-language", "let unknown = 1;"),
        ] {
            let body = body_of(&format!("```{lang}\n{marker}\n```\n"));
            assert!(
                body.contains(&format!("<code class=\"language-{lang}\">{marker}")),
                "语言语义应保留：{body}"
            );
            assert!(!body.contains("<script"), "未知语言不得产生脚本：{body}");
        }
    }

    #[test]
    fn fenced_code_language_attribute_is_escaped() {
        let body = body_of("```unknown\"><script>alert(1)</script>\nlet x = 1;\n```\n");
        assert!(
            !body.contains("<script>"),
            "info string 不得注入标签：{body}"
        );
        assert!(
            body.contains("&quot;&gt;&lt;script&gt;"),
            "语言名应转义：{body}"
        );
        assert!(body.contains("let x = 1;"), "代码正文须保留：{body}");
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
        // GFM 自动链接扩展：`www.` 补 http、裸邮箱补 mailto；
        // URL 中的 `&` 在 href 与可见文本里都必须转义。
        // 非白名单 scheme（ftp 等）只保留可见文本，不产出 `<a>`。
        assert_eq!(
            body_of("Visit www.a.com/x, or ftp://a.b/c\n"),
            "<p>Visit <a href=\"http://www.a.com/x\">www.a.com/x</a>, or \
             ftp://a.b/c</p>\n"
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
    fn standalone_image_exports_semantic_figure_and_caption() {
        assert_eq!(
            body_of("![示例](img/a.png)\n"),
            "<figure>\n<img src=\"img/a.png\" alt=\"示例\" />\n<figcaption>示例</figcaption>\n</figure>\n"
        );
        let body = body_of("![长 CJK 题注](img/a.png)\n");
        assert!(body.contains("<figure>"), "独立图片应使用 figure：{body}");
        assert!(
            body.contains("<figcaption>长 CJK 题注</figcaption>"),
            "{body}"
        );
    }

    #[test]
    fn inline_and_empty_alt_images_do_not_get_captions() {
        for body in [
            body_of("前文 ![行内图](img/a.png) 后文\n"),
            body_of("![](img/a.png)\n"),
            body_of("![   ](img/a.png)\n"),
        ] {
            assert!(!body.contains("<figure>"), "不得生成独立题注：{body}");
            assert!(
                !body.contains("<figcaption>"),
                "不得生成 figcaption：{body}"
            );
        }
    }

    #[test]
    fn consecutive_standalone_images_each_get_caption() {
        // 与屏幕/PDF 共用内核判定：两个独占段落的图片各自出题注
        let body = body_of("![甲](img/a.png)\n\n![乙](img/b.png)\n");
        assert_eq!(body.matches("<figcaption>").count(), 2, "{body}");
        assert!(body.contains("<figcaption>甲</figcaption>"), "{body}");
        assert!(body.contains("<figcaption>乙</figcaption>"), "{body}");
    }

    #[test]
    fn multiple_images_in_one_paragraph_get_no_caption() {
        // 同段多图（换行或同一行）不派生题注，与屏幕语义一致
        for body in [
            body_of("![甲](img/a.png)\n![乙](img/b.png)\n"),
            body_of("![甲](img/a.png) ![乙](img/b.png)\n"),
        ] {
            assert!(!body.contains("<figcaption>"), "{body}");
        }
    }

    #[test]
    fn standalone_html_img_exports_figure_without_relaxing_attribute_whitelist() {
        let body = body_of(
            "<img src=\"img/a.png\" alt=\"安全题注\" onerror=\"alert(1)\" width=\"120\">\n",
        );
        assert!(body.contains("<figure>"), "{body}");
        assert!(body.contains("<figcaption>安全题注</figcaption>"), "{body}");
        assert!(body.contains("width=\"120\""), "{body}");
        assert!(!body.contains("onerror"), "事件属性仍须清理：{body}");
    }

    #[test]
    fn caption_text_and_dangerous_image_scheme_stay_sanitized() {
        let body = body_of("![<script>alert(1)</script>](javascript:alert(1))\n");
        assert!(body.contains("<figcaption>&lt;script&gt;alert(1)&lt;/script&gt;</figcaption>"));
        assert!(!body.contains("<script>"), "题注不得注入脚本：{body}");
        assert!(
            !body.contains("src=\"javascript:"),
            "危险 scheme 仍须清理：{body}"
        );
    }

    #[test]
    fn html_img_explicit_size_exported() {
        // `<img width height>` 的显式尺寸须写回导出 HTML（parse_open_tag 白名单收集）。
        let body = body_of("<img src=\"img/a.png\" alt=\"示例\" width=\"120\" height=\"80\">\n");
        assert!(body.contains("src=\"img/a.png\""), "实际输出: {body}");
        assert!(body.contains("width=\"120\""), "实际输出: {body}");
        assert!(body.contains("height=\"80\""), "实际输出: {body}");
        // 非法值被解析层丢弃 → 导出不得出现 width/height
        let body = body_of("<img src=\"img/a.png\" width=\"100px\">\n");
        assert!(!body.contains("width="), "实际输出: {body}");
        assert!(!body.contains("height="), "实际输出: {body}");
    }

    #[test]
    fn html_img_alignment_exports_figure_class_without_leaking_attr() {
        let body = body_of("<img src=\"img/a.png\" align=\"center\">\n");
        assert!(body.contains("<figure class=\"img-center\">"), "{body}");
        assert!(!body.contains("align="), "对齐属性不得原样泄漏：{body}");
        let body = body_of("<img src=\"img/a.png\" align=\"right\">\n");
        assert!(body.contains("<figure class=\"img-right\">"), "{body}");
        // 行内图片 / 非法值：不改变版式，也不写 align。
        let body = body_of("前文 <img src=\"img/a.png\" align=\"center\"> 后文\n");
        assert!(!body.contains("<figure"), "{body}");
        assert!(!body.contains("align="), "{body}");
        let body = body_of("<img src=\"img/a.png\" align=\"justify\">\n");
        assert!(!body.contains("class=\"img-"), "{body}");
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
            css.contains("p>img:only-child,figure img{max-height:9.6em}"),
            "实际输出: {css}"
        );
        assert!(
            css.contains("figcaption{margin-top:.4em;color:#666666;font-size:.9em;overflow-wrap:anywhere;word-break:break-word}"),
            "题注样式与换行兜底：{css}"
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
    fn block_html_strips_scripts_styles_and_embeds() {
        // DEF-T11-001：块级原始 HTML 不得原样交给 push_html。
        let body = body_of(
            "<script>alert(1)</script>\n\n<style>body{display:none}</style>\n\n\
             <iframe src=\"https://evil/x\"></iframe>\n\n<object data=\"file:///C:/x\"></object>\n\n\
             <embed src=\"data:text/html;base64,AA\">\n\n\
             <form action=\"https://evil\"><input value=\"x\"></form>\n\n\
             <textarea>do not submit</textarea>\n",
        );
        for bad in [
            "<script",
            "</script",
            "<style",
            "<iframe",
            "<object",
            "<embed",
            "<form",
            "<textarea",
        ] {
            assert!(!body.contains(bad), "不应含 {bad}: {body}");
        }
        for leaked in ["alert(1)", "display:none", "do not submit"] {
            assert!(!body.contains(leaked), "危险标签内容须整段剥除: {body}");
        }
    }

    #[test]
    fn block_html_keeps_whitelisted_tags_with_safe_attrs() {
        // 块级白名单标签保留语义；on* 与危险 style 声明被剥除。
        let body = body_of("<pre class=\"x\" onclick=\"alert(1)\" style=\"color:red;position:fixed\">a < b</pre>\n");
        assert!(
            body.contains("<pre class=\"x\" style=\"color:red\">"),
            "{body}"
        );
        assert!(!body.contains("onclick"), "{body}");
        assert!(!body.contains("position"), "{body}");
        assert!(body.contains("a &lt; b"), "正文须转义: {body}");
        assert!(body.contains("</pre>"), "闭标签须保留: {body}");
        // 注释与声明整体丢弃，不残留隐藏载体。
        let body = body_of("<!-- <script>alert(1)</script> -->\n\n<!DOCTYPE html>\n");
        assert!(!body.contains("<script"), "{body}");
        assert!(!body.contains("DOCTYPE"), "{body}");
    }

    #[test]
    fn dangerous_link_schemes_are_stripped_from_export() {
        // DEF-T11-002：Markdown 链接、行内 <a>、图片目标与引用式定义统一过滤。
        let body = body_of(
            "[JS](javascript:alert(1))\n[JS mixed](JaVaScRiPt:alert(1))\n\
             [entity](javascript&#58;alert(1))\n[data](data:text/html;base64,AA)\n\
             [file](file:///C:/x)\n[vb](vbscript:MsgBox(1))\n\
             [ws](java%0ascript:alert(1))\n\n\
             <a href=\"javascript:alert(1)\">html</a>\n\n\
             ![img](javascript:alert(1))\n",
        );
        let lower = body.to_ascii_lowercase();
        for bad in ["javascript:", "data:", "file:", "vbscript:"] {
            assert!(!lower.contains(bad), "不应含 {bad}: {body}");
        }
        assert!(body.contains(">html</a>"), "标签文字保留: {body}");
    }

    #[test]
    fn safe_link_schemes_and_relative_targets_are_preserved() {
        let body = body_of(
            "[a](https://example.com/x) [b](mailto:a@b.c) [c](tel:+123) \
             [d](#install) [e](img/a.png)\n",
        );
        for good in [
            "href=\"https://example.com/x\"",
            "href=\"mailto:a@b.c\"",
            "href=\"tel:+123\"",
            "href=\"#install\"",
            "href=\"img/a.png\"",
        ] {
            assert!(body.contains(good), "应保留 {good}: {body}");
        }
    }

    #[test]
    fn dangerous_reference_link_definition_is_stripped() {
        // 引用式目标的 href 由 pulldown 从定义处生成，须在成品标签上复核。
        let body = body_of("[x][r]\n\n[r]: javascript:alert(1)\n");
        assert!(
            !body.to_ascii_lowercase().contains("javascript:"),
            "实际输出: {body}"
        );
        assert!(body.contains('x'), "可见文字保留: {body}");
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

    // ===== Task 4：HTML/PDF 导出缺陷回归 =====

    /// 唯一临时目录（带进程/纳秒后缀，避免并行测试互相踩踏）。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "veil_export_test_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 最小 PNG 头（导出只做魔数判定，不解码像素）。
    const PNG_HEADER: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];

    #[test]
    fn front_matter_is_hidden_from_export() {
        // DEF-T09-DEEP-001：元数据不得变成 <hr> + 正文标题。
        let body = body_of("---\ntitle: 标题\ntags: [a, b]\n---\n正文\n");
        assert!(!body.contains("title:"), "实际输出: {body}");
        assert!(!body.contains("<hr"), "实际输出: {body}");
        assert!(!body.contains("tags:"), "实际输出: {body}");
        assert!(body.contains("<p>正文</p>"), "正文须保留: {body}");
    }

    #[test]
    fn remote_image_src_is_not_exported() {
        // DEF-T09-001：远程 URL 不得写入 src，也不得出现在导出件任何位置。
        let body = body_of("![远程](https://example.invalid/p.png)\n");
        assert!(!body.contains("example.invalid"), "实际输出: {body}");
        assert!(!body.contains("src=\"http"), "实际输出: {body}");
        assert!(
            body.contains("<img alt=\"远程\">") && body.contains("<!-- veil blocked remote -->"),
            "须降级为 alt-only: {body}"
        );
    }

    #[test]
    fn remote_inline_img_src_is_not_exported() {
        // 行内 HTML 图片的远程地址（core 存在 remote_src）同样不得还原。
        let body = body_of("<img src=\"https://example.invalid/a.png\" alt=\"远程\">\n");
        assert!(!body.contains("example.invalid"), "实际输出: {body}");
        assert!(!body.contains("src=\"http"), "实际输出: {body}");
    }

    #[test]
    fn local_image_is_embedded_as_data_uri() {
        // DEF-T09-002：本地图片内嵌为 data URI，移动导出件后图片仍可用。
        let dir = temp_dir("embed");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/p.png"), PNG_HEADER).unwrap();
        let s = Session::open("![pic](assets/p.png)\n");
        let html = html_of_doc(&s, &ExportStyle::default(), &font(), Some(&dir));
        assert!(
            html.contains("src=\"data:image/png;base64,iVBORw0KGgoAAAAA\""),
            "须内嵌为 data URI: {html}"
        );
        assert!(!html.contains("src=\"assets/p.png\""), "不得保留相对外链");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_local_image_degrades_without_src() {
        let dir = temp_dir("missing");
        let s = Session::open("![pic](assets/missing.png)\n");
        let html = html_of_doc(&s, &ExportStyle::default(), &font(), Some(&dir));
        assert!(!html.contains("src="), "不得保留断链: {html}");
        assert!(
            html.contains("<img alt=\"pic\">") && html.contains("<!-- veil image unavailable -->"),
            "须降级为 alt-only: {html}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn non_raster_image_is_not_embedded() {
        // SVG 可携带脚本，导出器只内嵌位图格式。
        let dir = temp_dir("svg");
        std::fs::write(dir.join("v.svg"), b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>")
            .unwrap();
        let s = Session::open("![v](v.svg)\n");
        let html = html_of_doc(&s, &ExportStyle::default(), &font(), Some(&dir));
        assert!(!html.contains("src="), "SVG 不得内嵌/外链: {html}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn oversized_local_image_degrades_instead_of_bloating_export() {
        let dir = temp_dir("oversize");
        let mut bytes = PNG_HEADER.to_vec();
        bytes.resize(MAX_INLINE_IMAGE_BYTES as usize + 1, 0);
        std::fs::write(dir.join("big.png"), &bytes).unwrap();
        let s = Session::open("![big](big.png)\n");
        let html = html_of_doc(&s, &ExportStyle::default(), &font(), Some(&dir));
        assert!(!html.contains("src="), "超限图片应降级: {}", &html[..200.min(html.len())]);
        assert!(html.contains("<!-- veil image unavailable -->"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn long_unbroken_line_wraps_within_page() {
        // DEF-T09-E1-002：超长无断点行不得造成页面级横向溢出。
        let long = "A".repeat(400);
        let html = html_of(
            &Session::open(&format!("{long}\n")),
            &ExportStyle::default(),
            &font(),
        );
        assert!(
            html.contains("overflow-wrap:anywhere") && html.contains("word-break:break-word"),
            "须有换行兜底 CSS"
        );
    }

    #[test]
    fn export_target_increments_on_collision() {
        // DEF-T09-004：重名导出递增命名，不替换既有文件。
        let dir = temp_dir("increment");
        let first = dir.join("doc.html");
        std::fs::write(&first, b"old").unwrap();
        assert_eq!(unique_file_path(&first), dir.join("doc (2).html"));
        std::fs::write(dir.join("doc (2).html"), b"old2").unwrap();
        assert_eq!(unique_file_path(&first), dir.join("doc (3).html"));

        let mut e = HtmlExporter {
            font: font(),
            store: Box::new(MemStore::default()),
        };
        let got = e
            .export_html(
                &Session::open("x\n"),
                ExportStyle::default(),
                None,
                Some(&first),
            )
            .unwrap();
        assert_eq!(got, dir.join("doc (3).html"));
        assert_eq!(std::fs::read(&first).unwrap(), b"old", "既有文件不得被替换");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn data_uri_images_only_allow_raster_mime() {
        // 自包含 data URI 只对位图放行；SVG/文本 data URI 仍按危险 scheme 处理。
        let body = body_of("![x](data:image/png;base64,iVBORw0KGgo=)\n");
        assert!(body.contains("src=\"data:image/png;base64,iVBORw0KGgo=\""), "{body}");
        let body = body_of("![x](data:image/svg+xml;base64,PHN2Zy8+)\n");
        assert!(!body.contains("data:image/svg"), "{body}");
        assert!(!body.contains("src="), "{body}");
    }
}
