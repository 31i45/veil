//! Task 7（Agent G）：屏幕（cosmic-text）与 PDF（本模块自研断行）的一致性差分工具。
//!
//! 只做测量与分类，不改任何生产行为：
//! - 策略表差分：屏幕禁则表从 `veil-ui/src/view/linebreak.rs` 源码读取（唯一真值），
//!   与 PDF 的 `is_line_start_forbidden` / `is_line_end_forbidden` 逐字符比对；
//! - 断点差分：同一语料在 PDF `fill_lines` 与真机 `LayoutEngine` 里的行首偏移比较；
//! - 分页契约：PDF 固定行高、页边界、表格跨页只按行切分。
//!
//! 屏幕侧需要系统字体才能成形，因此凡是「字形 advance / 字体回退」引起的差异都记为
//! 渲染层差异，不做数值断言；策略层差异（禁则表、断行点）才做断言。

use std::collections::HashSet;

use veil_core::{BlockKind, Range};
use veil_ui::{LayoutEngine, BASE_FONT_SIZE};

use super::super::*;
use super::FakeMetrics;

// =============================================================================
// 屏幕策略表：从 veil-ui 源码解析（不复制常量，防止两边漂移）
// =============================================================================

fn veil_ui_linebreak_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("veil-ui")
        .join("src")
        .join("view")
        .join("linebreak.rs");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取屏幕禁则表失败 {}：{error}", path.display()))
}

/// 解析 `const NAME: &[char] = &[ ... ];` 里的字符字面量（支持 `'\u{XXXX}'` 与 `'x'`）。
fn parse_char_array(src: &str, name: &str) -> Vec<char> {
    let at = src
        .find(name)
        .unwrap_or_else(|| panic!("屏幕源码里找不到 {name}"));
    let rest = &src[at..];
    let open = rest.find("= &[").expect("数组起始") + 4;
    let close = rest.find("];").expect("数组结束");
    let body = &rest[open..close];

    let mut out = Vec::new();
    let mut it = body.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\'' {
            continue;
        }
        let mut literal = String::new();
        while let Some(&next) = it.peek() {
            it.next();
            if next == '\'' {
                break;
            }
            if next == '\\' {
                match it.next() {
                    Some('u') => {
                        let mut hex = String::new();
                        for c in it.by_ref() {
                            if c == '{' {
                                continue;
                            }
                            if c == '}' {
                                break;
                            }
                            hex.push(c);
                        }
                        let code = u32::from_str_radix(&hex, 16).expect("\\u{..} 十六进制");
                        literal.push(char::from_u32(code).expect("合法码点"));
                    }
                    Some('n') => literal.push('\n'),
                    Some('t') => literal.push('\t'),
                    Some('r') => literal.push('\r'),
                    Some('0') => literal.push('\0'),
                    Some(other) => literal.push(other),
                    None => break,
                }
            } else {
                literal.push(next);
            }
        }
        if let Some(ch) = literal.chars().next() {
            assert_eq!(
                literal.chars().count(),
                1,
                "屏幕表出现多字符字面量：{literal:?}"
            );
            out.push(ch);
        }
    }
    out
}

struct ScreenTables {
    start: HashSet<char>,
    end: HashSet<char>,
}

fn screen_tables() -> ScreenTables {
    let src = veil_ui_linebreak_source();
    let start = parse_char_array(&src, "LINE_START_FORBIDDEN")
        .into_iter()
        .collect::<HashSet<_>>();
    let end = parse_char_array(&src, "LINE_END_FORBIDDEN")
        .into_iter()
        .collect::<HashSet<_>>();
    assert!(!start.is_empty() && !end.is_empty(), "屏幕禁则表解析为空");
    ScreenTables { start, end }
}

/// 比对用的候选字符集：两边表成员 + 全部可打印 ASCII + 常见全角/中文标点。
fn candidate_alphabet(t: &ScreenTables) -> Vec<char> {
    let mut all: Vec<char> = t.start.iter().chain(t.end.iter()).copied().collect();
    all.extend((0x20u8..=0x7E).map(char::from));
    all.extend([
        '　', '、', '。', '，', '．', '；', '：', '？', '！', '）', '〕', '］', '｝', '〉', '》',
        '」', '』', '】', '”', '’', '…', '‥', '·', '—', '～', '（', '〔', '［', '｛', '〈', '《',
        '「', '『', '【', '“', '‘', '〗',
    ]);
    all.sort_unstable();
    all.dedup();
    all
}

fn sorted(mut v: Vec<char>) -> Vec<char> {
    v.sort_unstable();
    v
}

