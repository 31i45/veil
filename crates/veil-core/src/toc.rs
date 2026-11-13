//! 文档目录（Table of Contents）生成与 `[TOC]` 标记检测。纯函数，零 GUI 依赖。
//!
//! - [`generate_toc`] 从 (级别, 可见文本) 流生成 Markdown 无序列表；
//! - [`is_toc_marker`] 判定段落块的 trim 内容是否为 TOC 锚点标记。

use crate::slug::{heading_slug, heading_slugs};

/// 检查一个段落块的 trim 后内容是否为 TOC 标记。
/// 接受 `[TOC]`、`[toc]`、`[[TOC]]`、`[[toc]]`（大小写不敏感，trim 后精确匹配）。
pub fn is_toc_marker(text: &str) -> bool {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    lower == "[toc]" || lower == "[[toc]]"
}

/// 生成 Markdown 无序列表形式的 TOC 文本。
///
/// # 参数
/// `headings`：`(level: u8, visible_text: &str)` 迭代器；level 预期为 1–6。
///
/// # 规则
/// - 跳过可见文本为空或纯标点/符号的条目（其 slug 为空）；
/// - 使用 [`heading_slugs`] 进行全局去重（`a`, `a-1`, `a-2` …）；
/// - 嵌套缩进 = 每级 3 空格（1 级无缩进，2 级 3 空格，3 级 6 空格 …）；
/// - 列表项统一用 `-`：排版层按源码列表标记显隐，不做有序列表自动编号，
///   写 `1.` 会让每个条目都显示 `1.`（`generate_toc` 的输出是源码文本，不是渲染结果）；
/// - 可见文本中的 `[` / `]` 在链接展示部分反斜杠转义（锚点 slug 不变）；
/// - 空迭代器 → 返回空字符串。
pub fn generate_toc<'a>(headings: impl Iterator<Item = (u8, &'a str)>) -> String {
    let items: Vec<(u8, &'a str)> = headings.collect();
    if items.is_empty() {
        return String::new();
    }

    // 过滤掉可见文本全空 / 全符号（slug 为空）的条目
    let kept: Vec<(u8, &'a str)> = items
        .into_iter()
        .filter(|(_, t)| !t.is_empty() && !heading_slug(t).is_empty())
        .collect();
    if kept.is_empty() {
        return String::new();
    }

    // 全局去重 slug
    let slugs = heading_slugs(kept.iter().map(|(_, t)| *t));
    debug_assert_eq!(slugs.len(), kept.len());

    // 维护标题层级栈以正确处理跳跃（h1 → h3 视为 h3 直接嵌套在 h1 下）：
    // 遇到 level 时 pop 栈顶直到栈顶 level < 当前 level，然后 push 当前 level；
    // 缩进 = (栈深度 - 1) * 3 空格。
    let mut levels: Vec<u8> = Vec::new();
    let mut out = String::new();
    for ((level, text), slug) in kept.iter().zip(slugs.iter()) {
        let effective_level = level.max(&1);
        while let Some(&top) = levels.last() {
            if top >= *effective_level {
                levels.pop();
            } else {
                break;
            }
        }
        let indent_spaces = 3usize.saturating_mul(levels.len());
        levels.push(*effective_level);
        out.push_str(&" ".repeat(indent_spaces));
        out.push_str("- [");
        out.push_str(&escape_brackets(text));
        out.push_str("](#");
        out.push_str(slug);
        out.push(')');
        out.push('\n');
    }

    // 去掉末尾多余的换行（保留单个 \n 让后续段落与 TOC 之间有分隔）
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

/// 只转义 Markdown 链接显示文本中的 `[` 与 `]`，避免破坏链接语法。
fn escape_brackets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '[' => out.push_str("\\["),
            ']' => out.push_str("\\]"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_headings_yield_empty() {
        assert_eq!(generate_toc(std::iter::empty::<(u8, &str)>()), "");
    }

    #[test]
    fn single_h1() {
        let got = generate_toc(vec![(1, "Intro")].into_iter());
        assert_eq!(got, "- [Intro](#intro)");
    }

    #[test]
    fn nested_h1_h2_h3() {
        let got = generate_toc(vec![(1, "Top"), (2, "Mid"), (3, "Deep")].into_iter());
        assert_eq!(
            got,
            "- [Top](#top)\n   - [Mid](#mid)\n      - [Deep](#deep)"
        );
    }

    #[test]
    fn duplicate_slugs_deduped() {
        let got = generate_toc(vec![(1, "A"), (2, "A"), (3, "A")].into_iter());
        assert_eq!(got, "- [A](#a)\n   - [A](#a-1)\n      - [A](#a-2)");
    }

    #[test]
    fn empty_visible_text_skipped() {
        let got = generate_toc(vec![(1, ""), (2, "!!!"), (3, "Real")].into_iter());
        assert_eq!(got, "- [Real](#real)");
    }

    #[test]
    fn mixed_levels_with_gaps() {
        // h1 → h3（跳过 h2）：h3 仍在 h1 下缩进 3 空格
        let got = generate_toc(vec![(1, "A"), (3, "B")].into_iter());
        assert_eq!(got, "- [A](#a)\n   - [B](#b)");
    }

    #[test]
    fn gap_after_first_nests_by_depth_not_level() {
        // 从 h2 开始：h2 入栈 → indent=0；h4 作为 h2 的子级 → indent=3
        let got = generate_toc(vec![(2, "A"), (4, "B")].into_iter());
        assert_eq!(got, "- [A](#a)\n   - [B](#b)");
    }

    #[test]
    fn level_drop_pops_stack() {
        // h1→h2→h3→h2：最后一个 h2 会 pop 掉 h3 和中间的 h2，push 新 h2 → indent=3
        let got = generate_toc(vec![(1, "A"), (2, "B"), (3, "C"), (2, "D")].into_iter());
        assert_eq!(
            got,
            "- [A](#a)\n   - [B](#b)\n      - [C](#c)\n   - [D](#d)"
        );
    }

    #[test]
    fn brackets_in_heading_text_are_escaped_in_display_only() {
        let got = generate_toc(vec![(1, "Hea[ding]")].into_iter());
        assert_eq!(got, r#"- [Hea\[ding\]](#heading)"#);
    }

    #[test]
    fn nested_bracket_headings_escaped_and_slugs_unaffected() {
        // 多个含 `[`/`]` 的标题：只转义链接展示文本，锚点 slug 仍按原文生成/去重。
        let got = generate_toc(vec![(1, "[A]"), (2, "B [C]"), (3, "[A]")].into_iter());
        assert_eq!(
            got,
            r#"- [\[A\]](#a)
   - [B \[C\]](#b-c)
      - [\[A\]](#a-1)"#
        );
    }

    #[test]
    fn blank_then_real_toc() {
        // 纯符号 + 空字符串全被过滤 → 但如果最终仍为 0 条就返回空
        let got = generate_toc(vec![(1, "!!!"), (2, "")].into_iter());
        assert_eq!(got, "");
    }

    #[test]
    fn toc_marker_positives() {
        assert!(is_toc_marker("[TOC]"));
        assert!(is_toc_marker("[toc]"));
        assert!(is_toc_marker("[ToC]"));
        assert!(is_toc_marker("[[toc]]"));
        assert!(is_toc_marker("[[TOC]]"));
        assert!(is_toc_marker("  [toc]  "));
    }

    #[test]
    fn toc_marker_negatives() {
        assert!(!is_toc_marker("[TOC](url)"));
        assert!(!is_toc_marker("[TOCX]"));
        assert!(!is_toc_marker("[[toc]"));
        assert!(!is_toc_marker(""));
        assert!(!is_toc_marker("toc"));
        assert!(!is_toc_marker("X[TOC]"));
    }

    #[test]
    fn setext_heading_level_two() {
        let got = generate_toc(vec![(1, "Top"), (2, "Setext H2")].into_iter());
        assert_eq!(got, "- [Top](#top)\n   - [Setext H2](#setext-h2)");
    }
}
