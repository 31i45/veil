//! winit + softbuffer 薄壳：事件翻译、脏块重建、帧合成与呈现。
//! 策略全在 editor/input；本层只做接线与像素搬运（INSTANCE §7 接缝）。

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Icon, Window, WindowId};

use crate::autosave::Autosave;
use crate::chrome::{self, Chrome, NoticeKind, Viewport};
use crate::docio::{decode, encode};
use crate::document::{history, recovery, session};
use crate::editor::Editor;
use crate::geometry::{clamp_window_geom, ScreenRect, BASE_LINE_HEIGHT};
use crate::i18n::{clip_error, t, tf, Key};
use crate::image::{AssetStore, ImageData, ImageLoader};
use crate::input::{Key as InputKey, Mods};
use crate::interaction::ZOOM_WHEEL_PX;
use crate::math::MathRenderer;
use crate::mermaid_renderer::MermaidRenderer;
use crate::mode::Mode;
use crate::print::{Print, PrintState};
use crate::theme::{
    self, Config, ConfigStore, ExportStyle, SystemTheme, Theme, ThemeEnv, ThemeMode, WindowEffects,
    WindowGeom,
};
use crate::view::{
    quick_open, search, source_layout::SourceLayout, Cache, Frame, History, LayoutEngine, Outline,
    QuickOpen, Revs, Search,
};

// 各域 impl App 的注册由 lib.rs 的模块声明承担（同 crate 内 impl 自动链接）。

use crate::document::block_index::BlockIndex;
use crate::document::Persist;
use veil_core::{BlockContainer, BlockId, BlockKind, Range};

/// 外部文件监听轮询间隔（切片 16）：事件在实现侧入队，主线程按此节奏非阻塞取走。
/// 仅在有路径时设置唤醒，草稿空转不产生周期性唤醒。
const WATCH_POLL: Duration = Duration::from_millis(300);
/// 系统外观轮询间隔（切片 17）：仅在「跟随系统」时启用；注册表读取开销可忽略。
const THEME_POLL: Duration = Duration::from_secs(1);
/// 远程图片结果轮询间隔：仅在有 pending 请求时启用（干净文档不产生周期唤醒）。
const REMOTE_POLL: Duration = Duration::from_millis(100);
/// 光标闪烁半周期（600 ms 可见 / 600 ms 不可见）——标准文本编辑器节拍。
const CURSOR_BLINK: Duration = Duration::from_millis(600);

