//! 表格过渡门面：策略层 `table_strategy` 与视图层 `table_view` 的再导出。
//!
//! 保留本模块名只为让既有调用点（`table::TableLayout` / `table::MIN_COL_WIDTH`）零改动。
//! 新代码请直接引用分层模块：
//! - 列宽/行高/命中与活动格模型 → `super::table_strategy`
//! - 单元格 shaping/折行与网格绘制 → `super::table_view`

pub(crate) use super::table_strategy::*;
