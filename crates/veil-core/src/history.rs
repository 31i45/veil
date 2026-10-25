//! 撤销/重做事务栈。仅文本变更入栈；Reveal 等注解是视图临时状态，不入栈。

/// 单次原语编辑的完整记录（自逆：交换 deleted/inserted 即逆操作）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct PrimEdit {
    pub start: usize,
    pub deleted: String,
    pub inserted: String,
}

/// 一次 `Session::apply` 对应一个事务，原语按应用顺序排列。
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct Transaction {
    pub prims: Vec<PrimEdit>,
}

#[derive(Default)]
pub(crate) struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
}

impl History {
    /// 新编辑使 redo 栈失效（编辑器标准语义）。
    pub fn record(&mut self, tx: Transaction) {
        if !tx.prims.is_empty() {
            self.undo.push(tx);
            self.redo.clear();
        }
    }

    pub fn pop_undo(&mut self) -> Option<Transaction> {
        self.undo.pop()
    }

    pub fn pop_redo(&mut self) -> Option<Transaction> {
        self.redo.pop()
    }

    pub fn push_undo(&mut self, tx: Transaction) {
        self.undo.push(tx);
    }

    pub fn push_redo(&mut self, tx: Transaction) {
        self.redo.push(tx);
    }
}
