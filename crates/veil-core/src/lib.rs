//! 策略层 (Strategy)：数据模型 + 纯计算，无 IO，无 GUI。
//! veil-core：Veil 的纯逻辑内核。零 GUI、零异步运行时依赖（INSTANCE §2 硬门禁）。
//!
//! 唯一公开入口是 [`Session`] 门面（docs/01-define-design.md B2 深模块判定）。
//! Markdown 源码是唯一真值；内核单线程。

mod buffer;
mod callouts;
mod history;
mod link_safety;
mod mermaid;
mod range;
mod session;
mod slug;
mod syntax;
pub mod toc;

pub use callouts::CalloutRegistry;
pub use link_safety::{link_scheme, link_url_is_safe, percent_decode_path};
pub use mermaid::{
    detect_type, MermaidTemplate, CLASS_TEMPLATE, ER_TEMPLATE, FLOWCHART_TEMPLATE,
    SEQUENCE_TEMPLATE, STATE_TEMPLATE,
};
pub use range::{Bias, MarkId, Range};
pub use session::{Edit, EditError, Events, Session};
pub use slug::{heading_slug, heading_slugs};
pub use syntax::{
    block_is_image_only, emphasis_spans, image_standalone, list_signature, sanitize_style,
    should_strip_html, split_table_row, standalone_image_caption, task_span, AdmonitionType,
    BlockContainer, BlockId, BlockKind, Callout, CellAlign, EmphasisSpan, HtmlAttrs, HtmlStyle,
    HtmlTag, ImageAlign, InlineKind, InlineNode, Table, TableCell, HTML_INLINE_WHITELIST,
};
pub use toc::{generate_toc, is_toc_marker};
