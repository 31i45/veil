//! winit + softbuffer 薄壳：事件翻译、脏块重建、帧合成与呈现。
//! 策略全在 editor/input；本层只做接线与像素搬运（INSTANCE §7 接缝）。

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::autosave::Autosave;
use crate::chrome::{self, Chrome, Viewport};
use crate::docio::{decode, encode};
use crate::document::{recovery, session};
use crate::editor::Editor;
use crate::geometry::BASE_LINE_HEIGHT;
use crate::image::{AssetStore, ImageData, ImageLoader};
use crate::input::Mods;
use crate::interaction::ZOOM_WHEEL_PX;
use crate::math::MathRenderer;
use crate::print::{Print, PrintState};
use crate::theme::{
    self, Config, ConfigStore, SystemTheme, Theme, ThemeEnv, ThemeMode, WindowEffects,
};
use crate::view::{
    quick_open, search, Cache, Frame, LayoutEngine, Outline, QuickOpen, Revs, Search,
};

// 各域 impl App 的注册由 lib.rs 的模块声明承担（同 crate 内 impl 自动链接）。

use crate::document::block_index::BlockIndex;
use crate::document::Persist;

/// 外部文件监听轮询间隔（切片 16）：事件在实现侧入队，主线程按此节奏非阻塞取走。
/// 仅在有路径时设置唤醒，草稿空转不产生周期性唤醒。
const WATCH_POLL: Duration = Duration::from_millis(300);
/// 系统外观轮询间隔（切片 17）：仅在「跟随系统」时启用；注册表读取开销可忽略。
const THEME_POLL: Duration = Duration::from_secs(1);

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
    /// 列出目录内的普通文件（快速打开，阶段 1）：只做 IO，不排序/不按扩展名过滤
    /// （候选语义在视图层）。默认空实现：测试与不关心文件树的实现无需改动。
    fn list_files(&mut self, _dir: &Path) -> Result<Vec<PathBuf>, String> {
        Ok(Vec::new())
    }
}

/// 文件对话框抽象（切片 15）：实现由产品层（veil）注入，本层不依赖具体 GUI 框架。
/// 取消（用户关闭对话框）返回 None；路径选择成功后由 shell 负责读写与状态切换。
pub trait FileDialog {
    fn open_markdown(&mut self) -> Option<PathBuf>;
    fn save_markdown(&mut self, suggested: &str) -> Option<PathBuf>;
    /// 选择图片文件（2f 插入图片入口）。默认 None：不关心图片的实现无需改动。
    fn open_image(&mut self) -> Option<PathBuf> {
        None
    }
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

/// 崩溃恢复草稿存储（0b）：单槽字节读写，实现由产品层注入（应用数据目录单文件）。
/// `save` 须原子（tmp+rename）：恢复文件自身半截比没有更糟。
/// `load` 在无恢复文件时返回 Err（正常状态）；`clear` 在文件不存在时视为成功。
pub trait RecoveryStore {
    fn load(&mut self) -> Result<Vec<u8>, String>;
    fn save(&mut self, data: &[u8]) -> Result<(), String>;
    fn clear(&mut self) -> Result<(), String>;
}

/// 会话状态存储（0b）：最近文件小文本读写（形状同 ConfigStore，语义独立）。
/// 实现由产品层注入；`load` 在文件不存在时返回 Err（首次运行属正常）。
pub trait SessionStore {
    fn load(&mut self) -> Result<String, String>;
    fn save(&mut self, text: &str) -> Result<(), String>;
}

/// 持久化能力打包（0b）：崩溃恢复 + 会话/最近文件。与 ThemeEnv 同模式，
/// 避免 run/App::new 参数继续膨胀。
pub struct PersistEnv {
    pub recovery: Box<dyn RecoveryStore>,
    pub session: Box<dyn SessionStore>,
}

/// 待编辑文档：产品层从 CLI 参数或默认样例构造。
pub struct Doc {
    pub text: String,
    /// None = 无关联文件的草稿（Ctrl+S 拒绝，另存为待 chrome UI）。
    pub path: Option<PathBuf>,
    /// 读入时是否剥除了 BOM（保存时还原，保往返字节一致）。
    pub bom: bool,
    /// 启动恢复模式（0b）：无 CLI 参数时置 true——允许从崩溃恢复草稿/会话
    /// 最近文件重建文档；CLI 显式打开时置 false（恢复文件保留不动）。
    pub resume: bool,
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
    persist_env: PersistEnv,
    print: Box<dyn Print>,
) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("创建事件循环失败: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(
        doc,
        clipboard,
        store,
        math,
        images,
        assets,
        exporter,
        dialogs,
        watcher,
        theme_env,
        persist_env,
        print,
    );
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("事件循环错误: {e}"))
}

