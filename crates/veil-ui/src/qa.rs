//! 仅 T5 语料 harness 使用的只读门面（`#[doc(hidden)]`）。
//!
//! 产品路径不经过本模块：它只把既有的 headless `App`（无窗口、无副作用端口）
//! 装配起来，并向集成测试暴露块几何、纯文本、模式开关、光标带与打印分页模型。
//! 不新增依赖、不改变任何产品行为；所有端口均为空实现。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::app::{
    App, Clipboard, Clock, Doc, Exporter, FileDialog, FileEvent, FileStore, FileWatcher, Ports,
    RecoveryStore, SessionStore, SnapshotMeta, SnapshotStore, UrlOpener,
};
use crate::chrome;
use crate::image::{AssetMigration, AssetStore, ImageData, ImageLoader};
use crate::math::{MathImage, MathRenderer};
use crate::mermaid_renderer::{MermaidImage, MermaidRenderer};
use crate::print::{NoPrint, PrintSession};
use crate::theme::{ConfigStore, ExportStyle, NativeWindow, SystemTheme, ThemeEnv, WindowEffects};
use veil_core::{BlockKind, Range};

/// 单块几何（逻辑像素，文档坐标）：用途仅为 harness 的不重叠/越界断言。
pub struct QaBlock {
    pub kind: BlockKind,
    pub top: f32,
    pub height: f32,
    /// 因祖先 `<details>` 折叠而不可见：此时高度 0 属预期，harness 不得判为缺陷。
    pub details_hidden: bool,
}

/// 打字机模式下的光标带（物理像素，屏幕坐标）。
pub struct QaCaret {
    pub y: f32,
    pub h: f32,
    pub viewport_y: f32,
    pub viewport_h: f32,
    pub scroll: f32,
    pub max_scroll: f32,
}

/// 打印模型的分页结构。
pub struct QaPrint {
    pub pages: usize,
    pub breaks: Vec<f32>,
    pub total_h: f32,
}

/// headless App 包装：构造后即完成索引/字体系统初始化，无窗口与文件副作用。
pub struct QaApp {
    app: App,
}

impl QaApp {
    pub fn new(text: &str) -> Self {
        Self {
            app: App::new(
                Doc {
                    text: text.to_string(),
                    path: None,
                    bom: false,
                    resume: false,
                    open_error: None,
                },
                qa_ports(),
            ),
        }
    }

    /// 触发一次真实渲染（视口驱动惰性排版 + 几何重算）。
    pub fn render(&mut self) {
        let _ = self.app.render();
    }

    pub fn blocks(&self) -> Vec<QaBlock> {
        self.app
            .index
            .blocks
            .iter()
            .map(|g| QaBlock {
                kind: g.kind.clone(),
                top: g.top,
                height: g.height,
                details_hidden: g.details_hidden,
            })
            .collect()
    }

    pub fn total_h(&self) -> f32 {
        self.app.index.total_h
    }

    /// 文档源文本（Markdown 原文；模式开关是纯视觉，不应改变它）。
    pub fn text(&self) -> String {
        let s = self.app.editor.session();
        s.text_slice(Range::new(0, s.text_len()))
            .unwrap_or_default()
    }

    pub fn toggle_focus(&mut self) {
        self.app.toggle_focus();
    }

    pub fn toggle_typewriter(&mut self) {
        self.app.toggle_typewriter();
    }

    /// 把光标放到中段块起点（多字节安全：取块区间起点，必为字符边界）。
    pub fn set_caret_middle(&mut self) {
        let n = self.app.index.blocks.len();
        if n == 0 {
            return;
        }
        let start = self.app.index.blocks[n / 2].range.start;
        self.app.editor.set_caret(start);
    }

    pub fn scroll_to_caret(&mut self) {
        self.app.scroll_to_caret();
    }

    pub fn caret(&mut self) -> QaCaret {
        let vp = self.app.viewport();
        let (_, y, h) = self.app.caret_screen();
        let max_scroll = chrome::max_scroll(vp, self.app.doc_height());
        QaCaret {
            y,
            h,
            viewport_y: vp.y,
            viewport_h: vp.h,
            scroll: self.app.window_state.scroll_y,
            max_scroll,
        }
    }

    /// 打印模型：进入打印态（全量重排 + 分页），取页数/断点后立即恢复屏幕态。
    /// 物理打印（GDI 设备上下文）不在本门面内，harness 一律记 NOT RUN。
    pub fn print_model(&mut self, w: u32, h: u32) -> QaPrint {
        let mut session = PrintSession::for_qa(&mut self.app);
        let pages = session.begin(w, h, 1.0);
        let breaks = session.page_breaks().to_vec();
        // 打印态下的文档高度（页帧语义）；`end()` 后恢复为屏幕语义，故须先取。
        let total_h = session.app.doc_height();
        session.end();
        QaPrint {
            pages,
            breaks,
            total_h,
        }
    }
}

// ---- 端口：全部空实现（headless，无 IO/无 GUI） ----

struct QaClipboard;
impl Clipboard for QaClipboard {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _text: &str) {}
}

