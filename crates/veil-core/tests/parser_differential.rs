//! 双解析器差分 oracle（Task 5）。
//!
//! 编辑器使用自研增量语义树；HTML 导出/一致性门禁使用 pulldown-cmark。本测试不让两者
//! 合流，只让差异可见、可测、可解释。
//!
//! 运行：
//! ```powershell
//! cargo test -p veil-core parser_differential -- --nocapture
//! ```
//!
//! 语料（复用，不新造）：
//! - `tests/conformance/examples`：CommonMark 0.31.2 官方 652 例（含期望 HTML）。
//! - `tests/conformance/gfm`：GFM 扩展 23 例（表格/任务列表/删除线/裸链接）。
//! - `tests/conformance/real`：47 篇真实文档（无期望 HTML，只比结构与文本）。
//! - 内联样例：CJK、callouts、公式、脚注等编辑器扩展（语料集中没有的部分）。
//!
//! 方法（四个维度）：
//! 1. 块级类型：Veil 顶层块序列 vs pulldown-cmark 顶层块序列（复用 `tests/common`
//!    口径：列表项折叠为 List，原始 HTML 块两侧都不计）。
//! 2. 文本归一化：两侧块一一对齐时，按源码切片折叠空白后比较。
//! 3. 边界：对齐块比较字节区间；文本相同而区间不同记为「边界表示差异」。
//! 4. 渲染：`canonical(导出 HTML)` vs `canonical(规范期望 HTML)`（与一致性门禁同口径）。
//!
//! 分类：语法覆盖（一侧独有的语法）/ 位置边界（同内容不同切口）/ 渲染文本差异。
//! 门禁方向：只允许改善，不允许退化（`<=` 基线）。

mod common;

use std::ops::Range as ByteRange;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use common::{actual_blocks, canonical, export_body, pulldown_blocks, short, B};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use veil_core::{AdmonitionType, BlockKind, InlineKind, Range, Session};

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/conformance/examples");
const GFM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/conformance/gfm");
const REAL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/conformance/real");

/// 差分基线（2026-11-26 当前 HEAD 实测，只允许改善）。
const BASELINE_TYPE_MISMATCH_FILES: usize = 0; // CommonMark + GFM 的块骨架差异
const BASELINE_REAL_TYPE_MISMATCH_FILES: usize = 19; // 与 real_world_check 的已接受差异同阈
/// 对齐块的归一化文本差异，当前 3 处，均与链接引用定义位置有关：
/// - commonmark 0210：多行链接定义（`[foo]: /url` + 次行 `"title" ok`）编辑器整段视为段落，
///   参照把首行当作定义、段落从 `"title"` 开始；
/// - real rust / tokio README：列表末尾的链接定义在编辑器是独立 `LinkRefDef` 块
///   （不并入 List 区间），参照将其折进 List 容器区间。
const BASELINE_TEXT_DIFF_BLOCKS: usize = 3;
/// canonical HTML 差异（CommonMark 24 + GFM 4），全部是渲染器序列化差异
/// （实体转义、`align` vs `style`、`<input>` 属性序、空 `<tbody>`），不是块模型差异。
const BASELINE_RENDER_DIFF_FILES: usize = 28;

#[derive(Debug, Default)]
struct Stats {
    files: usize,
    panics: usize,
    /// 块骨架不一致的文件数。
    type_mismatch_files: usize,
    /// 含编辑器独有块（callout/math/mermaid/footnote 定义/details/front matter）的文件数。
    extension_files: usize,
    /// 骨架一致但含编辑器独有语义块（如 Admonition vs BlockQuote）的文件数。
    semantic_only_files: usize,
    /// 骨架一致且无编辑器独有块的文件数。
    match_files: usize,
    /// 块级对齐（两侧块数相同）的文件数。
    aligned_files: usize,
    /// 骨架一致但块数不同，跳过逐块比较的文件数。
    unaligned_files: usize,
    blocks_compared: usize,
    text_equal_blocks: usize,
    offset_equal_blocks: usize,
    /// 文本相同、字节区间不同（尾部换行/容器切口等表示差异）。
    boundary_notation_blocks: usize,
    /// 归一化文本不同。
    text_diff_blocks: usize,
    /// canonical HTML 不同。
    render_diff_files: usize,
}

