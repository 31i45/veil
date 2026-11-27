//! 跨域行为测试：以 headless App 驱动真实编辑/排版/绘制/持久化路径。

use crate::app::*;
use crate::chrome;
use crate::chrome::MenuAction;
use crate::docio::encode;
use crate::document::block_index::{gap_after, gap_before, gap_between, BlockGeom};
use crate::document::recovery;
use crate::document::ExternalState;
use crate::editor::Intent;
use crate::geometry::{ScreenRect, BASE_LINE_HEIGHT, MARGIN, PARA_GAP};
use crate::i18n::{t, Key as I18nKey};
use crate::image::{AssetMigration, AssetStore, ImageData, ImageLoader};
use crate::input::{Action, Key, Mods};
use crate::interaction::map_key_parts;
use crate::math::{MathImage, MathRenderer};
use crate::mermaid_renderer::{MermaidImage, MermaidRenderer};
use crate::mode::{Mode, TextSink};
use crate::paint::{
    can_present, fit_size, local_image_path, present_frame, surface_step, PresentSink, SurfaceStep,
};

use crate::print::{NoPrint, Print, PrintSession};
use crate::theme::{
    self, ConfigStore, ExportStyle, NativeWindow, SystemTheme, ThemeEnv, ThemeMode, WindowEffects,
};

use crate::view::ctx_menu;
use crate::view::outline;
use crate::view::search;
use crate::view::{Frame, InlineAsset, Memo, MAX_BLOCK_BYTES};
use crate::Ports;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use veil_core::{BlockId, BlockKind, InlineKind, Range};
use winit::dpi::PhysicalPosition;
use winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::keyboard::Key as WKey;
use winit::keyboard::{KeyCode, NamedKey, PhysicalKey};

struct NoClip;
impl Clipboard for NoClip {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _text: &str) {}
}

/// 默认 URL 打开端口：成功且无副作用（既有测试不关心「打开链接」）。
struct NoUrl;
impl UrlOpener for NoUrl {
    fn open_url(&mut self, _url: &str) -> Result<(), String> {
        Ok(())
    }
}

pub(crate) fn no_url() -> Box<dyn UrlOpener> {
    Box::new(NoUrl)
}

/// 固定时钟：状态栏「已保存 HH:MM」的断言全部基于此值，不依赖真实系统时间。
struct FixedClock;
impl Clock for FixedClock {
    fn local_hhmm(&self) -> String {
        "14:32".into()
    }
}

fn fixed_clock() -> Box<dyn Clock> {
    Box::new(FixedClock)
}

/// 记录窗口效果调用：断言主题切换后 `set_background` 收到当前主题底色。
#[derive(Clone, Default)]
struct RecordingEffects {
    backgrounds: Rc<RefCell<Vec<[u8; 3]>>>,
}

impl WindowEffects for RecordingEffects {
    fn apply(&mut self, _window: NativeWindow, _dark: bool) {}
    fn set_background(&mut self, _window: NativeWindow, color: [u8; 3]) {
        self.backgrounds.borrow_mut().push(color);
    }
}

/// 可编排的 URL 打开端口：记录每次调用的 URL，结果由 `result` 决定（校验失败注入）。
#[derive(Clone)]
struct FakeUrl {
    calls: Rc<RefCell<Vec<String>>>,
    result: Result<(), String>,
}

impl UrlOpener for FakeUrl {
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        self.calls.borrow_mut().push(url.to_string());
        self.result.clone()
    }
}

// ---- 0b 持久化 mock：崩溃恢复槽 + 会话/最近文件 ----

/// 恢复槽 mock：内存单槽；fail_save/fail_clear 注入写失败（Rc 句柄可在 App 构造后翻转）；
/// 调用记录供断言。
#[derive(Clone, Default)]
struct FakeRecovery {
    slot: Rc<RefCell<Option<Vec<u8>>>>,
    calls: Rc<RefCell<Vec<String>>>,
    fail_save: Rc<Cell<bool>>,
    fail_clear: Rc<Cell<bool>>,
}

impl RecoveryStore for FakeRecovery {
    fn load(&mut self) -> Result<Vec<u8>, String> {
        self.calls.borrow_mut().push("load".into());
        self.slot
            .borrow()
            .clone()
            .ok_or_else(|| "无恢复文件".into())
    }
    fn save(&mut self, data: &[u8]) -> Result<(), String> {
        self.calls.borrow_mut().push("save".into());
        if self.fail_save.get() {
            return Err("模拟恢复写入失败".into());
        }
        *self.slot.borrow_mut() = Some(data.to_vec());
        Ok(())
    }
    fn clear(&mut self) -> Result<(), String> {
        self.calls.borrow_mut().push("clear".into());
        if self.fail_clear.get() {
            return Err("模拟恢复清理失败".into());
        }
        *self.slot.borrow_mut() = None;
        Ok(())
    }
}

/// 会话 mock：内存文本；load 无内容时 Err（首次运行正常态）。
#[derive(Clone, Default)]
struct FakeSession {
    text: Rc<RefCell<Option<String>>>,
}

impl SessionStore for FakeSession {
    fn load(&mut self) -> Result<String, String> {
        self.text
            .borrow()
            .clone()
            .ok_or_else(|| "无会话文件".into())
    }
    fn save(&mut self, text: &str) -> Result<(), String> {
        *self.text.borrow_mut() = Some(text.to_string());
        Ok(())
    }
}

/// 既有测试的默认持久化环境：空恢复槽 + 空会话 + 空快照。
fn persist_env() -> PersistEnv {
    PersistEnv {
        recovery: Box::new(FakeRecovery::default()),
        session: Box::new(FakeSession::default()),
        snapshot: Box::new(FakeSnapshot::default()),
    }
}

/// 0b 测试专用装配：恢复槽/会话可由测试预置并持有句柄断言。
fn persist_env_with(recovery: FakeRecovery, session: FakeSession) -> PersistEnv {
    PersistEnv {
        recovery: Box::new(recovery),
        session: Box::new(session),
        snapshot: Box::new(FakeSnapshot::default()),
    }
}

/// 内存快照库：按文档键分槽，供版本历史测试注入并断言。
#[derive(Clone, Default)]
struct FakeSnapshot {
    slots: Rc<RefCell<Vec<(u64, SnapshotMeta, String)>>>,
    fail_list: Rc<Cell<bool>>,
    /// 注入写入失败：验证「失败不静默」（状态栏可见）且正文/磁盘照常保存。
    fail_save: Rc<Cell<bool>>,
    /// 注入历史迁移失败：返回 Err 且不移动任何快照（验证失败可见 + 旧历史保留）。
    fail_migrate: Rc<Cell<bool>>,
}

impl FakeSnapshot {
    fn texts(&self, key: u64) -> Vec<String> {
        let mut v: Vec<(SnapshotMeta, String)> = self
            .slots
            .borrow()
            .iter()
            .filter(|(k, _, _)| *k == key)
            .map(|(_, m, t)| (*m, t.clone()))
            .collect();
        v.sort_by_key(|a| std::cmp::Reverse(a.0.ts_ms));
        v.into_iter().map(|(_, t)| t).collect()
    }
}

impl SnapshotStore for FakeSnapshot {
    fn save_snapshot(&mut self, key: u64, text: &str) -> Result<SnapshotMeta, String> {
        if self.fail_save.get() {
            return Err("模拟版本历史写入失败".into());
        }
        let hash = content_hash(text.as_bytes());
        if let Some((_, m, _)) = self
            .slots
            .borrow()
            .iter()
            .find(|(k, m, _)| *k == key && m.hash == hash)
        {
            return Ok(*m);
        }
        let ts = self
            .slots
            .borrow()
            .iter()
            .map(|(_, m, _)| m.ts_ms)
            .max()
            .unwrap_or(0)
            + 1;
        let meta = SnapshotMeta {
            ts_ms: ts,
            hash,
            bytes: text.len() as u64,
        };
        self.slots.borrow_mut().push((key, meta, text.to_string()));
        Ok(meta)
    }

