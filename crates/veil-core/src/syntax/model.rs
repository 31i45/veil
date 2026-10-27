//! 语法树纯数据模型：块 / 行内节点 / 表格派生数据。
//!
//! 本模块只含类型定义，无任何解析逻辑；块级扫描见 `block`、行内扫描见
//! `inline`、增量状态维护见父模块（Syntax / reparse）。

use crate::range::Range;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Paragraph,
    /// ATX 标题，级别 1–6。
    AtxHeading(u8),
    /// Setext 标题（0c P0-2）：内容行 + `===`/`---` 下划线行，级别 1–2。
    /// 块区间含下划线行；下划线仅识别于段落续行位置（块起始的 `---` 仍为分隔线，
    /// 列表项/引用块后的 `---` 同理）。inline 仅解析内容行（下划线行不参与）。
    SetextHeading(u8),
    /// 围栏代码块。`lang` 为 info string 首词（```rust → Some("rust")），无标记为 None。
    FencedCode(Option<String>),
    /// 展示数学块（`$$...$$`）。块内不解析 inline，TeX 源码经 `math_source` 提取。
    Math,
    /// GFM 表格（表头行 + 分隔行 + 至多若干表体行）。块内不解析 inline，
    /// 单元格文本/对齐经 `table` 提取（视图只读渲染，切片 9）。
    Table,
    /// 列表项（0c P0-3：多行项——项内续行/空行/懒续行并入本块；嵌套项与外层续块
    /// 各为独立块，续块无标记、`task` 恒 None）。`ordered` 区分有序/无序；
    /// `depth` = 缩进字节 / 2（上限 8，视觉深度；结构嵌套由块序列表达）。
    /// inline 仅解析标记后内容（续块无标记则解析全行）。
    /// `task` = GFM 任务列表复选框：`Some(false)`/`Some(true)` 为未勾/勾选。
    ListItem {
        ordered: bool,
        depth: u8,
        task: Option<bool>,
    },
    /// 引用块：连续 `>` 标记行合为一块，支持懒续行（0c P0-3：段落的无标记
    /// 续行并入，`---` 等可打断段落者除外）。inline 逐行解析标记后内容，
    /// 懒续行从行首解析。
    BlockQuote,
    /// 分隔线：`---` / `***` / `___`（同字符 ≥ 3，其间可夹空白）。无 inline。
    ThematicBreak,
    /// 链接引用定义（`[label]: dest "title"`，可多行）：注册进文档引用表，
    /// 无 HTML 输出、无 inline；编辑器按源码行渲染（不背叛源码）。
    LinkRefDef,
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

#[derive(Clone, PartialEq, Eq, Debug)]
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
    /// `~~del~~`（GFM 删除线）
    Strikethrough,
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
