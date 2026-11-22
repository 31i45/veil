//! 机制层 (Mechanism)：渲染管线 + 平台桥接，依赖策略层。
//! veil-ui：视图适配器（阶段二）。
//! 切片 1 栈：winit 直收事件（IME 走 spike 验证路径）+ softbuffer 软件呈现 + cosmic-text 排版。
//! egui/GL 栈引入前提已由切片 15 推翻：自绘 chrome 与自绘模态，保持零 GUI 框架依赖。
//! 公共 API 已由 Ports struct 收敛（run/doc 2 参数），不再需要 too_many_arguments 豁免。
#![allow(clippy::chunks_exact_to_as_chunks)]
// 通道展开用 chunks_exact(4) 更直观
// 内部绘制/布局函数参数天然多，在各自模块局部标注 #[allow]，不做全局豁免
//!
//! 分层：editor/input/style/frame 纯逻辑零 GUI（机制与策略分离，全部 headless 可测）；
//! layout 依赖 cosmic-text（字体相关断言从宽）；app 为 winit/softbuffer 薄胶合。
//!
//! 公共导出（ADR-009 S5）：严格以产品层实际使用定义——只暴露端口 trait、启动入口、
//! 解码函数、端口关联类型。内部类型（Editor / Chrome / Frame / LayoutEngine 等）不导出。

mod anim;
mod app;
mod autosave;
mod chrome;
mod docio;
mod document;
mod editor;
mod editor_helpers;
mod geometry;
mod html;
pub mod i18n;
mod icon;
mod image;
mod input;
mod interaction;
mod interaction_menu;
mod math;
mod mermaid_renderer;
mod mode;
mod paint;
mod paint_overlay;
mod paint_table;
mod perf;
mod print;
mod shell;
mod stats;
mod theme;
mod tokens;
mod view;

#[cfg(test)]
mod app_tests;
// 测试内按原路径加载产品平台错误框实现时，保持其 veil_ui 引用不变。
#[cfg(test)]
extern crate self as veil_ui;

pub use app::{
    content_hash, format_unix_ms_utc, normalize_link_url, url_is_openable, Clipboard, Clock, Doc,
    Exporter, FileDialog, FileEvent, FileStore, FileWatcher, PersistEnv, Ports, RecoveryStore,
    SessionStore, SnapshotMeta, SnapshotStore, StartupError, UrlOpener,
};
pub use chrome::path_label;
pub use docio::decode;
pub use image::{AssetMigration, AssetStore, ImageData, ImageLoader};
pub use math::{MathImage, MathRenderer};
pub use mermaid_renderer::{MermaidImage, MermaidRenderer};
pub use print::{ExportOptions, NoPrint, Print, PrintSession};
pub use shell::run;
pub use theme::{
    config_path, data_dir, ConfigStore, ExportStyle, NativeWindow, SystemTheme, ThemeEnv,
    WindowEffects,
};
// bench_typing（产品层纯度量 example）需要 LayoutEngine / Frame 做纯排版与帧分配基准
pub use view::{BlockLayout, Frame, LayoutEngine, BASE_FONT_SIZE, BASE_LINE_HEIGHT};