impl Stats {
    fn merge(&mut self, o: &Stats) {
        self.files += o.files;
        self.panics += o.panics;
        self.type_mismatch_files += o.type_mismatch_files;
        self.extension_files += o.extension_files;
        self.semantic_only_files += o.semantic_only_files;
        self.match_files += o.match_files;
        self.aligned_files += o.aligned_files;
        self.unaligned_files += o.unaligned_files;
        self.blocks_compared += o.blocks_compared;
        self.text_equal_blocks += o.text_equal_blocks;
        self.offset_equal_blocks += o.offset_equal_blocks;
        self.boundary_notation_blocks += o.boundary_notation_blocks;
        self.text_diff_blocks += o.text_diff_blocks;
        self.render_diff_files += o.render_diff_files;
    }
}

/// 参照侧的顶层块种类（只取结构，不含内容）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RKind {
    P,
    H(u8),
    Code,
    Quote,
    List,
    Table,
    Rule,
    Html,
}

impl RKind {
    fn to_b(self) -> Option<B> {
        Some(match self {
            Self::P => B::P,
            Self::H(n) => B::H(n),
            Self::Code => B::Pre,
            Self::Quote => B::Quote,
            Self::List => B::List,
            Self::Table => B::Table,
            Self::Rule => B::Rule,
            Self::Html => return None,
        })
    }
}

fn ref_options() -> Options {
    // 与 `common::pulldown_blocks` / 一致性门禁一致的扩展集：
    // 表格、删除线、任务列表、GFM 引用块类型。导出路径本身不启用 GFM，
    // 但 GFM 只影响引用块 kind，不影响块骨架。
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
}

/// pulldown-cmark 顶层块 + 源码字节区间（原始 HTML 块保留，供上层剔除）。
fn ref_blocks(md: &str) -> Vec<(RKind, ByteRange<usize>)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    for (ev, range) in Parser::new_ext(md, ref_options()).into_offset_iter() {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph if depth == 0 => out.push((RKind::P, range)),
                Tag::Heading { level, .. } if depth == 0 => {
                    out.push((RKind::H(level as u8), range))
                }
                Tag::CodeBlock(_) if depth == 0 => out.push((RKind::Code, range)),
                Tag::BlockQuote(_) => {
                    if depth == 0 {
                        out.push((RKind::Quote, range));
                    }
                    depth += 1;
                }
                Tag::List(_) => {
                    if depth == 0 {
                        out.push((RKind::List, range));
                    }
                    depth += 1;
                }
                Tag::Table(_) => {
                    if depth == 0 {
                        out.push((RKind::Table, range));
                    }
                    depth += 1;
                }
                Tag::HtmlBlock if depth == 0 => out.push((RKind::Html, range)),
                _ => {}
            },
            Event::Rule if depth == 0 => out.push((RKind::Rule, range)),
            Event::End(TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Table) => depth -= 1,
            _ => {}
        }
    }
    out
}

/// 编辑器侧用于逐块比对的顶层块：与 `actual_blocks` 同粒度（连续列表项合并为一个
/// List 区间），并跳过不产 HTML 的定义块与 html 块。
#[derive(Clone)]
struct EBlock {
    kind: BlockKind,
    start: usize,
    end: usize,
}

fn editor_diff_blocks(s: &Session) -> Vec<EBlock> {
    let mut out: Vec<EBlock> = Vec::new();
    // 当前顶层列表的标记签名（ordered, marker）。标记变化起新列表（与
    // `common::actual_blocks` 的 Example 301/302 口径一致）。
    let mut cur_sig: Option<(bool, u8)> = None;
    for (_, kind, r) in s.blocks() {
        match &kind {
            BlockKind::LinkRefDef
            | BlockKind::FootnoteDef
            | BlockKind::FrontMatter(_)
            | BlockKind::Details { .. }
            | BlockKind::HtmlBlock => {
                cur_sig = None;
            }
            BlockKind::ListItem { .. } => {
                let sig = s
                    .text_slice(Range::new(r.start, r.end))
                    .and_then(|t| list_marker_sig(t.lines().next().unwrap_or_default()));
                let Some(sig) = sig else {
                    // 续块（首行无合法标记）：并入前项区间，不产生新块。
                    if let Some(last) = out
                        .last_mut()
                        .filter(|b| matches!(&b.kind, BlockKind::ListItem { .. }))
                    {
                        last.end = r.end;
                    }
                    continue;
                };
                let same = cur_sig == Some(sig);
                if same {
                    if let Some(last) = out
                        .last_mut()
                        .filter(|b| matches!(&b.kind, BlockKind::ListItem { .. }))
                    {
                        last.end = r.end;
                    }
                } else {
                    out.push(EBlock {
                        kind,
                        start: r.start,
                        end: r.end,
                    });
                    cur_sig = Some(sig);
                }
            }
            _ => {
                out.push(EBlock {
                    kind,
                    start: r.start,
                    end: r.end,
                });
                cur_sig = None;
            }
        }
    }
    out
}

