//! winit + softbuffer 薄壳：事件翻译、脏块失效、帧合成与呈现。
//! 策略全在 editor/input；本层只做接线与像素搬运（INSTANCE §7 接缝）。

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use veil_core::{BlockId, BlockKind, InlineNode};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::autosave::Autosave;
use crate::chrome::{self, Chrome, Viewport};
use crate::docio::encode;
use crate::editor::Editor;
use crate::frame::Frame;
use crate::image::{AssetStore, ImageData, ImageLoader};
use crate::input::Mods;
use crate::layout::{node_active, BlockLayout, LayoutEngine, BASE_LINE_HEIGHT};
use crate::math::{MathImage, MathRenderer};
use crate::table::TableLayout;
use crate::theme::{
    self, Config, ConfigStore, SystemTheme, Theme, ThemeEnv, ThemeMode, WindowEffects,
};

mod document;
mod geometry;
mod interaction;
mod paint;

use geometry::BlockIndex;

/// 外部文件监听轮询间隔（切片 16）：事件在实现侧入队，主线程按此节奏非阻塞取走。
/// 仅在有路径时设置唤醒，草稿空转不产生周期性唤醒。
const WATCH_POLL: Duration = Duration::from_millis(300);
/// 系统外观轮询间隔（切片 17）：仅在「跟随系统」时启用；注册表读取开销可忽略。
const THEME_POLL: Duration = Duration::from_secs(1);

const MARGIN: f32 = 16.0;
const PARA_GAP: f32 = 10.0;

/// 系统剪贴板抽象：实现由产品层（veil）注入，本层不依赖具体剪贴板库（INSTANCE §6）。
pub trait Clipboard {
    fn get(&mut self) -> Option<String>;
    fn set(&mut self, text: &str);
    /// 富文本（HTML）读取（切片 11）：默认 None，纯文本实现无需改动。
    /// 产品层接入系统 HTML 剪贴板；粘贴时优先于纯文本（HTML→Markdown）。
    fn get_html(&mut self) -> Option<String> {
        None
    }
    /// 位图读取（切片 14）：默认 None；无文本/HTML 时粘贴图片落盘并插入链接。
    fn get_image(&mut self) -> Option<ImageData> {
        None
    }
}

/// 文件读写抽象：实现由产品层（veil）注入（同 Clipboard 模式，INSTANCE §6）。
/// 字节级接口：BOM/UTF-8 编解码为本层纯逻辑（docio），实现只做字节搬运。
/// `save` 须原子（产品层用 tmp+rename）：任何失败不得留下半截文件。
pub trait FileStore {
    fn load(&mut self, path: &Path) -> Result<Vec<u8>, String>;
    fn save(&mut self, path: &Path, data: &[u8]) -> Result<(), String>;
}

/// 文件对话框抽象（切片 15）：实现由产品层（veil）注入，本层不依赖具体 GUI 框架。
/// 取消（用户关闭对话框）返回 None；路径选择成功后由 shell 负责读写与状态切换。
pub trait FileDialog {
    fn open_markdown(&mut self) -> Option<PathBuf>;
    fn save_markdown(&mut self, suggested: &str) -> Option<PathBuf>;
}

/// 外部文件变更事件（切片 16）：只关心当前关联文件，不携带路径。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileEvent {
    /// 内容被写入/替换（外部编辑器的原子保存也算）。
    Changed,
    /// 文件被删除或改名离开原路径。
    Removed,
}

/// 文件系统监听抽象（切片 16）：实现由产品层（veil）注入 notify，本层只做策略。
/// 实现须监听目标文件的父目录并过滤目标名，以兼容「tmp + rename」式原子保存；
/// `poll` 非阻塞取走事件，无事件返回空。
pub trait FileWatcher {
    fn watch(&mut self, path: &Path) -> Result<(), String>;
    fn unwatch(&mut self);
    fn poll(&mut self) -> Vec<FileEvent>;
}

/// HTML 导出抽象：实现由产品层（veil）注入（同 FileStore 模式，INSTANCE §6）。
/// 成功返回产物路径；草稿无路径等可预期失败返回 Err（调用方走 stderr，不崩溃）。
pub trait Exporter {
    fn export_html(
        &mut self,
        session: &veil_core::Session,
        source_path: Option<&Path>,
    ) -> Result<PathBuf, String>;
}