/// 文档视图 UI 组件状态（ADR-009 S6 C6）：chrome/大纲/查找栏/写作模式。
/// 四者是 UI 交互态，无数据互借，合并减少顶层字段。
pub(crate) struct UIState {
    /// 自绘 chrome（切片 15）：菜单/状态栏/滚动条交互态与标签缓存。
    pub(crate) chrome: Chrome,
    /// 大纲面板（P3-2）：标题条目、开关/滚动/悬停。
    pub(crate) outline: Outline,
    /// 文档内查找栏（P3-3）：查询、命中缓存与当前下标。
    pub(crate) search: Search,
    /// 快速打开浮层（阶段 1）：最近文件 + 当前目录候选。
    pub(crate) quick: QuickOpen,
    /// 右键上下文菜单（2f）：选区/链接/块三类上下文的瞬态菜单。
    pub(crate) ctx: crate::view::ctx_menu::CtxMenu,
    /// 写作模式与全文统计（阶段 1）。
    pub(crate) writing: WritingState,
}

/// 写作模式与全文统计（阶段 1）：纯视图开关 + 状态栏派生数字。
#[derive(Default)]
pub(crate) struct WritingState {
    /// 专注模式：非光标所在块整体压暗，仅视觉，不改真值。
    pub(crate) focus: bool,
    /// 打字机模式：光标行固定在视口上部锚点。
    pub(crate) typewriter: bool,
    /// 全文字数（中文口径），随 `refresh_index` 重算。
    pub(crate) words: usize,
}

/// 外部输入状态（ADR-009 S6 C4）：全部从系统/用户注入的输入信号。
/// 字段间无复杂借用冲突，主题代次（revs.theme）与 theme/system_dark 的耦合
/// 由 set_theme_mode / refresh_theme_visuals 统一处理，不跨子结构散落。
pub(crate) struct InputState {
    pub(crate) revs: Revs,
    pub(crate) mouse: PhysicalPosition<f64>,
    pub(crate) mods: Mods,
    /// 有效缩放 = dpi_scale × user_zoom（一切排版/命中/光栅的唯一基准）。
    /// 打印期间被页帧 DPI 整体覆写（PrintState 保存/恢复）。
    pub(crate) scale: f32,
    /// winit 报告的 DPI 缩放原值；用户缩放不改变它，二者乘积写入 scale。
    pub(crate) dpi_scale: f32,
    /// 用户缩放因子（3c-2）：Ctrl+滚轮/捏合、Ctrl+=/-/0 与视图菜单；0.1 步进。
    /// 纸张打印不随它变（打印按页帧 DPI 覆写 scale）。
    pub(crate) user_zoom: f32,
    /// 高精度滚轮/触控板捏合的缩放累积量（像素）：每满一档阈值触发一次 0.1 步进。
    pub(crate) zoom_accum: f32,
    pub(crate) theme: Theme,
    pub(crate) system_dark: bool,
    pub(crate) last_theme_check: Option<Instant>,
    /// 拖拽悬停文件（阶段 1）：仅驱动落点提示叠层；松开/离开窗口即清空。
    pub(crate) drop_file: Option<PathBuf>,
}