/// 列表项首行的标记签名：`(ordered, marker)`；无合法标记返回 None。
fn list_marker_sig(line: &str) -> Option<(bool, u8)> {
    let b = line.as_bytes();
    let mut i = 0usize;
    while i < b.len() && matches!(b[i], b' ' | b'\t') {
        i += 1;
    }
    if b.get(i)?.is_ascii_digit() {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        match b.get(i)? {
            b'.' => Some((true, b'.')),
            b')' => Some((true, b')')),
            _ => None,
        }
    } else {
        match b.get(i)? {
            b'-' => Some((false, b'-')),
            b'*' => Some((false, b'*')),
            b'+' => Some((false, b'+')),
            _ => None,
        }
    }
}

/// 编辑器独有、pulldown-cmark（导出配置）无对应语义的块。
fn extension_block(kind: &BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::Math
            | BlockKind::Mermaid
            | BlockKind::Admonition(_)
            | BlockKind::Callout(_)
            | BlockKind::FootnoteDef
            | BlockKind::FrontMatter(_)
            | BlockKind::Details { .. }
    )
}

fn has_extension_block(s: &Session) -> bool {
    s.blocks().iter().any(|(_, k, _)| extension_block(k))
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn kind_label(kind: &BlockKind) -> String {
    match kind {
        BlockKind::Paragraph => "Paragraph".into(),
        BlockKind::AtxHeading(n) => format!("AtxHeading({n})"),
        BlockKind::SetextHeading(n) => format!("SetextHeading({n})"),
        BlockKind::FencedCode(lang) => format!("FencedCode({})", lang.as_deref().unwrap_or("")),
        BlockKind::IndentedCode => "IndentedCode".into(),
        BlockKind::Math => "Math".into(),
        BlockKind::Mermaid => "Mermaid".into(),
        BlockKind::Table => "Table".into(),
        BlockKind::ListItem { ordered, depth, .. } => format!("ListItem({ordered},{depth})"),
        BlockKind::BlockQuote => "BlockQuote".into(),
        BlockKind::Admonition(t) => format!("Admonition({})", t.label()),
        BlockKind::Callout(c) => format!("Callout({})", c.name),
        BlockKind::ThematicBreak => "ThematicBreak".into(),
        BlockKind::LinkRefDef => "LinkRefDef".into(),
        BlockKind::FootnoteDef => "FootnoteDef".into(),
        BlockKind::HtmlBlock => "HtmlBlock".into(),
        BlockKind::Details { .. } => "Details".into(),
        BlockKind::FrontMatter(_) => "FrontMatter".into(),
    }
}

/// 单文件四维差分。`expected` 为规范期望 HTML（真实文档为 None）。
fn analyze_doc(label: &str, md: &str, expected: Option<&str>, details: &mut Vec<String>) -> Stats {
    let mut st = Stats::default();
    st.files = 1;
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let s = Session::open(md);
        let vb = actual_blocks(&s);
        let rb = pulldown_blocks(md);
        let eb = editor_diff_blocks(&s);
        let refs = ref_blocks(md);
        let render_diff = expected.is_some_and(|exp| {
            let got = export_body(&s);
            canonical(&normalize_newlines(&got)) != canonical(&normalize_newlines(exp))
        });
        (s, vb, rb, eb, refs, render_diff)
    }));
    let (s, vb, rb, mut eb, refs, render_diff) = match outcome {
        Ok(v) => v,
        Err(_) => {
            st.panics = 1;
            details.push(format!("PANIC {label}"));
            return st;
        }
    };

    let extension = has_extension_block(&s);
    if extension {
        st.extension_files = 1;
    }
    if render_diff {
        st.render_diff_files = 1;
        details.push(format!("[render] {label}"));
    }

    if vb != rb {
        st.type_mismatch_files = 1;
        let class = if extension {
            "语法覆盖"
        } else {
            "位置/边界"
        };
        details.push(format!(
            "[type/{class}] {label}: veil=[{}] pd=[{}]",
            short(&vb),
            short(&rb)
        ));
        return st;
    }
    if extension {
        st.semantic_only_files = 1;
        let kinds: Vec<String> = s
            .blocks()
            .into_iter()
            .filter(|(_, k, _)| extension_block(k))
            .map(|(_, k, _)| kind_label(&k))
            .collect();
        details.push(format!(
            "[semantic/语法覆盖] {label}: extension_kinds={kinds:?}"
        ));
        return st;
    }
    st.match_files = 1;

    let mut refs_leaf: Vec<(RKind, ByteRange<usize>)> = refs
        .into_iter()
        .filter(|(k, _)| k.to_b().is_some())
        .collect();
    if eb.len() != refs_leaf.len() {
        st.unaligned_files = 1;
        details.push(format!(
            "[unaligned] {label}: veil_blocks={} pd_blocks={}",
            eb.len(),
            refs_leaf.len()
        ));
        return st;
    }
    st.aligned_files = 1;
    // 两侧按文档顺序对齐；骨架已相等，逐块比较文本与边界。
    while let Some((e, (rk, rr))) = eb.first().zip(refs_leaf.first()) {
        st.blocks_compared += 1;
        let et = s.text_slice(Range::new(e.start, e.end)).unwrap_or_default();
        let rt = md.get(rr.start..rr.end).unwrap_or("");
        let (en, rn) = (norm(&et), norm(rt));
        if en == rn {
            st.text_equal_blocks += 1;
            if e.start == rr.start && e.end == rr.end {
                st.offset_equal_blocks += 1;
            } else {
                st.boundary_notation_blocks += 1;
            }
        } else {
            st.text_diff_blocks += 1;
            if details.len() < 60 {
                details.push(format!(
                    "[text] {label}: veil({})={:?} pd({})={:?}",
                    kind_label(&e.kind),
                    en,
                    format!("{:?}", rk),
                    rn
                ));
            }
        }
        eb.remove(0);
        refs_leaf.remove(0);
    }
    st
}

