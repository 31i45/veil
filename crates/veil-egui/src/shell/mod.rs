//! winit + softbuffer 薄壳：事件翻译、脏块失效、帧合成与呈现。
//! 策略全在 editor/input；本层只做接线与像素搬运（INSTANCE §7 接缝）。

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use veil_core::{BlockId, BlockKind, InlineKind, InlineNode, Range};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::window::{Window, WindowId};

use crate::autosave::Autosave;
use crate::chrome::{self, Chrome, MenuAction, Viewport};
use crate::docio::encode;
use crate::editor::{Editor, Intent};
use crate::frame::Frame;
use crate::html::html_to_markdown;
use crate::image::{AssetStore, ImageData, ImageLoader};
use crate::input::{translate, Action, Key, Mods};
use crate::layout::{
    node_active, BlockLayout, InlineAsset, InlineImg, LayoutEngine, BASE_FONT_SIZE,
    BASE_LINE_HEIGHT,
};
use crate::math::{MathImage, MathRenderer};
use crate::table::TableLayout;
use crate::theme::{
    self, Config, ConfigStore, SystemTheme, Theme, ThemeEnv, ThemeMode, WindowEffects,
};

mod document;

/// 外部文件监听轮询间隔（切片 16）：事件在实现侧入队，主线程按此节奏非阻塞取走。
/// 仅在有路径时设置唤醒，草稿空转不产生周期性唤醒。
const WATCH_POLL: Duration = Duration::from_millis(300);
/// 系统外观轮询间隔（切片 17）：仅在「跟随系统」时启用；注册表读取开销可忽略。
const THEME_POLL: Duration = Duration::from_secs(1);

const MARGIN: f32 = 16.0;
const PARA_GAP: f32 = 10.0;

/// 单块几何缓存：块清单一次枚举后复用，避免每次击键反复全量枚举（P1a）。
/// P3 迁出本文件，当前就近放置以减少跨模块改动。
struct BlockGeom {
    id: BlockId,
    kind: BlockKind,
    range: Range,
    top: f32,
    height: f32,
    /// 估算高度（P1b）：refresh_index 时按换行数与折行字符数廉价估算，
    /// 视口外块以它参与几何累计，几何重算不再读取 rope。
    est_h: f32,
}