/// 状态栏提示的短暂存活时长（成功/普通提示）；新提示写入后重新计时。
/// 错误类通知（`NoticeKind::Error`）不参与该超时，保留到被新通知替换或用户处理。
const NOTICE_TTL: Duration = Duration::from_secs(6);

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
    /// 文件列表读取（资源管理器 Ctrl+C 复制的文件）：默认空。
    /// 系统剪贴板的文件复制同时会附带路径文本，故粘贴时须先于文本分支判定，
    /// 否则粘出来的是路径字符串而不是图片。
    fn get_files(&mut self) -> Vec<PathBuf> {
        Vec::new()
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

/// 本地时钟抽象（信任显性化）：状态栏「已保存 HH:MM」的时间来源。
/// 由产品层注入（Windows 走手写 Win32 `GetLocalTime`），本层不依赖任何系统时间 API，
/// 测试注入固定时钟即可断言文案。返回值须为两位补零的 `HH:MM`。
pub trait Clock {
    fn local_hhmm(&self) -> String;
    /// 快照时间戳的本地展示文本（`YYYY-MM-DD HH:MM:SS`）。默认按 UTC 派生，
    /// 产品层可覆写为本地时区；测试注入固定值即可断言文案。
    fn local_datetime(&self, ts_ms: u64) -> String {
        format_unix_ms_utc(ts_ms)
    }
}

/// Unix 毫秒 → `YYYY-MM-DD HH:MM:SS`（UTC）。快照列表的兜底格式：
/// 不依赖系统时区，保证 headless 测试与跨平台文案稳定。
pub fn format_unix_ms_utc(ts_ms: u64) -> String {
    let secs = (ts_ms / 1000) as i64;
    let sod = secs.rem_euclid(86_400);
    let days = secs.div_euclid(86_400);
    // Howard Hinnant civil_from_days：1970-01-01 起的天数 → 公历日期
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// 文件对话框抽象（切片 15）：实现由产品层（veil）注入，本层不依赖具体 GUI 框架。
/// 取消（用户关闭对话框）返回 None；路径选择成功后由 shell 负责读写与状态切换。
pub trait FileDialog {
    fn open_markdown(&mut self) -> Option<PathBuf>;
    /// 另存为对话框。`suggested` 可为完整路径（含目录，用于定位到文档所在目录），
    /// 也可为裸文件名（草稿首次保存）：平台层负责补上用户「文档」目录。
    fn save_markdown(&mut self, suggested: &str) -> Option<PathBuf>;
    /// 选择图片文件（2f 插入图片入口）。默认 None：不关心图片的实现无需改动。
    fn open_image(&mut self) -> Option<PathBuf> {
        None
    }
    /// 导出目标对话框：每次导出都调用（导出是显式副作用，不复用源文件目录）。
    /// `ext` 为扩展名（`html` / `pdf`），决定标题与过滤器；`suggested` 语义同
    /// `save_markdown`。默认 None：不实现导出的平台视为取消，不产生输出。
    fn save_export(&mut self, _suggested: &str, _ext: &str) -> Option<PathBuf> {
        None
    }
    /// 导出纯文本（诊断信息）保存对话框：默认 None（视为取消）。
    /// `ext` 为扩展名（如 `txt`），决定过滤器与默认后缀。
    fn save_text(&mut self, _suggested: &str, _ext: &str) -> Option<PathBuf> {
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
/// `style` 是当前主题的不可变快照（值传递）：导出器无需感知 Theme/App，只按色值出样式。
pub trait Exporter {
    /// `target` 为调用方已选定的输出路径（草稿态经对话框选定）；None 表示沿用
    /// 源文件同目录同名规则（`source_path.with_extension("html")`）。
    /// `mermaids` 是已光栅化的 Mermaid 渲染结果（BlockId → MermaidImage）。
    /// None 或空时，Mermaid 块降级为源码代码围栏。
    fn export_html(
        &mut self,
        session: &veil_core::Session,
        style: ExportStyle,
        source_path: Option<&Path>,
        target: Option<&Path>,
        mermaids: Option<&std::collections::HashMap<veil_core::BlockId, crate::MermaidImage>>,
    ) -> Result<PathBuf, String>;
}

/// 用系统默认程序打开链接：实现由产品层（veil）注入（同 Clipboard 模式）。
/// `Err(String)` 是可直接展示给用户的失败原因（不崩溃、不吞错）。
pub trait UrlOpener {
    fn open_url(&mut self, url: &str) -> Result<(), String>;
}

/// 链接协议白名单：只放行 http/https/mailto/tel（大小写不敏感，允许前导空白）。
/// 为什么：链接文本来自文档内容（不可信输入），相对路径、`file:`、`javascript:` 等
/// 一旦交给系统 shell 就是任意本地程序/脚本执行入口，故一律拒绝。
/// scheme 提取与导出/剪贴板共用 `veil_core::link_scheme`（含实体/百分号/控制字符混淆还原）。
/// 应用内锚点（`#...`）不归本端口处理（由锚点跳转任务负责），调用方须先自行忽略。
pub fn url_is_openable(url: &str) -> bool {
    matches!(
        veil_core::link_scheme(url).as_deref(),
        Some("http" | "https" | "mailto" | "tel")
    )
}

/// 链接目标归一化：GFM 裸链接补全为可直接使用的目标——
/// `www.example.com` → `http://www.example.com`，裸邮箱 → `mailto:…`。
///
/// 为什么需要：裸链接成链时 `InlineNode.url` 只能指向源码字节（`Range`），
/// 无法在解析层合成协议前缀；而打开链接与导出都要求带协议。
/// 归一化放在消费点（单一函数），解析层保持「不产生文档之外的内容」。
pub fn normalize_link_url(url: &str) -> String {
    let u = url.trim();
    if u.len() >= 4 && u.as_bytes()[..4].eq_ignore_ascii_case(b"www.") {
        format!("http://{u}")
    } else if is_bare_email_target(u) {
        format!("mailto:{u}")
    } else {
        u.to_string()
    }
}

/// 裸邮箱目标（GFM 自动链接）：`local@domain`，domain 含 `.` 且以字母数字结尾。
/// 显式 `mailto:` 前缀与含 scheme 的目标不在此列（保持原样）。
fn is_bare_email_target(u: &str) -> bool {
    if u.contains("://") || u.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = u.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !local.contains(':')
        && domain.contains('.')
        && domain
            .as_bytes()
            .last()
            .is_some_and(|b| b.is_ascii_alphanumeric())
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

/// 单份版本快照的元数据（N1）：时间戳 + 内容哈希 + 字节数。
/// `ts_ms` 同时是槽名（目录内唯一），`hash` 用于去重与损坏校验，`bytes` 供列表展示。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SnapshotMeta {
    pub ts_ms: u64,
    pub hash: u64,
    pub bytes: u64,
}

/// 快照内容哈希（FNV-1a 64）：产品层写元数据/校验与 UI 去重共用同一实现。
pub fn content_hash(data: &[u8]) -> u64 {
    recovery::fnv64(data)
}

/// 版本历史快照存储（N1）：多槽、按文档稳定身份分目录、原子写。
/// 实现由产品层注入；snapshot_text 是 Markdown 源码纯文本。
///
/// 稳定身份键（`doc_key`）由调用方按「规范化绝对路径」或「未命名草稿会话 id」派生；
/// 实现不得把内容哈希当目录键（内容一变历史就碎成多个目录）。
pub trait SnapshotStore {
    /// 保存一份快照；同一内容（哈希相同）不重复写，返回既有元数据。
    fn save_snapshot(&mut self, doc_key: u64, snapshot_text: &str) -> Result<SnapshotMeta, String>;
    /// 列出某文档的全部快照，按时间倒序（最新在前）。损坏/缺失项跳过而不是报错。
    fn list_snapshots(&mut self, doc_key: u64) -> Result<Vec<SnapshotMeta>, String>;
    /// 读取某槽纯文本；缺失/校验失败返回可读错误（调用方展示错误态，不崩溃）。
    fn read_snapshot(&mut self, doc_key: u64, ts_ms: u64) -> Result<String, String>;
    /// 删除某槽（快照 + 元数据）；已不存在视为成功。
    fn delete_snapshot(&mut self, doc_key: u64, ts_ms: u64) -> Result<(), String>;
    /// 清空某文档全部快照，返回删除份数。
    fn clear_snapshots(&mut self, doc_key: u64) -> Result<usize, String>;
    /// 删除某文档最旧的 N 槽（超出保留上限时调用）。
    fn prune(&mut self, doc_key: u64, keep_newest: usize) -> Result<usize, String>;
    /// 未命名草稿首次保存：把临时会话键的快照迁移到路径键目录，返回迁移份数。
    /// 目标目录已有同内容快照时去重跳过。
    fn migrate_key(&mut self, from: u64, to: u64) -> Result<usize, String>;
}

/// 持久化能力打包（0b）：崩溃恢复 + 会话/最近文件 + 版本快照。与 ThemeEnv 同模式，
/// 避免 run/App::new 参数继续膨胀。
pub struct PersistEnv {
    pub recovery: Box<dyn RecoveryStore>,
    pub session: Box<dyn SessionStore>,
    pub snapshot: Box<dyn SnapshotStore>,
}

/// 启动即失败：产品层无法读取目标文件时，以空白草稿开窗并让首帧弹出应用内错误模态。
#[derive(Clone, Debug)]
pub struct StartupError {
    /// 读取失败的目标路径（「重试」重读同一文件）。
    pub path: PathBuf,
    /// 弹窗标题（产品层传 `t(Key::MainOpenFailedTitle)`）。
    pub title: String,
    /// 已含路径/原因/下一步的正文（产品层传 `open_failure_text`）。
    pub message: String,
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
    /// 启动即失败：以空白草稿开窗，并在首帧弹出应用内错误模态。
    pub open_error: Option<StartupError>,
}

/// 外部依赖端口聚合（公共 API）：所有产品层注入的 trait/配置打包成一个 struct，
/// 避免 run/App::new 参数列表继续膨胀。ThemeEnv / PersistEnv 在此保持分组，
/// 内部 AppPorts 会拆平它们。
pub struct Ports {
    pub clipboard: Box<dyn Clipboard>,
    pub store: Box<dyn FileStore>,
    /// 本地时钟：状态栏「已保存 HH:MM」的唯一时间来源（测试注入固定值）。
    pub clock: Box<dyn Clock>,
    pub math: Box<dyn MathRenderer>,
    pub mermaid: Box<dyn MermaidRenderer>,
    pub images: Box<dyn ImageLoader>,
    pub assets: Box<dyn AssetStore>,
    pub exporter: Box<dyn Exporter>,
    pub dialogs: Box<dyn FileDialog>,
    pub watcher: Box<dyn FileWatcher>,
    pub theme_env: ThemeEnv,
    pub persist_env: PersistEnv,
    pub print: Box<dyn Print>,
    pub url_opener: Box<dyn UrlOpener>,
}

pub(crate) enum ConfigSave {
    Theme,
    Window,
    RemoteImages,
    /// 完整写入：settings 模态 OK 时，把所有字段一次性写 config.toml。
    All,
}

/// 被挂起的操作意图：模态结论出来后才执行，避免阻塞事件循环。
pub(crate) enum PendingIntent {
    Close,
    New,
    Open,
    OpenRecent(PathBuf),
    /// 版本历史恢复（B8a）：未保存确认通过后把快照文本写回缓冲区。
    RestoreSnapshot(String),
}

/// 错误弹窗的重试语义：失败后用户点「重试」时应重新执行的动作。
/// 保留失败目标（路径），避免用户在弹窗里重试却落到另一个文件。
#[derive(Clone, Debug)]
pub(crate) enum ErrorPurpose {
    /// Ctrl+S 保存失败：重试写同一路径。
    Save,
    /// 另存为失败：重试写同一目标路径。
    SaveAs(PathBuf),
    /// 打开失败：重试读同一路径。
    Open(PathBuf),
    /// 启动即失败：重试读同一路径；其余出口为「打开其他文件」/关闭本窗口。
    Startup(PathBuf),
    /// 导出 HTML 失败：重试走同一导出流程（重新确认输出位置）。
    ExportHtml,
    /// 导出 PDF 失败：重试走同一导出流程。
    ExportPdf,
    /// 打印失败/中断：重试重新打开打印对话框。
    Print,
    /// 删除某份版本快照（B8a）：危险确认后按时间戳删除。
    HistoryDelete(u64),
    /// 清空版本历史（B8a）：危险确认后清空该文档目录。
    HistoryClear,
}

/// 模态用途：区分「未保存确认」（保存/不保存/取消）、错误弹窗（重试/取消）
/// 与外部变更选择（重新加载/保留当前/另存为/取消）。
#[derive(Clone, Debug)]
pub(crate) enum ModalPurpose {
    Unsaved,
    Error(ErrorPurpose),
    External,
    /// 关于弹窗：任何按钮都只关闭；主按钮额外打开官方 Release 页。
    About,
}

/// 官方唯一入口：关于弹窗「打开项目主页」与「检查更新」共用。
pub(crate) const RELEASES_URL: &str = "https://github.com/31i45/veil/releases";

/// 诊断事件薄封装：veil-ui 单元测试并行运行，测试构建与 `diag` 共用串行锁，
/// 保证依赖事件内容的断言稳定；生产构建直接写入，不引入额外开销。
pub(crate) fn record_diag(kind: &'static str, detail: impl AsRef<str>) {
    #[cfg(test)]
    {
        let _guard = crate::diag::test_lock();
        crate::diag::record(kind, detail);
    }
    #[cfg(not(test))]
    crate::diag::record(kind, detail);
}

/// 应用内模态：规格 + 交互态 + 待执行意图（结论落地后整条替换）。
pub(crate) struct ActiveModal {
    pub(crate) spec: crate::view::modal::ModalSpec,
    pub(crate) state: crate::view::modal::ModalState,
    /// 未保存确认的挂起意图；错误弹窗无此意图（重试动作由 `purpose` 决定）。
    pub(crate) intent: Option<PendingIntent>,
    pub(crate) purpose: ModalPurpose,
}

/// 设置模态：承载 SettingsForm + 独立交互态 + 独立布局。
/// 与 ActiveModal 平行（不走 ModalSpec），因为设置模态有 10+ 可编辑表单控件。
pub(crate) struct SettingsModal {
    pub(crate) form: crate::view::settings::SettingsForm,
    pub(crate) state: crate::view::settings::SettingsState,
    pub(crate) layout: Option<crate::view::settings::SettingsLayout>,
    pub(crate) buttons: Vec<crate::view::modal::ModalButton>,
}

/// 文档视图 UI 组件状态（ADR-009 S6 C6）：chrome/大纲/查找栏/写作模式。
/// 四者是 UI 交互态，无数据互借，合并减少顶层字段。
pub(crate) struct UIState {
    /// 应用内模态（未保存确认等）：Some 时它是唯一输入接收者，绘制在最后一层。
    pub(crate) modal: Option<ActiveModal>,
    /// 设置模态（独立于 modal — 有 10+ 表单控件）。优先级高于 modal。
    pub(crate) settings: Option<SettingsModal>,
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
    /// 表格视图态（B2）：用户拖动的列宽覆盖，纯视图，不写回文档。
    pub(crate) tables: TableView,
    /// 窗口焦点（光标表达）：true = 实心插入点，false = 空心框。
    /// 由 `WindowEvent::Focused` 经 `set_window_focused` 写入，默认聚焦。
    pub(crate) focused: bool,
    /// Details 块展开集合：id 在 set 里表示展开，不在里表示收起。
    /// spec 默认收起（空 set = 全收起）；open_default = true 的块在 refresh_index
    /// 后自动预塞进此集合，尊重 HTML `<details open>`。
    pub(crate) expanded_details: std::collections::HashSet<BlockId>,
    /// WYSIWYG 非活动态隐藏 Front Matter（REN-SWEEP-001）：光标是否停在
    /// Front Matter 块内（最近一次 `sync_frontmatter_at_caret` 的判定）。
    /// 状态翻转时触发几何重算，使块高在 0 与源码高之间切换。
    pub(crate) frontmatter_revealed: bool,
    /// 行内 `<details>` 展开集合（B3）：(块 id, `<details` 开标签的块内相对字节)。
    /// 默认收起（空集）；纯视图态，不写文档字节、不进撤销栈。
    pub(crate) expanded_inline_details: std::collections::HashSet<(BlockId, usize)>,
    /// 链接悬停浮层（N5）：(归一化目标 URL, 锚点屏幕 x, y)；None = 不显示。
    /// 由 `update_hover` 每次鼠标移动即时重算，无定时器。
    pub(crate) hover_link: Option<(String, f32, f32)>,
    /// 当前悬停浮层来自真实链接（而非 `<abbr title>`）：指针形状用（链接=手型）。
    pub(crate) hover_link_is_link: bool,
    /// 版本历史面板（N1/B8a）：左侧快照列表 + 正文区只读预览。
    pub(crate) history: History,
    /// 图片放大预览覆盖层：纯视图态，关闭后文档字节与光标不变。
    pub(crate) image_preview: Option<ImagePreview>,
    /// 正文焦点快照（键盘闭环）：最近一次处于正文模式时的光标/选区/滚动。
    /// `Esc` 关闭浮层后据此归还焦点，保证 caret/selection/scroll 与打开前一致。
    pub(crate) focus_return: Option<DocFocus>,
}

/// 正文焦点快照：`caret` 与 `selection` 用文档偏移表达，`scroll` 用视口像素。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DocFocus {
    pub(crate) caret: usize,
    pub(crate) selection: Option<Range>,
    pub(crate) scroll: f32,
}

/// 图片放大预览（覆盖层）：键为本地绝对路径或远程 URL（与行内图片缓存同键）。
/// `zoom = 1.0` 表示适应窗口；`pan` 为相对居中的屏幕像素偏移。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImagePreview {
    pub(crate) key: String,
    /// 标题（图片 alt 原文；空串时标题只显示「图片预览」）。
    pub(crate) title: String,
    pub(crate) zoom: f32,
    pub(crate) pan: (f32, f32),
}

/// Mermaid 自动实时预览请求：后台本地渲染，回包后直接嵌进编辑界面的
/// LayoutEngine 管线（不经预览浮层）。
#[derive(Clone, Debug)]
pub(crate) struct PendingMermaidAuto {
    pub(crate) block_id: BlockId,
    pub(crate) source_hash: u64,
    /// 请求标签：`mermaid-auto:<hash>`，回包时原样带回用于匹配。
    pub(crate) label: String,
    pub(crate) dark: bool,
}

impl Default for UIState {
    fn default() -> Self {
        Self {
            modal: None,
            settings: None,
            chrome: Chrome::default(),
            outline: Outline::default(),
            search: Search::default(),
            quick: QuickOpen::default(),
            ctx: crate::view::ctx_menu::CtxMenu::default(),
            writing: WritingState::default(),
            tables: TableView::default(),
            focused: true,
            hover_link: None,
            hover_link_is_link: false,
            history: History::default(),
            image_preview: None,
            expanded_details: std::collections::HashSet::new(),
            expanded_inline_details: std::collections::HashSet::new(),
            frontmatter_revealed: false,
            focus_return: None,
        }
    }
}

/// 表格视图态（B2）：列宽覆盖 + 拖动会话。键为块 id，文档切换即清空。
/// 覆盖只在视图层生效：不写回 Markdown，不进入撤销栈。
#[derive(Default)]
pub(crate) struct TableView {
    /// 块 id → 各列宽（物理像素）；长度必须等于该表列数才被采纳。
    pub(crate) widths: HashMap<veil_core::BlockId, Vec<f32>>,
    /// 拖动会话：(块 id, 列下标, 起始鼠标 x, 起始列宽)。
    pub(crate) drag: Option<(veil_core::BlockId, usize, f32, f32)>,
}

impl TableView {
    /// 覆盖内容的 FNV-1a 指纹（0 = 无覆盖）：作为表格缓存键的一部分。
    pub(crate) fn fingerprint(&self, id: veil_core::BlockId) -> u64 {
        match self.widths.get(&id) {
            None => 0,
            Some(ws) => {
                let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                for w in ws {
                    for b in w.to_bits().to_le_bytes() {
                        h ^= b as u64;
                        h = h.wrapping_mul(0x1000_0000_01b3);
                    }
                }
                h
            }
        }
    }
}

/// 写作模式与全文统计（阶段 1）：纯视图开关 + 状态栏派生数字。
#[derive(Default)]
pub(crate) struct WritingState {
    /// 专注模式：非光标所在块整体压暗，仅视觉，不改真值。
    pub(crate) focus: bool,
    /// 打字机模式：光标行中心固定在视口高度 42% 处（仅 `scroll_to_caret` 生效）。
    /// 纯视图开关，不进配置文件，重启后回到默认关闭。
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
    /// 系统高对比度模式（跟随系统时覆盖 light/dark 外观）。
    pub(crate) high_contrast: bool,
    pub(crate) last_theme_check: Option<Instant>,
    /// 拖拽悬停文件（阶段 1）：仅驱动落点提示叠层；松开/离开窗口即清空。
    pub(crate) drop_file: Option<PathBuf>,
    /// 源码模式（Ctrl+/）：全文按 Markdown 源码排版。打印/导出恒按渲染态（见 `App::source_mode`）。
    pub(crate) source_mode: bool,
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
    /// 本会话最近一次成功保存的本地时间（`HH:MM`）：状态栏「已保存」文本的唯一来源。
    /// 只在落盘成功时刷新；失败/打开新文档（None）不得沿用旧值。
    pub(crate) last_saved_hhmm: Option<String>,
    /// 下次定时快照的最早时刻；None = 尚未开始计时（首次 dirty 即触发）。
    /// 节流窗口 30 min；保存成功后立即触发一次快照并重置 deadline。
    pub(crate) snapshot_deadline: Option<Instant>,
}

/// 快照节流间隔：默认 30 分钟（Autosave 2s 是"即时落盘"，快照是"版本历史"，
/// 两者职责不同；节流避免每次 keystroke 写盘）。
const SNAPSHOT_INTERVAL_MIN: u64 = 30;

/// 文档真值状态（ADR-009 S6 C2）：路径/编码标记/基线字节。
pub(crate) struct DocState {
    /// 关联文件路径（None = 草稿）；标题由此派生。
    pub(crate) path: Option<PathBuf>,
    /// 版本历史的稳定文档身份键（N1/B8a）：路径派生或未命名草稿会话 id。
    pub(crate) history_key: u64,
    /// 读入时的 BOM 标记，保存时还原。
    pub(crate) bom: bool,
    /// 上次保存的磁盘字节（或新建时的初始字节）：冲突检测的基线。
    pub(crate) baseline: Vec<u8>,
    /// 当前主题的导出样式快照（Ctrl+E 导出用）：在主题初始化/切换点同步更新，
    /// 使导出与屏幕配色一致；放在文档真值旁是因为导出对象就是当前文档。
    pub(crate) export_style: ExportStyle,
}

/// 未绘制区域的擦除色：取当前主题底色的 RGB（丢弃 alpha），
/// 由窗口类背景画刷在首帧覆盖前兜底，避免露出系统默认黑色。
#[cfg(any(windows, test))]
pub(crate) fn window_erase_rgb(bg: [u8; 4]) -> [u8; 3] {
    [bg[0], bg[1], bg[2]]
}

/// 首帧显示保护时限：呈现失败/无事件唤醒时也必须显示窗口，杜绝永久不可见。
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(1);

/// 首帧显示状态机（Task 5）：窗口以隐藏状态创建，首帧 `present` 成功后一次性显示；
/// 失败时由保护时限兜底显示。纯状态（无 winit/GUI），可独立单测。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StartupGate {
    /// 隐藏起点：保护时限从此刻起算。
    started: Instant,
    revealed: bool,
}

impl StartupGate {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            started: now,
            revealed: false,
        }
    }

    /// 保护时限到期点（`next_wake` 据此登记唯一一次唤醒）。
    pub(crate) fn deadline(&self) -> Instant {
        self.started + FIRST_FRAME_TIMEOUT
    }

    #[cfg(test)]
    pub(crate) fn revealed(&self) -> bool {
        self.revealed
    }

    /// 首帧呈现结果：成功且尚未显示时返回 true（调用方执行一次显示）。
    /// 之后再次成功（尺寸变化触发的重复首帧）不再重复显示。
    pub(crate) fn on_present(&mut self, ok: bool) -> bool {
        if ok && !self.revealed {
            self.revealed = true;
            return true;
        }
        false
    }

    /// 保护时限到达：无论首帧是否成功都显示。已显示则返回 false。
    pub(crate) fn on_timeout(&mut self, now: Instant) -> bool {
        if !self.revealed && now >= self.deadline() {
            self.revealed = true;
            return true;
        }
        false
    }
}

