//! 撤销/重做事务栈。仅文本变更入栈；Reveal 等注解是视图临时状态，不入栈。
//!
//! 分组语义（VS Code 式纯连续性合并，3e 修 U1——去掉 500ms 时间窗）：
//! - 连续单字符插入（位置紧邻向后延展）合并为一步，停顿再久也不断组；
//! - 连续单字符删除同理：Backspace 向左延展、Delete 在原地向右延展；
//! - 换行、多字符插入/删除（粘贴、选区替换）一律单独成步并断开分组；
//! - 撤销/重做、显式断组（光标移动/选区变化）后分组断开，新编辑另起一步；
//! - 撤销栈上限 1000 步，超出丢弃最旧（Redo 栈随之只保留可达项）。

use std::collections::VecDeque;

/// 单次原语编辑的完整记录（自逆：交换 deleted/inserted 即逆操作）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct PrimEdit {
    pub start: usize,
    pub deleted: String,
    pub inserted: String,
}

/// 一次 `Session::apply` 原语操作对应一个事务。
/// v1 全走单个 Replace，多原语事务无消费者。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Transaction {
    pub prim: PrimEdit,
}

/// 可合并的编辑类别：只有「单字符、无换行、单原语」的编辑参与分组。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Insert,
    Delete,
}

/// 撤销栈上限：内存有界，远超人类回退需求。
const MAX_UNDO: usize = 1000;
/// 撤销栈字节预算：整篇替换 ×N 时不再 RSS 爆炸。
/// 单步自身超 16MB 保留（保证完整撤销），后续编辑入栈后才字节裁剪。
const MAX_UNDO_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct History {
    undo: VecDeque<Transaction>,
    redo: VecDeque<Transaction>,
    /// 上一步编辑的分组类别；`None` 表示不参与合并（撤销后/不可合并编辑后）。
    last: Option<Group>,
    /// undo 栈累计字节（deleted.len() + inserted.len()）。pub(crate) 供测试断言。
    pub(crate) undo_bytes: usize,
}

impl History {
    fn tx_byte_size(tx: &Transaction) -> usize {
        tx.prim.deleted.len() + tx.prim.inserted.len()
    }

    /// 当前可撤销步数（只读观测值；不改变分组/裁剪语义）。
    pub(crate) fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// 显式断开分组：下一次 `record` 不与栈顶事务合并（光标移动/选区变化时调用）。
    pub(crate) fn break_group(&mut self) {
        self.last = None;
    }

    /// 新编辑使 redo 栈失效（编辑器标准语义），并按分组规则合并/入栈。
    pub fn record(&mut self, tx: Transaction) {
        self.redo.clear();
        let group = classify(&tx);
        if let (Some(group), Some(last_group)) = (group, self.last) {
            if group == last_group {
                let old_top_sz = Self::tx_byte_size(self.undo.back().unwrap());
                if self.try_merge(&tx, group) {
                    let new_top_sz = Self::tx_byte_size(self.undo.back().unwrap());
                    self.undo_bytes = self
                        .undo_bytes
                        .saturating_add(new_top_sz.saturating_sub(old_top_sz));
                    // merge 路径栈长度不变，无需步数/字节裁剪
                    return;
                }
            }
        }
        self.undo.push_back(tx);
        let sz = Self::tx_byte_size(self.undo.back().unwrap());
        self.undo_bytes = self.undo_bytes.saturating_add(sz);
        // 步数裁剪（保留最旧步直到超出 MAX_UNDO）
        while self.undo.len() > MAX_UNDO {
            if let Some(old) = self.undo.pop_front() {
                let old_sz = Self::tx_byte_size(&old);
                self.undo_bytes = self.undo_bytes.saturating_sub(old_sz);
            }
        }
        // 字节裁剪（单步超限保留——len > 1 时才进 while）
        while self.undo.len() > 1 && self.undo_bytes > MAX_UNDO_BYTES {
            if let Some(old) = self.undo.pop_front() {
                let old_sz = Self::tx_byte_size(&old);
                self.undo_bytes = self.undo_bytes.saturating_sub(old_sz);
            }
        }
        self.last = group;
    }

    /// 把新事务并入栈顶事务。返回 false 表示位置不连续，需单独成步。
    fn try_merge(&mut self, tx: &Transaction, group: Group) -> bool {
        let np = &tx.prim;
        let Some(last) = self.undo.back_mut() else {
            return false;
        };
        let lp = &mut last.prim;
        match group {
            Group::Insert if np.start == lp.start + lp.inserted.len() => {
                lp.inserted.push_str(&np.inserted);
                true
            }
            // Backspace：新删除点在旧记录左缘，旧文本拼在其后
            Group::Delete if np.start + np.deleted.len() == lp.start => {
                lp.start = np.start;
                let mut deleted = np.deleted.clone();
                deleted.push_str(&lp.deleted);
                lp.deleted = deleted;
                true
            }
            // Delete 键：删除点原地不动，旧文本在前
            Group::Delete if np.start == lp.start => {
                lp.deleted.push_str(&np.deleted);
                true
            }
            _ => false,
        }
    }

