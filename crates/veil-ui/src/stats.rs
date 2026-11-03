//! 全文统计（阶段 1）：中文口径字数。
//! 口径与 Word/WPS 的中文「字数」一致：CJK 字符（汉字/假名/谚文）逐字计 1；
//! 连续的非 CJK 字母数字串计 1 词；空白与标点（含 Markdown 标记符）不计。
//! 统计是全量文本的派生量，刷新点挂在 `refresh_index`（文本真值唯一重建点）。

use veil_core::Range;

use crate::app::App;

/// 中文口径字数：CJK 逐字 + 非 CJK 字母数字串按词。
pub(crate) fn count_words(text: &str) -> usize {
    let mut words = 0;
    let mut in_word = false;
    for c in text.chars() {
        if is_cjk(c) {
            words += 1;
            in_word = false;
        } else if c.is_alphanumeric() {
            if !in_word {
                words += 1;
                in_word = true;
            }
        } else {
            in_word = false; // 空白/标点/标记符既不计字也不续词
        }
    }
    words
}

/// CJK 表意/假名/谚文：逐字计字。不含 CJK 标点——标点非字母数字，任何口径下都不计。
fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4DBF     // 扩展 A
        | 0x4E00..=0x9FFF   // 基本区
        | 0xF900..=0xFAFF   // 兼容表意
        | 0x3040..=0x30FF   // 平假名/片假名
        | 0xAC00..=0xD7AF   // 谚文音节
        | 0x20000..=0x2FA1F // 扩展 B 起（含兼容补充）
    )
}

impl App {
    /// 重算全文字数并写回状态栏数据。调用方：`refresh_index` 末尾——
    /// 编辑/撤销/打开/新建都经此重建块结构，统计不可能落后于真值。
    pub(crate) fn refresh_stats(&mut self) {
        let s = self.editor.session();
        let text = s.text_slice(Range::new(0, s.text_len())).unwrap_or_default();
        self.ui_state.writing.words = count_words(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_cjk_per_char_and_latin_per_word() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("你好世界"), 4);
        assert_eq!(count_words("hello world"), 2);
        // 汉字逐字 + 两个单纯词：2 + 1 + 1
        assert_eq!(count_words("你好 world 2026"), 4);
    }

    #[test]
    fn markdown_markers_and_punctuation_do_not_count() {
        assert_eq!(count_words("**加粗**"), 2);
        assert_eq!(count_words("# 标题"), 2);
        assert_eq!(count_words("- 列表项，带标点。"), 6);
        // 下划线是标记符而非字母：`_x` 仍算 1 词（x）
        assert_eq!(count_words("a_b"), 2);
    }

    #[test]
    fn kana_and_hangul_count_per_char() {
        assert_eq!(count_words("こんにちは"), 5);
        assert_eq!(count_words("안녕하세요"), 5);
        assert_eq!(count_words("漢字ひらがな漢字"), 8);
    }

    #[test]
    fn whitespace_and_line_breaks_break_word_runs() {
        assert_eq!(count_words("abc\ndef"), 2);
        assert_eq!(count_words("abc  def"), 2);
        // 数字串与字母串相邻但被标点分开：v1 / 2 各算 1 词
        assert_eq!(count_words("v1.2"), 2);
    }
}