/// 瞬态窗口/呈现状态（ADR-009 S6 C1）：6 个字段无交叉借用风险，单独子结构。
pub(crate) struct WindowState {
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    /// softbuffer 已设置的 surface 尺寸。softbuffer 0.4 不暴露 `size()`，
    /// 因此由本状态跟踪；`present` 必须在 `buffer_mut()` 之前用它判断是否先 resize。
    pub(crate) surface_size: Option<(u32, u32)>,
    /// 跨帧复用的像素缓冲（P2）：尺寸不变时零分配，绘制结束整块拷入 surface。
    pub(crate) frame: Option<Frame>,
    pub(crate) scroll_y: f32,
    /// 滚动目标：on_wheel/PageUp/PageDown/Home/End/拖拽滚动条写 goal_y；
    /// 产品定位为「瞬时」，写入后立即同步 scroll_y，无插值。
    pub(crate) goal_y: f32,
    /// 垂直移动的目标列（goal column），上下移动时保持。
    pub(crate) goal_x: Option<f32>,
    /// 左键按住中：CursorMoved 转化为拖选（扩选）。
    pub(crate) dragging: bool,
    /// 最近一次正文左键：(时刻, 位置, 连续点击数 1..=3)，双击/三击判定依据。
    pub(crate) last_click: Option<(Instant, PhysicalPosition<f64>, u8)>,
    /// 打印会话态（3b）：Some 期间 win_size/viewport/绘制全部切换到打印页帧语义。
    pub(crate) print: Option<PrintState>,
    /// 无边框全屏（3c-3）：F11/视图菜单切换；Esc 不退出（Esc 只关菜单）。
    pub(crate) fullscreen: bool,
    /// 尺寸待处理（3e R2）：Resized 只置位，索引重建/重排合并到下一帧一次执行，
    /// 避免拖拽窗口时逐事件全量重建造成键盘输入积压。
    pub(crate) resize_pending: bool,
    /// 窗口可见性（3g）：最小化/失焦/Occluded 时置 false，阻止 RedrawRequested → render → paint 路径。
    pub(crate) visible: bool,
    /// 首帧守卫：Some 期间窗口仍隐藏，首帧成功/兜底超时后置 None 并显示（Task 5）。
    pub(crate) startup: Option<StartupGate>,
    /// 已获准退出：仅当「关闭意图」真正执行后置 true，由 Shell 只移除当前窗口。
    pub(crate) exit_requested: bool,
    /// 用户请求新建独立窗口；Shell 消费后在主事件循环中创建另一个 App。
    pub(crate) new_window_requested: bool,
    /// 窗口几何有变更待落盘（3d）：移动/缩放期间只置位不写盘（避免拖动窗口时反复 IO），
    /// 退出出口统一保存一次后清零。
    pub(crate) geom_dirty: bool,
    /// 本次会话的实时几何：与 `config.window` 分开存放，最大化/最小化期间不写入，
    /// 避免把最大化尺寸覆盖成「上次常规尺寸」。
    pub(crate) geom: Option<WindowGeom>,
    /// 悬停过渡上一帧时刻（S6）：按 ~60fps 限频重绘，避免窗口空转。
    pub(crate) anim_last_frame: Option<Instant>,
    /// 最近一次绘制时编辑器光标位置：`tick` 用它判断「光标移动」是否已被
    /// 编辑路径自带的重绘覆盖，避免同一击键再补画一帧完全相同的画面。
    pub(crate) last_painted_caret: Option<usize>,
    /// 已写入窗口的标题缓存：脏标只在首次编辑翻转，后续击键跳过 SetWindowText。
    pub(crate) last_title: Option<String>,
    /// 状态栏提示的可见起点：(文本, 首次见到它的 tick 时刻)。同一条提示展示满
    /// `NOTICE_TTL` 后自动消退；换文本即重新计时。
    pub(crate) notice_seen: Option<(String, Instant)>,
    /// 已写入窗口的指针形状：变化时才调用 `set_cursor`，避免每次鼠标移动都下发。
    pub(crate) pointer: Option<winit::window::CursorIcon>,
    /// 代码块复制按钮的就地成功反馈：指向最近复制的块（按钮标签临时显示「已复制」）。
    pub(crate) code_copied: Option<BlockId>,
    /// 测试专用窗口尺寸覆盖：headless 测试无 winit 窗口，需要验证超宽窗口排版时注入。
    #[cfg(test)]
    pub(crate) test_size: Option<(u32, u32)>,
    /// 测试 hook：`set_ime_area` 每次被调时的 (x, y, h) 坐标入栈。
    /// 无窗口（headless 单测）时同样记录——用于锁定候选窗锚点正确性。
    #[cfg(test)]
    pub(crate) ime_area_calls: Vec<(f32, f32, f32)>,
    /// 启动失败的可读详情：文档打开错误在 `App::new` 预置，窗口/呈现上下文创建失败时追加原始诊断。
    /// 仅在「首窗创建失败」时由 Shell 取走并作为致命错误上报；正常启动路径不消费。
    pub(crate) startup_error: Option<String>,
}
impl Default for WindowState {
    fn default() -> Self {
        Self {
            window: None,
            surface: None,
            surface_size: None,
            frame: None,
            scroll_y: 0.0,
            goal_y: 0.0,
            goal_x: None,
            dragging: false,
            last_click: None,
            print: None,
            fullscreen: false,
            resize_pending: false,
            visible: true,
            startup: None,
            exit_requested: false,
            new_window_requested: false,
            geom_dirty: false,
            geom: None,
            anim_last_frame: None,
            last_painted_caret: None,
            last_title: None,
            notice_seen: None,
            pointer: None,
            code_copied: None,
            #[cfg(test)]
            test_size: None,
            #[cfg(test)]
            ime_area_calls: Vec::new(),
            startup_error: None,
        }
    }
}

impl WindowState {
    /// 取走启动失败详情（一次性）：Shell 只在首窗创建失败时消费。
    pub(crate) fn take_startup_error(&mut self) -> Option<String> {
        self.startup_error.take()
    }
}

/// 首帧呈现后才执行的非关键启动工作：不影响首帧内容，但必须保留语义
/// （最近文件仍会登记、文件监听仍会启动，只是晚到首帧之后）。
#[derive(Default)]
pub(crate) struct DeferredStartup {
    /// 待登记并落盘的最近文件路径（`App::new` 时的当前文档）。
    pub(crate) note_opened: Option<PathBuf>,
    /// 是否需要在首帧后启动文件监听。
    pub(crate) watch: bool,
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
    /// 启动时读入的配置（主题模式 + PDF 导出设置）；写回时整体落盘。
    pub(crate) config: Config,
    /// 自定义 Callout 表（启动时读 `callouts.json`）：重建编辑器时随文档解析注入。
    pub(crate) callouts: veil_core::CalloutRegistry,
    pub(crate) window_state: WindowState,
    pub(crate) editor: Editor,
    pub(crate) engine: LayoutEngine,
    /// 块几何索引（P1a）：全量块枚举的唯一缓存来源，替代每次读取时的 `blocks()`。
    pub(crate) index: BlockIndex,
    /// 派生缓存（布局/光栅/公式/图片/表格）与因果键查询。
    pub(crate) cache: Cache,
    /// 等待本地 Mermaid 渲染结果的自动预览队列（label 为匹配键）。
    pub(crate) pending_mermaid_auto: Vec<PendingMermaidAuto>,
    /// 已请求过的 Mermaid (source_hash, dark) 集合——同一源码 + 主题不重复请求。
    pub(crate) mermaid_auto_requested: std::collections::HashSet<(u64, bool)>,
    pub(crate) input_state: InputState,
    /// 11 个外部注入的端口（Clipboard / FileStore / MathRenderer / ImageLoader /
    /// AssetStore / Exporter / FileDialog / FileWatcher / ConfigStore / SystemTheme /
    /// WindowEffects）聚合为单结构（ADR-009 S5）。
    pub(crate) ports: AppPorts,
    pub(crate) persist_state: PersistState,
    pub(crate) ui_state: UIState,
    // —— 光标闪烁状态 —— //
    /// 当前光标是否可见（tick 中每 600 ms 翻转一次）。
    pub(crate) cursor_visible: bool,
    /// 最近一次可见性翻转时刻：`None` 表示尚未开始闪烁（或首次进入）。
    pub(crate) last_blink: Option<Instant>,
    /// 上一次 tick 记录的 caret 值——用于检测 caret 移动并立即重置可见性。
    pub(crate) last_seen_caret: usize,
    /// 推迟到首帧之后执行的启动工作（最近文件登记 / 文件监听启动）。
    pub(crate) deferred_startup: DeferredStartup,
}

/// 15 个平台端口聚合结构：能力集合，不携带状态语义（ADR-009 决策六）。
/// 全部为 Box<dyn Trait>，由产品层注入（veil/main.rs 各 impl 对应 trait）。
pub(crate) struct AppPorts {
    pub(crate) clipboard: Box<dyn Clipboard>,
    pub(crate) store: Box<dyn FileStore>,
    pub(crate) clock: Box<dyn Clock>,
    pub(crate) math: Box<dyn MathRenderer>,
    pub(crate) mermaid: Box<dyn MermaidRenderer>,
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
    pub(crate) snapshot: Box<dyn SnapshotStore>,
    pub(crate) print: Box<dyn Print>,
    pub(crate) url_opener: Box<dyn UrlOpener>,
}

/// 启动文档决议（0b）：resume 模式下按「崩溃恢复 > 会话恢复 > 传入文档」重建。
/// 非 resume（CLI 显式打开）只加载最近列表（菜单需要），恢复文件保留不动。
fn startup_doc(ports: &mut AppPorts, doc: Doc) -> StartupDoc {
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
        recovery_hash: None,
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
                        // 恢复提示带上真实对象、时间与保存去向：路径为文件名的文件，无路径即「未命名草稿」；
                        // 时间为本次恢复发生的本地时刻（恢复文件本身不含时间戳）。保存去向按原路径
                        // 推导（父目录名），无路径时说明可直接选择新位置，避免提示只在状态栏止步。
                        let name = r
                            .path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .and_then(|n| n.to_str())
                            .unwrap_or(t(Key::UntitledDraft));
                        let save_hint = match r
                            .path
                            .as_ref()
                            .and_then(|p| p.parent())
                            .and_then(|dir| dir.file_name())
                            .and_then(|n| n.to_str())
                        {
                            Some(dir) => tf(Key::RecoveredSaveToDir, &[dir]),
                            None if r.path.is_some() => t(Key::RecoveredSaveToFile).to_string(),
                            None => t(Key::RecoveredSaveChooseLocation).to_string(),
                        };
                        let hhmm = ports.clock.local_hhmm();
                        out.notice = tf(Key::NoticeRecovered, &[name, &hhmm, &save_hint]);
                        out.doc.text = r.text;
                        out.doc.path = r.path;
                        out.doc.bom = r.bom;
                        out.recovered = true;
                        // 快照仍在磁盘上且已被本会话消费：记下它的指纹，保存成功时才能
                        // 正确作废。否则 clear_recovery 会误判「本会话没写过」而跳过清除，
                        // 导致旧草稿每次启动都被恢复、每次都要另存为（3e 修 R1）。
                        out.recovery_hash = Some(recovery::fnv64(&bytes));
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
    /// 启动时从恢复文件消费到的快照指纹（R1）：非 None 表示磁盘上尚存属于本会话文档的
    /// 恢复副本，保存成功后须清除；None 表示磁盘上无本会话副本（不得误清他处快照）。
    recovery_hash: Option<u64>,
    recent: Vec<PathBuf>,
}