    fn list_snapshots(&mut self, key: u64) -> Result<Vec<SnapshotMeta>, String> {
        if self.fail_list.get() {
            return Err("模拟读取失败".into());
        }
        let mut v: Vec<SnapshotMeta> = self
            .slots
            .borrow()
            .iter()
            .filter(|(k, _, _)| *k == key)
            .map(|(_, m, _)| *m)
            .collect();
        v.sort_by_key(|m| std::cmp::Reverse(m.ts_ms));
        Ok(v)
    }

    fn read_snapshot(&mut self, key: u64, ts: u64) -> Result<String, String> {
        self.slots
            .borrow()
            .iter()
            .find(|(k, m, _)| *k == key && m.ts_ms == ts)
            .map(|(_, _, t)| t.clone())
            .ok_or_else(|| "快照不存在".into())
    }

    fn delete_snapshot(&mut self, key: u64, ts: u64) -> Result<(), String> {
        self.slots
            .borrow_mut()
            .retain(|(k, m, _)| !(*k == key && m.ts_ms == ts));
        Ok(())
    }

    fn clear_snapshots(&mut self, key: u64) -> Result<usize, String> {
        let before = self.slots.borrow().len();
        self.slots.borrow_mut().retain(|(k, _, _)| *k != key);
        Ok(before - self.slots.borrow().len())
    }

    fn prune(&mut self, key: u64, keep_newest: usize) -> Result<usize, String> {
        let list = self.list_snapshots(key)?;
        let mut removed = 0;
        for m in list.into_iter().skip(keep_newest) {
            self.delete_snapshot(key, m.ts_ms)?;
            removed += 1;
        }
        Ok(removed)
    }

    fn migrate_key(&mut self, from: u64, to: u64) -> Result<usize, String> {
        if self.fail_migrate.get() {
            return Err("模拟历史迁移失败".into());
        }
        let moved: Vec<_> = self
            .slots
            .borrow()
            .iter()
            .filter(|(k, _, _)| *k == from)
            .cloned()
            .collect();
        let mut count = 0;
        for (_, m, t) in moved {
            let exists = self
                .slots
                .borrow()
                .iter()
                .any(|(k, x, _)| *k == to && x.hash == m.hash);
            if !exists {
                self.slots.borrow_mut().push((to, m, t));
                count += 1;
            }
            self.slots
                .borrow_mut()
                .retain(|(k, x, _)| !(*k == from && x.ts_ms == m.ts_ms));
        }
        Ok(count)
    }
}

/// 不加载任何图片（既有测试的默认注入）：所有图片降级 alt 文本。
struct NoImages;
impl ImageLoader for NoImages {
    fn load(&mut self, _path: &Path) -> Result<ImageData, String> {
        Err("测试未注入图片".into())
    }
}

/// 图片加载 mock：按路径返回自然尺寸位图并记录调用；未知路径 Err。
/// `files` 可共享修改：用于模拟「磁盘图片被替换后重新加载」。
type ImageFiles = Rc<RefCell<HashMap<PathBuf, (u32, u32)>>>;

struct FakeImages {
    files: ImageFiles,
    calls: Rc<RefCell<Vec<PathBuf>>>,
}

