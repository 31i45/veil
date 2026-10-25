//! 语法树纯数据模型：块 / 行内节点 / 表格派生数据。
//!
//! 本模块只含类型定义，无任何解析逻辑；块级扫描见 `block`、行内扫描见
//! `inline`、增量状态维护见父模块（Syntax / reparse）。

use crate::range::Range;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Paragraph,
    /// ATX 标题，级别 1–6。
    AtxHeading(u8),
    FencedCode,
    /// 展示数学块（`$$...$$`）。块内不解析 inline，TeX 源码经 `math_source` 提取。
    Math,
    /// GFM 表格（表头行 + 分隔行 + 至多若干表体行）。块内不解析 inline，
    /// 单元格文本/对齐经 `table` 提取（视图只读渲染，切片 9）。
    Table,
    /// 列表项（v0：单行项，续行不并入）。`ordered` 区分有序/无序；`depth` = 缩进字节 / 2
    /// （上限 8，v0 简化：GFM 的 4 空格制与续行属后续增量）。inline 仅解析标记后内容。
    ListItem {
        ordered: bool,
        depth: u8,
    },
    /// 引用块：连续以 `>` 开头的行合为一块（v0 不支持惰性续行）。inline 逐行解析标记后内容。
    BlockQuote,
    /// 分隔线：`---` / `***` / `___`（同字符 ≥ 3，其间可夹空白）。无 inline。
    ThematicBreak,
}

/// 表格列对齐（GFM 分隔行 `:--` / `--:` / `:-:` 语法）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CellAlign {
    #[default]
    None,
    Left,
    Center,
    Right,
}

/// 单元格派生数据：可见文本 + 内容区间（文档字节偏移，已 trim）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TableCell {
    /// 剥除 inline 标记后的可见文本（`**b**` → `b`），供只读渲染直接使用。
    pub text: String,
    /// 内容区间；网格点击以此起点进入表格源码态。
    pub range: Range,
}

/// 表格派生数据（不持久化、不进撤销栈，随块结构自动正确）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Table {
    /// 每列对齐，长度 = 列数。
    pub aligns: Vec<CellAlign>,
    pub header: Vec<TableCell>,
    /// 表体行：每行长度恒等于列数（缺格补空，多格截断，GFM 行为）。
    pub rows: Vec<Vec<TableCell>>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Block {
    pub id: BlockId,
    pub kind: BlockKind,
    /// 行区间 `[start_line, end_line)`。
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InlineKind {
    /// `**strong**`
    Strong,
    /// `*emph*`
    Emph,
    /// `` `code` ``，内容原样不嵌套
    Code,
    /// `[text](url)`
    Link,
    /// `![alt](url)`：`content` 为 alt 文本（可为空），`url` 为图片路径/URL。
    Image,
    /// `$math$`，内容原样不嵌套
    Math,
}

/// 一棵 inline 子树。所有区间为文档字节偏移、半开 `[start, end)`。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InlineNode {
    pub kind: InlineKind,
    /// 含开闭标记的整个节点区间。
    pub range: Range,
    /// 内容区间（不含标记；Link 为可见文本）。
    pub content: Range,
    /// 链接 URL 区间（仅 Link 为 Some）。
    pub url: Option<Range>,
    /// 嵌套子节点（Code/Math 恒为空）。
    pub children: Vec<InlineNode>,
}