impl App {
    pub(crate) fn new(doc: Doc, ports: Ports) -> Self {
        // 启动即失败的目标：文档以空白草稿开窗，错误在窗口建好后用应用内模态告知
        let open_error = doc.open_error.clone();
        let ThemeEnv {
            mut config,
            mut system,
            effects,
            callouts,
        } = ports.theme_env;
        let PersistEnv {
            recovery,
            session,
            snapshot,
        } = ports.persist_env;
        // 系统外观读不到（非 Windows/注册表异常）按浅色处理：不影响其它功能
        let system_dark = system.is_dark().unwrap_or(false);
        let high_contrast = system.is_high_contrast();
        let (app_config, config_notice) = match config.load() {
            Ok(text) => match theme::decode_config(text.as_bytes()) {
                Ok(p) if p.ignored == 0 => (p.config, String::new()),
                Ok(p) => (
                    p.config,
                    tf(Key::NoticeConfigIgnored, &[&p.ignored.to_string()]),
                ),
                Err(e) => (
                    Config::default(),
                    tf(Key::NoticeConfigInvalid, &[&e.to_string()]),
                ),
            },
            // 文件不存在属正常状态（首次运行）：静默用默认配置
            Err(_) => (Config::default(), String::new()),
        };
        crate::diag::perf_stage("app-config");
        // 跟随系统且高对比度开启时，强制使用高对比度主题（覆盖 light/dark）
        let theme = if app_config.mode == ThemeMode::System && high_contrast {
            Theme::high_contrast()
        } else {
            Theme::of(app_config.mode, system_dark)
        };
        // 导出样式随主题一起初始化：之后只在 refresh_theme_visuals 一处同步，避免两处漂移
        let export_style = ExportStyle::from_theme(&theme);
        crate::diag::perf_stage("app-theme");
        let mut ports = AppPorts {
            clipboard: ports.clipboard,
            store: ports.store,
            clock: ports.clock,
            math: ports.math,
            mermaid: ports.mermaid,
            images: ports.images,
            assets: ports.assets,
            exporter: ports.exporter,
            dialogs: ports.dialogs,
            watcher: ports.watcher,
            config,
            system,
            effects,
            recovery,
            session,
            snapshot,
            print: ports.print,
            url_opener: ports.url_opener,
        };
        let start = startup_doc(&mut ports, doc);
        crate::diag::perf_stage("app-startup-doc");
        let mut editor = Editor::new_with_callouts(&start.doc.text, &callouts);
        crate::diag::perf_stage("app-editor");
        if start.recovered {
            editor.set_dirty(true); // 恢复内容相对磁盘基线是未保存修改
        }
        let initial_caret = editor.caret();
        let mut persist = Persist::default();
        if start.missing {
            persist.set_missing(true);
        }
        // 图片缓存字节上限固化为常量 DEFAULT_IMAGE_CACHE_LIMIT_MB（200 MiB，0 = 不限制）
        let mut cache = Cache::default();
        cache.set_image_limit_mb(theme::DEFAULT_IMAGE_CACHE_LIMIT_MB);
        // 字体系统在 LayoutEngine::new 内创建：提前绑定到局部变量，便于在其后埋点。
        let engine = LayoutEngine::new();
        crate::diag::perf_stage("app-fonts");
        let mut app = App {
            doc: DocState {
                path: start.doc.path.clone(),
                history_key: start
                    .doc
                    .path
                    .as_deref()
                    .map(history::doc_key)
                    .unwrap_or_else(history::session_key),
                bom: start.doc.bom,
                baseline: start
                    .baseline
                    .unwrap_or_else(|| encode(&start.doc.text, start.doc.bom)),
                export_style,
            },
            config: app_config,
            window_state: WindowState::default(),
            editor,
            engine,
            index: BlockIndex::default(),
            cache,
            pending_mermaid_auto: Vec::new(),
            mermaid_auto_requested: std::collections::HashSet::new(),
            callouts,
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
                high_contrast,
                last_theme_check: None,
                drop_file: None,
                source_mode: false,
            },
            ports,
            persist_state: PersistState {
                persist,
                autosave: Autosave::default(),
                recovery_hash: start.recovery_hash,
                recovery_failed_at: None,
                recent: start.recent,
                last_saved_hhmm: None,
                snapshot_deadline: None,
            },
            ui_state: UIState::default(),
            cursor_visible: true,
            last_blink: None,
            last_seen_caret: initial_caret,
            deferred_startup: DeferredStartup::default(),
        };
        app.ui_state.chrome.notify(
            if start.notice.is_empty() {
                config_notice
            } else {
                start.notice // 恢复提示优先于配置提示
            },
            NoticeKind::Info,
        );
        app.engine.set_syntax_colors(app.input_state.theme.syntax);
        app.engine.set_link_color(app.input_state.theme.link);
        app.engine.set_muted_color(app.input_state.theme.muted);
        // 正文排版（字号/行高/字体族）：必须在首次排版前设置；缓存此时为空，无需失效。
        app.engine.set_doc_typography(
            app.config.doc_font_size,
            app.config.doc_line_height,
            app.config.doc_font_family,
            app.config.doc_font_face.as_deref(),
        );
        app.engine
            .set_doc_margin(app.config.doc_margin, app.config.doc_margin_px);
        app.doc.export_style.font_family = app.engine.doc_face();
        app.refresh_index();
        crate::diag::perf_stage("app-index");
        // 真实 `recompute_geometry()` 推迟到首次 render 的 `geometry_stale` 分支。
        // 窗口尚未创建时 cosmic-text 无 viewport，提前重算只会在 cache.layout()
        // 上做全量 miss 判定（每块构造 BaseKey + 哈希 + 查表），算完首次 render 还会
        // 重跑一次——直接删掉，让 estimate_geometry 用 est_h 先顶一下。
        // 启动即登记最近文件（CLI 打开/会话恢复/崩溃恢复路径统一入口）+ 启动文件监听：
        // 两者都不影响首帧内容（最近列表只喂菜单/快速打开，监听只为后续外部变更轮询），
        // 推迟到首帧呈现后再执行，把会话落盘与监听线程创建的 IO 移出首帧关键路径。
        // 语义不变：`flush_deferred_startup` 在首次呈现（或兜底显示）后必执行一次。
        if let Some(p) = app.doc.path.clone() {
            app.deferred_startup.note_opened = Some(p);
        }
        app.deferred_startup.watch = true;
        if let Some(err) = open_error {
            // 窗口建不起来时这条详情就是用户唯一能看到的线索，先留存再交给应用内模态。
            app.window_state.startup_error = Some(err.message.clone());
            app.show_startup_error(err);
        }
        app
    }

    /// 窗口外观效果（深色标题栏/Mica）：窗口创建或主题切换时调用；无窗口/非 Win32 静默跳过。
    /// 效果失败不影响内容绘制（内容区始终为不透明底色，Win10 上即纯色外观）。
    fn apply_window_effects(&mut self) {
        #[cfg(windows)]
        {
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let Some(win) = &self.window_state.window else {
                return;
            };
            let Ok(handle) = win.window_handle() else {
                return;
            };
            if let RawWindowHandle::Win32(p) = handle.as_raw() {
                self.apply_window_effects_to(crate::theme::NativeWindow { hwnd: p.hwnd.get() });
            }
        }
        #[cfg(not(windows))]
        let _ = &self.ports.effects;
    }

    /// 把窗口外观效果下发给已解析的原生窗口：深色标题栏 + 擦除色 = 当前主题底色。
    /// 与 HWND 获取解耦，使「主题切换后使用当前主题 bg」可在无窗口测试中验证。
    #[cfg(any(windows, test))]
    pub(crate) fn apply_window_effects_to(&mut self, native: crate::theme::NativeWindow) {
        self.ports
            .effects
            .apply(native, self.input_state.theme.dark);
        // 未绘制区域擦除色 = 主题底色：首帧覆盖前不会露出系统默认黑色
        self.ports
            .effects
            .set_background(native, window_erase_rgb(self.input_state.theme.bg));
    }

    /// 记录窗口移动后的几何（尺寸沿用已存值）。`minimized` 为真时忽略事件：
    /// 最小化窗口的 Moved 坐标是 -32000 哨兵，不得写入持久化几何。
    pub(crate) fn on_window_moved(&mut self, x: i32, y: i32, minimized: bool) {
        if minimized {
            return;
        }
        let (width, height) = self.window_state.geom.or(self.config.window).map_or_else(
            || self.logical_window_size_of().unwrap_or((900, 700)),
            |g| (g.width, g.height),
        );
        self.window_state.geom = Some(WindowGeom {
            width,
            height,
            x,
            y,
        });
        self.window_state.geom_dirty = true;
    }

    /// 主题切换后刷新按颜色生成的全部缓存：主题代次递增使全部颜色相关派生缓存
    /// 按因果键失效（布局含语法高亮色，自本次起同步重排）；窗口外观同步更新。
    /// pub(crate) 供性质测试（S6 阶段 B）构造主题切换序列。
    pub(crate) fn refresh_theme_visuals(&mut self) {
        self.engine.set_syntax_colors(self.input_state.theme.syntax);
        self.engine.set_link_color(self.input_state.theme.link);
        self.engine.set_muted_color(self.input_state.theme.muted);
        // 导出样式与渲染样式同点更新：主题切换后立即导出也应拿到新配色
        self.doc.export_style = ExportStyle::from_theme(&self.input_state.theme);
        // 导出正文字族与屏内排版保持同族（引擎在设置生效时已驻留该名）。
        self.doc.export_style.font_family = self.engine.doc_face();
        self.input_state.revs.theme += 1;
        self.ui_state.chrome.clear_label_cache();
        // Mermaid 位图随主题重渲：dark 参数进渲染请求，主题切换必须清空位图/失败记忆/去重集合，
        // 下一帧 trigger_mermaid_auto 会按新 theme 重新投递本地渲染请求
        self.cache.mermaid_imgs.clear();
        self.cache.mermaid_failed.clear();
        self.mermaid_auto_requested.clear();
        self.pending_mermaid_auto.clear();
        // 代次递增后按新调色板重排/重渲：只重做可见块，几何按新高度重算
        self.refresh_index();
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        self.apply_window_effects();
        self.request_redraw();
    }

    /// 源码模式有效值：打印/导出期间恒 false（纸面永远是渲染态），
    /// 打印结束回到屏幕后按用户开关恢复。base_key 与渲染判定统一读此口。
    pub(crate) fn source_mode(&self) -> bool {
        self.input_state.source_mode && self.window_state.print.is_none()
    }

    /// 切换主题模式并持久化。切到「跟随系统」时立即重读一次系统外观与高对比度状态，
    /// 避免沿用陈旧值。高对比度开启时覆盖 light/dark 外观。
    pub(crate) fn set_theme_mode(&mut self, mode: ThemeMode) {
        if mode == ThemeMode::System {
            if let Some(dark) = self.ports.system.is_dark() {
                self.input_state.system_dark = dark;
            }
            self.input_state.high_contrast = self.ports.system.is_high_contrast();
        }
        let new_theme = if mode == ThemeMode::System && self.input_state.high_contrast {
            Theme::high_contrast()
        } else {
            Theme::of(mode, self.input_state.system_dark)
        };
        self.input_state.theme = new_theme;
        self.config.mode = mode;
        crate::app::record_diag("theme", mode.as_str());
        self.input_state.last_theme_check = None;
        self.refresh_theme_visuals();
        // 先给成功提示再落盘：保存失败时错误提示更重要，覆盖为「配置未保存」
        self.ui_state
            .chrome
            .notify(tf(Key::NoticeTheme, &[mode.label()]), NoticeKind::Success);
        self.save_config(ConfigSave::Theme);
        self.request_redraw();
    }

    /// 配置落盘：先读最新文件，只覆盖当前动作负责的字段，避免多窗口旧快照互写。
    /// 返回是否写入成功；失败时已写入可见错误提示，调用方不得再覆盖为成功提示
    /// （DEF-T10-008）。
    pub(crate) fn save_config(&mut self, part: ConfigSave) -> bool {
        let mut merged = self.config.clone();
        // 保留原文：严格 TOML 时原地编辑，注释与未知键不丢（ADR-012 / S2）。
        let raw = self.ports.config.load().ok();
        if let Some(text) = raw.as_deref() {
            if let Ok(parsed) = theme::decode_config(text.as_bytes()) {
                merged = parsed.config;
            }
        }
        match part {
            ConfigSave::Theme => merged.mode = self.config.mode,
            ConfigSave::Window => merged.window = self.config.window,
            ConfigSave::RemoteImages => {
                merged.allow_remote_images = self.config.allow_remote_images
            }
            // All: 完整覆盖所有字段（settings 模态 OK 时）
            ConfigSave::All => {
                merged = self.config.clone();
            }
        }
        let text = merged.to_toml_preserving(raw.as_deref());
        if let Err(e) = self.ports.config.save(&text) {
            self.ui_state
                .chrome
                .notify_error(tf(Key::NoticeConfigNotSaved, &[&e.to_string()]));
            eprintln!("配置保存失败: {e}");
            return false;
        }
        true
    }

    /// 跟随系统时轮询系统外观（限频）；外观/高对比度切换则重建调色板并刷新颜色缓存。
    /// WM_SYSCOLORCHANGE 不直接触发——winit 未暴露该消息；改为 1s 轮询检测。
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
        let hc = self.ports.system.is_high_contrast();
        let old_dark = self.input_state.system_dark;
        let old_hc = self.input_state.high_contrast;
        self.input_state.system_dark = dark;
        self.input_state.high_contrast = hc;

        // 任何一项变化都需要重建主题
        if hc != old_hc || dark != old_dark {
            let new_theme = if hc {
                Theme::high_contrast()
            } else {
                Theme::of(ThemeMode::System, dark)
            };
            self.input_state.theme = new_theme;
            self.refresh_theme_visuals();
        }
    }

    /// 全局阻止/允许远程图片（视图菜单）：翻转配置并落盘，递增远程图片代次使
    /// 全部布局失配重排（本地图片缓存键不变，不触发重新读盘）。
    pub(crate) fn toggle_remote_images(&mut self) {
        self.config.allow_remote_images = !self.config.allow_remote_images;
        let saved = self.save_config(ConfigSave::RemoteImages);
        self.input_state.revs.remote_images += 1;
        self.refresh_index();
        self.ensure_layouts_visible();
        self.ensure_math_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        // 保存失败时保留 save_config 写入的「配置未保存」，不得假报开关已持久化
        // （DEF-T10-008）。
        if saved {
            self.ui_state.chrome.notify(
                if self.config.allow_remote_images {
                    t(Key::NoticeRemoteImagesAllowed)
                } else {
                    t(Key::NoticeRemoteImagesBlocked)
                },
                NoticeKind::Success,
            );
        }
        self.request_redraw();
    }

    /// 自动触发 Mermaid 实时预览：扫描所有 BlockKind::Mermaid 块，
    /// 非源码模式 + caret 不在块内 + 未请求过 → 用稳定 label 投递本地渲染器。
    /// 渲染完全在进程内完成，不产生任何网络请求。
    ///
    /// 失败记忆策略：同一 (source_hash, dark) 连续失败 ≥ MAX_MERMAID_FAIL_COUNT（3）
    /// 次才永久禁用；源码超限（> 128 KiB）不进失败记忆，允许用户编辑后自动重试。
    pub(crate) fn trigger_mermaid_auto(&mut self) {
        if self.source_mode() {
            return;
        }
        let theme_dark = self.input_state.theme.dark;
        let caret = self.effective_caret();
        let session = self.editor.session();
        /// 同一源码连续失败 N 次后永久禁用自动预览，直到源码变化或主题切换。
        const MAX_MERMAID_FAIL_COUNT: u8 = 3;

        for (block_id, kind, range) in session.blocks() {
            if kind != veil_core::BlockKind::Mermaid {
                continue;
            }
            // caret 在块内 → 编辑态，不自动渲染（与 math_shows_image 同逻辑）
            if range.start <= caret && caret <= range.end {
                continue;
            }
            // 源码 hash + 主题做去重键：同一 Mermaid 块同一主题只请求一次。
            // 必须用 fenced_code_source 取围栏**内容**：block range 含 ``` 围栏行，
            // 直接 text_slice 会把 fence 一起喂给 mermaid（解析失败）。
            let source = session
                .fenced_code_source(block_id)
                .map(|(_, body)| body)
                .unwrap_or_default();
            // 空块没有可渲染图形，不投递必然失败的请求。
            if source.trim().is_empty() {
                continue;
            }
            let source_hash = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                source.hash(&mut h);
                h.finish()
            };
            let key = (source_hash, theme_dark);
            // 已成功渲染过的块不再请求
            if self.cache.mermaid_imgs.contains_key(&block_id) {
                continue;
            }
            // 已请求过（pending）的不再重复投递（等待回包）
            if self.mermaid_auto_requested.contains(&key) {
                continue;
            }
            // 同一源码连续失败 ≥ MAX 次 → 永久禁用自动预览
            if self.cache.mermaid_failed.get(&key).copied().unwrap_or(0) >= MAX_MERMAID_FAIL_COUNT {
                continue;
            }
            // 源码超长 → 直接跳过 + 提示；不进失败记忆：
            // 用户编辑源码变小后应该能自动重试
            if source.len() > 128 * 1024 {
                self.ui_state
                    .chrome
                    .notify_error(t(Key::NoticeMermaidSourceTooLarge));
                continue;
            }

            let label = format!(
                "mermaid-auto:{:016x}",
                content_hash(format!("{source}\0{theme_dark}").as_bytes())
            );
            self.mermaid_auto_requested.insert(key);
            self.pending_mermaid_auto.push(PendingMermaidAuto {
                block_id,
                source_hash,
                label: label.clone(),
                dark: theme_dark,
            });
            self.ports.mermaid.request(label, source, theme_dark);
        }
    }

    /// 取走已完成的远程图片 + 本地 Mermaid 渲染结果。
    ///
    /// 两条管线：
    /// 1. 远程图片管线：`ports.images.poll_remote()` 取走 `ImageData`（PNG 解码后的字节）
    ///    —— 仅服务普通远程 `<img>`（用户显式允许时）。
    /// 2. Mermaid SVG 管线：`ports.mermaid.poll()` 取走本地渲染的 `MermaidImage`。
    ///
    /// 任一结果都递增 remote_images 代次并触发重排重绘。
    pub(crate) fn poll_remote_images(&mut self) {
        // —— 管线 1：远程图片（PNG/JPEG/WebP 等 ImageData） ——
        let img_results = self.ports.images.poll_remote();
        let img_had = !img_results.is_empty();
        let mut failed = false;
        for (url, result) in img_results {
            self.cache.remote_pending.remove(&url);
            match result {
                Ok(img) => {
                    self.cache.remote_failed.remove(&url);
                    self.cache.insert_remote_image(url, img);
                }
                Err(_) => {
                    self.cache.remote_failed.insert(url);
                    failed = true;
                }
            }
        }

        // —— 管线 2：Mermaid 本地渲染 + resvg 光栅化 ——
        let mermaid_results = self.ports.mermaid.poll();
        let mut mermaid_changed = false;
        // Mermaid 专属失败原因（裁剪后进状态栏）：与普通远程图片失败区分，符合 AC-7
        let mut mermaid_err: Option<String> = None;
        for (label, result) in mermaid_results {
            if let Some(pos) = self
                .pending_mermaid_auto
                .iter()
                .position(|p| p.label == label)
            {
                let pending = self.pending_mermaid_auto.remove(pos);
                match result {
                    Ok(bitmap) => {
                        let key = (pending.source_hash, pending.dark);
                        // 成功渲染后清除失败记忆：同一源码下次变更（key 变了）自然重新计时
                        self.cache.mermaid_failed.remove(&key);
                        self.mermaid_auto_requested.remove(&key);
                        self.cache.mermaid_imgs.insert(pending.block_id, bitmap);
                        mermaid_changed = true;
                    }
                    Err(e) => {
                        // 同一 (source_hash, dark) 连续失败 N 次才永久禁用
                        let key = (pending.source_hash, pending.dark);
                        let count = self.cache.mermaid_failed.get(&key).copied().unwrap_or(0);
                        self.cache.mermaid_failed.insert(key, count + 1);
                        self.mermaid_auto_requested.remove(&key);
                        failed = true;
                        mermaid_err = Some(clip_error(&e));
                        eprintln!("Mermaid 渲染失败 (连续第 {} 次): {e}", count + 1);
                    }
                }
            }
        }

        if failed || mermaid_changed || img_had {
            if failed {
                // Mermaid 失败优先展示其具体原因（已裁剪为单行）；普通远程图片失败用通用文案
                self.ui_state.chrome.notify_error(match mermaid_err {
                    Some(e) => tf(Key::NoticeMermaidRenderFailed, &[&e]),
                    None => t(Key::NoticeRemoteImageFailed).to_string(),
                });
            }
            self.input_state.revs.remote_images += 1;
            self.refresh_index();
            self.ensure_layouts_visible();
            self.ensure_math_visible();
            self.ensure_tables_visible();
            self.recompute_geometry();
            self.request_redraw();
        }
    }

    pub(crate) fn win_size(&self) -> (u32, u32) {
        // 打印中：「窗口」即打印页帧（无菜单栏/状态栏），render() 产出整页帧
        if let Some(p) = &self.window_state.print {
            return (p.w.max(1), p.h.max(1));
        }
        // 测试注入的尺寸优先于无窗口默认值（打印语义已在上一分支返回，不受影响）
        #[cfg(test)]
        if let Some(size) = self.window_state.test_size {
            return size;
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

    /// 当前窗口的逻辑尺寸（物理尺寸 / 缩放因子）。无窗口、物理尺寸为 0（最小化）
    /// 或缩放因子非法时返回 None——此时尺寸不可作为可持久化的几何。
    fn logical_window_size_of(&self) -> Option<(u32, u32)> {
        let window = self.window_state.window.as_ref()?;
        let scale = window.scale_factor();
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return None;
        }
        let w = (size.width as f64 / scale).round().max(1.0) as u32;
        let h = (size.height as f64 / scale).round().max(1.0) as u32;
        Some((w, h))
    }

    pub(crate) fn request_redraw(&self) {
        if let Some(w) = &self.window_state.window {
            w.request_redraw();
        }
    }

    /// 输入活跃信号：键按下 / 鼠标落点 / 拖选 / IME 提交 / 正文滚轮。
    /// 立即让光标回到可见态，并把闪烁周期的起点清空交由下一次 `tick` 重新起算。
    /// 只靠 `tick` 检测 caret 位移不够：Ctrl/Shift、点击原位、滚轮等输入不改变
    /// caret，光标会继续停在被隐藏的半周期里，用户感知为「闪烁响应慢」。
    pub(crate) fn note_caret_activity(&mut self) {
        // 本来可见时不需要为了闪烁重置而重绘（像素不变）；只在隐藏半周期里补帧。
        let was_hidden = !self.cursor_visible;
        self.cursor_visible = true;
        // None 而非 Some(now)：周期起点由后续 tick 的 `now` 决定，
        // 避免事件处理与下一帧之间存在时差时把到期点算偏。
        self.last_blink = None;
        if was_hidden {
            self.request_redraw();
        }
    }

    /// 闪烁是否活跃：文档已加载 + 非打印 + 无 IME 组合串。
    fn blink_active(&self) -> bool {
        self.doc.path.is_some()
            && self.window_state.print.is_none()
            && self.editor.preedit().is_none()
    }

    /// 窗口焦点变化（winit `WindowEvent::Focused`）：写入焦点表达状态，仅在
    /// 真的变化时请求一次重绘（失焦 → 空心插入点）。不注册任何定时器/周期回调。
    pub(crate) fn set_window_focused(&mut self, focused: bool) {
        if self.ui_state.focused != focused {
            self.ui_state.focused = focused;
            self.request_redraw();
        }
    }

    pub(crate) fn set_ime_area(&mut self) {
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
        // 测试 hook：无论有没有窗口都记录坐标，供锚点正确性断言。
        #[cfg(test)]
        self.window_state.ime_area_calls.push((x, y, h));
        // 生产路径：只有有窗口才下发。
        if let Some(w) = self.window_state.window.clone() {
            w.set_ime_cursor_area(
                PhysicalPosition::new(x as f64, y as f64),
                PhysicalSize::new(2.0, h as f64),
            );
        }
    }

    /// 打开/关闭模态时同步 IME：模态独占键盘，若 IME 仍启用，
    /// 中文输入法会先吞掉字母与 Esc，助记符和取消键都到不了弹窗。
    pub(crate) fn sync_ime_for_modal(&mut self) {
        let allowed = self.ui_state.modal.is_none() && self.ui_state.settings.is_none();
        if let Some(w) = &self.window_state.window {
            w.set_ime_allowed(allowed);
        }
        if !allowed {
            self.enter_exclusive_overlay();
        }
    }

    /// 独占覆盖层（菜单/右键/历史/预览/模态）打开的唯一收口：
    /// 组合串属于正文，覆盖层接管输入前必须立即收掉，避免残留下划线或误上屏。
    pub(crate) fn enter_exclusive_overlay(&mut self) {
        self.editor.clear_preedit();
    }

    /// 模态独占覆盖层：弹窗出现前收掉菜单/右键菜单/快速打开/查找栏，
    /// 避免弹窗下方仍可交互，也避免结论落地后浮层重新出现。
    pub(crate) fn close_lite_overlays(&mut self) {
        self.ui_state.chrome.close();
        self.ui_state.ctx.close();
        self.close_quick();
        self.close_search();
    }
}