impl InputState {
    /// DPI 输入（窗口创建 / ScaleFactorChanged）：有效缩放随用户因子重算。
    pub(crate) fn set_dpi(&mut self, dpi: f32) {
        self.dpi_scale = dpi;
        self.scale = dpi * self.user_zoom;
    }
}

/// 持久化策略状态（ADR-009 S6 C3）：外部变更事实 + 自动保存调度 + 恢复/会话。
/// 两者总被同一路径访问（保存/外部变更轮询/自动保存到期检查），合并无借用冲突。
pub(crate) struct PersistState {
    /// 持久化事实（外部变更/冲突备份/自动保存挂起/活动时刻）：单一结构承载，
    /// 提示优先级只在 `Persist::status` 实现一次。
    pub(crate) persist: Persist,
    pub(crate) autosave: Autosave,
    /// 已写入恢复文件的快照哈希（0b）：内容未变不重复落盘；None = 磁盘无恢复副本。
    pub(crate) recovery_hash: Option<u64>,
    /// 恢复写盘上次失败时刻（0b）：退避重试的调度输入；成功/干净时归零。
    pub(crate) recovery_failed_at: Option<Instant>,
    /// 最近文件（0b）：首元素即会话恢复目标；打开/另存为成功时更新并落盘。
    pub(crate) recent: Vec<PathBuf>,
}

/// 文档真值状态（ADR-009 S6 C2）：路径/编码标记/基线字节。
pub(crate) struct DocState {
    /// 关联文件路径（None = 草稿）；标题由此派生。
    pub(crate) path: Option<PathBuf>,
    /// 读入时的 BOM 标记，保存时还原。
    pub(crate) bom: bool,
    /// 上次保存的磁盘字节（或新建时的初始字节）：冲突检测的基线。
    pub(crate) baseline: Vec<u8>,
}

/// 瞬态窗口/呈现状态（ADR-009 S6 C1）：6 个字段无交叉借用风险，单独子结构。
pub(crate) struct WindowState {
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    /// 跨帧复用的像素缓冲（P2）：尺寸不变时零分配，绘制结束整块拷入 surface。
    pub(crate) frame: Option<Frame>,
    pub(crate) scroll_y: f32,
    /// 垂直移动的目标列（goal column），上下移动时保持。
    pub(crate) goal_x: Option<f32>,
    /// 左键按住中：CursorMoved 转化为拖选（扩选）。
    pub(crate) dragging: bool,
    /// 最近一次正文左键：(时刻, 位置, 连续点击数 1..=3)，双击/三击判定依据。
    pub(crate) last_click: Option<(Instant, PhysicalPosition<f64>, u8)>,
    /// 打印会话态（3b）：Some 期间 win_size/viewport/绘制全部切换到打印页帧语义。
    pub(crate) print: Option<PrintState>,
}
impl Default for WindowState {
    fn default() -> Self {
        Self {
            window: None,
            surface: None,
            frame: None,
            scroll_y: 0.0,
            goal_x: None,
            dragging: false,
            last_click: None,
            print: None,
        }
    }
}

/// 应用编排器：三域（document 真值 / view 派生 / 交互事件）的装配点。
/// 字段 pub(crate) 供各域文件的 `impl App` 直接读写——同一 crate 内的模块即协作者，
/// 行为约束由各领域方法的文档与测试承载，不经访问器转发。
///
/// 状态五分类登记（ADR-009 决策一）：
/// - 真值 Truth：doc 子结构（path / bom / baseline）
/// - 输入 Input：scale / system_dark / last_theme_check / mouse / mods
/// - 派生 Derived：index / cache / outline / search
/// - 端口 Port：聚合为 ports 字段
/// - 瞬态 Transient：WindowState 子结构（scroll_y / goal_x / dragging / window / surface / frame）
pub(crate) struct App {
    pub(crate) doc: DocState,
    pub(crate) window_state: WindowState,
    pub(crate) editor: Editor,
    pub(crate) engine: LayoutEngine,
    /// 块几何索引（P1a）：全量块枚举的唯一缓存来源，替代每次读取时的 `blocks()`。
    pub(crate) index: BlockIndex,
    /// 派生缓存（布局/光栅/公式/图片/表格）与因果键查询。
    pub(crate) cache: Cache,
    pub(crate) input_state: InputState,
    /// 11 个外部注入的端口（Clipboard / FileStore / MathRenderer / ImageLoader /
    /// AssetStore / Exporter / FileDialog / FileWatcher / ConfigStore / SystemTheme /
    /// WindowEffects）聚合为单结构（ADR-009 S5）。
    pub(crate) ports: Ports,
    pub(crate) persist_state: PersistState,
    pub(crate) ui_state: UIState,
}