impl ImageLoader for FakeImages {
    fn load(&mut self, path: &Path) -> Result<ImageData, String> {
        self.calls.borrow_mut().push(path.to_path_buf());
        match self.files.borrow().get(path) {
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
/// 导出调用记录：(会话序列化, 源路径, 主题样式快照, 调用方指定目标)。
type ExportCalls = Rc<RefCell<Vec<(String, Option<PathBuf>, ExportStyle, Option<PathBuf>)>>>;

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
    image: Option<PathBuf>,
    export: Option<PathBuf>,
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
    fn open_image(&mut self) -> Option<PathBuf> {
        self.calls.borrow_mut().push("open_image".into());
        self.image.clone()
    }
    fn save_export(&mut self, suggested: &str, ext: &str) -> Option<PathBuf> {
        self.calls
            .borrow_mut()
            .push(format!("export:{ext}:{suggested}"));
        self.export.clone()
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

/// 跨目录另存资产迁移 mock：记录 (旧文档, 新文档, 相对 URL 列表)，返回注入结果。
type AssetMigrateCalls = Rc<RefCell<Vec<(PathBuf, PathBuf, Vec<String>)>>>;

#[derive(Clone)]
struct FakeMigrateAssets {
    calls: AssetMigrateCalls,
    report: AssetMigration,
}

impl AssetStore for FakeMigrateAssets {
    fn save_pasted_image(&mut self, _doc: &Path, _image: &ImageData) -> Result<String, String> {
        Err("测试未注入粘贴资产".into())
    }
    fn migrate_relative_assets(
        &mut self,
        old_doc: &Path,
        new_doc: &Path,
        urls: &[String],
    ) -> AssetMigration {
        self.calls
            .borrow_mut()
            .push((old_doc.to_path_buf(), new_doc.to_path_buf(), urls.to_vec()));
        self.report.clone()
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

/// 文件列表剪贴板 mock：模拟资源管理器 Ctrl+C（Windows 同时附带路径文本）。
struct ClipFiles {
    files: Vec<PathBuf>,
    text: Option<String>,
}

impl Clipboard for ClipFiles {
    fn get(&mut self) -> Option<String> {
        self.text.clone()
    }
    fn set(&mut self, _text: &str) {}
    fn get_files(&mut self) -> Vec<PathBuf> {
        self.files.clone()
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
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(Clip {
                html: html.map(str::to_string),
                text: plain.map(str::to_string),
            }),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

pub(crate) fn doc_text(a: &App) -> String {
    let s = a.editor.session();
    s.text_slice(Range::new(0, s.text_len()))
        .expect("全文档区间恒合法")
}

// ---- 富文本粘贴（切片 11）：HTML 优先转 Markdown，无内容回退纯文本 ----

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

/// 测试用 MermaidRenderer 空实现：不做任何渲染，poll 永远返回空。
/// 现有测试大多不涉及 Mermaid 渲染，注入此实现即可通过 Ports 构造。
struct NoopMermaid;
impl MermaidRenderer for NoopMermaid {
    fn request(&mut self, _label: String, _source: String, _dark: bool) {}
    fn poll(&mut self) -> Vec<(String, Result<MermaidImage, String>)> {
        Vec::new()
    }
    fn set_dpi_scale(&mut self, _scale: f32) {}
}

/// 可观测的 MermaidRenderer stub：记录所有 request 调用；poll 从预设队列返回
/// 结果（用于失败计数/超限跳过驱动）。
///
/// 所有内部状态都用 Rc/Cell 包起来，derive Clone 后与 Box<dyn MermaidRenderer> 共享
/// poll() 结果队列（key + 位图/错误）。
type MermaidPollQueue = Rc<RefCell<Vec<(String, Result<MermaidImage, String>)>>>;
/// render_blocking() 预设结果槽。
type MermaidBlockingSlot = Rc<RefCell<Option<Result<MermaidImage, String>>>>;
/// request() 收到的 Mermaid 源码日志。
type MermaidSourceLog = Rc<RefCell<Vec<String>>>;

/// 同一份底层数据——测试可在外层通过 clone 句柄断言 Box 里 Renderer 的真实状态。
#[derive(Clone)]
struct RecordingMermaid {
    /// request() 收到的所有 label（本地渲染的稳定键）
    requests: Rc<RefCell<Vec<String>>>,
    /// request() 收到的源码（校验围栏是否已被剥离）
    sources: MermaidSourceLog,
    /// poll() 返回队列；每次 poll 调 `take` 全部取走
    poll_queue: MermaidPollQueue,
    /// render_blocking() 调用次数（导出/打印前置补齐路径）
    blocking_calls: Rc<Cell<usize>>,
    /// render_blocking() 预设返回值；None 时按 trait 默认语义失败
    blocking_result: MermaidBlockingSlot,
}

impl RecordingMermaid {
    fn new() -> Self {
        Self {
            requests: Rc::new(RefCell::new(Vec::new())),
            sources: Rc::new(RefCell::new(Vec::new())),
            poll_queue: Rc::new(RefCell::new(Vec::new())),
            blocking_calls: Rc::new(Cell::new(0)),
            blocking_result: Rc::new(RefCell::new(None)),
        }
    }
    fn push_err(&self, url: String, msg: &str) {
        self.poll_queue
            .borrow_mut()
            .push((url, Err(msg.to_string())));
    }
    fn request_count(&self) -> usize {
        self.requests.borrow().len()
    }
    fn last_request_url(&self) -> Option<String> {
        self.requests.borrow().last().cloned()
    }
    fn last_request_source(&self) -> Option<String> {
        self.sources.borrow().last().cloned()
    }
    /// 设置 render_blocking() 的返回结果（导出前置补齐测试用）。
    fn set_blocking_ok(&self, img: MermaidImage) {
        *self.blocking_result.borrow_mut() = Some(Ok(img));
    }
    /// render_blocking() 调用次数。
    fn blocking_count(&self) -> usize {
        self.blocking_calls.get()
    }
}

impl MermaidRenderer for RecordingMermaid {
    fn request(&mut self, label: String, source: String, _dark: bool) {
        self.requests.borrow_mut().push(label);
        self.sources.borrow_mut().push(source);
    }
    fn poll(&mut self) -> Vec<(String, Result<MermaidImage, String>)> {
        std::mem::take(&mut *self.poll_queue.borrow_mut())
    }
    fn set_dpi_scale(&mut self, _scale: f32) {}
    fn render_blocking(
        &mut self,
        _label: String,
        _source: String,
        _dark: bool,
    ) -> Result<MermaidImage, String> {
        self.blocking_calls.set(self.blocking_calls.get() + 1);
        self.blocking_result
            .borrow()
            .clone()
            .unwrap_or_else(|| Err("blocking render unsupported".into()))
    }
}

/// 带 RecordingMermaid 端口的 App 装配（Mermaid 稳定性测试专用）。
fn app_with_mermaid(text: &str, mermaid: RecordingMermaid) -> App {
    let doc = Doc {
        text: text.into(),
        path: None,
        bom: false,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(mermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

// ---- Mermaid 本地渲染测试（不联网 / 失败计数 / 超限 notice） ----

/// 默认导出器：不记录、恒失败（不关心导出的既有测试用）。
struct NoExport;
impl Exporter for NoExport {
    fn export_html(
        &mut self,
        _s: &veil_core::Session,
        _style: ExportStyle,
        _p: Option<&Path>,
        _t: Option<&Path>,
        _mermaids: Option<&std::collections::HashMap<veil_core::BlockId, MermaidImage>>,
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
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail,
        calls: calls.clone(),
    };
    let a = App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
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

/// 可见范围内是否仍有键不匹配（因需重排）的块：模拟生产 paint 的两遍收敛判定。
fn visible_layout_missing(a: &App) -> bool {
    let base = a.base_key();
    let caret = a.editor.caret();
    a.index
        .blocks
        .get(a.visible_index_range())
        .is_some_and(|bs| {
            bs.iter().any(|g| {
                a.cache
                    .layout(g.id, &g.kind, g.range, base, caret, a.toc_layout_epoch(g))
                    .is_none()
            })
        })
}

fn app_with_images(
    text: &str,
    path: Option<&str>,
    files: &[(&str, u32, u32)],
) -> (App, Rc<RefCell<Vec<PathBuf>>>, ImageFiles) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let files: ImageFiles = Rc::new(RefCell::new(
        files
            .iter()
            .map(|(p, w, h)| (PathBuf::from(p), (*w, *h)))
            .collect(),
    ));
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let a = App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(FakeImages {
                files: files.clone(),
                calls: calls.clone(),
            }),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    );
    (a, calls, files)
}

/// 取指定文本所在的块布局（测试辅助：按块文本定位，避开 BlockId 细节）。
fn layout_of_text<'a>(a: &'a App, needle: &str) -> &'a crate::view::BlockLayout {
    let session = a.editor.session();
    let (id, _, _) = session
        .blocks()
        .into_iter()
        .find(|(_, _, r)| session.text_slice(*r).is_some_and(|t| t.contains(needle)))
        .expect("应存在目标块");
    &a.cache.layouts[&id]
}

/// 取指定文本所在块的几何（测试辅助：按块文本定位）。
fn geom_of_text<'a>(a: &'a App, needle: &str) -> &'a BlockGeom {
    let session = a.editor.session();
    let (id, _, _) = session
        .blocks()
        .into_iter()
        .find(|(_, _, r)| session.text_slice(*r).is_some_and(|t| t.contains(needle)))
        .expect("应存在目标块");
    a.index
        .blocks
        .iter()
        .find(|g| g.id == id)
        .expect("块几何应存在")
}

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
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let app = App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(ClipImage {
                text: plain.map(str::to_string),
                html: html.map(str::to_string),
                image,
            }),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(FakeAssets {
                result,
                calls: calls.clone(),
            }),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    );
    PasteFixture { app, calls }
}

/// 远程图片 mock 的结果队列句柄：(url, 加载结果)。
type RemoteResults = Rc<RefCell<Vec<(String, Result<ImageData, String>)>>>;

/// 远程图片 mock：记录请求 URL，`poll_remote` 返回测试预置的结果队列。
#[derive(Clone, Default)]
struct RemoteImages {
    requests: Rc<RefCell<Vec<String>>>,
    results: RemoteResults,
}

impl ImageLoader for RemoteImages {
    fn load(&mut self, _path: &Path) -> Result<ImageData, String> {
        Err("测试未注入本地图片".into())
    }
    fn request_remote(&mut self, url: &str) {
        self.requests.borrow_mut().push(url.to_string());
    }
    fn poll_remote(&mut self) -> Vec<(String, Result<ImageData, String>)> {
        std::mem::take(&mut *self.results.borrow_mut())
    }
}

fn app_remote(text: &str, config: Box<dyn ConfigStore>, images: RemoteImages) -> App {
    let doc = Doc {
        text: text.into(),
        path: None,
        bom: false,
        resume: false,
        open_error: None,
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(FakeMath {
                fail: false,
                calls: Rc::new(RefCell::new(Vec::new())),
            }),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(images),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: ThemeEnv {
                config,
                system: Box::new(DynSystem(Rc::new(Cell::new(Some(false))))),
                effects: Box::new(theme::test_support::NoEffects),
                callouts: veil_core::CalloutRegistry::default(),
            },
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

const REMOTE_URL: &str = "https://example.com/a.png";

struct DropFixture {
    app: App,
    images: Rc<RefCell<Vec<PathBuf>>>,
    assets: Rc<RefCell<Vec<(PathBuf, u32, u32)>>>,
}

fn app_drop(
    text: &str,
    path: Option<&str>,
    files: &[(&str, u32, u32)],
    assets: Result<String, String>,
) -> DropFixture {
    let images = Rc::new(RefCell::new(Vec::new()));
    let asset_calls = Rc::new(RefCell::new(Vec::new()));
    let map = files
        .iter()
        .map(|(p, w, h)| (PathBuf::from(*p), (*w, *h)))
        .collect();
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let app = App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(FakeImages {
                files: Rc::new(RefCell::new(map)),
                calls: images.clone(),
            }),
            assets: Box::new(FakeAssets {
                result: assets,
                calls: asset_calls.clone(),
            }),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    );
    DropFixture {
        app,
        images,
        assets: asset_calls,
    }
}

const TABLE_DOC: &str = "p\n\n| a | b |\n| --- | --- |\n| c | d |\n";

const TABLE_RICH_DOC: &str = "p\n\n| **MMMM** | x |\n| --- | --- |\n";

const TABLE_2X3: &str = "p\n\n| a | b | c |\n| --- | --- | --- |\n| d | e | f |\n";

const TABLE_MENU_DOC: &str = "p\n\n| a | b |\n| --- | --- |\n| c | d |\n| e | f |\n";

/// 在表格网格第 cell 个单元格（行优先：表头在前，正文随后）中心右键。
fn right_click_table_cell(a: &mut App, cell: usize) {
    a.ensure_layouts_all();
    a.ensure_tables();
    let id = a
        .editor
        .session()
        .blocks()
        .into_iter()
        .find(|(_, k, _)| *k == BlockKind::Table)
        .map(|(id, _, _)| id)
        .expect("文档应含表格块");
    let top = geom(a, id).top;
    let (x, y) = {
        let c = &a.cache.tables[&id].cells[cell];
        (c.x + c.w / 2.0, c.y + c.h / 2.0)
    };
    let scale = a.input_state.scale;
    let vp = a.viewport();
    a.input_state.mouse = PhysicalPosition::new(
        (MARGIN * scale + x) as f64,
        (vp.y + top - a.window_state.scroll_y + y) as f64,
    );
    a.on_right_click();
}

/// 经右键菜单项集执行一次表格结构操作（光标先落到上下文单元格）。
fn exec_table_ctx(a: &mut App, tc: ctx_menu::TableCtx, action: ctx_menu::CtxAction) {
    let id = a
        .editor
        .session()
        .blocks()
        .into_iter()
        .find(|(_, k, _)| *k == BlockKind::Table)
        .map(|(id, _, _)| id)
        .expect("文档应含表格块");
    let caret = {
        let tbl = a.editor.session().table(id).unwrap();
        match tc.body_row {
            Some(r) => tbl.rows[r][tc.col].range.start,
            None => tbl.header[tc.col].range.start,
        }
    };
    a.editor.set_caret(caret);
    a.ui_state
        .ctx
        .open_at(10.0, 20.0, ctx_menu::CtxKind::Table(tc), None, None);
    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == action)
        .unwrap_or_else(|| panic!("菜单缺少 {action:?}"));
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);
}

#[derive(Clone, Default)]
pub(crate) struct MemStore {
    files: Rc<RefCell<HashMap<PathBuf, Vec<u8>>>>,
    fail_save: bool,
    fail_list: bool,
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
    fn list_files(&mut self, dir: &Path) -> Result<Vec<PathBuf>, String> {
        if self.fail_list {
            return Err("模拟目录读取失败".to_string());
        }
        // 内存实现：目录内容即已登记文件的父目录匹配（打开时必然可 load）
        Ok(self
            .files
            .borrow()
            .keys()
            .filter(|p| p.parent() == Some(dir))
            .cloned()
            .collect())
    }
}

fn app(text: &str, path: Option<&str>, bom: bool, store: &MemStore) -> App {
    app_with(text, path, bom, store, no_url())
}

/// 可替换 URL 打开端口的无路径/有路径装配（链接打开测试用）。
pub(crate) fn ports(store: &MemStore, url_opener: Box<dyn UrlOpener>) -> Ports {
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    Ports {
        ime_anchor: Box::new(NoopImeAnchor),
        clipboard: Box::new(NoClip),
        store: Box::new(store.clone()),
        math: Box::new(m),
        mermaid: Box::new(NoopMermaid),
        images: Box::new(NoImages),
        assets: Box::new(NoAssets),
        exporter: Box::new(NoExport),
        dialogs: Box::new(NoDialogs),
        watcher: Box::new(NoWatcher),
        theme_env: theme::test_support::env(),
        persist_env: persist_env(),
        print: Box::new(NoPrint),
        url_opener,
        clock: fixed_clock(),
    }
}

fn app_with(
    text: &str,
    path: Option<&str>,
    bom: bool,
    store: &MemStore,
    url_opener: Box<dyn UrlOpener>,
) -> App {
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom,
        resume: false,
        open_error: None,
    };
    App::new(doc, ports(store, url_opener))
}

/// 记录型剪贴板：断言复制出的原文（不依赖系统剪贴板）。
#[derive(Clone)]
struct ClipLog(Rc<RefCell<Vec<String>>>);
impl Clipboard for ClipLog {
    fn get(&mut self) -> Option<String> {
        self.0.borrow().last().cloned()
    }
    fn set(&mut self, text: &str) {
        self.0.borrow_mut().push(text.to_string());
    }
}

/// 可观测剪贴板的装配（代码块复制测试）。
fn app_code_copy(text: &str) -> (App, Rc<RefCell<Vec<String>>>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let doc = Doc {
        text: text.into(),
        path: None,
        bom: false,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let app = App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(ClipLog(log.clone())),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    );
    (app, log)
}

/// 启动即失败装配：Doc 带 open_error（空白草稿 + 首帧错误模态），store 决定「重试」结果。
const STARTUP_ERROR_MESSAGE: &str = "无法读取 a.md：文件不存在。\n\n下一步：可重试或打开其他文件。";

fn app_startup_error(path: &str, store: &MemStore) -> App {
    let doc = Doc {
        text: String::new(),
        path: None,
        bom: false,
        resume: false,
        open_error: Some(StartupError {
            path: PathBuf::from(path),
            title: t(I18nKey::MainOpenFailedTitle).to_string(),
            message: STARTUP_ERROR_MESSAGE.to_string(),
        }),
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

/// 版本历史测试装配：快照库句柄由测试持有，用于预置/断言稳定键与槽内容。
fn app_history(text: &str, path: Option<&str>, store: &MemStore, snapshot: &FakeSnapshot) -> App {
    app_history_dialogs(text, path, store, snapshot, None)
}

fn app_history_dialogs(
    text: &str,
    path: Option<&str>,
    store: &MemStore,
    snapshot: &FakeSnapshot,
    save: Option<&str>,
) -> App {
    app_history_with_assets(text, path, store, snapshot, save, Box::new(NoAssets))
}

/// 同 `app_history_dialogs`，但可注入资产端口（另存为跨目录资产迁移测试）。
fn app_history_with_assets(
    text: &str,
    path: Option<&str>,
    store: &MemStore,
    snapshot: &FakeSnapshot,
    save: Option<&str>,
    assets: Box<dyn AssetStore>,
) -> App {
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let d = FakeDialogs {
        open: None,
        save: save.map(PathBuf::from),
        image: None,
        export: None,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets,
            exporter: Box::new(NoExport),
            dialogs: Box::new(d),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: PersistEnv {
                recovery: Box::new(FakeRecovery::default()),
                session: Box::new(FakeSession::default()),
                snapshot: Box::new(snapshot.clone()),
            },
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

/// 记录调用（会话序列化 + 源路径 + 主题样式透传）的导出器；fail 时返回 Err。
#[derive(Default)]
struct FakeExport {
    calls: ExportCalls,
    fail: bool,
}

impl Exporter for FakeExport {
    fn export_html(
        &mut self,
        session: &veil_core::Session,
        style: ExportStyle,
        source_path: Option<&Path>,
        target: Option<&Path>,
        _mermaids: Option<&std::collections::HashMap<veil_core::BlockId, MermaidImage>>,
    ) -> Result<PathBuf, String> {
        self.calls.borrow_mut().push((
            session.serialize(),
            source_path.map(Path::to_path_buf),
            style,
            target.map(Path::to_path_buf),
        ));
        if self.fail {
            Err("模拟导出失败".into())
        } else {
            Ok(target
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("out.html")))
        }
    }
}

fn app_export(text: &str, path: Option<&str>, fail: bool) -> (App, ExportCalls) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let e = FakeExport {
        calls: calls.clone(),
        fail,
    };
    // 导出必弹对话框（spec 要求）：mock 直接把建议路径当作用户选择，
    // 既有断言只关心「导出器被调用」，不受目标路径影响。
    let d = FakeDialogs {
        open: None,
        save: None,
        image: None,
        export: path.map(|p| PathBuf::from(p).with_extension("html")),
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let a = App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(e),
            dialogs: Box::new(d),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    );
    (a, calls)
}

/// 导出 + 对话框联合注入：用于「草稿态需先选导出目标」的路径。
fn app_export_with_dialog(
    text: &str,
    export_target: Option<&str>,
) -> (App, ExportCalls, Rc<RefCell<Vec<String>>>) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let dialog_calls = Rc::new(RefCell::new(Vec::new()));
    let doc = Doc {
        text: text.into(),
        path: None,
        bom: false,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let e = FakeExport {
        calls: calls.clone(),
        fail: false,
    };
    let d = FakeDialogs {
        open: None,
        save: None,
        image: None,
        export: export_target.map(PathBuf::from),
        calls: dialog_calls.clone(),
    };
    let a = App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(e),
            dialogs: Box::new(d),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    );
    (a, calls, dialog_calls)
}

fn dirty_app(text: &str, path: Option<&str>, store: &MemStore) -> App {
    let mut a = app(text, path, false, store);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    a
}

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
        image: None,
        export: None,
        calls: calls.clone(),
    };
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let a = App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(d),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    );
    (a, calls)
}