    /// 取一步撤销：分组随之断开（撤销后的新编辑另起一步）。
    pub fn pop_undo(&mut self) -> Option<Transaction> {
        self.last = None;
        let tx = self.undo.pop_back()?;
        let sz = Self::tx_byte_size(&tx);
        self.undo_bytes = self.undo_bytes.saturating_sub(sz);
        Some(tx)
    }

    pub fn pop_redo(&mut self) -> Option<Transaction> {
        self.last = None;
        self.redo.pop_back()
    }

    pub fn push_undo(&mut self, tx: Transaction) {
        self.last = None;
        let sz = Self::tx_byte_size(&tx);
        self.undo.push_back(tx);
        self.undo_bytes = self.undo_bytes.saturating_add(sz);
        while self.undo.len() > MAX_UNDO {
            if let Some(old) = self.undo.pop_front() {
                let old_sz = Self::tx_byte_size(&old);
                self.undo_bytes = self.undo_bytes.saturating_sub(old_sz);
            }
        }
        while self.undo.len() > 1 && self.undo_bytes > MAX_UNDO_BYTES {
            if let Some(old) = self.undo.pop_front() {
                let old_sz = Self::tx_byte_size(&old);
                self.undo_bytes = self.undo_bytes.saturating_sub(old_sz);
            }
        }
    }

    pub fn push_redo(&mut self, tx: Transaction) {
        self.last = None;
        self.redo.push_back(tx);
    }
}