/// 待编辑文档：产品层从 CLI 参数或默认样例构造。
pub struct Doc {
    pub text: String,
    /// None = 无关联文件的草稿（Ctrl+S 拒绝，另存为待 chrome UI）。
    pub path: Option<PathBuf>,
    /// 读入时是否剥除了 BOM（保存时还原，保往返字节一致）。
    pub bom: bool,
}

/// 运行编辑器窗口。公式渲染器、图片加载/资产存储与导出器由产品层注入
/// （latex-rust / png / pulldown-cmark，ADR-002）；主题配置与系统外观同样注入（切片 17）。
pub fn run(
    doc: Doc,
    clipboard: Box<dyn Clipboard>,
    store: Box<dyn FileStore>,
    math: Box<dyn MathRenderer>,
    images: Box<dyn ImageLoader>,
    assets: Box<dyn AssetStore>,
    exporter: Box<dyn Exporter>,
    dialogs: Box<dyn FileDialog>,
    watcher: Box<dyn FileWatcher>,
    theme_env: ThemeEnv,
) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("创建事件循环失败: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(
        doc, clipboard, store, math, images, assets, exporter, dialogs, watcher, theme_env,
    );
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("事件循环错误: {e}"))
}

struct App {
    /// 关联文件路径（None = 草稿）；标题由此派生。
    path: Option<PathBuf>,
    /// 读入时的 BOM 标记，保存时还原。
    bom: bool,
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    /// 跨帧复用的像素缓冲（P2）：尺寸不变时零分配，绘制结束整块拷入 surface。
    frame: Option<Frame>,
    editor: Editor,
    engine: LayoutEngine,
    /// 块几何索引（P1a）：全量块枚举的唯一缓存来源，替代每次读取时的 `blocks()`。
    index: BlockIndex,
    /// 块布局与光栅缓存：干净块不重排（脏块由 Events.dirty_blocks 驱动失效）。
    layouts: HashMap<BlockId, BlockLayout>,
    rasters: HashMap<BlockId, (Vec<u8>, u32, u32)>,
    scroll_y: f32,
    mouse: PhysicalPosition<f64>,
    mods: Mods,
    scale: f32,
    /// 垂直移动的目标列（goal column），上下移动时保持。
    goal_x: Option<f32>,
    /// 左键按住中：CursorMoved 转化为拖选（扩选）。
    dragging: bool,
    clipboard: Box<dyn Clipboard>,
    store: Box<dyn FileStore>,
    math: Box<dyn MathRenderer>,
    exporter: Box<dyn Exporter>,
    /// 数学块渲染图缓存 / 失败记忆：脏块驱动失效（同 layouts/rasters）。
    math_imgs: HashMap<BlockId, MathImage>,
    math_failed: HashSet<BlockId>,
    /// 行内公式图缓存 / 失败记忆（切片 8）：按 tex 内容寻址，布局失效即重取，
    /// 无需随编辑显式失效（旧 tex 残留仅费少量内存，DPI 变化时随 scale 清）。
    inline_math: HashMap<String, MathImage>,
    inline_failed: HashSet<String>,
    /// 文档内图片缓存（切片 13）：键 = 解析后的路径，值为自然尺寸位图（绘制时最近邻缩放）；
    /// 失败记忆不每帧重试，打开新文档/拖放时清空。
    images: Box<dyn ImageLoader>,
    inline_images: HashMap<String, ImageData>,
    image_failed: HashSet<String>,
    /// 粘贴图片落盘（切片 14）：产品层实现，视图只插入其返回的相对 URL。
    assets: Box<dyn AssetStore>,
    /// 表格网格缓存（切片 9）：光标离开表格时按需排版；脏块/宽度/DPI 变化即失效。
    tables: HashMap<BlockId, TableLayout>,
    /// 磁盘基线：上次读取/成功保存的字节。保存前与之对比即「内容哈希对比」的字节等价物。
    baseline: Vec<u8>,
    /// 上次内容编辑或自动保存尝试的时刻；None = 无待自动保存内容。
    last_activity: Option<Instant>,
    /// 自动保存曾撞上外部修改：挂起自动保存，等待用户显式保存（绝不静默覆盖）。
    autosave_hold: bool,
    /// 曾发生外部修改冲突且已写备份副本：标题提示，直至下次无冲突保存。
    conflict: bool,
    /// 已备份的外部版本字节：同一版本不重复写冲突副本（自动冲突后手工保存走此路径）。
    external_saved: Option<Vec<u8>>,
    autosave: Autosave,
    /// 自绘 chrome（切片 15）：菜单/状态栏/滚动条交互态与标签缓存。
    chrome: Chrome,
    dialogs: Box<dyn FileDialog>,
    /// 外部文件监听（切片 16）：实现由产品层注入（notify），本层只消费事件。
    watcher: Box<dyn FileWatcher>,
    /// 监听检测到但尚未备份的外部版本：同一版本只提示一次；保存覆盖前才写副本。
    external_changed: Option<Vec<u8>>,
    /// 磁盘文件被删除/不可读的提示去重标记（恢复可读后清除）。
    external_missing: bool,
    /// 主题调色板（切片 17）：全部绘制颜色唯一来源。
    theme: Theme,
    /// 上次读取到的系统外观（跟随系统模式下用于判定是否切换）。
    system_dark: bool,
    /// 上次系统外观轮询时刻（跟随系统模式限频）。
    last_theme_check: Option<Instant>,
    config: Box<dyn ConfigStore>,
    system: Box<dyn SystemTheme>,
    effects: Box<dyn WindowEffects>,
}