fn click_at(a: &mut App, x: f32, y: f32) -> bool {
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.on_left_click()
}

// ===== 任务列表复选框点击切换（P3-5） =====

fn menu_title_center(menu: usize) -> (f32, f32) {
    let (_, x, w) = chrome::menu_bar_rects(1.0)[menu];
    (x + w / 2.0, chrome::MENU_H / 2.0)
}

fn popup_item_center(menu: usize, item: usize) -> (f32, f32) {
    popup_item_center_recent(menu, item, &[])
}

/// 带最近文件的弹窗项中心（0b 动态菜单测试用）。
fn popup_item_center_recent(menu: usize, item: usize, recent: &[String]) -> (f32, f32) {
    let p = chrome::popup_layout(menu, 1.0, recent, 1000.0);
    let (_, x, y, w, h) = p.items[item];
    (x + w / 2.0, y + h / 2.0)
}

fn app_watch(text: &str, path: Option<&str>, store: &MemStore, watch: &FakeWatch) -> App {
    app_watch_dialogs(text, path, store, watch, None)
}

/// 外部变更 + 文件对话框联合装配：另存为出口需要真实返回目标的对话框。
fn app_watch_dialogs(
    text: &str,
    path: Option<&str>,
    store: &MemStore,
    watch: &FakeWatch,
    save: Option<&str>,
) -> App {
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let d = FakeDialogs {
        open: None,
        save: save.map(PathBuf::from),
        image: None,
        export: None,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(d),
            watcher: Box::new(watch.clone()),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

fn disk(store: &MemStore, path: &str, bytes: &[u8]) {
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from(path), bytes.to_vec());
}

/// 装配：指定对话框 + 恢复槽/会话；其余端口无副作用。
#[allow(clippy::too_many_arguments)]
fn app_confirm(
    text: &str,
    path: Option<&str>,
    store: &MemStore,
    recovery: FakeRecovery,
    session: &FakeSession,
    dialogs: Box<dyn FileDialog>,
) -> App {
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs,
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env_with(recovery, session.clone()),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

/// 断言模态已打开并按给定键作答（走真实路由，等价用户操作）。
fn answer_modal(a: &mut App, key: Key) {
    assert!(a.ui_state.modal.is_some(), "应弹出未保存确认模态");
    a.route_key(key);
}

/// 写一份当前内容的恢复快照（模拟编辑中的崩溃兜底已落盘）。
fn seed_recovery(a: &mut App, recovery: &FakeRecovery) {
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_recovery_write(t0 + Duration::from_secs(2));
    assert!(recovery.slot.borrow().is_some(), "快照应已写入");
}

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
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: ThemeEnv {
                config,
                system: Box::new(DynSystem(system)),
                effects: Box::new(theme::test_support::NoEffects),
                callouts: veil_core::CalloutRegistry::default(),
            },
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

/// 5005 块压力文档（715 × 7 块），冷启动/击键探针与惰性排版测试共用。
fn stress_doc() -> String {
    stress_doc_n(715)
}

/// 参数化压力文档：`iters` 轮 × 7 块；715 → 5005 块，1500 → 10500 块，2857 → 19999 块。
fn stress_doc_n(iters: usize) -> String {
    let mut text = String::with_capacity(iters * 280);
    for i in 0..iters {
        text.push_str(&format!(
            "## Section {i}\n\nParagraph {i} with **bold** text, `code`, and a [link](https://example.com/{i}).\n\n- item A {i}\n- item B {i}\n\n```rust\nfn f{i}() -> u32 {{ {i} }}\n```\n\n> quote {i}\n\n---\n\n"
        ));
    }
    text
}

// ---- P1b：视口驱动惰性排版 ----

/// 中文围栏代码块所在的 ≥4000 字长文档：多级标题 + 一个 ```markdown 中文代码块。
fn cjk_fence_long_doc() -> String {
    let mut text = String::new();
    for i in 0..60 {
        text.push_str(&format!("# 一级标题 {i}\n\n"));
        text.push_str("这是用于构造长文档的中文段落，包含足够多的文字以产生多行折行效果。\n\n");
        text.push_str(&format!("## 二级标题 {i}\n\n"));
        text.push_str(&format!(
            "### 三级标题 {i}\n\n段落 {i}：中文正文，**粗体** 与 `代码`。\n\n"
        ));
        if i == 30 {
            text.push_str(
                "```markdown\n# 任务：[精准任务名称]\n\n- 第一步：确认中文围栏代码块的字符边界\n- 第二步：滚动至文末不崩溃\n\n正文：中文标识_1 与 emoji 😀。\n```\n\n",
            );
        }
    }
    text
}

/// 按几何标尺（B5）重算期望文档高：块高 + 块距（含空行行高）+ 文末留白。
/// 与 `recompute_geometry` 使用同一组公开规则，用于校验 top/total_h 累计一致性。
fn expected_doc_height(a: &App) -> f32 {
    let scale = a.input_state.scale;
    let line_h = a.engine.doc_line_height();
    let mut y = MARGIN * scale;
    let mut prev: Option<&BlockGeom> = None;
    for g in &a.index.blocks {
        if let Some(p) = prev {
            let blank = a.blank_lines_in_gap(p.range, g.range);
            y += gap_between(
                &p.kind,
                p.range,
                p.container,
                &g.kind,
                g.range,
                g.container,
                blank,
                line_h,
            ) * scale;
        }
        y += g.height;
        prev = Some(g);
    }
    let Some(last) = a.index.blocks.last() else {
        return 2.0 * MARGIN * scale;
    };
    let len = a.editor.session().text_len();
    let tail_blank = a.blank_lines_in_gap(last.range, Range::new(len, len));
    let tail = gap_after(&last.kind)
        + (tail_blank as f32
            + crate::tokens::doc::BLANK_LINE_LINES
            + crate::tokens::doc::BOTTOM_PAD_LINES)
            * line_h;
    y + tail * scale
}

/// 假呈现面：复刻 softbuffer win32 的契约——未 resize 就取 buffer 直接 panic。
/// 用它跑真实 `present_frame` 调用链，顺序写错会在断言处失败，而不是只验证纯函数。
struct FakeSink {
    size: Option<(u32, u32)>,
    resizes: Vec<(u32, u32)>,
    presents: usize,
    resize_ok: bool,
    present_ok: bool,
}

impl FakeSink {
    fn new() -> Self {
        Self {
            size: None,
            resizes: Vec::new(),
            presents: 0,
            resize_ok: true,
            present_ok: true,
        }
    }
}

impl PresentSink for FakeSink {
    fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) -> bool {
        if !self.resize_ok {
            return false;
        }
        let size = (width.get(), height.get());
        self.size = Some(size);
        self.resizes.push(size);
        true
    }

    fn present_pixels(&mut self, pixels: &[u32]) -> bool {
        let (w, h) = self
            .size
            .expect("Must set size of surface before calling buffer_mut()");
        assert_eq!(
            pixels.len(),
            (w as usize) * (h as usize),
            "帧长度必须与已设置的 surface 尺寸一致"
        );
        self.presents += 1;
        self.present_ok
    }
}

fn test_frame(w: u32, h: u32) -> Frame {
    Frame::new(w, h, [0, 0, 0, 255])
}

/// 性质测试最小反例的确定性回归（2026-10-29 修复）：
/// `edits = [Newline, MoveDocStart]` 曾使标题块命中旧布局残留 `#` 标记。
/// 根因两处：指纹输入存文档绝对坐标（块整体平移后与块内 caret 误配）；
/// 指纹未折叠标题/围栏的块级激活位（caret 进出块不失效）。
fn memo_matches_forced_after(a: &mut crate::app::App) -> Vec<(BlockId, String)> {
    a.ensure_layouts_all();
    let before: Vec<(BlockId, String)> = a
        .index
        .blocks
        .iter()
        .filter_map(|g| {
            a.cache
                .layouts
                .get(&g.id)
                .map(|m| (g.id, m.layout_text().to_string()))
        })
        .collect();
    a.cache.layouts.clear();
    a.cache.rasters.clear();
    a.ensure_layouts_all();
    let after: Vec<(BlockId, String)> = a
        .index
        .blocks
        .iter()
        .filter_map(|g| {
            a.cache
                .layouts
                .get(&g.id)
                .map(|m| (g.id, m.layout_text().to_string()))
        })
        .collect();
    before
        .into_iter()
        .zip(after)
        .filter_map(|((id, memo), (_, forced))| (memo != forced).then_some((id, memo)))
        .collect()
}

/// 快照函数：收集 alive BlockId 集合后，按 id 从 cache.layouts 逐块取 (height, reveal, layout_text)。
/// 用 HashMap 按 BlockId 索引，避免 BlockId 动态变化导致位置错位。
fn snapshot_layouts(a: &App) -> std::collections::HashMap<BlockId, (f32, u64, String)> {
    a.index
        .blocks
        .iter()
        .filter_map(|g| {
            let m = a.cache.layouts.get(&g.id)?;
            Some((g.id, (m.height, m.key.reveal, m.layout_text().to_string())))
        })
        .collect()
}

#[derive(Clone, Debug)]
enum PersistEvent {
    /// 内容编辑：置脏并登记活动时刻（自动保存调度的输入）
    Edit,
    /// 外部写入新版本
    DiskChange(u8),
    /// 外部内容回退到当前基线（触发「撤销待备份」路径）
    DiskRevert,
    /// 外部删除文件
    DiskRemove,
    /// 监听事件 + 轮询检测
    Poll,
    /// 手工保存
    ManualSave,
    /// 自动保存时钟推进（静默期已过）
    AutoSaveTick,
}

fn persist_event() -> impl proptest::strategy::Strategy<Value = PersistEvent> {
    use proptest::strategy::{Just, Strategy as _};
    proptest::prop_oneof![
        3 => Just(PersistEvent::Edit),
        3 => (0u8..3).prop_map(PersistEvent::DiskChange),
        2 => Just(PersistEvent::DiskRevert),
        2 => Just(PersistEvent::DiskRemove),
        4 => Just(PersistEvent::Poll),
        3 => Just(PersistEvent::ManualSave),
        3 => Just(PersistEvent::AutoSaveTick),
    ]
}

fn version_bytes(v: u8) -> Vec<u8> {
    format!("external-{v}").into_bytes()
}

/// 磁盘上的冲突副本清单（按路径排序，内容一并取出用于比对）。
fn conflict_files(store: &MemStore) -> Vec<(PathBuf, Vec<u8>)> {
    let mut v: Vec<(PathBuf, Vec<u8>)> = store
        .files
        .borrow()
        .iter()
        .filter(|(p, _)| p.to_string_lossy().contains(".conflict-"))
        .map(|(p, b)| (p.clone(), b.clone()))
        .collect();
    v.sort();
    v
}

/// 事件执行（与有效用户操作同路径）：编辑/磁盘变更/轮询/保存/自动保存。
fn apply_persist_event(
    a: &mut App,
    store: &MemStore,
    watch: &FakeWatch,
    clock: &mut Instant,
    ev: &PersistEvent,
) {
    match ev {
        PersistEvent::Edit => {
            // 与用户编辑对持久化机器的输入完全同效：置脏 + 登记活动时刻。
            // 不走 after_edit：几何/排版与本次性质无关，跳过可把 100×30 步压到秒级。
            if a.editor.apply(Intent::InsertText("x".into())).is_some() {
                a.persist_state.persist.note_activity(*clock);
            }
        }
        PersistEvent::DiskChange(v) => disk(store, "a.md", &version_bytes(*v)),
        PersistEvent::DiskRevert => {
            let base = a.doc.baseline.clone();
            disk(store, "a.md", &base);
        }
        PersistEvent::DiskRemove => {
            store.files.borrow_mut().remove(Path::new("a.md"));
        }
        PersistEvent::Poll => {
            let present = store.files.borrow().contains_key(Path::new("a.md"));
            watch.events.borrow_mut().push(if present {
                FileEvent::Changed
            } else {
                FileEvent::Removed
            });
            a.poll_external();
        }
        PersistEvent::ManualSave => a.save_now(),
        PersistEvent::AutoSaveTick => {
            *clock += Duration::from_secs(2);
            a.maybe_autosave(*clock);
        }
    }
}

/// 可观察投影（判定确定性与轮询幂等用）：五个事实 + 磁盘全量内容 + 基线。
type PersistProjection = (
    ExternalState,
    bool,
    Option<Vec<u8>>,
    bool,
    bool,
    HashMap<PathBuf, Vec<u8>>,
    Vec<u8>,
);

/// 0b 装配助手：恢复槽/会话由测试预置并持句柄断言；`resume` 模拟 CLI（false）/裸启动（true）。
fn app_persist(
    text: &str,
    path: Option<&str>,
    store: &MemStore,
    recovery: &FakeRecovery,
    session: &FakeSession,
    resume: bool,
) -> App {
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env_with(recovery.clone(), session.clone()),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

/// 恢复槽中解码出的（路径, 内容）。
fn slot_content(recovery: &FakeRecovery) -> (Option<PathBuf>, String) {
    let data = recovery.slot.borrow().clone().expect("恢复槽应有快照");
    let r = recovery::decode_recovery(&data).expect("快照应可解码");
    (r.path, r.text)
}

/// 0b 装配 + 文件对话框：用于「恢复草稿后另存为」这类需要对话框的持久化场景。
fn app_persist_dialogs(
    text: &str,
    path: Option<&str>,
    store: &MemStore,
    recovery: &FakeRecovery,
    session: &FakeSession,
    resume: bool,
    save: Option<&str>,
) -> App {
    let draft = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let d = FakeDialogs {
        open: None,
        save: save.map(PathBuf::from),
        image: None,
        export: None,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        draft,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(d),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env_with(recovery.clone(), session.clone()),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

const LINK_DOC: &str = "[链接](https://example.com)\n";

/// 链接文字上的鼠标落点（与右键测试同口径：偏移 1 的字符盒 + 少量内缩）。
fn link_point(a: &mut App) -> (f32, f32) {
    a.ensure_layouts_all();
    let id = a.editor.session().blocks()[0].0;
    let top = a
        .index
        .blocks
        .iter()
        .find(|g| g.id == id)
        .expect("正文块应存在")
        .top;
    let (lx, ly, _) = {
        let l = &a.cache.layouts[&id];
        a.engine.caret_xy(l, 1)
    };
    let scale = a.input_state.scale;
    let vp = a.viewport();
    (
        MARGIN * scale + lx + 2.0,
        vp.y + top - a.window_state.scroll_y + ly + 5.0,
    )
}

fn link_app(text: &str, calls: &Rc<RefCell<Vec<String>>>, result: Result<(), String>) -> App {
    app_with(
        text,
        None,
        false,
        &MemStore::default(),
        Box::new(FakeUrl {
            calls: calls.clone(),
            result,
        }),
    )
}

// ---- Ctrl+单击应用内跳转（脚注引用↔定义、`#slug` → 标题）----

/// 文档偏移 → 鼠标坐标（落到该偏移所在块的排版位置）。
/// 装饰（`[n]`/`↩`）没有独立源码偏移，但它们都能用锚点 docs 偏移定位：
/// `caret_xy` 内部走 `doc_to_layout`，零宽插入段会钳到装饰左缘。
fn point_at_doc(a: &mut App, offset: usize) -> (f32, f32) {
    a.ensure_layouts_all();
    let idx = a
        .index
        .blocks
        .iter()
        .rposition(|g| g.range.start <= offset && offset <= g.range.end)
        .expect("偏移应落在某块内");
    let (id, top, start) = {
        let g = &a.index.blocks[idx];
        (g.id, g.top, g.range.start)
    };
    let (lx, ly, lh) = {
        let l = &a.cache.layouts[&id];
        a.engine.caret_xy(l, offset - start)
    };
    let vp = a.viewport();
    (
        a.content_x() + lx + 1.0,
        vp.y + top - a.window_state.scroll_y + ly + lh / 2.0,
    )
}

/// Ctrl + 单击某文档偏移（按下/抬起 Ctrl 模拟真实修饰键状态）。
fn ctrl_click_doc(a: &mut App, offset: usize) {
    let (x, y) = point_at_doc(a, offset);
    a.input_state.mods.ctrl = true;
    click_at(a, x, y);
    a.input_state.mods.ctrl = false;
}

/// 某块首个 `FootnoteRef` 节点的源码起点。
fn first_ref_start(a: &App, block_idx: usize) -> usize {
    let id = a.index.blocks[block_idx].id;
    a.editor
        .session()
        .block_inlines(id)
        .expect("块应有 inline 节点")
        .iter()
        .find(|n| n.kind == InlineKind::FootnoteRef)
        .expect("块内应有脚注引用")
        .range
        .start
}

/// 标题块起点（文档顺序）。
fn heading_starts(a: &App) -> Vec<usize> {
    a.editor
        .session()
        .blocks()
        .iter()
        .filter(|(_, k, _)| matches!(k, BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_)))
        .map(|(_, _, r)| r.start)
        .collect()
}

/// 在帧的 [y0, y1) 行内找首个非背景像素（文本/光标/装饰）。
fn first_non_bg(f: &Frame, y0: i32, y1: i32, bg: [u8; 4]) -> Option<(u32, u32, [u8; 4])> {
    for y in y0.max(0) as u32..y1.max(0) as u32 {
        for x in 0..f.width {
            let p = f.get(x, y);
            if p != bg {
                return Some((x, y, p));
            }
        }
    }
    None
}

/// 打印几何采样：begin 后各块 (top, 行顶序列)，断点合法性（不劈行）的判定依据。
type PrintGeoms = Rc<RefCell<Vec<(f32, Vec<f32>)>>>;
/// 页帧采样：逐页 (宽, 高, 四角像素)，无 chrome 的直接证据。
type PrintFrames = Rc<RefCell<Vec<(u32, u32, [[u8; 4]; 4])>>>;

/// 打印端口 mock：按注入页帧跑完整 begin→逐页→end 流程，采样页数/断点/
/// 各块行顶/页帧尺寸与四角像素，供测试断言（真实 Win32 粘合在产品层）。
/// 渲染态表格采样：(块顶, 各行行顶偏移, 表头行高)。
type PrintTableRows = Rc<RefCell<Vec<(f32, Vec<f32>, f32)>>>;

#[derive(Clone, Default)]
struct FakePrint {
    w: u32,
    h: u32,
    scale: f32,
    pages: Rc<Cell<usize>>,
    breaks: Rc<RefCell<Vec<f32>>>,
    geoms: PrintGeoms,
    frames: PrintFrames,
    ended: Rc<Cell<bool>>,
    table_rows: PrintTableRows,
}

impl Print for FakePrint {
    fn print(&mut self, s: &mut PrintSession) {
        let pages = s.begin(self.w.max(1), self.h.max(1), self.scale.max(0.1));
        self.pages.set(pages);
        *self.breaks.borrow_mut() = s.page_breaks().to_vec();
        // 打印几何采样：块 top + 各视觉行行顶（有效光标 = usize::MAX，与分页同源）
        let base = s.app.base_key();
        let mut gs = Vec::new();
        for g in &s.app.index.blocks {
            let tops = s
                .app
                .cache
                .layout(
                    g.id,
                    &g.kind,
                    g.range,
                    base,
                    usize::MAX,
                    s.app.toc_layout_epoch(g),
                )
                .map(|l| s.app.engine.line_tops(l))
                .unwrap_or_default();
            gs.push((g.top, tops));
        }
        *self.geoms.borrow_mut() = gs;
        // 渲染态表格行几何：分页候选与续页表头高都来自同一份 TableLayout。
        let mut trs = Vec::new();
        for g in &s.app.index.blocks {
            if g.kind == BlockKind::Table && s.app.table_shows_grid(g.id, &g.range) {
                if let Some(tl) = s.app.cache.table(g.id, s.app.table_key(g.id)) {
                    trs.push((
                        g.top,
                        tl.rows.iter().map(|r| r.top).collect::<Vec<_>>(),
                        tl.rows.first().map_or(0.0, |r| r.height),
                    ));
                }
            }
        }
        *self.table_rows.borrow_mut() = trs;
        for i in 0..pages {
            let f = s.render_page(i);
            self.frames.borrow_mut().push((
                f.width,
                f.height,
                [
                    f.get(0, 0),
                    f.get(f.width - 1, 0),
                    f.get(0, f.height - 1),
                    f.get(f.width - 1, f.height - 1),
                ],
            ));
        }
        s.end();
        self.ended.set(true);
    }
}

/// 打印 fixture：注入打印端口，其余与默认 fixture 一致。
fn app_print(text: &str, print: Box<dyn Print>) -> App {
    let doc = Doc {
        text: text.into(),
        path: None,
        bom: false,
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print,
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

/// 断点合法性：除 0.0 外每个断点落在某块 top 或块内行顶（不劈行）。
fn assert_breaks_on_boundaries(bs: &[f32], gs: &[(f32, Vec<f32>)]) {
    for &b in &bs[1..] {
        let on = gs.iter().any(|(top, lines)| {
            (top - b).abs() < 0.5 || lines.iter().any(|&t| (top + t - b).abs() < 0.5)
        });
        assert!(on, "断点 {b} 应落在块顶或行顶: {gs:?}");
    }
}

/// Reveal 探针：begin 后采样全部块排版文本（打印中 Reveal 全收的观测口）。
struct RevealProbe {
    texts: Rc<RefCell<Vec<String>>>,
}

impl Print for RevealProbe {
    fn print(&mut self, s: &mut PrintSession) {
        s.begin(300, 600, 1.0);
        let mut out = Vec::new();
        for g in &s.app.index.blocks {
            if let Some(m) = s.app.cache.layouts.get(&g.id) {
                out.push(m.value.layout_text().to_string());
            }
        }
        *self.texts.borrow_mut() = out;
        s.end();
    }
}

/// 超高数学图渲染器（3b 硬切测试）：500px 图超 100px 页帧。
struct TallMath;
impl MathRenderer for TallMath {
    fn render(
        &mut self,
        _tex: &str,
        _display: bool,
        _scale: f32,
        _color: [u8; 3],
    ) -> Result<MathImage, String> {
        Ok(MathImage {
            pixels: vec![255; 10 * 500 * 4],
            width: 10,
            height: 500,
            baseline: 0,
        })
    }
}

/// 按块种类取 BlockId（源码模式测试的观测口）。
fn block_id_of(a: &App, pred: impl Fn(&BlockKind) -> bool) -> BlockId {
    a.index
        .blocks
        .iter()
        .find(|g| pred(&g.kind))
        .map(|g| g.id)
        .expect("文档应含目标块")
}

/// 打印探针：begin 后采样标题块（打印态）、end 后按屏幕态重排再采样。
struct SourcePrintProbe {
    during_heading: Rc<RefCell<String>>,
    after_heading: Rc<RefCell<String>>,
    source_during_print: Rc<Cell<bool>>,
}

impl Print for SourcePrintProbe {
    fn print(&mut self, s: &mut PrintSession) {
        s.begin(400, 600, 1.0);
        let heading = s
            .app
            .index
            .blocks
            .iter()
            .find(|g| matches!(g.kind, BlockKind::AtxHeading(_)))
            .map(|g| g.id)
            .expect("文档含标题块");
        self.source_during_print.set(s.app.source_mode());
        *self.during_heading.borrow_mut() = s.app.cache.layouts[&heading].layout_text().to_string();
        s.end();
        s.app.ensure_layouts_all();
        *self.after_heading.borrow_mut() = s.app.cache.layouts[&heading].layout_text().to_string();
    }
}

const EMPTY_MERMAID_DOC: &str = "p\n\n```mermaid\n```\n";

/// 空 mermaid 块 body 起点的绝对字节偏移（开围栏行换行之后）。
fn empty_mermaid_body_start(doc: &str) -> usize {
    doc.find("```mermaid\n").expect("文档应含 mermaid 开围栏") + "```mermaid\n".len()
}

/// 把鼠标放到 mermaid 块首行（卡片头）内；空块允许整块卡片命中。
fn hover_mermaid_block(a: &mut App) {
    a.ensure_layouts_all();
    let id = block_id_of(a, |k| matches!(k, BlockKind::Mermaid));
    let top = geom(a, id).top;
    let scale = a.input_state.scale;
    let vp = a.viewport();
    a.input_state.mouse = PhysicalPosition::new(
        (MARGIN * scale + 8.0) as f64,
        (vp.y + top - a.window_state.scroll_y + BASE_LINE_HEIGHT * scale / 2.0) as f64,
    );
}

fn click_empty_mermaid_block(a: &mut App) {
    hover_mermaid_block(a);
    a.on_left_click();
}

/// 悬停第 n 个 Mermaid 块（0-based），用于同源码多块的目标选择测试。
fn hover_nth_mermaid_block(a: &mut App, nth: usize) {
    a.ensure_layouts_all();
    let id = a
        .index
        .blocks
        .iter()
        .filter(|g| matches!(g.kind, BlockKind::Mermaid))
        .nth(nth)
        .map(|g| g.id)
        .expect("文档应含第 n 个 Mermaid 块");
    let top = geom(a, id).top;
    let scale = a.input_state.scale;
    let vp = a.viewport();
    a.input_state.mouse = PhysicalPosition::new(
        (MARGIN * scale + 8.0) as f64,
        (vp.y + top - a.window_state.scroll_y + BASE_LINE_HEIGHT * scale / 2.0) as f64,
    );
}

/// 在第 n 个 Mermaid 块上右键并执行指定菜单项。
fn exec_nth_mermaid_action(a: &mut App, nth: usize, action: ctx_menu::CtxAction) {
    hover_nth_mermaid_block(a, nth);
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "mermaid 块右键应打开菜单");
    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == action)
        .expect("菜单应含目标项");
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);
}

/// 远程图片 + 资产双端口装配（Mermaid 本地预览/插入测试用）。
fn app_remote_assets(
    text: &str,
    path: Option<&str>,
    store: &MemStore,
    images: RemoteImages,
    assets: FakeAssets,
) -> App {
    app_remote_assets_env(
        text,
        path,
        store,
        images,
        assets,
        theme::test_support::env(),
    )
}

fn app_remote_assets_env(
    text: &str,
    path: Option<&str>,
    store: &MemStore,
    images: RemoteImages,
    assets: FakeAssets,
    theme_env: ThemeEnv,
) -> App {
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume: false,
        open_error: None,
    };
    App::new(
        doc,
        Ports {
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(FakeMath {
                fail: false,
                calls: Rc::new(RefCell::new(Vec::new())),
            }),
            mermaid: Box::new(NoopMermaid),
            images: Box::new(images),
            assets: Box::new(assets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env,
            persist_env: persist_env(),
            print: Box::new(NoPrint),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    )
}

fn test_mermaid_image(width: u32, height: u32) -> MermaidImage {
    MermaidImage {
        data: vec![255; (width * height * 4) as usize],
        width,
        height,
        svg_text: None,
        px_to_pt: 0.375,
    }
}

/// 在 mermaid 块上右键并执行指定菜单项。
fn exec_mermaid_action(a: &mut App, action: ctx_menu::CtxAction) {
    hover_mermaid_block(a);
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "mermaid 块右键应打开菜单");
    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == action)
        .expect("菜单应含目标项");
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);
}

