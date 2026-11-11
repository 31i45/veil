//! 查找栏过渡门面：策略层 `search_strategy` 与视图层 `search_view` 的再导出。
//!
//! 保留本模块名只为让既有调用点（`search::Hit` / `search::layout` / `search::fit_query` …）
//! 零改动。新代码请直接引用分层模块：
//! - 匹配扫描、浮栏几何、命中判定、状态模型 → `super::search_strategy`
//! - 文本截断与浮栏绘制 → `super::search_view`

pub(crate) use super::search_strategy::*;
pub(crate) use super::search_view::{fit_query, text_width};