impl App {
    fn new(
        doc: Doc,
        clipboard: Box<dyn Clipboard>,
        store: Box<dyn FileStore>,
        math: Box<dyn MathRenderer>,
        images: Box<dyn ImageLoader>,
        assets: Box<dyn AssetStore>,
        exporter: Box<dyn Exporter>,
        dialogs: Box<dyn FileDialog>,
        watcher: Box<dyn FileWatcher>,
        theme_env: ThemeEnv,
    ) -> Self {
        let ThemeEnv {
            mut config,
            mut system,
            effects,
        } = theme_env;
        // 系统外观读不到（非 Windows/注册表异常）按浅色处理：不影响其它功能
        let system_dark = system.is_dark().unwrap_or(false);
        let (mode, config_notice) = match config.load() {
            Ok(text) => match theme::decode_config(text.as_bytes()) {
                Ok(p) if p.ignored == 0 => (p.config.mode, String::new()),
                Ok(p) => (
                    p.config.mode,
                    format!("配置有 {} 处无法识别，已忽略", p.ignored),
                ),
                Err(e) => (ThemeMode::System, format!("配置文件无效，已用默认主题: {e}")),
            },
            // 文件不存在属正常状态（首次运行）：静默用默认配置
            Err(_) => (ThemeMode::System, String::new()),
        };
        let theme = Theme::of(mode, system_dark);
        let mut app = App {
            path: doc.path,
            bom: doc.bom,
            window: None,
            surface: None,
            frame: None,
            editor: Editor::new(&doc.text),
            engine: LayoutEngine::new(),
            index: BlockIndex::default(),
            layouts: HashMap::new(),
            rasters: HashMap::new(),
            scroll_y: 0.0,
            mouse: PhysicalPosition::new(0.0, 0.0),
            mods: Mods::default(),
            scale: 1.0,
            goal_x: None,
            dragging: false,
            clipboard,
            store,
            math,
            exporter,
            math_imgs: HashMap::new(),
            math_failed: HashSet::new(),
            inline_math: HashMap::new(),
            inline_failed: HashSet::new(),
            images,
            inline_images: HashMap::new(),
            image_failed: HashSet::new(),
            assets,
            tables: HashMap::new(),
            // 入参文本即磁盘内容（产品层解码所得）：编码还原后与源字节逐字节一致
            baseline: encode(&doc.text, doc.bom),
            last_activity: None,
            autosave_hold: false,
            conflict: false,
            external_saved: None,
            autosave: Autosave::default(),
            chrome: Chrome::default(),
            dialogs,
            watcher,
            external_changed: None,
            external_missing: false,
            theme,
            system_dark,
            last_theme_check: None,
            config,
            system,
            effects,
        };
        app.chrome.notice = config_notice;
        app.refresh_index();
        app.recompute_geometry();
        app.watch_current();
        app
    }