/// 通过右键菜单执行「预览」项（菜单动作与键盘路径共用 `ctx_exec`）。
fn exec_mermaid_preview(a: &mut App) {
    hover_mermaid_block(a);
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "mermaid 块右键应打开菜单");
    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::MermaidPreview)
        .expect("菜单应含预览项");
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);
}

const DETAILS_DOC: &str = r#"前

<details>
<summary>安装步骤</summary>
pip install veil
然后运行 veil
</details>

后
"#;

const DETAILS_OPEN_DOC: &str = r#"前

<details open>
<summary>配置</summary>
export VEIL_HOME=~/.veil
</details>

后
"#;

/// 文档偏移 → 块内相对偏移（测试取样辅助）。
fn rel_offset(a: &App, id: BlockId, abs: usize) -> usize {
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    abs - g.range.start
}

/// 多次取样中寻找矩形内第一个满足条件的像素（避免依赖字形墨迹具体位置）。
fn any_pixel(f: &Frame, x0: i32, y0: i32, w: u32, h: u32, pred: impl Fn([u8; 4]) -> bool) -> bool {
    for y in y0..y0 + h as i32 {
        for x in x0..x0 + w as i32 {
            if x >= 0
                && y >= 0
                && (x as u32) < f.width
                && (y as u32) < f.height
                && pred(f.get(x as u32, y as u32))
            {
                return true;
            }
        }
    }
    false
}