/// 14 个平台端口聚合结构：能力集合，不携带状态语义（ADR-009 决策六）。
/// 全部为 Box<dyn Trait>，由产品层注入（veil/main.rs 各 impl 对应 trait）。
pub(crate) struct Ports {
    pub(crate) clipboard: Box<dyn Clipboard>,
    pub(crate) store: Box<dyn FileStore>,
    pub(crate) math: Box<dyn MathRenderer>,
    pub(crate) images: Box<dyn ImageLoader>,
    pub(crate) assets: Box<dyn AssetStore>,
    pub(crate) exporter: Box<dyn Exporter>,
    pub(crate) dialogs: Box<dyn FileDialog>,
    pub(crate) watcher: Box<dyn FileWatcher>,
    pub(crate) config: Box<dyn ConfigStore>,
    pub(crate) system: Box<dyn SystemTheme>,
    pub(crate) effects: Box<dyn WindowEffects>,
    pub(crate) recovery: Box<dyn RecoveryStore>,
    pub(crate) session: Box<dyn SessionStore>,
    pub(crate) print: Box<dyn Print>,
}

/// 启动文档决议（0b）：resume 模式下按「崩溃恢复 > 会话恢复 > 传入文档」重建。
/// 非 resume（CLI 显式打开）只加载最近列表（菜单需要），恢复文件保留不动。
fn startup_doc(ports: &mut Ports, doc: Doc) -> StartupDoc {
    let recent = ports
        .session
        .load()
        .map(|t| session::parse_session(&t))
        .unwrap_or_default();
    let mut out = StartupDoc {
        doc,
        baseline: None,
        recovered: false,
        missing: false,
        notice: String::new(),
        recent,
    };
    if !out.doc.resume {
        return out;
    }
    // 1. 崩溃恢复优先：恢复文件 = 最后一个未保存状态
    if let Ok(bytes) = ports.recovery.load() {
        match recovery::decode_recovery(&bytes) {
            Ok(r) => {
                let disk = r.path.as_ref().and_then(|p| ports.store.load(p).ok());
                match disk {
                    // 恢复内容 == 磁盘内容：上次保存成功但清理残留失败——干净打开并自清
                    Some(d) if d == encode(&r.text, r.bom) => {
                        let _ = ports.recovery.clear();
                        out.doc.text = r.text;
                        out.doc.path = r.path;
                        out.doc.bom = r.bom;
                        out.baseline = Some(d);
                    }
                    _ => {
                        out.baseline = disk; // 磁盘读不到时 None → 以内容编码兜底
                        out.missing = r.path.is_some() && out.baseline.is_none();
                        out.doc.text = r.text;
                        out.doc.path = r.path;
                        out.doc.bom = r.bom;
                        out.recovered = true;
                        out.notice = "已从崩溃恢复未保存的内容；确认后请保存".into();
                    }
                }
            }
            Err(e) => {
                // 损坏的恢复文件：忽略并清除（否则每次启动都报错；内容已在磁盘基线之外不可信）
                eprintln!("崩溃恢复文件损坏，已忽略: {e}");
                let _ = ports.recovery.clear();
            }
        }
    }
    // 2. 会话恢复：恢复未生效且传入的是无路径草稿（默认样例）→ 打开最近文件
    if !out.recovered && out.doc.path.is_none() {
        if let Some(last) = out.recent.first().cloned() {
            match ports
                .store
                .load(&last)
                .map_err(|e| e.to_string())
                .and_then(|b| decode(&b).map(|(t, bom)| (t, bom, b)))
            {
                Ok((text, bom, bytes)) => {
                    out.doc.text = text;
                    out.doc.path = Some(last);
                    out.doc.bom = bom;
                    out.baseline = Some(bytes);
                }
                Err(e) => eprintln!("会话恢复失败（打开默认文档） {}: {e}", last.display()),
            }
        }
    }
    out
}

