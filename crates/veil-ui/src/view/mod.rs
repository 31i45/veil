//! 派生视图域：文档真值的纯函数投影——排版、样式、表格网格、帧缓冲与派生缓存。
//! 策略层（geometry/模型/命中）全部 headless 可测；交互策略与窗口胶合在 shell/app。
//! 例外：`search_view` / `table_view` 是本域的绘制片段（需要 App 上下文落帧）。

pub(crate) mod cache;
pub(crate) mod ctx_menu;
mod frame;
mod highlight;
mod layout;
pub(crate) mod linebreak;
pub(crate) mod modal;
pub(crate) mod outline;
pub(crate) mod quick_open;
pub(crate) mod search;
mod search_strategy;
mod search_view;
mod style;
pub(crate) mod table;
mod table_strategy;
mod table_view;

pub use crate::geometry::{BASE_FONT_SIZE, BASE_LINE_HEIGHT};
pub(crate) use cache::{Cache, LayoutKey, Memo, Revs, TableKey};
pub use frame::Frame;
#[cfg(test)]
pub(crate) use layout::MAX_BLOCK_BYTES;
pub(crate) use layout::{cell_math_spans, node_active, oversize_notice};
pub use layout::{BlockLayout, InlineAsset, InlineImg, LayoutEngine};
pub(crate) use outline::{Outline, OutlineEntry};
pub(crate) use quick_open::QuickOpen;
pub(crate) use search::Search;
