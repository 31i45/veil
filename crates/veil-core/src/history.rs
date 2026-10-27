//! 撤销/重做事务栈。仅文本变更入栈；Reveal 等注解是视图临时状态，不入栈。
//!
//! 分组语义（编辑器惯例）：
//! - 连续单字符插入（同一位置向后延展）在 500ms 静默窗内合并为一步；
//! - 连续单字符删除同理：Backspace 向左延展、Delete 在原地向右延展；
//! - 换行、多字符插入/删除（粘贴、选区替换）、多原语事务一律单独成步；
//! - 撤销/重做后分组断开，新编辑另起一步；
//! - 撤销栈上限 1000 步，超出丢弃最旧（Redo 栈随之只保留可达项）。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// 单次原语编辑的完整记录（自逆：交换 deleted/inserted 即逆操作）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct PrimEdit {
    pub start: usize,
    pub deleted: String,
    pub inserted: String,
}

/// 一次 `Session::apply` 原语操作对应一个事务，原语按应用顺序排列。
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct Transaction {
    pub prims: Vec<PrimEdit>,
}

/// 可合并的编辑类别：只有「单字符、无换行、单原语」的编辑参与分组。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Insert,
    Delete,
}

/// 合并静默窗：超过此时长的两次编辑各自成步。
const MERGE_WINDOW: Duration = Duration::from_millis(500);
/// 撤销栈上限：内存有界，远超人类回退需求。
const MAX_UNDO: usize = 1000;

#[derive(Default)]
pub(crate) struct History {
    undo: VecDeque<Transaction>,
    redo: VecDeque<Transaction>,
    /// 上一步编辑的分组类别与时刻；`None` 表示不参与合并（撤销后/不可合并编辑后）。
    last: Option<(Group, Instant)>,
}

impl History {
    /// 新编辑使 redo 栈失效（编辑器标准语义），并按分组规则合并/入栈。
    pub fn record(&mut self, tx: Transaction) {
        self.record_at(tx, Instant::now());
    }

    fn record_at(&mut self, tx: Transaction, now: Instant) {
        if tx.prims.is_empty() {
            return;
        }
        self.redo.clear();
        let group = classify(&tx);
        if let (Some(group), Some((last_group, at))) = (group, self.last) {
            if group == last_group
                && now.saturating_duration_since(at) <= MERGE_WINDOW
                && self.try_merge(&tx, group)
            {
                self.last = Some((group, now));
                return;
            }
        }
        self.undo.push_back(tx);
        if self.undo.len() > MAX_UNDO {
            self.undo.pop_front();
        }
        self.last = group.map(|g| (g, now));
    }

    /// 把新事务并入栈顶事务。返回 false 表示位置不连续，需单独成步。
    fn try_merge(&mut self, tx: &Transaction, group: Group) -> bool {
        let np = &tx.prims[0];
        let Some(last) = self.undo.back_mut() else {
            return false;
        };
        let lp = &mut last.prims[0];
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
        self.undo.pop_back()
    }

    pub fn pop_redo(&mut self) -> Option<Transaction> {
        self.last = None;
        self.redo.pop_back()
    }

    pub fn push_undo(&mut self, tx: Transaction) {
        self.last = None;
        self.undo.push_back(tx);
        if self.undo.len() > MAX_UNDO {
            self.undo.pop_front();
        }
    }

    pub fn push_redo(&mut self, tx: Transaction) {
        self.last = None;
        self.redo.push_back(tx);
    }
}

