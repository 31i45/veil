//! veil-core：Veil 的纯逻辑内核。零 GUI、零异步运行时依赖（INSTANCE §2 硬门禁）。
//!
//! 唯一公开入口是 [`Session`] 门面（docs/01-define-design.md B2 深模块判定）。
//! Markdown 源码是唯一真值；注解不持久化；内核单线程。

mod annotate;
mod buffer;
mod history;
mod range;
mod session;
mod syntax;

pub use annotate::{AnnotationId, AnnotationKind};
pub use range::{Bias, MarkId, Range};
pub use session::{Edit, EditError, Events, Session};
pub use syntax::{BlockId, BlockKind, CellAlign, InlineKind, InlineNode, Table, TableCell};
