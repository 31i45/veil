//! 撤销/重做事务栈。仅文本变更入栈；Reveal 等注解是视图临时状态，不入栈。
//!
//! 分组语义（VS Code 式纯连续性合并，3e 修 U1——去掉 500ms 时间窗）：
//! - 连续单字符插入（位置紧邻向后延展）合并为一步，停顿再久也不断组；
//! - 连续单字符删除同理：Backspace 向左延展、Delete 在原地向右延展；
//! - 换行、多字符插入/删除（粘贴、选区替换）、多原语事务一律单独成步并断开分组；
//! - 撤销/重做后分组断开，新编辑另起一步；
//! - 撤销栈上限 1000 步，超出丢弃最旧（Redo 栈随之只保留可达项）。

use std::collections::VecDeque;

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

/// 撤销栈上限：内存有界，远超人类回退需求。
const MAX_UNDO: usize = 1000;

#[derive(Default)]
pub(crate) struct History {
    undo: VecDeque<Transaction>,
    redo: VecDeque<Transaction>,
    /// 上一步编辑的分组类别；`None` 表示不参与合并（撤销后/不可合并编辑后）。
    last: Option<Group>,
}

impl History {
    /// 新编辑使 redo 栈失效（编辑器标准语义），并按分组规则合并/入栈。
    pub fn record(&mut self, tx: Transaction) {
        if tx.prims.is_empty() {
            return;
        }
        self.redo.clear();
        let group = classify(&tx);
        if let (Some(group), Some(last_group)) = (group, self.last) {
            if group == last_group && self.try_merge(&tx, group) {
                return;
            }
        }
        self.undo.push_back(tx);
        if self.undo.len() > MAX_UNDO {
            self.undo.pop_front();
        }
        self.last = group;
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
}
