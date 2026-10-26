//! 派生视图域：文档真值的纯函数投影——排版、样式、表格网格、帧缓冲与派生缓存。
//! 全部 headless 可测；交互策略与窗口胶合在 shell/app，不进入本域。

pub(crate) mod cache;
mod frame;
mod layout;
mod style;
mod table;

pub(crate) use cache::Cache;
pub(crate) use layout::node_active;
pub use frame::Frame;
pub use layout::{
    BlockLayout, InlineAsset, InlineImg, LayoutEngine, PlacedImg, BASE_FONT_SIZE, BASE_LINE_HEIGHT,
};
pub use style::{style_spans, Style};
pub use table::{TableCellLayout, TableLayout, TableRow};