impl App {
    /// 合并执行待处理的窗口尺寸变化（3e R2）：拖拽改尺寸时系统会连发 Resized，
    /// 逐事件 `refresh_index`（含全部块高度估算与全文统计）会让主线程积压，
    /// 后续方向键等输入被推迟。这里一帧最多重建/重排一次。
    /// 返回是否实际执行了重建。
    pub(crate) fn apply_pending_resize(&mut self) -> bool {
        if !self.window_state.resize_pending {
            return false;
        }
        self.window_state.resize_pending = false;
        // 宽度变化：换行/列宽依赖版心宽；键中的版心宽量化值随之变化，
        // 布局/光栅/表格自然失配，无需显式清缓存
        self.refresh_index();
        self.ensure_layouts_visible();
        self.ensure_tables_visible();
        self.recompute_geometry();
        true
    }

    /// 时间输入的唯一入口：外部轮询、系统外观、自动保存、恢复写盘都只读调用方注入的
    /// `now`（ADR-009 决策三「时间作为输入」）。顺序不可交换：外部修改须先挂起自动保存。
    pub(crate) fn tick(&mut self, now: Instant) {
        // 首帧兜底（Task 5）：隐藏窗口的唤醒源只有这里，超时后无论如何都要显示。
        if self.window_state.startup.is_some() {
            self.settle_startup(false, now);
        }
        self.poll_external();
        self.poll_system_theme(now);
        self.trigger_mermaid_auto();
        self.poll_remote_images();
        // 只读浏览版本历史期间冻结所有持久化副作用：浏览不得改变文档字节，
        // 也不得在预览暂存实时编辑器时触发自动保存/恢复写盘。
        if self.ui_state.history.open {
            return;
        }
        // —— 光标闪烁：caret 移动立即重置；条件满足时每 600 ms 翻转可见性 —— //
        let caret = self.editor.caret();
        if caret != self.last_seen_caret {
            self.cursor_visible = true;
            self.last_blink = Some(now);
            // 显式补帧：不能依赖各编辑路径自带的重绘。只有光标移动、无脏块时
            // 没有任何 after_edit 调用，画面会停留在隐藏半周期的旧帧（光标没跟上）。
            // 若该位置已在最近一帧画过（编辑路径的重绘），再请求只会重复整帧。
            if self.window_state.last_painted_caret != Some(caret) {
                self.request_redraw();
            }
            self.last_seen_caret = caret;
        }
        if self.blink_active() {
            match self.last_blink {
                None => {
                    self.last_blink = Some(now);
                    self.cursor_visible = true;
                }
                Some(t) if now.duration_since(t) >= CURSOR_BLINK => {
                    self.cursor_visible = !self.cursor_visible;
                    self.last_blink = Some(now);
                    // WaitUntil 只唤醒事件循环，不会自行产生 RedrawRequested；
                    // 可见性翻转后必须显式请求重绘，否则屏幕会一直保留旧光标帧。
                    self.request_redraw();
                }
                _ => {} // 间隔未到，保持当前可见性
            }
        } else {
            // 闪烁不活跃：强制可见（preedit / 打印 / 无路径草稿都应显示）
            self.cursor_visible = true;
            self.last_blink = None;
        }
        self.maybe_autosave(now);
        self.maybe_recovery_write(now);
        self.maybe_snapshot(now);
        self.expire_notice(now);
    }