/// 单字符、无换行 → 可分组；其余一律单独成步。
fn classify(tx: &Transaction) -> Option<Group> {
    let p = &tx.prim;
    let one = |s: &str| s.chars().count() == 1 && !s.contains('\n');
    if p.deleted.is_empty() && one(&p.inserted) {
        Some(Group::Insert)
    } else if p.inserted.is_empty() && one(&p.deleted) {
        Some(Group::Delete)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ins(start: usize, text: &str) -> Transaction {
        Transaction {
            prim: PrimEdit {
                start,
                deleted: String::new(),
                inserted: text.into(),
            },
        }
    }

    fn del(start: usize, text: &str) -> Transaction {
        Transaction {
            prim: PrimEdit {
                start,
                deleted: text.into(),
                inserted: String::new(),
            },
        }
    }

    fn only(h: &mut History) -> PrimEdit {
        h.pop_undo().expect("应有一步").prim
    }

    #[test]
    fn consecutive_typing_merges() {
        // 纯连续性语义：无时间窗，同类单字符且位置紧邻即合并，与两次按键的间隔无关
        let mut h = History::default();
        h.record(ins(3, "a"));
        h.record(ins(4, "b"));
        let p = only(&mut h);
        assert_eq!((p.start, p.inserted.as_str()), (3, "ab"));
        assert!(h.pop_undo().is_none(), "合并后只有一步");
    }

    #[test]
    fn unmergeable_edit_breaks_group() {
        let mut h = History::default();
        h.record(ins(3, "a"));
        h.record(ins(4, "bc")); // 多字符：单独成步并断开分组
        h.record(ins(6, "d")); // 位置虽连续，也不与 a 合并
        assert_eq!(only(&mut h).inserted, "d");
        assert_eq!(only(&mut h).inserted, "bc");
        assert_eq!(only(&mut h).inserted, "a");
    }

    #[test]
    fn non_adjacent_insert_does_not_merge() {
        let mut h = History::default();
        h.record(ins(3, "a"));
        h.record(ins(9, "b"));
        assert_eq!(only(&mut h).start, 9);
        assert_eq!(only(&mut h).start, 3);
    }

    #[test]
    fn newline_and_paste_are_own_steps() {
        let mut h = History::default();
        h.record(ins(0, "a"));
        h.record(ins(1, "\n"));
        h.record(ins(2, "bc"));
        assert_eq!(only(&mut h).inserted, "bc");
        assert_eq!(only(&mut h).inserted, "\n");
        assert_eq!(only(&mut h).inserted, "a");
    }

    #[test]
    fn backspace_merges_leftwards() {
        let mut h = History::default();
        h.record(del(4, "b")); // 删 "ab|" 的 b
        h.record(del(3, "a")); // 再删 a，向左延展
        let p = only(&mut h);
        assert_eq!((p.start, p.deleted.as_str()), (3, "ab"));
        assert!(h.pop_undo().is_none());
    }

    #[test]
    fn forward_delete_merges_in_place() {
        let mut h = History::default();
        h.record(del(3, "a"));
        h.record(del(3, "b"));
        let p = only(&mut h);
        assert_eq!((p.start, p.deleted.as_str()), (3, "ab"));
        assert!(h.pop_undo().is_none());
    }

    #[test]
    fn mixed_insert_then_delete_does_not_merge() {
        let mut h = History::default();
        h.record(ins(3, "a"));
        h.record(del(4, "a"));
        let d = only(&mut h);
        assert_eq!(
            (d.start, d.deleted.as_str(), d.inserted.as_str()),
            (4, "a", "")
        );
        assert_eq!(only(&mut h).inserted, "a");
    }

    #[test]
    fn undo_breaks_group() {
        let mut h = History::default();
        h.record(ins(3, "a"));
        let _ = h.pop_undo();
        h.record(ins(3, "b"));
        assert_eq!(only(&mut h).inserted, "b");
        assert!(h.pop_undo().is_none());
    }

    #[test]
    fn undo_stack_is_bounded() {
        let mut h = History::default();
        // 位置不连续（恒在 0 处插入）→ 逐步入栈
        for _ in 0..(MAX_UNDO + 5) {
            h.record(ins(0, "a"));
        }
        let mut steps = 0;
        while h.pop_undo().is_some() {
            steps += 1;
        }
        assert_eq!(steps, MAX_UNDO, "上限裁剪最旧步");
    }

    // ---- 批次 2-4：字节预算回归测试 ----

    #[test]
    fn undo_budget_bytes_caps_stable_total() {
        // 整篇替换 ×200，每步 deleted=1MB → 撤销栈字节稳定在 ~32MB（2步×16MB 区间）
        let mut h = History::default();
        for _ in 0..200 {
            h.record(Transaction {
                prim: PrimEdit {
                    start: 0,
                    deleted: "x".repeat(1024 * 1024), // 1MB
                    inserted: String::new(),
                },
            });
        }
        let mut total: usize = 0;
        let mut steps: usize = 0;
        while let Some(tx) = h.pop_undo() {
            total += History::tx_byte_size(&tx);
            steps += 1;
        }
        // 字节预算 ~16MB，步数裁剪上限 1000 → 两者同时作用
        // 最终栈应 ≤ ~32MB（单步 1MB + 一些 padding）
        assert!(
            total <= 32 * 1024 * 1024,
            "200 × 1MB 撤销栈总字节 {}B 应 ≤ 32MB",
            total
        );
        assert!(steps <= MAX_UNDO, "步数 {} 超过 {}", steps, MAX_UNDO);
    }

    #[test]
    fn undo_max_steps_not_affected() {
        // 连续 1001 次单字符插入 → undo.len() == 1000（原有步数裁剪语义不变）
        let mut h = History::default();
        for c in 'a'..='z' {
            for _ in 0..40 {
                h.record(ins(0, &c.to_string()));
            }
        }
        // 撤销栈应该正好 MAX_UNDO 步
        let mut steps = 0;
        while h.pop_undo().is_some() {
            steps += 1;
        }
        assert_eq!(steps, MAX_UNDO);
    }

    #[test]
    fn undo_single_oversized_step_kept_alone() {
        // 单个 20MB Transaction → 保留在栈里（单步超限不裁，len=1 不进 while）
        let mut h = History::default();
        h.record(Transaction {
            prim: PrimEdit {
                start: 0,
                deleted: "x".repeat(20 * 1024 * 1024), // 20MB
                inserted: String::new(),
            },
        });
        let tx = h.pop_undo().expect("单步应保留");
        assert_eq!(
            tx.prim.deleted.len(),
            20 * 1024 * 1024,
            "20MB 单步不应被字节裁剪"
        );
    }

    #[test]
    fn undo_oversized_oldest_evicted_when_second_arrives() {
        // 20MB 步 + 小步 → 总字节超预算 → 最旧的 20MB 步被裁掉，只剩小步
        let mut h = History::default();
        h.record(Transaction {
            prim: PrimEdit {
                start: 0,
                deleted: "x".repeat(20 * 1024 * 1024),
                inserted: String::new(),
            },
        });
        h.record(ins(0, "a"));
        // 推完小步后：undo_bytes ≈ 20MB+ → 进 while len>1 && undo_bytes>16MB
        // pop_front 裁掉最旧的 20MB → 栈只剩小步
        let only = h.pop_undo().expect("小步应保留");
        assert_eq!(only.prim.inserted, "a");
        assert!(
            h.pop_undo().is_none(),
            "旧的 20MB 步应被字节裁剪淘汰"
        );
    }

    // ---- 对抗性审查 Step 1：undo_bytes merge 同步 bug 最小复现 ----

    #[test]
    fn undo_bytes_sync_after_merge() {
        // 连续单字符插入触发 try_merge（连续性合并）
        let mut h = History::default();
        h.record(ins(3, "a"));           // push_back, undo_bytes 应 = 1
        assert_eq!(h.undo_bytes, 1, "第一次 record（不合并）undo_bytes 应为 1");
        h.record(ins(4, "b"));           // try_merge 成功 → return，undo_bytes 应 = 2（被合并了新字节）

        // 计算栈顶 Transaction 的真实字节数（merged inserted="ab", 字节=2）
        let top_tx = h.undo.back().unwrap();
        let actual_top_sz = top_tx.prim.inserted.len();

        // undo_bytes 应等于实际累计字节数
        assert_eq!(
            h.undo_bytes, actual_top_sz,
            "merge 成功后 undo_bytes 应同步增加合并的字节：undo_bytes={} vs actual_top_sz={}",
            h.undo_bytes, actual_top_sz
        );
    }
}
