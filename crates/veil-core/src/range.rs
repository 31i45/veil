//! 跟随文本的标记点（mark）与字节区间快照。
//! mark 不绑定静态偏移；编辑时由 buffer 统一变换（ADR-001、ADR-003）。

/// 插入恰好发生在 mark 处时的跟随方向（ADR-003）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bias {
    /// 跟随左侧内容：插入物出现在 mark 之后，mark 不动。
    Left,
    /// 跟随右侧内容：插入物出现在 mark 之前，mark 后移。
    Right,
}

/// mark 的全局句柄。其偏移是内部状态，经 [`crate::Session::mark_offset`] 查询快照。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MarkId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Mark {
    pub id: MarkId,
    pub offset: usize,
    pub bias: Bias,
}

/// 字节半开区间 `[start, end)` 快照。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end);
        Range { start, end }
    }

    pub fn len(self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}