    /// 状态栏提示超时消退：同一文本从首次被看到起计时，到期清空并重绘。
    /// 错误类通知不参与超时，保留到被新通知替换或用户处理。
    fn expire_notice(&mut self, now: Instant) {
        if self.ui_state.chrome.notice.is_empty() {
            self.window_state.notice_seen = None;
            return;
        }
        if !self.ui_state.chrome.current_notice_kind().expires() {
            // 错误提示常驻：清掉计时，下一次新通知会重新计时。
            self.window_state.notice_seen = None;
            return;
        }
        let notice = &self.ui_state.chrome.notice;
        match &self.window_state.notice_seen {
            Some((text, at)) if text == notice => {
                if now.duration_since(*at) >= NOTICE_TTL {
                    self.ui_state.chrome.notice.clear();
                    self.window_state.notice_seen = None;
                    // 就地复制反馈与提示同寿：提示消退时按钮也回到「复制」。
                    self.window_state.code_copied = None;
                    self.request_redraw();
                }
            }
            _ => self.window_state.notice_seen = Some((notice.clone(), now)),
        }
    }

    /// 版本历史快照触发（N1）：30 分钟节流 + 保存前留档（`save` 内）。
    /// 稳定身份键：有路径按路径哈希，未命名草稿按本次会话 id。
    fn maybe_snapshot(&mut self, now: Instant) {
        // 基础条件：dirty + 非 hold + 非 IME（未命名草稿也允许按会话键归档）
        if !self.editor.is_dirty()
            || self.persist_state.persist.hold()
            || self.editor.preedit().is_some()
        {
            return;
        }
        // 节流：deadline 未到则跳过（None = 首次 dirty 立即触发）
        if let Some(deadline) = self.persist_state.snapshot_deadline {
            if now < deadline {
                return;
            }
        }
        let text = self.editor.session().serialize();
        if !text.is_empty() {
            // 先留档磁盘基线，再记当前缓冲：保证列表里旧版本早于新版本，
            // 也避免 30 分钟快照抢在保存留档之前造成顺序倒挂。
            let archive_notice = self.archive_baseline();
            let current_err = self
                .ports
                .snapshot
                .save_snapshot(self.doc.history_key, &text)
                .err();
            // 周期快照失败同样不可静默（release 无控制台）：状态栏留痕。
            if let Some(e) = current_err {
                eprintln!("版本快照写入失败: {e}");
                self.ui_state
                    .chrome
                    .notify_error(tf(Key::NoticeSnapshotFailed, &[&e]));
            } else if let Some(n) = archive_notice {
                self.ui_state.chrome.notify_error(n);
            }
        }
        // 重置节流 deadline
        self.persist_state.snapshot_deadline =
            Some(now + Duration::from_secs(SNAPSHOT_INTERVAL_MIN * 60));
    }

    /// 推进全部悬停过渡（S6）：绘制前调用，paint 只读缓存后的 alpha。
    /// 目标项全部来自现有交互态，事件侧无需额外接线。
    pub(crate) fn animate(&mut self, now: Instant) {
        let ui = &mut self.ui_state;
        let outline = ui.outline.hover;
        ui.outline.fade.tick(now, outline);
        let search = ui
            .search
            .pressed
            .or(ui.search.hover)
            .and_then(search::Hit::anim_index);
        ui.search.fade.tick(now, search);
        let quick = ui.quick.hover;
        ui.quick.fade.tick(now, quick);
        let ctx = ui.ctx.hover;
        ui.ctx.fade.tick(now, ctx);
        ui.chrome.anim_tick(now);
        if let Some(m) = ui.modal.as_mut() {
            let hovered = m.state.hovered;
            m.state.fade.tick(now, hovered);
        }
    }

    /// 是否仍有悬停过渡在跑（事件循环据此按帧继续重绘）。
    pub(crate) fn anim_running(&self) -> bool {
        let ui = &self.ui_state;
        ui.outline.fade.running()
            || ui.search.fade.running()
            || ui.quick.fade.running()
            || ui.ctx.fade.running()
            || ui.chrome.anim_running()
            || ui.modal.as_ref().is_some_and(|m| m.state.fade.running())
    }

    /// 动效总开关（S6）：系统「显示动画」关闭时全部立即到位（逐像素同无动效版本）。
    pub(crate) fn set_animations(&mut self, on: bool) {
        let ui = &mut self.ui_state;
        ui.outline.fade.set_enabled(on);
        ui.search.fade.set_enabled(on);
        ui.quick.fade.set_enabled(on);
        ui.ctx.fade.set_enabled(on);
        ui.chrome.set_animations(on);
        if let Some(m) = ui.modal.as_mut() {
            m.state.fade.set_enabled(on);
        }
    }
}

/// 首窗默认逻辑尺寸（无窗口几何记录时使用）。
const DEFAULT_WIN_W: u32 = 900;
const DEFAULT_WIN_H: u32 = 700;
/// 主窗口最小内部尺寸（逻辑像素）：再小状态栏与多行通知浮层就压成不可读。
/// 仍小于默认 900×700，小屏上不会被强行放大。
pub(crate) const MIN_WINDOW_SIZE: (f32, f32) = (520.0, 360.0);
/// 最小高度须完整容纳的通知行数（错误通知常驻，必须可读）。
#[cfg(test)]
pub(crate) const MIN_NOTICE_LINES: f32 = 4.0;
/// 非客户区预留（物理像素）：覆盖标题栏与边框；150% 下实测约 22×56。
const NON_CLIENT_W: u32 = 24;
const NON_CLIENT_H: u32 = 64;
/// 首窗与工作区边缘的留白（物理像素，每侧）：避免贴住任务栏/屏幕边缘。
const WORK_AREA_MARGIN: u32 = 8;

/// 系统主显示器工作区（物理像素）：Windows 用 `SPI_GETWORKAREA` 扣除任务栏/停靠栏；
/// 非 Windows 或调用失败返回 None，由 winit 显示器整屏矩形回退。
#[cfg(windows)]
fn system_work_area() -> Option<ScreenRect> {
    #[repr(C)]
    struct WorkRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(
            action: u32,
            param: u32,
            data: *mut core::ffi::c_void,
            win_ini: u32,
        ) -> i32;
    }
    const SPI_GETWORKAREA: u32 = 0x0030;
    let mut rect = WorkRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: rect 是有效的可写 RECT；SPI_GETWORKAREA 只写入该结构，不持有指针。
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut rect as *mut WorkRect).cast(), 0)
    };
    if ok == 0 {
        return None;
    }
    let w = (rect.right - rect.left).max(0) as u32;
    let h = (rect.bottom - rect.top).max(0) as u32;
    (w > 0 && h > 0).then_some(ScreenRect {
        x: rect.left,
        y: rect.top,
        w,
        h,
    })
}

#[cfg(not(windows))]
fn system_work_area() -> Option<ScreenRect> {
    None
}

/// 首窗几何：把默认逻辑尺寸收敛到当前显示器工作区（优先 `work_area`，否则整屏）
/// 内并居中，保证窗口与标题栏不越界且不压住任务栏。
/// `screens` 为空（无显示器信息）时原样返回默认尺寸。
pub(crate) fn fit_initial_window(
    desired_w: u32,
    desired_h: u32,
    screens: &[(ScreenRect, f32)],
    work_area: Option<(ScreenRect, f32)>,
) -> WindowGeom {
    let Some(&(screen, screen_scale)) = screens.first() else {
        return WindowGeom {
            width: desired_w.max(1),
            height: desired_h.max(1),
            x: 0,
            y: 0,
        };
    };
    let (area, scale) = work_area.unwrap_or((screen, screen_scale));
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let chrome_w = 2 * WORK_AREA_MARGIN + NON_CLIENT_W;
    let chrome_h = 2 * WORK_AREA_MARGIN + NON_CLIENT_H;
    let max_w = (((area.w.saturating_sub(chrome_w)) as f32) / scale)
        .floor()
        .max(1.0) as u32;
    let max_h = (((area.h.saturating_sub(chrome_h)) as f32) / scale)
        .floor()
        .max(1.0) as u32;
    // 最小尺寸是硬约束：工作区极小时也不能把窗口收敛到不可读尺寸；
    // 由 winit 的最小尺寸保证真实窗口不会更小。
    let (min_w, min_h) = (MIN_WINDOW_SIZE.0 as u32, MIN_WINDOW_SIZE.1 as u32);
    let width = desired_w.clamp(min_w, max_w.max(min_w));
    let height = desired_h.clamp(min_h, max_h.max(min_h));
    let phys_w = (width as f32 * scale).round() as i64;
    let phys_h = (height as f32 * scale).round() as i64;
    let outer_w = phys_w + NON_CLIENT_W as i64;
    let outer_h = phys_h + NON_CLIENT_H as i64;
    let avail_w = area.w as i64 - i64::from(2 * WORK_AREA_MARGIN);
    let avail_h = area.h as i64 - i64::from(2 * WORK_AREA_MARGIN);
    let x = area.x as i64 + i64::from(WORK_AREA_MARGIN) + (avail_w - outer_w).max(0) / 2;
    let y = area.y as i64 + i64::from(WORK_AREA_MARGIN) + (avail_h - outer_h).max(0) / 2;
    let clamp_pos = |v: i64| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    WindowGeom {
        width,
        height,
        x: clamp_pos(x),
        y: clamp_pos(y),
    }
}

impl App {
    /// 记录启动失败详情：已有文档打开错误时保留（用户更关心打开了什么），追加窗口侧诊断。
    fn record_startup_error(&mut self, detail: String) {
        self.window_state.startup_error = Some(match self.window_state.startup_error.take() {
            Some(existing) => format!("{existing}\n{detail}"),
            None => detail,
        });
    }

