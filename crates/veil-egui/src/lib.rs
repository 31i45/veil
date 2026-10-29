//! veil-egui：视图适配器（阶段二）。
//! 切片 1 栈：winit 直收事件（IME 走 spike 验证路径）+ softbuffer 软件呈现 + cosmic-text 排版。
//! egui/GL 栈引入前提已由切片 15 推翻：自绘 chrome + 系统对话框，保持零 GUI 框架依赖。
#![allow(clippy::too_many_arguments)] // 布局/绘制/门面参数天然多，拆 struct 反而遮蔽语义
#![allow(clippy::chunks_exact_to_as_chunks)] // 通道展开用 chunks_exact(4) 更直观
//!
//! 分层：editor/input/style/frame 纯逻辑零 GUI（机制与策略分离，全部 headless 可测）；
//! layout 依赖 cosmic-text（字体相关断言从宽）；app 为 winit/softbuffer 薄胶合。
//!
//! 公共导出（ADR-009 S5）：严格以产品层实际使用定义——只暴露端口 trait、启动入口、
//! 解码函数、端口关联类型。内部类型（Editor / Chrome / Frame / LayoutEngine 等）不导出。

mod app;
mod autosave;
mod chrome;
mod docio;
mod document;
mod editor;
mod geometry;
mod html;
mod image;
mod input;
mod interaction;
mod math;
mod mode;
mod paint;
mod stats;
mod theme;
mod view;

#[cfg(test)]
mod app_tests;

pub use app::{
    run, Clipboard, Doc, Exporter, FileDialog, FileEvent, FileStore, FileWatcher, PersistEnv,
    RecoveryStore, SessionStore,
};
pub use docio::decode;
pub use image::{AssetStore, ImageData, ImageLoader};
pub use math::{MathImage, MathRenderer};
pub use theme::{
    config_path, data_dir, ConfigStore, NativeWindow, SystemTheme, ThemeEnv, WindowEffects,
};
// bench_typing（产品层纯度量 example）需要 LayoutEngine / Frame 做纯排版与帧分配基准
pub use view::{BlockLayout, Frame, LayoutEngine, BASE_FONT_SIZE, BASE_LINE_HEIGHT};