fn load_index(dir: &Path) -> Vec<(String, String, Option<String>)> {
    let index = std::fs::read_to_string(dir.join("index.tsv")).expect("index.tsv");
    let mut out = Vec::new();
    for line in index.lines() {
        let mut it = line.split('\t');
        let (Some(id), Some(section), Some(_num)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let md = std::fs::read_to_string(dir.join(format!("{id}.md"))).expect("md");
        let html = std::fs::read_to_string(dir.join(format!("{id}.html"))).ok();
        out.push((format!("{id} {section}"), md, html));
    }
    out
}

fn real_corpus() -> Vec<(String, String, Option<String>)> {
    let dir = Path::new(REAL);
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut paths: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n != "index.tsv"))
        .collect();
    paths.sort();
    for path in paths {
        let Ok(md) = std::fs::read_to_string(&path) else {
            continue;
        };
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().to_string());
        out.push((name, md, None));
    }
    out
}

fn run_corpus(
    name: &str,
    docs: &[(String, String, Option<String>)],
    details: &mut Vec<String>,
) -> Stats {
    let mut st = Stats::default();
    for (label, md, expected) in docs {
        let one = analyze_doc(&format!("{name}:{label}"), md, expected.as_deref(), details);
        st.merge(&one);
    }
    println!(
        "CORPUS {name}: files={} panics={} type_mismatch={} semantic_only={} match={} \
         aligned={} unaligned={} blocks={} text_equal={} offset_equal={} boundary_notation={} \
         text_diff={} render_diff={}",
        st.files,
        st.panics,
        st.type_mismatch_files,
        st.semantic_only_files,
        st.match_files,
        st.aligned_files,
        st.unaligned_files,
        st.blocks_compared,
        st.text_equal_blocks,
        st.offset_equal_blocks,
        st.boundary_notation_blocks,
        st.text_diff_blocks,
        st.render_diff_files
    );
    st
}