// =============================================================================
// 线装配：PDF 用生产函数，屏幕用真机 LayoutEngine
// =============================================================================

/// PDF 侧：与 `Ctx::rich` 相同的入口（纯文本段落 → 逐字 LItem → fill_lines）。
fn pdf_line_texts(text: &str, width_em: f32) -> Vec<String> {
    let style = Style::body();
    let items: Vec<LItem> = text.chars().map(|c| LItem::Ch(c, style)).collect();
    fill_lines(items, width_em * BODY_SIZE, &FakeMetrics)
        .into_iter()
        .map(|line| {
            line.into_iter()
                .filter_map(|it| match it {
                    LItem::Ch(c, _) => Some(c),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// 屏幕侧：真机 cosmic-text 排版，返回 (行文本, 行首文档字节偏移)。
fn screen_line_texts(
    engine: &mut LayoutEngine,
    text: &str,
    width_px: f32,
) -> (Vec<String>, Vec<usize>) {
    let layout = engine.layout_block(
        BlockKind::Paragraph,
        text,
        Range::new(0, text.len()),
        &[],
        width_px,
        usize::MAX,
        &[],
        false,
    );
    let tops = engine.line_tops(&layout);
    let starts: Vec<usize> = tops
        .iter()
        .map(|&top| engine.hit(&layout, 0.0, top + 1.0).min(text.len()))
        .collect();
    let mut lines = Vec::new();
    for (i, &start) in starts.iter().enumerate() {
        let end = starts
            .get(i + 1)
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        let (s, e) = (start.min(end), end);
        lines.push(text[s..e].to_string());
    }
    (lines, starts)
}

/// 去掉 WJ 与空白后的行内容：跨表面比较断点位置时忽略空格归属差异。
fn content(line: &str) -> String {
    line.chars()
        .filter(|&c| c != '\u{2060}' && !c.is_whitespace())
        .collect()
}

fn visible_edges(line: &str) -> (Option<char>, Option<char>) {
    let chars: Vec<char> = content(line).chars().collect();
    (chars.first().copied(), chars.last().copied())
}

/// 无空格语料：PDF 行首字符下标（fill_lines 丢弃行首空格，故用可见内容累计）。
fn visible_char_starts(lines: &[String]) -> Vec<usize> {
    let mut out = Vec::with_capacity(lines.len().saturating_sub(1));
    let mut acc = 0usize;
    for line in lines.iter().take(lines.len().saturating_sub(1)) {
        acc += content(line).chars().count();
        out.push(acc);
    }
    out
}

// =============================================================================
// PDF 文档级采样
// =============================================================================

fn lay_plain(md: &str) -> DocLayout {
    let session = Session::open(md);
    layout_document(
        &session,
        &FakeMetrics,
        PageGeom::a4(20.0),
        LayoutOpts::default(),
        None,
        &HashMap::new(),
    )
}

/// 按基线 y 聚行（0.5pt 容差），再按 x 排序拼回每页的文本行。
fn pdf_page_lines(doc: &DocLayout) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (page_idx, page) in doc.pages.iter().enumerate() {
        let mut runs: Vec<(f32, f32, &str)> = page
            .iter()
            .filter_map(|op| match op {
                DrawOp::Text(t) => Some((t.y, t.x, t.text.as_str())),
                _ => None,
            })
            .collect();
        runs.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap()
                .then(a.1.partial_cmp(&b.1).unwrap())
        });
        let mut line = String::new();
        let mut line_y: Option<f32> = None;
        for (y, _x, text) in runs {
            if let Some(cy) = line_y {
                if (y - cy).abs() > 0.5 {
                    out.push((page_idx, std::mem::take(&mut line)));
                    line_y = Some(y);
                }
            } else {
                line_y = Some(y);
            }
            line.push_str(text);
        }
        if !line.is_empty() {
            out.push((page_idx, line));
        }
    }
    out
}

// =============================================================================
// 1) 禁则表差分
// =============================================================================

#[test]
fn kinoksu_tables_differ_only_in_documented_entries() {
    let screen = screen_tables();
    let alphabet = candidate_alphabet(&screen);

    let pdf_start: Vec<char> = alphabet
        .iter()
        .copied()
        .filter(|&c| is_line_start_forbidden(c))
        .collect();
    let pdf_end: Vec<char> = alphabet
        .iter()
        .copied()
        .filter(|&c| is_line_end_forbidden(c))
        .collect();

    let pdf_only_start = sorted(
        pdf_start
            .iter()
            .copied()
            .filter(|c| !screen.start.contains(c))
            .collect(),
    );
    let screen_only_start = sorted(
        screen
            .start
            .iter()
            .copied()
            .filter(|&c| !is_line_start_forbidden(c))
            .collect(),
    );
    let pdf_only_end = sorted(
        pdf_end
            .iter()
            .copied()
            .filter(|c| !screen.end.contains(c))
            .collect(),
    );
    let screen_only_end = sorted(
        screen
            .end
            .iter()
            .copied()
            .filter(|&c| !is_line_end_forbidden(c))
            .collect(),
    );

    eprintln!(
        "[layout] 行首禁则：screen={} pdf={}；PDF 独有={pdf_only_start:?}；屏幕独有={screen_only_start:?}",
        screen.start.len(),
        pdf_start.len()
    );
    eprintln!(
        "[layout] 行尾禁则：screen={} pdf={}；PDF 独有={pdf_only_end:?}；屏幕独有={screen_only_end:?}",
        screen.end.len(),
        pdf_end.len()
    );

    // 屏幕表：GB/T 15834 通行集（24 个行首 / 11 个行尾）。
    assert_eq!(screen.start.len(), 24, "屏幕行首表规模变化，需同步本测试");
    assert_eq!(screen.end.len(), 11, "屏幕行尾表规模变化，需同步本测试");

    // 已知差异（记录在 layout-consistency-2026-11-26.md）。
    assert_eq!(
        pdf_only_start,
        vec!['!', ')', ',', '.', ':', ';', '?', ']', '}'],
        "PDF 额外的行首禁排（半角标点）集合变化"
    );
    assert_eq!(
        screen_only_start,
        vec!['—', '～'],
        "屏幕独占的行首禁排（破折号/波浪号）集合变化"
    );
    assert_eq!(
        pdf_only_end,
        vec!['(', '[', '{'],
        "PDF 额外的行尾禁排（半角开符号）集合变化"
    );
    assert!(screen_only_end.is_empty(), "行尾禁则不应有屏幕独占项");

    // 共享核心：中日文闭符号/开符号必须在两边都禁排。
    for c in ['。', '，', '、', '）', '】', '」', '》', '〕'] {
        assert!(
            screen.start.contains(&c) && is_line_start_forbidden(c),
            "{c:?}"
        );
    }
    for c in ['（', '【', '「', '《', '〔', '“'] {
        assert!(screen.end.contains(&c) && is_line_end_forbidden(c), "{c:?}");
    }
}

// =============================================================================
// 2) 共享策略下断点一致；PDF 半角标点另有一套
// =============================================================================

/// 屏幕版心宽度（em）与 PDF 版心宽度（em）相等时的屏幕像素宽。
/// PDF：A4(595.28pt) - 2×20mm；正文 11pt。屏幕正文 16px。
fn pdf_content_em() -> f32 {
    PageGeom::a4(20.0).content_w() / BODY_SIZE
}

#[test]
fn shared_kinsoku_policy_breaks_agree_at_normalized_capacity() {
    let em = pdf_content_em();
    let mut engine = LayoutEngine::new();

    // 纯 CJK：两边都按版心 em 容量贪心断行，断点应完全一致。
    for (name, text) in [
        ("cjk_plain", "汉".repeat(300)),
        ("cjk_close_punct", ("甲".repeat(42) + "。").repeat(5)),
        (
            "cjk_open_bracket",
            format!("{}后", ("乙".repeat(42) + "（").repeat(5)),
        ),
    ] {
        let pdf = pdf_line_texts(&text, em);
        let (screen, _) = screen_line_texts(&mut engine, &text, em * BASE_FONT_SIZE);

        assert_eq!(
            visible_char_starts(&pdf),
            visible_char_starts(&screen),
            "{name}: 共享禁则下断点应一致\nPDF={pdf:?}\nSCREEN={screen:?}"
        );
        assert_eq!(
            pdf.iter().map(|l| content(l)).collect::<Vec<_>>(),
            screen.iter().map(|l| content(l)).collect::<Vec<_>>(),
            "{name}: 行内容应一致"
        );
        assert_eq!(
            content(&pdf.concat()),
            content(&text),
            "{name}: PDF 不得丢字或多字"
        );
        assert_eq!(
            content(&screen.concat()),
            content(&text),
            "{name}: 屏幕同上"
        );
    }
}

#[test]
fn pdf_rejects_halfwidth_punctuation_at_line_start() {
    // 43 个「丁」约满一行；再接半角标点。PDF 的禁则表含 '.'，会把末字一起下移，
    // 因此首行只剩 42 字，而不是让 '.' 落在下一行行首。
    let em = pdf_content_em();
    let text = format!("{}...ABC", "丁".repeat(43));
    let pdf = pdf_line_texts(&text, em);
    assert!(pdf.len() >= 2, "{pdf:?}");
    assert_eq!(
        content(&pdf[0]).chars().count(),
        42,
        "PDF 应把末字与半角标点一起下移：{pdf:?}"
    );
    for line in pdf.iter().skip(1) {
        assert!(
            !content(line).starts_with('.'),
            "PDF 行首不得出现半角标点：{pdf:?}"
        );
    }
    assert!(content(&pdf[1]).contains("..."), "{pdf:?}");
    assert_eq!(content(&pdf.concat()), content(&text));
}

/// 屏幕表把破折号列为行首禁排（前置 WJ），PDF 表没有；同容量下两者应表现出差异。
#[test]
fn em_dash_is_glued_on_screen_but_allowed_at_pdf_line_start() {
    let em = pdf_content_em();
    let text = format!("{}—{}", "甲".repeat(43), "乙".repeat(10));

    let pdf = pdf_line_texts(&text, em);
    assert!(pdf.len() >= 2, "{pdf:?}");
    assert!(
        content(&pdf[1]).starts_with('—'),
        "PDF 表未收录破折号，应允许其出现在行首：{pdf:?}"
    );

    let mut engine = LayoutEngine::new();
    let (screen, _) = screen_line_texts(&mut engine, &text, em * BASE_FONT_SIZE);
    assert_eq!(
        content(&screen[0]).chars().count(),
        42,
        "屏幕破折号前置 WJ，应把前一字一起下移：{screen:?}"
    );
    for line in &screen {
        assert!(
            !content(line).starts_with('—'),
            "屏幕不得让破折号出现在行首：{screen:?}"
        );
    }
    assert_eq!(content(&pdf.concat()), content(&text));
    assert_eq!(content(&screen.concat()), content(&text));
}

// =============================================================================
// 3) 英文长词：两边都必须硬断且不丢字
// =============================================================================
#[test]
fn long_english_word_hard_breaks_without_loss_on_both_surfaces() {
    let word = "x".repeat(200);

    let pdf = pdf_line_texts(&word, 60.0);
    assert!(pdf.len() > 1, "PDF 超长 ASCII 词必须硬断");
    assert!(pdf
        .iter()
        .all(|l| FakeMetrics.text_width(l, BODY_SIZE) <= 60.0 * BODY_SIZE + 0.01));
    assert_eq!(pdf.concat(), word, "PDF 硬断不得丢字");

    let mut engine = LayoutEngine::new();
    let (screen, _) = screen_line_texts(&mut engine, &word, 60.0 * BASE_FONT_SIZE);
    assert!(screen.len() > 1, "屏幕超长 ASCII 词必须逐字形兜底");
    assert_eq!(content(&screen.concat()), word, "屏幕硬断不得丢字");
}

// =============================================================================
// 4) 分页契约：固定行高、页边界、表格跨页只按行切
// =============================================================================

fn line_height() -> f32 {
    FakeMetrics.line_height(BODY_SIZE)
}

#[test]
fn pdf_pagination_uses_fixed_line_height_and_stays_inside_margins() {
    let doc = lay_plain(&"汉".repeat(3000));
    let geom = PageGeom::a4(20.0);
    let right = geom.margin_x + geom.content_w();
    assert!(doc.pages.len() >= 2, "长段落应分页：{}", doc.pages.len());

    for page in &doc.pages {
        let mut ys: Vec<f32> = page
            .iter()
            .filter_map(|op| match op {
                DrawOp::Text(t) => Some(t.y),
                _ => None,
            })
            .collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ys.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        for pair in ys.windows(2) {
            assert!(
                (pair[1] - pair[0] - line_height()).abs() < 0.01,
                "正文行距应恒为 {}：{:?}",
                line_height(),
                pair
            );
        }
        for op in page {
            if let DrawOp::Text(t) = op {
                let w = FakeMetrics.text_width(&t.text, t.size);
                assert!(t.x >= geom.margin_x - 0.5, "越过左页边距：{}", t.x);
                assert!(t.x + w <= right + 0.5, "越过右页边距：{t:?}");
                assert!(
                    t.y >= geom.margin_y - 0.5 && t.y <= geom.height - geom.margin_y + 0.5,
                    "基线越出正文区：{t:?}"
                );
            }
        }
    }
}

#[test]
fn tall_table_paginates_by_rows_without_leaving_the_content_box() {
    let cell = "表".repeat(4000);
    let md = format!("| 列 |\n| --- |\n| {cell} |\n");
    let doc = lay_plain(&md);
    let geom = PageGeom::a4(20.0);
    let right = geom.margin_x + geom.content_w();
    assert!(
        doc.pages.len() >= 2,
        "超长单元格应跨页：{}",
        doc.pages.len()
    );

    let mut text_chars = 0usize;
    for page in &doc.pages {
        let mut texts = 0usize;
        for op in page {
            if let DrawOp::Text(t) = op {
                texts += 1;
                let w = FakeMetrics.text_width(&t.text, t.size);
                assert!(t.x >= geom.margin_x - 0.5, "单元格越过左边距：{}", t.x);
                assert!(t.x + w <= right + 0.5, "单元格越过右边距：{t:?}");
                assert!(
                    t.y >= geom.margin_y - 0.5 && t.y <= geom.height - geom.margin_y + 0.5,
                    "单元格基线越出正文区：{t:?}"
                );
                text_chars += t.text.chars().count();
            }
        }
        assert!(texts > 0, "跨页后每页都应有表格文本");
    }
    assert!(
        text_chars >= 4000,
        "跨页续排不得截断单元格：只排了 {text_chars} 字"
    );
}

// =============================================================================
// 5) 产品宽度下的端到端差分摘要（记录用，断言只守不变量）
// =============================================================================

#[test]
fn product_width_break_difference_summary() {
    let screen_px = 40.0 * BASE_FONT_SIZE; // Standard 档可读列宽上限 40 全角
    let mut engine = LayoutEngine::new();
    let screen = screen_tables();

    let corpora: [(&str, String); 4] = [
        ("cjk", "汉".repeat(120)),
        ("cjk_punct", ("甲".repeat(42) + "。").repeat(3)),
        (
            "mixed",
            format!("{} ascii words here {}", "中".repeat(20), "文".repeat(20)),
        ),
        ("longword", "x".repeat(120)),
    ];

    let mut total_diff = 0usize;
    for (name, text) in corpora {
        let doc = lay_plain(&text);
        let pdf_lines: Vec<String> = pdf_page_lines(&doc).into_iter().map(|(_, l)| l).collect();
        let (screen_lines, _) = screen_line_texts(&mut engine, &text, screen_px);

        // 不变量：两侧都不丢字。
        assert_eq!(
            content(&pdf_lines.concat()),
            content(&text),
            "{name}: PDF 丢字"
        );
        assert_eq!(
            content(&screen_lines.concat()),
            content(&text),
            "{name}: 屏幕丢字"
        );

        // 不变量：两侧各自的行首/行尾不违反自己的禁则表。
        for line in pdf_lines.iter().skip(1) {
            if let (Some(c), _) = visible_edges(line) {
                assert!(!is_line_start_forbidden(c), "{name}: PDF 行首禁则 {c:?}");
            }
        }
        for (i, line) in screen_lines.iter().enumerate() {
            let (first, last) = visible_edges(line);
            if i > 0 {
                if let Some(c) = first {
                    assert!(!screen.start.contains(&c), "{name}: 屏幕行首禁则 {c:?}");
                }
            }
            if i + 1 < screen_lines.len() {
                if let Some(c) = last {
                    assert!(!screen.end.contains(&c), "{name}: 屏幕行尾禁则 {c:?}");
                }
            }
        }

        let pdf_text: Vec<String> = pdf_lines.iter().map(|l| content(l)).collect();
        let screen_text: Vec<String> = screen_lines.iter().map(|l| content(l)).collect();
        let same = pdf_text
            .iter()
            .zip(&screen_text)
            .filter(|(a, b)| a == b)
            .count();
        let diff = pdf_text.len().max(screen_text.len()) - same;
        total_diff += diff;
        eprintln!(
            "[layout] {name}: pdf_lines={} screen_lines={} diff_lines={diff}",
            pdf_text.len(),
            screen_text.len()
        );
        eprintln!("[layout]   pdf   ={:?}", pdf_text);
        eprintln!("[layout]   screen={:?}", screen_text);
    }

    // 容量差：同一版心物理宽度下，屏幕 40 全角 / PDF 43 全角 → 断点必然不同。
    let cjk = "汉".repeat(120);
    let pdf = pdf_line_texts(&cjk, pdf_content_em());
    assert_eq!(content(&pdf[0]).chars().count(), 43, "PDF 每行 43 全角");
    let (screen_cjk, _) = screen_line_texts(&mut engine, &cjk, screen_px);
    assert_eq!(
        content(&screen_cjk[0]).chars().count(),
        40,
        "屏幕每行 40 全角"
    );

    eprintln!("[layout] 总差异行数（产品宽度，规范化后逐行比较）：{total_diff}");
}