    /// 窗口外观效果（深色标题栏/Mica）：窗口创建或主题切换时调用；无窗口/非 Win32 静默跳过。
    /// 效果失败不影响内容绘制（内容区始终为不透明底色，Win10 上即纯色外观）。
    fn apply_window_effects(&mut self) {
        #[cfg(windows)]
        {
            use crate::theme::NativeWindow;
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let Some(win) = &self.window else {
                return;
            };
            let Ok(handle) = win.window_handle() else {
                return;
            };
            if let RawWindowHandle::Win32(p) = handle.as_raw() {
                self.effects
                    .apply(NativeWindow { hwnd: p.hwnd.get() }, self.theme.dark);
            }
        }
        #[cfg(not(windows))]
        let _ = &self.effects;
    }

    /// 主题切换后刷新按颜色生成的全部缓存：文本/表格/公式光栅须按新前景色重做，
    /// 布局几何与图片位图不受影响。窗口外观同步更新。
    fn refresh_theme_visuals(&mut self) {
        self.rasters.clear();
        self.tables.clear();
        self.math_imgs.clear();
        self.inline_math.clear();
        self.chrome.clear_label_cache();
        // 清缓存后可能按新调色板重排/重渲：只重做可见块，几何按新高度重算
        self.refresh_index();
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        self.apply_window_effects();
        self.request_redraw();
    }

    /// 切换主题模式并持久化。切到「跟随系统」时立即重读一次系统外观，避免沿用陈旧值。
    fn set_theme_mode(&mut self, mode: ThemeMode) {
        if mode == ThemeMode::System {
            if let Some(dark) = self.system.is_dark() {
                self.system_dark = dark;
            }
        }
        self.theme = Theme::of(mode, self.system_dark);
        self.last_theme_check = None;
        self.refresh_theme_visuals();
        // 先给成功提示再落盘：保存失败时错误提示更重要，覆盖为「配置未保存」
        self.chrome.notice = format!("主题：{}", mode.label());
        self.save_config();
        self.request_redraw();
    }

    /// 配置落盘（极小文本、全量重写；实现侧保证原子与父目录创建）。
    fn save_config(&mut self) {
        let text = Config {
            mode: self.theme.mode,
        }
        .to_toml();
        if let Err(e) = self.config.save(&text) {
            self.chrome.notice = format!("配置未保存: {e}");
            eprintln!("配置保存失败: {e}");
        }
    }

    /// 跟随系统时轮询系统外观（限频）；外观切换则重建调色板并刷新颜色缓存。
    fn poll_system_theme(&mut self, now: Instant) {
        if self.theme.mode != ThemeMode::System {
            return;
        }
        if self
            .last_theme_check
            .is_some_and(|t| now.duration_since(t) < THEME_POLL)
        {
            return;
        }
        self.last_theme_check = Some(now);
        let Some(dark) = self.system.is_dark() else {
            return;
        };
        if dark != self.system_dark {
            self.system_dark = dark;
            self.theme = Theme::of(ThemeMode::System, dark);
            self.refresh_theme_visuals();
        }
    }

