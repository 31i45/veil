//! 文档域：持久化真值与生命周期策略（路径/BOM/脏/基线/外部冲突/自动保存），
//! 以及文档的几何投影（块索引与累计 top）。不绘制、不处理输入事件。

pub(crate) mod block_index;
pub(crate) mod doc_state;