struct QaStore;
impl FileStore for QaStore {
    fn load(&mut self, path: &Path) -> Result<Vec<u8>, String> {
        Err(format!("qa store: 不读取文件（{}）", path.display()))
    }
    fn save(&mut self, _path: &Path, _data: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

struct QaClock;
impl Clock for QaClock {
    fn local_hhmm(&self) -> String {
        "00:00".into()
    }
}

struct QaMath;
impl MathRenderer for QaMath {
    fn render(
        &mut self,
        _tex: &str,
        _display: bool,
        _scale: f32,
        _color: [u8; 3],
    ) -> Result<MathImage, String> {
        Err("qa matherer: 未注入公式渲染器".into())
    }
}

struct QaMermaid;
impl MermaidRenderer for QaMermaid {
    fn request(&mut self, _label: String, _source: String, _dark: bool) {}
    fn poll(&mut self) -> Vec<(String, Result<MermaidImage, String>)> {
        Vec::new()
    }
    fn set_dpi_scale(&mut self, _scale: f32) {}
}

struct QaImages;
impl ImageLoader for QaImages {
    fn load(&mut self, path: &Path) -> Result<ImageData, String> {
        Err(format!("qa images: 不读取图片（{}）", path.display()))
    }
}

struct QaAssets;
impl AssetStore for QaAssets {
    fn save_pasted_image(&mut self, _doc: &Path, _image: &ImageData) -> Result<String, String> {
        Err("qa assets: 只读".into())
    }
    fn migrate_relative_assets(
        &mut self,
        _old_doc: &Path,
        _new_doc: &Path,
        _urls: &[String],
    ) -> AssetMigration {
        AssetMigration::default()
    }
}

struct QaExporter;
impl Exporter for QaExporter {
    fn export_html(
        &mut self,
        _session: &veil_core::Session,
        _style: ExportStyle,
        _source_path: Option<&Path>,
        _target: Option<&Path>,
        _mermaids: Option<&HashMap<veil_core::BlockId, MermaidImage>>,
    ) -> Result<PathBuf, String> {
        Err("qa exporter: 只读".into())
    }
}

struct QaDialogs;
impl FileDialog for QaDialogs {
    fn open_markdown(&mut self) -> Option<PathBuf> {
        None
    }
    fn save_markdown(&mut self, _suggested: &str) -> Option<PathBuf> {
        None
    }
}

struct QaWatcher;
impl FileWatcher for QaWatcher {
    fn watch(&mut self, _path: &Path) -> Result<(), String> {
        Ok(())
    }
    fn unwatch(&mut self) {}
    fn poll(&mut self) -> Vec<FileEvent> {
        Vec::new()
    }
}

struct QaUrl;
impl UrlOpener for QaUrl {
    fn open_url(&mut self, _url: &str) -> Result<(), String> {
        Ok(())
    }
}

struct QaConfig;
impl ConfigStore for QaConfig {
    fn load(&mut self) -> Result<String, String> {
        Err("qa config: 无配置文件".into())
    }
    fn save(&mut self, _text: &str) -> Result<(), String> {
        Ok(())
    }
}

struct QaSystem;
impl SystemTheme for QaSystem {
    fn is_dark(&mut self) -> Option<bool> {
        None
    }
}

struct QaEffects;
impl WindowEffects for QaEffects {
    fn apply(&mut self, _window: NativeWindow, _dark: bool) {}
}

struct QaRecovery;
impl RecoveryStore for QaRecovery {
    fn load(&mut self) -> Result<Vec<u8>, String> {
        Err("qa recovery: 空槽".into())
    }
    fn save(&mut self, _data: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn clear(&mut self) -> Result<(), String> {
        Ok(())
    }
}

struct QaSession;
impl SessionStore for QaSession {
    fn load(&mut self) -> Result<String, String> {
        Err("qa session: 空".into())
    }
    fn save(&mut self, _text: &str) -> Result<(), String> {
        Ok(())
    }
}

struct QaSnapshots;
impl SnapshotStore for QaSnapshots {
    fn save_snapshot(
        &mut self,
        _doc_key: u64,
        _snapshot_text: &str,
    ) -> Result<SnapshotMeta, String> {
        Err("qa snapshots: 只读".into())
    }
    fn list_snapshots(&mut self, _doc_key: u64) -> Result<Vec<SnapshotMeta>, String> {
        Ok(Vec::new())
    }
    fn read_snapshot(&mut self, _doc_key: u64, _ts_ms: u64) -> Result<String, String> {
        Err("qa snapshots: 只读".into())
    }
    fn delete_snapshot(&mut self, _doc_key: u64, _ts_ms: u64) -> Result<(), String> {
        Ok(())
    }
    fn clear_snapshots(&mut self, _doc_key: u64) -> Result<usize, String> {
        Ok(0)
    }
    fn prune(&mut self, _doc_key: u64, _keep_newest: usize) -> Result<usize, String> {
        Ok(0)
    }
    fn migrate_key(&mut self, _from: u64, _to: u64) -> Result<usize, String> {
        Ok(0)
    }
}

fn qa_ports() -> Ports {
    Ports {
        clipboard: Box::new(QaClipboard),
        store: Box::new(QaStore),
        clock: Box::new(QaClock),
        math: Box::new(QaMath),
        mermaid: Box::new(QaMermaid),
        images: Box::new(QaImages),
        assets: Box::new(QaAssets),
        exporter: Box::new(QaExporter),
        dialogs: Box::new(QaDialogs),
        watcher: Box::new(QaWatcher),
        theme_env: ThemeEnv {
            config: Box::new(QaConfig),
            system: Box::new(QaSystem),
            effects: Box::new(QaEffects),
            callouts: veil_core::CalloutRegistry::default(),
        },
        persist_env: crate::app::PersistEnv {
            recovery: Box::new(QaRecovery),
            session: Box::new(QaSession),
            snapshot: Box::new(QaSnapshots),
        },
        print: Box::new(NoPrint),
        url_opener: Box::new(QaUrl),
        ime_anchor: Box::new(crate::app::NoopImeAnchor),
    }
}