    /// 以隐藏状态创建窗口并完成 surface 准备，随后立即进入启动首帧（`begin_startup`）。
    pub(crate) fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Option<WindowId> {
        if self.window_state.window.is_some() {
            return None;
        }
        // 窗口几何（3d）：读回上次退出时的逻辑尺寸 + 物理坐标，先按当前显示器集合夹取，
        // 避免显示器拔出/分辨率改变后窗口落在屏外。无几何记录时把默认 900×700
        // 收敛到当前主显示器工作区（150% 小屏 + 任务栏下不再被遮挡）并居中。
        let monitor_rect = |m: &winit::monitor::MonitorHandle| {
            let pos = m.position();
            let size = m.size();
            (
                ScreenRect {
                    x: pos.x,
                    y: pos.y,
                    w: size.width,
                    h: size.height,
                },
                m.scale_factor() as f32,
            )
        };
        let mut monitors: Vec<(ScreenRect, f32)> = event_loop
            .available_monitors()
            .map(|m| monitor_rect(&m))
            .collect();
        // 工作区来自系统主显示器：把主显示器排到首位，保证工作区与其缩放同源。
        let primary = event_loop.primary_monitor().map(|m| monitor_rect(&m));
        if let Some((primary_rect, _)) = primary {
            if let Some(i) = monitors.iter().position(|(rect, _)| *rect == primary_rect) {
                monitors.swap(0, i);
            }
        }
        let screens: Vec<ScreenRect> = monitors.iter().map(|(s, _)| *s).collect();
        let saved_geom = self.config.window.map(|g| clamp_window_geom(g, &screens));
        // 工作区只服务「无保存几何」的默认尺寸与居中：有保存几何时那次系统查询是纯浪费，
        // 因此把查询与默认几何一起按需计算（几何结果与原来完全一致）。
        let mut geom = saved_geom.unwrap_or_else(|| {
            let work_area = match (primary, monitors.first()) {
                (Some((primary_rect, primary_scale)), Some((first, _)))
                    if *first == primary_rect =>
                {
                    system_work_area().map(|area| (area, primary_scale))
                }
                _ => None,
            };
            fit_initial_window(DEFAULT_WIN_W, DEFAULT_WIN_H, &monitors, work_area)
        });
        // 历史几何可能记录过更小的尺寸：最小尺寸对恢复路径同样是硬约束。
        geom.width = geom.width.max(MIN_WINDOW_SIZE.0 as u32);
        geom.height = geom.height.max(MIN_WINDOW_SIZE.1 as u32);
        let attrs = Window::default_attributes()
            .with_title(self.title_text())
            .with_inner_size(LogicalSize::new(geom.width as f32, geom.height as f32))
            .with_min_inner_size(LogicalSize::new(MIN_WINDOW_SIZE.0, MIN_WINDOW_SIZE.1))
            .with_position(PhysicalPosition::new(geom.x, geom.y))
            // 3e 修 A1：exe 内嵌 ICO 只喂任务栏/文件管理器，标题栏图标须显式设置
            .with_window_icon(Some(
                Icon::from_rgba(crate::icon::icon_rgba(), 32, 32).expect("32×32 RGBA 尺寸恒合法"),
            ))
            // 首帧完整性（Task 5）：首帧准备好之前不显示，避免露出黑色未绘制区域
            .with_visible(false);
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                let msg = tf(Key::FatalWindowCreateDetail, &[&e.to_string()]);
                eprintln!("{msg}");
                self.record_startup_error(msg);
                return None;
            }
        };
        window.set_ime_allowed(true);
        self.input_state.set_dpi(window.scale_factor() as f32);
        crate::diag::set_dpi_scale(self.input_state.scale);
        let context = match Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                let msg = tf(Key::FatalWindowRenderDetail, &[&e.to_string()]);
                eprintln!("{msg}");
                self.record_startup_error(msg);
                return None;
            }
        };
        self.window_state.surface_size = None;
        self.window_state.surface = Surface::new(&context, window.clone()).ok();
        self.set_ime_area();
        self.window_state.window = Some(window);
        self.apply_window_effects();
        // 动效开关（S6）：系统「显示动画」关闭时悬停高亮立即到位。
        let animations = self.ports.system.animations_enabled();
        self.set_animations(animations);
        // 隐藏窗口不会收到首次 Resized：必须在首次 paint 前显式设置 surface 尺寸，
        // 否则 softbuffer 的 buffer_mut() 会 panic（Must set size of surface...）。
        self.sync_surface_to_client();
        let id = self.window_state.window.as_ref().map(|w| w.id());
        crate::diag::perf_stage("window-created");
        if id.is_some() {
            self.begin_startup();
        }
        id
    }

    /// 把 surface 尺寸显式同步到当前客户区（隐藏窗口收不到首次 Resized）。
    /// 尺寸取 `win_size()`：恒非零且与 `render` 产出的帧一致，避免「未 resize 就
    /// buffer_mut()」的启动 panic。resize 失败时保持未知尺寸，交由 present 的
    /// 尺寸检查与 1s 兜底处理，不在这里 panic。
    fn sync_surface_to_client(&mut self) {
        let (w, h) = self.win_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else {
            return;
        };
        let Some(surface) = self.window_state.surface.as_mut() else {
            return;
        };
        if surface.resize(w, h).is_ok() {
            self.window_state.surface_size = Some((w.get(), h.get()));
        }
    }

    /// 进入首帧隐藏阶段并立即尝试显示。
    pub(crate) fn begin_startup(&mut self) {
        self.window_state.startup = Some(StartupGate::new(Instant::now()));
        self.try_show_first_frame();
    }

    /// 在当前客户区尺寸上完成一次同步首帧：先合并待处理尺寸变化，再 render + present。
    /// 呈现成功即显示；`present` 失败时保持隐藏，等 Resized/DPI 事件重试，超时兜底。
    /// surface 尺寸已在 `create_window` 中显式同步；`present` 仍会在 buffer_mut 前
    /// 复核尺寸（隐藏窗口/DPI 变化后不依赖 Resized 一定到达）。
    pub(crate) fn try_show_first_frame(&mut self) {
        if self.window_state.startup.is_none() {
            return;
        }
        // 呈现面不可用（softbuffer 创建/取缓冲失败）：无从「准备好首帧」，
        // 直接显示并由窗口类背景画刷（主题底色）兜底，避免白等 1 秒。
        if self.window_state.surface.is_none() {
            self.settle_startup(true, Instant::now());
            return;
        }
        self.apply_pending_resize();
        crate::diag::perf_stage("first-render-start");
        let _ = self.render();
        let presented = self.present();
        self.settle_startup(presented, Instant::now());
    }

    /// 首帧状态机推进：成功立即显示；超时（无论成败）兜底显示。显示只执行一次。
    fn settle_startup(&mut self, presented: bool, now: Instant) {
        let Some(gate) = self.window_state.startup.as_mut() else {
            return;
        };
        // 只有「首次 present 成功」才算真实首帧；超时兜底显示不打印首帧埋点。
        let first_present = gate.on_present(presented);
        if !(first_present || gate.on_timeout(now)) {
            return;
        }
        self.window_state.startup = None;
        if first_present {
            crate::diag::perf_trace_first_frame();
            crate::diag::perf_stage("first-present");
        }
        if let Some(window) = &self.window_state.window {
            window.set_visible(true);
            window.request_redraw();
        }
        // 首帧已呈现（或超时兜底显示）：执行推迟的非关键启动工作。
        // 放在 set_visible 之后，保证上屏路径不被会话落盘/监听线程创建耽误。
        self.flush_deferred_startup();
    }

    /// 执行推迟到首帧之后的启动工作（最近文件登记 + 文件监听）。幂等，只执行一次。
    /// 语义与 `App::new` 中原先的就地调用完全一致，仅时间点更晚。
    pub(crate) fn flush_deferred_startup(&mut self) {
        if let Some(path) = self.deferred_startup.note_opened.take() {
            self.note_opened(path);
        }
        if std::mem::take(&mut self.deferred_startup.watch) {
            self.watch_current();
        }
    }

    pub(crate) fn handle_window_event(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                // 关窗只是「请求」：脏文档先挂起模态，获准后由 exit_requested 统一退出
                self.confirm_close();
            }
            WindowEvent::Resized(size) => {
                // 重建延迟到下一帧合并执行（见 `apply_pending_resize`）：
                // 事件风暴期间只置位，避免逐事件全量重建。
                // 0×0 尺寸（最小化）也要置位，但跳过 surface.resize 和 request_redraw，
                // 避免让 softbuffer 处理非法尺寸、同时防止 RedrawRequested 驱动
                // 一个仍沿用旧 frame 的 render → scroll_thumb 在 vp.h=1.0 时 panic。
                self.window_state.resize_pending = true;
                // 几何记录（3d）：最大化/最小化（0×0）/非法缩放时的尺寸不是用户想要的大小，
                // 不更新已存几何，只标记待保存；拖动/缩放期间不写盘。
                if size.width == 0 || size.height == 0 {
                    self.window_state.visible = false;
                    return;
                }
                let maximized = self
                    .window_state
                    .window
                    .as_ref()
                    .is_some_and(|w| w.is_maximized());
                if !maximized {
                    if let Some((width, height)) = self.logical_window_size_of() {
                        let (x, y) = self
                            .window_state
                            .geom
                            .or(self.config.window)
                            .map_or((0, 0), |g| (g.x, g.y));
                        self.window_state.geom = Some(WindowGeom {
                            width,
                            height,
                            x,
                            y,
                        });
                        self.window_state.geom_dirty = true;
                    }
                }
                if let (Some(s), Some(w), Some(h)) = (
                    self.window_state.surface.as_mut(),
                    NonZeroU32::new(size.width),
                    NonZeroU32::new(size.height),
                ) {
                    // 只有 resize 成功才记录尺寸：present 以该记录判断能否直接
                    // buffer_mut，失败时保持旧记录，由 present 再做一次 resize。
                    if s.resize(w, h).is_ok() {
                        self.window_state.surface_size = Some((size.width, size.height));
                    }
                }
                if !self.window_state.visible {
                    self.window_state.visible = true;
                }
                if self.window_state.startup.is_some() {
                    // 隐藏阶段：立即在新客户区尺寸上重做首帧，避免旧尺寸帧进入新客户区
                    self.try_show_first_frame();
                } else {
                    self.request_redraw();
                }
            }
            // 窗口移动（3d）：只记录物理坐标，尺寸沿用已存值（无记录时取当前逻辑尺寸），
            // 置脏位待退出时统一落盘。最小化时 Windows 会报 -32000 哨兵坐标，
            // 必须跳过，否则持久化会覆盖可用位置（DEF-T10-E1-001）。
            WindowEvent::Moved(pos) => {
                let minimized = self
                    .window_state
                    .window
                    .as_ref()
                    .is_some_and(|w| w.is_minimized() == Some(true));
                self.on_window_moved(pos.x, pos.y, minimized);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(w) = &self.window_state.window {
                    self.input_state.set_dpi(w.scale_factor() as f32);
                }
                crate::diag::set_dpi_scale(self.input_state.scale);
                // DPI 变化：键中的 scale 量化值变化使全部按 scale 生成的缓存失配；
                // chrome 标签按旧 scale 光栅化，仍需显式作废
                self.ui_state.chrome.clear_label_cache();
                // scale 变了，几何累计均按新 scale：索引重建、可见块按新 DPI 重排
                self.refresh_index();
                self.ensure_layouts_visible();
                self.ensure_math_visible();
                self.ensure_tables_visible();
                self.recompute_geometry();
                if self.window_state.startup.is_some() {
                    // 启动期间 DPI 变化同样按新客户区重做首帧
                    self.try_show_first_frame();
                } else {
                    self.request_redraw();
                }
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => self.ime_preedit(text, cursor),
            WindowEvent::Ime(Ime::Commit(text)) => self.ime_commit(text),
            WindowEvent::Ime(Ime::Disabled) => self.ime_disabled(),
            // 拖放（阶段 1）：图片插入光标处；其余文件按文档打开
            WindowEvent::DroppedFile(path) => self.drop_file(path),
            WindowEvent::HoveredFile(path) => self.hover_file(Some(path)),
            WindowEvent::HoveredFileCancelled => self.hover_file(None),
            // `on_key` 经 `route_key_with_focus` 路由：无浮层时持续记录正文焦点，
            // Esc 从浮层回到正文时归还，与测试走同一入口。
            WindowEvent::KeyboardInput { event, .. } => self.on_key(&event),
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.input_state.mods = Mods {
                    ctrl: s.control_key(),
                    shift: s.shift_key(),
                    alt: s.alt_key(),
                };
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input_state.mouse = position;
                // 复制成功反馈只在该按钮上保留：移开即复原（提示超时也会清）。
                if self.window_state.code_copied.is_some()
                    && self.code_copy_hit() != self.window_state.code_copied
                {
                    self.window_state.code_copied = None;
                    self.request_redraw();
                }
                // 模态打开时它是唯一输入接收者：只更新悬停，不再触达其下浮层/正文
                if self.ui_state.modal.is_some() {
                    let m = (position.x as f32, position.y as f32);
                    let l = self.modal_layout();
                    let changed = self
                        .ui_state
                        .modal
                        .as_mut()
                        .is_some_and(|a| a.state.on_hover(m, &l));
                    if changed {
                        self.request_redraw();
                    }
                    self.apply_pointer();
                    return;
                }
                if self.ui_state.tables.drag.is_some() {
                    // 表格列宽拖动（B2）：视图态即时重排，不触碰文档
                    self.drag_col_to();
                } else if let Some(off) = self.ui_state.chrome.scroll_drag {
                    // 拖动滚动条：按滑块顶边换算滚动量
                    let vp = self.viewport();
                    let doc_h = self.doc_height();
                    self.window_state.goal_y = chrome::scroll_from_thumb(
                        vp,
                        doc_h,
                        self.input_state.mouse.y as f32 - off,
                        self.input_state.scale,
                    );
                    self.window_state.scroll_y = self.window_state.goal_y;
                    self.set_ime_area();
                    self.request_redraw();
                } else if self.window_state.dragging {
                    self.point_caret(true); // 拖选：扩选跟随
                } else {
                    self.update_hover();
                }
                self.apply_pointer();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                // 「退出」已不再是点击的副作用：模态结论会写 exit_requested
                self.on_left_click();
                // 鼠标改光标后刷新快照，否则 Esc 归还的是上一次键盘位置。
                if !self.keyboard_overlay_open() {
                    self.ui_state.focus_return = Some(self.doc_focus());
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
                self.ui_state.search.pressed = None;
                self.end_col_drag();
                // 拖选结束：选区已定型，刷新快照供随后的 Esc 归还。
                if !self.keyboard_overlay_open() {
                    self.ui_state.focus_return = Some(self.doc_focus());
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // 图片放大预览：滚轮只服务预览（普通=平移，Ctrl=缩放），不滚动背景文档。
                if self.ui_state.image_preview.is_some() {
                    self.image_preview_wheel(delta);
                    return;
                }
                // 模态/浮层打开时滚轮只服务浮层，绝不缩放背景文档（DEF-T07-003）。
                let overlay_open = self.overlay_blocks_background_wheel();
                if self.input_state.mods.ctrl {
                    if overlay_open {
                        return;
                    }
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
                // 滚轮改滚动位置后刷新快照，保证 Esc 归还的是用户最后看到的位置。
                if !self.keyboard_overlay_open() {
                    self.ui_state.focus_return = Some(self.doc_focus());
                }
            }
            WindowEvent::RedrawRequested => {
                // 先合并执行待处理尺寸变化，再绘制（一帧一次）
                self.apply_pending_resize();
                // 悬停过渡推进到当前时刻（S6）：绘制侧只读 alpha。
                self.animate(Instant::now());
                if self.window_state.startup.is_some() {
                    // 隐藏阶段的首帧：成功即显示，失败保持隐藏等兜底
                    self.try_show_first_frame();
                    return;
                }
                self.paint();
            }
            // 窗口重新获得焦点：立即校验系统外观（限频轮询最多滞后 1s，此处消除等待）
            WindowEvent::Focused(focused) => {
                // 光标焦点表达：失焦改空心框，仅在状态变化时补一次重绘（无定时器）
                self.set_window_focused(focused);
                self.on_window_focus(focused);
                if !focused {
                    return;
                }
                if !self.window_state.visible {
                    self.window_state.visible = true;
                    self.request_redraw();
                }
                self.input_state.last_theme_check = None;
                self.poll_system_theme(Instant::now());
            }
            // 窗口被系统遮挡：同样视为不可见，防止 paint 在被遮挡窗口上渲染无效帧
            WindowEvent::Occluded(occluded) => {
                self.window_state.visible = !occluded;
                if !occluded {
                    self.request_redraw();
                }
            }
            _ => {}
        }
    }

    /// 窗口焦点变化：失焦即隐藏并清理列宽拖动会话（DEF-T07-004），回焦恢复可见。
    /// 抽成方法便于无事件循环的回归测试直接驱动。
    pub(crate) fn on_window_focus(&mut self, focused: bool) {
        if !focused {
            self.window_state.visible = false;
            self.end_col_drag();
        }
    }

    /// 模态/浮层打开时背景滚轮（含 Ctrl 缩放）必须被拦截（DEF-T07-003）。
    pub(crate) fn overlay_blocks_background_wheel(&self) -> bool {
        self.ui_state.settings.is_some()
            || self.ui_state.modal.is_some()
            || self.ui_state.quick.open
            || self.ui_state.search.open
            || self.ui_state.history.open
    }

    /// 当前是否有独占键盘的浮层（含未进入 `Mode` 的模态/设置/图片预览）。
    fn keyboard_overlay_open(&self) -> bool {
        self.ui_state.modal.is_some()
            || self.ui_state.settings.is_some()
            || self.ui_state.image_preview.is_some()
            || self.mode() != Mode::Document
    }

    /// 正文焦点快照：caret/选区/滚动同源读取，避免三处各写一遍。
    fn doc_focus(&self) -> DocFocus {
        DocFocus {
            caret: self.editor.caret(),
            selection: self.editor.selection(),
            scroll: self.window_state.scroll_y,
        }
    }

    /// 键盘路由前：无浮层时刷新快照（浮层内不得覆盖，否则归还的是浮层内位置）。
    fn begin_keyboard_route(&mut self) -> bool {
        let open = self.keyboard_overlay_open();
        if !open {
            self.ui_state.focus_return = Some(self.doc_focus());
        }
        open
    }

    /// 键盘路由后：仅当「Esc 关闭了浮层」才归还正文焦点与插入点。
    /// 不做无差别归还：回车接受快速打开会换文档，此时保留新文档的焦点。
    fn end_keyboard_route(&mut self, was_open: bool, escape: bool) {
        let open = self.keyboard_overlay_open();
        if escape && was_open && !open {
            if let Some(focus) = self.ui_state.focus_return.take() {
                self.restore_doc_focus(focus);
            }
        }
        if !open {
            self.ui_state.focus_return = Some(self.doc_focus());
        }
    }

    /// 归还正文焦点：光标/选区/滚动回到打开浮层前的位置。
    /// 浮层内若改写文本（替换/恢复），失效偏移保持原样（尽力而为，不 panic）。
    fn restore_doc_focus(&mut self, focus: DocFocus) {
        let len = self.editor.session().text_len();
        let _restored = match focus.selection.filter(|r| r.end <= len) {
            Some(range) => {
                self.editor.set_caret(range.start) && self.editor.set_caret_extend(range.end)
            }
            None if focus.caret <= len => self.editor.set_caret(focus.caret),
            None => false,
        };
        // 偏移失效（浮层内改写文本）时光标保持原样，滚动仍归还到打开前位置。
        let vp = self.viewport();
        let max = chrome::max_scroll(vp, self.doc_height());
        let y = focus.scroll.clamp(0.0, max);
        self.window_state.goal_y = y;
        self.window_state.scroll_y = y;
        self.set_ime_area();
        self.request_redraw();
    }

    /// 单个内部键的路由入口（测试与事件循环共用语义）：包装 `route_key`，
    /// 让纯键盘路径也拥有与真实键盘一致的焦点记录与 Esc 归还。
    pub(crate) fn route_key_with_focus(&mut self, key: InputKey) {
        let escape = key == InputKey::Escape;
        let was_open = self.begin_keyboard_route();
        self.route_key(key);
        self.end_keyboard_route(was_open, escape);
    }

    /// 保存当前窗口几何并报告是否应关闭。Shell 据此只移除一个 WindowId。
    pub(crate) fn finish_close(&mut self) -> bool {
        if !self.window_state.exit_requested {
            return false;
        }
        if self.window_state.geom_dirty {
            let maximized = self
                .window_state
                .window
                .as_ref()
                .is_some_and(|w| w.is_maximized());
            if !maximized {
                if let Some(g) = self.window_state.geom {
                    self.config.window = Some(g);
                }
            }
            self.save_config(ConfigSave::Window);
            self.window_state.geom_dirty = false;
        }
        true
    }

    /// 取外部变更事件、校验系统外观、尝试自动保存与恢复写盘，并返回下次唤醒时刻。
    /// 「恢复/自动保存静默期终点」「监听轮询」「系统外观轮询」的较早者
    /// （干净草稿且显式主题下完全休眠）。
    pub(crate) fn next_wake(&mut self) -> Option<Instant> {
        // 窗口不可见时跳过所有周期性工作——恢复路径由 Resized/Focused/Occluded 主动 request_redraw。
        // 唯一例外是首帧隐藏阶段：必须保留一次唤醒用于兜底显示，否则窗口可能永久不可见。
        if !self.window_state.visible && self.window_state.startup.is_none() {
            return None;
        }
        let now = Instant::now();
        self.tick(now);
        // 恢复写盘（0b）：脏文档（含无路径草稿/冲突挂起）都按静默期调度——
        // 它是比自动保存更宽的最后防线
        let dirty_editing = self.editor.is_dirty() && self.editor.preedit().is_none();
        let mut next = if dirty_editing {
            let autosave_at = self
                .persist_state
                .autosave
                .deadline(self.persist_state.persist.activity());
            // 恢复快照有独立期限（最后一次真实编辑），不受自动保存失败重试影响。
            let recovery_at = self
                .persist_state
                .autosave
                .deadline(self.persist_state.persist.recovery_activity());
            match (autosave_at, recovery_at) {
                (Some(a), Some(r)) => Some(a.min(r)),
                (a, r) => a.or(r),
            }
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
        // 远程图片请求未回：周期轮询结果；全部取完即停止唤醒。
        if !self.cache.remote_pending.is_empty() {
            let remote_at = now + REMOTE_POLL;
            next = Some(next.map_or(remote_at, |t| t.min(remote_at)));
        }
        // 光标闪烁：以「最近一次翻转/活跃重置」时刻为基准算下一次到期点。
        // 若改用 now + CURSOR_BLINK，活跃信号（note_caret_activity）刚重置的周期
        // 会被推到「当前 + 600ms」，与 tick 实际起算的 last_blink 错位，翻转时刻抖动。
        // 历史面板打开时 tick 会提前返回、不会推进闪烁，此时不登记唤醒，避免
        // 陈旧的 last_blink 造成到期点已过 → 事件循环空转。
        if self.blink_active() && !self.ui_state.history.open {
            let blink_at = self
                .last_blink
                .map_or(now + CURSOR_BLINK, |t| (t + CURSOR_BLINK).max(now));
            next = Some(next.map_or(blink_at, |t| t.min(blink_at)));
        }
        // 悬停过渡（S6）：按 ~60fps 推进；帧点未到只登记唤醒，不空转重绘。
        if self.anim_running() {
            let due = self
                .window_state
                .anim_last_frame
                .map_or(now, |t| t + crate::anim::FRAME);
            if now >= due {
                self.window_state.anim_last_frame = Some(now);
                self.request_redraw();
                let frame_at = now + crate::anim::FRAME;
                next = Some(next.map_or(frame_at, |t| t.min(frame_at)));
            } else {
                next = Some(next.map_or(due, |t| t.min(due)));
            }
        }
        // 首帧隐藏阶段的保护时限：tick 已在此刻之前尝试过兜底显示，仍挂起则登记唤醒。
        if let Some(gate) = &self.window_state.startup {
            let at = gate.deadline();
            next = Some(next.map_or(at, |t| t.min(at)));
        }
        next
    }
}

/// 源码模式纯文本行视图的合成块 id 起点：与文档真实 BlockId（自 0 递增）隔离，
/// 保证模式切换时布局缓存不会串键。
const SOURCE_PLAIN_ID_BASE: u64 = 1 << 40;

impl App {
    /// 源码模式纯文本行视图是否生效：源码模式 && （配置开 或 灰度环境变量强制）。
    pub(crate) fn source_plain_active(&self) -> bool {
        self.source_mode() && self.config.source_plain_lines_enabled()
    }

    /// 当前块索引是否已替换为逐行合成块。
    fn index_is_source_plain(&self) -> bool {
        self.index
            .blocks
            .first()
            .is_some_and(|g| g.id.0 >= SOURCE_PLAIN_ID_BASE)
    }

    /// 让块索引与当前视图模型一致（每次排版前调用）：
    /// - 开关生效且索引还不是逐行块 → 按 `SourceLayout` 重建；
    /// - 开关不生效但索引仍是逐行块（打印恢复/运行时关开关）→ 恢复真实块索引。
    pub(crate) fn sync_source_plain_index(&mut self) {
        if self.source_plain_active() {
            if !self.index_is_source_plain() {
                self.rebuild_source_plain_index();
            }
        } else if self.index_is_source_plain() {
            self.refresh_index();
        }
    }

    /// 用 `SourceLayout` 的每行内容区间生成「一行一块」的合成块几何。
    ///
    /// 块种类取 `Paragraph`（源码模式 `reveal_all` 下不做任何块级替换），行尾换行
    /// 不会进入行块区间；空行区块区间为零长，但仍按正文行高占位，因而可停光标、
    /// 可选中、可删除。块间段距沿用段落间距，整个视图保持等距行节奏。
    fn rebuild_source_plain_index(&mut self) {
        let text = self.editor.session().serialize();
        let layout = SourceLayout::new(&text);
        let scale = self.input_state.scale;
        let line_h = (self.engine.doc_line_height() * scale).max(1.0);
        let mut blocks = Vec::with_capacity(layout.line_count());
        for line in 0..layout.line_count() {
            let (start, end) = layout.line_range(line).unwrap_or((0, 0));
            blocks.push(crate::document::block_index::BlockGeom {
                id: BlockId(SOURCE_PLAIN_ID_BASE + line as u64),
                kind: BlockKind::Paragraph,
                range: Range::new(start, end),
                // 行号恒为 0：几何累计不会把真实行号差当成空行再补一遍高度。
                start_line: 0,
                end_line: 0,
                top: 0.0,
                height: 0.0,
                est_h: line_h,
                words: 0,
                container: BlockContainer::default(),
                details_parent: None,
                details_hidden: false,
                toc_marker: false,
                toc_list: false,
                frontmatter_like: false,
            });
        }
        let count = blocks.len();
        self.index.blocks = blocks;
        self.index.incremental_pos = None;
        // 行数变少时回收越界合成 id 的布局/光栅（其余键不变，正确性不依赖回收）。
        self.cache.layouts.retain(|id, _| {
            id.0 < SOURCE_PLAIN_ID_BASE || id.0 - SOURCE_PLAIN_ID_BASE < count as u64
        });
        self.cache.rasters.retain(|id, _| {
            id.0 < SOURCE_PLAIN_ID_BASE || id.0 - SOURCE_PLAIN_ID_BASE < count as u64
        });
        self.recompute_geometry();
    }
}
