//! 断行策略的唯一真值层：行首/行尾禁则表 + 胶水决策。
//!
//! 阶段 0 只负责「哪些边界不允许断」这一判定面；需要成形后字宽的行装配（把
//! 允许断点装进版心）留到阶段 1，避免在没有 advance 的阶段重复造轮子。
//!
//! 机制分工：
//! - UAX #14 已天然禁止的类（CL/CP/EX/IS/NS 之前，OP 之后）不重复注入 WJ，
//!   避免每个标点都多出约 1.3px 的发丝缝；
//! - 仅对中文特有的补集（省略号/破折号/间隔号/全角波浪/闭引号/开引号）注入
//!   WJ 胶水，把「不能断」的意图显式交给断行器。
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

/// UAX #14 已在「其前」禁止断行的类：CL/CP/EX/IS/NS。
/// 判定只覆盖禁则表中确实由 UAX 承担的部分；闭引号（QU）保守纳入胶水，
/// 避免不同 linebreak 实现/版本对 QU 处理不一致。
fn uax_forbids_break_before(ch: char) -> bool {
    matches!(
        ch,
        '\u{3002}' // 。 CL
            | '\u{3001}' // 、 CL
            | '\u{FF0C}' // ， CL
            | '\u{FF0E}' // ． CL
            | '\u{FF1B}' // ； CL
            | '\u{FF1A}' // ： IS
            | '\u{FF1F}' // ？ EX
            | '\u{FF01}' // ！ EX
            | '\u{FF09}' // ） CL
            | '\u{3011}' // 】 CL
            | '\u{300B}' // 》 CL
            | '\u{300D}' // 」 CL
            | '\u{300F}' // 』 CL
            | '\u{3015}' // 〕 CL
            | '\u{3009}' // 〉 CL
            | '\u{FF3D}' // ］ CL
            | '\u{FF5D}' // ｝ CL
    )
}

/// UAX #14 已在「其后」禁止断行的类：OP（开括号/开引号的主体）。
fn uax_forbids_break_after(ch: char) -> bool {
    matches!(
        ch,
        '\u{FF08}' // （ OP
            | '\u{3010}' // 【 OP
            | '\u{300A}' // 《 OP
            | '\u{300C}' // 「 OP
            | '\u{300E}' // 『 OP
            | '\u{3014}' // 〔 OP
            | '\u{3008}' // 〈 OP
            | '\u{FF3B}' // ［ OP
            | '\u{FF5B}' // ｛ OP
    )
}

pub(crate) fn forbidden_line_start(ch: char) -> bool {
    LINE_START_FORBIDDEN.contains(&ch)
}

pub(crate) fn forbidden_line_end(ch: char) -> bool {
    LINE_END_FORBIDDEN.contains(&ch)
}

/// 该字符前需要 WJ 胶水（行首禁则中 UAX 未覆盖的补集）。
pub(crate) fn glue_before(ch: char) -> bool {
    forbidden_line_start(ch) && !uax_forbids_break_before(ch)
}

/// 该字符后需要 WJ 胶水（行尾禁则中 UAX 未覆盖的补集）。
pub(crate) fn glue_after(ch: char) -> bool {
    forbidden_line_end(ch) && !uax_forbids_break_after(ch)
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
    fn uax_covered_punctuation_is_not_glued() {
        for ch in ['。', '，', '、', '；', '：', '？', '！', '）', '】', '」'] {
            assert!(!glue_before(ch), "{ch:?} 已由 UAX #14 承担，不应重复注入 WJ");
        }
        for ch in ['（', '【', '《', '「', '『', '〔', '〈', '［', '｛'] {
            assert!(!glue_after(ch), "{ch:?} 已由 UAX #14 承担，不应重复注入 WJ");
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
        let head = glue_spans("……文");
        assert!(head[0].fill.starts_with('…'));
        assert!(!head[0].fill.starts_with(WJ));
        // 行尾（下一字符为换行）不注入右粘 WJ
        assert_eq!(glue_spans("中“\n")[0].fill, "“");
        // 非行尾则注入
        assert!(glue_spans("中“")[0].fill.ends_with(WJ));
    }

    #[test]
    fn glue_spans_leave_uax_covered_punctuation_untouched() {
        for text in ["中。", "中，", "中？", "（中", "中）"] {
            assert!(glue_spans(text).is_empty(), "{text:?} 不应产生胶水段");
        }
    }
}