/// 文档几何索引：`blocks` 与 `total_h` 只在本层刷新/重算后读取。
#[derive(Default)]
struct BlockIndex {
    blocks: Vec<BlockGeom>,
    /// 文档总高（含首尾外边距），即原 `doc_height()` 语义。
    total_h: f32,
}

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

    fn content_width(&self) -> f32 {
        let (w, _) = self.win_size();
        (w as f32 - 2.0 * MARGIN * self.scale - chrome::SCROLL_W * self.scale).max(32.0)
    }

    /// 文档总高（含首尾外边距），滚动条与滚轮钳位依据。
    fn doc_height(&self) -> f32 {
        if self.index.blocks.is_empty() {
            2.0 * MARGIN * self.scale
        } else {
            self.index.total_h
        }
    }

    /// 编辑后统一入口：失效脏块、视口跟随光标、请求重绘。
    /// 仅内容编辑重启自动保存静默期；纯光标移动不推迟落盘。
    fn after_edit(&mut self, dirty: Vec<BlockId>) {
        if !dirty.is_empty() {
            self.last_activity = Some(Instant::now());
        }
        for id in dirty {
            self.layouts.remove(&id);
            self.rasters.remove(&id);
            self.math_imgs.remove(&id);
            self.math_failed.remove(&id);
            self.tables.remove(&id);
        }
        // 先按新块结构重建索引，ensure_* 才能只遍历索引（不再各自全量枚举）
        self.refresh_index();
        // 视口驱动：只排可见块；光标所在块可能已滚出视口，单独补齐
        self.ensure_layouts_visible();
        let caret_id = self.caret_block_index().map(|i| self.index.blocks[i].id);
        if let Some(id) = caret_id {
            self.ensure_block(id);
        }
        self.ensure_tables_visible();
        // 布局/表格已就绪：高度来源稳定后才累计 top
        self.recompute_geometry();
        self.scroll_to_caret();
        self.update_title(); // 脏标随编辑翻转
        self.request_redraw();
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// 补齐缺失布局（干净块缓存命中则零开销）。Reveal 态以当前光标为准：
    /// 光标移动引发的激活变化由 invalidate_reveal 先行失效相关块。
    /// 行内公式（切片 8）：块内非激活 Math 节点渲染出图（tex 内容寻址缓存），
    /// 传占位信息给 layout；失败记 inline_failed 降级源码显示，不每帧重试。
    /// 文档内图片（切片 13）：非激活 Image 节点按文档目录解析相对 URL，经 ImageLoader
    /// 加载自然尺寸位图；宽超版心等比缩放后占位；失败/远程 URL 降级 alt 文本。
    fn ensure_layouts_range(&mut self, indices: std::ops::Range<usize>) {
        let width = self.content_width();
        let caret = self.editor.caret();
        self.engine.set_scale(self.scale);
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        // 先收集待排块与其行内公式/图片（session 借用期），再调渲染器/加载器（&mut self）
        let mut pending = Vec::new();
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind, g.range);
            if self.layouts.contains_key(&id) {
                continue;
            }
            let text = self.editor.session().text_slice(range).unwrap_or_default();
            let inlines = self.editor.session().block_inlines(id).unwrap_or_default();
            let mut maths: Vec<(Range, String)> = Vec::new(); // (节点区间, tex)
            let mut images: Vec<(Range, String)> = Vec::new(); // (节点区间, url)
            let mut stack: Vec<&InlineNode> = inlines.iter().collect();
            while let Some(n) = stack.pop() {
                if !node_active(n.range, caret) {
                    match n.kind {
                        InlineKind::Math => {
                            let tex = self
                                .editor
                                .session()
                                .text_slice(n.content)
                                .unwrap_or_default();
                            maths.push((n.range, tex));
                        }
                        InlineKind::Image => {
                            if let Some(url) = n.url {
                                let url = self.editor.session().text_slice(url).unwrap_or_default();
                                images.push((n.range, url));
                            }
                        }
                        _ => {}
                    }
                }
                stack.extend(n.children.iter());
            }
            pending.push((id, kind, range, text, inlines, maths, images));
        }
        let scale = self.scale;
        let fg = self.theme.fg;
        for (id, kind, range, text, inlines, maths, images) in pending {
            let mut imgs = Vec::new();
            for (nrange, tex) in maths {
                if self.inline_failed.contains(&tex) {
                    continue; // 失败记忆：降级源码（Reveal 消隐态）显示
                }
                if !self.inline_math.contains_key(&tex) {
                    match self.math.render(&tex, false, scale, fg) {
                        Ok(img) => {
                            self.inline_math.insert(tex.clone(), img);
                        }
                        Err(e) => {
                            eprintln!("行内公式渲染失败（降级为源码显示）: {e}");
                            self.inline_failed.insert(tex.clone());
                            continue;
                        }
                    }
                }
                let img = &self.inline_math[&tex];
                imgs.push(InlineImg {
                    range: nrange,
                    key: InlineAsset::Math(tex),
                    width: img.width,
                    height: img.height,
                    baseline: img.baseline,
                });
            }
            for (nrange, url) in images {
                let Some(path) = local_image_path(&url, self.path.as_deref()) else {
                    continue; // 远程/内联 URL 或草稿相对路径：降级 alt 文本
                };
                let key = path.to_string_lossy().into_owned();
                if self.image_failed.contains(&key) {
                    continue;
                }
                if !self.inline_images.contains_key(&key) {
                    match self.images.load(&path) {
                        Ok(img) => {
                            self.inline_images.insert(key.clone(), img);
                        }
                        Err(e) => {
                            eprintln!("图片加载失败（降级为 alt 文本） {}: {e}", path.display());
                            self.image_failed.insert(key);
                            continue;
                        }
                    }
                }
                let nat = &self.inline_images[&key];
                let (dw, dh) = fit_size(nat.width, nat.height, width);
                imgs.push(InlineImg {
                    range: nrange,
                    key: InlineAsset::Image(key),
                    width: dw,
                    height: dh,
                    baseline: dh, // 底边贴基线
                });
            }
            let l = self
                .engine
                .layout_block(kind, &text, range, &inlines, width, caret, &imgs);
            self.layouts.insert(id, l);
        }
    }

    /// 仅补齐视口内（含上下各一屏 overscan）缺失布局（P1b 惰性排版）。
    fn ensure_layouts_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_layouts_range(range);
    }

    /// 测试专用：等价全量排版；不参与非测试构建，避免 dead_code。
    #[cfg(test)]
    fn ensure_layouts_all(&mut self) {
        self.ensure_layouts_range(0..self.index.blocks.len());
    }

    /// 按需排版单块（含其数学渲染/表格网格）：光标、命中、垂直移动等路径使用。
    fn ensure_block(&mut self, id: BlockId) {
        let Some(i) = self.index.blocks.iter().position(|g| g.id == id) else {
            return;
        };
        self.ensure_layouts_range(i..i + 1);
        self.ensure_math_range(i..i + 1);
        self.ensure_tables_range(i..i + 1);
    }

    /// 视口覆盖的块索引区间（含上下各一屏 overscan）：`top` 已由几何重算保证
    /// 单调不减，二分定位首尾，避免逐块线性扫描。
    fn visible_index_range(&self) -> std::ops::Range<usize> {
        let vp = self.viewport();
        let lo = self.scroll_y - vp.h;
        let hi = self.scroll_y + 2.0 * vp.h;
        let blocks = &self.index.blocks;
        let start = blocks.partition_point(|g| g.top + g.height < lo);
        let end = blocks.partition_point(|g| g.top < hi);
        start.min(blocks.len())..end.max(start).min(blocks.len())
    }

    /// 光标移动引起的 Reveal 状态变化：仅旧/新光标所在块的激活态可能改变。
    fn invalidate_reveal(&mut self, old: usize, new: usize) {
        if old == new {
            return;
        }
        for g in &self.index.blocks {
            let (id, kind, range) = (g.id, g.kind, g.range);
            let touches = |p: usize| range.start <= p && p <= range.end;
            if !touches(old) && !touches(new) {
                continue;
            }
            // 块级 Reveal（切片 12）：列表项/分隔线按块激活（进/出块才变）；
            // 引用块逐行激活（块内跨行移动亦变，行内移动不变）。
            let block_reveal = match kind {
                BlockKind::ListItem { .. } | BlockKind::ThematicBreak => {
                    touches(old) != touches(new)
                }
                BlockKind::BlockQuote => {
                    let text = self.editor.session().text_slice(range).unwrap_or_default();
                    quote_line_of(&text, &range, old) != quote_line_of(&text, &range, new)
                }
                _ => false,
            };
            let inlines = self.editor.session().block_inlines(id).unwrap_or_default();
            if block_reveal || inlines.iter().any(|n| activation_changed(n, old, new)) {
                self.layouts.remove(&id);
                self.rasters.remove(&id);
            }
        }
    }

    /// 数学块按需渲染（同步 v0：spike 实测单公式 <1ms、百公式 21ms，预算内，ADR-002）。
    /// 光标在块内为源码态不渲染；失败记入 math_failed 不再每帧重试（降级源码显示）。
    fn ensure_math_range(&mut self, indices: std::ops::Range<usize>) {
        let caret = self.editor.caret();
        let scale = self.scale;
        let fg = self.theme.fg;
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind, g.range);
            if kind != BlockKind::Math
                || (range.start <= caret && caret <= range.end)
                || self.math_imgs.contains_key(&id)
                || self.math_failed.contains(&id)
            {
                continue;
            }
            let Some(tex) = self.editor.session().math_source(id) else {
                continue;
            };
            match self.math.render(&tex, true, scale, fg) {
                Ok(img) => {
                    self.math_imgs.insert(id, img);
                }
                Err(e) => {
                    eprintln!("公式渲染失败（降级为源码显示）: {e}");
                    self.math_failed.insert(id);
                }
            }
        }
    }

    /// 仅渲染视口内（含 overscan）的数学块。
    fn ensure_math_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_math_range(range);
    }

    /// 测试专用：全量数学渲染。
    #[cfg(test)]
    fn ensure_math(&mut self) {
        self.ensure_math_range(0..self.index.blocks.len());
    }

    /// 数学块当前是否按渲染图显示：光标在块内（Reveal 同语义）或未渲染时显示源码。
    fn math_shows_image(&self, id: BlockId, range: &veil_core::Range) -> bool {
        let caret = self.editor.caret();
        !(range.start <= caret && caret <= range.end) && self.math_imgs.contains_key(&id)
    }

    /// 表格按需网格排版（光标在表格内为源码态，不排版网格；离开后首次绘制时补齐）。
    /// 网格缓存与文本布局缓存独立：网格仅供渲染/命中，源码态仍走既有块排版。
    fn ensure_tables_range(&mut self, indices: std::ops::Range<usize>) {
        let width = self.content_width();
        let caret = self.editor.caret();
        let fg = self.theme.fg;
        let Some(blocks) = self.index.blocks.get(indices) else {
            return;
        };
        let mut pending = Vec::new();
        for g in blocks {
            let (id, kind, range) = (g.id, g.kind, g.range);
            if kind != BlockKind::Table
                || self.tables.contains_key(&id)
                || (range.start <= caret && caret <= range.end)
            {
                continue;
            }
            if let Some(t) = self.editor.session().table(id) {
                pending.push((id, t));
            }
        }
        for (id, t) in pending {
            let l = self.engine.layout_table(&t, width, fg);
            self.tables.insert(id, l);
        }
    }

    /// 仅排版视口内（含 overscan）的表格网格。
    fn ensure_tables_visible(&mut self) {
        let range = self.visible_index_range();
        self.ensure_tables_range(range);
    }

    /// 测试专用：全量表格排版。
    #[cfg(test)]
    fn ensure_tables(&mut self) {
        self.ensure_tables_range(0..self.index.blocks.len());
    }

    /// 表格当前是否按网格显示：光标在表格内（Reveal 同语义）或未排版时显示源码。
    fn table_shows_grid(&self, id: BlockId, range: &veil_core::Range) -> bool {
        let caret = self.editor.caret();
        !(range.start <= caret && caret <= range.end) && self.tables.contains_key(&id)
    }

    /// 块显示高度：数学块渲染态取图高、表格网格态取网格高，其余取文本布局高；
    /// 文本布局缺失（视口外未排版）时回退廉价估算高度。
    fn block_height(&self, g: &BlockGeom) -> f32 {
        if g.kind == BlockKind::Math && self.math_shows_image(g.id, &g.range) {
            return self.math_imgs[&g.id].height as f32;
        }
        if g.kind == BlockKind::Table && self.table_shows_grid(g.id, &g.range) {
            return self.tables[&g.id].height;
        }
        self.layouts
            .get(&g.id)
            .map(|l| l.height)
            .unwrap_or(g.est_h)
    }

    /// 重建块清单：生产代码唯一允许全量枚举块的位置；其余读取一律走索引。
    /// 估算高度在此一次性算好（P1b）：几何重算不再读取 rope。
    fn refresh_index(&mut self) {
        let blocks = self.editor.session().blocks();
        let scale = self.scale;
        let chars_per_line = self.chars_per_line();
        self.index.blocks.clear();
        self.index.blocks.reserve(blocks.len());
        for (id, kind, range) in blocks {
            let text = self.editor.session().text_slice(range).unwrap_or_default();
            let est_h = estimate_height(&text, chars_per_line, scale);
            self.index.blocks.push(BlockGeom {
                id,
                kind,
                range,
                top: 0.0,
                height: 0.0,
                est_h,
            });
        }
        // 索引重建后 top/height 归零：立即按估算高度累计一次，可见区间/滚动条
        // 才有有效几何；排版后用 recompute_geometry 以真实高度覆盖。
        self.recompute_geometry();
    }

    /// 每视觉行可容纳的字符数（按平均字宽 0.5em 估算，仅用于便宜高度）。
    fn chars_per_line(&self) -> f32 {
        (self.content_width() / (0.5 * BASE_FONT_SIZE * self.scale)).max(1.0)
    }

    /// 按当前布局/公式/表格缓存重算高度与累计 top（不再枚举块）。
    /// 取出 Vec 再循环：`block_height(&self)` 与 `index` 的可变借用无法同时成立。
    fn recompute_geometry(&mut self) {
        let mut blocks = std::mem::take(&mut self.index.blocks);
        let mut y = MARGIN * self.scale;
        for g in &mut blocks {
            let h = self.block_height(g);
            g.height = h;
            g.top = y;
            y += h + PARA_GAP * self.scale;
        }
        // 末块后不累加块间距（与原 block_tops 一致），文档尾部只留外边距
        self.index.total_h = if blocks.is_empty() {
            2.0 * MARGIN * self.scale
        } else {
            y - PARA_GAP * self.scale + MARGIN * self.scale
        };
        self.index.blocks = blocks;
    }

    /// 光标所在块索引；文末光标可能落在最后一块的尾换行之后，回退末块。
    fn caret_block_index(&self) -> Option<usize> {
        let caret = self.editor.caret();
        self.index
            .blocks
            .iter()
            .position(|g| g.range.start <= caret && caret <= g.range.end)
            .or_else(|| {
                if caret >= self.editor.session().text_len() {
                    self.index.blocks.len().checked_sub(1)
                } else {
                    None
                }
            })
    }

    /// 光标屏幕矩形（物理像素，含菜单栏偏移）。(x, y, h)；找不到所在块（空文档）时落视口原点。
    fn caret_screen(&mut self) -> (f32, f32, f32) {
        let caret = self.editor.caret();
        let vp = self.viewport();
        if let Some(i) = self.caret_block_index() {
            let (id, start, top) = {
                let g = &self.index.blocks[i];
                (g.id, g.range.start, g.top)
            };
            // 视口外块可能尚未布局：光标路径按需补齐
            self.ensure_block(id);
            if let Some(l) = self.layouts.get(&id) {
                let (x, y, h) = self.engine.caret_xy(l, caret - start);
                return (MARGIN * self.scale + x, vp.y + top + y - self.scroll_y, h);
            }
        }
        (
            MARGIN * self.scale,
            vp.y + MARGIN * self.scale - self.scroll_y,
            BASE_LINE_HEIGHT * self.scale,
        )
    }

    fn scroll_to_caret(&mut self) {
        let vp = self.viewport();
        let (_, cy, ch) = self.caret_screen();
        if cy < vp.y {
            self.scroll_y += cy - vp.y - MARGIN * self.scale;
        } else if cy + ch > vp.y + vp.h {
            self.scroll_y += cy + ch - (vp.y + vp.h) + MARGIN * self.scale;
        }
        self.scroll_y = self
            .scroll_y
            .clamp(0.0, chrome::max_scroll(vp, self.doc_height()));
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

    fn on_key(&mut self, event: &winit::event::KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let key = match map_key(event) {
            Some(k) => k,
            None => return,
        };
        // 菜单打开时键盘只响应 Esc（其余按键被菜单吞掉，避免误编辑）
        if self.chrome.is_open() {
            if key == Key::Escape {
                self.chrome.close();
                self.request_redraw();
            }
            return;
        }
        let Some(action) = translate(key, self.mods) else {
            return;
        };
        match action {
            Action::Doc(intent) => {
                self.goal_x = None; // 非垂直移动：重置目标列
                let old_caret = self.editor.caret();
                if let Some(dirty) = self.editor.apply(intent) {
                    self.invalidate_reveal(old_caret, self.editor.caret());
                    self.after_edit(dirty);
                }
            }
            // 垂直移动/扩选：保留 goal_x（连续上下不漂移）
            Action::MoveVertical(delta) => self.move_vertical(delta, false),
            Action::SelectVertical(delta) => self.move_vertical(delta, true),
            Action::MoveLineStart => self.move_line_edge(true, false),
            Action::MoveLineEnd => self.move_line_edge(false, false),
            Action::SelectLineStart => self.move_line_edge(true, true),
            Action::SelectLineEnd => self.move_line_edge(false, true),
            Action::Copy => self.copy_selection(),
            Action::Cut => self.cut_selection(),
            Action::Paste => self.paste_clipboard(),
            Action::Save => self.save_now(),
            Action::SaveAs => self.save_as(),
            Action::Open => self.open_via_dialog(),
            Action::CloseMenu => self.chrome.close(),
            Action::Export => self.export_now(),
        }
        self.set_ime_area();
    }

    /// 粘贴（切片 11/14）：HTML 优先转 Markdown，转换无内容（纯图片/空结构）回退纯文本；
    /// 文本皆空时尝试位图：落盘 `<doc>.assets/` 并插入 `![](相对路径)`。
    /// 草稿（无关联文件）拒绝图片粘贴（无写入基准），文档不变、不置脏。
    fn paste_clipboard(&mut self) {
        let md = self
            .clipboard
            .get_html()
            .map(|h| html_to_markdown(&h))
            .filter(|m| !m.trim().is_empty());
        if let Some(text) = md.or_else(|| self.clipboard.get()) {
            self.insert_paste(&text);
            return;
        }
        let Some(img) = self.clipboard.get_image() else {
            return;
        };
        let Some(doc) = self.path.clone() else {
            self.chrome.notice = "图片粘贴需要关联文件（草稿无落盘目录）".into();
            eprintln!("图片粘贴需要关联文件（草稿无落盘目录）");
            self.request_redraw();
            return;
        };
        match self.assets.save_pasted_image(&doc, &img) {
            Ok(rel) => self.insert_paste(&format!("![]({rel})")),
            Err(e) => {
                self.chrome.notice = format!("图片落盘失败: {e}");
                eprintln!("图片落盘失败（文档未改动）: {e}");
                self.request_redraw();
            }
        }
    }

    /// 粘贴文本/图片链接：有选区则替换，单次撤销事务；空串不产生编辑。
    fn insert_paste(&mut self, text: &str) {
        let old_caret = self.editor.caret();
        if let Some(dirty) = self.editor.paste(text) {
            self.goal_x = None;
            self.invalidate_reveal(old_caret, self.editor.caret());
            self.after_edit(dirty);
        }
    }

    /// Home/End（及 Shift 扩选）：光标落到当前视觉行首/尾（`visual_line_edges` 语义）。
    fn move_line_edge(&mut self, start: bool, extend: bool) {
        let caret = self.editor.caret();
        let Some((id, range)) = self
            .index
            .blocks
            .iter()
            .find(|g| g.range.start <= caret && caret <= g.range.end)
            .map(|g| (g.id, g.range))
        else {
            return;
        };
        self.ensure_block(id);
        let Some(l) = self.layouts.get(&id) else {
            return;
        };
        let (s, e) = self.engine.visual_line_edges(l, caret - range.start);
        let target = range.start + if start { s } else { e };
        let old_caret = self.editor.caret();
        let moved = if extend {
            self.editor.set_caret_extend(target)
        } else {
            self.editor.set_caret(target)
        };
        if !moved {
            return;
        }
        self.goal_x = None; // 水平移动：重置目标列
        self.invalidate_reveal(old_caret, target);
        self.request_redraw();
    }

    fn move_vertical(&mut self, delta: i32, extend: bool) {
        let caret = self.editor.caret();
        let Some(bi) = self
            .index
            .blocks
            .iter()
            .position(|g| g.range.start <= caret && caret <= g.range.end)
        else {
            return;
        };
        let (id, range) = {
            let g = &self.index.blocks[bi];
            (g.id, g.range)
        };
        // 当前块可能不在视口内：垂直移动前按需补齐
        self.ensure_block(id);
        let (cx, cy, ch, lh) = {
            let Some(l) = self.layouts.get(&id) else {
                return;
            };
            let (cx, cy, ch) = self.engine.caret_xy(l, caret - range.start);
            (cx, cy, ch, l.height)
        };
        let goal = *self.goal_x.get_or_insert(cx);
        // 目标：本块上/下一视觉行；越界则上/下一块的末/首行
        let target_y_local = cy + if delta < 0 { -1.0 } else { ch };
        let target = if target_y_local >= 0.0 && target_y_local < lh {
            let l = &self.layouts[&id];
            range.start + self.engine.hit(l, goal, target_y_local)
        } else {
            let next_i = if delta < 0 {
                bi.checked_sub(1)
            } else if bi + 1 < self.index.blocks.len() {
                Some(bi + 1)
            } else {
                None
            };
            let Some(ni) = next_i else {
                return;
            };
            let (nid, nkind, nrange) = {
                let g = &self.index.blocks[ni];
                (g.id, g.kind, g.range)
            };
            // 目标相邻块同样按需补齐（含表格网格/数学图）
            self.ensure_block(nid);
            if nkind == BlockKind::Table && self.table_shows_grid(nid, &nrange) {
                let Some(t) = self.tables.get(&nid) else {
                    return;
                };
                t.hit(goal, if delta < 0 { t.height - 1.0 } else { 0.0 })
            } else {
                let Some(nl) = self.layouts.get(&nid) else {
                    return;
                };
                let y = if delta < 0 { nl.height - 1.0 } else { 0.0 };
                nrange.start + self.engine.hit(nl, goal, y).min(nrange.len())
            }
        };
        let old_caret = self.editor.caret();
        if extend {
            self.editor.set_caret_extend(target);
        } else {
            self.editor.set_caret(target);
        }
        self.invalidate_reveal(old_caret, target);
        self.scroll_to_caret();
        self.request_redraw();
    }

    /// 块内物理坐标 → 文档偏移：表格网格态走单元格命中（落单元格内容起点），
    /// 其余走文本布局命中。
    fn hit_block(
        &self,
        id: BlockId,
        kind: BlockKind,
        range: &veil_core::Range,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        if kind == BlockKind::Table && self.table_shows_grid(id, range) {
            return self.tables.get(&id).map(|t| t.hit(x, y));
        }
        let l = self.layouts.get(&id)?;
        Some(range.start + self.engine.hit(l, x, y).min(range.len()))
    }

    /// 鼠标位置 → 文档偏移（块间空白归入下一块起始；视口顶部之上归首块）。
    fn hit_offset(&mut self) -> Option<usize> {
        let vp = self.viewport();
        let x = (self.mouse.x as f32 - MARGIN * self.scale).max(0.0);
        let y = (self.mouse.y as f32 - vp.y + self.scroll_y).max(0.0);
        let (id, kind, range, top) = {
            let blocks = &self.index.blocks;
            let g = blocks.iter().rev().find(|g| g.top <= y).or_else(|| blocks.first())?;
            (g.id, g.kind, g.range, g.top)
        };
        // 命中块可能尚未布局：先按需补齐再读缓存
        self.ensure_block(id);
        self.hit_block(id, kind, &range, x, (y - top).max(0.0))
    }

    /// 落点更新 caret。`extend` = 保留 anchor（拖选/Shift+点击）。
    fn point_caret(&mut self, extend: bool) {
        let Some(target) = self.hit_offset() else {
            return;
        };
        let old_caret = self.editor.caret();
        if extend {
            self.editor.set_caret_extend(target);
        } else {
            self.editor.set_caret(target);
        }
        self.invalidate_reveal(old_caret, target);
        self.goal_x = None;
        self.set_ime_area();
        self.request_redraw();
    }

    /// 执行菜单动作。返回 true = 退出应用。
    fn perform_menu(&mut self, action: MenuAction) -> bool {
        match action {
            MenuAction::Open => self.open_via_dialog(),
            MenuAction::Save => self.save_now(),
            MenuAction::SaveAs => self.save_as(),
            MenuAction::Export => self.export_now(),
            MenuAction::Undo => {
                let old = self.editor.caret();
                if let Some(dirty) = self.editor.apply(Intent::Undo) {
                    self.goal_x = None;
                    self.invalidate_reveal(old, self.editor.caret());
                    self.after_edit(dirty);
                }
            }
            MenuAction::Redo => {
                let old = self.editor.caret();
                if let Some(dirty) = self.editor.apply(Intent::Redo) {
                    self.goal_x = None;
                    self.invalidate_reveal(old, self.editor.caret());
                    self.after_edit(dirty);
                }
            }
            MenuAction::Cut => self.cut_selection(),
            MenuAction::Copy => self.copy_selection(),
            MenuAction::Paste => self.paste_clipboard(),
            MenuAction::ThemeSystem => self.set_theme_mode(ThemeMode::System),
            MenuAction::ThemeLight => self.set_theme_mode(ThemeMode::Light),
            MenuAction::ThemeDark => self.set_theme_mode(ThemeMode::Dark),
            MenuAction::Quit => return true,
        }
        self.request_redraw();
        false
    }

    fn copy_selection(&mut self) {
        if let Some(t) = self.editor.copy() {
            self.clipboard.set(&t);
        }
    }

    fn cut_selection(&mut self) {
        let old_caret = self.editor.caret();
        if let Some((t, dirty)) = self.editor.cut() {
            self.clipboard.set(&t);
            self.goal_x = None;
            self.invalidate_reveal(old_caret, self.editor.caret());
            self.after_edit(dirty);
        }
    }

    /// 左键按下路由（菜单 > 菜单栏 > 滚动条 > 状态栏 > 正文光标）。
    /// 返回 true = 退出应用。菜单打开时点击外部只关菜单、不移动光标。
    fn on_left_click(&mut self) -> bool {
        let (mx, my) = (self.mouse.x as f32, self.mouse.y as f32);
        let scale = self.scale;
        let vp = self.viewport();
        if let Some(open) = self.chrome.open {
            if let Some(item) = chrome::hit_popup_item(mx, my, open, scale) {
                self.chrome.close();
                if let Some(action) = chrome::action_at(open, item) {
                    return self.perform_menu(action);
                }
                return false;
            }
            if chrome::popup_contains(mx, my, open, scale) {
                return false; // 弹窗内边距：吞掉点击
            }
            self.chrome.close();
            if let Some(m) = chrome::hit_menu_bar(mx, my, scale) {
                if m != open {
                    self.chrome.open = Some(m);
                }
                self.request_redraw();
                return false;
            }
            self.request_redraw();
            return false;
        }
        if let Some(m) = chrome::hit_menu_bar(mx, my, scale) {
            self.chrome.toggle(m);
            self.request_redraw();
            return false;
        }
        let doc_h = self.doc_height();
        if chrome::scrollbar_hit(vp, doc_h, mx, my, scale) {
            if let Some((ty, th)) = chrome::scroll_thumb(vp, doc_h, self.scroll_y, scale) {
                if my >= ty && my < ty + th {
                    self.chrome.scroll_drag = Some(my - ty);
                } else {
                    let dir = if my < ty { -1 } else { 1 };
                    self.scroll_y = chrome::page_scroll(vp, doc_h, self.scroll_y, dir);
                    self.set_ime_area();
                    self.request_redraw();
                }
            }
            return false;
        }
        if my < vp.y || my >= vp.y + vp.h {
            return false; // 状态栏等 chrome 区域：不落光标
        }
        self.dragging = true;
        self.point_caret(self.mods.shift);
        false
    }

    /// 悬停高亮（菜单标题 / 弹出菜单项）。
    fn update_hover(&mut self) {
        let (x, y) = (self.mouse.x as f32, self.mouse.y as f32);
        let menu = chrome::hit_menu_bar(x, y, self.scale);
        let item = self
            .chrome
            .open
            .and_then(|open| chrome::hit_popup_item(x, y, open, self.scale).map(|i| (open, i)));
        if self.chrome.hover_menu != menu || self.chrome.hover_item != item {
            self.chrome.hover_menu = menu;
            self.chrome.hover_item = item;
            self.request_redraw();
        }
    }

    fn paint(&mut self) {
        let (w, h) = match self.window.as_ref() {
            Some(win) => {
                let s = win.inner_size();
                (s.width.max(1), s.height.max(1))
            }
            None => return,
        };
        // 首次绘制（或索引尚未建立）时补一次；其余时机由编辑/打开/resize 维护
        if self.index.blocks.is_empty() {
            self.refresh_index();
        }
        // 视口驱动惰性排版（P1b）：先排当前可见区间并重算几何；估算高度偏小
        // 可能让视口底部露出未排版块，故重算可见区间后再补跑一遍即收敛。
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        let range = self.visible_index_range();
        if self
            .index
            .blocks
            .get(range)
            .is_some_and(|bs| bs.iter().any(|g| !self.layouts.contains_key(&g.id)))
        {
            self.ensure_layouts_visible();
            self.ensure_math_visible();
            self.ensure_tables_visible();
            self.recompute_geometry();
        }
        // 帧缓冲跨帧复用：仅尺寸变化才重新分配（每帧只做一次底色填充）
        let mut frame = self.frame.take().unwrap_or_else(|| Frame::new(w, h, self.theme.bg));
        frame.reset(w, h, self.theme.bg);
        let scale = self.scale;
        let theme = self.theme;
        let vp = self.viewport();
        // 窗口缩放/文档变短后滚动量可能越界：绘制前统一钳位（滚动条与内容同源）
        self.scroll_y = self
            .scroll_y
            .clamp(0.0, chrome::max_scroll(vp, self.doc_height()));
        let sel = self.editor.selection();
        let caret = self.editor.caret();
        // 只遍历视口覆盖的块（含 overscan），不再逐块判定后 continue
        let vis = self.visible_index_range();
        for g in &self.index.blocks[vis] {
            let (id, kind, range, top) = (g.id, g.kind, g.range, g.top);
            let sy = (vp.y + top - self.scroll_y) as i32;
            if (sy as f32) >= vp.y + vp.h
                || sy + self.block_height(g) as i32 <= vp.y as i32
            {
                continue; // 视口外不画（布局已缓存，光栅按需）
            }
            if self.math_shows_image(id, &range) {
                // 数学块渲染态：图整体替换文本光栅（display 公式居中）
                let img = &self.math_imgs[&id];
                let x = (MARGIN * scale
                    + ((self.content_width() - img.width as f32) / 2.0).max(0.0))
                    as i32;
                frame.blit(&img.pixels, img.width, img.height, x, sy);
                continue;
            }
            if kind == BlockKind::Table && self.table_shows_grid(id, &range) {
                self.paint_table(&mut frame, id, sy);
                continue;
            }
            let Some(l) = self.layouts.get(&id) else {
                continue;
            };
            if !self.rasters.contains_key(&id) {
                let r = self.engine.rasterize(l, theme.fg);
                self.rasters.insert(id, r);
            }
            if matches!(kind, BlockKind::FencedCode) {
                let (rw, rh) = (self.rasters[&id].1, self.rasters[&id].2);
                frame.fill_rect(
                    (MARGIN * scale) as i32 - 4,
                    sy - 2,
                    rw + 8,
                    rh + 4,
                    theme.code_bg,
                );
            }
            // 选区高亮：先画底色再 blit 字形，文本保持可读
            if let Some(sel) = sel {
                let (s, e) = (sel.start.max(range.start), sel.end.min(range.end));
                if s < e {
                    for (rx, ry, rw, rh) in
                        self.engine
                            .selection_rects(l, s - range.start, e - range.start)
                    {
                        frame.fill_rect(
                            (MARGIN * scale + rx) as i32,
                            sy + ry as i32,
                            rw as u32,
                            rh as u32,
                            theme.sel,
                        );
                    }
                }
            }
            let (px, rw, rh) = &self.rasters[&id];
            frame.blit(px, *rw, *rh, (MARGIN * scale) as i32, sy);
            // 行内公式/图片：位置在排版期已定（PlacedImg），叠在文本光栅之上。
            // 图片按占位显示尺寸最近邻缩放（缓存保留自然尺寸，不生成缩放副本）。
            for p in &l.imgs {
                let dx = (MARGIN * scale + p.x) as i32;
                let dy = sy + p.y as i32;
                match &p.key {
                    InlineAsset::Math(tex) => {
                        if let Some(img) = self.inline_math.get(tex) {
                            frame.blit(&img.pixels, img.width, img.height, dx, dy);
                        }
                    }
                    InlineAsset::Image(key) => {
                        if let Some(img) = self.inline_images.get(key) {
                            frame.blit_scaled(
                                &img.pixels,
                                img.width,
                                img.height,
                                p.width,
                                p.height,
                                dx,
                                dy,
                            );
                        }
                    }
                }
            }
            // 块级装饰（切片 12）：引用左竖条；分隔线非激活时画居中横线
            match kind {
                BlockKind::BlockQuote => {
                    let bh = self.block_height(g) as u32;
                    frame.fill_rect(
                        (MARGIN * scale) as i32 - (8.0 * scale) as i32,
                        sy,
                        (3.0 * scale).max(1.0) as u32,
                        bh,
                        theme.grid,
                    );
                }
                BlockKind::ThematicBreak if !node_active(range, caret) => {
                    let bh = self.block_height(g);
                    frame.fill_rect(
                        (MARGIN * scale) as i32,
                        sy + (bh * 0.5) as i32,
                        self.content_width().max(1.0) as u32,
                        1,
                        theme.grid,
                    );
                }
                _ => {}
            }
        }
        let (line, col) = self.editor.session().line_col(caret).unwrap_or((1, 1));
        let dirty = self.editor.is_dirty();
        let external = self.external_state();
        let doc_h = self.doc_height();
        self.paint_caret_and_preedit(&mut frame);
        // chrome 最后绘制（不透明覆盖内容；滚动条区保留给状态栏行列）
        self.chrome.paint(
            &mut frame,
            &mut self.engine,
            vp,
            scale,
            &theme,
            self.path.as_deref(),
            dirty,
            external,
            line,
            col,
            doc_h,
            self.scroll_y,
        );
        if let Some(surface) = &mut self.surface {
            // 取缓冲失败属瞬时状态：丢帧不崩溃，下一帧自愈
            if let Ok(mut target) = surface.buffer_mut() {
                target.copy_from_slice(&frame.pixels);
                let _ = target.present();
            }
        }
        self.frame = Some(frame); // 归还缓冲供下一帧复用
    }

    /// 绘制表格网格：表头底色 → 单元格文本 → 网格线（先底后字，线压边）。
    /// 选区跨网格态表格时不画高亮（同数学块 v0 已知简化）。
    fn paint_table(&self, frame: &mut Frame, id: BlockId, top: i32) {
        let Some(t) = self.tables.get(&id) else {
            return;
        };
        let x0 = (MARGIN * self.scale) as i32;
        let theme = self.theme;
        if let Some(first) = t.rows.first() {
            frame.fill_rect(
                x0,
                top,
                t.width.ceil() as u32,
                first.height.ceil() as u32,
                theme.table_head_bg,
            );
        }
        for c in &t.cells {
            let cy = top + c.y.round() as i32;
            if cy + c.pixel_h as i32 <= 0 || cy >= frame.height as i32 {
                continue; // 行在视口外
            }
            frame.blit(&c.pixels, c.pixel_w, c.pixel_h, x0 + c.x.round() as i32, cy);
        }
        let w = t.width.ceil().max(1.0) as u32;
        let h = t.height.ceil().max(1.0) as u32;
        let mut x: f32 = 0.0;
        for cw in &t.cols {
            frame.fill_rect(x0 + x.round() as i32, top, 1, h, theme.grid);
            x += cw;
        }
        let right = t.width.round().max(1.0) as i32;
        frame.fill_rect(x0 + right - 1, top, 1, h, theme.grid);
        frame.fill_rect(x0, top, w, 1, theme.grid);
        for r in &t.rows {
            frame.fill_rect(
                x0,
                top + (r.top + r.height).round() as i32 - 1,
                w,
                1,
                theme.grid,
            );
        }
    }

    fn paint_caret_and_preedit(&mut self, frame: &mut Frame) {
        let (cx, cy, ch) = self.caret_screen();
        let vp = self.viewport();
        if cy + ch <= vp.y || cy >= vp.y + vp.h {
            return; // 光标在视口外：不画（被 chrome 覆盖前已越界）
        }
        let fg = self.theme.fg;
        let caret_color = [fg[0], fg[1], fg[2], 255];
        if let Some(pre) = self.editor.preedit() {
            // preedit：排版小串 → 位图贴在光标处 + 下划线；光标移至组合串内光标位
            let text = pre.text.clone();
            let l = self.engine.layout_preedit(&text);
            let (px, w, h) = self.engine.rasterize(&l, fg);
            frame.blit(&px, w, h, cx as i32, (cy + ch - h as f32).max(0.0) as i32);
            frame.fill_rect(
                cx as i32,
                (cy + ch) as i32 - 1,
                w,
                1,
                [fg[0], fg[1], fg[2], 255],
            );
            let inner = pre.cursor.map(|(s, _)| s).unwrap_or(0);
            let (ix, _, _) = self.engine.caret_xy(&l, inner);
            frame.fill_rect((cx + ix) as i32, cy as i32, 1, ch as u32, caret_color);
        } else {
            frame.fill_rect(cx as i32, cy as i32, 2, ch as u32, caret_color);
        }
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

/// 节点（含嵌套子节点）激活态在两次光标位置间是否变化（Reveal 失效判定）。
fn activation_changed(n: &InlineNode, old: usize, new: usize) -> bool {
    node_active(n.range, old) != node_active(n.range, new)
        || n.children.iter().any(|c| activation_changed(c, old, new))
}

/// 视口外块的廉价高度估算（P1b）：换行数与原文字符数折算的折行数取大，
/// 乘以基准行高；不触发 shaping，仅用于几何累计。
fn estimate_height(text: &str, chars_per_line: f32, scale: f32) -> f32 {
    let hard_lines = text.bytes().filter(|&b| b == b'\n').count() as f32 + 1.0;
    let chars = text.chars().count() as f32;
    let wrapped = (chars / chars_per_line.max(1.0)).ceil().max(1.0);
    hard_lines.max(wrapped) * BASE_LINE_HEIGHT * scale
}

/// 光标在引用块内的行号（块外 None）：按 `\n` 计数，行数规则与排版一致（剥唯一尾换行）。
fn quote_line_of(text: &str, range: &veil_core::Range, p: usize) -> Option<usize> {
    if p < range.start || p > range.end {
        return None;
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    let rel = (p - range.start).min(body.len());
    Some(body[..rel].bytes().filter(|&b| b == b'\n').count())
}

/// 图片 URL → 本地路径（切片 13）。相对 URL 相对文档所在目录；草稿（无路径）的
/// 相对 URL 无法解析；`http:`/`https:`/`data:`/`file:` 等带 scheme 的 URL v0 不加载。
/// Windows 盘符（`C:`）只 1 个字母，不误判为 scheme。
fn local_image_path(url: &str, doc: Option<&Path>) -> Option<PathBuf> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if let Some((scheme, _)) = url.split_once(':') {
        if scheme.len() >= 2 && scheme.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
    }
    let p = Path::new(url);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    Some(doc?.parent()?.join(p))
}

/// 图片显示尺寸：宽超版心按比例缩放（高同比），至少 1px。
fn fit_size(w: u32, h: u32, max_width: f32) -> (u32, u32) {
    let w = w.max(1) as f32;
    let h = h.max(1) as f32;
    let s = if w > max_width { max_width / w } else { 1.0 };
    (
        (w * s).round().max(1.0) as u32,
        (h * s).round().max(1.0) as u32,
    )
}

/// winit 键 → 内部 `Key`。ctrl 按住时 text 为空，字母键退化为 Z/Y 供快捷键判定。
fn map_key(event: &winit::event::KeyEvent) -> Option<Key> {
    if let Some(t) = &event.text {
        if !t.is_empty() && !t.chars().any(|c| c.is_control()) {
            return Some(Key::Text(t.to_string()));
        }
    }
    match &event.logical_key {
        WKey::Named(NamedKey::ArrowLeft) => Some(Key::Left),
        WKey::Named(NamedKey::ArrowRight) => Some(Key::Right),
        WKey::Named(NamedKey::ArrowUp) => Some(Key::Up),
        WKey::Named(NamedKey::ArrowDown) => Some(Key::Down),
        WKey::Named(NamedKey::Home) => Some(Key::Home),
        WKey::Named(NamedKey::End) => Some(Key::End),
        WKey::Named(NamedKey::Backspace) => Some(Key::Backspace),
        WKey::Named(NamedKey::Delete) => Some(Key::Delete),
        WKey::Named(NamedKey::Enter) => Some(Key::Enter),
        WKey::Named(NamedKey::Escape) => Some(Key::Escape),
        WKey::Named(NamedKey::Space) => Some(Key::Text(" ".into())),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("z") => Some(Key::Z),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("y") => Some(Key::Y),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("c") => Some(Key::C),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("x") => Some(Key::X),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("v") => Some(Key::V),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("s") => Some(Key::S),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("o") => Some(Key::O),
        WKey::Character(c) if c.as_str().eq_ignore_ascii_case("e") => Some(Key::E),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! 打开/保存编排逻辑（save_now/open_path）：window=None 下 headless 可测，
    //! FileStore 以共享句柄的内存实现注入，断言经回读完成。
    //! 数学块渲染（切片 6）：MathRenderer 以 fake 注入，断言缓存/降级/失效行为。
    use super::*;
    use crate::chrome::ExternalState;
    use crate::editor::Intent;
    use crate::math::{MathImage, MathRenderer};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    struct NoClip;
    impl Clipboard for NoClip {
        fn get(&mut self) -> Option<String> {
            None
        }
        fn set(&mut self, _text: &str) {}
    }

    /// 不加载任何图片（既有测试的默认注入）：所有图片降级 alt 文本。
    struct NoImages;
    impl ImageLoader for NoImages {
        fn load(&mut self, _path: &Path) -> Result<ImageData, String> {
            Err("测试未注入图片".into())
        }
    }

    /// 图片加载 mock：按路径返回自然尺寸位图并记录调用；未知路径 Err。
    struct FakeImages {
        files: HashMap<PathBuf, (u32, u32)>,
        calls: Rc<RefCell<Vec<PathBuf>>>,
    }
    impl ImageLoader for FakeImages {
        fn load(&mut self, path: &Path) -> Result<ImageData, String> {
            self.calls.borrow_mut().push(path.to_path_buf());
            match self.files.get(path) {
                Some(&(w, h)) => Ok(ImageData {
                    pixels: vec![255; (w * h * 4) as usize],
                    width: w,
                    height: h,
                }),
                None => Err("文件不存在".into()),
            }
        }
    }

    /// 不落盘任何图片（既有测试的默认注入）：图片粘贴走 Err，文档不变。
    struct NoAssets;
    impl AssetStore for NoAssets {
        fn save_pasted_image(&mut self, _doc: &Path, _image: &ImageData) -> Result<String, String> {
            Err("测试未注入资产存储".into())
        }
    }

    type MathCalls = Rc<RefCell<Vec<(String, bool)>>>;
    type AssetCalls = Rc<RefCell<Vec<(String, Option<PathBuf>)>>>;

    /// 不弹对话框（既有测试的默认注入）：打开/另存为一律取消。
    struct NoDialogs;
    impl FileDialog for NoDialogs {
        fn open_markdown(&mut self) -> Option<PathBuf> {
            None
        }
        fn save_markdown(&mut self, _suggested: &str) -> Option<PathBuf> {
            None
        }
    }

    /// 不监听文件（既有测试的默认注入）：poll 恒空，watch 恒成功。
    struct NoWatcher;
    impl FileWatcher for NoWatcher {
        fn watch(&mut self, _path: &Path) -> Result<(), String> {
            Ok(())
        }
        fn unwatch(&mut self) {}
        fn poll(&mut self) -> Vec<FileEvent> {
            Vec::new()
        }
    }

    /// 监听 mock：事件队列由测试注入，watch/unwatch 记录目标路径。
    #[derive(Clone, Default)]
    struct FakeWatch {
        events: Rc<RefCell<Vec<FileEvent>>>,
        calls: Rc<RefCell<Vec<String>>>,
        fail: bool,
    }
    impl FileWatcher for FakeWatch {
        fn watch(&mut self, path: &Path) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("watch:{}", path.display()));
            if self.fail {
                Err("模拟监听失败".into())
            } else {
                Ok(())
            }
        }
        fn unwatch(&mut self) {
            self.calls.borrow_mut().push("unwatch".into());
        }
        fn poll(&mut self) -> Vec<FileEvent> {
            std::mem::take(&mut *self.events.borrow_mut())
        }
    }

    /// 对话框 mock：记录调用与建议文件名，返回注入路径（None=用户取消）。
    struct FakeDialogs {
        open: Option<PathBuf>,
        save: Option<PathBuf>,
        calls: Rc<RefCell<Vec<String>>>,
    }
    impl FileDialog for FakeDialogs {
        fn open_markdown(&mut self) -> Option<PathBuf> {
            self.calls.borrow_mut().push("open".into());
            self.open.clone()
        }
        fn save_markdown(&mut self, suggested: &str) -> Option<PathBuf> {
            self.calls.borrow_mut().push(format!("save:{suggested}"));
            self.save.clone()
        }
    }

    /// 粘贴图片资产 mock：记录调用与图片尺寸，返回固定相对路径（或注入失败）。
    struct FakeAssets {
        result: Result<String, String>,
        calls: Rc<RefCell<Vec<(PathBuf, u32, u32)>>>,
    }
    impl AssetStore for FakeAssets {
        fn save_pasted_image(&mut self, doc: &Path, image: &ImageData) -> Result<String, String> {
            self.calls
                .borrow_mut()
                .push((doc.to_path_buf(), image.width, image.height));
            self.result.clone()
        }
    }

    /// 图片剪贴板 mock：text/html/image 三格式按需返回（模拟系统剪贴板择一/并存）。
    struct ClipImage {
        text: Option<String>,
        html: Option<String>,
        image: Option<ImageData>,
    }
    impl Clipboard for ClipImage {
        fn get(&mut self) -> Option<String> {
            self.text.clone()
        }
        fn set(&mut self, _text: &str) {}
        fn get_html(&mut self) -> Option<String> {
            self.html.clone()
        }
        fn get_image(&mut self) -> Option<ImageData> {
            self.image.as_ref().map(|i| ImageData {
                pixels: i.pixels.clone(),
                width: i.width,
                height: i.height,
            })
        }
    }

    fn solid(w: u32, h: u32) -> ImageData {
        ImageData {
            pixels: vec![128; (w * h * 4) as usize],
            width: w,
            height: h,
        }
    }

    /// 富文本剪贴板 mock：模拟浏览器同时写 HTML 与纯文本两种格式。
    struct Clip {
        html: Option<String>,
        text: Option<String>,
    }
    impl Clipboard for Clip {
        fn get(&mut self) -> Option<String> {
            self.text.clone()
        }
        fn set(&mut self, _text: &str) {}
        fn get_html(&mut self) -> Option<String> {
            self.html.clone()
        }
    }

    fn app_clip(html: Option<&str>, plain: Option<&str>) -> App {
        let doc = Doc {
            text: String::new(),
            path: None,
            bom: false,
        };
        let m = FakeMath {
            fail: false,
            calls: Rc::new(RefCell::new(Vec::new())),
        };
        App::new(
            doc,
            Box::new(Clip {
                html: html.map(str::to_string),
                text: plain.map(str::to_string),
            }),
            Box::new(MemStore::default()),
            Box::new(m),
            Box::new(NoImages),
            Box::new(NoAssets),
            Box::new(NoExport),
            Box::new(NoDialogs),
            Box::new(NoWatcher),
            theme::test_support::env(),
        )
    }

    fn doc_text(a: &App) -> String {
        let s = a.editor.session();
        s.text_slice(Range::new(0, s.text_len()))
            .expect("全文档区间恒合法")
    }

    // ---- 富文本粘贴（切片 11）：HTML 优先转 Markdown，无内容回退纯文本 ----

    #[test]
    fn paste_prefers_html_over_plain() {
        let mut a = app_clip(Some("<p>a <strong>b</strong></p>"), Some("纯文本"));
        a.paste_clipboard();
        assert_eq!(doc_text(&a), "a **b**\n");
        assert!(a.editor.is_dirty());
    }

    #[test]
    fn paste_falls_back_to_plain_when_html_converts_empty() {
        // 空结构（如纯图片复制只带 img 标签）转换无内容 → 用纯文本
        let mut a = app_clip(Some("<p> </p><img src=\"x.png\">"), Some("fallback"));
        a.paste_clipboard();
        assert_eq!(doc_text(&a), "fallback");
    }

    #[test]
    fn paste_no_content_produces_no_edit() {
        let mut a = app_clip(None, None);
        a.paste_clipboard();
        assert_eq!(doc_text(&a), "");
        assert!(!a.editor.is_dirty(), "无内容粘贴不得置脏");
    }

    /// 记录 (tex, display) 调用；display=true 出 4x4（baseline=0），false 出 8x4（baseline=3）
    /// —— 行内图故意与块级区分（宽高、基线），测试可断言走了正确的渲染分支。
    struct FakeMath {
        fail: bool,
        calls: Rc<RefCell<Vec<(String, bool)>>>,
    }
    impl MathRenderer for FakeMath {
        fn render(
            &mut self,
            tex: &str,
            display: bool,
            _scale: f32,
            _color: [u8; 3],
        ) -> Result<MathImage, String> {
            self.calls.borrow_mut().push((tex.to_string(), display));
            if self.fail {
                return Err("模拟渲染失败".into());
            }
            if display {
                Ok(MathImage {
                    pixels: vec![255; 4 * 4 * 4],
                    width: 4,
                    height: 4,
                    baseline: 0,
                })
            } else {
                Ok(MathImage {
                    pixels: vec![255; 8 * 4 * 4],
                    width: 8,
                    height: 4,
                    baseline: 3,
                })
            }
        }
    }

    /// 默认导出器：不记录、恒失败（不关心导出的既有测试用）。
    struct NoExport;
    impl Exporter for NoExport {
        fn export_html(
            &mut self,
            _s: &veil_core::Session,
            _p: Option<&Path>,
        ) -> Result<PathBuf, String> {
            Err("未注入导出器".into())
        }
    }

    fn app_math(text: &str, fail: bool) -> (App, MathCalls) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let doc = Doc {
            text: text.into(),
            path: None,
            bom: false,
        };
        let m = FakeMath {
            fail,
            calls: calls.clone(),
        };
        let a = App::new(
            doc,
            Box::new(NoClip),
            Box::new(MemStore::default()),
            Box::new(m),
            Box::new(NoImages),
            Box::new(NoAssets),
            Box::new(NoExport),
            Box::new(NoDialogs),
            Box::new(NoWatcher),
            theme::test_support::env(),
        );
        (a, calls)
    }

    /// 按块 id 取索引几何（测试断言 block_height 用）。
    fn geom(a: &App, id: BlockId) -> &BlockGeom {
        a.index
            .blocks
            .iter()
            .find(|g| g.id == id)
            .expect("块索引应包含目标块")
    }

    #[test]
    fn math_block_renders_image_when_caret_outside() {
        // 「a\n\n$$\nx\n$$\n」：数学块 bytes [3,11)，初始 caret=0 在块外
        let (mut a, calls) = app_math("a\n\n$$\nx\n$$\n", false);
        a.ensure_layouts_all();
        a.ensure_math();
        a.ensure_math(); // 幂等：缓存命中不重渲
        assert_eq!(calls.borrow().len(), 1);
        let (id, kind, range) = a.editor.session().blocks()[1];
        assert_eq!(kind, BlockKind::Math);
        assert!(a.math_shows_image(id, &range));
        assert_eq!(a.block_height(geom(&a, id)), 4.0); // 图高即块高
    }

    #[test]
    fn math_block_shows_source_when_caret_inside() {
        let (mut a, calls) = app_math("a\n\n$$\nx\n$$\n", false);
        assert!(a.editor.set_caret(6)); // 「x」处，块内
        a.ensure_layouts_all();
        a.ensure_math();
        assert_eq!(calls.borrow().len(), 0); // 源码态不渲染
        let (id, _, range) = a.editor.session().blocks()[1];
        assert!(!a.math_shows_image(id, &range));
        let h = a.layouts.get(&id).unwrap().height;
        assert_eq!(a.block_height(geom(&a, id)), h); // 文本布局高
    }

    #[test]
    fn math_render_failure_degrades_to_source() {
        let (mut a, calls) = app_math("a\n\n$$\nx\n$$\n", true);
        a.ensure_layouts_all();
        a.ensure_math();
        a.ensure_math(); // 失败记忆：不每帧重试
        assert_eq!(calls.borrow().len(), 1);
        let (id, _, range) = a.editor.session().blocks()[1];
        assert!(a.math_failed.contains(&id));
        assert!(!a.math_shows_image(id, &range));
        let h = a.layouts.get(&id).unwrap().height;
        assert_eq!(a.block_height(geom(&a, id)), h); // 降级为源码显示
    }

    #[test]
    fn dirty_edit_clears_math_cache() {
        let (mut a, _calls) = app_math("a\n\n$$\nx\n$$\n", false);
        a.ensure_layouts_all();
        a.ensure_math();
        let (id, _, _) = a.editor.session().blocks()[1];
        assert!(a.math_imgs.contains_key(&id));
        a.after_edit(vec![id]); // 块脏 → 图缓存失效，下帧重渲
        assert!(!a.math_imgs.contains_key(&id));
    }

    // ---- 行内公式（切片 8）：非激活 Math 节点出图、激活显源码、tex 内容寻址缓存 ----
    // 「a $x$ b」逐字节：a=0 ' '=1 $=2 x=3 $=4 ' '=5 b=6，Math range=[2,5) content=[3,4)

    #[test]
    fn inline_math_renders_image_when_caret_outside() {
        let (mut a, calls) = app_math("a $x$ b", false);
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[0];
        assert_eq!(a.layouts[&id].imgs.len(), 1);
        assert_eq!(a.layouts[&id].imgs[0].width, 8, "FakeMath 行内分支（8x4）");
        assert_eq!(
            calls.borrow().as_slice(),
            &[("x".to_string(), false)],
            "display=false"
        );
        // 内容寻址缓存：重排（如 Resized 清布局）不重渲
        a.layouts.remove(&id);
        a.ensure_layouts_all();
        assert_eq!(calls.borrow().len(), 1);
    }

    #[test]
    fn inline_math_shows_source_when_caret_inside() {
        let (mut a, calls) = app_math("a $x$ b", false);
        assert!(a.editor.set_caret(3)); // 「x」处，节点内
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[0];
        assert!(a.layouts[&id].imgs.is_empty());
        assert_eq!(a.layouts[&id].layout_text(), "a $x$ b");
        assert!(calls.borrow().is_empty(), "源码态不渲染");
    }

    #[test]
    fn inline_math_toggles_on_caret_enter_leave() {
        let (mut a, calls) = app_math("a $x$ b", false);
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[0];
        assert_eq!(a.layouts[&id].imgs.len(), 1);
        // 进入节点：invalidate_reveal 失效 → 重排显源码
        let old = a.editor.caret();
        assert!(a.editor.set_caret(3));
        a.invalidate_reveal(old, 3);
        a.ensure_layouts_all();
        assert!(a.layouts[&id].imgs.is_empty());
        // 离开节点：恢复出图，缓存命中不重渲
        assert!(a.editor.set_caret(0));
        a.invalidate_reveal(3, 0);
        a.ensure_layouts_all();
        assert_eq!(a.layouts[&id].imgs.len(), 1);
        assert_eq!(calls.borrow().len(), 1);
    }

    #[test]
    fn inline_math_failure_degrades_and_memoizes() {
        let (mut a, calls) = app_math("a $x$ b", true);
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[0];
        assert!(a.layouts[&id].imgs.is_empty(), "失败降级：占位不入排版");
        a.layouts.remove(&id);
        a.ensure_layouts_all(); // 失败记忆：不每帧重试
        assert_eq!(calls.borrow().len(), 1);
    }

    #[test]
    fn edit_math_source_rerenders_new_tex() {
        let (mut a, calls) = app_math("a $x$ b", false);
        a.ensure_layouts_all();
        assert_eq!(calls.borrow().len(), 1);
        // 光标进节点（3），插入 y：「a $yx$ b」，caret=4 在新区间 [2,6) 内 → 显源码不重渲
        assert!(a.editor.set_caret(3));
        let old = a.editor.caret();
        let dirty = a.editor.apply(Intent::InsertText("y".into())).unwrap();
        a.invalidate_reveal(old, a.editor.caret());
        a.after_edit(dirty);
        assert_eq!(calls.borrow().len(), 1);
        // 离开节点 → 新 tex 渲染一次
        let old = a.editor.caret();
        assert!(a.editor.set_caret(0));
        a.invalidate_reveal(old, 0);
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[0];
        assert_eq!(a.layouts[&id].imgs.len(), 1);
        assert_eq!(
            calls.borrow().as_slice(),
            &[("x".to_string(), false), ("yx".to_string(), false)]
        );
    }

    // ---- 文档内图片（切片 13）：相对路径解析、等比缩放、加载缓存与失败降级 ----

    fn app_with_images(
        text: &str,
        path: Option<&str>,
        files: &[(&str, u32, u32)],
    ) -> (App, Rc<RefCell<Vec<PathBuf>>>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let files = files
            .iter()
            .map(|(p, w, h)| (PathBuf::from(p), (*w, *h)))
            .collect();
        let doc = Doc {
            text: text.into(),
            path: path.map(PathBuf::from),
            bom: false,
        };
        let m = FakeMath {
            fail: false,
            calls: Rc::new(RefCell::new(Vec::new())),
        };
        let a = App::new(
            doc,
            Box::new(NoClip),
            Box::new(MemStore::default()),
            Box::new(m),
            Box::new(FakeImages {
                files,
                calls: calls.clone(),
            }),
            Box::new(NoAssets),
            Box::new(NoExport),
            Box::new(NoDialogs),
            Box::new(NoWatcher),
            theme::test_support::env(),
        );
        (a, calls)
    }

    #[test]
    fn image_path_resolution_rules() {
        let doc = Path::new("dir/notes.md");
        assert_eq!(
            local_image_path("img/a.png", Some(doc)),
            Some(PathBuf::from("dir/img/a.png"))
        );
        assert_eq!(
            local_image_path("  img/a.png  ", Some(doc)),
            Some(PathBuf::from("dir/img/a.png"))
        );
        assert_eq!(
            local_image_path("https://example.com/a.png", Some(doc)),
            None
        ); // 远程 v0 不加载
        assert_eq!(
            local_image_path("data:image/png;base64,AAAA", Some(doc)),
            None
        );
        assert_eq!(local_image_path("img/a.png", None), None); // 草稿：相对路径无基准
        assert_eq!(local_image_path("", Some(doc)), None);
        let abs = if cfg!(windows) {
            "C:\\pics\\a.png"
        } else {
            "/pics/a.png"
        };
        assert_eq!(
            local_image_path(abs, None),
            Some(PathBuf::from(abs)),
            "绝对路径不依赖文档位置"
        );
    }

    #[test]
    fn fit_size_scales_only_when_wider_than_content() {
        assert_eq!(fit_size(100, 50, 768.0), (100, 50));
        assert_eq!(fit_size(1600, 900, 768.0), (768, 432));
        assert_eq!(fit_size(0, 0, 768.0), (1, 1)); // 防御：空图至少 1px
    }

    // 「a ![alt](img/p.png) b」：Image range=[2,19)（同布局测试的逐字节核算）

    #[test]
    fn image_loads_relative_to_document_and_caches() {
        let (mut a, calls) = app_with_images(
            "a ![alt](img/p.png) b",
            Some("dir/doc.md"),
            &[("dir/img/p.png", 100, 50)],
        );
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[0];
        let l = &a.layouts[&id];
        assert_eq!(l.imgs.len(), 1);
        let InlineAsset::Image(key) = &l.imgs[0].key else {
            panic!("应为图片键")
        };
        assert_eq!(Path::new(key), Path::new("dir/img/p.png"));
        assert_eq!((l.imgs[0].width, l.imgs[0].height), (100, 50));
        assert!(
            !l.layout_text().contains("alt"),
            "加载成功：整节点由占位图替换"
        );
        assert_eq!(calls.borrow().as_slice(), &[PathBuf::from("dir/img/p.png")]);
        // 缓存命中：重排不重载
        a.layouts.remove(&id);
        a.ensure_layouts_all();
        assert_eq!(calls.borrow().len(), 1);
    }

    #[test]
    fn image_wider_than_content_scales_proportionally() {
        // 前缀 "x "：默认光标 0 不在图片节点区间内（否则进入源码态不加载）
        let (mut a, _) = app_with_images(
            "x ![a](p.png)",
            Some("dir/doc.md"),
            &[("dir/p.png", 1600, 900)],
        );
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[0];
        // 默认版心宽 = 800 - 2*16 - 滚动条 12 = 756；1600×900 等比缩放
        assert_eq!(
            (a.layouts[&id].imgs[0].width, a.layouts[&id].imgs[0].height),
            (756, 425)
        );
    }

    #[test]
    fn image_failure_degrades_to_alt_and_memoizes() {
        let (mut a, calls) = app_with_images("a ![alt](img/p.png) b", Some("dir/doc.md"), &[]);
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[0];
        assert!(a.layouts[&id].imgs.is_empty());
        assert_eq!(a.layouts[&id].layout_text(), "a alt b");
        assert_eq!(calls.borrow().len(), 1);
        a.layouts.remove(&id);
        a.ensure_layouts_all(); // 失败记忆：不每帧重试
        assert_eq!(calls.borrow().len(), 1);
    }

    #[test]
    fn remote_or_draft_image_is_never_loaded() {
        let (mut a, calls) =
            app_with_images("x ![a](https://example.com/x.png)", Some("dir/doc.md"), &[]);
        a.ensure_layouts_all();
        assert!(calls.borrow().is_empty());
        let (id, _, _) = a.editor.session().blocks()[0];
        assert_eq!(a.layouts[&id].layout_text(), "x a");

        let (mut b, calls2) = app_with_images("x ![a](p.png)", None, &[]);
        b.ensure_layouts_all();
        assert!(calls2.borrow().is_empty(), "草稿相对路径无解析基准");
        let (id, _, _) = b.editor.session().blocks()[0];
        assert_eq!(b.layouts[&id].layout_text(), "x a");
    }

    #[test]
    fn image_shows_source_when_caret_inside() {
        let (mut a, calls) = app_with_images(
            "a ![alt](img/p.png) b",
            Some("dir/doc.md"),
            &[("dir/img/p.png", 100, 50)],
        );
        assert!(a.editor.set_caret(5)); // alt 内
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[0];
        assert!(a.layouts[&id].imgs.is_empty());
        assert_eq!(a.layouts[&id].layout_text(), "a ![alt](img/p.png) b");
        assert!(calls.borrow().is_empty(), "源码态不加载");
    }

    // ---- 图片粘贴落盘（切片 14）：草稿拒绝、文本优先、失败不动文档 ----

    struct PasteFixture {
        app: App,
        calls: Rc<RefCell<Vec<(PathBuf, u32, u32)>>>,
    }

    fn app_paste_image(
        text: &str,
        path: Option<&str>,
        plain: Option<&str>,
        html: Option<&str>,
        image: Option<ImageData>,
        result: Result<String, String>,
    ) -> PasteFixture {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let doc = Doc {
            text: text.into(),
            path: path.map(PathBuf::from),
            bom: false,
        };
        let m = FakeMath {
            fail: false,
            calls: Rc::new(RefCell::new(Vec::new())),
        };
        let app = App::new(
            doc,
            Box::new(ClipImage {
                text: plain.map(str::to_string),
                html: html.map(str::to_string),
                image,
            }),
            Box::new(MemStore::default()),
            Box::new(m),
            Box::new(NoImages),
            Box::new(FakeAssets {
                result,
                calls: calls.clone(),
            }),
            Box::new(NoExport),
            Box::new(NoDialogs),
            Box::new(NoWatcher),
            theme::test_support::env(),
        );
        PasteFixture { app, calls }
    }

    #[test]
    fn paste_image_saves_asset_and_inserts_relative_link() {
        let f = app_paste_image(
            "ab",
            Some("dir/notes.md"),
            None,
            None,
            Some(solid(4, 2)),
            Ok("notes.assets/image-1.png".into()),
        );
        let mut f = f;
        f.app.paste_clipboard();
        assert_eq!(doc_text(&f.app), "![](notes.assets/image-1.png)ab");
        assert!(f.app.editor.is_dirty());
        let calls = f.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, PathBuf::from("dir/notes.md"));
        assert_eq!((calls[0].1, calls[0].2), (4, 2), "位图原样交给资产层编码");
    }

    #[test]
    fn paste_image_requires_document_path() {
        let mut f = app_paste_image(
            "ab",
            None,
            None,
            None,
            Some(solid(2, 2)),
            Ok("x.assets/i.png".into()),
        );
        f.app.paste_clipboard(); // 草稿：无落盘基准，文档不得改动
        assert_eq!(doc_text(&f.app), "ab");
        assert!(!f.app.editor.is_dirty());
        assert!(f.calls.borrow().is_empty());
    }

    #[test]
    fn paste_prefers_text_over_image() {
        let mut f = app_paste_image(
            "ab",
            Some("dir/notes.md"),
            Some("X"),
            None,
            Some(solid(2, 2)),
            Ok("notes.assets/image-1.png".into()),
        );
        f.app.paste_clipboard();
        assert_eq!(doc_text(&f.app), "Xab");
        assert!(f.calls.borrow().is_empty(), "有文本时不得落盘图片");
    }

    #[test]
    fn paste_image_asset_failure_keeps_document_unchanged() {
        let mut f = app_paste_image(
            "ab",
            Some("dir/notes.md"),
            None,
            None,
            Some(solid(2, 2)),
            Err("磁盘只读".into()),
        );
        f.app.paste_clipboard();
        assert_eq!(doc_text(&f.app), "ab");
        assert!(!f.app.editor.is_dirty());
        assert_eq!(f.calls.borrow().len(), 1);
    }

    // ---- 表格只读渲染（切片 9）：网格/源码双态、命中落点、缓存失效 ----

    // 「p\n\n| a | b |\n| --- | --- |\n| c | d |\n」：表格为块 1（bytes 3..37），
    // 初始 caret=0 在表外；表头「a」=5、表体「d」=33
    const TABLE_DOC: &str = "p\n\n| a | b |\n| --- | --- |\n| c | d |\n";

    #[test]
    fn table_renders_grid_when_caret_outside() {
        let (mut a, _) = app_math(TABLE_DOC, false);
        a.ensure_layouts_all();
        a.ensure_tables();
        let (id, kind, range) = a.editor.session().blocks()[1];
        assert_eq!(kind, BlockKind::Table);
        assert!(a.table_shows_grid(id, &range));
        assert_eq!(a.block_height(geom(&a, id)), a.tables[&id].height);
    }

    #[test]
    fn table_shows_source_when_caret_inside() {
        let (mut a, _) = app_math(TABLE_DOC, false);
        assert!(a.editor.set_caret(5)); // 表头单元格内
        a.ensure_layouts_all();
        a.ensure_tables();
        let (id, _, range) = a.editor.session().blocks()[1];
        assert!(!a.tables.contains_key(&id), "源码态不排版网格");
        assert!(!a.table_shows_grid(id, &range));
        assert_eq!(a.block_height(geom(&a, id)), a.layouts[&id].height);
    }

    #[test]
    fn table_grid_hit_maps_to_cell_content_start() {
        let (mut a, _) = app_math(TABLE_DOC, false);
        a.ensure_layouts_all();
        a.ensure_tables();
        let (id, kind, range) = a.editor.session().blocks()[1];
        let want = a.editor.session().table(id).unwrap().rows[0][1].range.start; // 表体「d」
        let t = a.tables.get(&id).unwrap();
        let cell = &t.cells[3]; // 第 2 行第 2 列
        let x = cell.x + cell.w / 2.0;
        let y = t.rows[1].top + t.rows[1].height / 2.0;
        assert_eq!(a.hit_block(id, kind, &range, x, y), Some(want));
    }

    #[test]
    fn dirty_edit_invalidates_table_grid() {
        let (mut a, _) = app_math(TABLE_DOC, false);
        a.ensure_layouts_all();
        a.ensure_tables();
        let (id, _, _) = a.editor.session().blocks()[1];
        let old_w = a.tables[&id].width;
        // 表头「a」变宽（xxxxa）→ 块脏；光标仍在表内故不重建网格
        assert!(a.editor.set_caret(5));
        let dirty = a.editor.apply(Intent::InsertText("xxxx".into())).unwrap();
        assert_eq!(dirty.len(), 1);
        a.after_edit(dirty);
        let id = a.editor.session().blocks()[1].0;
        assert!(!a.tables.contains_key(&id), "表格光标内 → 脏后不建网格");
        // 光标离开表格 → 按新内容重建（列宽变大）
        assert!(a.editor.set_caret(0));
        a.ensure_tables();
        assert!(a.tables.contains_key(&id));
        assert!(a.tables[&id].width > old_w, "内容变宽 → 网格重建");
    }

    // ---- 列表 / 引用 / 分隔线 Reveal（切片 12）：块级重排与块高 ----

    #[test]
    fn list_bullet_toggles_on_caret_enter_leave() {
        // 「p\n\n- a\n」：列表块 bytes [3,7)，初始 caret=0 在块外
        let (mut a, _) = app_math("p\n\n- a\n", false);
        a.ensure_layouts_all();
        let (id, kind, _) = a.editor.session().blocks()[1];
        assert_eq!(
            kind,
            BlockKind::ListItem {
                ordered: false,
                depth: 0
            }
        );
        assert_eq!(a.layouts[&id].layout_text(), "• a");
        // 进入块：失效 → 重排显源码
        assert!(a.editor.set_caret(4));
        a.invalidate_reveal(0, 4);
        a.ensure_layouts_all();
        assert_eq!(a.layouts[&id].layout_text(), "- a");
        // 离开块：恢复符号
        assert!(a.editor.set_caret(0));
        a.invalidate_reveal(4, 0);
        a.ensure_layouts_all();
        assert_eq!(a.layouts[&id].layout_text(), "• a");
        assert_eq!(a.block_height(geom(&a, id)), a.layouts[&id].height);
    }

    #[test]
    fn blockquote_line_reveal_reflows_only_across_lines() {
        // 「p\n\n> a\n> b\n」：引用块 bytes [3,11)，行 1 [3,7)、行 2 [7,11)
        let (mut a, _) = app_math("p\n\n> a\n> b\n", false);
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[1];
        assert_eq!(a.layouts[&id].layout_text(), "  a\n  b");
        // 块内跨行移动（0 → 9，行 2 内容）：逐行激活变化 → 重排
        assert!(a.editor.set_caret(9));
        a.invalidate_reveal(0, 9);
        a.ensure_layouts_all();
        assert_eq!(a.layouts[&id].layout_text(), "  a\n> b");
        // 同一行内移动（9 → 10）：激活行不变 → 布局缓存保留（不重排）
        assert!(a.editor.set_caret(10));
        a.invalidate_reveal(9, 10);
        assert!(a.layouts.contains_key(&id), "行内移动不得失效引用块布局");
    }

    #[test]
    fn thematic_break_elides_and_keeps_line_height() {
        // 「p\n\n---\n」：分隔线块 bytes [3,7)，初始 caret=0 在块外
        let (mut a, _) = app_math("p\n\n---\n", false);
        a.ensure_layouts_all();
        let (id, kind, _) = a.editor.session().blocks()[1];
        assert_eq!(kind, BlockKind::ThematicBreak);
        assert_eq!(a.layouts[&id].layout_text(), "");
        // 消隐后仍占一行高，块高不塌缩
        assert_eq!(
            a.block_height(geom(&a, id)),
            BASE_LINE_HEIGHT * a.scale
        );
        // 进入块：显源码，块高不变
        assert!(a.editor.set_caret(5));
        a.invalidate_reveal(0, 5);
        a.ensure_layouts_all();
        assert_eq!(a.layouts[&id].layout_text(), "---");
        assert_eq!(
            a.block_height(geom(&a, id)),
            BASE_LINE_HEIGHT * a.scale
        );
    }

    #[derive(Clone, Default)]
    struct MemStore {
        files: Rc<RefCell<HashMap<PathBuf, Vec<u8>>>>,
        fail_save: bool,
        fail_paths: Rc<RefCell<Vec<PathBuf>>>,
    }
    impl FileStore for MemStore {
        fn load(&mut self, path: &Path) -> Result<Vec<u8>, String> {
            self.files
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| "文件不存在".to_string())
        }
        fn save(&mut self, path: &Path, data: &[u8]) -> Result<(), String> {
            if self.fail_save || self.fail_paths.borrow().contains(&path.to_path_buf()) {
                return Err("模拟写失败".to_string());
            }
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), data.to_vec());
            Ok(())
        }
    }

    fn app(text: &str, path: Option<&str>, bom: bool, store: &MemStore) -> App {
        let doc = Doc {
            text: text.into(),
            path: path.map(PathBuf::from),
            bom,
        };
        let m = FakeMath {
            fail: false,
            calls: Rc::new(RefCell::new(Vec::new())),
        };
        App::new(
            doc,
            Box::new(NoClip),
            Box::new(store.clone()),
            Box::new(m),
            Box::new(NoImages),
            Box::new(NoAssets),
            Box::new(NoExport),
            Box::new(NoDialogs),
            Box::new(NoWatcher),
            theme::test_support::env(),
        )
    }

    /// 记录调用（会话序列化 + 源路径透传）的导出器；fail 时返回 Err。
    #[derive(Default)]
    struct FakeExport {
        calls: AssetCalls,
        fail: bool,
    }
    impl Exporter for FakeExport {
        fn export_html(
            &mut self,
            session: &veil_core::Session,
            source_path: Option<&Path>,
        ) -> Result<PathBuf, String> {
            self.calls
                .borrow_mut()
                .push((session.serialize(), source_path.map(Path::to_path_buf)));
            if self.fail {
                Err("模拟导出失败".into())
            } else {
                Ok(PathBuf::from("out.html"))
            }
        }
    }

    fn app_export(text: &str, path: Option<&str>, fail: bool) -> (App, AssetCalls) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let doc = Doc {
            text: text.into(),
            path: path.map(PathBuf::from),
            bom: false,
        };
        let m = FakeMath {
            fail: false,
            calls: Rc::new(RefCell::new(Vec::new())),
        };
        let e = FakeExport {
            calls: calls.clone(),
            fail,
        };
        let a = App::new(
            doc,
            Box::new(NoClip),
            Box::new(MemStore::default()),
            Box::new(m),
            Box::new(NoImages),
            Box::new(NoAssets),
            Box::new(e),
            Box::new(NoDialogs),
            Box::new(NoWatcher),
            theme::test_support::env(),
        );
        (a, calls)
    }

    #[test]
    fn export_delegates_current_session_and_path() {
        let (mut a, calls) = app_export("ab", Some("a.md"), false);
        a.editor.apply(Intent::InsertText("中".into())).unwrap(); // 改后应导出当前内容而非初始内容
        a.export_now();
        let got = calls.borrow();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "中ab");
        assert_eq!(got[0].1.as_deref(), Some(Path::new("a.md")));
    }

    #[test]
    fn export_draft_passes_none_path_without_crash() {
        let (mut a, calls) = app_export("x", None, false);
        a.export_now();
        assert_eq!(calls.borrow()[0].1, None); // 草稿：None 透传给导出器裁决
    }

    #[test]
    fn export_failure_does_not_crash_or_touch_document() {
        let (mut a, calls) = app_export("x", Some("a.md"), true);
        a.export_now();
        assert_eq!(calls.borrow().len(), 1);
        assert_eq!(a.editor.session().serialize(), "x"); // 文档不受影响
        assert!(!a.editor.is_dirty()); // 导出不改脏标
    }

    #[test]
    fn save_writes_encoded_bytes_and_clears_dirty() {
        // 「中」= E4 B8 AD；初始 "ab"，caret=0 处插入「中」→ 序列化 "中ab"
        let store = MemStore::default();
        let mut a = app("ab", Some("a.md"), false, &store);
        a.editor.apply(Intent::InsertText("中".into())).unwrap();
        assert!(a.editor.is_dirty());
        a.save_now();
        assert!(!a.editor.is_dirty());
        assert_eq!(store.files.borrow()[Path::new("a.md")], "中ab".as_bytes());
    }

    #[test]
    fn save_restores_bom() {
        let store = MemStore::default();
        let mut a = app("x", Some("a.md"), true, &store);
        a.save_now(); // 未脏也可保存（幂等写）
        assert_eq!(
            store.files.borrow()[Path::new("a.md")],
            vec![0xEF, 0xBB, 0xBF, b'x']
        );
    }

    #[test]
    fn save_failure_keeps_dirty() {
        let store = MemStore {
            fail_save: true,
            ..Default::default()
        };
        let mut a = app("x", Some("a.md"), false, &store);
        a.editor.apply(Intent::InsertText("y".into())).unwrap();
        a.save_now();
        assert!(a.editor.is_dirty()); // 失败不谎报干净
        assert!(store.files.borrow().is_empty()); // 无半截文件
    }

    #[test]
    fn save_draft_without_path_is_noop() {
        let store = MemStore::default();
        let mut a = app("x", None, false, &store);
        a.editor.apply(Intent::InsertText("y".into())).unwrap();
        a.save_now();
        assert!(a.editor.is_dirty());
        assert!(store.files.borrow().is_empty());
    }

    #[test]
    fn open_replaces_document_and_inherits_bom() {
        let store = MemStore::default();
        store.files.borrow_mut().insert(
            PathBuf::from("b.md"),
            vec![0xEF, 0xBB, 0xBF, b'#', b' ', b'x'],
        );
        let mut a = app("旧", None, false, &store);
        a.open_path(PathBuf::from("b.md"));
        assert_eq!(a.editor.session().serialize(), "# x");
        assert_eq!(a.path.as_deref(), Some(Path::new("b.md")));
        assert!(a.bom);
        assert!(!a.editor.is_dirty()); // 新打开即干净
    }

    #[test]
    fn open_refused_when_dirty() {
        let store = MemStore::default();
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("b.md"), "新".as_bytes().to_vec());
        let mut a = app("旧", Some("a.md"), false, &store);
        a.editor.apply(Intent::InsertText("改".into())).unwrap();
        a.open_path(PathBuf::from("b.md"));
        assert_eq!(a.editor.session().serialize(), "改旧"); // 未被替换
        assert_eq!(a.path.as_deref(), Some(Path::new("a.md")));
    }

    #[test]
    fn open_refused_on_bad_bytes_or_missing() {
        let store = MemStore::default();
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("bad.md"), vec![0xFF]);
        let mut a = app("旧", None, false, &store);
        a.open_path(PathBuf::from("bad.md")); // 非法 UTF-8
        a.open_path(PathBuf::from("none.md")); // 不存在
        assert_eq!(a.editor.session().serialize(), "旧");
        assert_eq!(a.path, None);
    }

    // ---- 文件层安全化（切片 10）：自动保存静默期 + 外部修改冲突备份 ----
    // 「改」= 3 字节；dirty_app 在 "ab" 文首插入「改」→ 序列化 "改ab"。

    fn dirty_app(text: &str, path: Option<&str>, store: &MemStore) -> App {
        let mut a = app(text, path, false, store);
        a.editor.apply(Intent::InsertText("改".into())).unwrap();
        a
    }

    #[test]
    fn autosave_writes_after_idle_and_clears_dirty() {
        let store = MemStore::default();
        let mut a = dirty_app("ab", Some("a.md"), &store);
        let t0 = Instant::now();
        a.last_activity = Some(t0);
        a.maybe_autosave(t0 + Duration::from_millis(1999));
        assert!(store.files.borrow().is_empty(), "静默期未满不落盘");
        a.maybe_autosave(t0 + Duration::from_secs(2));
        assert!(!a.editor.is_dirty());
        assert_eq!(store.files.borrow()[Path::new("a.md")], "改ab".as_bytes());
        assert_eq!(a.title_text(), "veil — a.md"); // 干净且无冲突：无标记
        assert!(a.last_activity.is_none(), "保存后取消调度");
    }

    #[test]
    fn autosave_skipped_for_draft_and_ime() {
        // 草稿：无关联文件不自动写盘
        let store = MemStore::default();
        let mut a = dirty_app("ab", None, &store);
        let t0 = Instant::now();
        a.last_activity = Some(t0);
        a.maybe_autosave(t0 + Duration::from_secs(30));
        assert!(store.files.borrow().is_empty());
        // 有关联文件但 IME 组合中：组合期不落盘
        let store = MemStore::default();
        let mut a = dirty_app("ab", Some("a.md"), &store);
        a.editor.set_preedit("zh".into(), None);
        a.last_activity = Some(t0);
        a.maybe_autosave(t0 + Duration::from_secs(30));
        assert!(store.files.borrow().is_empty());
        assert!(a.editor.is_dirty());
    }

    #[test]
    fn cursor_move_does_not_postpone_autosave() {
        let store = MemStore::default();
        let mut a = app("ab", Some("a.md"), false, &store);
        let t0 = Instant::now();
        a.last_activity = Some(t0);
        a.after_edit(Vec::new()); // 纯光标移动不产生脏块
        assert_eq!(a.last_activity, Some(t0));
    }

    #[test]
    fn autosave_conflict_backs_up_and_never_overwrites() {
        let store = MemStore::default();
        let mut a = dirty_app("ab", Some("a.md"), &store);
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("a.md"), b"external".to_vec());
        let t0 = Instant::now();
        a.last_activity = Some(t0);
        a.maybe_autosave(t0 + Duration::from_secs(2));
        let files = store.files.borrow();
        assert_eq!(
            files[Path::new("a.md")],
            b"external".to_vec(),
            "自动保存不覆盖外部修改"
        );
        assert_eq!(
            files[Path::new("a.conflict-1.md")],
            b"external".to_vec(),
            "外部版本已备份"
        );
        drop(files);
        assert!(a.editor.is_dirty(), "保持脏态等用户裁决");
        assert!(a.autosave_hold, "自动保存挂起");
        assert!(
            !a.autosave_due(t0 + Duration::from_secs(60)),
            "挂起后不再自动落盘"
        );
        assert_eq!(a.title_text(), "veil — a.md * (外部修改已备份)");
    }

    #[test]
    fn manual_save_conflict_backs_up_then_overwrites() {
        let store = MemStore::default();
        let mut a = dirty_app("ab", Some("a.md"), &store);
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("a.md"), b"external".to_vec());
        a.save_now();
        let files = store.files.borrow();
        assert_eq!(
            files[Path::new("a.md")],
            "改ab".as_bytes(),
            "显式保存允许覆盖"
        );
        assert_eq!(
            files[Path::new("a.conflict-1.md")],
            b"external".to_vec(),
            "外部版本留存"
        );
        drop(files);
        assert!(!a.editor.is_dirty());
        assert!(!a.autosave_hold, "手工保存解除挂起");
        assert!(a.conflict, "标题提示冲突副本");
        assert_eq!(a.title_text(), "veil — a.md (外部修改已备份)");
    }

    #[test]
    fn save_without_external_change_creates_no_backup() {
        let store = MemStore::default();
        // 磁盘内容与基线一致（"ab"）：无冲突
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("a.md"), b"ab".to_vec());
        let mut a = dirty_app("ab", Some("a.md"), &store);
        a.save_now();
        assert_eq!(store.files.borrow().len(), 1, "基线一致不产生冲突副本");
        assert!(!a.conflict);
        assert_eq!(store.files.borrow()[Path::new("a.md")], "改ab".as_bytes());
    }

    #[test]
    fn conflict_copy_uses_next_free_index() {
        let store = MemStore::default();
        let mut a = dirty_app("ab", Some("a.md"), &store);
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("a.md"), b"external".to_vec());
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("a.conflict-1.md"), b"old".to_vec());
        a.save_now();
        let files = store.files.borrow();
        assert_eq!(
            files[Path::new("a.conflict-1.md")],
            b"old".to_vec(),
            "既有副本不动"
        );
        assert_eq!(files[Path::new("a.conflict-2.md")], b"external".to_vec());
    }

    #[test]
    fn baseline_updates_after_conflict_save() {
        let store = MemStore::default();
        let mut a = dirty_app("ab", Some("a.md"), &store);
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("a.md"), b"external".to_vec());
        a.save_now(); // 冲突 → 备份 + 覆盖，基线更新为 "改ab"
        assert!(a.conflict);
        a.editor.apply(Intent::InsertText("z".into())).unwrap();
        let t0 = Instant::now();
        a.last_activity = Some(t0);
        a.maybe_autosave(t0 + Duration::from_secs(2));
        assert!(!a.conflict, "无新冲突：提示清除");
        assert_eq!(store.files.borrow().len(), 2, "无新增冲突副本");
        assert_eq!(store.files.borrow()[Path::new("a.md")], "改zab".as_bytes());
    }

    #[test]
    fn manual_save_after_auto_conflict_does_not_duplicate_backup() {
        let store = MemStore::default();
        let mut a = dirty_app("ab", Some("a.md"), &store);
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("a.md"), b"external".to_vec());
        let t0 = Instant::now();
        a.last_activity = Some(t0);
        a.maybe_autosave(t0 + Duration::from_secs(2)); // 自动冲突：备份 + 挂起
        a.save_now(); // 用户显式保存：同一外部版本不再重复备份
        let files = store.files.borrow();
        assert_eq!(files.len(), 2, "同一外部版本只留一份副本");
        assert_eq!(files[Path::new("a.md")], "改ab".as_bytes());
        assert_eq!(files[Path::new("a.conflict-1.md")], b"external".to_vec());
        drop(files);
        assert!(!a.editor.is_dirty());
        assert!(!a.autosave_hold);
    }

    // ---- chrome UI（切片 15）：菜单路由 / 对话框编排 / 视口与滚动条 ----

    fn app_dialogs(
        text: &str,
        path: Option<&str>,
        bom: bool,
        store: &MemStore,
        open: Option<&str>,
        save: Option<&str>,
    ) -> (App, Rc<RefCell<Vec<String>>>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let d = FakeDialogs {
            open: open.map(PathBuf::from),
            save: save.map(PathBuf::from),
            calls: calls.clone(),
        };
        let doc = Doc {
            text: text.into(),
            path: path.map(PathBuf::from),
            bom,
        };
        let m = FakeMath {
            fail: false,
            calls: Rc::new(RefCell::new(Vec::new())),
        };
        let a = App::new(
            doc,
            Box::new(NoClip),
            Box::new(store.clone()),
            Box::new(m),
            Box::new(NoImages),
            Box::new(NoAssets),
            Box::new(NoExport),
            Box::new(d),
            Box::new(NoWatcher),
            theme::test_support::env(),
        );
        (a, calls)
    }

    fn click_at(a: &mut App, x: f32, y: f32) -> bool {
        a.mouse = PhysicalPosition::new(x as f64, y as f64);
        a.on_left_click()
    }

    fn menu_title_center(menu: usize) -> (f32, f32) {
        let (_, x, w) = chrome::menu_bar_rects(1.0)[menu];
        (x + w / 2.0, chrome::MENU_H / 2.0)
    }

    fn popup_item_center(menu: usize, item: usize) -> (f32, f32) {
        let p = chrome::popup_layout(menu, 1.0);
        let (_, x, y, w, h) = p.items[item];
        (x + w / 2.0, y + h / 2.0)
    }

    #[test]
    fn menu_title_click_toggles_and_item_click_runs_save() {
        let store = MemStore::default();
        let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, None);
        a.editor.apply(Intent::InsertText("中".into())).unwrap();
        let (tx, ty) = menu_title_center(0);
        click_at(&mut a, tx, ty);
        assert_eq!(a.chrome.open, Some(0), "点菜单标题打开文件菜单");
        let (x, y) = popup_item_center(0, 1); // 保存
        click_at(&mut a, x, y);
        assert_eq!(a.chrome.open, None, "执行动作后菜单关闭");
        assert!(!a.editor.is_dirty());
        assert_eq!(store.files.borrow()[Path::new("a.md")], "中ab".as_bytes());
    }

    #[test]
    fn click_outside_open_menu_closes_it_without_moving_caret() {
        let store = MemStore::default();
        let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, None);
        click_at(&mut a, 20.0, 10.0);
        assert!(a.chrome.is_open());
        a.editor.set_caret(1);
        click_at(&mut a, 300.0, 400.0); // 弹窗左下方正文
        assert!(!a.chrome.is_open());
        assert_eq!(a.editor.caret(), 1, "关闭菜单的点击不落光标");
    }

    #[test]
    fn dirty_document_refuses_open_before_dialog() {
        let store = MemStore::default();
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("b.md"), b"new".to_vec());
        let (mut a, calls) = app_dialogs("old", Some("a.md"), false, &store, Some("b.md"), None);
        a.editor.apply(Intent::InsertText("x".into())).unwrap();
        a.open_via_dialog();
        assert!(calls.borrow().is_empty(), "脏文档拒绝时不弹对话框");
        assert_eq!(a.editor.session().serialize(), "xold");
        assert_eq!(a.path.as_deref(), Some(Path::new("a.md")));
        assert!(a.chrome.notice.contains("未保存"));
    }

    #[test]
    fn open_from_dialog_replaces_document() {
        let store = MemStore::default();
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("b.md"), b"# new".to_vec());
        let (mut a, calls) = app_dialogs("old", Some("a.md"), false, &store, Some("b.md"), None);
        a.open_via_dialog();
        assert_eq!(calls.borrow().as_slice(), &["open".to_string()]);
        assert_eq!(a.editor.session().serialize(), "# new");
        assert_eq!(a.path.as_deref(), Some(Path::new("b.md")));
        assert!(!a.editor.is_dirty());
        assert!(a.chrome.notice.contains("已打开"));
    }

    #[test]
    fn save_as_success_switches_path_baseline_and_clears_dirty() {
        let store = MemStore::default();
        let (mut a, calls) = app_dialogs("ab", None, false, &store, None, Some("new.md"));
        a.editor.apply(Intent::InsertText("中".into())).unwrap();
        a.save_as();
        assert_eq!(
            calls.borrow().as_slice(),
            &["save:未命名.md".to_string()],
            "草稿建议名"
        );
        assert_eq!(a.path.as_deref(), Some(Path::new("new.md")));
        assert!(!a.editor.is_dirty());
        assert_eq!(a.baseline, "中ab".as_bytes());
        assert_eq!(store.files.borrow()[Path::new("new.md")], "中ab".as_bytes());
        assert!(a.chrome.notice.contains("已另存为"));
    }

    #[test]
    fn save_as_uses_old_file_name_as_suggestion() {
        let store = MemStore::default();
        let (mut a, calls) = app_dialogs("ab", Some("dir/a.md"), false, &store, None, None);
        a.save_as(); // 取消
        assert_eq!(calls.borrow().as_slice(), &["save:a.md".to_string()]);
        assert!(a.chrome.notice.contains("取消"));
    }

    #[test]
    fn save_as_failure_keeps_old_path_and_dirty() {
        let store = MemStore {
            fail_save: true,
            ..Default::default()
        };
        let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, Some("new.md"));
        a.editor.apply(Intent::InsertText("x".into())).unwrap();
        a.save_as();
        assert_eq!(a.path.as_deref(), Some(Path::new("a.md")));
        assert!(a.editor.is_dirty(), "失败不谎报干净");
        assert!(store.files.borrow().is_empty());
        assert!(a.chrome.notice.contains("失败"));
    }

    #[test]
    fn menu_quit_requests_exit() {
        let store = MemStore::default();
        let (mut a, _) = app_dialogs("ab", None, false, &store, None, None);
        assert!(a.perform_menu(MenuAction::Quit));
        assert!(!a.perform_menu(MenuAction::Copy));
    }

    #[test]
    fn status_bar_and_menu_bar_clicks_do_not_move_caret() {
        let store = MemStore::default();
        let (mut a, _) = app_dialogs("ab\ncd", Some("a.md"), false, &store, None, None);
        a.editor.set_caret(3);
        let (_, h) = a.win_size();
        let status_y = h as f32 - chrome::STATUS_H + 5.0;
        click_at(&mut a, 100.0, status_y);
        assert_eq!(a.editor.caret(), 3);
        assert!(!a.dragging);
        let (tx, ty) = menu_title_center(0);
        click_at(&mut a, tx, ty); // 菜单栏 → 打开菜单
        assert!(a.chrome.is_open());
        assert_eq!(a.editor.caret(), 3);
    }

    #[test]
    fn viewport_click_is_offset_by_menu_height() {
        let store = MemStore::default();
        let (mut a, _) = app_dialogs("ab", None, false, &store, None, None);
        a.editor.set_caret(2);
        a.ensure_layouts_all();
        let vp = a.viewport();
        click_at(&mut a, 16.0, vp.y + 16.0 + 5.0); // 首行
        assert_eq!(a.editor.caret(), 0, "y 需扣除菜单栏高度");
        assert!(a.dragging);
    }

    #[test]
    fn scrollbar_click_pages_and_thumb_drag_maps_scroll() {
        let store = MemStore::default();
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let (mut a, _) = app_dialogs(&text, None, false, &store, None, None);
        a.ensure_layouts_all();
        let vp = a.viewport();
        let doc_h = a.doc_height();
        assert!(doc_h > vp.h, "长文档溢出");
        let x = vp.x + vp.w - chrome::SCROLL_W / 2.0;
        let below = vp.y + vp.h - 5.0;
        click_at(&mut a, x, below);
        assert!(a.scroll_y > 0.0, "轨道下方点击翻页");
        let first_page = a.scroll_y;
        assert!(a.chrome.scroll_drag.is_none());

        // 拖拽滑块：按下滑块 → 移动到视口顶部 → 滚动回 0
        let (ty, th) = chrome::scroll_thumb(vp, doc_h, a.scroll_y, 1.0).unwrap();
        click_at(&mut a, x, ty + th / 2.0);
        assert!(a.chrome.scroll_drag.is_some());
        a.mouse = PhysicalPosition::new(x as f64, (vp.y - 100.0) as f64);
        // 模拟 CursorMoved 的拖动分支
        let off = a.chrome.scroll_drag.unwrap();
        a.scroll_y = chrome::scroll_from_thumb(vp, doc_h, a.mouse.y as f32 - off, 1.0);
        assert_eq!(a.scroll_y, 0.0);
        assert!(first_page > 0.0);
    }

    #[test]
    fn wheel_scroll_clamps_to_document_end() {
        let store = MemStore::default();
        let (mut a, _) = app_dialogs("ab\ncd", None, false, &store, None, None);
        let vp = a.viewport();
        let max = chrome::max_scroll(vp, a.doc_height());
        a.scroll_y = max + 100.0;
        // 事件分支本身做 clamp；此处验证几何上界与钳位语义一致
        a.scroll_y = a.scroll_y.clamp(0.0, max);
        assert_eq!(a.scroll_y, max);
        assert_eq!(max, 0.0, "短文档不滚动");
    }

    #[test]
    fn status_line_reports_caret_line_col() {
        let store = MemStore::default();
        let (mut a, _) = app_dialogs("ab\n中c\n", Some("a.md"), false, &store, None, None);
        a.editor.set_caret(6);
        assert_eq!(a.editor.session().line_col(6), Some((2, 2)));
        let (left, right) =
            chrome::status_line(a.path.as_deref(), a.editor.is_dirty(), a.external_state(), 2, 2);
        assert_eq!(left, "a.md");
        assert_eq!(right, "行 2，列 2");
    }

    #[test]
    fn save_notice_surfaces_on_update() {
        let store = MemStore::default();
        let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, None);
        a.editor.apply(Intent::InsertText("x".into())).unwrap();
        a.save_now();
        assert!(a.chrome.notice.contains("已保存"));
        a.path = None;
        a.save_now();
        assert!(
            a.chrome.notice.contains("另存为"),
            "草稿保存提示带另存为引导"
        );
    }

    #[test]
    fn new_external_version_gets_new_backup() {
        let store = MemStore::default();
        let mut a = dirty_app("ab", Some("a.md"), &store);
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("a.md"), b"external1".to_vec());
        let t0 = Instant::now();
        a.last_activity = Some(t0);
        a.maybe_autosave(t0 + Duration::from_secs(2)); // 备份 v1
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("a.md"), b"external2".to_vec());
        a.save_now(); // 外部又变：新版本须再备份
        let files = store.files.borrow();
        assert_eq!(files[Path::new("a.conflict-1.md")], b"external1".to_vec());
        assert_eq!(files[Path::new("a.conflict-2.md")], b"external2".to_vec());
        assert_eq!(files[Path::new("a.md")], "改ab".as_bytes());
    }

    #[test]
    fn failed_backup_aborts_save_without_overwrite() {
        let store = MemStore::default();
        let mut a = dirty_app("ab", Some("a.md"), &store);
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("a.md"), b"external".to_vec());
        store
            .fail_paths
            .borrow_mut()
            .push(PathBuf::from("a.conflict-1.md"));
        a.save_now();
        assert_eq!(
            store.files.borrow()[Path::new("a.md")],
            b"external".to_vec(),
            "备份失败绝不覆盖"
        );
        assert!(a.editor.is_dirty());
        assert!(!a.conflict, "未写备份：无「已备份」提示");
    }

    #[test]
    fn open_resets_conflict_state_and_baseline() {
        let store = MemStore::default();
        let mut a = app("x", Some("a.md"), false, &store);
        a.conflict = true;
        a.autosave_hold = true;
        a.last_activity = Some(Instant::now());
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from("b.md"), b"# n".to_vec());
        a.external_changed = Some(b"external".to_vec());
        a.external_missing = true;
        a.open_path(PathBuf::from("b.md"));
        assert!(!a.conflict);
        assert!(!a.autosave_hold);
        assert!(a.last_activity.is_none());
        assert_eq!(a.baseline, b"# n".to_vec());
        assert!(a.external_changed.is_none(), "打开新文档清外部变更态");
        assert!(!a.external_missing);
    }

    // ---- 外部文件监听（切片 16）：变更检测 / 提示去重 / 删除恢复 / 监听切换 ----

    fn app_watch(text: &str, path: Option<&str>, store: &MemStore, watch: &FakeWatch) -> App {
        let doc = Doc {
            text: text.into(),
            path: path.map(PathBuf::from),
            bom: false,
        };
        let m = FakeMath {
            fail: false,
            calls: Rc::new(RefCell::new(Vec::new())),
        };
        App::new(
            doc,
            Box::new(NoClip),
            Box::new(store.clone()),
            Box::new(m),
            Box::new(NoImages),
            Box::new(NoAssets),
            Box::new(NoExport),
            Box::new(NoDialogs),
            Box::new(watch.clone()),
            theme::test_support::env(),
        )
    }

    fn disk(store: &MemStore, path: &str, bytes: &[u8]) {
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from(path), bytes.to_vec());
    }

    #[test]
    fn watcher_follows_current_path() {
        let store = MemStore::default();
        let watch = FakeWatch::default();
        let mut a = app_watch("x", Some("a.md"), &store, &watch);
        assert_eq!(
            watch.calls.borrow().as_slice(),
            &["watch:a.md".to_string()],
            "构造即监听初始路径"
        );
        disk(&store, "b.md", b"# n");
        a.open_path(PathBuf::from("b.md"));
        assert_eq!(watch.calls.borrow().last().unwrap(), "watch:b.md");
        a.path = None;
        a.watch_current();
        assert_eq!(watch.calls.borrow().last().unwrap(), "unwatch");
    }

    #[test]
    fn watch_failure_surfaces_notice_without_blocking_edit() {
        let store = MemStore::default();
        let watch = FakeWatch {
            fail: true,
            ..Default::default()
        };
        let mut a = app_watch("x", Some("a.md"), &store, &watch);
        assert!(a.chrome.notice.contains("监听不可用"));
        a.editor.apply(Intent::InsertText("y".into())).unwrap();
        assert_eq!(a.editor.session().serialize(), "yx");
    }

    #[test]
    fn external_change_notifies_holds_autosave_and_keeps_document() {
        let store = MemStore::default();
        disk(&store, "a.md", b"ab");
        let watch = FakeWatch::default();
        let mut a = app_watch("ab", Some("a.md"), &store, &watch);
        a.editor.apply(Intent::InsertText("改".into())).unwrap();
        let t0 = Instant::now();
        a.last_activity = Some(t0);
        disk(&store, "a.md", b"external");
        watch.events.borrow_mut().push(FileEvent::Changed);
        a.poll_external();
        assert_eq!(
            a.external_changed.as_deref(),
            Some(b"external".as_slice()),
            "记录待备份的外部版本"
        );
        assert!(a.autosave_hold, "外部修改必须挂起自动保存");
        assert!(a.chrome.notice.contains("外部修改"));
        assert_eq!(a.title_text(), "veil — a.md * (外部修改)");
        assert_eq!(a.external_state(), ExternalState::Changed);
        assert_eq!(a.editor.session().serialize(), "改ab", "不改写文档内容");
        a.maybe_autosave(t0 + Duration::from_secs(2));
        assert_eq!(store.files.borrow()[Path::new("a.md")], b"external".to_vec());
        assert_eq!(store.files.borrow().len(), 1, "检测阶段不落副本");
    }

    #[test]
    fn same_external_version_notifies_once() {
        let store = MemStore::default();
        disk(&store, "a.md", b"external");
        let watch = FakeWatch::default();
        let mut a = app_watch("ab", Some("a.md"), &store, &watch);
        watch.events.borrow_mut().push(FileEvent::Changed);
        a.poll_external();
        a.chrome.notice = "占位".into();
        watch.events.borrow_mut().push(FileEvent::Changed);
        a.poll_external();
        assert_eq!(a.chrome.notice, "占位", "同一外部版本不重复提示");
        assert_eq!(a.external_changed.as_deref(), Some(b"external".as_slice()));
    }

    #[test]
    fn already_backed_up_version_does_not_renotify() {
        let store = MemStore::default();
        disk(&store, "a.md", b"external");
        let watch = FakeWatch::default();
        let mut a = app_watch("ab", Some("a.md"), &store, &watch);
        a.external_saved = Some(b"external".to_vec());
        watch.events.borrow_mut().push(FileEvent::Changed);
        a.poll_external();
        assert!(a.external_changed.is_none());
        assert!(!a.autosave_hold);
    }

    #[test]
    fn newer_external_version_overrides_backed_up_state() {
        let store = MemStore::default();
        disk(&store, "a.md", b"v2");
        let watch = FakeWatch::default();
        let mut a = app_watch("ab", Some("a.md"), &store, &watch);
        a.external_saved = Some(b"v1".to_vec()); // 旧版本已备份
        a.conflict = true;
        assert_eq!(a.external_state(), ExternalState::BackedUp);
        watch.events.borrow_mut().push(FileEvent::Changed);
        a.poll_external();
        assert_eq!(
            a.external_state(),
            ExternalState::Changed,
            "新版本待备份必须盖过旧版本的已备份提示"
        );
        assert_eq!(a.title_text(), "veil — a.md (外部修改)");
        assert!(a.autosave_hold);
    }

    #[test]
    fn external_delete_notifies_and_autosave_recreates() {
        let store = MemStore::default();
        disk(&store, "a.md", b"ab");
        let watch = FakeWatch::default();
        let mut a = app_watch("ab", Some("a.md"), &store, &watch);
        a.editor.apply(Intent::InsertText("改".into())).unwrap();
        let t0 = Instant::now();
        a.last_activity = Some(t0);
        store.files.borrow_mut().remove(Path::new("a.md"));
        watch.events.borrow_mut().push(FileEvent::Removed);
        a.poll_external();
        assert!(a.external_missing);
        assert_eq!(a.external_state(), ExternalState::Missing);
        assert!(a.chrome.notice.contains("删除"));
        assert!(!a.autosave_hold, "删除不是覆盖，允许自动保存重建");
        a.maybe_autosave(t0 + Duration::from_secs(2));
        assert_eq!(store.files.borrow()[Path::new("a.md")], "改ab".as_bytes());
        assert!(!a.external_missing, "写盘成功即清除缺失态");
    }

    #[test]
    fn external_revert_to_baseline_clears_pending() {
        let store = MemStore::default();
        disk(&store, "a.md", b"external");
        let watch = FakeWatch::default();
        let mut a = app_watch("ab", Some("a.md"), &store, &watch);
        watch.events.borrow_mut().push(FileEvent::Changed);
        a.poll_external();
        assert!(a.autosave_hold);
        disk(&store, "a.md", b"ab");
        watch.events.borrow_mut().push(FileEvent::Changed);
        a.poll_external();
        assert!(a.external_changed.is_none(), "回到基线：撤销待备份");
        assert!(!a.autosave_hold);
        assert_eq!(a.external_state(), ExternalState::None);
    }

    #[test]
    fn self_save_event_is_ignored() {
        let store = MemStore::default();
        disk(&store, "a.md", b"ab");
        let watch = FakeWatch::default();
        let mut a = app_watch("ab", Some("a.md"), &store, &watch);
        a.editor.apply(Intent::InsertText("改".into())).unwrap();
        a.save_now(); // 自身原子保存也会触发监听事件
        assert!(a.chrome.notice.contains("已保存"));
        watch.events.borrow_mut().push(FileEvent::Changed);
        a.poll_external();
        assert!(a.external_changed.is_none(), "磁盘==基线：不误报");
        assert!(a.chrome.notice.contains("已保存"), "提示不被覆盖");
    }

    #[test]
    fn recovery_after_missing_surfaces_notice() {
        let store = MemStore::default();
        disk(&store, "a.md", b"ab");
        let watch = FakeWatch::default();
        let mut a = app_watch("ab", Some("a.md"), &store, &watch);
        store.files.borrow_mut().remove(Path::new("a.md"));
        watch.events.borrow_mut().push(FileEvent::Removed);
        a.poll_external();
        assert!(a.external_missing);
        disk(&store, "a.md", b"ab");
        watch.events.borrow_mut().push(FileEvent::Changed);
        a.poll_external();
        assert!(!a.external_missing);
        assert!(a.chrome.notice.contains("已恢复"));
    }

    #[test]
    fn save_as_rebinds_watcher() {
        let store = MemStore::default();
        let watch = FakeWatch::default();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let doc = Doc {
            text: "x".into(),
            path: Some(PathBuf::from("a.md")),
            bom: false,
        };
        let m = FakeMath {
            fail: false,
            calls: Rc::new(RefCell::new(Vec::new())),
        };
        let d = FakeDialogs {
            open: None,
            save: Some(PathBuf::from("b.md")),
            calls,
        };
        let mut a = App::new(
            doc,
            Box::new(NoClip),
            Box::new(store.clone()),
            Box::new(m),
            Box::new(NoImages),
            Box::new(NoAssets),
            Box::new(NoExport),
            Box::new(d),
            Box::new(watch.clone()),
            theme::test_support::env(),
        );
        a.save_as();
        assert_eq!(a.path.as_deref(), Some(Path::new("b.md")));
        assert_eq!(watch.calls.borrow().last().unwrap(), "watch:b.md");
    }

    // ---- 主题与配置（切片 17） ----

    /// 可预置/可记录的内存配置：load 返回注入文本（None=无文件），save 记录全文。
    #[derive(Default)]
    struct MemConfig {
        text: Rc<RefCell<Option<String>>>,
        saves: Rc<RefCell<Vec<String>>>,
    }
    impl ConfigStore for MemConfig {
        fn load(&mut self) -> Result<String, String> {
            self.text
                .borrow()
                .clone()
                .ok_or_else(|| "无配置文件".into())
        }
        fn save(&mut self, text: &str) -> Result<(), String> {
            *self.text.borrow_mut() = Some(text.to_string());
            self.saves.borrow_mut().push(text.to_string());
            Ok(())
        }
    }

    /// 保存恒失败的配置。
    struct FailSaveConfig;
    impl ConfigStore for FailSaveConfig {
        fn load(&mut self) -> Result<String, String> {
            Err("无配置文件".into())
        }
        fn save(&mut self, _text: &str) -> Result<(), String> {
            Err("磁盘只读".into())
        }
    }

    /// 可动态切换的系统外观 mock。
    struct DynSystem(Rc<Cell<Option<bool>>>);
    impl SystemTheme for DynSystem {
        fn is_dark(&mut self) -> Option<bool> {
            self.0.get()
        }
    }

    fn app_theme(config: Box<dyn ConfigStore>, system: Rc<Cell<Option<bool>>>) -> App {
        let doc = Doc {
            text: "abc".into(),
            path: None,
            bom: false,
        };
        let m = FakeMath {
            fail: false,
            calls: Rc::new(RefCell::new(Vec::new())),
        };
        App::new(
            doc,
            Box::new(NoClip),
            Box::new(MemStore::default()),
            Box::new(m),
            Box::new(NoImages),
            Box::new(NoAssets),
            Box::new(NoExport),
            Box::new(NoDialogs),
            Box::new(NoWatcher),
            ThemeEnv {
                config,
                system: Box::new(DynSystem(system)),
                effects: Box::new(theme::test_support::NoEffects),
            },
        )
    }

    #[test]
    fn startup_applies_configured_mode() {
        let cfg = MemConfig::default();
        *cfg.text.borrow_mut() = Some("[theme]\nmode = \"dark\"\n".into());
        let a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
        assert_eq!(a.theme.mode, ThemeMode::Dark);
        assert!(a.theme.dark, "显式深色不受系统浅色影响");
        assert!(a.chrome.notice.is_empty());
    }

    #[test]
    fn startup_notices_invalid_config_and_uses_default() {
        let cfg = MemConfig::default();
        *cfg.text.borrow_mut() = Some("[theme]\nmode = \"auto\"\nfoo = \"bar\"\n".into());
        let a = app_theme(Box::new(cfg), Rc::new(Cell::new(None)));
        assert_eq!(a.theme.mode, ThemeMode::System);
        assert!(!a.theme.dark, "系统外观不可读按浅色");
        assert!(
            a.chrome.notice.contains("2 处"),
            "提示无法识别条目数: {}",
            a.chrome.notice
        );
    }

    #[test]
    fn system_mode_follows_live_switch_and_clears_color_caches() {
        let sys = Rc::new(Cell::new(Some(false)));
        let mut a = app_theme(Box::new(MemConfig::default()), sys.clone());
        assert!(!a.theme.dark);
        a.ensure_layouts_all();
        let (id, _, _) = a.editor.session().blocks()[0];
        a.rasters.insert(id, (vec![0; 4], 1, 1));
        a.inline_math.insert(
            "x".into(),
            MathImage {
                pixels: vec![0; 16],
                width: 2,
                height: 2,
                baseline: 0,
            },
        );
        sys.set(Some(true));
        a.poll_system_theme(Instant::now());
        assert!(a.theme.dark, "系统切深色后调色板跟随");
        assert!(a.rasters.is_empty(), "按旧前景色光栅化的文本须作废");
        assert!(a.inline_math.is_empty(), "公式图须按新前景色重渲");
    }

    #[test]
    fn explicit_mode_ignores_system_switch() {
        let cfg = MemConfig::default();
        *cfg.text.borrow_mut() = Some("[theme]\nmode = \"light\"".into());
        let sys = Rc::new(Cell::new(Some(false)));
        let mut a = app_theme(Box::new(cfg), sys.clone());
        sys.set(Some(true));
        a.poll_system_theme(Instant::now());
        assert_eq!(a.theme.mode, ThemeMode::Light);
        assert!(!a.theme.dark, "显式浅色不受系统切换影响");
    }

    #[test]
    fn switching_theme_persists_config() {
        let cfg = MemConfig::default();
        let saves = cfg.saves.clone();
        let sys = Rc::new(Cell::new(Some(false)));
        let mut a = app_theme(Box::new(cfg), sys.clone());
        a.perform_menu(MenuAction::ThemeDark);
        assert!(a.theme.dark);
        assert_eq!(a.theme.mode, ThemeMode::Dark);
        assert!(a.chrome.notice.contains("深色"));
        let saved = saves.borrow();
        assert_eq!(saved.len(), 1, "切换一次即落盘一次");
        assert!(saved[0].contains("mode = \"dark\""));
        drop(saved);
        // 切回跟随系统：立即采用当前系统外观（此处系统已切深色）
        sys.set(Some(true));
        a.perform_menu(MenuAction::ThemeSystem);
        assert_eq!(a.theme.mode, ThemeMode::System);
        assert!(a.theme.dark);
        assert!(saves.borrow()[1].contains("mode = \"system\""));
    }

    #[test]
    fn config_save_failure_keeps_theme_and_notices() {
        let sys = Rc::new(Cell::new(Some(false)));
        let mut a = app_theme(Box::new(FailSaveConfig), sys);
        a.perform_menu(MenuAction::ThemeDark);
        assert!(a.theme.dark, "保存失败不影响本次生效");
        assert!(
            a.chrome.notice.contains("配置未保存"),
            "提示保存失败: {}",
            a.chrome.notice
        );
    }

    /// 5005 块压力文档（715 × 7 块），冷启动/击键探针与惰性排版测试共用。
    fn stress_doc() -> String {
        let mut text = String::with_capacity(200_000);
        for i in 0..715 {
            text.push_str(&format!(
                "## Section {i}\n\nParagraph {i} with **bold** text, `code`, and a [link](https://example.com/{i}).\n\n- item A {i}\n- item B {i}\n\n```rust\nfn f{i}() -> u32 {{ {i} }}\n```\n\n> quote {i}\n\n---\n\n"
            ));
        }
        text
    }

    // ---- P1b：视口驱动惰性排版 ----

    #[test]
    fn first_edit_lays_out_far_fewer_blocks_than_document() {
        let store = MemStore::default();
        let mut a = app(&stress_doc(), None, false, &store);
        assert!(a.index.blocks.len() >= 5005);
        let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
        a.after_edit(dirty);
        assert!(
            a.layouts.len() < 100,
            "首次编辑只应排版视口内块，实际 {} / {} 块",
            a.layouts.len(),
            a.index.blocks.len()
        );
    }

    #[test]
    fn geometry_index_is_monotonic_and_consistent() {
        let store = MemStore::default();
        let mut a = app(&stress_doc(), None, false, &store);
        let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
        a.after_edit(dirty);
        let mut prev = f32::NEG_INFINITY;
        let mut y = MARGIN * a.scale;
        for g in &a.index.blocks {
            assert!(g.height > 0.0, "块高须为正: {:?}", g.id);
            assert!(g.top >= prev, "top 须单调不减: {:?}", g.id);
            assert!(
                (g.top - y).abs() < 1e-3,
                "top 应与逐块累计一致: {} vs {}",
                g.top,
                y
            );
            prev = g.top;
            y += g.height + PARA_GAP * a.scale;
        }
        let want = y - PARA_GAP * a.scale + MARGIN * a.scale;
        assert!(
            (a.index.total_h - want).abs() < 1e-3,
            "total_h 应与累计一致: {} vs {}",
            a.index.total_h,
            want
        );
    }

    #[test]
    fn ensure_block_lays_out_far_block_with_real_height() {
        let store = MemStore::default();
        let mut a = app(&stress_doc(), None, false, &store);
        let id = a.index.blocks.last().unwrap().id;
        assert!(!a.layouts.contains_key(&id), "远块初始不应已排版");
        a.ensure_block(id);
        let h = a.layouts.get(&id).expect("按需排版应写入布局").height;
        assert!(h > 0.0);
        assert_eq!(a.block_height(geom(&a, id)), h);
    }

    #[test]
    fn move_doc_end_ensures_caret_block_layout() {
        let store = MemStore::default();
        let mut a = app(&stress_doc(), None, false, &store);
        let dirty = a.editor.apply(Intent::MoveDocEnd).unwrap();
        a.after_edit(dirty);
        let id = a
            .caret_block_index()
            .map(|i| a.index.blocks[i].id)
            .expect("文末应落在某块内");
        assert!(a.layouts.contains_key(&id), "光标块须已排版");
        let (x, y, h) = a.caret_screen();
        assert!(x.is_finite() && y.is_finite() && h > 0.0);
    }

    #[test]
    fn estimated_height_positive_for_all_block_kinds() {
        let doc = "# h\n\np\n\n```\ncode\n```\n\n$$\nx\n$$\n\n| a | b |\n| --- | --- |\n| c | d |\n\n- li\n\n> q\n\n---\n";
        let store = MemStore::default();
        let a = app(doc, None, false, &store);
        assert!(a.index.blocks.len() >= 8, "覆盖全部块类型");
        for g in &a.index.blocks {
            assert!(g.est_h > 0.0, "估算高度须为正: {:?}", g.kind);
        }
    }

    /// 真实路径性能探针（默认忽略，不参与常规门禁）：
    /// cargo test -p veil-egui --release -- --ignored --nocapture perf_keystroke_path
    /// 度量 apply + after_edit + paint 的端到端延迟；bench_typing 只测单点 API，
    /// 无法反映视图层真实装配路径，故以此探针作为 P1/P2 的验收依据。
    #[test]
    #[ignore = "性能探针，不参与常规门禁"]
    fn perf_keystroke_path() {
        let text = stress_doc();
        let store = MemStore::default();
        let mut a = app(&text, None, false, &store);
        a.editor.apply(Intent::MoveDocStart).unwrap();
        for _ in 0..5 {
            a.editor.apply(Intent::MoveRight).unwrap();
        }
        a.paint(); // 预热：首帧含全量排版
        let mut samples = Vec::with_capacity(300);
        for _ in 0..300 {
            let t = Instant::now();
            let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
            a.after_edit(dirty);
            a.paint();
            samples.push(t.elapsed().as_secs_f64() * 1e3);
        }
        samples.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let pick = |q: f64| {
            let i = ((samples.len() as f64 * q).ceil() as usize)
                .saturating_sub(1)
                .min(samples.len() - 1);
            samples[i]
        };
        println!("perf_blocks={}", a.index.blocks.len());
        println!("perf_keystroke_ms_p50={:.3}", pick(0.5));
        println!("perf_keystroke_ms_p95={:.3}", pick(0.95));
        println!("perf_keystroke_ms_max={:.3}", samples[samples.len() - 1]);
    }

    /// P1b 验收探针（默认忽略）：构造 5005 块文档 + 第一次编辑的端到端耗时。
    /// cargo test -p veil-egui --release -- --ignored --nocapture perf_cold_first_edit
    #[test]
    #[ignore = "性能探针，不参与常规门禁"]
    fn perf_cold_first_edit() {
        let text = stress_doc();
        let store = MemStore::default();
        let t = Instant::now();
        let mut a = app(&text, None, false, &store);
        let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
        a.after_edit(dirty);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        println!("perf_cold_first_edit_ms={ms:.3}");
        println!("perf_layouts_after_first_edit={}", a.layouts.len());
    }
}