#[test]
fn parser_differential_corpus() {
    let mut details = Vec::new();
    let mut total = Stats::default();

    let cm = run_corpus("commonmark", &load_index(Path::new(EXAMPLES)), &mut details);
    let gfm = run_corpus("gfm", &load_index(Path::new(GFM)), &mut details);
    let real = run_corpus("real", &real_corpus(), &mut details);
    total.merge(&cm);
    total.merge(&gfm);
    total.merge(&real);

    // CommonMark/GFM 一侧的骨架与文本必须与门禁参照完全一致；real 允许已接受差异。
    let cm_gfm = Stats {
        files: cm.files + gfm.files,
        type_mismatch_files: cm.type_mismatch_files + gfm.type_mismatch_files,
        text_diff_blocks: cm.text_diff_blocks + gfm.text_diff_blocks,
        render_diff_files: cm.render_diff_files + gfm.render_diff_files,
        ..Stats::default()
    };
    println!(
        "TOTAL: files={} panics={} type_mismatch={} extension_files={} text_diff_blocks={} render_diff_files={}",
        total.files,
        total.panics,
        total.type_mismatch_files,
        total.extension_files,
        total.text_diff_blocks,
        total.render_diff_files
    );
    println!("--- mismatch details (first {}) ---", details.len());
    for d in &details {
        println!("{d}");
    }

    assert_eq!(total.files, cm.files + gfm.files + real.files);
    assert_eq!(total.panics, 0, "解析链路不允许 panic");
    assert_eq!(cm.files, 652, "CommonMark 语料规模应为 652");
    assert_eq!(gfm.files, 23, "GFM 语料规模应为 23");
    assert!(real.files > 0, "真实文档语料为空");
    // 分类完整性：每篇文件恰好落入一类。
    assert_eq!(
        total.type_mismatch_files + total.semantic_only_files + total.match_files + total.panics,
        total.files,
        "文件分类不完整"
    );
    assert!(
        cm_gfm.type_mismatch_files <= BASELINE_TYPE_MISMATCH_FILES,
        "CommonMark/GFM 块骨架退化：{} > {}",
        cm_gfm.type_mismatch_files,
        BASELINE_TYPE_MISMATCH_FILES
    );
    assert!(
        real.type_mismatch_files <= BASELINE_REAL_TYPE_MISMATCH_FILES,
        "真实文档块骨架差异退化：{} > {}",
        real.type_mismatch_files,
        BASELINE_REAL_TYPE_MISMATCH_FILES
    );
    assert!(
        total.text_diff_blocks <= BASELINE_TEXT_DIFF_BLOCKS,
        "对齐块文本差异退化：{} > {}",
        total.text_diff_blocks,
        BASELINE_TEXT_DIFF_BLOCKS
    );
    assert!(
        total.render_diff_files <= BASELINE_RENDER_DIFF_FILES,
        "渲染差异退化：{} > {}",
        total.render_diff_files,
        BASELINE_RENDER_DIFF_FILES
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// 块骨架与语义都一致。
    Match,
    /// 一侧独有的语法（编辑器扩展或参照缺失）。
    Coverage,
    /// 同内容的块切口/边界不同。
    Position,
}

fn classify(s: &Session, md: &str) -> Class {
    let vb = actual_blocks(s);
    let rb = pulldown_blocks(md);
    if vb == rb {
        if has_extension_block(s) {
            Class::Coverage
        } else {
            Class::Match
        }
    } else if has_extension_block(s) {
        Class::Coverage
    } else {
        Class::Position
    }
}

/// 对齐块中归一化文本不同的个数（None = 两侧块数不同，无法逐块比较）。
fn aligned_text_diffs(s: &Session, md: &str) -> Option<usize> {
    let eb = editor_diff_blocks(s);
    let refs: Vec<(RKind, ByteRange<usize>)> = ref_blocks(md)
        .into_iter()
        .filter(|(k, _)| k.to_b().is_some())
        .collect();
    if eb.len() != refs.len() {
        return None;
    }
    let mut diffs = 0;
    for (e, (_, rr)) in eb.iter().zip(refs) {
        let et = s.text_slice(Range::new(e.start, e.end)).unwrap_or_default();
        let rt = md.get(rr.start..rr.end).unwrap_or("");
        if norm(&et) != norm(rt) {
            diffs += 1;
        }
    }
    Some(diffs)
}

fn has_kind(s: &Session, pred: impl Fn(&BlockKind) -> bool) -> bool {
    s.blocks().iter().any(|(_, k, _)| pred(k))
}

#[test]
fn parser_differential_feature_samples() {
    let mut rows: Vec<String> = Vec::new();

    // 1. CommonMark 基线：块骨架、文本都一致。
    let md = "# 标题\n\n段落 **粗体** 与 `code` 与 [链接](https://example.com)\n";
    let s = Session::open(md);
    assert_eq!(classify(&s, md), Class::Match);
    assert_eq!(aligned_text_diffs(&s, md), Some(0));
    rows.push(format!(
        "commonmark_basic: class={:?} text_diff={:?}",
        Class::Match,
        aligned_text_diffs(&s, md)
    ));

    // 2. GFM 表格：两侧同为 Table；单元格文本/对齐由编辑器派生数据校验。
    let md = "| 名称 | 值 |\n| :--- | ---: |\n| 中文 | 1 |\n";
    let s = Session::open(md);
    assert_eq!(classify(&s, md), Class::Match);
    let tid = s
        .blocks()
        .into_iter()
        .find_map(|(id, k, _)| (k == BlockKind::Table).then_some(id))
        .expect("表格块");
    let t = s.table(tid).expect("表格派生数据");
    let cells: Vec<&str> = t
        .header
        .iter()
        .chain(t.rows.iter().flatten())
        .map(|c| c.text.as_str())
        .collect();
    assert_eq!(cells, vec!["名称", "值", "中文", "1"]);
    rows.push(format!(
        "gfm_table: class={:?} cells={:?}",
        Class::Match,
        cells
    ));

    // 3. CJK：全角标点/宽字符切片不影响边界与文本归一化。
    let md = "中文段落，含全角标点。\n\n第二段：中文与 English 混排。\n";
    let s = Session::open(md);
    assert_eq!(classify(&s, md), Class::Match);
    assert_eq!(aligned_text_diffs(&s, md), Some(0));
    rows.push(format!(
        "cjk: class={:?} text_diff={:?}",
        Class::Match,
        aligned_text_diffs(&s, md)
    ));

    // 4. Callouts：编辑器是 Admonition/Quote，参照只是普通引用块（语法覆盖）。
    let md = "> [!NOTE]\n> 提示内容\n";
    let s = Session::open(md);
    assert!(has_kind(&s, |k| matches!(
        k,
        BlockKind::Admonition(AdmonitionType::Note)
    )));
    assert_eq!(classify(&s, md), Class::Coverage);
    rows.push(format!("callout_note: class={:?}", classify(&s, md)));

    // 5. 公式：编辑器 Math 块，参照退化为段落（语法覆盖）。
    let md = "$$\nx + y\n$$\n";
    let s = Session::open(md);
    assert!(has_kind(&s, |k| *k == BlockKind::Math));
    assert_eq!(classify(&s, md), Class::Coverage);
    rows.push(format!("math_block: class={:?}", classify(&s, md)));

    // 6. 脚注：编辑器 FootnoteRef + FootnoteDef，参照无脚注语法（语法覆盖）。
    let md = "见脚注[^a]。\n\n[^a]: 注释内容\n";
    let s = Session::open(md);
    assert!(has_kind(&s, |k| *k == BlockKind::FootnoteDef));
    let pid = s
        .blocks()
        .into_iter()
        .find_map(|(id, k, _)| (k == BlockKind::Paragraph).then_some(id))
        .expect("脚注引用段落");
    let inline_has_footnote = s
        .block_inlines(pid)
        .unwrap_or_default()
        .iter()
        .any(|n| n.kind == InlineKind::FootnoteRef);
    assert!(inline_has_footnote, "段落内应有脚注引用节点");
    assert_eq!(classify(&s, md), Class::Coverage);
    rows.push(format!(
        "footnote: class={:?} inline_ref={inline_has_footnote}",
        classify(&s, md)
    ));

    println!("SAMPLE DIFFERENTIAL:");
    for r in &rows {
        println!("  {r}");
    }
    assert_eq!(rows.len(), 6);
}
