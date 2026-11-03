//! HTML 导出（切片 7）：逐块翻译为自包含 HTML 文档。
//! Math 块经 latex-rust → 内嵌 SVG（字形为 path，无外部字体/脚本依赖）；
//! 其余块交 pulldown-cmark（CommonMark 参考实现，仅作导出辅助，不进编辑主链路）。
//! 公式渲染失败降级为转义源码 <pre>，绝不中断导出。

use std::path::{Path, PathBuf};

use veil_core::{BlockKind, Range, Session};
use veil_ui::{Exporter, FileStore};

/// 文档外壳：最小自包含模板（无外部资源）。样式与编辑器视觉对齐（代码灰底、公式居中）。
const HEAD: &str = "<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>veil</title>\n<style>\nbody{max-width:44em;margin:2em auto;padding:0 1em;font:16px/1.6 system-ui,sans-serif;color:#1e1e1e}\ncode,pre{background:#f5f5f5;font-family:Consolas,Menlo,monospace}\ncode{padding:.1em .3em}\npre{padding:.8em;overflow-x:auto}\npre code{padding:0}\ndiv.math{text-align:center;margin:1em 0}\npre.math-source{color:#a00;background:#fdf5f5}\n</style>\n</head>\n<body>\n";

/// 会话 → 自包含 HTML 文档（纯函数，headless 可测）。
pub fn html_of(session: &Session, font: &latex_rust::MathFont) -> String {
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
    let mut i = 0;
    while i < blocks.len() {
        let (id, kind, range) = blocks[i].clone();
        if kind == BlockKind::Math {
            let tex = session.math_source(id).unwrap_or_default();
            body.push_str(&math_html(&tex, font));
            i += 1;
            continue;
        }
        // 定义块自身无输出（已并入 defs 前缀）
        if kind == BlockKind::LinkRefDef {
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
                pulldown_cmark::html::push_html(
                    &mut body,
                    pulldown_cmark::Parser::new_ext(&format!("{defs}{text}"), opts),
                );
            }
            i = j;
            continue;
        }
        // v0 其余块类型均自包含（标题/段落/代码/表格/引用/分隔线），逐块独立解析无跨块上下文
        let Some(text) = session.text_slice(range) else {
            i += 1;
            continue;
        };
        pulldown_cmark::html::push_html(
            &mut body,
            pulldown_cmark::Parser::new_ext(&format!("{defs}{text}"), opts),
        );
        i += 1;
    }
    format!("{HEAD}{body}</body>\n</html>\n")
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
        source_path: Option<&Path>,
    ) -> Result<PathBuf, String> {
        let src = source_path.ok_or_else(|| "草稿无关联文件，导出被忽略".to_string())?;
        let out = src.with_extension("html");
        self.store
            .save(&out, html_of(session, &self.font).as_bytes())?;
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
        let html = html_of(&s, &font());
        let body = html.split_once("<body>\n").unwrap().1;
        body.strip_suffix("</body>\n</html>\n").unwrap().to_string()
    }

    #[test]
    fn heading_and_paragraph_inlines() {
        // 逐字节期望按 pulldown-cmark 0.13 默认输出手工核算：ATX → <h1>…</h1>\n；
        // 段落 → <p>…</p>\n；** → <strong>；` → <code>
        assert_eq!(
            body_of("# Hi\n\nplain **b** and `c`\n"),
            "<h1>Hi</h1>\n<p>plain <strong>b</strong> and <code>c</code></p>\n"
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
        let html = html_of(&s, &font());
        assert!(
            html.starts_with("<!DOCTYPE html>\n<html"),
            "实际输出: {html}"
        );
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("<body>\n<p>x</p>\n</body>"));
        assert!(html.ends_with("</html>\n"));
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
        let got = e.export_html(&s, Some(Path::new("dir/a.md"))).unwrap();
        assert_eq!(got, PathBuf::from("dir/a.html")); // 同名换扩展名
        let bytes = files.borrow()[Path::new("dir/a.html")].clone();
        assert_eq!(String::from_utf8(bytes).unwrap(), html_of(&s, &font())); // 落盘内容 = html_of 输出
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
        assert!(e.export_html(&s, None).is_err()); // 草稿：可读错误
        assert!(files.borrow().is_empty()); // 无半截产物
    }
}