/// 启动决议结果：文档 + 基线/脏态/提示等副产物。
struct StartupDoc {
    doc: Doc,
    /// 磁盘基线（恢复/会话场景）；None = 以 text 编码为基线（干净打开）。
    baseline: Option<Vec<u8>>,
    /// 崩溃恢复的脏文档：打开即标脏。
    recovered: bool,
    /// 恢复目标磁盘缺失/不可读：置 missing 提示态。
    missing: bool,
    notice: String,
    recent: Vec<PathBuf>,
}

impl App {
    pub(crate) fn new(
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
        persist_env: PersistEnv,
        print: Box<dyn Print>,
    ) -> Self {
        let ThemeEnv {
            mut config,
            mut system,
            effects,
        } = theme_env;
        let PersistEnv { recovery, session } = persist_env;
        // 系统外观读不到（非 Windows/注册表异常）按浅色处理：不影响其它功能
        let system_dark = system.is_dark().unwrap_or(false);
        let (mode, config_notice) = match config.load() {
            Ok(text) => match theme::decode_config(text.as_bytes()) {
                Ok(p) if p.ignored == 0 => (p.config.mode, String::new()),
                Ok(p) => (
                    p.config.mode,
                    format!("配置有 {} 处无法识别，已忽略", p.ignored),
                ),
                Err(e) => (
                    ThemeMode::System,
                    format!("配置文件无效，已用默认主题: {e}"),
                ),
            },
            // 文件不存在属正常状态（首次运行）：静默用默认配置
            Err(_) => (ThemeMode::System, String::new()),
        };
        let theme = Theme::of(mode, system_dark);
        let mut ports = Ports {
            clipboard,
            store,
            math,
            images,
            assets,
            exporter,
            dialogs,
            watcher,
            config,
            system,
            effects,
            recovery,
            session,
            print,
        };
        let start = startup_doc(&mut ports, doc);
        let mut editor = Editor::new(&start.doc.text);
        if start.recovered {
            editor.set_dirty(true); // 恢复内容相对磁盘基线是未保存修改
        }
        let mut persist = Persist::default();
        if start.missing {
            persist.set_missing(true);
        }
        let mut app = App {
            doc: DocState {
                path: start.doc.path,
                bom: start.doc.bom,
                baseline: start
                    .baseline
                    .unwrap_or_else(|| encode(&start.doc.text, start.doc.bom)),
            },
            window_state: WindowState::default(),
            editor,
            engine: LayoutEngine::new(),
            index: BlockIndex::default(),
            cache: Cache::default(),
            input_state: InputState {
                revs: Revs::default(),
                mouse: PhysicalPosition::new(0.0, 0.0),
                mods: Mods::default(),
                scale: 1.0,
                dpi_scale: 1.0,
                user_zoom: 1.0,
                zoom_accum: 0.0,
                theme,
                system_dark,
                last_theme_check: None,
                drop_file: None,
            },
            ports,
            persist_state: PersistState {
                persist,
                autosave: Autosave::default(),
                recovery_hash: None,
                recovery_failed_at: None,
                recent: start.recent,
            },
            ui_state: UIState {
                chrome: Chrome::default(),
                outline: Outline::default(),
                search: Search::default(),
                quick: QuickOpen::default(),
                ctx: crate::view::ctx_menu::CtxMenu::default(),
                writing: WritingState::default(),
            },
        };
        app.ui_state.chrome.notice = if start.notice.is_empty() {
            config_notice
        } else {
            start.notice // 恢复提示优先于配置提示
        };
        app.engine.set_syntax_colors(app.input_state.theme.syntax);
        app.refresh_index();
        app.recompute_geometry();
        // 启动即登记最近文件（CLI 打开/会话恢复/崩溃恢复路径统一入口）
        if let Some(p) = app.doc.path.clone() {
            app.note_opened(p);
        }
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
            let Some(win) = &self.window_state.window else {
                return;
            };
            let Ok(handle) = win.window_handle() else {
                return;
            };
            if let RawWindowHandle::Win32(p) = handle.as_raw() {
                self.ports.effects.apply(
                    NativeWindow { hwnd: p.hwnd.get() },
                    self.input_state.theme.dark,
                );
            }
        }
        #[cfg(not(windows))]
        let _ = &self.ports.effects;
    }

    /// 主题切换后刷新按颜色生成的全部缓存：主题代次递增使全部颜色相关派生缓存
    /// 按因果键失效（布局含语法高亮色，自本次起同步重排）；窗口外观同步更新。
    /// pub(crate) 供性质测试（S6 阶段 B）构造主题切换序列。
    pub(crate) fn refresh_theme_visuals(&mut self) {
        self.engine.set_syntax_colors(self.input_state.theme.syntax);
        self.input_state.revs.theme += 1;
        self.ui_state.chrome.clear_label_cache();
        // 代次递增后按新调色板重排/重渲：只重做可见块，几何按新高度重算
        self.refresh_index();
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        self.apply_window_effects();
        self.request_redraw();
    }

    /// 切换主题模式并持久化。切到「跟随系统」时立即重读一次系统外观，避免沿用陈旧值。
    pub(crate) fn set_theme_mode(&mut self, mode: ThemeMode) {
        if mode == ThemeMode::System {
            if let Some(dark) = self.ports.system.is_dark() {
                self.input_state.system_dark = dark;
            }
        }
        self.input_state.theme = Theme::of(mode, self.input_state.system_dark);
        self.input_state.last_theme_check = None;
        self.refresh_theme_visuals();
        // 先给成功提示再落盘：保存失败时错误提示更重要，覆盖为「配置未保存」
        self.ui_state.chrome.notice = format!("主题：{}", mode.label());
        self.save_config();
        self.request_redraw();
    }

    /// 配置落盘（极小文本、全量重写；实现侧保证原子与父目录创建）。
    fn save_config(&mut self) {
        let text = Config {
            mode: self.input_state.theme.mode,
        }
        .to_toml();
        if let Err(e) = self.ports.config.save(&text) {
            self.ui_state.chrome.notice = format!("配置未保存: {e}");
            eprintln!("配置保存失败: {e}");
        }
    }

    /// 跟随系统时轮询系统外观（限频）；外观切换则重建调色板并刷新颜色缓存。
    pub(crate) fn poll_system_theme(&mut self, now: Instant) {
        if self.input_state.theme.mode != ThemeMode::System {
            return;
        }
        if self
            .input_state
            .last_theme_check
            .is_some_and(|t| now.duration_since(t) < THEME_POLL)
        {
            return;
        }
        self.input_state.last_theme_check = Some(now);
        let Some(dark) = self.ports.system.is_dark() else {
            return;
        };
        if dark != self.input_state.system_dark {
            self.input_state.system_dark = dark;
            self.input_state.theme = Theme::of(ThemeMode::System, dark);
            self.refresh_theme_visuals();
        }
    }

    pub(crate) fn win_size(&self) -> (u32, u32) {
        // 打印中：「窗口」即打印页帧（无菜单栏/状态栏），render() 产出整页帧
        if let Some(p) = &self.window_state.print {
            return (p.w.max(1), p.h.max(1));
        }
        self.window_state
            .window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width.max(1), s.height.max(1))
            })
            .unwrap_or((800, 600))
    }

    pub(crate) fn viewport(&self) -> Viewport {
        let (w, h) = self.win_size();
        // 打印中：内容视口 = 整帧（chrome 不绘制，无菜单栏/状态栏扣减）
        if self.window_state.print.is_some() {
            return Viewport {
                x: 0.0,
                y: 0.0,
                w: w as f32,
                h: h as f32,
            };
        }
        chrome::viewport(w, h, self.input_state.scale)
    }

    pub(crate) fn request_redraw(&self) {
        if let Some(w) = &self.window_state.window {
            w.request_redraw();
        }
    }

    pub(crate) fn set_ime_area(&mut self) {
        let Some(w) = self.window_state.window.clone() else {
            return; // 无窗口（测试）：不触发按需排版
        };
        let (x, y, h) = if let Some(l) = self.quick_rect() {
            // 快速打开：候选框贴过滤串末端
            let pad = quick_open::PAD * self.input_state.scale;
            let qw = quick_open::text_width(&self.ui_state.quick.query, self.input_state.scale)
                .min((l.input.w - 2.0 * pad).max(0.0));
            (l.input.x + pad + qw, l.input.y + l.input.h * 0.5, l.input.h)
        } else {
            match self.search_rect() {
                // 查找栏：候选框贴当前焦点框文本末端（文档光标被栏覆盖，不参与定位）
                Some(l)
                    if self.ui_state.search.replace_open
                        && self.ui_state.search.focus == search::Field::Replace =>
                {
                    let r = search::replace_layout(&l, self.input_state.scale);
                    let w =
                        search::text_width(&self.ui_state.search.replace, self.input_state.scale)
                            .min(r.text.w);
                    (r.text.x + w, r.row.y + r.row.h * 0.5, r.row.h)
                }
                Some(l) => {
                    let qw =
                        search::text_width(&self.ui_state.search.query, self.input_state.scale)
                            .min(l.text.w);
                    (l.text.x + qw, l.bar.y + l.bar.h * 0.5, l.bar.h)
                }
                None => self.caret_screen(),
            }
        };
        w.set_ime_cursor_area(
            PhysicalPosition::new(x as f64, y as f64),
            PhysicalSize::new(2.0, h as f64),
        );
    }
}