/// 统计区域内明显深于背景的墨迹像素数（文本可见性判定）。
fn ink_pixels(f: &Frame, x0: i32, y0: i32, w: u32, h: u32) -> usize {
    let mut n = 0;
    for y in y0..y0 + h as i32 {
        for x in x0..x0 + w as i32 {
            if x >= 0 && y >= 0 && (x as u32) < f.width && (y as u32) < f.height {
                let p = f.get(x as u32, y as u32);
                n += usize::from(p[0] < 200 && p[1] < 200 && p[2] < 200);
            }
        }
    }
    n
}

/// 单元格光栅的墨迹行范围（alpha > 32）；无墨迹 → None。
fn raster_ink_rows(pixels: &[u8], w: u32, h: u32) -> Option<(usize, usize)> {
    let (mut lo, mut hi) = (usize::MAX, 0usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            if pixels[(y * w as usize + x) * 4 + 3] > 32 {
                lo = lo.min(y);
                hi = hi.max(y);
            }
        }
    }
    (lo != usize::MAX).then_some((lo, hi))
}

/// 断言某一行存在「间隔的暗像素段」（≥2 段 + 段间背景缝隙）：abbr 点线回归。
fn assert_dotted_row(f: &Frame, x0: i32, y: i32, w: i32, bg: [u8; 4]) {
    let (mut runs, mut gaps) = (0usize, 0usize);
    let mut prev_dark = false;
    for x in x0..x0 + w {
        let p = f.get(x.max(0) as u32, y.max(0) as u32);
        let dark = p != bg;
        if dark && !prev_dark {
            runs += 1;
        }
        if !dark && prev_dark {
            gaps += 1;
        }
        prev_dark = dark;
    }
    assert!(runs >= 2, "下划线应为间隔短段（实际段数 {runs}，行 {y}）");
    assert!(gaps >= 1, "下划线短段之间应有缝隙（实际缝隙 {gaps}）");
}