    fn win_size(&self) -> (u32, u32) {
        self.window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width.max(1), s.height.max(1))
            })
            .unwrap_or((800, 600))
    }

    fn viewport(&self) -> Viewport {
        let (w, h) = self.win_size();
        chrome::viewport(w, h, self.scale)
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn set_ime_area(&mut self) {
        let Some(w) = self.window.clone() else {
            return; // 无窗口（测试）：不触发按需排版
        };
        let (x, y, h) = self.caret_screen();
        w.set_ime_cursor_area(
            PhysicalPosition::new(x as f64, y as f64),
            PhysicalSize::new(2.0, h as f64),
        );
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(self.title_text())
            .with_inner_size(LogicalSize::new(900.0, 700.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("创建窗口失败: {e}");
                event_loop.exit();
                return;
            }
        };
        window.set_ime_allowed(true);
        self.scale = window.scale_factor() as f32;
        let context = match Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("创建呈现上下文失败: {e}");
                event_loop.exit();
                return;
            }
        };
        self.surface = Surface::new(&context, window.clone()).ok();
        self.set_ime_area();
        self.window = Some(window);
        self.apply_window_effects();
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(s), Some(w), Some(h)) = (
                    self.surface.as_mut(),
                    NonZeroU32::new(size.width),
                    NonZeroU32::new(size.height),
                ) {
                    let _ = s.resize(w, h);
                }
                self.layouts.clear(); // 宽度变化：全量重排（换行依赖宽度）
                self.rasters.clear();
                self.tables.clear(); // 列宽/折行依赖版心宽
                // 清缓存后宽度/块高都可能变化：只重排可见块并立即重建几何
                self.refresh_index();
                self.ensure_layouts_visible();
                self.ensure_tables_visible();
                self.recompute_geometry();
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(w) = &self.window {
                    self.scale = w.scale_factor() as f32;
                }
                self.layouts.clear();
                self.rasters.clear();
                self.math_imgs.clear(); // DPI 变化：按新 scale 重渲
                self.math_failed.clear();
                self.inline_math.clear();
                self.inline_failed.clear();
                self.tables.clear(); // 网格尺寸随 DPI 变化
                self.chrome.clear_label_cache(); // chrome 标签按旧 scale 光栅化
                // scale 变了，几何累计均按新 scale：索引重建、可见块按新 DPI 重排
                self.refresh_index();
                self.ensure_layouts_visible();
                self.ensure_math_visible();
                self.ensure_tables_visible();
                self.recompute_geometry();
                self.request_redraw();
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                self.editor.set_preedit(text, cursor);
                self.request_redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                if self.chrome.is_open() {
                    return; // 菜单打开：吞掉输入
                }
                let old_caret = self.editor.caret();
                let dirty = self.editor.commit_ime(text);
                self.invalidate_reveal(old_caret, self.editor.caret());
                self.after_edit(dirty);
                self.set_ime_area();
            }
            WindowEvent::Ime(Ime::Disabled) => self.editor.clear_preedit(),
            WindowEvent::DroppedFile(path) => self.open_path(path), // 拖放打开
            WindowEvent::KeyboardInput { event, .. } => self.on_key(&event),
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.mods = Mods {
                    ctrl: s.control_key(),
                    shift: s.shift_key(),
                };
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = position;
                if let Some(off) = self.chrome.scroll_drag {
                    // 拖动滚动条：按滑块顶边换算滚动量
                    let vp = self.viewport();
                    let doc_h = self.doc_height();
                    self.scroll_y =
                        chrome::scroll_from_thumb(vp, doc_h, self.mouse.y as f32 - off, self.scale);
                    self.set_ime_area();
                    self.request_redraw();
                } else if self.dragging {
                    self.point_caret(true); // 拖选：扩选跟随
                } else {
                    self.update_hover();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if self.on_left_click() {
                    event_loop.exit();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = false;
                self.chrome.scroll_drag = None;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 3.0 * BASE_LINE_HEIGHT * self.scale,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                let vp = self.viewport();
                let max = chrome::max_scroll(vp, self.doc_height());
                self.scroll_y = (self.scroll_y - d).clamp(0.0, max);
                self.set_ime_area();
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.paint(),
            // 窗口重新获得焦点：立即校验系统外观（限频轮询最多滞后 1s，此处消除等待）
            WindowEvent::Focused(true) => {
                self.last_theme_check = None;
                self.poll_system_theme(Instant::now());
            }
            _ => {}
        }
    }

    /// 空闲回调：取外部变更事件、校验系统外观、尝试自动保存，并把下次唤醒定在
    /// 「自动保存静默期终点」「监听轮询」「系统外观轮询」的较早者
    /// （草稿且显式主题下无路径则完全休眠）。
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.poll_external(); // 先于自动保存：外部修改须先挂起，绝不在本轮静默覆盖
        self.poll_system_theme(now);
        self.maybe_autosave(now);
        let pending = self.editor.is_dirty()
            && self.path.is_some()
            && self.editor.preedit().is_none()
            && !self.autosave_hold;
        let mut next = if pending {
            self.autosave.deadline(self.last_activity)
        } else {
            None
        };
        // 系统外观只可能影响「跟随系统」模式；显式主题/无窗口时不唤醒
        if self.theme.mode == ThemeMode::System && self.window.is_some() {
            let theme_at = now + THEME_POLL;
            next = Some(next.map_or(theme_at, |t| t.min(theme_at)));
        }
        if self.path.is_some() {
            let watch_at = now + WATCH_POLL;
            next = Some(next.map_or(watch_at, |t| t.min(watch_at)));
        }
        event_loop.set_control_flow(match next {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}

#[cfg(test)]
mod tests;