impl App {
    /// 时间输入的唯一入口：外部轮询、系统外观、自动保存、恢复写盘都只读调用方注入的
    /// `now`（ADR-009 决策三「时间作为输入」）。顺序不可交换：外部修改须先挂起自动保存。
    fn tick(&mut self, now: Instant) {
        self.poll_external();
        self.poll_system_theme(now);
        self.maybe_autosave(now);
        self.maybe_recovery_write(now);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window_state.window.is_some() {
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
        self.input_state.set_dpi(window.scale_factor() as f32);
        let context = match Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("创建呈现上下文失败: {e}");
                event_loop.exit();
                return;
            }
        };
        self.window_state.surface = Surface::new(&context, window.clone()).ok();
        self.set_ime_area();
        self.window_state.window = Some(window);
        self.apply_window_effects();
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(s), Some(w), Some(h)) = (
                    self.window_state.surface.as_mut(),
                    NonZeroU32::new(size.width),
                    NonZeroU32::new(size.height),
                ) {
                    let _ = s.resize(w, h);
                }
                // 宽度变化：换行/列宽依赖版心宽；键中的版心宽量化值随之变化，
                // 布局/光栅/表格自然失配，无需显式清缓存
                self.refresh_index();
                self.ensure_layouts_visible();
                self.ensure_tables_visible();
                self.recompute_geometry();
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(w) = &self.window_state.window {
                    self.input_state.set_dpi(w.scale_factor() as f32);
                }
                // DPI 变化：键中的 scale 量化值变化使全部按 scale 生成的缓存失配；
                // chrome 标签按旧 scale 光栅化，仍需显式作废
                self.ui_state.chrome.clear_label_cache();
                // scale 变了，几何累计均按新 scale：索引重建、可见块按新 DPI 重排
                self.refresh_index();
                self.ensure_layouts_visible();
                self.ensure_math_visible();
                self.ensure_tables_visible();
                self.recompute_geometry();
                self.request_redraw();
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => self.ime_preedit(text, cursor),
            WindowEvent::Ime(Ime::Commit(text)) => self.ime_commit(text),
            WindowEvent::Ime(Ime::Disabled) => self.ime_disabled(),
            // 拖放（阶段 1）：图片插入光标处；其余文件按文档打开
            WindowEvent::DroppedFile(path) => self.drop_file(path),
            WindowEvent::HoveredFile(path) => self.hover_file(Some(path)),
            WindowEvent::HoveredFileCancelled => self.hover_file(None),
            WindowEvent::KeyboardInput { event, .. } => self.on_key(&event),
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.input_state.mods = Mods {
                    ctrl: s.control_key(),
                    shift: s.shift_key(),
                };
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input_state.mouse = position;
                if let Some(off) = self.ui_state.chrome.scroll_drag {
                    // 拖动滚动条：按滑块顶边换算滚动量
                    let vp = self.viewport();
                    let doc_h = self.doc_height();
                    self.window_state.scroll_y = chrome::scroll_from_thumb(
                        vp,
                        doc_h,
                        self.input_state.mouse.y as f32 - off,
                        self.input_state.scale,
                    );
                    self.set_ime_area();
                    self.request_redraw();
                } else if self.window_state.dragging {
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
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                self.on_right_click();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.window_state.dragging = false;
                self.ui_state.chrome.scroll_drag = None;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.input_state.mods.ctrl {
                    // Ctrl+滚轮 / 触控板捏合（Windows 捏合上报为带 Ctrl 的滚轮事件）：
                    // 用户缩放（3c-2），不滚动。LineDelta 一格折一档像素阈值。
                    let px = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y * ZOOM_WHEEL_PX,
                        MouseScrollDelta::PixelDelta(p) => p.y as f32,
                    };
                    self.zoom_by(px);
                    return;
                }
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => {
                        y * 3.0 * BASE_LINE_HEIGHT * self.input_state.scale
                    }
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                self.on_wheel(d);
            }
            WindowEvent::RedrawRequested => self.paint(),
            // 窗口重新获得焦点：立即校验系统外观（限频轮询最多滞后 1s，此处消除等待）
            WindowEvent::Focused(true) => {
                self.input_state.last_theme_check = None;
                self.poll_system_theme(Instant::now());
            }
            _ => {}
        }
    }

    /// 空闲回调：取外部变更事件、校验系统外观、尝试自动保存与恢复写盘，并把下次唤醒定在
    /// 「恢复/自动保存静默期终点」「监听轮询」「系统外观轮询」的较早者
    /// （干净草稿且显式主题下完全休眠）。
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.tick(now);
        // 恢复写盘（0b）：脏文档（含无路径草稿/冲突挂起）都按静默期调度——
        // 它是比自动保存更宽的最后防线
        let dirty_editing = self.editor.is_dirty() && self.editor.preedit().is_none();
        let mut next = if dirty_editing {
            self.persist_state
                .autosave
                .deadline(self.persist_state.persist.activity())
        } else {
            None
        };
        // 恢复写失败后的退避重试
        if let Some(t) = self.persist_state.recovery_failed_at {
            let retry_at = t + crate::document::doc_state::RECOVERY_RETRY;
            next = Some(next.map_or(retry_at, |x| x.min(retry_at)));
        }
        // 系统外观只可能影响「跟随系统」模式；显式主题/无窗口时不唤醒
        if self.input_state.theme.mode == ThemeMode::System && self.window_state.window.is_some() {
            let theme_at = now + THEME_POLL;
            next = Some(next.map_or(theme_at, |t| t.min(theme_at)));
        }
        if self.doc.path.is_some() {
            let watch_at = now + WATCH_POLL;
            next = Some(next.map_or(watch_at, |t| t.min(watch_at)));
        }
        event_loop.set_control_flow(match next {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}