/// 单字符、无换行、单原语 → 可分组；其余一律单独成步。
fn classify(tx: &Transaction) -> Option<Group> {
    if tx.prims.len() != 1 {
        return None;
    }
    let p = &tx.prims[0];
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
            prims: vec![PrimEdit {
                start,
                deleted: String::new(),
                inserted: text.into(),
            }],
        }
    }

    fn del(start: usize, text: &str) -> Transaction {
        Transaction {
            prims: vec![PrimEdit {
                start,
                deleted: text.into(),
                inserted: String::new(),
            }],
        }
    }

    fn only(h: &mut History) -> PrimEdit {
        let tx = h.pop_undo().expect("应有一步");
        assert_eq!(tx.prims.len(), 1);
        tx.prims.into_iter().next().unwrap()
    }

    #[test]
    fn consecutive_typing_merges_within_window() {
        let mut h = History::default();
        let t0 = Instant::now();
        h.record_at(ins(3, "a"), t0);
        h.record_at(ins(4, "b"), t0 + Duration::from_millis(400));
        let p = only(&mut h);
        assert_eq!((p.start, p.inserted.as_str()), (3, "ab"));
        assert!(h.pop_undo().is_none(), "合并后只有一步");
    }

    #[test]
    fn pause_over_window_breaks_group() {
        let mut h = History::default();
        let t0 = Instant::now();
        h.record_at(ins(3, "a"), t0);
        h.record_at(ins(4, "b"), t0 + Duration::from_millis(501));
        assert_eq!(only(&mut h).inserted, "b");
        assert_eq!(only(&mut h).inserted, "a");
    }

    #[test]
    fn non_adjacent_insert_does_not_merge() {
        let mut h = History::default();
        let t0 = Instant::now();
        h.record_at(ins(3, "a"), t0);
        h.record_at(ins(9, "b"), t0);
        assert_eq!(only(&mut h).start, 9);
        assert_eq!(only(&mut h).start, 3);
    }

    #[test]
    fn newline_and_paste_are_own_steps() {
        let mut h = History::default();
        let t0 = Instant::now();
        h.record_at(ins(0, "a"), t0);
        h.record_at(ins(1, "\n"), t0);
        h.record_at(ins(2, "bc"), t0);
        assert_eq!(only(&mut h).inserted, "bc");
        assert_eq!(only(&mut h).inserted, "\n");
        assert_eq!(only(&mut h).inserted, "a");
    }

    #[test]
    fn backspace_merges_leftwards() {
        let mut h = History::default();
        let t0 = Instant::now();
        h.record_at(del(4, "b"), t0); // 删 "ab|" 的 b -> [3,4)
        h.record_at(del(3, "a"), t0); // 再删 a -> [2,3)? 用连续位置：新删除点左缘
        let p = only(&mut h);
        assert_eq!((p.start, p.deleted.as_str()), (3, "ab"));
        assert!(h.pop_undo().is_none());
    }

    #[test]
    fn forward_delete_merges_in_place() {
        let mut h = History::default();
        let t0 = Instant::now();
        h.record_at(del(3, "a"), t0);
        h.record_at(del(3, "b"), t0);
        let p = only(&mut h);
        assert_eq!((p.start, p.deleted.as_str()), (3, "ab"));
        assert!(h.pop_undo().is_none());
    }

    #[test]
    fn mixed_insert_then_delete_does_not_merge() {
        let mut h = History::default();
        let t0 = Instant::now();
        h.record_at(ins(3, "a"), t0);
        h.record_at(del(4, "a"), t0);
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
        let t0 = Instant::now();
        h.record_at(ins(3, "a"), t0);
        let _ = h.pop_undo();
        h.record_at(ins(3, "b"), t0);
        assert_eq!(only(&mut h).inserted, "b");
        assert!(h.pop_undo().is_none());
    }

    #[test]
    fn undo_stack_is_bounded() {
        let mut h = History::default();
        let t0 = Instant::now();
        // 每次间隔超过合并窗 → 逐步入栈
        for i in 0..(MAX_UNDO + 5) {
            h.record_at(ins(0, "a"), t0 + Duration::from_secs(i as u64));
        }
        let mut steps = 0;
        while h.pop_undo().is_some() {
            steps += 1;
        }
        assert_eq!(steps, MAX_UNDO, "上限裁剪最旧步");
    }
}
