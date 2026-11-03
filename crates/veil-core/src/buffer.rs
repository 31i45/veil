//! rope 文本存储 + mark 存储与变换。
//! ADR-001：mark 语义在此自封装，不向外暴露 rope 类型，rope 实现可替换。

use lapce_xi_rope::Rope;

use crate::range::{Bias, Mark, MarkId};

/// mark 数量设计前提（ADR-001 §3）。当前全 workspace 常驻 mark 恒为 2
/// （caret + anchor，移动光标峰值 4）。达到下列任一条件应改用 sum-tree：
/// 常驻 mark ≥ `MARK_VEC_SCALE_LIMIT`、mark 变换占帧预算 > 5%、或 mark 内存 > 1 MB
/// （`Mark` = 24 B）。
const MARK_VEC_SCALE_LIMIT: usize = 1_000;

pub(crate) struct Buffer {
    rope: Rope,
    /// 按创建顺序存储，**不维护全局有序**：`offset_of`/`remove_mark`/`transform_marks`
    /// 都做全量扫描，无消费者依赖顺序。每次编辑全量变换——O(#marks) 纯算术，
    /// 由 INSTANCE §2 的 p95 红线兜底，超标再换 sum-tree（ADR-001 §3）。
    marks: Vec<Mark>,
    next_mark: u64,
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        Buffer {
            rope: Rope::from(text),
            marks: Vec::new(),
            next_mark: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.rope.len()
    }

    pub fn is_char_boundary(&self, offset: usize) -> bool {
        offset <= self.len() && self.rope.is_codepoint_boundary(offset)
    }

    pub fn text(&self) -> String {
        String::from(self.rope.clone())
    }

    pub fn slice(&self, start: usize, end: usize) -> String {
        self.rope.slice_to_cow(start..end).into_owned()
    }

    pub fn line_count(&self) -> usize {
        self.rope.line_of_offset(self.rope.len()) + 1
    }

    pub fn line_of_offset(&self, offset: usize) -> usize {
        self.rope.line_of_offset(offset)
    }

    pub fn offset_of_line(&self, line: usize) -> usize {
        self.rope.offset_of_line(line)
    }

    pub fn create_mark(&mut self, offset: usize, bias: Bias) -> MarkId {
        debug_assert!(offset <= self.len());
        debug_assert!(
            self.marks.len() < MARK_VEC_SCALE_LIMIT,
            "mark 数量达到 ADR-001 §3 升级阈值：应改用 sum-tree"
        );
        let id = MarkId(self.next_mark);
        self.next_mark += 1;
        // 顺序无消费者依赖（见 marks 字段注释），push 取代 partition_point + insert：
        // O(1) 追加，避免每次建 mark 的 O(#marks) memmove。
        self.marks.push(Mark { id, offset, bias });
        id
    }

    pub fn remove_mark(&mut self, id: MarkId) {
        self.marks.retain(|m| m.id != id);
    }

    pub fn offset_of(&self, id: MarkId) -> Option<usize> {
        self.marks.iter().find(|m| m.id == id).map(|m| m.offset)
    }

    /// `offset` 之前的 grapheme 边界；`offset == 0` 时为 None。
    pub fn prev_grapheme(&self, offset: usize) -> Option<usize> {
        self.rope.prev_grapheme_offset(offset)
    }

    /// `offset` 之后的 grapheme 边界；`offset == len` 时为 None。
    pub fn next_grapheme(&self, offset: usize) -> Option<usize> {
        self.rope.next_grapheme_offset(offset)
    }

    /// 删除 `[start, end)` 并在 `start` 插入 `text`，返回被删除的文本（undo 用）。
    /// 边界合法性由调用方（session）校验。
    pub fn edit(&mut self, start: usize, end: usize, text: &str) -> String {
        let deleted = self.slice(start, end);
        self.transform_marks(start, end, text.len());
        self.rope.edit(start..end, text);
        deleted
    }

