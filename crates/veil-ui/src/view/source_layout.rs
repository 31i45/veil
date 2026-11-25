//! 源码模式「纯文本行视图」的文档几何模型。
//!
//! 与排版/像素无关：只把整篇文本按行切分，记录每行内容字节区间，并提供
//! 行号 ↔ 文档偏移 ↔ (行, 列) 的纯函数映射。视图层据此为每一行生成独立的
//! 块几何（空行也有几何），从而让空行成为真正可寻址、可停光标、可选中删除的行。
//!
//! CRLF 口径：`\r\n` 只算一个行分隔，`\r` 不进入行内容；孤立的 `\r` 仍是正文。
//! 文本末尾以 `\n` 结束时，末尾的空串同样是一行（编辑器语义：最后一行可为空）。

/// 单行内容区间（字节偏移，`\r`/`\n` 均不计入 `len`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceLine {
    pub(crate) start: usize,
    pub(crate) len: usize,
}

/// 整篇文本的行切分结果。持有文本副本，便于列（字符）与字节偏移互查。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceLayout {
    text: String,
    lines: Vec<SourceLine>,
}

// 行模型是纯几何 API：当前由逐行合成块消费 `new/line_count/line_range`，
// 其余列/跳转 API 供交互与单测使用（避免非测试构建的 dead_code 噪音）。
#[cfg_attr(not(test), allow(dead_code))]
impl SourceLayout {
    /// 按 `\n` 切行；空文档也有一行（空行），保证光标总有落点。
    pub(crate) fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut lines = Vec::new();
        let mut start = 0usize;
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'\n' {
                // `\r\n`：只把 `\r` 之前的字节算作内容。
                let mut end = i;
                if end > start && bytes[end - 1] == b'\r' {
                    end -= 1;
                }
                lines.push(SourceLine {
                    start,
                    len: end - start,
                });
                start = i + 1;
            }
            i += 1;
        }
        lines.push(SourceLine {
            start,
            len: bytes.len() - start,
        });
        Self {
            text: text.to_string(),
            lines,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn text_len(&self) -> usize {
        self.text.len()
    }

    /// 行数（空文档为 1）。
    pub(crate) fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// 某行内容的字节区间 `[start, end)`（不含行分隔符）。
    pub(crate) fn line_range(&self, line: usize) -> Option<(usize, usize)> {
        self.lines.get(line).map(|l| (l.start, l.start + l.len))
    }

    /// 某行的垂直区间：`[top + line * line_height, +line_height)`。
    ///
    /// 空行同样返回一个完整行高，保证「行高统一、空行有几何」。
    pub(crate) fn line_y(&self, line: usize, line_height: f32, top: f32) -> Option<(f32, f32)> {
        if line >= self.lines.len() {
            return None;
        }
        let y0 = top + line as f32 * line_height;
        Some((y0, y0 + line_height))
    }

    /// 文档偏移 → `(行, 列)`；列按字符计。偏移落在行分隔符或越界时夹到该行内容尾。
    pub(crate) fn line_col(&self, offset: usize) -> (usize, usize) {
        let off = offset.min(self.text.len());
        // 最后一个 `start <= off` 的行；空行 start == 上一行 content end + 分隔符长度，
        // 因此行分隔符字节归上一行（列夹到行尾）。
        let idx = self
            .lines
            .partition_point(|l| l.start <= off)
            .saturating_sub(1);
        let l = self.lines[idx];
        let end = l.start + l.len;
        let mut clamped = off.max(l.start).min(end);
        // 偏移可能落在 UTF-8 字符内部（非编辑路径喂入）：回退到字符边界再切。
        while clamped > l.start && !self.text.is_char_boundary(clamped) {
            clamped -= 1;
        }
        (idx, self.text[l.start..clamped].chars().count())
    }

    /// `(行, 列)` → 文档偏移；列按字符计并夹到行内容长度。行越界返回 `None`。
    pub(crate) fn offset_of(&self, line: usize, col: usize) -> Option<usize> {
        let l = self.lines.get(line)?;
        let slice = &self.text[l.start..l.start + l.len];
        if col == 0 {
            return Some(l.start);
        }
        for (count, (byte, _)) in slice.char_indices().enumerate() {
            if count == col {
                return Some(l.start + byte);
            }
        }
        Some(l.start + l.len)
    }

    pub(crate) fn line_start_offset(&self, line: usize) -> Option<usize> {
        self.lines.get(line).map(|l| l.start)
    }

    pub(crate) fn line_end_offset(&self, line: usize) -> Option<usize> {
        self.lines.get(line).map(|l| l.start + l.len)
    }

    /// 上/下移 `delta` 行并「保持列」：目标行短于目标列时夹到行尾。
    ///
    /// `goal_col` 为 `None` 时沿用当前列。目标行越界（文档首/尾）时夹到首/末行。
    pub(crate) fn jump_line(&self, offset: usize, delta: isize, goal_col: Option<usize>) -> usize {
        let (cur_line, cur_col) = self.line_col(offset);
        let last = self.lines.len().saturating_sub(1);
        let target = (cur_line as isize + delta).clamp(0, last as isize) as usize;
        let col = goal_col.unwrap_or(cur_col);
        self.offset_of(target, col)
            .unwrap_or_else(|| self.lines[target].start)
    }

    /// 下一行（保持列）。已是末行时停在本行同列。
    pub(crate) fn next_line(&self, offset: usize, goal_col: Option<usize>) -> usize {
        self.jump_line(offset, 1, goal_col)
    }

    /// 上一行（保持列）。已是首行时停在本行同列。
    pub(crate) fn prev_line(&self, offset: usize, goal_col: Option<usize>) -> usize {
        self.jump_line(offset, -1, goal_col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines_and_strips_crlf() {
        let l = SourceLayout::new("a\r\nbb\n\nc");
        assert_eq!(l.line_count(), 4);
        assert_eq!(l.line_range(0), Some((0, 1)));
        // 第 2 行 `bb` 从 `\r\n` 之后开始。
        assert_eq!(l.line_range(1), Some((3, 5)));
        assert_eq!(l.line_range(2), Some((6, 6)), "空行有零长内容区间");
        assert_eq!(l.line_range(3), Some((7, 8)));
        assert_eq!(l.text(), "a\r\nbb\n\nc", "不得改写原文");
    }

    #[test]
    fn trailing_newline_yields_empty_last_line() {
        let l = SourceLayout::new("a\n");
        assert_eq!(l.line_count(), 2);
        assert_eq!(l.line_range(0), Some((0, 1)));
        assert_eq!(l.line_range(1), Some((2, 2)));
        let empty = SourceLayout::new("");
        assert_eq!(empty.line_count(), 1);
        assert_eq!(empty.line_range(0), Some((0, 0)));
    }

    #[test]
    fn empty_line_has_geometry() {
        let l = SourceLayout::new("a\n\nb");
        let (y0, y1) = l.line_y(1, 20.0, 10.0).unwrap();
        assert!((y0 - 30.0).abs() < f32::EPSILON);
        assert!((y1 - 50.0).abs() < f32::EPSILON);
        assert_eq!(l.line_y(3, 20.0, 10.0), None);
    }

    #[test]
    fn offset_and_line_col_round_trip() {
        let l = SourceLayout::new("ab\n\n中文");
        assert_eq!(l.line_col(0), (0, 0));
        assert_eq!(l.line_col(2), (0, 2), "行尾偏移");
        assert_eq!(l.line_col(3), (1, 0), "空行起点");
        assert_eq!(l.line_col(4), (2, 0));
        assert_eq!(l.line_col(6), (2, 0), "落在多字节字符内部回退到字符起点");
        assert_eq!(l.line_col(7), (2, 1), "中文字符按字符计列");
        assert_eq!(l.offset_of(2, 1), Some(7));
        assert_eq!(l.offset_of(0, 99), Some(2), "列越界夹到行尾");
        assert_eq!(l.offset_of(9, 0), None);
    }

    #[test]
    fn jump_line_preserves_column_and_clamps() {
        let l = SourceLayout::new("abcdef\nxy\n\n123456");
        // 从首行第 5 列下移：第 2 行只有 2 列，夹到行尾。
        let off = l.jump_line(5, 1, Some(5));
        assert_eq!(off, l.line_end_offset(1).unwrap());
        // 继续下移带原目标列 5：空行夹到 0。
        let off = l.jump_line(off, 1, Some(5));
        assert_eq!(off, l.line_start_offset(2).unwrap());
        // 再下移恢复到第 4 行第 5 列。
        let off = l.jump_line(off, 1, Some(5));
        assert_eq!(l.line_col(off), (3, 5));
        // 越界夹到末行；保持列口径下停在原列，指定超长目标列则夹到行尾。
        assert_eq!(l.jump_line(off, 99, None), off);
        assert_eq!(
            l.jump_line(off, 99, Some(99)),
            l.line_end_offset(3).unwrap()
        );
        assert_eq!(l.prev_line(0, None), 0);
        assert_eq!(l.next_line(l.text_len(), None), l.text_len());
    }
}
