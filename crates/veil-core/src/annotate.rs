//! 注解：挂载在 mark 上的临时元信息，不修改源码、不持久化（INSTANCE §3）。
//! 经 mark 间接寻址：文本编辑时 mark 自动跟随，注解集合无需重排——
//! 这是相对 BTreeSet<Range> 方案的关键优势（键不可变问题不存在）。

use crate::range::MarkId;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AnnotationKind {
    /// 展开显示该区间内的 inline 标记（inline-WYSIWYG 核心交互）。
    Reveal,
    /// 公式等后台渲染结果的缓存句柄；块 dirty 时由视图层废弃。
    BlockRenderCacheId(u64),
    Custom(String),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct AnnotationId(pub u64);

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Annotation {
    pub id: AnnotationId,
    pub kind: AnnotationKind,
    pub start: MarkId,
    pub end: MarkId,
}