    /// mark 变换规则（ADR-003）。设删除 `[s, e)`、插入 n 字节：
    /// - `p < s`：不动；`p > e`：平移 `n - (e - s)`
    /// - `s < p < e`：锚定内容已删，坍缩到 s
    /// - `p == s`：Left → s（左内容存活）；Right → 纯插入时 s+n，删除时锚已失、坍缩到 s
    /// - `p == e`（e > s）：Right → s+n（右内容存活）；Left → s（右锚已删、坍缩）
    fn transform_marks(&mut self, s: usize, e: usize, n: usize) {
        for m in &mut self.marks {
            let p = m.offset;
            m.offset = if p < s {
                p
            } else if p > e {
                p + n - (e - s)
            } else if p == s {
                match m.bias {
                    // 纯插入时 Right 跟随右内容到插入物之后；删除时右内容已删，坍缩
                    Bias::Right if s == e => s + n,
                    _ => s,
                }
            } else if p == e {
                match m.bias {
                    Bias::Right => s + n,
                    Bias::Left => s,
                }
            } else {
                s
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> Buffer {
        Buffer::new(s)
    }

    #[test]
    fn bias_left_stays_before_inserted_text() {
        let mut b = buf("hello");
        let m = b.create_mark(2, Bias::Left);
        b.edit(2, 2, "xy");
        assert_eq!(b.text(), "hexyllo");
        assert_eq!(b.offset_of(m), Some(2));
    }

    #[test]
    fn bias_right_moves_after_inserted_text() {
        let mut b = buf("hello");
        let m = b.create_mark(2, Bias::Right);
        b.edit(2, 2, "xy");
        assert_eq!(b.offset_of(m), Some(4));
    }

    #[test]
    fn delete_collapses_interior_mark_to_start() {
        let mut b = buf("hello world");
        let m = b.create_mark(4, Bias::Left);
        b.edit(2, 6, "");
        assert_eq!(b.text(), "heworld");
        assert_eq!(b.offset_of(m), Some(2));
    }

    #[test]
    fn delete_end_right_bias_follows_right_content() {
        let mut b = buf("hello world");
        let m = b.create_mark(6, Bias::Right);
        b.edit(2, 6, "x");
        assert_eq!(b.text(), "hexworld");
        assert_eq!(b.offset_of(m), Some(3));
    }

    #[test]
    fn marks_after_edit_shift_by_delta() {
        let mut b = buf("hello world");
        let m = b.create_mark(8, Bias::Left);
        b.edit(2, 6, "x");
        assert_eq!(b.offset_of(m), Some(5)); // 8 + 1 - 4
    }

    #[test]
    fn marks_before_edit_untouched() {
        let mut b = buf("hello world");
        let m = b.create_mark(1, Bias::Left);
        b.edit(2, 6, "x");
        assert_eq!(b.offset_of(m), Some(1));
    }

    #[test]
    fn insert_into_empty_doc() {
        let mut b = buf("");
        b.edit(0, 0, "a");
        assert_eq!(b.text(), "a");
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn delete_everything_collapses_all_marks() {
        let mut b = buf("hello");
        let m0 = b.create_mark(0, Bias::Right);
        let m3 = b.create_mark(3, Bias::Left);
        let m5 = b.create_mark(5, Bias::Right);
        b.edit(0, 5, "");
        assert_eq!(b.text(), "");
        // 全删：p==s 的 Right 坍缩到 0，interior 坍缩到 0，p==e 的 Right 到 s+n=0
        assert_eq!(b.offset_of(m0), Some(0));
        assert_eq!(b.offset_of(m3), Some(0));
        assert_eq!(b.offset_of(m5), Some(0));
    }

    #[test]
    fn removed_mark_is_gone() {
        let mut b = buf("hello");
        let m = b.create_mark(2, Bias::Left);
        b.remove_mark(m);
        assert_eq!(b.offset_of(m), None);
    }

    #[test]
    fn mark_at_end_left_stays_right_moves_on_append() {
        let mut b = buf("ab");
        let ml = b.create_mark(2, Bias::Left);
        let mr = b.create_mark(2, Bias::Right);
        b.edit(2, 2, "c");
        assert_eq!(b.offset_of(ml), Some(2));
        assert_eq!(b.offset_of(mr), Some(3));
    }

    #[test]
    fn replace_collapses_interior_marks() {
        let mut b = buf("abcdef");
        let m1 = b.create_mark(1, Bias::Left);
        let m3 = b.create_mark(3, Bias::Right);
        let m5 = b.create_mark(5, Bias::Right);
        b.edit(2, 4, "X"); // "abXef"
        assert_eq!(b.offset_of(m1), Some(1)); // 编辑点前不动
        assert_eq!(b.offset_of(m3), Some(2)); // 区间内坍缩
        assert_eq!(b.offset_of(m5), Some(4)); // 编辑点后平移 5+1-2
    }

    #[test]
    fn line_offset_roundtrip_multibyte() {
        let b = buf("中a\n文b\n");
        assert_eq!(b.offset_of_line(1), 5); // 中(3) + a(1) + \n(1)
        assert_eq!(b.line_of_offset(5), 1);
        assert_eq!(b.line_of_offset(3), 0);
    }

    #[test]
    fn slice_multibyte_substring() {
        let b = buf("中文 test");
        assert_eq!(b.slice(0, 6), "中文");
    }

    #[test]
    fn edit_returns_deleted_text() {
        // undo 依赖：edit 返回被删除的原文
        let mut b = buf("abcd");
        let deleted = b.edit(1, 3, "x");
        assert_eq!(deleted, "bc");
        assert_eq!(b.text(), "axd");
    }

    /// ADR-003 规则的独立复述：用于随机编辑序列的回归对照。
    fn expected(p: usize, bias: Bias, s: usize, e: usize, n: usize) -> usize {
        if p < s {
            p
        } else if p > e {
            p + n - (e - s)
        } else if p == s {
            match bias {
                Bias::Right if s == e => s + n,
                _ => s,
            }
        } else if p == e {
            match bias {
                Bias::Right => s + n,
                Bias::Left => s,
            }
        } else {
            s
        }
    }

    #[test]
    fn mark_transform_matches_adr003_across_random_edits() {
        // 确定性伪随机（xorshift64）：不引入新 dev-dependency，失败可复现。
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        let mut b = buf("abcdefghij");
        let mut alive: Vec<(MarkId, Bias)> = Vec::new();

        for step in 0..300 {
            match next() % 3 {
                0 => {
                    let off = (next() as usize) % (b.len() + 1);
                    let bias = if next() % 2 == 0 {
                        Bias::Left
                    } else {
                        Bias::Right
                    };
                    alive.push((b.create_mark(off, bias), bias));
                }
                1 if !alive.is_empty() => {
                    let i = (next() as usize) % alive.len();
                    let (id, _) = alive.remove(i);
                    b.remove_mark(id);
                }
                _ => {
                    let start = (next() as usize) % (b.len() + 1);
                    let end = start + (next() as usize) % (b.len() - start + 1);
                    let insert = &"xyz"[..(next() % 4) as usize];
                    // 快照 + 期望值必须在 edit 前算好（offset 会被变换）
                    let snapshot: Vec<(MarkId, usize, Bias)> = alive
                        .iter()
                        .map(|&(id, bias)| (id, b.offset_of(id).unwrap(), bias))
                        .collect();
                    b.edit(start, end, insert);
                    for (id, p, bias) in snapshot {
                        let want = expected(p, bias, start, end, insert.len());
                        assert_eq!(
                            b.offset_of(id),
                            Some(want),
                            "step {step}: s={start} e={end} n={} p={p} bias={bias:?}",
                            insert.len()
                        );
                    }
                }
            }
            for &(id, _) in &alive {
                let off = b.offset_of(id).expect("alive mark 丢失");
                assert!(off <= b.len(), "step {step}: mark 越界 {off} > {}", b.len());
            }
        }
    }

    #[test]
    fn mark_transform_scales_linearly_with_mark_count() {
        // 只测 mark 数量对单位编辑成本的影响；20k 个 mark 直接按创建顺序推入，
        // 避免 create_mark 的阈值断言（该断言只约束真实产品路径）。
        let mut b = buf(&"x".repeat(40_000));
        for i in 0..20_000u64 {
            b.marks.push(Mark {
                id: MarkId(i),
                offset: (i as usize) * 2,
                bias: Bias::Left,
            });
        }
        b.next_mark = 20_000;

        let t = std::time::Instant::now();
        for _ in 0..50 {
            b.edit(0, 0, "y");
        }
        let ms = t.elapsed().as_secs_f64() * 1e3;
        println!("20k mark × 50 次插入编辑：{ms:.2} ms");
        assert!(ms < 500.0, "mark 变换出现超线性退化：{ms:.2} ms");
    }
}