const CODE_MULTILINE: &str =
    "```rust\nlet x = 1;\nlet 中 = \"你好\";\n\nprintln!(\"{}\", 中);\n```\n";

/// 导出对话框：总是给出目标路径，用于触发导出器失败路径。
struct ExportPathDialog;
impl FileDialog for ExportPathDialog {
    fn open_markdown(&mut self) -> Option<PathBuf> {
        None
    }
    fn save_markdown(&mut self, _suggested: &str) -> Option<PathBuf> {
        None
    }
    fn save_export(&mut self, _suggested: &str, _ext: &str) -> Option<PathBuf> {
        Some(PathBuf::from("out.html"))
    }
}

/// 打印端口：直接走失败出口，模拟平台层打印/PDF 失败。
struct FailingPrint;
impl Print for FailingPrint {
    fn print(&mut self, session: &mut PrintSession) {
        session.print_error("模拟打印失败");
    }
    fn export_pdf(&mut self, session: &mut PrintSession) {
        session.export_pdf_error("模拟导出失败");
    }
}

/// 打印端口：模拟平台层拿不到打印机上下文（hDC 为空）。
struct NoPrinterContextPrint;
impl Print for NoPrinterContextPrint {
    fn print(&mut self, session: &mut PrintSession) {
        session.notice_error(t(I18nKey::PrintNoPrinterContext));
    }
}

/// Ctrl + 单键：真实路径与测试共用取模后的 `route_key_with_focus`。
fn ctrl_key(a: &mut App, key: Key) {
    a.input_state.mods.ctrl = true;
    a.route_key_with_focus(key);
    a.input_state.mods.ctrl = false;
}

/// 焦点环对比度（WCAG）：测试侧独立实现，避免与被测代码共用同一个函数。
fn ring_contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let lum = |c: [u8; 3]| {
        let ch = |v: u8| {
            let s = v as f32 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
    };
    let (l1, l2) = (lum(a), lum(b));
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

// 主题测试子模块：按领域拆分，各文件仅含 #[test]，共享 helper 留在本模块（私有，子模块作为后代可直接访问）。

mod app;

mod blocks;

mod chrome_ui;

mod io;

mod media;

mod persist;

mod render;

mod tables;

mod view;
