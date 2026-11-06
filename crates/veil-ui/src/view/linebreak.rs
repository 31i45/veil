//! 断行策略的唯一真值层：行首/行尾禁则表 + 胶水决策。
//!
//! 阶段 0 只负责「哪些边界不允许断」这一判定面；需要成形后字宽的行装配（把
//! 允许断点装进版心）留到阶段 1，避免在没有 advance 的阶段重复造轮子。
//!
//! 机制分工：
//! - 所有禁则标点（含 UAX #14 已覆盖的 CL/CP/EX/IS/NS/OP）都前置/后置 WJ
//!   胶水。理由：cargo-text 的 linebreaker 虽然理论上遵守 UAX，但在链接文本、
//!   内联格式等复杂场景下会绕过禁则断点；WJ 是零宽字符，不会引入发丝缝。
//! - 本表的成员与注入逻辑共用同一份常量：禁止实现侧另写子集。

/// 行首禁则标点表（GB/T 15834 通行集）。
/// 这些字符不允许出现在视觉行行首。
pub(crate) const LINE_START_FORBIDDEN: &[char] = &[
    '\u{3002}', '\u{3001}', '\u{FF0C}', '\u{FF0E}', '\u{FF1B}', '\u{FF1A}', '\u{FF1F}',
    '\u{FF01}', '\u{2026}', '\u{2025}', '\u{2014}', '\u{00B7}', '\u{FF5E}', '\u{FF09}',
    '\u{3011}', '\u{300B}', '\u{300D}', '\u{300F}', '\u{3015}', '\u{3009}', '\u{FF3D}',
    '\u{FF5D}', '\u{201D}', '\u{2019}',
];

/// 行尾禁则标点表（开括号/开引号）：不允许出现在视觉行行尾。
pub(crate) const LINE_END_FORBIDDEN: &[char] = &[
    '\u{FF08}', '\u{3010}', '\u{300A}', '\u{300C}', '\u{300E}', '\u{3014}', '\u{3008}',
    '\u{FF3B}', '\u{FF5B}', '\u{201C}', '\u{2018}',
];

/// 文内零宽粘连符（Word Joiner）：UAX #14 的 WJ 类，前后都不允许断行。
const WJ: char = '\u{2060}';

pub(crate) fn forbidden_line_start(ch: char) -> bool {
    LINE_START_FORBIDDEN.contains(&ch)
}

pub(crate) fn forbidden_line_end(ch: char) -> bool {
    LINE_END_FORBIDDEN.contains(&ch)
}

/// 该字符前需要 WJ 胶水（所有行首禁则标点；UAX 已覆盖的 CL 也强制注入 WJ，
/// 因 cargo-text linebreaker 在链接/格式场景会绕过 UAX 禁则）。
pub(crate) fn glue_before(ch: char) -> bool {
    forbidden_line_start(ch)
}

/// 该字符后需要 WJ 胶水（所有行尾禁则标点）。
pub(crate) fn glue_after(ch: char) -> bool {
    forbidden_line_end(ch)
}

/// 排版文本里的一段胶水替换：`[start, end)` 源区间替换为 `fill`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GlueSpan {
    pub start: usize,
    pub end: usize,
    pub fill: String,
}

/// 生成胶水替换段（源文本坐标）。
///
/// 规则与旧实现逐字一致：左粘字符前插 WJ（文首/硬行首除外），右粘字符后插
/// WJ（行尾除外）；连续胶水字符各自成对注入，保证内部也不可断。
pub(crate) fn glue_spans(body: &str) -> Vec<GlueSpan> {
    let chars: Vec<(usize, char)> = body.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !(glue_before(chars[i].1) || glue_after(chars[i].1)) {
            i += 1;
            continue;
        }
        let start = chars[i].0;
        let mut j = i;
        while j < chars.len() && (glue_before(chars[j].1) || glue_after(chars[j].1)) {
            j += 1;
        }
        let end = chars.get(j).map(|&(p, _)| p).unwrap_or(body.len());
        let mut fill = String::with_capacity(end - start + 3 * (j - i));
        for idx in i..j {
            let c = chars[idx].1;
            let prev_is_newline = idx > 0 && matches!(chars[idx - 1].1, '\n' | '\r');
            if glue_before(c) && idx > 0 && !prev_is_newline {
                fill.push(WJ);
            }
            fill.push(c);
            let next_is_newline =
                matches!(chars.get(idx + 1).map(|&(_, c)| c), Some('\n' | '\r'));
            if glue_after(c) && !next_is_newline {
                fill.push(WJ);
            }
        }
        out.push(GlueSpan { start, end, fill });
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_tables_cover_gb_set() {
        for ch in ['。', '，', '、', '；', '：', '？', '！', '）', '】', '」', '』'] {
            assert!(forbidden_line_start(ch), "行首禁则表缺 {ch:?}");
        }
        for ch in ['（', '【', '《', '「', '『', '〔', '〈', '［', '｛'] {
            assert!(forbidden_line_end(ch), "行尾禁则表缺 {ch:?}");
        }
    }

    #[test]
    fn all_forbidden_punctuation_is_glued() {
        // UAX #14 已覆盖的 CL/EX/IS 也强制注入 WJ，防止 cargo-text 绕过
        for ch in ['。', '，', '、', '；', '：', '？', '！', '）', '】', '」'] {
            assert!(glue_before(ch), "{ch:?} 是行首禁则，应注入前置 WJ");
        }
        for ch in ['（', '【', '《', '「', '『', '〔', '〈', '［', '｛'] {
            assert!(glue_after(ch), "{ch:?} 是行尾禁则，应注入后置 WJ");
        }
    }

    #[test]
    fn glue_applies_to_cjk_supplement_set() {
        for ch in ['…', '‥', '—', '·', '～', '”', '’'] {
            assert!(glue_before(ch), "{ch:?} 属行首禁则补集，应注入 WJ");
        }
        for ch in ['“', '‘'] {
            assert!(glue_after(ch), "{ch:?} 属行尾禁则补集，应注入 WJ");
        }
    }

    #[test]
    fn glue_spans_insert_wj_before_and_after_supplement_runs() {
        let g = glue_spans("中……文");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].start, 3);
        assert_eq!(g[0].end, 9);
        // 两个省略号各自前置 WJ（内部也不可断）
        assert_eq!(g[0].fill.chars().filter(|&c| c == WJ).count(), 2);
        assert!(g[0].fill.starts_with(WJ));
        assert!(g[0].fill.ends_with('…'));
    }

    #[test]
    fn glue_spans_skip_line_start_and_line_end() {
        // 硬行首的省略号不前置 WJ
        let head = glue_spans("……文");
        assert!(head[0].fill.starts_with('…'));
        assert!(!head[0].fill.starts_with(WJ));
        // 行尾（下一字符为换行）不注入右粘 WJ
        assert_eq!(glue_spans("中“\n")[0].fill, "“");
        // 非行尾则注入
        assert!(glue_spans("中“")[0].fill.ends_with(WJ));
    }

    #[test]
    fn glue_spans_include_uax_covered_punctuation() {
        // CL 类标点也应注入前置 WJ
        let g = glue_spans("中。");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].fill, "\u{2060}。");
        let g2 = glue_spans("中，");
        assert_eq!(g2[0].fill, "\u{2060}，");
        // OP 类标点应注入后置 WJ
        let g3 = glue_spans("（中");
        assert_eq!(g3[0].fill, "（\u{2060}");
    }
}
