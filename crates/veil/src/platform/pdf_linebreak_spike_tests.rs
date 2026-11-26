//! Task 9（Agent I）：PDF 断行候选组件 test-first spike。
//!
//! 只做对比测量，不进入生产路径：
//! - 当前策略：`fill_lines` + `kinsoku_break`（本模块自研贪心 + 简化禁则）；
//! - 候选：`unicode-linebreak` 0.1.5（UAX #14 断点机会），仅 dev-dependency；
//! - `icu_segmenter` 本阶段不引入，理由见 `docs/review/evidence/spike/pdf-linebreak.md`。
//!
//! 语料沿用 Task 7（`layout-consistency-2026-11-26.md`）的 CJK / 标点 / 半角 /
//! 中西混排 / 超长词 / 表格单元格集合，容量统一用 PDF 正文版心 em
//! （A4 − 2×20mm，11pt），使差异只来自断行策略本身。
//!
//! 复现：`cargo test -p veil pdf_linebreak_spike -- --nocapture`

use std::collections::HashMap;

use unicode_linebreak::{linebreaks, BreakOpportunity};

use super::super::*;
use super::FakeMetrics;

/// 去掉空白后的可见内容：比较断点位置时忽略空格归属。
fn content(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 当前生产策略：纯文本段落 → 逐字 `LItem` → `fill_lines`。
fn current_lines(text: &str, width_em: f32) -> Vec<String> {
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

/// 候选策略：UAX #14 断点机会 + 同一 `FakeMetrics` 字宽的贪心装行。
///
/// 仅用于 spike 比较：候选只处理纯文本，不处理链接标注/图片盒，生产集成成本另计。
fn candidate_lines(text: &str, max_w: f32) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut byte_to_char = HashMap::new();
    for (index, (byte, _)) in text.char_indices().enumerate() {
        byte_to_char.insert(byte, index);
    }
    byte_to_char.insert(text.len(), n);

    let mut allowed = vec![false; n + 1];
    let mut mandatory = vec![false; n + 1];
    for (byte, opportunity) in linebreaks(text) {
        let index = *byte_to_char
            .get(&byte)
            .unwrap_or_else(|| panic!("UAX 断点 {byte} 不在字符边界：{text:?}"));
        match opportunity {
            BreakOpportunity::Allowed => allowed[index] = true,
            BreakOpportunity::Mandatory => mandatory[index] = true,
        }
    }

    let mut lines = Vec::new();
    let mut start = 0usize;
    while start < n {
        // 本行能装下的字符上限（至少一个字，避免容量小于单字时死循环）。
        let mut width = 0.0f32;
        let mut end = start;
        while end < n {
            let char_w = FakeMetrics.width_of(chars[end], BODY_SIZE);
            if end > start && width + char_w > max_w {
                break;
            }
            width += char_w;
            end += 1;
        }

        // 强制断点（硬换行/文末）优先；否则取容量内最后一个允许断点；都没有就硬断。
        let first_mandatory = (start + 1..=n).find(|&p| mandatory[p]);
        let cut = match first_mandatory {
            Some(p) if p <= end => p,
            _ => (start + 1..=end)
                .rev()
                .find(|&p| allowed[p])
                .unwrap_or(end),
        };

        let mut line: String = chars[start..cut].iter().collect();
        line.retain(|c| c != '\n' && c != '\r');
        lines.push(line.trim_start_matches(' ').to_string());

        start = cut;
        while start < n && chars[start] == ' ' {
            start += 1;
        }
    }
    lines
}

/// 行首禁则的统一目标：PDF 自研表 ∪ 屏幕表独有项（`—` `～`），
/// 与 Task 7 的禁则差分结果一致。
fn forbidden_line_start_unified(c: char) -> bool {
    is_line_start_forbidden(c) || matches!(c, '—' | '～')
}

fn start_violations(lines: &[String], forbidden: impl Fn(char) -> bool) -> Vec<(usize, char)> {
    lines
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(i, line)| {
            content(line)
                .chars()
                .next()
                .filter(|&c| forbidden(c))
                .map(|c| (i, c))
        })
        .collect()
}

fn line_text_diff(a: &[String], b: &[String]) -> usize {
    let n = a.len().max(b.len());
    (0..n)
        .filter(|&i| {
            a.get(i).map(|s| content(s)) != b.get(i).map(|s| content(s))
        })
        .count()
}

fn visible_char_starts(lines: &[String]) -> Vec<usize> {
    let mut acc = 0usize;
    let mut out = Vec::new();
    for line in lines.iter().take(lines.len().saturating_sub(1)) {
        acc += content(line).chars().count();
        out.push(acc);
    }
    out
}

fn width_em(line: &str) -> f32 {
    FakeMetrics.text_width(line, BODY_SIZE) / BODY_SIZE
}

fn overflow_lines(lines: &[String], capacity_em: f32) -> usize {
    lines
        .iter()
        .filter(|line| width_em(line) > capacity_em + 0.01)
        .count()
}

