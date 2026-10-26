//! veil-egui：视图适配器（阶段二）。
//! 切片 1 栈：winit 直收事件（IME 走 spike 验证路径）+ softbuffer 软件呈现 + cosmic-text 排版。
//! egui/GL 栈引入前提已由切片 15 推翻：自绘 chrome + 系统对话框，保持零 GUI 框架依赖。
#![allow(clippy::too_many_arguments)] // 布局/绘制/门面参数天然多，拆 struct 反而遮蔽语义
#![allow(clippy::chunks_exact_to_as_chunks)] // 通道展开用 chunks_exact(4) 更直观
//!
//! 分层：editor/input/style/frame 纯逻辑零 GUI（机制与策略分离，全部 headless 可测）；
//! layout 依赖 cosmic-text（字体相关断言从宽）；shell 为 winit/softbuffer 薄胶合。

mod autosave;
mod chrome;
mod docio;
mod document;
mod editor;
mod html;
mod image;
mod input;
mod math;
mod shell;
mod theme;
mod view;

pub use autosave::Autosave;
pub use chrome::{Chrome, ExternalState, MenuAction, Viewport};
pub use docio::{decode, encode};
pub use editor::{Editor, Intent, Preedit};
pub use html::html_to_markdown;
pub use image::{AssetStore, ImageData, ImageLoader};
pub use input::{translate, Action, Key, Mods};
pub use math::{MathImage, MathRenderer};
pub use shell::{
    run, Clipboard, Doc, Exporter, FileDialog, FileEvent, FileStore, FileWatcher,
};
pub use theme::{
    config_path, ConfigStore, NativeWindow, SystemTheme, Theme, ThemeEnv, ThemeMode, WindowEffects,
};
pub use view::{
    style_spans, BlockLayout, Frame, InlineAsset, InlineImg, LayoutEngine, PlacedImg, Style,
    TableCellLayout, TableLayout, TableRow, BASE_FONT_SIZE, BASE_LINE_HEIGHT,
};