/// Task 7 语料（CJK / 闭符号 / 开符号 / 半角 / 破折号 / 混排 / 超长词 / 表格单元格）。
fn corpora() -> Vec<(&'static str, String)> {
    vec![
        ("cjk", "汉".repeat(120)),
        ("cjk_punct", ("甲".repeat(42) + "。").repeat(3)),
        (
            "cjk_open",
            format!("{}后", ("乙".repeat(42) + "（").repeat(5)),
        ),
        ("halfwidth", format!("{}...ABC", "丁".repeat(43))),
        (
            "em_dash",
            format!("{}—{}", "甲".repeat(43), "乙".repeat(10)),
        ),
        (
            "mixed",
            format!("{} ascii words here {}", "中".repeat(20), "文".repeat(20)),
        ),
        ("longword", "x".repeat(200)),
        ("table_cell", "表".repeat(4000)),
    ]
}

#[test]
fn pdf_linebreak_spike_compare_current_and_unicode_linebreak() {
    let em = PageGeom::a4(20.0).content_w() / BODY_SIZE;
    let mut total_diff = 0usize;

    for (name, text) in corpora() {
        let current = current_lines(&text, em);
        let candidate = candidate_lines(&text, em * BODY_SIZE);

        // 不变量：两侧都不丢字。
        assert_eq!(
            content(&current.concat()),
            content(&text),
            "{name}: 当前策略丢字"
        );
        assert_eq!(
            content(&candidate.concat()),
            content(&text),
            "{name}: 候选丢字"
        );
        // 不变量：两侧都不越出版心（超长不可断串由硬断处理）。
        assert_eq!(overflow_lines(&current, em), 0, "{name}: 当前策略行宽溢出");
        assert_eq!(
            overflow_lines(&candidate, em),
            0,
            "{name}: 候选行宽溢出"
        );

        let current_starts = visible_char_starts(&current);
        let candidate_starts = visible_char_starts(&candidate);
        let diff = line_text_diff(&current, &candidate);
        total_diff += diff;

        let current_viol = start_violations(&current, forbidden_line_start_unified);
        let candidate_viol = start_violations(&candidate, forbidden_line_start_unified);
        let max_current = current.iter().map(|l| width_em(l)).fold(0.0f32, f32::max);
        let max_candidate = candidate.iter().map(|l| width_em(l)).fold(0.0f32, f32::max);

        eprintln!(
            "[spike] {name}: lines current={} candidate={} diff={diff}",
            current.len(),
            candidate.len()
        );
        eprintln!(
            "[spike]   starts current={current_starts:?} candidate={candidate_starts:?}"
        );
        eprintln!(
            "[spike]   max_width_em current={max_current:.2} candidate={max_candidate:.2} capacity={em:.2}"
        );
        eprintln!(
            "[spike]   line_start_violations current={current_viol:?} candidate={candidate_viol:?}"
        );
        if diff > 0 {
            eprintln!("[spike]   current  ={current:?}");
            eprintln!("[spike]   candidate={candidate:?}");
        }

        // 半角 `.`：当前 PDF 表禁排；UAX #14 在本语料下同样不在 `.` 前断行
        // （实测候选与当前一致），因此该项不构成质量回归。
        if name == "halfwidth" {
            assert!(
                current_viol.is_empty(),
                "当前策略不应让半角标点落在行首：{current_viol:?}"
            );
            assert!(
                candidate_viol.is_empty(),
                "候选也不应让半角标点落在行首：{candidate_viol:?}"
            );
        }
        // 破折号：两边都允许其出现在行首，屏幕靠 WJ 禁止；这是统一禁则表的待裁决项。
        if name == "em_dash" {
            assert!(
                current_viol.iter().any(|&(_, c)| c == '—')
                    && candidate_viol.iter().any(|&(_, c)| c == '—'),
                "当前与候选都应允许破折号行首：current={current_viol:?} candidate={candidate_viol:?}"
            );
        }
    }

    eprintln!("[spike] 合计不同行（规范化逐行比较）：{total_diff}");
}

#[test]
fn pdf_linebreak_spike_mandatory_breaks_and_overlong_words() {
    let em = PageGeom::a4(20.0).content_w() / BODY_SIZE;

    // 硬换行：候选必须保留段落内换行（含空行）；生产按块切段，换行不会进入 fill_lines。
    let lines = candidate_lines("第一行\n\n第三行", em * BODY_SIZE);
    assert_eq!(lines.len(), 3, "强制断点应产生 3 行：{lines:?}");
    assert_eq!(content(&lines[0]), "第一行");
    assert_eq!(content(&lines[1]), "", "空行须保留：{lines:?}");
    assert_eq!(content(&lines[2]), "第三行");

    // 超长不可断词：候选必须硬断、不丢字、不越界。
    let word = "x".repeat(200);
    let lines = candidate_lines(&word, 60.0 * BODY_SIZE);
    assert!(lines.len() > 1, "超长 ASCII 词必须硬断");
    assert_eq!(lines.concat(), word, "硬断不得丢字");
    assert!(
        lines
            .iter()
            .all(|line| FakeMetrics.text_width(line, BODY_SIZE) <= 60.0 * BODY_SIZE + 0.01),
        "硬断后的行不得越出版心：{lines:?}"
    );
}
