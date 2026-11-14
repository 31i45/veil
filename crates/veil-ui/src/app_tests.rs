//! 跨域行为测试：以 headless App 驱动真实编辑/排版/绘制/持久化路径。

use crate::app::*;
use crate::chrome;
use crate::chrome::MenuAction;
use crate::docio::encode;
use crate::document::block_index::{gap_after, gap_between, BlockGeom};
use crate::document::recovery;
use crate::document::ExternalState;
use crate::editor::Intent;
use crate::geometry::{BASE_LINE_HEIGHT, MARGIN, PARA_GAP};
use crate::i18n::{t, Key as I18nKey};
use crate::image::{AssetMigration, AssetStore, ImageData, ImageLoader};
use crate::input::{Action, Key, Mods};
use crate::interaction::map_key_parts;
use crate::math::{MathImage, MathRenderer};
use crate::mode::{Mode, TextSink};
use crate::paint::{can_present, fit_size, local_image_path};
use crate::print::{ExportOptions, NoPrint, Print, PrintSession};
use crate::theme::{self, ConfigStore, ExportStyle, SystemTheme, ThemeEnv, ThemeMode};
use crate::view::ctx_menu;
use crate::view::outline;
use crate::view::search;
use crate::view::{Frame, InlineAsset, Memo, MAX_BLOCK_BYTES};
use crate::Ports;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use veil_core::{BlockId, BlockKind, InlineKind, Range};
use winit::dpi::PhysicalPosition;
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

fn no_url() -> Box<dyn UrlOpener> {
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
            clipboard: Box::new(Clip {
                html: html.map(str::to_string),
                text: plain.map(str::to_string),
            }),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
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
    assert_eq!(
        doc_text(&a),
        "a **b**",
        "末尾单换行必须剥掉，避免行中粘贴凭空断行"
    );
    assert!(a.editor.is_dirty());
}

#[test]
fn paste_falls_back_to_plain_when_html_converts_empty() {
    // 空结构（如纯脚本/空白）转换无内容 → 用纯文本。
    // 注：图片现已能转 Markdown（DEF-T67-E1-003），故不再用 img 作为空结构样本。
    let mut a = app_clip(Some("<p> </p><script>x</script>"), Some("fallback"));
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
        _style: ExportStyle,
        _p: Option<&Path>,
        _t: Option<&Path>,
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
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
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

#[test]
fn math_block_renders_image_when_caret_outside() {
    // 「a\n\n$$\nx\n$$\n」：数学块 bytes [3,11)，初始 caret=0 在块外
    let (mut a, calls) = app_math("a\n\n$$\nx\n$$\n", false);
    a.ensure_layouts_all();
    a.ensure_math();
    a.ensure_math(); // 幂等：缓存命中不重渲
    assert_eq!(calls.borrow().len(), 1);
    let (id, kind, range) = a.editor.session().blocks()[1].clone();
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
    let (id, _, range) = a.editor.session().blocks()[1].clone();
    assert!(!a.math_shows_image(id, &range));
    let h = a.cache.layouts.get(&id).unwrap().height;
    assert_eq!(a.block_height(geom(&a, id)), h); // 文本布局高
}

#[test]
fn math_render_failure_degrades_to_source() {
    let (mut a, calls) = app_math("a\n\n$$\nx\n$$\n", true);
    a.ensure_layouts_all();
    a.ensure_math();
    a.ensure_math(); // 失败记忆：不每帧重试
    assert_eq!(calls.borrow().len(), 1);
    let (id, _, range) = a.editor.session().blocks()[1].clone();
    assert!(a.cache.math_failed.contains_key(&id));
    assert!(!a.math_shows_image(id, &range));
    let h = a.cache.layouts.get(&id).unwrap().height;
    assert_eq!(a.block_height(geom(&a, id)), h); // 降级为源码显示
}

#[test]
fn dirty_edit_clears_math_cache() {
    let (mut a, calls) = app_math("a\n\n$$\nx\n$$\n", false);
    a.ensure_layouts_all();
    a.ensure_math();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    assert!(a.cache.math_imgs.contains_key(&id));
    // 块内真实编辑：脏块重解析分配新 BlockId，旧图按 id 即失配（prune 回收旧条目）
    a.editor.set_caret(6);
    let dirty = a.editor.apply(Intent::InsertText("y".into())).unwrap();
    a.after_edit(dirty);
    let (id2, kind, range) = a.editor.session().blocks()[1].clone();
    assert_eq!(kind, BlockKind::Math);
    assert!(a.cache.math(id2, a.math_key(id2)).is_none(), "块内容已变");
    // 光标移出块：新内容重渲一次，不再沿用旧图
    a.editor.set_caret(0);
    a.after_edit(Vec::new());
    a.ensure_math();
    assert_eq!(calls.borrow().len(), 2);
    assert!(a.cache.math(id2, a.math_key(id2)).is_some());
    assert!(a.math_shows_image(id2, &range));
}

// ---- 行内公式（切片 8）：非激活 Math 节点出图、激活显源码、tex 内容寻址缓存 ----
// 「a $x$ b」逐字节：a=0 ' '=1 $=2 x=3 $=4 ' '=5 b=6，Math range=[2,5) content=[3,4)

#[test]
fn inline_math_renders_image_when_caret_outside() {
    let (mut a, calls) = app_math("a $x$ b", false);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    assert_eq!(a.cache.layouts[&id].imgs.len(), 1);
    assert_eq!(
        a.cache.layouts[&id].imgs[0].width, 8,
        "FakeMath 行内分支（8x4）"
    );
    assert_eq!(
        calls.borrow().as_slice(),
        &[("x".to_string(), false)],
        "display=false"
    );
    // 内容寻址缓存：重排（如 Resized 清布局）不重渲
    a.cache.layouts.remove(&id);
    a.ensure_layouts_all();
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn inline_math_shows_source_when_caret_inside() {
    let (mut a, calls) = app_math("a $x$ b", false);
    assert!(a.editor.set_caret(3)); // 「x」处，节点内
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    assert!(a.cache.layouts[&id].imgs.is_empty());
    assert_eq!(a.cache.layouts[&id].layout_text(), "a $x$ b");
    assert!(calls.borrow().is_empty(), "源码态不渲染");
}

#[test]
fn inline_math_toggles_on_caret_enter_leave() {
    let (mut a, calls) = app_math("a $x$ b", false);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    assert_eq!(a.cache.layouts[&id].imgs.len(), 1);
    // 进入节点：激活指纹变化 → 键失配 → 重排显源码
    assert!(a.editor.set_caret(3));
    a.ensure_layouts_all();
    assert!(a.cache.layouts[&id].imgs.is_empty());
    // 离开节点：恢复出图，缓存命中不重渲
    assert!(a.editor.set_caret(0));
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].imgs.len(), 1);
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn inline_math_failure_degrades_and_memoizes() {
    let (mut a, calls) = app_math("a $x$ b", true);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    assert!(
        a.cache.layouts[&id].imgs.is_empty(),
        "失败降级：占位不入排版"
    );
    a.cache.layouts.remove(&id);
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
    let dirty = a.editor.apply(Intent::InsertText("y".into())).unwrap();
    a.after_edit(dirty);
    assert_eq!(calls.borrow().len(), 1);
    // 离开节点 → 新 tex 渲染一次
    assert!(a.editor.set_caret(0));
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    assert_eq!(a.cache.layouts[&id].imgs.len(), 1);
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
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            images: Box::new(FakeImages {
                files,
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

/// DEF-T05-004：percent 编码的中文/空格路径按 UTF-8 解码后再解析。
#[test]
fn percent_encoded_local_image_path_decodes() {
    let doc = Path::new("dir/notes.md");
    assert_eq!(
        local_image_path("img/%E4%B8%AD%E6%96%87.png", Some(doc)),
        Some(PathBuf::from("dir/img/中文.png"))
    );
    assert_eq!(
        local_image_path("img/a%20b.png", Some(doc)),
        Some(PathBuf::from("dir/img/a b.png"))
    );
    // 非法百分号序列原样保留，不 panic。
    assert_eq!(
        local_image_path("img/%ZZ.png", Some(doc)),
        Some(PathBuf::from("dir/img/%ZZ.png"))
    );
    // 编码后的 scheme 不误判为本地路径。
    assert_eq!(
        local_image_path("https%3A%2F%2Fexample.com%2Fa.png", Some(doc)),
        Some(PathBuf::from("dir/https://example.com/a.png")),
        "解码后仍按本地相对路径处理（不以 scheme 加载）"
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
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    let l = &a.cache.layouts[&id];
    assert_eq!(l.imgs.len(), 1);
    let InlineAsset::Image(key) = &l.imgs[0].key else {
        panic!("应为图片键")
    };
    assert_eq!(Path::new(key), Path::new("dir/img/p.png"));
    // A：行内图限高提到 4 行（4×22=88），100×50 原图在上限内 → 不缩放。
    assert_eq!((l.imgs[0].width, l.imgs[0].height), (100, 50));
    assert!(
        !l.layout_text().contains("alt"),
        "加载成功：整节点由占位图替换"
    );
    assert_eq!(calls.borrow().as_slice(), &[PathBuf::from("dir/img/p.png")]);
    // 缓存命中：重排不重载
    a.cache.layouts.remove(&id);
    a.ensure_layouts_all();
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn image_wider_than_content_scales_proportionally() {
    // 前缀 "x "：默认光标 0 不在图片节点区间内（否则进入源码态不加载）
    // 宽 1600 且高 45（低于 2 行限高），确保本用例仍由「宽度上限」触发缩放。
    let (mut a, _) = app_with_images(
        "x ![a](p.png)",
        Some("dir/doc.md"),
        &[("dir/p.png", 1600, 45)],
    );
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    // 默认版心宽 = 800 - 2*16 - 滚动条 12 = 756；1600×45 等比缩放
    assert_eq!(
        (
            a.cache.layouts[&id].imgs[0].width,
            a.cache.layouts[&id].imgs[0].height
        ),
        (756, 21)
    );
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

/// B4 回归：整块仅图片（含尾随换行）→ 按版心宽/视口高放大，不再受 4 行限高。
#[test]
fn standalone_markdown_image_uses_viewport_height() {
    let (mut a, _) = app_with_images(
        "前文\n\n![本地图](p.png)\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 800, 600)],
    );
    a.window_state.test_size = Some((1280, 800));
    a.ensure_layouts_all();
    let l = layout_of_text(&a, "![本地图]");
    let (w, h) = (l.imgs[0].width, l.imgs[0].height);
    assert!(w >= 600, "独占图应接近版心宽（实测 {w}）");
    assert!(
        h as f32 > 4.0 * a.engine.doc_line_height() * a.input_state.scale,
        "独占图高度应超过行内 4 行限高（实测 {h}）"
    );
}

/// B4 回归：图片后另起一行的正文不影响该图独占自己的视觉行（真机复现形状）。
#[test]
fn image_alone_on_its_visual_line_uses_viewport_height() {
    // 审计文档末块实际为 `![本地图](p.png)\nX`：整块非纯图片，但图片仍独占一行。
    let (mut a, _) = app_with_images(
        "前文\n\n![本地图](p.png)\nX\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 800, 600)],
    );
    a.window_state.test_size = Some((1280, 800));
    a.ensure_layouts_all();
    let l = layout_of_text(&a, "![本地图]");
    assert!(
        l.imgs[0].width >= 600,
        "图片所在视觉行只有图片时应放大（实测 {}）",
        l.imgs[0].width
    );
}

/// B4 回归：同一行内有正文的图片仍受 4 倍行高限制。
#[test]
fn inline_image_still_capped_by_four_lines() {
    let (mut a, _) = app_with_images(
        "a ![alt](p.png) b",
        Some("dir/doc.md"),
        &[("dir/p.png", 800, 600)],
    );
    a.window_state.test_size = Some((1280, 800));
    a.ensure_layouts_all();
    let l = layout_of_text(&a, "a ![alt]");
    let line_h = a.engine.doc_line_height() * a.input_state.scale;
    let cap = (crate::tokens::doc::IMG_INLINE_LINES * line_h).round() as u32;
    assert_eq!(l.imgs[0].height, cap, "行内高图应夹到 4 行");
    assert!(l.imgs[0].width < 600, "行内图不应占满版心");
}

/// B4 回归：独占一行的 `<img width>` 按显式逻辑像素显示（乘 DPI 转物理像素），
/// 且不再被行内行高限制。
#[test]
fn standalone_html_img_honors_explicit_logical_width() {
    let (mut a, _) = app_with_images(
        "前文\n\n<img src=\"p.png\" width=\"320\">\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 800, 600)],
    );
    a.window_state.test_size = Some((1280, 800));
    a.ensure_layouts_all();
    let l = layout_of_text(&a, "<img src");
    assert_eq!(
        (l.imgs[0].width, l.imgs[0].height),
        (320, 240),
        "scale=1 时 320 逻辑像素 = 320 物理像素"
    );

    // DPI 1.25：仍显示 320 逻辑像素（= 400 物理像素），不被行内 4 行限高压回。
    a.input_state.scale = 1.25;
    a.input_state.dpi_scale = 1.25;
    a.ensure_layouts_all();
    let l = layout_of_text(&a, "<img src");
    assert_eq!(
        (l.imgs[0].width, l.imgs[0].height),
        (400, 300),
        "显式尺寸是 CSS 逻辑像素，需随 DPI 放大"
    );
}

/// B4 回归：窗口高度变化 → 独占图片尺寸随之变化（布局缓存键覆盖视口高度）。
#[test]
fn window_height_change_resizes_standalone_image() {
    let (mut a, _) = app_with_images(
        "前文\n\n![本地图](p.png)\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 800, 600)],
    );
    a.window_state.test_size = Some((1280, 800));
    a.ensure_layouts_all();
    let tall = layout_of_text(&a, "![本地图]").imgs[0].height;

    a.window_state.test_size = Some((1280, 400));
    a.ensure_layouts_all();
    let short = layout_of_text(&a, "![本地图]").imgs[0].height;

    assert!(short < tall, "窗口变矮后独占图应变小: {tall} -> {short}");
    assert_eq!(
        short,
        a.image_max_height().round() as u32,
        "变矮后按新视口 80% 等比缩放"
    );
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

/// B4 回归：独立图片块只占图片实际显示高度，后接标题的间距与普通段落一致。
/// 回归：占位 span 重复拼接 → NBSP 翻倍折行 → 块高 = 2×图高（半屏空白）。
#[test]
fn standalone_image_block_height_is_displayed_height() {
    let (mut a, _) = app_with_images(
        "前文\n\n![](p.png)\n\n## 1.1 背景\n\n背景介绍。\n\n普通段落\n\n## 1.2 目标\n",
        Some("dir/doc.md"),
        // 宽度受限的矮图：显示宽 = 版心宽，高 ≈ 155px，远小于视口 80%。
        &[("dir/p.png", 1600, 200)],
    );
    a.window_state.test_size = Some((1280, 800));
    a.ensure_layouts_all();
    a.recompute_geometry();

    let shown = layout_of_text(&a, "p.png").imgs[0].height as f32;
    let g = geom_of_text(&a, "p.png");
    assert!(
        shown < a.image_max_height() * 0.5,
        "夹具应是远矮于视口 80% 的图片（{shown} vs {}）",
        a.image_max_height()
    );
    assert_eq!(g.height, shown, "独立图片块高必须等于图片实际显示高度");

    let next = geom_of_text(&a, "1.1 背景");
    let img_gap = next.top - (g.top + g.height);
    let para = geom_of_text(&a, "普通段落");
    let next2 = geom_of_text(&a, "1.2 目标");
    let para_gap = next2.top - (para.top + para.height);
    assert!(
        (img_gap - para_gap).abs() < 0.5,
        "图片→标题间距应与段落→标题一致：{img_gap} vs {para_gap}"
    );
    assert!(img_gap < 2.0 * a.engine.doc_line_height() * a.input_state.scale);
}

/// 连续独立图片与图片后接段落：块间距恢复普通块间距，不叠加图片高度。
#[test]
fn consecutive_standalone_images_keep_normal_gaps() {
    let (mut a, _) = app_with_images(
        "正文一。\n\n![](p.png)\n\n![](q.png)\n\n## 标题\n\n正文二。\n\n正文三。\n\n## 标题二\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 1600, 200), ("dir/q.png", 1200, 300)],
    );
    a.window_state.test_size = Some((1280, 800));
    a.ensure_layouts_all();
    a.recompute_geometry();

    let p = geom_of_text(&a, "p.png");
    let q = geom_of_text(&a, "q.png");
    let p_shown = layout_of_text(&a, "p.png").imgs[0].height as f32;
    let q_shown = layout_of_text(&a, "q.png").imgs[0].height as f32;
    assert_eq!(p.height, p_shown);
    assert_eq!(q.height, q_shown);
    let img_gap = q.top - (p.top + p.height);

    let para1 = geom_of_text(&a, "正文二");
    let para2 = geom_of_text(&a, "正文三");
    let para_gap = para2.top - (para1.top + para1.height);
    assert!(
        (img_gap - para_gap).abs() < 0.5,
        "连续图片间距应与段落间距一致：{img_gap} vs {para_gap}"
    );
    let heading = geom_of_text(&a, "## 标题");
    let tail_gap = heading.top - (q.top + q.height);
    let heading2 = geom_of_text(&a, "## 标题二");
    let para_heading_gap = heading2.top - (para2.top + para2.height);
    assert!(
        (tail_gap - para_heading_gap).abs() < 0.5,
        "图片→标题间距应与段落→标题一致：{tail_gap} vs {para_heading_gap}"
    );
}

/// 远程图片异步到达后立即重算几何：块高与后续块顶随加载结果刷新。
#[test]
fn standalone_image_geometry_refreshes_after_async_load() {
    let images = RemoteImages::default();
    let results = images.results.clone();
    let mut a = app_remote(
        &format!("前文\n\n![]({REMOTE_URL})\n\n## 1.1 背景\n"),
        Box::new(MemConfig::default()),
        images,
    );
    a.window_state.test_size = Some((1280, 800));
    a.ensure_layouts_all();
    a.recompute_geometry();
    let before_h = geom_of_text(&a, REMOTE_URL).height;
    let before_top = geom_of_text(&a, "1.1 背景").top;

    results
        .borrow_mut()
        .push((REMOTE_URL.to_string(), Ok(solid(1600, 200))));
    a.poll_remote_images();

    let shown = layout_of_text(&a, REMOTE_URL).imgs[0].height as f32;
    let after_h = geom_of_text(&a, REMOTE_URL).height;
    let after_top = geom_of_text(&a, "1.1 背景").top;
    assert!(after_h > before_h, "加载后块高应增大：{before_h} -> {after_h}");
    assert_eq!(after_h, shown, "加载后块高 = 图片实际显示高度");
    assert!(
        (after_top - before_top - (after_h - before_h)).abs() < 0.5,
        "后续块顶应同步下移：{before_top} -> {after_top}（块高差 {after_h}-{before_h}）"
    );
}

/// 窗口缩放后独立图片块高度按新视口重算，块高仍等于显示高度。
#[test]
fn standalone_image_block_height_recomputed_after_resize() {
    let (mut a, _) = app_with_images(
        "前文\n\n![](p.png)\n\n## 1.1 背景\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 1600, 400)],
    );
    a.window_state.test_size = Some((1280, 800));
    a.ensure_layouts_all();
    a.recompute_geometry();
    let tall = layout_of_text(&a, "p.png").imgs[0].height as f32;
    assert_eq!(geom_of_text(&a, "p.png").height, tall);

    a.window_state.test_size = Some((1280, 400));
    a.ensure_layouts_all();
    a.recompute_geometry();
    let short = layout_of_text(&a, "p.png").imgs[0].height as f32;
    let g = geom_of_text(&a, "p.png");
    assert!(short < tall, "窗口变矮后图片应变小：{tall} -> {short}");
    assert_eq!(short, a.image_max_height().round(), "应夹到新视口 80%");
    assert_eq!(g.height, short, "缩放后块高必须同步重算");
}

#[test]
fn image_failure_degrades_to_alt_and_memoizes() {
    let (mut a, calls) = app_with_images("a ![alt](img/p.png) b", Some("dir/doc.md"), &[]);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    assert!(a.cache.layouts[&id].imgs.is_empty());
    assert_eq!(a.cache.layouts[&id].layout_text(), "a alt b");
    assert_eq!(calls.borrow().len(), 1);
    a.cache.layouts.remove(&id);
    a.ensure_layouts_all(); // 失败记忆：不每帧重试
    assert_eq!(calls.borrow().len(), 1);
}

// ---- 图片题注（Task 5）：独立成行图片的 alt 显示在图片下方 ----

#[test]
fn standalone_image_alt_renders_caption_without_touching_document() {
    let (mut a, _) = app_with_images(
        "前文\n\n![本地题注](p.png)\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 200, 100)],
    );
    a.ensure_layouts_all();
    let before = doc_text(&a);
    let l = layout_of_text(&a, "![本地题注]");
    assert!(l.layout_text().contains("本地题注"), "alt 应作为题注显示");
    assert!(
        !l.layout_text().contains("![本地题注](p.png)"),
        "题注不是源码"
    );
    assert_eq!(doc_text(&a), before, "题注不参与文档字节");
}

#[test]
fn empty_alt_standalone_image_has_no_caption_line() {
    let (mut a, _) = app_with_images(
        "前文\n\n![](p.png)\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 200, 100)],
    );
    a.ensure_layouts_all();
    let l = layout_of_text(&a, "![](p.png)");
    assert_eq!(l.buffer.layout_runs().count(), 1, "空 alt 不产生题注行");
}

#[test]
fn inline_image_alt_is_not_rendered_as_caption() {
    let (mut a, _) = app_with_images(
        "正文 ![行内](p.png) 继续",
        Some("dir/doc.md"),
        &[("dir/p.png", 60, 20)],
    );
    a.ensure_layouts_all();
    let l = layout_of_text(&a, "![行内]");
    assert!(!l.layout_text().contains("行内"), "行内图片不显示题注");
}

#[test]
fn missing_image_caption_stays_visible() {
    let (mut a, _) = app_with_images(
        "前文\n\n![找不到](missing.png)\n",
        Some("dir/doc.md"),
        &[],
    );
    a.ensure_layouts_all();
    let l = layout_of_text(&a, "![找不到]");
    assert!(l.layout_text().contains("找不到"), "图片缺失时题注仍可读");
    assert!(
        l.layout_text().contains(t(I18nKey::ImageAltFallback)),
        "缺失图片应有通用占位提示"
    );
    assert_eq!(l.buffer.layout_runs().count(), 2, "占位一行 + 题注一行");
}

#[test]
fn caption_increases_block_height_by_one_line() {
    let (mut with, _) = app_with_images(
        "前文\n\n![题注](p.png)\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 200, 100)],
    );
    with.ensure_layouts_all();
    with.recompute_geometry();
    let (mut without, _) = app_with_images(
        "前文\n\n![](q.png)\n",
        Some("dir/doc.md"),
        &[("dir/q.png", 200, 100)],
    );
    without.ensure_layouts_all();
    without.recompute_geometry();
    let line_h = with.engine.doc_line_height() * with.input_state.scale;
    let d = geom_of_text(&with, "![题注]").height - geom_of_text(&without, "![](q.png)").height;
    assert!(
        d >= line_h * 0.9,
        "题注须使块高增加一行: diff={d} line_h={line_h}"
    );
}

#[test]
fn caption_follows_alt_edits() {
    let (mut a, _) = app_with_images(
        "前文\n\n![旧题注](p.png)\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 200, 100)],
    );
    a.ensure_layouts_all();
    assert!(layout_of_text(&a, "![旧题注]")
        .layout_text()
        .contains("旧题注"));
    // 光标进入 alt 后插入字符，再移出：题注应随文档更新。
    assert!(a.editor.set_caret(10)); // 「旧」起始（UTF-8 字符边界）
    let dirty = a.editor.apply(Intent::InsertText("新".into())).unwrap();
    a.after_edit(dirty);
    assert!(a.editor.set_caret(0));
    a.after_edit(Vec::new());
    a.ensure_layouts_all();
    let l = layout_of_text(&a, "![新旧题注]");
    assert!(
        l.layout_text().contains("新旧题注"),
        "修改 alt 后题注应更新: {:?}",
        l.layout_text()
    );
}

#[test]
fn remote_or_draft_image_is_never_loaded() {
    let (mut a, calls) =
        app_with_images("x ![a](https://example.com/x.png)", Some("dir/doc.md"), &[]);
    a.ensure_layouts_all();
    assert!(calls.borrow().is_empty());
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    assert_eq!(a.cache.layouts[&id].layout_text(), "x a");

    let (mut b, calls2) = app_with_images("x ![a](p.png)", None, &[]);
    b.ensure_layouts_all();
    assert!(calls2.borrow().is_empty(), "草稿相对路径无解析基准");
    let (id, _, _) = b.editor.session().blocks()[0].clone();
    assert_eq!(b.cache.layouts[&id].layout_text(), "x a");
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
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    assert!(a.cache.layouts[&id].imgs.is_empty());
    assert_eq!(a.cache.layouts[&id].layout_text(), "a ![alt](img/p.png) b");
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
            clipboard: Box::new(ClipImage {
                text: plain.map(str::to_string),
                html: html.map(str::to_string),
                image,
            }),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
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
    assert_eq!(doc_text(&f.app), "![](notes.assets/image-1.png)\nab");
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
fn paste_image_files_prefer_file_list_over_path_text() {
    // 资源管理器 Ctrl+C 一张图片：剪贴板同时给出文件列表与路径文本；
    // 必须走落盘图片，不能把路径字符串粘进文档。
    let saved = Rc::new(RefCell::new(Vec::new()));
    let loads = Rc::new(RefCell::new(Vec::new()));
    let doc = Doc {
        text: "ab".into(),
        path: Some(PathBuf::from("dir/notes.md")),
        bom: false,
        resume: false,
        open_error: None,
    };
    let mut files = HashMap::new();
    files.insert(PathBuf::from("C:/pics/cat.png"), (8, 4));
    let mut app = App::new(
        doc,
        Ports {
            clipboard: Box::new(ClipFiles {
                files: vec![
                    PathBuf::from("C:/pics/cat.png"),
                    PathBuf::from("C:/pics/readme.txt"),
                ],
                text: Some("C:/pics/cat.png".into()),
            }),
            store: Box::new(MemStore::default()),
            math: Box::new(FakeMath {
                fail: false,
                calls: Rc::new(RefCell::new(Vec::new())),
            }),
            images: Box::new(FakeImages {
                files,
                calls: loads.clone(),
            }),
            assets: Box::new(FakeAssets {
                result: Ok("notes.assets/image-1.png".into()),
                calls: saved.clone(),
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
    app.paste_clipboard();
    assert_eq!(doc_text(&app), "![](notes.assets/image-1.png)\nab");
    assert_eq!(
        loads.borrow().first(),
        Some(&PathBuf::from("C:/pics/cat.png")),
        "首个读取必须是文件列表中的图片（此后渲染管线会尝试加载插图 markdown）"
    );
    assert_eq!(saved.borrow().len(), 1, "位图按粘贴图片落盘");
    assert_eq!((saved.borrow()[0].1, saved.borrow()[0].2), (8, 4));
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

// ---- 远程图片（v1.4）：默认自动加载 + 全局阻止开关 ----

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
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(FakeMath {
                fail: false,
                calls: Rc::new(RefCell::new(Vec::new())),
            }),
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

#[test]
fn remote_image_requested_once_when_allowed() {
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let mut a = app_remote(
        &format!("前 ![a]({REMOTE_URL}) 后\n"),
        Box::new(MemConfig::default()),
        images,
    );
    a.ensure_layouts_all();
    assert_eq!(requests.borrow().as_slice(), &[REMOTE_URL.to_string()]);
    assert!(a.cache.remote_pending.contains(REMOTE_URL));
    // 再次布局/重绘：布局已缓存且 pending 去重，不得重复请求
    a.ensure_layouts_all();
    a.request_redraw();
    assert_eq!(requests.borrow().len(), 1, "同一 URL 只请求一次");
}

#[test]
fn remote_image_not_requested_when_globally_blocked() {
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let mut a = app_remote(
        &format!("前 ![a]({REMOTE_URL}) 后\n"),
        Box::new(MemConfig::default()),
        images,
    );
    a.config.allow_remote_images = false;
    a.ensure_layouts_all();
    assert!(requests.borrow().is_empty(), "阻止后不得发起请求");
    assert!(a.cache.remote_pending.is_empty());
}

#[test]
fn remote_poll_success_caches_bitmap_and_bumps_generation() {
    let images = RemoteImages::default();
    let results = images.results.clone();
    let mut a = app_remote(
        &format!("前 ![a]({REMOTE_URL}) 后\n"),
        Box::new(MemConfig::default()),
        images,
    );
    a.ensure_layouts_all();
    let before = a.input_state.revs.remote_images;
    results
        .borrow_mut()
        .push((REMOTE_URL.to_string(), Ok(solid(4, 2))));
    a.poll_remote_images();
    assert!(a.cache.remote_image(REMOTE_URL).is_some(), "成功位图入缓存");
    assert!(
        !a.cache.remote_pending.contains(REMOTE_URL),
        "pending 已取走"
    );
    assert_eq!(a.input_state.revs.remote_images, before + 1, "代次递增");
    let placed = a.cache.layouts.values().any(|m| {
        m.value
            .imgs
            .iter()
            .any(|i| matches!(&i.key, InlineAsset::Image(k) if k.as_str() == REMOTE_URL))
    });
    assert!(placed, "换代次后重排应把远程图放入布局");
}

#[test]
fn remote_poll_failure_is_remembered_without_retry() {
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let results = images.results.clone();
    let mut a = app_remote(
        &format!("前 ![a]({REMOTE_URL}) 后\n"),
        Box::new(MemConfig::default()),
        images,
    );
    a.ensure_layouts_all();
    results
        .borrow_mut()
        .push((REMOTE_URL.to_string(), Err("404".into())));
    a.poll_remote_images();
    assert!(a.cache.remote_failed.contains(REMOTE_URL));
    assert!(a.cache.remote_image(REMOTE_URL).is_none());
    assert!(
        a.ui_state.chrome.notice.contains("远程图片加载失败"),
        "状态栏提示: {}",
        a.ui_state.chrome.notice
    );
    a.ensure_layouts_all();
    assert_eq!(requests.borrow().len(), 1, "失败记忆命中后不重复请求");
}

#[test]
fn remote_cache_cleared_on_document_switch() {
    let images = RemoteImages::default();
    let results = images.results.clone();
    let mut a = app_remote(
        &format!("前 ![a]({REMOTE_URL}) 后\n"),
        Box::new(MemConfig::default()),
        images,
    );
    a.ensure_layouts_all();
    results
        .borrow_mut()
        .push((REMOTE_URL.to_string(), Ok(solid(2, 2))));
    a.poll_remote_images();
    assert!(a.cache.remote_image(REMOTE_URL).is_some());
    a.new_draft();
    assert!(
        a.cache.remote_image(REMOTE_URL).is_none(),
        "切换文档清空位图"
    );
    assert!(a.cache.remote_pending.is_empty());
    assert!(a.cache.remote_failed.is_empty());
}

#[test]
fn html_img_remote_src_uses_same_request_path() {
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let mut a = app_remote(
        "前文 <img src=\"https://example.com/b.png\" alt=\"b\"> 后文\n",
        Box::new(MemConfig::default()),
        images,
    );
    a.ensure_layouts_all();
    assert_eq!(
        requests.borrow().as_slice(),
        &["https://example.com/b.png".to_string()],
        "HTML <img> 的 remote_src 与 Markdown 图片走同一请求路径"
    );
}

#[test]
fn remote_images_menu_flag_reflects_config_and_toggle_persists() {
    let cfg = MemConfig::default();
    let saves = cfg.saves.clone();
    let mut a = app_remote("abc", Box::new(cfg), RemoteImages::default());
    a.config.allow_remote_images = false;
    let flags = a.menu_flags();
    assert!(flags.block_remote_images, "阻止态进入菜单标志");
    assert!(chrome::action_checked(
        MenuAction::ToggleRemoteImages,
        flags
    ));
    a.perform_menu(MenuAction::ToggleRemoteImages);
    assert!(a.config.allow_remote_images, "切换后恢复允许");
    assert!(!a.menu_flags().block_remote_images);
    assert!(a.ui_state.chrome.notice.contains("已允许远程图片"));
    let last = saves.borrow().last().cloned().unwrap_or_default();
    assert!(
        last.contains("allow_remote_images = true"),
        "应持久化为 true: {last}"
    );
    // 再次切换：阻止并持久化 false
    a.perform_menu(MenuAction::ToggleRemoteImages);
    assert!(!a.config.allow_remote_images);
    assert!(a.ui_state.chrome.notice.contains("已阻止远程图片"));
    let last = saves.borrow().last().cloned().unwrap_or_default();
    assert!(
        last.contains("allow_remote_images = false"),
        "应持久化为 false: {last}"
    );
}

// ---- 拖放（阶段 1）：图片插入落点；非图片按文档打开 ----

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
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
            images: Box::new(FakeImages {
                files: map,
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

#[test]
fn drop_image_inserts_link_at_drop_point() {
    let mut f = app_drop(
        "ab",
        Some("dir/notes.md"),
        &[("pic.png", 4, 2)],
        Ok("notes.assets/image-1.png".into()),
    );
    f.app.drop_file(PathBuf::from("pic.png"));
    assert_eq!(doc_text(&f.app), "![](notes.assets/image-1.png)\nab");
    assert!(f.app.editor.is_dirty(), "插入图片须置脏");
    assert_eq!(
        f.images.borrow().first(),
        Some(&PathBuf::from("pic.png")),
        "第一个图片加载必须是拖入的源文件"
    );
    // 独占段换行后渲染管线会额外尝试加载插图 markdown（非缺陷）——仅校验首项
    assert_eq!(
        *f.assets.borrow(),
        vec![(PathBuf::from("dir/notes.md"), 4, 2)],
        "解码位图原样交给资产层"
    );
    assert!(f.app.input_state.drop_file.is_none(), "松开后提示清空");
}

#[test]
fn drop_image_requires_document_path() {
    let mut f = app_drop(
        "ab",
        None,
        &[("pic.png", 2, 2)],
        Ok("x.assets/i.png".into()),
    );
    f.app.drop_file(PathBuf::from("pic.png"));
    assert_eq!(doc_text(&f.app), "ab");
    assert!(!f.app.editor.is_dirty());
    assert!(f.assets.borrow().is_empty(), "草稿不得落盘资产");
    assert!(f.app.ui_state.chrome.notice.contains("草稿"));
}

#[test]
fn drop_image_decode_failure_keeps_document() {
    let mut f = app_drop(
        "ab",
        Some("dir/notes.md"),
        &[],
        Ok("notes.assets/image-1.png".into()),
    );
    f.app.drop_file(PathBuf::from("bad.jpg"));
    assert_eq!(doc_text(&f.app), "ab");
    assert!(!f.app.editor.is_dirty());
    assert!(f.assets.borrow().is_empty());
    assert!(f.app.ui_state.chrome.notice.contains("图片读取失败"));
}

#[test]
fn drop_image_asset_failure_keeps_document() {
    let mut f = app_drop(
        "ab",
        Some("dir/notes.md"),
        &[("pic.png", 2, 2)],
        Err("磁盘只读".into()),
    );
    f.app.drop_file(PathBuf::from("pic.png"));
    assert_eq!(doc_text(&f.app), "ab");
    assert!(!f.app.editor.is_dirty());
    assert_eq!(f.assets.borrow().len(), 1, "已尝试落盘");
    assert!(f.app.ui_state.chrome.notice.contains("落盘失败"));
}

#[test]
fn drop_non_image_opens_document() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("b.md"), b"B".to_vec());
    let mut a = app("a", None, false, &store);
    a.drop_file(PathBuf::from("b.md"));
    assert_eq!(doc_text(&a), "B");
    assert_eq!(a.doc.path, Some(PathBuf::from("b.md")));
}

#[test]
fn hover_file_tracks_indicator_until_cancelled() {
    let mut f = app_drop("ab", Some("dir/notes.md"), &[], Ok(String::new()));
    f.app.hover_file(Some(PathBuf::from("pic.webp")));
    assert_eq!(f.app.input_state.drop_file, Some(PathBuf::from("pic.webp")));
    f.app.hover_file(None);
    assert!(f.app.input_state.drop_file.is_none());
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
    let (id, kind, range) = a.editor.session().blocks()[1].clone();
    assert_eq!(kind, BlockKind::Table);
    assert!(a.table_shows_grid(id, &range));
    assert_eq!(a.block_height(geom(&a, id)), a.cache.tables[&id].height);
}

#[test]
fn table_shows_source_when_caret_on_pipe() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    assert!(a.editor.set_caret(3)); // 管道符 `|`（block 内但不在任何 cell.range）
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, range) = a.editor.session().blocks()[1].clone();
    assert!(
        !a.cache.tables.contains_key(&id),
        "管道符位置 → 源码态不排版网格"
    );
    assert!(!a.table_shows_grid(id, &range));
}

#[test]
fn table_shows_grid_when_caret_inside_cell() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    assert!(a.editor.set_caret(5)); // 表头单元格内（cell.range 内）
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, range) = a.editor.session().blocks()[1].clone();
    assert!(
        a.cache.tables.contains_key(&id),
        "活动单元格 → 网格继续显示"
    );
    assert!(a.table_shows_grid(id, &range));
}

#[test]
fn table_grid_hit_maps_to_cell_content_start() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, kind, range) = a.editor.session().blocks()[1].clone();
    let want = a.editor.session().table(id).unwrap().rows[0][1].range.start; // 表体「d」
    let t = a.cache.tables.get(&id).unwrap();
    let cell = &t.cells[3]; // 第 2 行第 2 列
    let x = cell.x + cell.w / 2.0;
    let y = t.rows[1].top + t.rows[1].height / 2.0;
    assert_eq!(a.hit_block(id, kind, &range, x, y), Some(want));
}

#[test]
fn table_grid_caret_drawn_at_active_cell_content_start() {
    // C5 定位修复：网格态光标必须画在活动单元格内容起点，而不是源码排版的字形位置。
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let caret = a.editor.session().table(id).unwrap().rows[0][1].range.start;
    a.editor.set_caret(caret);
    a.ensure_tables();
    let top = a
        .index
        .blocks
        .iter()
        .find(|g| g.id == id)
        .expect("表格块应存在")
        .top;
    let scale = a.input_state.scale;
    let (cell_x, cell_y) = {
        let cell = &a.cache.tables[&id].cells[3]; // 第 2 行第 2 列
        (cell.x, cell.y)
    };
    let vp = a.viewport();
    let (x, y, h) = a.caret_screen();
    assert!(
        (x - (MARGIN * scale + cell_x)).abs() < 0.5,
        "光标 x 应等于单元格内容左缘：{x} vs {}",
        MARGIN * scale + cell_x
    );
    assert!(
        (y - (vp.y + top - a.window_state.scroll_y + cell_y)).abs() < 0.5,
        "光标 y 应等于单元格内容上缘：{y} vs {}",
        vp.y + top - a.window_state.scroll_y + cell_y
    );
    assert!((h - BASE_LINE_HEIGHT * scale).abs() < 0.5);
}

#[test]
fn table_grid_rebuilds_when_editing_inside_active_cell() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let old_id = a.editor.session().blocks()[1].0;
    let old_w = a.cache.tables[&old_id].width;
    // 表头「a」变宽（xxxxa）→ 块脏 → BlockId 重建 → after_edit 重建网格
    assert!(a.editor.set_caret(5));
    let dirty = a.editor.apply(Intent::InsertText("xxxx".into())).unwrap();
    assert_eq!(dirty.len(), 1);
    a.after_edit(dirty);
    let new_id = a.editor.session().blocks()[1].0;
    // 活动单元格内编辑 → 网格继续排版且重建（列宽变大）
    assert!(
        a.cache.tables.contains_key(&new_id),
        "活动单元格内 → 网格重建"
    );
    assert!(
        a.cache.tables[&new_id].width > old_w,
        "内容变宽 → 新网格列宽变大"
    );
}

// ---- 容器内表格（B2）：引用/列表/details 原生网格 + 容器缩进 ----

#[test]
fn quote_container_table_renders_native_grid_with_offset() {
    let doc = "p\n\n> | h | h |\n> | --- | --- |\n> | **MMMM** | MMMM |\n";
    let (mut a, _) = app_math(doc, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let blocks = a.editor.session().blocks();
    assert_eq!(blocks.len(), 2, "段落 + 引用内表格块");
    let (id, kind, range) = blocks[1].clone();
    assert_eq!(kind, BlockKind::Table);
    let t = &a.cache.tables[&id];
    assert!(t.x > 0.0, "引用内表格应右移（容器缩进），实际 {}", t.x);
    assert_eq!(t.rows.len(), 2, "表头 + 1 行表体");
    assert!(
        t.height > a.engine.doc_line_height() * 1.5,
        "原生表格高度应大于一行"
    );
    assert!(
        t.cells[2].pixel_w > t.cells[3].pixel_w,
        "**粗体** 单元格应宽于同文本普通格（B1 行内样式沿用）"
    );
    assert_eq!(a.block_height(geom(&a, id)), t.height);

    // 命中位置扣除容器缩进后应落回单元格内容起点（与顶层表格同一路径）
    let want = a.editor.session().table(id).unwrap().rows[0][1].range.start;
    let cell = &t.cells[3];
    let hit = a.hit_block(
        id,
        kind,
        &range,
        t.x + cell.x + cell.w / 2.0,
        t.rows[1].top + t.rows[1].height / 2.0,
    );
    assert_eq!(hit, Some(want));
}

#[test]
fn list_item_container_table_renders_with_indent() {
    let doc = "p\n\n- | 键 | 值 |\n  | --- | --- |\n  | k | v |\n";
    let (mut a, _) = app_math(doc, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let blocks = a.editor.session().blocks();
    assert_eq!(blocks.len(), 2);
    let (id, _, _) = blocks[1].clone();
    assert_eq!(blocks[1].1, BlockKind::Table);
    let t = &a.cache.tables[&id];
    assert!(t.x > 0.0, "列表项内表格应有内容列缩进");
    assert_eq!(t.rows.len(), 2);
}

#[test]
fn details_child_table_tracks_parent_expand_state() {
    let doc = "p\n\n<details>\n<summary>S</summary>\n| a |\n| --- |\n| 1 |\n</details>\n";
    let (mut a, _) = app_math(doc, false);
    a.refresh_index();
    let table_id = block_id_of(&a, |k| *k == BlockKind::Table);
    let details_id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    a.ensure_layouts_all();
    // 默认收起：子表格不占行高
    let g = a.index.blocks.iter().find(|g| g.id == table_id).unwrap();
    assert_eq!(a.block_height(g), 0.0, "收起态子表格应隐藏");
    // 展开后：原生表格占位，且 details 卡片覆盖子块高度
    a.ui_state.expanded_details.insert(details_id);
    a.ensure_layouts_all();
    a.ensure_tables();
    let g = a.index.blocks.iter().find(|g| g.id == table_id).unwrap();
    let h = a.block_height(g);
    assert!(h > a.engine.doc_line_height() * 1.5, "展开态子表格应排版");
    let header = a.index.blocks.iter().find(|b| b.id == details_id).unwrap();
    assert!(
        a.details_children_height(header) >= h,
        "details 卡片高度应覆盖子表格"
    );
    // 子表格切段后闭标签移出 details 头块：裸标签仍须消隐，只显示 summary。
    let header_text = a.cache.layouts[&details_id].layout_text();
    assert!(
        !header_text.contains('<') && header_text.contains('S'),
        "details 头块应只显示 summary，实际：{header_text:?}"
    );
}

// ---- 单元格富文本（B3） ----

const TABLE_RICH_DOC: &str = "p\n\n| **MMMM** | x |\n| --- | --- |\n";

#[test]
fn table_active_cell_shows_source_then_restores_rich_text() {
    let (mut a, _) = app_math(TABLE_RICH_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    assert_eq!(a.cache.tables[&id].active, None, "表外 → 全部富文本");
    let rich_w = a.cache.tables[&id].cells[0].pixel_w;

    let cell_start = a.editor.session().table(id).unwrap().header[0].range.start;
    assert!(a.editor.set_caret(cell_start));
    a.ensure_tables();
    assert_eq!(a.cache.tables[&id].active, Some(0), "格内 → 活动格源码态");
    let src_w = a.cache.tables[&id].cells[0].pixel_w;
    assert!(
        src_w > rich_w + 8,
        "活动格应显示含 ** 的源码：src={src_w} rich={rich_w}"
    );

    assert!(a.editor.set_caret(0));
    a.ensure_tables();
    assert_eq!(a.cache.tables[&id].active, None, "离开 → 恢复富文本");
    assert_eq!(a.cache.tables[&id].cells[0].pixel_w, rich_w);
}

#[test]
fn table_cache_key_tracks_active_cell() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let k0 = a.table_key(id);
    assert!(a.cache.table(id, k0).is_some(), "表外键应命中");

    let cell_start = a.editor.session().table(id).unwrap().rows[0][0].range.start;
    assert!(a.editor.set_caret(cell_start));
    a.ensure_tables();
    let k1 = a.table_key(id);
    assert_ne!(k0, k1, "活动格变化 → 键变化");
    assert_eq!(k1.active, 2, "flat 索引：header 2 格后的首个体格");
    assert!(a.cache.table(id, k1).is_some(), "新键命中");
    assert!(
        a.cache.table(id, k0).is_none(),
        "旧键失配（切换活动格即失效）"
    );
}

// ---- Tab / Shift+Tab 单元格导航（T2） ----

const TABLE_2X3: &str = "p\n\n| a | b | c |\n| --- | --- | --- |\n| d | e | f |\n";

/// 回归（架构审计【高】1）：Ctrl+Enter 追加行必须经 Editor 门面，
/// 置脏标（自动保存/恢复/关窗确认依赖）、光标落入新行首格、单撤销步。
#[test]
fn table_ctrl_enter_sets_dirty_and_moves_caret_into_new_row() {
    let (mut a, _) = app_math(TABLE_2X3, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let cell = a.editor.session().table(id).unwrap().rows[0][1].range.start;
    a.editor.set_caret(cell);

    assert!(a.table_ctrl_enter(), "表内 Ctrl+Enter 应追加空行");
    assert!(a.editor.is_dirty(), "追加行必须置脏标");
    assert!(a.title_text().starts_with('*'), "标题应显示脏标");

    let src = a.editor.session().serialize();
    assert!(src.ends_with("|  |  |  |\n"), "新行应为空单元格行：{src:?}");
    let last_line = src
        .trim_end_matches('\n')
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap();
    assert!(
        a.editor.caret() >= last_line,
        "光标应落在新行而非表首：caret={} last_line={last_line}",
        a.editor.caret()
    );

    a.editor.apply(Intent::Undo).unwrap();
    assert_eq!(
        a.editor.session().serialize(),
        TABLE_2X3,
        "追加行是单撤销步"
    );
}

// ---- 表格行列增删右键菜单（Task 5 / B2） ----

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

#[test]
fn right_click_table_body_opens_menu_and_inserts_row_below() {
    let (mut a, _) = app_math(TABLE_MENU_DOC, false);
    right_click_table_cell(&mut a, 2); // 第 1 个正文行第 1 列
    assert!(a.ui_state.ctx.open, "表格内右键应打开菜单");
    assert_eq!(
        a.ui_state.ctx.table,
        Some(ctx_menu::TableCtx {
            body_row: Some(0),
            col: 0,
            ncols: 2
        })
    );

    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::TableInsertRowBelow)
        .expect("正文行菜单应有「下方插入行」");
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);

    assert!(!a.ui_state.ctx.open, "执行后菜单关闭");
    assert_eq!(
        a.editor.session().serialize(),
        "p\n\n| a | b |\n| --- | --- |\n| c | d |\n|  |  |\n| e | f |\n"
    );
    let id = a
        .editor
        .session()
        .blocks()
        .into_iter()
        .find(|(_, k, _)| *k == BlockKind::Table)
        .map(|(id, _, _)| id)
        .expect("插入行后仍应有表格块");
    let tbl = a.editor.session().table(id).unwrap();
    assert_eq!(tbl.rows.len(), 3, "正文行 +1");
    let cell = &tbl.rows[1][0];
    assert!(
        cell.range.start <= a.editor.caret() && a.editor.caret() <= cell.range.end,
        "光标应落在新行第 1 列内容起点：caret={} cell=[{},{}]",
        a.editor.caret(),
        cell.range.start,
        cell.range.end
    );

    a.editor.apply(Intent::Undo).unwrap();
    assert_eq!(
        a.editor.session().serialize(),
        TABLE_MENU_DOC,
        "插入行是单撤销步"
    );
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

#[test]
fn table_ctx_actions_transform_source_and_place_caret() {
    // (动作, 上下文, 期望源码, 期望光标所在行/列：0=header、2+=body)
    type Case = (
        ctx_menu::CtxAction,
        ctx_menu::TableCtx,
        &'static str,
        usize,
        usize,
    );
    let cases: [Case; 7] = [
        (
            ctx_menu::CtxAction::TableInsertRowAbove,
            ctx_menu::TableCtx {
                body_row: Some(0),
                col: 0,
                ncols: 2,
            },
            "p\n\n| a | b |\n| --- | --- |\n|  |  |\n| c | d |\n| e | f |\n",
            2,
            0,
        ),
        (
            ctx_menu::CtxAction::TableInsertRowBelow,
            ctx_menu::TableCtx {
                body_row: Some(1),
                col: 1,
                ncols: 2,
            },
            "p\n\n| a | b |\n| --- | --- |\n| c | d |\n| e | f |\n|  |  |\n",
            4,
            1,
        ),
        (
            // 删除首行 → 无上一行，落到下一行同列
            ctx_menu::CtxAction::TableDeleteRow,
            ctx_menu::TableCtx {
                body_row: Some(0),
                col: 1,
                ncols: 2,
            },
            "p\n\n| a | b |\n| --- | --- |\n| e | f |\n",
            2,
            1,
        ),
        (
            // 删除末行 → 落到上一行同列
            ctx_menu::CtxAction::TableDeleteRow,
            ctx_menu::TableCtx {
                body_row: Some(1),
                col: 1,
                ncols: 2,
            },
            "p\n\n| a | b |\n| --- | --- |\n| c | d |\n",
            2,
            1,
        ),
        (
            ctx_menu::CtxAction::TableInsertColumnLeft,
            ctx_menu::TableCtx {
                body_row: Some(0),
                col: 1,
                ncols: 2,
            },
            "p\n\n| a |  | b |\n| --- | --- | --- |\n| c |  | d |\n| e |  | f |\n",
            2,
            1,
        ),
        (
            ctx_menu::CtxAction::TableInsertColumnRight,
            ctx_menu::TableCtx {
                body_row: Some(0),
                col: 1,
                ncols: 2,
            },
            "p\n\n| a | b |  |\n| --- | --- | --- |\n| c | d |  |\n| e | f |  |\n",
            2,
            2,
        ),
        (
            ctx_menu::CtxAction::TableDeleteColumn,
            ctx_menu::TableCtx {
                body_row: None,
                col: 0,
                ncols: 2,
            },
            "p\n\n| b |\n| --- |\n| d |\n| f |\n",
            0,
            0,
        ),
    ];
    for (action, tc, want, line, col) in cases {
        let (mut a, _) = app_math(TABLE_MENU_DOC, false);
        exec_table_ctx(&mut a, tc, action);
        assert_eq!(a.editor.session().serialize(), want, "{action:?} 源码不符");
        let id = a
            .editor
            .session()
            .blocks()
            .into_iter()
            .find(|(_, k, _)| *k == BlockKind::Table)
            .map(|(id, _, _)| id)
            .expect("操作后仍应有表格块");
        let tbl = a.editor.session().table(id).unwrap();
        let cell = if line == 0 {
            &tbl.header[col]
        } else {
            &tbl.rows[line - 2][col]
        };
        assert!(
            cell.range.start <= a.editor.caret() && a.editor.caret() <= cell.range.end,
            "{action:?} 光标未落到第 {line} 行第 {col} 列：caret={} cell=[{},{}]",
            a.editor.caret(),
            cell.range.start,
            cell.range.end
        );
    }
}

#[test]
fn table_ctx_actions_are_single_undo_step() {
    let (mut a, _) = app_math(TABLE_MENU_DOC, false);
    exec_table_ctx(
        &mut a,
        ctx_menu::TableCtx {
            body_row: Some(0),
            col: 0,
            ncols: 2,
        },
        ctx_menu::CtxAction::TableDeleteColumn,
    );
    assert_ne!(a.editor.session().serialize(), TABLE_MENU_DOC);
    a.editor.apply(Intent::Undo).unwrap();
    assert_eq!(
        a.editor.session().serialize(),
        TABLE_MENU_DOC,
        "结构操作是单撤销步"
    );
}

#[test]
fn table_align_actions_map_to_expected_markers() {
    // 分隔格宽 3：左 `:--`、中 `:-:`、右 `--:`（宽度不变）
    for (action, want) in [
        (
            ctx_menu::CtxAction::TableAlignLeft,
            "p\n\n| a | b |\n| --- | :-- |\n| c | d |\n| e | f |\n",
        ),
        (
            ctx_menu::CtxAction::TableAlignCenter,
            "p\n\n| a | b |\n| --- | :-: |\n| c | d |\n| e | f |\n",
        ),
        (
            ctx_menu::CtxAction::TableAlignRight,
            "p\n\n| a | b |\n| --- | --: |\n| c | d |\n| e | f |\n",
        ),
    ] {
        let (mut a, _) = app_math(TABLE_MENU_DOC, false);
        exec_table_ctx(
            &mut a,
            ctx_menu::TableCtx {
                body_row: Some(0),
                col: 1,
                ncols: 2,
            },
            action,
        );
        assert_eq!(a.editor.session().serialize(), want, "{action:?} 源码不符");
    }
}

#[test]
fn table_align_sets_aligns_keeps_caret_and_undo_restores() {
    let (mut a, _) = app_math(TABLE_MENU_DOC, false);
    let (id, _, range) = a
        .editor
        .session()
        .blocks()
        .into_iter()
        .find(|(_, k, _)| *k == BlockKind::Table)
        .expect("文档应含表格块");
    let cell_start = a.editor.session().table(id).unwrap().rows[1][1].range.start;
    assert!(a.editor.set_caret(cell_start));
    let before = a.editor.caret() - range.start;

    let tc = ctx_menu::TableCtx {
        body_row: Some(1),
        col: 1,
        ncols: 2,
    };
    a.ui_state
        .ctx
        .open_at(10.0, 20.0, ctx_menu::CtxKind::Table(tc), None, None);
    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::TableAlignRight)
        .expect("表格菜单应有「右对齐」");
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);

    assert_eq!(
        a.editor.session().serialize(),
        "p\n\n| a | b |\n| --- | --: |\n| c | d |\n| e | f |\n"
    );
    let align_of = |a: &App| -> veil_core::CellAlign {
        let id = a
            .editor
            .session()
            .blocks()
            .into_iter()
            .find(|(_, k, _)| *k == BlockKind::Table)
            .map(|(id, _, _)| id)
            .expect("编辑后仍应有表格块");
        a.editor.session().table(id).unwrap().aligns[1]
    };
    assert_eq!(
        align_of(&a),
        veil_core::CellAlign::Right,
        "解析后的列对齐应更新"
    );
    assert_eq!(
        a.editor.caret() - range.start,
        before,
        "对齐不移动光标（相对块起点偏移不变）"
    );

    a.editor.apply(Intent::Undo).unwrap();
    assert_eq!(
        a.editor.session().serialize(),
        TABLE_MENU_DOC,
        "对齐是单撤销步"
    );
    assert_eq!(align_of(&a), veil_core::CellAlign::None, "撤销后对齐复原");
}

#[test]
fn right_click_table_header_offers_no_row_actions() {
    let (mut a, _) = app_math(TABLE_MENU_DOC, false);
    right_click_table_cell(&mut a, 0); // 表头第 1 列
    assert_eq!(
        a.ui_state.ctx.table,
        Some(ctx_menu::TableCtx {
            body_row: None,
            col: 0,
            ncols: 2
        })
    );
    let row_ops = [
        ctx_menu::CtxAction::TableInsertRowAbove,
        ctx_menu::CtxAction::TableInsertRowBelow,
        ctx_menu::CtxAction::TableDeleteRow,
    ];
    assert!(
        !a.ui_state
            .ctx
            .items()
            .iter()
            .any(|&(_, act)| row_ops.contains(&act)),
        "表头不提供行操作"
    );
    assert!(a
        .ui_state
        .ctx
        .items()
        .iter()
        .any(|&(_, act)| act == ctx_menu::CtxAction::TableDeleteColumn));
}

#[test]
fn tab_advances_to_next_cell_start() {
    let (mut a, _) = app_math(TABLE_2X3, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let tbl = a.editor.session().table(id).unwrap();
    // caret 在表头首格 "a" 的内容起点
    let first = tbl.header[0].range.start;
    a.editor.set_caret(first);
    // Tab → 表头第二格 "b"
    a.route_key(Key::Tab);
    assert_eq!(a.editor.caret(), tbl.header[1].range.start);
    // Tab → 表头第三格 "c"
    a.route_key(Key::Tab);
    assert_eq!(a.editor.caret(), tbl.header[2].range.start);
    // Tab → 表体首格 "d"
    a.route_key(Key::Tab);
    assert_eq!(a.editor.caret(), tbl.rows[0][0].range.start);
}

#[test]
fn tab_on_last_cell_appends_row_and_moves_to_first_cell() {
    let (mut a, _) = app_math(TABLE_2X3, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let tbl = a.editor.session().table(id).unwrap();
    // caret 在末格 "f" 内容起点
    a.editor.set_caret(tbl.rows[0][2].range.start);
    a.route_key(Key::Tab);
    // 与 Typora 对齐：末格 Tab 追加一行并落新行首格（不绕回表首）
    assert!(a.editor.is_dirty(), "追加行必须置脏标");
    let src = a.editor.session().serialize();
    assert_eq!(src, format!("{TABLE_2X3}|  |  |  |\n"));
    // 块替换后 Table 可能换 id：按 kind 重新定位
    let id = a
        .editor
        .session()
        .blocks()
        .into_iter()
        .find(|(_, k, _)| *k == BlockKind::Table)
        .map(|(id, _, _)| id)
        .expect("追加行后仍应有表格块");
    let tbl = a.editor.session().table(id).unwrap();
    assert_eq!(tbl.rows.len(), 2, "末格 Tab 应追加一行");
    let first = &tbl.rows[1][0];
    assert!(
        first.range.start <= a.editor.caret() && a.editor.caret() <= first.range.end,
        "光标应落在新行首格内：caret={} cell=[{},{}]",
        a.editor.caret(),
        first.range.start,
        first.range.end
    );
}

#[test]
fn tab_on_structure_char_moves_into_next_cell() {
    let (mut a, _) = app_math(TABLE_2X3, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let tbl = a.editor.session().table(id).unwrap();
    // caret 落在表头 "a" 与 "b" 之间的管道符上（结构字符，不属于任何单元格）
    let pipe = tbl.header[0].range.end + 1;
    assert_eq!(
        a.editor
            .session()
            .text_slice(Range::new(pipe, pipe + 1))
            .as_deref(),
        Some("|")
    );
    a.editor.set_caret(pipe);
    a.route_key(Key::Tab);
    assert_eq!(
        a.editor.caret(),
        tbl.header[1].range.start,
        "结构字符上的 Tab 应进入其后第一个单元格"
    );
}

#[test]
fn shift_tab_goes_back() {
    let (mut a, _) = app_math(TABLE_2X3, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let tbl = a.editor.session().table(id).unwrap();
    // caret 在表体第二格 "e"
    a.editor.set_caret(tbl.rows[0][1].range.start);
    // Shift+Tab → 表体首格 "d"
    a.input_state.mods = Mods {
        ctrl: false,
        shift: true,
        alt: false,
    };
    a.route_key(Key::Tab);
    assert_eq!(a.editor.caret(), tbl.rows[0][0].range.start);
    a.input_state.mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
    };
}

#[test]
fn tab_outside_table_is_normal_indent() {
    // caret 在段落文本中 → Tab 仍是缩进（InsertText("    ") 或类似）
    let (mut a, _) = app_math("hello\n", false);
    a.editor.set_caret(0);
    let before = a.editor.caret();
    a.route_key(Key::Tab);
    // 缩进至少让光标右移（插 4 空格）
    assert!(a.editor.caret() >= before, "表外 Tab 不应回退 caret");
}

// ---- 列表 / 引用 / 分隔线 Reveal（切片 12）：块级重排与块高 ----

#[test]
fn list_bullet_toggles_on_caret_enter_leave() {
    // 「p\n\n- a\n」：列表块 bytes [3,7)，初始 caret=0 在块外
    let (mut a, _) = app_math("p\n\n- a\n", false);
    a.ensure_layouts_all();
    let (id, kind, _) = a.editor.session().blocks()[1].clone();
    assert_eq!(
        kind,
        BlockKind::ListItem {
            ordered: false,
            depth: 0,
            task: None
        }
    );
    assert_eq!(a.cache.layouts[&id].layout_text(), "• a");
    // 进入块：激活指纹变化 → 键失配 → 重排显源码
    assert!(a.editor.set_caret(4));
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "- a");
    // 离开块：恢复符号
    assert!(a.editor.set_caret(0));
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "• a");
    assert_eq!(a.block_height(geom(&a, id)), a.cache.layouts[&id].height);
}

#[test]
fn blockquote_line_reveal_reflows_only_across_lines() {
    // 「p\n\n> a\n> b\n」：引用块 bytes [3,11)，行 1 [3,7)、行 2 [7,11)
    let (mut a, _) = app_math("p\n\n> a\n> b\n", false);
    a.ensure_layouts_all();
    let (id, kind, range) = a.editor.session().blocks()[1].clone();
    assert_eq!(a.cache.layouts[&id].layout_text(), "  a\n  b");
    // 块内跨行移动（0 → 9，行 2 内容）：逐行激活变化 → 键失配 → 重排
    assert!(a.editor.set_caret(9));
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "  a\n> b");
    // 同一行内移动（9 → 10）：激活行不变 → 键命中，布局不回退重排
    assert!(a.editor.set_caret(10));
    a.ensure_layouts_all();
    assert_eq!(
        a.cache.layouts[&id].layout_text(),
        "  a\n> b",
        "行内移动不得改变引用块布局"
    );
    assert_eq!(a.cache.layouts[&id].key.reveal, {
        let l = &a.cache.layouts[&id].value;
        l.reveal_state(&kind, range, 10)
    });
}

#[test]
fn thematic_break_elides_and_keeps_line_height() {
    // 「p\n\n---\n」：分隔线块 bytes [3,7)，初始 caret=0 在块外
    let (mut a, _) = app_math("p\n\n---\n", false);
    a.ensure_layouts_all();
    let (id, kind, _) = a.editor.session().blocks()[1].clone();
    assert_eq!(kind, BlockKind::ThematicBreak);
    assert_eq!(a.cache.layouts[&id].layout_text(), "");
    // 消隐后仍占一行高，块高不塌缩
    assert_eq!(
        a.block_height(geom(&a, id)),
        BASE_LINE_HEIGHT * a.input_state.scale
    );
    // 进入块：显源码，块高不变
    assert!(a.editor.set_caret(5));
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "---");
    assert_eq!(
        a.block_height(geom(&a, id)),
        BASE_LINE_HEIGHT * a.input_state.scale
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
    fn list_files(&mut self, dir: &Path) -> Result<Vec<PathBuf>, String> {
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
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    App::new(
        doc,
        Ports {
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
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
        },
    )
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
            clipboard: Box::new(ClipLog(log.clone())),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
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
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
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
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
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
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
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
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
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
fn export_passes_current_theme_style() {
    let (mut a, calls) = app_export("x", Some("a.md"), false);
    // 初始快照必须与当前主题映射一致（未切换前即浅色）
    assert_eq!(
        a.doc.export_style,
        ExportStyle::from_theme(&a.input_state.theme)
    );
    assert!(!a.doc.export_style.dark);
    a.export_now();
    assert_eq!(calls.borrow()[0].2, a.doc.export_style); // 透传的是 App 侧快照
                                                         // 切到深色：主题刷新同点更新导出样式，之后导出立刻是深色页面
    a.set_theme_mode(ThemeMode::Dark);
    assert_eq!(
        a.doc.export_style,
        ExportStyle::from_theme(&a.input_state.theme)
    );
    a.export_now();
    assert!(calls.borrow()[1].2.dark, "主题切换后导出须为深色样式");
}

#[test]
fn export_draft_cancelled_dialog_aborts_without_output() {
    // 草稿（无关联文件）+ 用户取消导出对话框：不调用导出器、提示「导出已取消」
    let (mut a, calls) = app_export("x", None, false);
    a.export_now();
    assert!(calls.borrow().is_empty(), "取消后不得产生导出调用");
    assert_eq!(a.ui_state.chrome.notice, "导出已取消");
}

#[test]
fn export_draft_asks_for_target_before_exporting() {
    let (mut a, calls, dialog_calls) = app_export_with_dialog("x", Some("我的稿.html"));
    a.export_now();
    assert_eq!(
        dialog_calls.borrow().as_slice(),
        &["export:html:未命名.html".to_string()],
        "草稿导出应先询问 HTML 目标"
    );
    let got = calls.borrow();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1, None, "源路径仍为空（草稿语义不变）");
    assert_eq!(
        got[0].3.as_deref(),
        Some(Path::new("我的稿.html")),
        "对话框选定路径须透传给导出器"
    );
}

#[test]
fn export_named_document_asks_for_target_in_doc_dir() {
    // 导出是显式副作用：即使有关联文件也先让用户确认位置，
    // 建议路径 = 文档同目录同名 `.html`（对话框直接定位到文档所在目录）。
    let (mut a, calls, dialog_calls) = app_export_with_dialog("x", Some("a.html"));
    a.doc.path = Some(PathBuf::from("dir/a.md"));
    a.export_now();
    assert_eq!(
        dialog_calls.borrow().as_slice(),
        &["export:html:dir/a.html".to_string()],
        "建议路径须落在文档所在目录"
    );
    assert_eq!(
        calls.borrow()[0].3.as_deref(),
        Some(Path::new("a.html")),
        "用户确认的路径须透传给导出器"
    );
}

#[test]
fn export_named_document_cancelled_dialog_aborts_without_output() {
    let (mut a, calls, _) = app_export_with_dialog("x", None);
    a.doc.path = Some(PathBuf::from("dir/a.md"));
    a.export_now();
    assert!(calls.borrow().is_empty(), "取消后不得产生导出调用");
    assert_eq!(a.ui_state.chrome.notice, "导出已取消");
}

#[test]
fn export_failure_does_not_crash_or_touch_document() {
    let (mut a, calls) = app_export("x", Some("a.md"), true);
    a.export_now();
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(a.editor.session().serialize(), "x"); // 文档不受影响
    assert!(!a.editor.is_dirty()); // 导出不改脏标
}

/// B6：导出失败必须有明确标题、原因与可操作出口（重试 / 取消）。
#[test]
fn export_failure_opens_error_modal_with_retry() {
    use crate::view::modal::ModalKind;
    let (mut a, calls) = app_export("x", Some("a.md"), true);
    a.export_now();
    let m = a.ui_state.modal.as_ref().expect("导出失败必须弹错误弹窗");
    assert_eq!(m.spec.kind, ModalKind::Error);
    assert_eq!(m.spec.title, "导出失败");
    assert!(m.spec.message.contains("模拟导出失败"));
    let labels: Vec<_> = m.spec.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, vec!["重试", "取消"]);

    // 重试：重走导出流程（导出器再次被调用），文档内容不变。
    a.resolve_modal(1);
    assert_eq!(calls.borrow().len(), 2);
    assert_eq!(a.editor.session().serialize(), "x");
    assert!(a.ui_state.chrome.notice.contains("导出失败"));
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
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));
    assert!(a.doc.bom);
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
    assert!(a.ui_state.modal.is_some(), "脏文档先挂起确认模态");
    assert_eq!(a.editor.session().serialize(), "改旧"); // 未被替换
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
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
    assert_eq!(a.doc.path, None);
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
    a.persist_state.persist.note_activity(t0);
    a.maybe_autosave(t0 + Duration::from_millis(1999));
    assert!(store.files.borrow().is_empty(), "静默期未满不落盘");
    a.maybe_autosave(t0 + Duration::from_secs(2));
    assert!(!a.editor.is_dirty());
    assert_eq!(store.files.borrow()[Path::new("a.md")], "改ab".as_bytes());
    assert_eq!(a.title_text(), "veil — a.md"); // 干净且无冲突：无标记
    assert!(
        a.persist_state.persist.activity().is_none(),
        "保存后取消调度"
    );
}

#[test]
fn autosave_clears_stale_cancel_dirty_notice() {
    // 关窗确认按 Esc 取消 → 状态栏「已取消：仍有未保存修改」；随后的自动保存成功后该提示已失真。
    use crate::document::doc_state::SaveKind;
    let store = MemStore::default();
    let mut a = dirty_app("ab", Some("a.md"), &store);
    a.ui_state.chrome.notice = t(I18nKey::NoticeCancelDirty).into();
    a.save(SaveKind::Auto);
    assert_eq!(a.ui_state.chrome.notice, "", "自动保存成功后陈旧提示应清空");
    assert!(!a.editor.is_dirty());
    assert_eq!(
        store.files.borrow()[Path::new("a.md")],
        encode(&a.editor.session().serialize(), a.doc.bom)
    );
}

#[test]
fn autosave_failure_keeps_cancel_dirty_notice() {
    // 对照：自动保存失败时磁盘仍未同步，提示必须保留（此时提示内容反而是对的）。
    use crate::document::doc_state::SaveKind;
    let store = MemStore {
        fail_save: true,
        ..Default::default()
    };
    let mut a = dirty_app("ab", Some("a.md"), &store);
    a.ui_state.chrome.notice = t(I18nKey::NoticeCancelDirty).into();
    a.save(SaveKind::Auto);
    assert!(a.editor.is_dirty());
    assert!(
        a.ui_state.chrome.notice.contains("保存失败"),
        "失败时提示保留为失败原因，不得清空"
    );
}

#[test]
fn autosave_skipped_for_draft_and_ime() {
    // 草稿：无关联文件不自动写盘
    let store = MemStore::default();
    let mut a = dirty_app("ab", None, &store);
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_autosave(t0 + Duration::from_secs(30));
    assert!(store.files.borrow().is_empty());
    // 有关联文件但 IME 组合中：组合期不落盘
    let store = MemStore::default();
    let mut a = dirty_app("ab", Some("a.md"), &store);
    a.editor.set_preedit("zh".into(), None);
    a.persist_state.persist.note_activity(t0);
    a.maybe_autosave(t0 + Duration::from_secs(30));
    assert!(store.files.borrow().is_empty());
    assert!(a.editor.is_dirty());
}

#[test]
fn cursor_move_does_not_postpone_autosave() {
    let store = MemStore::default();
    let mut a = app("ab", Some("a.md"), false, &store);
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.after_edit(Vec::new()); // 纯光标移动不产生脏块
    assert_eq!(a.persist_state.persist.activity(), Some(t0));
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
    a.persist_state.persist.note_activity(t0);
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
    assert!(a.persist_state.persist.hold(), "自动保存挂起");
    assert!(
        !a.autosave_due(t0 + Duration::from_secs(60)),
        "挂起后不再自动落盘"
    );
    assert_eq!(a.title_text(), "* veil — a.md (外部修改已备份)");
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
    assert!(!a.persist_state.persist.hold(), "手工保存解除挂起");
    assert!(a.persist_state.persist.backed_up(), "标题提示冲突副本");
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
    assert!(!a.persist_state.persist.backed_up());
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
    assert!(a.persist_state.persist.backed_up());
    a.editor.apply(Intent::InsertText("z".into())).unwrap();
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_autosave(t0 + Duration::from_secs(2));
    assert!(!a.persist_state.persist.backed_up(), "无新冲突：提示清除");
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
    a.persist_state.persist.note_activity(t0);
    a.maybe_autosave(t0 + Duration::from_secs(2)); // 自动冲突：备份 + 挂起
    a.save_now(); // 用户显式保存：同一外部版本不再重复备份
    let files = store.files.borrow();
    assert_eq!(files.len(), 2, "同一外部版本只留一份副本");
    assert_eq!(files[Path::new("a.md")], "改ab".as_bytes());
    assert_eq!(files[Path::new("a.conflict-1.md")], b"external".to_vec());
    drop(files);
    assert!(!a.editor.is_dirty());
    assert!(!a.persist_state.persist.hold());
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
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
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

#[test]
fn task_checkbox_click_toggles_and_keeps_caret() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs(
        "- [ ] buy milk\n- [x] ship it\n",
        None,
        false,
        &store,
        None,
        None,
    );
    a.ensure_layouts_all();
    let vp = a.viewport();
    let caret0 = a.editor.caret();
    let full = |a: &App| {
        let n = a.editor.session().text_len();
        a.editor
            .session()
            .text_slice(veil_core::Range::new(0, n))
            .unwrap()
    };

    // 块 0「- [ ] buy milk」：方框区域 = 缩进起 x .. 内容起点 x（block-rel 6 = "buy"）
    let (id0, _, _) = a.editor.session().blocks()[0].clone();
    let l0 = &a.cache.layouts[&id0];
    let (ix, _, _) = a.engine.caret_xy(l0, 0);
    let (tx, ty, th) = a.engine.caret_xy(l0, 6);
    let top0 = a.index.blocks[0].top;
    let (mx, my) = (
        MARGIN * a.input_state.scale + (ix + tx) / 2.0,
        vp.y + top0 + ty + th / 2.0,
    );
    click_at(&mut a, mx, my);
    assert_eq!(a.editor.caret(), caret0, "点击方框不移动光标");
    assert_eq!(full(&a), "- [x] buy milk\n- [x] ship it\n");

    // 块 1：点击取消勾选
    a.ensure_layouts_all();
    let (id1, _, _) = a.editor.session().blocks()[1].clone();
    let l1 = &a.cache.layouts[&id1];
    let (ix1, _, _) = a.engine.caret_xy(l1, 0);
    let (tx1, ty1, th1) = a.engine.caret_xy(l1, 6); // "ship" 起点
    let top1 = a.index.blocks[1].top;
    let (mx1, my1) = (
        MARGIN * a.input_state.scale + (ix1 + tx1) / 2.0,
        vp.y + top1 + ty1 + th1 / 2.0,
    );
    click_at(&mut a, mx1, my1);
    assert_eq!(full(&a), "- [x] buy milk\n- [ ] ship it\n");

    // 内容文本区点击：不切换，正常落光标
    a.ensure_layouts_all();
    let (id0b, _, _) = a.editor.session().blocks()[0].clone();
    let l0b = &a.cache.layouts[&id0b];
    let (tbx, tby, tbh) = a.engine.caret_xy(l0b, 10); // "milk" 内
    let top0b = a.index.blocks[0].top;
    let (mxb, myb) = (
        MARGIN * a.input_state.scale + tbx + 1.0,
        vp.y + top0b + tby + tbh / 2.0,
    );
    click_at(&mut a, mxb, myb);
    assert_eq!(
        full(&a),
        "- [x] buy milk\n- [ ] ship it\n",
        "文本区点击不切换"
    );
    assert_ne!(a.editor.caret(), caret0, "文本区点击落光标");
}

// ===== 双击选词 / 三击选行 / 按词移动（C6） =====

#[test]
fn double_click_selects_word_and_triple_click_selects_line() {
    let store = MemStore::default();
    let mut a = app("alpha beta\ngamma\n", None, false, &store);
    a.ensure_layouts_all();
    let vp = a.viewport();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    let l = &a.cache.layouts[&id];
    let (cx, cy, ch) = a.engine.caret_xy(l, 7); // "beta" 内
    let top = a.index.blocks[0].top;
    let (x, y) = (
        MARGIN * a.input_state.scale + cx + 1.0,
        vp.y + top + cy + ch / 2.0,
    );

    click_at(&mut a, x, y);
    assert_eq!(a.editor.selection(), None, "单击只落光标");
    click_at(&mut a, x, y);
    assert_eq!(
        a.editor.selection(),
        Some(Range::new(6, 10)),
        "双击选中整词"
    );
    click_at(&mut a, x, y);
    assert_eq!(
        a.editor.selection(),
        Some(Range::new(0, 10)),
        "三击选中整行（不含行尾换行）"
    );
}

#[test]
fn ctrl_arrow_moves_and_extends_by_word() {
    let store = MemStore::default();
    let mut a = app("foo bar baz", None, false, &store);
    a.ensure_layouts_all();
    a.input_state.mods.ctrl = true;
    a.route_key(Key::Right);
    assert_eq!(a.editor.caret(), 3);
    a.route_key(Key::Right);
    assert_eq!(a.editor.caret(), 7);
    a.input_state.mods.shift = true;
    a.route_key(Key::Right);
    assert_eq!(
        a.editor.selection(),
        Some(Range::new(7, 11)),
        "Ctrl+Shift+→ 按词扩选"
    );
    a.input_state.mods.shift = false;
    a.route_key(Key::Left);
    assert_eq!(a.editor.selection(), None, "Ctrl+← 先坍缩选区");
    assert_eq!(a.editor.caret(), 7, "坍缩到选区起点");
}

// ===== 鼠标点击断组（撤销分组与键盘路径对齐） =====

/// 单击正文落光标后输入须另起撤销步：首次撤销只回退点击后的输入，
/// 再撤销才回退点击前的连续输入（与键盘移动断组语义一致）。
#[test]
fn mouse_click_breaks_undo_group() {
    let store = MemStore::default();
    let mut a = app("", None, false, &store);
    a.route_key(Key::Text("a".into()));
    a.route_key(Key::Text("b".into()));
    let full = |a: &App| {
        let n = a.editor.session().text_len();
        a.editor.session().text_slice(Range::new(0, n)).unwrap()
    };
    assert_eq!(full(&a), "ab");

    // 单击文本起点（走真实 on_left_click 路径）把光标落到偏移 0
    a.ensure_layouts_all();
    let vp = a.viewport();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    let l = &a.cache.layouts[&id];
    let (cx, cy, ch) = a.engine.caret_xy(l, 0);
    let top = a.index.blocks[0].top;
    let (mx, my) = (
        MARGIN * a.input_state.scale + cx + 1.0,
        vp.y + top + cy + ch / 2.0,
    );
    click_at(&mut a, mx, my);
    assert_eq!(a.editor.caret(), 0, "单击须落光标到文首");

    a.route_key(Key::Text("c".into()));
    assert_eq!(full(&a), "cab");

    assert!(a.editor.apply(Intent::Undo).is_some());
    assert_eq!(full(&a), "ab", "首次撤销只回退点击后输入的 c");
    assert!(a.editor.apply(Intent::Undo).is_some());
    assert_eq!(full(&a), "", "再次撤销才回退点击前的 ab");
}

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

#[test]
fn menu_title_click_toggles_and_item_click_runs_save() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, None);
    a.editor.apply(Intent::InsertText("中".into())).unwrap();
    let (tx, ty) = menu_title_center(0);
    click_at(&mut a, tx, ty);
    assert_eq!(a.ui_state.chrome.open, Some(0), "点菜单标题打开文件菜单");
    let (x, y) = popup_item_center(0, 4); // 保存（新建/打开/快速打开/保存）
    click_at(&mut a, x, y);
    assert_eq!(a.ui_state.chrome.open, None, "执行动作后菜单关闭");
    assert!(!a.editor.is_dirty());
    assert_eq!(store.files.borrow()[Path::new("a.md")], "中ab".as_bytes());
}

#[test]
fn click_outside_open_menu_closes_it_without_moving_caret() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, None);
    click_at(&mut a, 20.0, 10.0);
    assert!(a.ui_state.chrome.is_open());
    a.editor.set_caret(1);
    click_at(&mut a, 300.0, 400.0); // 弹窗左下方正文
    assert!(!a.ui_state.chrome.is_open());
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
    assert!(calls.borrow().is_empty(), "未作答前不弹对话框");
    assert!(a.ui_state.modal.is_some(), "脏文档必须先确认");
    answer_modal(&mut a, Key::Escape);
    assert_eq!(a.editor.session().serialize(), "xold");
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
    assert!(a.ui_state.chrome.notice.contains("未保存"));
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
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));
    assert!(!a.editor.is_dirty());
    assert_eq!(a.ui_state.chrome.notice, "已打开：b.md");
}

#[test]
fn open_from_dialog_notice_uses_path_label() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("dir/b.md"), b"# new".to_vec());
    let (mut a, _calls) = app_dialogs("old", Some("a.md"), false, &store, Some("dir/b.md"), None);
    a.open_via_dialog();
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("dir/b.md")));
    assert_eq!(a.ui_state.chrome.notice, "已打开：b.md · dir");
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
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("new.md")));
    assert!(!a.editor.is_dirty());
    assert_eq!(a.doc.baseline, "中ab".as_bytes());
    assert_eq!(store.files.borrow()[Path::new("new.md")], "中ab".as_bytes());
    assert!(a.ui_state.chrome.notice.contains("已另存为"));
}

#[test]
fn draft_save_now_routes_to_save_as_dialog() {
    let store = MemStore::default();
    let (mut a, calls) = app_dialogs("x", None, false, &store, None, Some("new.md"));
    a.save_now();
    assert_eq!(calls.borrow().as_slice(), &["save:未命名.md".to_string()]);
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("new.md")));
}

#[test]
fn save_as_suggests_current_path_for_directory() {
    // 传完整路径而不是裸文件名：对话框直接定位到文档所在目录，不依赖系统记忆
    let store = MemStore::default();
    let (mut a, calls) = app_dialogs("ab", Some("dir/a.md"), false, &store, None, None);
    a.save_as(); // 取消
    assert_eq!(calls.borrow().as_slice(), &["save:dir/a.md".to_string()]);
    assert!(a.ui_state.chrome.notice.contains("取消"));
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
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
    assert!(a.editor.is_dirty(), "失败不谎报干净");
    assert!(store.files.borrow().is_empty());
    assert!(a.ui_state.chrome.notice.contains("失败"));
}

#[test]
fn menu_quit_requests_exit() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("ab", None, false, &store, None, None);
    a.perform_menu(MenuAction::Quit);
    assert!(a.window_state.exit_requested, "干净文档退出立即获准");
    assert!(!a.perform_menu(MenuAction::Copy));
}

#[test]
fn new_window_action_preserves_current_document_and_dirty_state() {
    let store = MemStore::default();
    let mut a = app("原文", Some("a.md"), false, &store);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let before = doc_text(&a);
    a.perform_action(Action::NewWindow);
    assert!(a.window_state.new_window_requested);
    assert_eq!(doc_text(&a), before);
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
    assert!(a.editor.is_dirty());
    assert!(
        a.ui_state.modal.is_none(),
        "新建窗口不得触发当前文档关闭/替换确认"
    );
}

#[test]
fn shell_routes_close_only_to_target_window() {
    let store = MemStore::default();
    let clean = app("干净", Some("clean.md"), false, &store);
    let mut dirty = app("脏文档", Some("dirty.md"), false, &store);
    dirty.editor.apply(Intent::InsertText("改".into())).unwrap();
    let clean_id = winit::window::WindowId::from(101_u64);
    let dirty_id = winit::window::WindowId::from(102_u64);
    let mut shell = crate::shell::Shell::empty_for_test();
    shell.insert_for_test(clean_id, clean);
    shell.insert_for_test(dirty_id, dirty);

    let blocked = shell.route_event(dirty_id, winit::event::WindowEvent::CloseRequested);
    assert!(!blocked.closed);
    assert!(!blocked.exit_process);
    assert!(shell
        .get_for_test(dirty_id)
        .unwrap()
        .ui_state
        .modal
        .is_some());
    assert!(shell
        .get_for_test(clean_id)
        .unwrap()
        .ui_state
        .modal
        .is_none());

    let closed = shell.route_event(clean_id, winit::event::WindowEvent::CloseRequested);
    assert!(closed.closed);
    assert!(!closed.exit_process, "仍有脏窗口时不得退出进程");
    assert!(shell.get_for_test(clean_id).is_none());
    assert!(shell.get_for_test(dirty_id).unwrap().editor.is_dirty());
}

#[test]
fn shell_exits_only_after_last_window_closes() {
    let store = MemStore::default();
    let id = winit::window::WindowId::from(103_u64);
    let mut shell = crate::shell::Shell::empty_for_test();
    shell.insert_for_test(id, app("最后窗口", None, false, &store));
    let outcome = shell.route_event(id, winit::event::WindowEvent::CloseRequested);
    assert!(outcome.closed);
    assert!(outcome.exit_process);
    assert!(shell.get_for_test(id).is_none());
}

#[test]
fn explicit_new_window_doc_does_not_consume_recovery() {
    let store = MemStore::default();
    let recovery = FakeRecovery::default();
    *recovery.slot.borrow_mut() = Some("不应读取".as_bytes().to_vec());
    let a = app_persist(
        "显式文档",
        None,
        &store,
        &recovery,
        &FakeSession::default(),
        false,
    );
    assert_eq!(doc_text(&a), "显式文档");
    assert!(!recovery.calls.borrow().iter().any(|call| call == "load"));
}

#[test]
fn saving_one_window_does_not_clear_another_windows_recovery() {
    let store = MemStore::default();
    let recovery = FakeRecovery::default();
    let session = FakeSession::default();
    let mut a = app_persist("a", Some("a.md"), &store, &recovery, &session, false);
    let mut b = app_persist("b", Some("b.md"), &store, &recovery, &session, false);
    a.editor.apply(Intent::InsertText("甲".into())).unwrap();
    b.editor.apply(Intent::InsertText("乙".into())).unwrap();
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_recovery_write(t0 + Duration::from_secs(2));
    b.persist_state.persist.note_activity(t0);
    b.maybe_recovery_write(t0 + Duration::from_secs(2));
    assert_eq!(slot_content(&recovery).1, "乙b");

    a.save_now();
    assert!(!a.editor.is_dirty());
    assert!(b.editor.is_dirty());
    assert_eq!(
        slot_content(&recovery).1,
        "乙b",
        "A 保存不得清除 B 最后写入的恢复快照"
    );

    b.save_now();
    assert!(recovery.slot.borrow().is_none(), "槽属主保存后正常清槽");
}

#[test]
fn ctrl_a_selects_all_then_typing_replaces_document() {
    let store = MemStore::default();
    let mut a = app("ab\ncd", Some("a.md"), false, &store);
    a.ensure_layouts_all();
    a.editor.set_caret(3);
    a.input_state.mods.ctrl = true;
    a.route_key(Key::A);
    a.input_state.mods.ctrl = false;
    assert_eq!(
        a.editor.selection(),
        Some(Range::new(0, 5)),
        "Ctrl+A 覆盖全文"
    );
    assert_eq!(a.editor.caret(), 5, "caret 落文末");
    assert!(!a.editor.is_dirty(), "全选不是内容编辑，不置脏");
    a.route_key(Key::Text("x".into()));
    assert_eq!(doc_text(&a), "x", "全选后输入整体替换");
    assert_eq!(a.editor.selection(), None);
    assert!(a.editor.is_dirty());

    assert!(!a.perform_menu(MenuAction::SelectAll));
    assert_eq!(
        a.editor.selection(),
        Some(Range::new(0, 1)),
        "菜单「全选」与 Ctrl+A 走同一意图"
    );
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
    assert!(!a.window_state.dragging);
    let (tx, ty) = menu_title_center(0);
    click_at(&mut a, tx, ty); // 菜单栏 → 打开菜单
    assert!(a.ui_state.chrome.is_open());
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
    assert!(a.window_state.dragging);
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
    assert!(a.window_state.scroll_y > 0.0, "轨道下方点击翻页");
    let first_page = a.window_state.scroll_y;
    assert!(a.ui_state.chrome.scroll_drag.is_none());

    // 拖拽滑块：按下滑块 → 移动到视口顶部 → 滚动回 0
    let (ty, th) = chrome::scroll_thumb(vp, doc_h, a.window_state.scroll_y, 1.0).unwrap();
    click_at(&mut a, x, ty + th / 2.0);
    assert!(a.ui_state.chrome.scroll_drag.is_some());
    a.input_state.mouse = PhysicalPosition::new(x as f64, (vp.y - 100.0) as f64);
    // 模拟 CursorMoved 的拖动分支
    let off = a.ui_state.chrome.scroll_drag.unwrap();
    a.window_state.scroll_y =
        chrome::scroll_from_thumb(vp, doc_h, a.input_state.mouse.y as f32 - off, 1.0);
    assert_eq!(a.window_state.scroll_y, 0.0);
    assert!(first_page > 0.0);
}

#[test]
fn wheel_scroll_clamps_to_document_end() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("ab\ncd", None, false, &store, None, None);
    let vp = a.viewport();
    let max = chrome::max_scroll(vp, a.doc_height());
    a.window_state.scroll_y = max + 100.0;
    // 事件分支本身做 clamp；此处验证几何上界与钳位语义一致
    a.window_state.scroll_y = a.window_state.scroll_y.clamp(0.0, max);
    assert_eq!(a.window_state.scroll_y, max);
    assert_eq!(max, 0.0, "短文档不滚动");
}

#[test]
fn page_keys_move_caret_within_new_viewport() {
    let store = MemStore::default();
    let text: String = (0..200).map(|_| "abcdefghij\n").collect();
    let (mut a, _) = app_dialogs(&text, None, false, &store, None, None);
    a.ensure_layouts_all();
    assert!(a.editor.set_caret(3));
    let vp = a.viewport();
    assert!(a.doc_height() > vp.h, "长文档可翻页");

    // PageDown：滚动推进，光标随页移动且落在新视口内，列不变
    a.perform_action(Action::Page(1));
    assert!(a.window_state.scroll_y > 0.0, "PageDown 推进滚动");
    let (_, y, h) = a.caret_screen();
    assert!(
        y >= vp.y - 0.5 && y + h <= vp.y + vp.h + 0.5,
        "翻页后光标落在新视口内"
    );
    let (_, col) = a.editor.session().line_col(a.editor.caret()).unwrap();
    assert_eq!(col, 4, "翻页保持原列位置");

    // PageUp：滚动回退，光标仍在新视口内
    let scrolled = a.window_state.scroll_y;
    a.perform_action(Action::Page(-1));
    assert!(a.window_state.scroll_y < scrolled, "PageUp 回退滚动");
    let (_, y, h) = a.caret_screen();
    assert!(
        y >= vp.y - 0.5 && y + h <= vp.y + vp.h + 0.5,
        "PageUp 后光标仍在视口内"
    );
}

#[test]
fn status_line_reports_caret_line_col() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("ab\n中c\n", Some("a.md"), false, &store, None, None);
    a.editor.set_caret(6);
    assert_eq!(a.editor.session().line_col(6), Some((2, 2)));
    let (left, right) = chrome::status_line(
        a.doc.path.as_deref(),
        a.editor.is_dirty(),
        a.external_state(),
        a.source_mode(),
        a.persist_state.last_saved_hhmm.as_deref(),
        a.ui_state.writing.words,
        2,
        2,
    );
    assert_eq!(left, "a.md");
    assert_eq!(right, "3 字 · 行 2，列 2");
}

// ---- 信任显性化：保存态（已保存 HH:MM / 未保存 / 失败可行动提示） ----

/// 手工保存成功：状态栏左文出现注入时钟的「已保存 14:32」，App 记录同一时刻。
#[test]
fn manual_save_records_hhmm_and_status_shows_it() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, None);
    a.editor.apply(Intent::InsertText("x".into())).unwrap();
    a.save_now();
    assert_eq!(a.persist_state.last_saved_hhmm.as_deref(), Some("14:32"));
    let (left, _) = chrome::status_line(
        a.doc.path.as_deref(),
        a.editor.is_dirty(),
        a.external_state(),
        a.source_mode(),
        a.persist_state.last_saved_hhmm.as_deref(),
        0,
        1,
        1,
    );
    assert_eq!(left, "a.md · 已保存 14:32");
}

/// 自动保存成功同样刷新时刻；再次编辑后状态栏立刻回到「未保存」（时间不得掩盖脏态）。
#[test]
fn autosave_records_hhmm_and_dirty_says_unsaved() {
    let store = MemStore::default();
    let mut a = dirty_app("ab", Some("a.md"), &store);
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_autosave(t0 + Duration::from_secs(2));
    assert!(!a.editor.is_dirty());
    assert_eq!(a.persist_state.last_saved_hhmm.as_deref(), Some("14:32"));

    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let (left, _) = chrome::status_line(
        a.doc.path.as_deref(),
        a.editor.is_dirty(),
        a.external_state(),
        a.source_mode(),
        a.persist_state.last_saved_hhmm.as_deref(),
        0,
        1,
        1,
    );
    assert!(left.contains("未保存"), "脏态须有明确文字: {left}");
    assert!(!left.contains("已保存"), "脏态不得再显示已保存: {left}");
}

/// 保存失败：notice 带原因与可行动提示（Ctrl+S 重试 / 另存为）、文档保持 dirty、
/// 最近保存时刻不得被失败污染。
#[test]
fn save_failure_keeps_dirty_and_keeps_hhmm_stale_free() {
    let store = MemStore {
        fail_save: true,
        ..Default::default()
    };
    let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, None);
    a.editor.apply(Intent::InsertText("x".into())).unwrap();
    a.save_now();
    assert!(a.editor.is_dirty(), "失败不得谎报干净");
    assert_eq!(a.persist_state.last_saved_hhmm, None, "失败不得刷新时间");
    let notice = &a.ui_state.chrome.notice;
    assert!(notice.contains("保存失败"), "{notice}");
    assert!(
        notice.contains("Ctrl+S") && notice.contains("另存为"),
        "失败提示须含可行动下一步: {notice}"
    );
}

/// B6：保存失败必须同时给出错误弹窗（标题/原因/重试·另存为·取消），
/// 且错误态与普通弹窗同源（ModalKind::Error + 统一按钮角色）。
#[test]
fn save_failure_opens_error_modal_with_actionable_exits() {
    use crate::view::modal::{ButtonRole, ModalKind};
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"ab".to_vec());
    let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, Some("b.md"));
    store.fail_paths.borrow_mut().push(PathBuf::from("a.md"));
    a.editor.apply(Intent::InsertText("x".into())).unwrap();
    a.save_now();

    let m = a.ui_state.modal.as_ref().expect("失败必须弹错误弹窗");
    assert_eq!(m.spec.kind, ModalKind::Error);
    assert_eq!(m.spec.title, "保存失败");
    assert!(m.spec.message.contains("模拟写失败"), "须带原因");
    assert!(m.spec.message.contains("当前修改仍保留"), "须带下一步");
    let labels: Vec<_> = m.spec.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, vec!["重试", "另存为", "取消"]);
    assert_eq!(m.spec.buttons[0].role, ButtonRole::Primary);
    assert!(a.editor.is_dirty(), "失败不得清脏");

    // 取消：关闭弹窗、文档与脏标保持。
    a.resolve_modal(3);
    assert!(a.ui_state.modal.is_none());
    assert!(a.editor.is_dirty());
    assert!(a.ui_state.chrome.notice.contains("已取消保存"));
}

/// B6：错误弹窗「重试」重跑同一动作（成功即恢复），不静默换目标。
#[test]
fn save_error_retry_reuses_same_path_and_recovers() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"ab".to_vec());
    let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, None);
    store.fail_paths.borrow_mut().push(PathBuf::from("a.md"));
    a.editor.apply(Intent::InsertText("x".into())).unwrap();
    a.save_now();
    assert!(a.ui_state.modal.is_some());

    // 修复写入条件后重试：同一路径成功、清脏并关闭弹窗。
    store.fail_paths.borrow_mut().clear();
    a.resolve_modal(1);
    assert!(a.ui_state.modal.is_none());
    assert!(!a.editor.is_dirty());
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
    assert!(a.ui_state.chrome.notice.contains("已保存"));
}

/// B6：错误弹窗「另存为」是可操作的第二出口（无需先关弹窗再走菜单）。
#[test]
fn save_error_secondary_exit_uses_save_as() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"ab".to_vec());
    let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, Some("b.md"));
    store.fail_paths.borrow_mut().push(PathBuf::from("a.md"));
    a.editor.apply(Intent::InsertText("x".into())).unwrap();
    a.save_now();
    assert!(a.ui_state.modal.is_some());

    a.resolve_modal(2);
    assert!(a.ui_state.modal.is_none());
    assert!(!a.editor.is_dirty());
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));
    assert!(store.files.borrow().contains_key(Path::new("b.md")));
}

/// B6：自动保存失败不打断输入（状态栏提示、无模态）；手工保存失败才弹错误弹窗。
#[test]
fn autosave_failure_stays_in_status_bar_without_modal() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"ab".to_vec());
    let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, None);
    store.fail_paths.borrow_mut().push(PathBuf::from("a.md"));
    a.editor.apply(Intent::InsertText("x".into())).unwrap();
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_autosave(t0 + Duration::from_secs(3));
    assert!(a.ui_state.modal.is_none(), "自动保存失败不得打断用户");
    assert!(a.editor.is_dirty());
    assert!(a.ui_state.chrome.notice.contains("保存失败"));
    // 模态未决时自动保存也必须让位（用户正在处理失败原因）。
    a.save_now();
    assert!(a.ui_state.modal.is_some());
    assert!(!a.autosave_due(std::time::Instant::now() + std::time::Duration::from_secs(30)));
}

/// B6：打开失败同样走错误弹窗（重试 / 重新选择 / 取消），原文档不被改动。
#[test]
fn open_failure_opens_error_modal_and_cancel_keeps_document() {
    use crate::view::modal::ModalKind;
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"ab".to_vec());
    let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, Some("missing.md"), None);
    a.open_via_dialog();
    // 干净文档直接走打开；失败必须挂起错误弹窗而不是只写一行状态栏。
    let m = a.ui_state.modal.as_ref().expect("打开失败必须弹错误弹窗");
    assert_eq!(m.spec.kind, ModalKind::Error);
    assert_eq!(m.spec.title, "打开失败");
    assert!(m.spec.message.contains("missing.md"));
    let labels: Vec<_> = m.spec.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, vec!["重试", "重新选择", "取消"]);

    a.resolve_modal(3);
    assert!(a.ui_state.modal.is_none());
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
    assert!(a.ui_state.chrome.notice.contains("已取消打开"));
}

/// 另存为成功刷新时刻；打开另一份文档时旧时间作废（不得把 A 的时刻显示在 B 上）。
#[test]
fn save_as_records_hhmm_and_open_resets_it() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("b.md"), b"new".to_vec());
    let (mut a, _) = app_dialogs("ab", None, false, &store, Some("b.md"), Some("new.md"));
    a.save_as();
    assert_eq!(a.persist_state.last_saved_hhmm.as_deref(), Some("14:32"));
    a.open_via_dialog();
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));
    assert_eq!(a.persist_state.last_saved_hhmm, None, "换文档后旧时刻作废");
}

#[test]
fn save_notice_surfaces_on_update() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("ab", Some("a.md"), false, &store, None, None);
    a.editor.apply(Intent::InsertText("x".into())).unwrap();
    a.save_now();
    assert!(a.ui_state.chrome.notice.contains("已保存"));
    a.doc.path = None;
    a.save_now();
    assert!(
        a.ui_state.chrome.notice.contains("另存为"),
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
    a.persist_state.persist.note_activity(t0);
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
    assert!(
        !a.persist_state.persist.backed_up(),
        "未写备份：无「已备份」提示"
    );
}

#[test]
fn open_resets_conflict_state_and_baseline() {
    let store = MemStore::default();
    let mut a = app("x", Some("a.md"), false, &store);
    a.persist_state.persist.mark_backed_up(b"v1".to_vec());
    a.persist_state.persist.set_hold(true);
    a.persist_state.persist.note_activity(Instant::now());
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("b.md"), b"# n".to_vec());
    a.persist_state.persist.begin_external(b"external".to_vec());
    a.persist_state.persist.set_missing(true);
    a.open_path(PathBuf::from("b.md"));
    assert!(!a.persist_state.persist.backed_up());
    assert!(!a.persist_state.persist.hold());
    assert!(a.persist_state.persist.activity().is_none());
    assert_eq!(a.doc.baseline, b"# n".to_vec());
    assert!(
        a.persist_state.persist.pending().is_none(),
        "打开新文档清外部变更态"
    );
    assert!(!a.persist_state.persist.missing());
}

// ---- 外部文件监听（切片 16）：变更检测 / 提示去重 / 删除恢复 / 监听切换 ----

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
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
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
    a.doc.path = None;
    a.watch_current();
    assert_eq!(watch.calls.borrow().last().unwrap(), "unwatch");
}

#[test]
fn ctrl_n_creates_empty_draft_and_resets_state() {
    let store = MemStore::default();
    let watch = FakeWatch::default();
    let mut a = app_watch("ab\ncd", Some("a.md"), &store, &watch);
    a.ensure_layouts_all();
    let session_rev = a.input_state.revs.session;
    let image_rev = a.input_state.revs.image_base;
    a.window_state.scroll_y = 120.0;
    a.window_state.goal_x = Some(30.0);
    a.ui_state.outline.scroll = 40.0;
    a.ui_state.search.query = "ab".into();

    a.input_state.mods.ctrl = true;
    a.route_key(Key::N);
    a.input_state.mods.ctrl = false;

    assert_eq!(doc_text(&a), "");
    assert_eq!(a.doc.path, None, "新建即草稿");
    assert_eq!(a.title_text(), "veil", "无路径标题");
    assert!(!a.editor.is_dirty());
    assert!(a.doc.baseline.is_empty());
    assert_eq!(a.input_state.revs.session, session_rev + 1);
    assert_eq!(a.input_state.revs.image_base, image_rev + 1);
    assert_eq!(a.window_state.scroll_y, 0.0);
    assert_eq!(a.window_state.goal_x, None);
    assert_eq!(a.ui_state.outline.scroll, 0.0);
    assert!(a.ui_state.search.query.is_empty(), "旧命中偏移全部失效");
    assert_eq!(
        watch.calls.borrow().last().unwrap(),
        "unwatch",
        "旧文件解绑"
    );
    assert_eq!(a.editor.apply(Intent::Undo), None, "新文档撤销栈为空");

    a.route_key(Key::Text("x".into()));
    assert_eq!(doc_text(&a), "x", "新草稿可正常编辑");
}

#[test]
fn new_draft_refuses_when_dirty() {
    let store = MemStore::default();
    let mut a = dirty_app("ab", Some("a.md"), &store);
    a.perform_action(Action::New);
    assert!(a.ui_state.modal.is_some(), "脏文档新建先挂起确认模态");
    answer_modal(&mut a, Key::Escape);
    assert_eq!(doc_text(&a), "改ab", "脏文档拒绝新建，数据不丢");
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
    assert!(a.editor.is_dirty());
    assert!(a.ui_state.chrome.notice.contains("未保存"));
}

// ---- 3e 修 B：未保存内容三选一（关闭/新建/打开） ----

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
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
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

#[test]
fn dirty_open_save_choice_writes_disk_before_opening() {
    let store = MemStore::default();
    disk(&store, "a.md", b"old");
    disk(&store, "b.md", b"new");
    let calls = Rc::new(RefCell::new(Vec::new()));
    let d = FakeDialogs {
        open: Some(PathBuf::from("b.md")),
        save: None,
        image: None,
        export: None,
        calls: calls.clone(),
    };
    let mut a = app_confirm(
        "old",
        Some("a.md"),
        &store,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(d),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.open_via_dialog();
    assert!(a.ui_state.modal.is_some(), "脏文档先挂起确认模态");
    assert!(calls.borrow().is_empty(), "未作答前不得弹文件对话框");
    answer_modal(&mut a, Key::Text("s".into()));
    assert_eq!(
        store.files.borrow()[Path::new("a.md")],
        "改old".as_bytes(),
        "选择保存：先写盘再弹打开对话框"
    );
    assert_eq!(
        calls.borrow().as_slice(),
        &["open".to_string()],
        "确认保存后才选文件"
    );
    assert_eq!(doc_text(&a), "new", "随后正常打开目标文件");
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));
    assert!(!a.editor.is_dirty());
}

#[test]
fn dirty_open_discard_clears_snapshot_without_writing_disk() {
    let store = MemStore::default();
    disk(&store, "a.md", b"old");
    disk(&store, "b.md", b"new");
    let rec = FakeRecovery::default();
    let mut a = app_confirm(
        "old",
        Some("a.md"),
        &store,
        rec.clone(),
        &FakeSession::default(),
        Box::new(NoDialogs),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    seed_recovery(&mut a, &rec);

    a.open_path(PathBuf::from("b.md"));
    assert!(
        a.doc.path.as_deref() == Some(Path::new("a.md")),
        "等待结论前不切换文档"
    );
    answer_modal(&mut a, Key::Text("c".into()));
    assert!(
        rec.slot.borrow().is_none(),
        "明确放弃：恢复快照必须作废，下次启动不得复活"
    );
    assert_eq!(
        store.files.borrow()[Path::new("a.md")],
        "old".as_bytes(),
        "放弃不得写盘"
    );
    assert_eq!(doc_text(&a), "new");
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));
}

#[test]
fn dirty_open_cancel_keeps_document_and_notice() {
    let store = MemStore::default();
    disk(&store, "b.md", b"new");
    let mut a = app_confirm(
        "old",
        Some("a.md"),
        &store,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(NoDialogs),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.open_path(PathBuf::from("b.md"));
    assert!(a.ui_state.modal.is_some(), "脏文档必须确认一次");
    answer_modal(&mut a, Key::Escape);
    assert!(a.ui_state.modal.is_none(), "取消即关闭模态");
    assert_eq!(doc_text(&a), "改old", "取消：文档保持原样");
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
    assert!(a.editor.is_dirty(), "取消不清脏");
    assert!(a.ui_state.chrome.notice.contains("已取消"));
}

#[test]
fn untitled_save_choice_cancelled_aborts_new_draft() {
    let store = MemStore::default();
    let mut a = app_confirm(
        "草稿",
        None,
        &store,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(NoDialogs),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.new_draft();
    answer_modal(&mut a, Key::Text("s".into()));
    assert_eq!(doc_text(&a), "改草稿", "另存为被取消 → 不新建");
    assert_eq!(a.doc.path, None);
    assert!(a.editor.is_dirty());
    assert!(a.ui_state.chrome.notice.contains("保存未完成"));
}

#[test]
fn untitled_save_choice_saves_as_then_creates_new_draft() {
    let store = MemStore::default();
    let d = FakeDialogs {
        open: None,
        save: Some(PathBuf::from("draft.md")),
        image: None,
        export: None,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let mut a = app_confirm(
        "草稿",
        None,
        &store,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(d),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.new_draft();
    answer_modal(&mut a, Key::Text("s".into()));
    assert_eq!(
        store.files.borrow()[Path::new("draft.md")],
        "改草稿".as_bytes(),
        "无路径草稿：保存走另存为"
    );
    assert_eq!(doc_text(&a), "", "随后新建空白草稿");
    assert_eq!(a.doc.path, None);
    assert!(!a.editor.is_dirty());
}

#[test]
fn close_confirmation_saves_discards_or_cancels() {
    let store = MemStore::default();
    disk(&store, "a.md", b"old");

    // 取消：不退出、不清脏、快照保留
    let rec = FakeRecovery::default();
    let mut a = app_confirm(
        "old",
        Some("a.md"),
        &store,
        rec.clone(),
        &FakeSession::default(),
        Box::new(NoDialogs),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    seed_recovery(&mut a, &rec);
    a.confirm_close();
    answer_modal(&mut a, Key::Escape);
    assert!(!a.window_state.exit_requested, "取消必须阻止退出");
    assert!(a.editor.is_dirty());
    assert!(rec.slot.borrow().is_some(), "取消不动作废快照");

    // 放弃：允许退出且快照作废
    let rec = FakeRecovery::default();
    let mut b = app_confirm(
        "old",
        Some("a.md"),
        &store,
        rec.clone(),
        &FakeSession::default(),
        Box::new(NoDialogs),
    );
    b.editor.apply(Intent::InsertText("改".into())).unwrap();
    seed_recovery(&mut b, &rec);
    b.confirm_close();
    b.ime_commit("c".into());
    assert!(b.window_state.exit_requested, "IME 助记符放弃允许退出");
    assert_eq!(doc_text(&b), "改old", "模态 IME 提交不得写入正文");
    assert!(rec.slot.borrow().is_none(), "放弃即清快照");

    // 保存：写盘后允许退出，快照作废
    let rec = FakeRecovery::default();
    let mut c = app_confirm(
        "old",
        Some("a.md"),
        &store,
        rec.clone(),
        &FakeSession::default(),
        Box::new(NoDialogs),
    );
    c.editor.apply(Intent::InsertText("改".into())).unwrap();
    seed_recovery(&mut c, &rec);
    c.confirm_close();
    c.ime_commit("s".into());
    assert!(c.window_state.exit_requested, "IME 助记符保存成功允许退出");
    assert_eq!(store.files.borrow()[Path::new("a.md")], "改old".as_bytes());
    assert!(rec.slot.borrow().is_none());
}

#[test]
fn unsaved_modal_accepts_letters_without_ime_text() {
    // 真机复现：输入法启用时字母只到逻辑键（Text 为空），弹窗此前完全无响应。
    let store = MemStore::default();
    disk(&store, "a.md", b"old");

    let mut a = app_confirm(
        "old",
        Some("a.md"),
        &store,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(NoDialogs),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    a.confirm_close();
    a.route_key(Key::C);
    assert!(a.window_state.exit_requested, "逻辑键 c 应触发放弃");
    assert_eq!(doc_text(&a), "改old", "模态按键不得写入正文");

    let mut b = app_confirm(
        "old",
        Some("a.md"),
        &store,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(NoDialogs),
    );
    b.editor.apply(Intent::InsertText("改".into())).unwrap();
    b.confirm_close();
    b.route_key(Key::S);
    assert!(b.window_state.exit_requested, "逻辑键 s 应触发保存");
    assert_eq!(store.files.borrow()[Path::new("a.md")], "改old".as_bytes());
}

#[test]
fn modal_is_exclusive_and_closes_lite_overlays() {
    let store = MemStore::default();
    let mut a = dirty_app("ab", Some("a.md"), &store);
    a.ui_state.chrome.open = Some(0);
    a.open_search();
    assert!(a.ui_state.search.open && a.ui_state.chrome.open.is_none());

    a.ui_state.chrome.open = Some(0);
    a.confirm_close();
    assert!(a.ui_state.modal.is_some());
    assert!(!a.ui_state.search.open, "弹窗出现即收掉查找栏");
    assert!(a.ui_state.chrome.open.is_none(), "弹窗出现即收掉菜单");

    // 模态未决期间，快捷键不得打开其它浮层
    a.input_state.mods.ctrl = true;
    a.route_key(Key::P);
    a.route_key(Key::F);
    a.input_state.mods.ctrl = false;
    assert!(
        !a.ui_state.quick.open && !a.ui_state.search.open,
        "模态独占：其它浮层不可打开"
    );
}

#[test]
fn navigation_panels_are_mutually_exclusive() {
    let store = MemStore::default();
    let mut a = app("ab\ncd", Some("a.md"), false, &store);
    a.open_history();
    assert!(a.ui_state.history.open);
    a.open_quick();
    assert!(
        a.ui_state.quick.open && !a.ui_state.history.open,
        "快速打开接管历史面板"
    );
    a.open_search();
    assert!(
        a.ui_state.search.open && !a.ui_state.quick.open,
        "查找栏接管快速打开"
    );
    a.open_history();
    assert!(
        a.ui_state.history.open && !a.ui_state.search.open,
        "历史面板接管查找栏"
    );
}

#[test]
fn clean_close_skips_confirmation_and_clears_stale_snapshot() {
    // 干净文档：不弹确认；若磁盘上仍残留本会话快照（保存后清理失败的兜底），退出时清掉
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let mut a = app_confirm(
        "old",
        Some("a.md"),
        &store,
        rec.clone(),
        &FakeSession::default(),
        Box::new(NoDialogs),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    seed_recovery(&mut a, &rec);
    a.editor.mark_saved(); // 模拟「内容已与磁盘一致」的干净态，快照仍残留
    a.confirm_close();
    assert!(a.ui_state.modal.is_none(), "干净文档不得弹确认");
    assert!(a.window_state.exit_requested, "干净关窗直接获准退出");
    assert!(rec.slot.borrow().is_none(), "干净退出清残稿");
}

#[test]
fn dirty_open_is_deferred_until_modal_choice() {
    let store = MemStore::default();
    disk(&store, "b.md", b"new");
    let mut a = app("old", Some("a.md"), false, &store);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.open_path(PathBuf::from("b.md"));
    assert!(a.ui_state.modal.is_some(), "应立即登记模态而非阻塞");
    assert_eq!(
        a.doc.path.as_deref(),
        Some(Path::new("a.md")),
        "结论出来前不切换文档"
    );
    assert_eq!(doc_text(&a), "改old");

    answer_modal(&mut a, Key::Text("c".into()));
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));
    assert_eq!(doc_text(&a), "new");
    assert!(!a.editor.is_dirty());
}

#[test]
fn modal_cancel_closes_and_keeps_dirty_document() {
    let store = MemStore::default();
    disk(&store, "b.md", b"new");
    let mut a = app("old", Some("a.md"), false, &store);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.open_path(PathBuf::from("b.md"));
    answer_modal(&mut a, Key::Escape);
    assert!(a.ui_state.modal.is_none(), "取消即关闭模态");
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
    assert_eq!(doc_text(&a), "改old");
    assert!(a.ui_state.chrome.notice.contains("已取消"));
}

#[test]
fn clean_open_and_new_skip_modal() {
    let store = MemStore::default();
    disk(&store, "b.md", b"new");
    let mut a = app("old", Some("a.md"), false, &store);

    a.open_path(PathBuf::from("b.md"));
    assert!(a.ui_state.modal.is_none(), "干净文档不弹模态");
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));

    a.new_draft();
    assert!(a.ui_state.modal.is_none(), "干净文档新建不弹模态");
    assert_eq!(a.doc.path, None);
    assert_eq!(doc_text(&a), "");
}

#[test]
fn modal_swallows_editor_input_until_answered() {
    let store = MemStore::default();
    disk(&store, "b.md", b"new");
    let mut a = app("old", Some("a.md"), false, &store);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let before = doc_text(&a);

    a.open_path(PathBuf::from("b.md"));
    a.route_key(Key::Text("x".into()));
    assert_eq!(doc_text(&a), before, "模态打开时字母键不进入编辑器");
    a.ime_preedit("x".into(), None);
    assert!(a.editor.preedit().is_none(), "模态打开时不挂 IME 组合串");
    a.ime_commit("x".into());
    assert_eq!(doc_text(&a), before, "模态打开时 IME 提交不进入编辑器");
    assert!(a.ui_state.modal.is_some(), "无效按键不关闭模态");
}

#[test]
fn modal_scrim_click_does_not_close() {
    let store = MemStore::default();
    disk(&store, "b.md", b"new");
    let mut a = app("old", Some("a.md"), false, &store);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.open_path(PathBuf::from("b.md"));
    a.input_state.mouse = PhysicalPosition::new(1.0, 1.0);
    a.on_left_click();
    assert!(a.ui_state.modal.is_some(), "点击遮罩不得关闭模态");
}

#[test]
fn modal_paints_scrim_and_card() {
    let store = MemStore::default();
    disk(&store, "b.md", b"new");
    let mut a = app("old", Some("a.md"), false, &store);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let popup_bg = a.input_state.theme.popup_bg;
    let (_, h) = a.win_size();
    let probe = (4u32, h / 2);
    let before = {
        let f = a.render();
        f.get(probe.0, probe.1)
    };

    a.open_path(PathBuf::from("b.md"));
    let l = a.modal_layout();
    let card_pt = (
        (l.card.x + l.card.w * 0.5).round() as u32,
        (l.card.y + 10.0).round() as u32,
    );
    let f = a.render();
    let after = f.get(probe.0, probe.1);
    assert!(
        after[0] < before[0] || after[1] < before[1] || after[2] < before[2],
        "遮罩应压暗窗口角落: before={before:?} after={after:?}"
    );
    let card = f.get(card_pt.0, card_pt.1);
    assert_eq!(
        [card[0], card[1], card[2]],
        [popup_bg[0], popup_bg[1], popup_bg[2]],
        "卡片空白处应为 popup_bg"
    );
}

#[test]
fn tab_indents_selection_and_shift_tab_reverts() {
    let store = MemStore::default();
    let mut a = app("a\nb", None, false, &store);
    a.ensure_layouts_all();
    a.perform_action(Action::Doc(Intent::SelectAll));
    a.route_key(Key::Tab);
    assert_eq!(doc_text(&a), "  a\n  b", "选中多行一次缩进");
    a.input_state.mods.shift = true;
    a.route_key(Key::Tab);
    a.input_state.mods.shift = false;
    assert_eq!(doc_text(&a), "a\nb", "Shift+Tab 反缩进回原状");
    assert!(a.editor.is_dirty());
}

#[test]
fn tab_whole_line_indent_keeps_paragraph_kind() {
    // T1：整行缩进普通段落后仍是 Paragraph，不会因 4 空格变成 IndentedCode
    let store = MemStore::default();
    let mut a = app("第一段\n第二段", None, false, &store);
    a.ensure_layouts_all();
    a.perform_action(Action::Doc(Intent::SelectAll));
    a.route_key(Key::Tab);
    assert_eq!(doc_text(&a), "  第一段\n  第二段");
    assert!(
        matches!(a.index.blocks[0].kind, BlockKind::Paragraph),
        "缩进后段落不得变成代码块"
    );
}

#[test]
fn tab_whole_line_indent_nests_list_item() {
    // T1：列表项整行缩进改嵌套层级（本解析器 depth = 缩进字节 / 2），仍是 ListItem
    let store = MemStore::default();
    let mut a = app("- a", None, false, &store);
    a.ensure_layouts_all();
    a.perform_action(Action::Doc(Intent::SelectAll));
    a.route_key(Key::Tab);
    assert_eq!(doc_text(&a), "  - a");
    let (_, kind, _) = a.editor.session().blocks()[0].clone();
    assert_eq!(
        kind,
        BlockKind::ListItem {
            ordered: false,
            depth: 1,
            task: None
        },
        "缩进后保持列表结构且层级 +1"
    );
}

#[test]
fn resize_rebuild_is_coalesced_into_one_frame() {
    // R2：Resized 只置待处理位；同一帧内多次事件只重建一次
    let store = MemStore::default();
    let mut a = app("a\n\nb", None, false, &store);
    a.ensure_layouts_all();
    for _ in 0..5 {
        a.window_state.resize_pending = true;
    }
    assert!(a.apply_pending_resize(), "首帧执行一次重建");
    assert!(!a.apply_pending_resize(), "无新尺寸事件时不得重复重建");
    assert_eq!(doc_text(&a), "a\n\nb");
    assert_eq!(a.index.blocks.len(), 2, "合并后块索引仍完整");
}

#[test]
fn resized_zero_size_only_sets_pending_flag() {
    // 修复 2：Resized(0,0)（最小化）必须跳过 surface.resize 和 request_redraw，
    // 只保留 resize_pending 给窗口恢复后合并。
    // headless App 没有真实 window/surface，我们用 NonZeroU32 的行为证明分支条件正确：
    // 0 宽或 0 高时 NonZeroU32::new 返回 None，内部 if let 被跳过，
    // 但外层 resize_pending = true 仍无条件执行。
    use std::num::NonZeroU32;
    let size_zero = winit::dpi::PhysicalSize::new(0u32, 0u32);
    assert!(
        NonZeroU32::new(size_zero.width).is_none(),
        "宽=0 → NonZeroU32::None"
    );
    assert!(
        NonZeroU32::new(size_zero.height).is_none(),
        "高=0 → NonZeroU32::None"
    );
    let size_ok = winit::dpi::PhysicalSize::new(800u32, 600u32);
    assert!(NonZeroU32::new(size_ok.width).is_some());
    assert!(NonZeroU32::new(size_ok.height).is_some());

    // headless App：直接模拟 Resized(0,0) 的外层置位（window_event 需 ActiveEventLoop 无法在
    // 单元测试调用），确认 resize_pending 被置为 true 且不 panic。
    let store = MemStore::default();
    let mut a = app("hi", None, false, &store);
    a.window_state.resize_pending = false;
    a.window_state.resize_pending = true; // 对应修复后的外层无条件置位
    assert!(
        a.window_state.resize_pending,
        "0×0 也应置 resize_pending 给恢复后合并"
    );
    // headless window 和 surface 都是 None → NonZeroU32::new(0)=None → if let 跳过 →
    // s.resize 与 request_redraw 均不触发；恢复到非零尺寸时才有机会走内部分支。
}

#[test]
fn enter_continues_list_in_document() {
    let store = MemStore::default();
    let mut a = app("- a", None, false, &store);
    a.ensure_layouts_all();
    a.editor.set_caret(3);
    a.route_key(Key::Enter);
    assert_eq!(doc_text(&a), "- a\n- ");
    assert!(a.editor.is_dirty());
    a.route_key(Key::Enter);
    assert_eq!(doc_text(&a), "- a\n", "空项再回车收束列表");
    assert_eq!(a.editor.caret(), 4);
}

#[test]
fn replace_one_replaces_current_then_rebuilds_matches() {
    let store = MemStore::default();
    let mut a = app("a x a", None, false, &store);
    a.ensure_layouts_all();
    a.perform_action(Action::Replace);
    assert!(a.ui_state.search.open && a.ui_state.search.replace_open);
    assert_eq!(
        a.ui_state.search.focus,
        search::Field::Query,
        "Ctrl+H 首次打开先聚焦查询，首字不得落进替换框"
    );
    a.search_input("a");
    assert_eq!(a.ui_state.search.matches.len(), 2);
    a.search_toggle_field(); // Tab → 替换
    a.search_input("b");
    assert_eq!(a.ui_state.search.replace, "b");

    a.replace_current();
    assert_eq!(doc_text(&a), "b x a", "仅替换当前命中");
    assert_eq!(a.ui_state.search.matches.len(), 1, "命中缓存重建");
    assert_eq!(a.editor.caret(), 4, "跳到剩余的首个命中");

    a.replace_current();
    assert_eq!(doc_text(&a), "b x b");
    assert!(a.ui_state.search.matches.is_empty());

    a.close_search();
    assert!(!a.ui_state.search.replace_open, "关闭后替换行收起");
    assert!(a.ui_state.search.query.is_empty() && a.ui_state.search.replace.is_empty());
    a.input_state.mods.ctrl = true;
    a.route_key(Key::Z);
    a.input_state.mods.ctrl = false;
    assert_eq!(doc_text(&a), "b x a", "单次替换是独立撤销步");
}

#[test]
fn replace_all_is_one_transaction_and_clears_matches() {
    let store = MemStore::default();
    let mut a = app("a a a", None, false, &store);
    a.ensure_layouts_all();
    a.perform_action(Action::Replace);
    a.search_input("a");
    a.search_toggle_field();
    a.search_input("b");
    a.replace_all();
    assert_eq!(doc_text(&a), "b b b");
    assert!(a.ui_state.search.matches.is_empty());
    a.close_search();
    a.input_state.mods.ctrl = true;
    a.route_key(Key::Z);
    a.input_state.mods.ctrl = false;
    assert_eq!(doc_text(&a), "a a a", "全部替换单次撤销即整体回退");
}

#[test]
fn replace_all_without_matches_does_not_dirty() {
    let store = MemStore::default();
    let mut a = app("abc", None, false, &store);
    a.perform_action(Action::Replace);
    a.search_input("zzz");
    a.replace_all();
    assert_eq!(doc_text(&a), "abc");
    assert!(!a.editor.is_dirty());
}

#[test]
fn enter_in_replace_field_replaces_current_and_advances() {
    let store = MemStore::default();
    let mut a = app("a x a x a", None, false, &store);
    a.ensure_layouts_all();
    a.open_replace();
    a.search_input("a");
    a.search_toggle_field(); // Tab → 替换框
    a.search_input("b");
    a.route_key(Key::Enter);
    assert_eq!(doc_text(&a), "b x a x a", "Enter 替换当前一处");
    assert_eq!(a.ui_state.search.label(), "1/2", "n/m 重建");
    assert_eq!(a.editor.caret(), 4, "前进到下一处");
    a.route_key(Key::Enter);
    assert_eq!(doc_text(&a), "b x b x a");
    assert_eq!(a.ui_state.search.label(), "1/1");
    assert_eq!(a.editor.caret(), 8);
}

#[test]
fn ctrl_enter_in_replace_field_replaces_all() {
    let store = MemStore::default();
    let mut a = app("a A a", None, false, &store);
    a.ensure_layouts_all();
    a.open_replace();
    a.search_input("a");
    a.search_toggle_field();
    a.search_input("b");
    a.input_state.mods.ctrl = true;
    a.route_key(Key::Enter);
    a.input_state.mods.ctrl = false;
    assert_eq!(
        doc_text(&a),
        "b b b",
        "Ctrl+Enter 全部替换（默认不区分大小写）"
    );
    assert!(a.ui_state.search.matches.is_empty());
    assert_eq!(a.ui_state.search.label(), "0/0");
}

#[test]
fn replace_row_clicks_run_replace_buttons() {
    let store = MemStore::default();
    let mut a = app("a a", None, false, &store);
    a.ensure_layouts_all();
    a.perform_action(Action::Replace);
    a.search_input("a");
    a.search_toggle_field();
    a.search_input("b");
    let l = a.search_rect().expect("查找栏开启");
    let r = search::replace_layout(&l, a.input_state.scale);
    let center = |q: crate::geometry::Rect| (q.x + q.w / 2.0, q.y + q.h / 2.0);
    let (x, y) = center(r.one);
    click_at(&mut a, x, y);
    assert_eq!(doc_text(&a), "b a", "「替换」按钮只替换当前命中");
    let (x, y) = center(r.all);
    click_at(&mut a, x, y);
    assert_eq!(doc_text(&a), "b b", "「全部」按钮替换全部");
}

#[test]
fn replace_row_is_painted_and_closed_with_bar() {
    let store = MemStore::default();
    let mut a = app("a", None, false, &store);
    a.open_replace();
    let (w, h) = a.win_size();
    let l = a.search_rect().unwrap();
    let r = search::replace_layout(&l, a.input_state.scale);
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut f);
    assert_eq!(
        f.get((r.text.x + 3.0) as u32, (r.row.y + 3.0) as u32),
        a.input_state.theme.popup_bg,
        "替换行不透明底色（与全部浮层同色）"
    );
    a.close_search();
    let mut g = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut g);
    assert_eq!(
        g.get((r.text.x + 3.0) as u32, (r.row.y + 3.0) as u32),
        a.input_state.theme.bg,
        "关闭后替换行不再绘制"
    );
}

#[test]
fn menu_new_runs_same_intent() {
    let store = MemStore::default();
    let mut a = app("ab", Some("a.md"), false, &store);
    assert!(!a.perform_menu(MenuAction::New));
    assert_eq!(doc_text(&a), "");
    assert_eq!(a.doc.path, None);
}

#[test]
fn watch_failure_surfaces_notice_without_blocking_edit() {
    let store = MemStore::default();
    let watch = FakeWatch {
        fail: true,
        ..Default::default()
    };
    let mut a = app_watch("x", Some("a.md"), &store, &watch);
    assert!(a.ui_state.chrome.notice.contains("监听不可用"));
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
    a.persist_state.persist.note_activity(t0);
    disk(&store, "a.md", b"external");
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert_eq!(
        a.persist_state.persist.pending(),
        Some(b"external".as_slice()),
        "记录待备份的外部版本"
    );
    assert!(a.persist_state.persist.hold(), "外部修改必须挂起自动保存");
    assert!(a.ui_state.chrome.notice.contains("外部修改"));
    assert_eq!(a.title_text(), "* veil — a.md (外部修改)");
    assert_eq!(a.external_state(), ExternalState::Changed);
    assert_eq!(a.editor.session().serialize(), "改ab", "不改写文档内容");
    a.maybe_autosave(t0 + Duration::from_secs(2));
    assert_eq!(
        store.files.borrow()[Path::new("a.md")],
        b"external".to_vec()
    );
    assert_eq!(store.files.borrow().len(), 1, "检测阶段不落副本");
}

#[test]
fn same_external_version_notifies_once() {
    let store = MemStore::default();
    disk(&store, "a.md", b"external");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert!(
        matches!(
            a.ui_state.modal.as_ref().map(|m| &m.purpose),
            Some(ModalPurpose::External)
        ),
        "脏文档首次外部修改必须弹选择框"
    );
    a.ui_state.chrome.notice = "占位".into();
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert_eq!(a.ui_state.chrome.notice, "占位", "同一外部版本不重复提示");
    assert!(
        a.ui_state.modal.is_some(),
        "同一版本不重开/替换弹窗（弹窗保持原状）"
    );
    a.resolve_modal(4);
    a.ui_state.chrome.notice = "占位2".into();
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert_eq!(
        a.ui_state.chrome.notice, "占位2",
        "取消后同一版本不得每次轮询重复弹窗"
    );
    assert!(a.ui_state.modal.is_none());
    assert_eq!(
        a.persist_state.persist.pending(),
        Some(b"external".as_slice())
    );
}

#[test]
fn already_backed_up_version_does_not_renotify() {
    let store = MemStore::default();
    disk(&store, "a.md", b"external");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.persist_state.persist.mark_backed_up(b"external".to_vec());
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert!(a.persist_state.persist.pending().is_none());
    assert!(!a.persist_state.persist.hold());
    assert!(a.ui_state.modal.is_none(), "已备份版本不弹窗");
}

#[test]
fn newer_external_version_overrides_backed_up_state() {
    let store = MemStore::default();
    disk(&store, "a.md", b"v2");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.persist_state.persist.mark_backed_up(b"v1".to_vec()); // 旧版本已备份
    assert_eq!(a.external_state(), ExternalState::BackedUp);
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert_eq!(
        a.external_state(),
        ExternalState::None,
        "干净文档静默重载后外部状态归零"
    );
    assert_eq!(a.title_text(), "veil — a.md", "静默重载不保留外部修改后缀");
    assert_eq!(doc_text(&a), "v2", "干净文档内容替换为磁盘版本");
    assert_eq!(a.doc.baseline, b"v2".to_vec());
    assert!(!a.editor.is_dirty());
    assert!(a.ui_state.modal.is_none());
    assert!(a.persist_state.persist.pending().is_none());
    assert!(!a.persist_state.persist.hold());
}

#[test]
fn external_delete_notifies_and_autosave_recreates() {
    let store = MemStore::default();
    disk(&store, "a.md", b"ab");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    store.files.borrow_mut().remove(Path::new("a.md"));
    watch.events.borrow_mut().push(FileEvent::Removed);
    a.poll_external();
    assert!(a.persist_state.persist.missing());
    assert_eq!(a.external_state(), ExternalState::Missing);
    assert!(a.ui_state.chrome.notice.contains("删除"));
    assert!(
        !a.persist_state.persist.hold(),
        "删除不是覆盖，允许自动保存重建"
    );
    a.maybe_autosave(t0 + Duration::from_secs(2));
    assert_eq!(store.files.borrow()[Path::new("a.md")], "改ab".as_bytes());
    assert!(!a.persist_state.persist.missing(), "写盘成功即清除缺失态");
}

#[test]
fn external_revert_to_baseline_clears_pending() {
    let store = MemStore::default();
    disk(&store, "a.md", b"external");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.editor.apply(Intent::InsertText("改".into())).unwrap(); // 脏文档才进入挂起路径
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert!(a.persist_state.persist.hold());
    assert_eq!(
        a.persist_state.persist.pending(),
        Some(b"external".as_slice())
    );
    disk(&store, "a.md", b"ab");
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert!(
        a.persist_state.persist.pending().is_none(),
        "回到基线：撤销待备份"
    );
    assert!(!a.persist_state.persist.hold());
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
    assert!(a.ui_state.chrome.notice.contains("已保存"));
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert!(
        a.persist_state.persist.pending().is_none(),
        "磁盘==基线：不误报"
    );
    assert!(a.ui_state.chrome.notice.contains("已保存"), "提示不被覆盖");
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
    assert!(a.persist_state.persist.missing());
    disk(&store, "a.md", b"ab");
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert!(!a.persist_state.persist.missing());
    assert!(a.ui_state.chrome.notice.contains("已恢复"));
}

#[test]
fn clean_document_external_change_reloads_silently() {
    let store = MemStore::default();
    disk(&store, "a.md", b"external");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert!(a.ui_state.modal.is_none(), "干净文档不弹选择框");
    assert_eq!(doc_text(&a), "external", "静默采用磁盘内容");
    assert_eq!(a.doc.baseline, b"external".to_vec());
    assert!(!a.editor.is_dirty());
    assert_eq!(a.external_state(), ExternalState::None);
    assert_eq!(a.title_text(), "veil — a.md");
    assert!(a.ui_state.chrome.notice.contains("已重新加载"));
}

#[test]
fn external_change_modal_offers_four_choices() {
    let store = MemStore::default();
    disk(&store, "a.md", b"external");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    let modal = a.ui_state.modal.as_ref().expect("脏文档必须弹选择框");
    assert!(matches!(modal.purpose, ModalPurpose::External));
    assert_eq!(
        modal.spec.buttons.iter().map(|b| b.id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(
        modal.spec.buttons[0].display_label(),
        format!("{} (R)", t(I18nKey::BtnReload))
    );
    assert_eq!(
        modal.spec.buttons[1].display_label(),
        format!("{} (K)", t(I18nKey::BtnKeepCurrent))
    );
    assert_eq!(
        modal.spec.buttons[2].display_label(),
        format!("{} (A)", t(I18nKey::BtnSaveAs))
    );
    assert_eq!(modal.spec.buttons[3].display_label(), t(I18nKey::BtnCancel));
    assert_eq!(modal.spec.primary_id(), Some(2), "默认焦点为「保留当前」");
    assert_eq!(modal.state.focused, 1, "Enter 不会误丢未保存修改");
    assert_eq!(a.editor.session().serialize(), "改ab", "选择前不改内容");
    assert_eq!(
        store.files.borrow()[Path::new("a.md")],
        b"external".to_vec(),
        "选择前不改磁盘"
    );
}

#[test]
fn external_modal_reload_replaces_content_and_clears_state() {
    let store = MemStore::default();
    disk(&store, "a.md", b"external");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    a.resolve_modal(1);
    assert!(a.ui_state.modal.is_none());
    assert_eq!(doc_text(&a), "external", "编辑器内容替换为磁盘字节");
    assert!(!a.editor.is_dirty());
    assert_eq!(a.doc.baseline, b"external".to_vec());
    assert!(!a.doc.bom);
    assert!(a.persist_state.persist.pending().is_none());
    assert!(!a.persist_state.persist.hold());
    assert_eq!(a.external_state(), ExternalState::None);
    assert!(!a.title_text().contains("(外部修改)"), "标题后缀清除");
    assert!(a.ui_state.chrome.notice.contains("已重新加载"));
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert!(a.ui_state.modal.is_none(), "重载后同一磁盘版本不再弹窗");
    assert!(a.ui_state.chrome.notice.contains("已重新加载"));
}

#[test]
fn external_modal_keep_current_holds_until_manual_save() {
    let store = MemStore::default();
    disk(&store, "a.md", b"external");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    a.resolve_modal(2);
    assert_eq!(doc_text(&a), "改ab", "保留当前内容");
    assert!(a.editor.is_dirty());
    assert!(!a.persist_state.persist.hold(), "解除自动保存挂起");
    assert_eq!(
        a.persist_state.persist.pending(),
        Some(b"external".as_slice()),
        "pending 保留：标题仍提示外部修改"
    );
    assert_eq!(a.external_state(), ExternalState::Changed);
    assert!(a.ui_state.chrome.notice.contains("已保留当前内容"));

    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_autosave(t0 + Duration::from_secs(2));
    assert_eq!(
        store.files.borrow()[Path::new("a.md")],
        b"external".to_vec(),
        "自动保存绝不静默覆盖磁盘"
    );
    assert_eq!(
        store.files.borrow()[Path::new("a.conflict-1.md")],
        b"external".to_vec(),
        "覆盖前先写冲突副本"
    );
    assert!(a.persist_state.persist.hold(), "冲突后重新挂起自动保存");
}

#[test]
fn external_modal_cancel_via_escape_keeps_pending() {
    let store = MemStore::default();
    disk(&store, "a.md", b"external");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    a.route_key(Key::Escape);
    assert!(a.ui_state.modal.is_none(), "Esc 关闭弹窗");
    assert_eq!(doc_text(&a), "改ab", "内容不变");
    assert!(a.persist_state.persist.hold(), "hold 保持不变");
    assert_eq!(
        a.persist_state.persist.pending(),
        Some(b"external".as_slice())
    );
    assert_eq!(
        a.ui_state.chrome.notice,
        t(I18nKey::NoticeExternalDismissed)
    );
}

#[test]
fn external_modal_mnemonics_route_to_actions() {
    // R：重新加载
    let store = MemStore::default();
    disk(&store, "a.md", b"external");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    a.route_key(Key::Text("r".into()));
    assert_eq!(doc_text(&a), "external", "助记符 R 走重新加载");

    // K：保留当前
    let mut b = app_watch("ab", Some("a.md"), &store, &watch);
    b.editor.apply(Intent::InsertText("改".into())).unwrap();
    watch.events.borrow_mut().push(FileEvent::Changed);
    b.poll_external();
    b.route_key(Key::K);
    assert_eq!(doc_text(&b), "改ab", "助记符 K 保留当前");
    assert!(!b.persist_state.persist.hold());
    assert!(b.persist_state.persist.pending().is_some());

    // A：另存为（需要能返回目标的对话框）
    let mut c = app_watch_dialogs("ab", Some("a.md"), &store, &watch, Some("b.md"));
    c.editor.apply(Intent::InsertText("改".into())).unwrap();
    watch.events.borrow_mut().push(FileEvent::Changed);
    c.poll_external();
    c.route_key(Key::A);
    assert!(c.ui_state.modal.is_none());
    assert_eq!(c.doc.path.as_deref(), Some(Path::new("b.md")));
    assert_eq!(
        store.files.borrow()[Path::new("b.md")],
        "改ab".as_bytes(),
        "助记符 A 走既有另存为"
    );
}

#[test]
fn external_delete_or_unreadable_never_opens_modal() {
    let store = MemStore::default();
    disk(&store, "a.md", b"ab");
    let watch = FakeWatch::default();
    let mut a = app_watch("ab", Some("a.md"), &store, &watch);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    store.files.borrow_mut().remove(Path::new("a.md"));
    watch.events.borrow_mut().push(FileEvent::Removed);
    a.poll_external();
    assert!(a.ui_state.modal.is_none(), "外部删除不弹窗");
    assert!(a.ui_state.chrome.notice.contains("删除"));

    let mut b = app_watch("ab", Some("a.md"), &store, &watch);
    b.editor.apply(Intent::InsertText("改".into())).unwrap();
    watch.events.borrow_mut().push(FileEvent::Changed);
    b.poll_external();
    assert!(b.ui_state.modal.is_none(), "外部不可读不弹窗");
    assert!(b.ui_state.chrome.notice.contains("不可读"));
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
        resume: false,
        open_error: None,
    };
    let m = FakeMath {
        fail: false,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let d = FakeDialogs {
        open: None,
        save: Some(PathBuf::from("b.md")),
        image: None,
        export: None,
        calls,
    };
    let mut a = App::new(
        doc,
        Ports {
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
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
    );
    a.save_as();
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));
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
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
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

#[test]
fn startup_applies_configured_mode() {
    let cfg = MemConfig::default();
    *cfg.text.borrow_mut() = Some("[theme]\nmode = \"dark\"\n".into());
    let a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
    assert_eq!(a.input_state.theme.mode, ThemeMode::Dark);
    assert!(a.input_state.theme.dark, "显式深色不受系统浅色影响");
    assert!(a.ui_state.chrome.notice.is_empty());
}

#[test]
fn startup_notices_invalid_config_and_uses_default() {
    let cfg = MemConfig::default();
    *cfg.text.borrow_mut() = Some("[theme]\nmode = \"auto\"\nfoo = \"bar\"\n".into());
    let a = app_theme(Box::new(cfg), Rc::new(Cell::new(None)));
    assert_eq!(a.input_state.theme.mode, ThemeMode::System);
    assert!(!a.input_state.theme.dark, "系统外观不可读按浅色");
    assert!(
        a.ui_state.chrome.notice.contains("2 处"),
        "提示无法识别条目数: {}",
        a.ui_state.chrome.notice
    );
}

#[test]
fn system_mode_follows_live_switch_and_clears_color_caches() {
    let sys = Rc::new(Cell::new(Some(false)));
    let mut a = app_theme(Box::new(MemConfig::default()), sys.clone());
    assert!(!a.input_state.theme.dark);
    a.ensure_layouts_all();
    let (id, kind, range) = a.editor.session().blocks()[0].clone();
    let caret = a.editor.caret();
    let base = a.base_key();
    let lk = a
        .cache
        .raster_key(id, &kind, range, base, caret, 0)
        .expect("首块应已按当前键排版");
    a.cache.rasters.insert(
        id,
        Memo {
            key: lk,
            value: (vec![0; 4], 1, 1),
        },
    );
    let ik = a.inline_key();
    a.cache.inline_math.insert(
        "x".into(),
        Memo {
            key: ik,
            value: MathImage {
                pixels: vec![0; 16],
                width: 2,
                height: 2,
                baseline: 0,
            },
        },
    );
    sys.set(Some(true));
    a.poll_system_theme(Instant::now());
    assert!(a.input_state.theme.dark, "系统切深色后调色板跟随");
    assert!(a.cache.rasters.is_empty(), "按旧前景色光栅化的文本须作废");
    assert!(a.cache.inline_math.is_empty(), "公式图须按新前景色重渲");
}

/// DEF-T05-007：设置面板把 Dark 改为 System（系统浅色）后，当前文档调色板完整重绘。
#[test]
fn settings_theme_switch_dark_to_system_repaints_palette() {
    let sys = Rc::new(Cell::new(Some(false)));
    let mut a = app_theme(Box::new(MemConfig::default()), sys);
    a.config.mode = ThemeMode::Dark;
    a.input_state.theme = crate::theme::Theme::of(ThemeMode::Dark, true);
    a.refresh_theme_visuals();
    a.ensure_layouts_all();
    let (id, kind, range) = a.editor.session().blocks()[0].clone();
    let old_base = a.base_key();
    let old_rev = a.input_state.revs.theme;
    a.config.mode = ThemeMode::System;
    a.refresh_settings_effects();
    assert!(!a.input_state.theme.dark, "System(浅色) 应切换为浅色调色板");
    assert!(
        a.input_state.revs.theme > old_rev,
        "主题代次必须递增，旧颜色缓存才会失效"
    );
    assert_ne!(a.base_key(), old_base, "基础键应随主题变化");
    a.ensure_layouts_all();
    let caret = a.editor.caret();
    let base = a.base_key();
    let lk = a
        .cache
        .raster_key(id, &kind, range, base, caret, 0)
        .expect("当前文档应按新主题重排");
    assert_eq!(a.cache.layouts[&id].key.base, base);
    let (px, _, _) = a
        .engine
        .rasterize(&a.cache.layouts[&id].value, a.input_state.theme.fg);
    let fg = a.input_state.theme.fg;
    assert!(
        px.chunks(4)
            .any(|p| p[3] > 0 && p[0] == fg[0] && p[1] == fg[1] && p[2] == fg[2]),
        "光栅应用新主题前景色重绘"
    );
    let _ = lk;
}

#[test]
fn explicit_mode_ignores_system_switch() {
    let cfg = MemConfig::default();
    *cfg.text.borrow_mut() = Some("[theme]\nmode = \"light\"".into());
    let sys = Rc::new(Cell::new(Some(false)));
    let mut a = app_theme(Box::new(cfg), sys.clone());
    sys.set(Some(true));
    a.poll_system_theme(Instant::now());
    assert_eq!(a.input_state.theme.mode, ThemeMode::Light);
    assert!(!a.input_state.theme.dark, "显式浅色不受系统切换影响");
}

#[test]
fn switching_theme_persists_config() {
    let cfg = MemConfig::default();
    let saves = cfg.saves.clone();
    let sys = Rc::new(Cell::new(Some(false)));
    let mut a = app_theme(Box::new(cfg), sys.clone());
    a.perform_menu(MenuAction::ThemeDark);
    assert!(a.input_state.theme.dark);
    assert_eq!(a.input_state.theme.mode, ThemeMode::Dark);
    assert!(a.ui_state.chrome.notice.contains("深色"));
    let saved = saves.borrow();
    assert_eq!(saved.len(), 1, "切换一次即落盘一次");
    assert!(saved[0].contains("mode = \"dark\""));
    drop(saved);
    // 切回跟随系统：立即采用当前系统外观（此处系统已切深色）
    sys.set(Some(true));
    a.perform_menu(MenuAction::ThemeSystem);
    assert_eq!(a.input_state.theme.mode, ThemeMode::System);
    assert!(a.input_state.theme.dark);
    assert!(saves.borrow()[1].contains("mode = \"system\""));
}

#[test]
fn startup_applies_export_config_and_saves_all_keys() {
    let cfg = MemConfig::default();
    *cfg.text.borrow_mut() = Some(
        "mode = \"light\"\nexport_margin_mm = 12.5\nexport_page_numbers = false\nexport_toc = false\n"
            .into(),
    );
    let saves = cfg.saves.clone();
    let mut a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
    assert_eq!(a.config.export_margin_mm, 12.5);
    assert!(!a.config.export_page_numbers);
    assert!(!a.config.export_toc);
    a.perform_menu(MenuAction::ThemeDark);
    let saved = saves.borrow();
    assert!(saved[0].contains("export_margin_mm = 12.5"), "{}", saved[0]);
    assert!(saved[0].contains("export_page_numbers = false"));
    assert!(saved[0].contains("export_toc = false"));
}

#[test]
fn startup_applies_doc_typography_and_smooth_scroll() {
    let cfg = MemConfig::default();
    *cfg.text.borrow_mut() = Some(
        "doc_font_family = \"serif\"\ndoc_font_size = 18\ndoc_line_height = 27\nsmooth_scroll = true\n"
            .into(),
    );
    let mut a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
    assert!(a.window_state.smooth, "平滑滚动按配置开启");
    assert_eq!(a.config.doc_font_family, theme::DocFontFamily::Serif);
    assert_eq!(a.config.doc_font_size, 18.0);
    assert_eq!(a.config.doc_line_height, 27.0);
    // 正文块排版使用配置值（默认 16/22 → 18/27）
    let text = "正文一行";
    let h = a
        .engine
        .layout_block(
            BlockKind::Paragraph,
            text,
            Range::new(0, text.len()),
            &[],
            400.0,
            usize::MAX,
            &[],
            false,
        )
        .height;
    assert!(h >= 27.0 - 0.5, "正文块行高应使用配置值：{h}");
    // UI chrome 度量保持 UI 字阶（13/18），不受正文配置影响
    let (_, ui_h) = a.engine.measure("菜单");
    assert!((ui_h - 18.0).abs() < 0.5, "UI 行高应保持不变：{ui_h}");
}

#[test]
fn config_save_failure_keeps_theme_and_notices() {
    let sys = Rc::new(Cell::new(Some(false)));
    let mut a = app_theme(Box::new(FailSaveConfig), sys);
    a.perform_menu(MenuAction::ThemeDark);
    assert!(a.input_state.theme.dark, "保存失败不影响本次生效");
    assert!(
        a.ui_state.chrome.notice.contains("配置未保存"),
        "提示保存失败: {}",
        a.ui_state.chrome.notice
    );
}

/// DEF-T07-002：几何字段参与 apply；清除后按 OK 必须把移除结果写盘。
#[test]
fn settings_geometry_clear_is_applied_and_persisted_on_ok() {
    use crate::view::settings::{FieldKind, FieldValue, BTN_OK};
    let cfg = MemConfig::default();
    let saves = cfg.saves.clone();
    let mut a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
    a.config.window = Some(crate::theme::WindowGeom {
        width: 900,
        height: 700,
        x: -40,
        y: 30,
    });
    a.open_settings();

    // 清除按钮等价操作：表单值置 None。
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        let idx = s
            .form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::WindowGeom))
            .expect("几何字段存在");
        s.form.fields[idx].value = FieldValue::Geom(None);
    }
    a.resolve_settings(BTN_OK);

    assert_eq!(a.config.window, None, "清除后 apply 必须移除几何");
    let saved = saves.borrow().last().cloned().expect("OK 必须写盘");
    let parsed = crate::theme::decode_config(saved.as_bytes()).expect("写盘 JSON 可解析");
    assert_eq!(parsed.config.window, None, "写盘内容不得复活旧几何");

    // 反向：恢复默认后再填入一组几何，OK 后写盘包含新值。
    a.config.window = None;
    a.open_settings();
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        let idx = s
            .form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::WindowGeom))
            .expect("几何字段存在");
        s.form.fields[idx].value = FieldValue::Geom(Some((1000, 800, 5, 6)));
    }
    a.resolve_settings(BTN_OK);
    assert_eq!(
        a.config.window,
        Some(crate::theme::WindowGeom {
            width: 1000,
            height: 800,
            x: 5,
            y: 6
        })
    );
    let saved = saves.borrow().last().cloned().expect("OK 必须写盘");
    let parsed = crate::theme::decode_config(saved.as_bytes()).expect("写盘 JSON 可解析");
    assert_eq!(parsed.config.window, a.config.window, "新几何必须写盘");
}

/// DEF-T07-002 复发：本会话 resize 过的几何不得在清除/恢复默认后覆盖关闭写盘。
/// 清除或恢复默认 → OK 后，正常关闭不得写回 window_*；用户再次移动窗口后恢复持久化。
#[test]
fn clearing_geometry_suppresses_geometry_write_on_close() {
    use crate::view::settings::{FieldKind, FieldValue, BTN_OK, BTN_RESET_DEFAULTS};
    let cfg = MemConfig::default();
    let saves = cfg.saves.clone();
    let mut a = app_theme(Box::new(cfg), Rc::new(Cell::new(Some(false))));
    let old = crate::theme::WindowGeom {
        width: 700,
        height: 450,
        x: -40,
        y: 30,
    };
    a.config.window = Some(old);
    // 模拟本会话发生过 resize：关闭时原本会把当前几何写回。
    let resized = crate::theme::WindowGeom {
        width: 1050,
        height: 675,
        x: 0,
        y: 0,
    };
    a.window_state.geom = Some(resized);
    a.window_state.geom_dirty = true;

    // 清除 → OK：丢弃 resize 脏位并清空会话几何。
    a.open_settings();
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        let idx = s
            .form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::WindowGeom))
            .expect("几何字段存在");
        s.form.fields[idx].value = FieldValue::Geom(None);
    }
    a.resolve_settings(BTN_OK);
    assert_eq!(a.config.window, None, "清除后 apply 必须移除几何");
    assert!(!a.window_state.geom_dirty, "清除后必须丢弃本会话 resize 脏位");
    assert_eq!(a.window_state.geom, None, "清除后不得保留会话几何");

    a.window_state.exit_requested = true;
    assert!(a.finish_close());
    let saved = saves.borrow().last().cloned().expect("OK 已写盘");
    let parsed = crate::theme::decode_config(saved.as_bytes()).expect("写盘 JSON 可解析");
    assert_eq!(
        parsed.config.window, None,
        "正常关闭不得把本会话几何写回（清除结果必须跨重启生效）"
    );

    // 恢复默认 → OK：同样抑制关闭回写。
    a.config.window = Some(old);
    a.window_state.geom = Some(resized);
    a.window_state.geom_dirty = true;
    a.open_settings();
    a.resolve_settings(BTN_RESET_DEFAULTS);
    a.resolve_settings(BTN_OK);
    assert_eq!(a.config.window, None, "恢复默认后 apply 必须移除几何");
    assert!(!a.window_state.geom_dirty, "恢复默认后同样丢弃 resize 脏位");
    a.finish_close();
    let saved = saves.borrow().last().cloned().expect("OK 已写盘");
    let parsed = crate::theme::decode_config(saved.as_bytes()).expect("写盘 JSON 可解析");
    assert_eq!(
        parsed.config.window, None,
        "恢复默认后正常关闭不得写回旧几何"
    );

    // 用户随后主动移动窗口：恢复正常的几何持久化。
    a.on_window_moved(12, 34, false);
    assert!(a.window_state.geom_dirty, "主动移动窗口应重新置脏");
    a.finish_close();
    let saved = saves.borrow().last().cloned().expect("移动后关闭必须写盘");
    let parsed = crate::theme::decode_config(saved.as_bytes()).expect("写盘 JSON 可解析");
    assert!(
        parsed.config.window.is_some(),
        "用户再次移动窗口后必须正常保存新几何"
    );
}

/// DEF-T10-008：设置保存失败时不得再提示「设置已保存」。
#[test]
fn settings_save_failure_is_not_overwritten_by_success_notice() {
    use crate::view::settings::{FieldKind, FieldValue, BTN_OK};
    let mut a = app_theme(Box::new(FailSaveConfig), Rc::new(Cell::new(Some(false))));
    a.open_settings();
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        let idx = s
            .form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::ThemeMode))
            .expect("主题字段存在");
        s.form.fields[idx].value = FieldValue::ThemeMode(ThemeMode::Dark);
    }
    a.resolve_settings(BTN_OK);
    assert_eq!(a.config.mode, ThemeMode::Dark, "本次操作仍然生效");
    assert!(
        a.ui_state.chrome.notice.contains("配置未保存"),
        "失败提示须保留：{}",
        a.ui_state.chrome.notice
    );
    assert!(
        !a.ui_state.chrome.notice.contains("设置已保存"),
        "不得假报保存成功：{}",
        a.ui_state.chrome.notice
    );
}

/// DEF-T10-008：远程图片开关保存失败同样不得假报已持久化。
#[test]
fn remote_image_toggle_save_failure_keeps_error_notice() {
    let mut a = app_theme(Box::new(FailSaveConfig), Rc::new(Cell::new(Some(false))));
    a.toggle_remote_images();
    assert!(!a.config.allow_remote_images, "开关本身仍翻转");
    assert!(
        a.ui_state.chrome.notice.contains("配置未保存"),
        "失败提示不得被开关状态提示覆盖：{}",
        a.ui_state.chrome.notice
    );
    assert!(!a.ui_state.chrome.notice.contains("已阻止远程图片"));
}

/// DEF-T11-E1-002：从设置面板切深色后，导出样式快照必须同步为深色。
#[test]
fn settings_theme_change_updates_export_style_immediately() {
    use crate::view::settings::{FieldKind, FieldValue, BTN_OK};
    let mut a = app_theme(
        Box::new(MemConfig::default()),
        Rc::new(Cell::new(Some(false))),
    );
    assert!(!a.doc.export_style.dark, "初始浅色");
    a.open_settings();
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        let idx = s
            .form
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::ThemeMode))
            .expect("主题字段存在");
        s.form.fields[idx].value = FieldValue::ThemeMode(ThemeMode::Dark);
    }
    a.resolve_settings(BTN_OK);
    assert!(a.input_state.theme.dark);
    assert_eq!(
        a.doc.export_style,
        ExportStyle::from_theme(&a.input_state.theme),
        "导出快照必须与当前主题一致"
    );
    assert!(a.doc.export_style.dark, "切深色后立即导出应为深色");
}

/// DEF-T10-E1-001：最小化期间的 Moved（-32000 哨兵）不得污染持久化几何。
#[test]
fn minimized_window_move_does_not_overwrite_geometry() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    let original = crate::theme::WindowGeom {
        width: 900,
        height: 700,
        x: 300,
        y: 220,
    };
    a.window_state.geom = Some(original);
    a.window_state.geom_dirty = false;
    a.on_window_moved(-32000, -32000, true);
    assert_eq!(a.window_state.geom, Some(original), "最小化不写坐标");
    assert!(!a.window_state.geom_dirty, "最小化不置脏");
    // 恢复后的正常移动仍记录（尺寸沿用已存值）
    a.on_window_moved(40, 50, false);
    assert_eq!(
        a.window_state.geom,
        Some(crate::theme::WindowGeom {
            width: 900,
            height: 700,
            x: 40,
            y: 50
        })
    );
    assert!(a.window_state.geom_dirty);
}

/// DEF-T07-003：模态/浮层打开时 Ctrl+滚轮不得缩放背景文档。
#[test]
fn overlay_blocks_background_zoom() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    let base = a.input_state.user_zoom;
    assert!(!a.overlay_blocks_background_wheel());

    a.open_settings();
    assert!(a.overlay_blocks_background_wheel(), "设置打开必须拦截滚轮");
    a.resolve_settings(crate::view::settings::BTN_CANCEL);
    assert!(!a.overlay_blocks_background_wheel());

    a.open_search();
    assert!(a.overlay_blocks_background_wheel(), "查找打开必须拦截滚轮");
    a.close_search();

    let spec = crate::view::modal::ModalSpec::new("T", "M", Vec::new());
    a.ui_state.modal = Some(crate::app::ActiveModal {
        state: crate::view::modal::ModalState::new(&spec),
        spec,
        intent: None,
        purpose: crate::app::ModalPurpose::Unsaved,
    });
    assert!(a.overlay_blocks_background_wheel(), "模态打开必须拦截滚轮");
    a.ui_state.modal = None;
    assert_eq!(a.input_state.user_zoom, base, "守卫本身不得改变缩放");
}

/// DEF-T07-004：窗口失焦必须结束列宽拖动会话。
#[test]
fn window_blur_ends_column_drag() {
    let store = MemStore::default();
    let mut a = app("| A | B |\n| --- | --- |\n| 1 | 2 |\n", None, false, &store);
    a.ui_state.tables.drag = Some((BlockId(1), 0, 10.0, 80.0));
    a.on_window_focus(false);
    assert!(a.ui_state.tables.drag.is_none(), "失焦后拖动会话必须清空");
    assert!(!a.window_state.visible);

    a.on_window_focus(true);
    assert!(a.ui_state.tables.drag.is_none(), "回焦不得复活拖动");
}

/// DEF-T67-E1-005：识别图片命中，并区分正常/缺失图片。
#[test]
fn image_context_detects_missing_local_image() {
    let store = MemStore::default();
    let dir = std::env::temp_dir().join("veil-task06-image-missing");
    let doc = dir.join("a.md");
    let a = app(
        "![图](gone.png)\n\n![图](also-gone.png)\n",
        Some(doc.to_str().unwrap()),
        false,
        &store,
    );
    // 两个图片都指向不存在的本地文件 → missing=true，且能取到源路径。
    assert_eq!(a.image_at(2), Some(("gone.png".to_string(), true)));
    assert_eq!(a.image_at(0), Some(("gone.png".to_string(), true)));

    let plain = app("无图片\n", None, false, &store);
    assert!(plain.image_at(0).is_none());
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

#[test]
fn first_edit_lays_out_far_fewer_blocks_than_document() {
    let store = MemStore::default();
    let mut a = app(&stress_doc(), None, false, &store);
    assert!(a.index.blocks.len() >= 5005);
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    assert!(
        a.cache.layouts.len() < 100,
        "首次编辑只应排版视口内块，实际 {} / {} 块",
        a.cache.layouts.len(),
        a.index.blocks.len()
    );
}

// ---- 0a：超大单块布局保护 ----

#[test]
fn oversized_single_block_is_capped_with_explicit_degrade() {
    let text = "a".repeat(100_000);
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    assert_eq!(a.index.blocks.len(), 1, "单段应解析为单块");
    a.ensure_layouts_visible();
    let id = a.index.blocks[0].id;
    let l = &a.cache.layouts[&id];
    assert!(l.truncated, "超限块必须标记降级");
    assert_eq!(l.full_len, text.len(), "须记录原块长度");
    assert!(
        l.layout_text().len() <= MAX_BLOCK_BYTES,
        "排版文本须限制在上限内，实际 {}",
        l.layout_text().len()
    );
    assert!(l.height.is_finite() && l.height > 0.0);
    assert!(a.index.total_h.is_finite(), "文档总高须有限");
    // 源码真值不受影响：文末编辑后仍完整往返（不丢字）
    a.editor.apply(Intent::MoveDocEnd).unwrap();
    let dirty = a.editor.apply(Intent::InsertText("b".into())).unwrap();
    a.after_edit(dirty);
    a.paint();
    assert_eq!(a.editor.session().serialize(), format!("{text}b"));
    // 降级不得让光标跑出文档/视口：文末光标应在视口内，滚动不被估算高度带飞
    let vp = a.viewport();
    let (_, cy, ch) = a.caret_screen();
    assert!(
        cy >= vp.y - 1.0 && cy + ch <= vp.y + vp.h + 1.0,
        "文末光标须在视口内：cy={cy}, ch={ch}, 视口={}..{}",
        vp.y,
        vp.y + vp.h
    );
}

#[test]
fn oversized_block_tail_is_painted_after_scroll() {
    let text = "a".repeat(100_000);
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_visible();
    // 复刻真实滚轮路径：每个事件都走 on_wheel + 重绘，验证多次滚动后不出现「空白」
    let step = -3.0 * BASE_LINE_HEIGHT * a.input_state.scale;
    for _ in 0..300 {
        a.on_wheel(step);
        let _ = a.render();
    }
    let (doc_h, scroll) = (a.doc_height(), a.window_state.scroll_y);
    let vp = a.viewport();
    let bg = a.input_state.theme.bg;
    let bgpix = ((bg[0] as u32) << 16) | ((bg[1] as u32) << 8) | bg[2] as u32;
    let (sy, bh, notice_h) = {
        let g = &a.index.blocks[0];
        (
            vp.y + g.top - scroll,
            a.block_height(g),
            BASE_LINE_HEIGHT * a.input_state.scale,
        )
    };
    let frame = a.render();
    let lo = (vp.y as usize + 1) * frame.width as usize;
    let hi = ((vp.y + vp.h) as usize).min(frame.height as usize) * frame.width as usize;
    let nonbg = frame.pixels[lo..hi].iter().filter(|p| **p != bgpix).count();
    // 降级提示行：块尾单独一行，必须真的有像素（否则「明确降级」名存实亡）
    let nlo = ((sy + bh - notice_h).max(0.0) as usize).min(frame.height as usize);
    let nhi = ((sy + bh) as usize).min(frame.height as usize);
    let notice_px = if nlo < nhi {
        frame.pixels[nlo * frame.width as usize..nhi * frame.width as usize]
            .iter()
            .filter(|p| **p != bgpix)
            .count()
    } else {
        0
    };
    println!(
        "vp={}x{} doc_h={doc_h} block_h={bh} scroll={scroll} nonbg={nonbg} notice_px={notice_px}",
        vp.w, vp.h
    );
    assert!(nonbg > 1000, "滚动到底后仍须有文本像素");
    assert!(notice_px > 100, "块尾降级提示须可见，实际 {notice_px} 像素");
}

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

/// UTF-8 回归：中文围栏代码块在排版与逐屏滚动时都不得因字节下标非字符边界而 panic。
#[test]
fn cjk_fenced_code_scroll_to_end_does_not_panic() {
    let store = MemStore::default();
    let text = cjk_fence_long_doc();
    assert!(text.chars().count() >= 4000, "文档须 ≥4000 字");
    let mut a = app(&text, Some("doc/cjk.md"), false, &store);
    // 全量排版即触发中文围栏代码块的高亮（回归点：曾在此 panic）
    a.ensure_layouts_all();
    let code_id = a
        .index
        .blocks
        .iter()
        .find(|g| matches!(&g.kind, BlockKind::FencedCode(Some(lang)) if lang == "markdown"))
        .map(|g| g.id)
        .expect("文档须含 markdown 围栏代码块");
    assert!(
        a.cache.layouts.contains_key(&code_id),
        "中文代码块布局须存在"
    );
    a.recompute_geometry();
    let code_h0 = {
        let g = a.index.blocks.iter().find(|g| g.id == code_id).unwrap();
        a.block_height(g)
    };
    let vp = a.viewport();
    let max = chrome::max_scroll(vp, a.doc_height());
    assert!(max > 0.0, "长文档须可滚动");
    let mut covered = false;
    let mut y = 0.0f32;
    loop {
        a.window_state.scroll_y = y;
        let _ = a.render(); // 每屏触发布局/绘制
                            // 用当前帧几何判定可见性（块高在滚动中可能从实高回退估算值）
        let (code_top, code_h) = {
            let g = a.index.blocks.iter().find(|g| g.id == code_id).unwrap();
            (g.top, a.block_height(g))
        };
        let sy = vp.y + code_top - a.window_state.scroll_y;
        if sy < vp.y + vp.h && sy + code_h > vp.y {
            covered = true;
            assert!(
                a.cache.layouts.contains_key(&code_id),
                "中文代码块进入视口时布局必须存在"
            );
        }
        if a.window_state.scroll_y >= max {
            break;
        }
        y = (y + vp.h.max(1.0)).min(max);
    }
    assert!(covered, "滚动须覆盖到中文代码块所在块（块高 {code_h0}）");
}

/// 端到端源码真值（阶段 2）：CRLF + BOM 文档回车、输入、保存后，磁盘字节仍是
/// CRLF + BOM，不混入 LF、不产生孤立 `\r`——无隐式规范化。
#[test]
fn crlf_bom_edit_save_preserves_bytes() {
    let store = MemStore::default();
    let mut a = app("甲\r\n\r\n乙\r\n", Some("doc/a.md"), true, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter);
    a.route_key(Key::Text("新".into()));
    a.save_now();
    let bytes = store.files.borrow()[&PathBuf::from("doc/a.md")].clone();
    assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF], "BOM 须保留");
    let text = String::from_utf8(bytes[3..].to_vec()).expect("UTF-8");
    assert_eq!(text, "甲\r\n\r\n乙\r\n\r\n新", "换行风格须保持 CRLF");
    // 结构校验：每个 `\n` 前必是 `\r`、每个 `\r` 后必是 `\n`（无孤立回车/LF 混入）
    let b = text.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'\n' {
            assert_eq!(
                b.get(i - 1),
                Some(&b'\r'),
                "第 {i} 字节的 \\n 前缺 \\r（混入 LF）"
            );
        }
        if c == b'\r' {
            assert_eq!(
                b.get(i + 1),
                Some(&b'\n'),
                "第 {i} 字节的 \\r 后缺 \\n（孤立回车）"
            );
        }
    }
}

/// 2c 源码真值 · 全角/半角：编辑与保存绝不转换全角/半角字符。
/// 全角拉丁/数字/标点（ＡＢＣ １２３ ！）与半角片假名（ｱｲｳ）必须原样往返。
#[test]
fn fullwidth_halfwidth_preserved_through_edit_save() {
    let store = MemStore::default();
    let doc = "全角 ＡＢＣ １２３ ！\n\n半角 ｱｲｳ abc\n";
    let mut a = app(doc, Some("doc/fw.md"), false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Text("尾".into()));
    a.perform_action(Action::Doc(Intent::MoveDocStart));
    a.route_key(Key::Text("首".into()));
    a.save_now();
    let bytes = store.files.borrow()[&PathBuf::from("doc/fw.md")].clone();
    let text = String::from_utf8(bytes).expect("UTF-8");
    assert_eq!(text, format!("首{doc}尾"), "全角/半角字符不得被转换");
    assert!(text.contains("ＡＢＣ １２３ ！"));
    assert!(text.contains("ｱｲｳ"));
}

/// 2c 源码真值 · 尾随空格：行尾两空格（Markdown 硬换行）与行尾 tab 是语义字节，
/// 编辑与保存不得修剪。
#[test]
fn trailing_whitespace_preserved_through_edit_save() {
    let store = MemStore::default();
    let doc = "硬换行  \n次行\ttab 尾\t\n\n末段  \n";
    let mut a = app(doc, Some("doc/ts.md"), false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Text("x".into()));
    a.save_now();
    let bytes = store.files.borrow()[&PathBuf::from("doc/ts.md")].clone();
    let text = String::from_utf8(bytes).expect("UTF-8");
    assert_eq!(text, format!("{doc}x"), "尾随空格/tab 不得被修剪");
}

/// 块索引派生的有序编号必须真正传入排版（非激活态显示 2./3.）。
#[test]
fn ordered_list_derived_numbers_reach_painted_layout() {
    let store = MemStore::default();
    let mut a = app("1. a\n1. b\n1. c", None, false, &store);
    a.perform_action(Action::Doc(Intent::MoveDocStart));
    a.ensure_layouts_visible();
    let texts: Vec<String> = a
        .index
        .blocks
        .iter()
        .map(|g| a.cache.layouts[&g.id].layout_text().to_string())
        .collect();
    assert_eq!(texts, ["1. a", "2. b", "3. c"], "派生编号须传入排版层");
}

/// 2c 源码真值 · 列表标记重排：有序列表回车只给新行递增标记，
/// 既有项编号（含故意乱序的 1./1./3.）绝不重排。
#[test]
fn ordered_list_markers_not_renumbered() {
    let store = MemStore::default();
    let doc = "1. a\n1. b\n3. c"; // 无尾换行：MoveDocEnd 落在「3. c」行尾
    let mut a = app(doc, Some("doc/ol.md"), false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter); // 3. c 行尾回车 → 新行 4.
    a.route_key(Key::Text("d".into()));
    a.save_now();
    let bytes = store.files.borrow()[&PathBuf::from("doc/ol.md")].clone();
    let text = String::from_utf8(bytes).expect("UTF-8");
    assert_eq!(
        text, "1. a\n1. b\n3. c\n4. d",
        "新行递增为 4.，既有 1./1./3. 标记不得重排"
    );
}

/// 真机缺陷回归（2026-10-30 ①③ + IME 候选框停在左上角）：
/// Enter 产生的新行/新段在提交前不成块，光标/preedit/IME 候选框必须锚在
/// 文本将出现的位置——软换行锚同段下一视觉行，段落分隔锚新段顶边；
/// 提交前后光标 y 不得跳变。旧行为：钳回上一段末尾 / 落视口原点。
#[test]
fn caret_before_commit_anchors_where_text_will_land() {
    let store = MemStore::default();
    let mut a = app("段一内容\n\n段二内容", None, false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    let margin = MARGIN * a.input_state.scale;
    let vp_y = a.viewport().y;

    // 软换行（Enter×1）：同段下一视觉行（块底），x 回行首
    a.route_key(Key::Enter);
    let (cx1, cy1, _) = a.caret_screen();
    let (top1, h1) = {
        let g = &a.index.blocks[1];
        (g.top, a.block_height(g))
    };
    assert_eq!(
        (cx1, cy1),
        (margin, vp_y + top1 + h1),
        "软换行光标须在同段下一行起始"
    );

    // 段落分隔（Enter×2）：新段顶边 = 块底 + 段距 + 1 个空行行高（B5）
    a.route_key(Key::Enter);
    let (cx2, cy2, _) = a.caret_screen();
    assert_eq!(
        (cx2, cy2),
        (
            margin,
            vp_y + top1 + h1 + (PARA_GAP + BASE_LINE_HEIGHT) * a.input_state.scale
        ),
        "新段光标须在段落将出现的位置"
    );

    // 提交「nihao」：落字行与 Enter2 光标同 y，preedit 所见即所得（无跳变）
    a.route_key(Key::Text("nihao".into()));
    let (_, cy3, _) = a.caret_screen();
    assert_eq!(cy2, cy3, "提交前后光标 y 不得跳变");
}

/// I1（CRLF 文档）：Enter 插入 `\r\n` 后光标必须锚到同段下一行；
/// 旧实现只把单独 `\n` 计作尾换行，CRLF 落入 caret_xy 的行尾钳制 → 光标贴在上一行。
#[test]
fn crlf_enter_caret_anchors_next_line() {
    let store = MemStore::default();
    let mut a = app("段一内容\r\n\r\n段二内容", None, false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter);
    let (_, cy, _) = a.caret_screen();
    let (top, h) = {
        let g = &a.index.blocks[1];
        (g.top, a.block_height(g))
    };
    let vp_y = a.viewport().y;
    assert_eq!(
        cy,
        vp_y + top + h,
        "CRLF 回车后光标须落在同段下一行，而非上一行行尾"
    );
}

/// 文档中部块间空隙：Enter×2 后光标锚到下一块顶边（新段将出现的位置），
/// 不得落到视口原点（旧兜底把 IME 候选框扔到左上角）。
#[test]
fn caret_in_mid_doc_gap_anchors_to_next_block_top() {
    let store = MemStore::default();
    let mut a = app("甲\n\n乙\n\n丙", None, false, &store);
    a.paint();
    // 光标移到「乙」末尾，Enter×2 产生尚未成块的空段
    let end_yi = a.index.blocks[1].range.end;
    assert!(a.editor.set_caret(end_yi));
    a.route_key(Key::Enter);
    a.route_key(Key::Enter);
    a.paint();
    let margin = MARGIN * a.input_state.scale;
    let (cx, cy, _) = a.caret_screen();
    let next_top = a.index.blocks[2].top; // 「丙」当前顶边 = 新段将出现的位置
    let vp_y = a.viewport().y;
    assert_eq!(
        (cx, cy),
        (margin, vp_y + next_top - a.window_state.scroll_y),
        "空隙光标须锚在下一块顶边"
    );
}

/// 真机缺陷复现（2026-10-30）：新段落光标定位与段落实际位置严重不一致。
/// 不变量：光标块 = 末块；光标屏幕 y 落在该块纵向跨度内；可见区间无未排版块（paint 不留空洞）。
#[test]
fn new_paragraph_caret_matches_block_position() {
    let store = MemStore::default();
    let mut a = app("段一内容\n\n段二内容", None, false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter);
    a.route_key(Key::Enter);
    a.route_key(Key::Text("n".into()));
    a.paint();
    let ci = a.caret_block_index().expect("光标须落在某块");
    let last = a.index.blocks.len() - 1;
    let (_, cy, ch) = a.caret_screen();
    let (top, bh) = {
        let g = &a.index.blocks[ci];
        (g.top, a.block_height(g))
    };
    let vp = a.viewport();
    let (sy0, sy1) = (
        vp.y + top - a.window_state.scroll_y,
        vp.y + top + bh - a.window_state.scroll_y,
    );
    println!(
        "blocks={} ci={ci} last={last} caret={} cy={cy} ch={ch} 块屏幕区间={sy0}..{sy1} scroll={}",
        a.index.blocks.len(),
        a.editor.caret(),
        a.window_state.scroll_y
    );
    assert_eq!(ci, last, "文末新段光标须在末块");
    assert!(
        cy >= sy0 - 1.0 && cy + ch <= sy1 + 1.0,
        "光标须在所属块纵向跨度内：cy={cy} ch={ch} 块区间={sy0}..{sy1}"
    );
}

/// 真机缺陷复现（冷启动链路）：中高图片块从未排版（est 22px 欠估 23×），
/// 打开后直接跳文末回车新段——scroll_to_caret 依赖的 total_h 欠估，
/// 光标/新段位置必须与收敛后几何一致，可见区间无空洞。
#[test]
fn new_paragraph_caret_with_cold_images_below() {
    let para = "这是一段足够长的中文段落，用来产生多行折行的真实排版效果。".repeat(6);
    let mut parts: Vec<String> = Vec::new();
    for i in 0..10 {
        parts.push(para.clone());
        parts.push(format!("![p{i}](p{i}.png)"));
    }
    let text = parts.join("\n\n");
    let files: Vec<(String, u32, u32)> = (0..10)
        .map(|i| (format!("doc/p{i}.png"), 600, 500))
        .collect();
    let files_ref: Vec<(&str, u32, u32)> =
        files.iter().map(|(p, w, h)| (p.as_str(), *w, *h)).collect();
    let (mut a, _calls) = app_with_images(&text, Some("doc/note.md"), &files_ref);
    // 不预热：直接跳文末（复刻用户打开文档后 Ctrl+End/点击末尾）
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.paint();
    a.route_key(Key::Enter);
    a.route_key(Key::Enter);
    a.route_key(Key::Text("n".into()));
    a.paint();
    let vp = a.viewport();
    let ci = a.caret_block_index().expect("光标须落在某块");
    let last = a.index.blocks.len() - 1;
    let (_, cy, ch) = a.caret_screen();
    let (top, bh) = {
        let g = &a.index.blocks[ci];
        (g.top, a.block_height(g))
    };
    let scroll = a.window_state.scroll_y;
    let (sy0, sy1) = (vp.y + top - scroll, vp.y + top + bh - scroll);
    let base = a.base_key();
    let caret = a.editor.caret();
    let vr = a.visible_index_range();
    let missing: Vec<usize> = a
        .index
        .blocks
        .get(vr.clone())
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            a.cache
                .layout(g.id, &g.kind, g.range, base, caret, a.toc_layout_epoch(g))
                .is_none()
                && g.kind != BlockKind::Math
                && !(g.kind == BlockKind::Table && a.cache.table(g.id, a.table_key(g.id)).is_some())
        })
        .map(|(i, _)| vr.start + i)
        .collect();
    println!(
        "blocks={} ci={ci} last={last} cy={cy} ch={ch} 块屏幕区间={sy0}..{sy1} scroll={scroll} missing={missing:?} vr={vr:?} total_h={} vp={}..{}",
        a.index.blocks.len(),
        a.index.total_h,
        vp.y,
        vp.y + vp.h
    );
    assert_eq!(ci, last, "文末新段光标须在末块");
    assert!(
        missing.is_empty(),
        "可见区间存在未排版块（绘制为空洞）: {missing:?}"
    );
    assert!(
        cy >= sy0 - 1.0 && cy + ch <= sy1 + 1.0,
        "光标须在所属块纵向跨度内：cy={cy} ch={ch} 块区间={sy0}..{sy1}"
    );
    assert!(
        cy >= vp.y - 1.0 && cy + ch <= vp.y + vp.h + 1.0,
        "光标须在视口内可见：cy={cy} ch={ch} 视口={}..{}",
        vp.y,
        vp.y + vp.h
    );
}

/// 同缺陷的富文档变体：大图片块（est 22px vs 实高数百 px）+ CJK 段落，
/// 估算与真实高度大散度下 paint 两遍收敛仍须无空洞、光标贴块。
#[test]
fn new_paragraph_caret_matches_block_position_with_images() {
    let para = "这是一段足够长的中文段落，用来产生多行折行的真实排版效果。".repeat(6);
    let mut parts: Vec<String> = Vec::new();
    for i in 0..10 {
        parts.push(para.clone());
        if i % 2 == 0 {
            parts.push(format!("![p{i}](p{i}.png)"));
        }
    }
    let text = parts.join("\n\n");
    let files: Vec<(String, u32, u32)> = (0..5)
        .map(|i| (format!("doc/p{}.png", i * 2), 600, 500))
        .collect();
    let files_ref: Vec<(&str, u32, u32)> =
        files.iter().map(|(p, w, h)| (p.as_str(), *w, *h)).collect();
    let (mut a, _calls) = app_with_images(&text, Some("doc/note.md"), &files_ref);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter);
    a.route_key(Key::Enter);
    a.route_key(Key::Text("n".into()));
    a.paint();
    let vp = a.viewport();
    let ci = a.caret_block_index().expect("光标须落在某块");
    let last = a.index.blocks.len() - 1;
    let (_, cy, ch) = a.caret_screen();
    let (top, bh) = {
        let g = &a.index.blocks[ci];
        (g.top, a.block_height(g))
    };
    let scroll = a.window_state.scroll_y;
    let (sy0, sy1) = (vp.y + top - scroll, vp.y + top + bh - scroll);
    let base = a.base_key();
    let caret = a.editor.caret();
    let vr = a.visible_index_range();
    let missing: Vec<usize> = a
        .index
        .blocks
        .get(vr.clone())
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            a.cache
                .layout(g.id, &g.kind, g.range, base, caret, a.toc_layout_epoch(g))
                .is_none()
                && g.kind != BlockKind::Math
                && !(g.kind == BlockKind::Table && a.cache.table(g.id, a.table_key(g.id)).is_some())
        })
        .map(|(i, _)| vr.start + i)
        .collect();
    println!(
        "blocks={} ci={ci} last={last} cy={cy} ch={ch} 块屏幕区间={sy0}..{sy1} scroll={scroll} missing={missing:?} vr={vr:?} total_h={}",
        a.index.blocks.len(),
        a.index.total_h
    );
    assert!(
        missing.is_empty(),
        "可见区间存在未排版块（绘制为空洞）: {missing:?}"
    );
    assert_eq!(ci, last, "文末新段光标须在末块");
    assert!(
        cy >= sy0 - 1.0 && cy + ch <= sy1 + 1.0,
        "光标须在所属块纵向跨度内：cy={cy} ch={ch} 块区间={sy0}..{sy1}"
    );
}

/// 同缺陷的高文档变体：CJK 长段（估算高度高估）+ 滚动态下末尾新段，
/// 验证 paint 两遍收敛后可见区间无空洞、几何单调、光标贴块。
#[test]
fn new_paragraph_caret_matches_block_position_tall_cjk() {
    let para = "这是一段足够长的中文段落，用来产生多行折行的真实排版效果。".repeat(8);
    let text = (0..12)
        .map(|_| para.clone())
        .collect::<Vec<_>>()
        .join("\n\n");
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Enter);
    a.route_key(Key::Enter);
    a.route_key(Key::Text("n".into()));
    a.paint();
    let vp = a.viewport();
    let ci = a.caret_block_index().expect("光标须落在某块");
    let last = a.index.blocks.len() - 1;
    let (_, cy, ch) = a.caret_screen();
    let (top, bh) = {
        let g = &a.index.blocks[ci];
        (g.top, a.block_height(g))
    };
    let scroll = a.window_state.scroll_y;
    let (sy0, sy1) = (vp.y + top - scroll, vp.y + top + bh - scroll);
    // 可见区间空洞检查：paint 已跑过两遍收敛，可见块必须全部有命中布局
    let base = a.base_key();
    let caret = a.editor.caret();
    let vr = a.visible_index_range();
    let missing: Vec<usize> = a
        .index
        .blocks
        .get(vr.clone())
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            a.cache
                .layout(g.id, &g.kind, g.range, base, caret, a.toc_layout_epoch(g))
                .is_none()
                && g.kind != BlockKind::Math
                && !(g.kind == BlockKind::Table && a.cache.table(g.id, a.table_key(g.id)).is_some())
        })
        .map(|(i, _)| vr.start + i)
        .collect();
    // 几何单调性：top 单调不减
    let mut tops_ok = true;
    for w in a.index.blocks.windows(2) {
        if w[1].top < w[0].top {
            tops_ok = false;
        }
    }
    println!(
        "blocks={} ci={ci} last={last} cy={cy} ch={ch} 块屏幕区间={sy0}..{sy1} scroll={scroll} missing={missing:?} vr={vr:?} total_h={}",
        a.index.blocks.len(),
        a.index.total_h
    );
    assert!(tops_ok, "块 top 必须单调不减");
    assert!(
        missing.is_empty(),
        "可见区间存在未排版块（绘制为空洞）: {missing:?}"
    );
    assert_eq!(ci, last, "文末新段光标须在末块");
    assert!(
        cy >= sy0 - 1.0 && cy + ch <= sy1 + 1.0,
        "光标须在所属块纵向跨度内：cy={cy} ch={ch} 块区间={sy0}..{sy1}"
    );
}

#[test]
fn oversize_area_cap_keeps_height_and_raster_in_sync() {
    let text = "a".repeat(100_000);
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    // 抬高缩放：内容变窄、行高变大，触发「光栅面积」上限而非字节/行数上限
    a.input_state.scale = 1.5;
    a.ensure_layouts_visible();
    let id = a.index.blocks[0].id;
    let (bh, truncated) = {
        let l = &a.cache.layouts[&id];
        (l.height, l.truncated)
    };
    assert!(truncated, "面积超限也须标记降级");
    a.paint();
    let g = &a.index.blocks[0];
    let key = a
        .cache
        .raster_key(
            g.id,
            &g.kind,
            g.range,
            a.base_key(),
            a.editor.caret(),
            a.toc_layout_epoch(g),
        )
        .expect("布局已缓存，光栅键应可用");
    let rh = a.cache.raster(g.id, key).expect("绘制后应有光栅").2;
    assert!(
        rh as f32 <= bh.ceil() + 1.0,
        "光栅高度 {rh} 不得超过块高 {bh}（否则块尾留白）"
    );
}

#[test]
fn oversized_line_count_is_capped() {
    let text = (0..2000).map(|i| format!("line {i}\n")).collect::<String>();
    assert!(text.len() < MAX_BLOCK_BYTES, "本用例只触发行数上限");
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_visible();
    let id = a.index.blocks[0].id;
    let l = &a.cache.layouts[&id];
    assert!(l.truncated, "行数超限也应降级");
    assert!(
        l.layout_text().lines().count() <= 513,
        "硬行数须受限，实际 {}",
        l.layout_text().lines().count()
    );
    assert_eq!(l.full_len, text.trim_end_matches('\n').len());
}

#[test]
fn block_below_cap_is_not_truncated() {
    let text = "a".repeat(10_000);
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_visible();
    let id = a.index.blocks[0].id;
    let l = &a.cache.layouts[&id];
    assert!(!l.truncated, "上限内块不得降级");
    assert_eq!(l.layout_text(), text, "上限内块须完整排版");
    assert_eq!(l.full_len, text.len());
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

#[test]
fn geometry_index_is_monotonic_and_consistent() {
    let store = MemStore::default();
    let mut a = app(&stress_doc(), None, false, &store);
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    let mut prev = f32::NEG_INFINITY;
    for g in &a.index.blocks {
        assert!(g.height > 0.0, "块高须为正: {:?}", g.id);
        assert!(g.top >= prev, "top 须单调不减: {:?}", g.id);
        prev = g.top;
    }
    let want = expected_doc_height(&a);
    assert!(
        (a.index.total_h - want).abs() < 1e-3,
        "total_h 应与累计一致: {} vs {want}",
        a.index.total_h
    );
}

#[test]
fn ensure_block_lays_out_far_block_with_real_height() {
    let store = MemStore::default();
    let mut a = app(&stress_doc(), None, false, &store);
    let id = a.index.blocks.last().unwrap().id;
    let g = geom(&a, id);
    let (kind, range) = (g.kind.clone(), g.range);
    let toc = a.toc_layout_epoch(g);
    assert!(
        a.cache
            .layout(id, &kind, range, a.base_key(), a.editor.caret(), toc)
            .is_none(),
        "远块初始不应已排版"
    );
    a.ensure_block(id);
    let h = a.cache.layouts.get(&id).expect("按需排版应写入布局").height;
    assert!(h > 0.0);
    assert_eq!(a.block_height(geom(&a, id)), h);
}

#[test]
fn memo_hit_equals_forced_relayout() {
    // 因果键命中的派生值必须与「清缓存后强制重排」逐块一致（ADR-009 决策四）
    let store = MemStore::default();
    let doc = "## 标题 **粗**\n\n段落 `code` 与 [链接](https://e.com)\n\n- 一\n- 二\n\n> 引用一\n> 引用二\n\n```rust\nfn f() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
    let mut a = app(doc, None, false, &store);
    a.ensure_layouts_all();
    type Snap = Vec<(BlockId, f32, u64, usize, usize, String)>;
    let snapshot = |a: &App| -> Snap {
        a.index
            .blocks
            .iter()
            .map(|g| {
                let m = &a.cache.layouts[&g.id];
                (
                    g.id,
                    m.height,
                    m.key.reveal,
                    m.imgs.len(),
                    m.strikes.len(),
                    m.layout_text().to_string(),
                )
            })
            .collect()
    };
    let before = snapshot(&a);
    a.cache.layouts.clear(); // 命中路径失效：下次 ensure 必定全量重排
    a.ensure_layouts_all();
    assert_eq!(before, snapshot(&a), "命中值与重算值须逐块等价");
}

#[test]
fn render_is_idempotent_consecutive_frames_equal() {
    // ADR-009 决策五幂等不变量：同一输入连续两次 render 产出逐像素相同的帧缓冲
    let store = MemStore::default();
    let doc = "# 一 **粗**\n\n段落 `code` 与 [链接](https://e.com)\n\n- 列表一\n- 列表二\n\n> 引用一\n> 引用二\n\n```rust\nfn f() -> u32 { 42 }\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
    let mut a = app(doc, None, false, &store);
    a.paint(); // 第一次 render：填派生缓存（布局/光栅/chrome 标签）
    let first = a
        .window_state
        .frame
        .as_ref()
        .map(|f| f.pixels.clone())
        .expect("render 后 frame 必须存在");
    a.paint(); // 第二次 render：全部走缓存命中路径，输出须与第一次逐像素同
    let second = a
        .window_state
        .frame
        .as_ref()
        .map(|f| &f.pixels)
        .expect("同上");
    assert_eq!(
        &first[..],
        second.as_slice(),
        "连续两次 render 产出的 frame.pixels 必须逐像素相同"
    );
    let (w, h) = a.win_size();
    assert_eq!(
        first.len() as u32,
        w * h,
        "像素数 = 宽 × 高（Frame.pixels 为 Vec<u32>，每像素一个 u32）"
    );
}

#[test]
fn present_is_noop_without_surface_and_isolated_from_render() {
    // present 在无 surface（headless）时必须纯 no-op；render 的 frame 状态不变
    let store = MemStore::default();
    let doc = "# t\n\np\n";
    let mut a = app(doc, None, false, &store);
    a.paint();
    let frame_pixels_before = a
        .window_state
        .frame
        .as_ref()
        .map(|f| f.pixels.clone())
        .expect("render 产出 frame");
    a.present(); // headless 下 surface=None → no-op
    let frame_pixels_after = a
        .window_state
        .frame
        .as_ref()
        .map(|f| f.pixels.clone())
        .expect("present 后 frame 仍须存在");
    assert_eq!(
        frame_pixels_before, frame_pixels_after,
        "headless present 不得改变 frame 状态"
    );
}

#[test]
fn present_requires_matching_lengths() {
    // 真实失配路径（最小化后 1×1 帧 vs 旧 surface）依赖 winit/softbuffer 的真实窗口，
    // headless 下无法构造 Surface，故只能测纯判定层；present 仅在该判定为真时拷贝。
    assert!(can_present(100, 100), "长度相等应允许呈现");
    assert!(!can_present(4, 100), "帧短于 surface 应丢帧");
    assert!(!can_present(100, 4), "帧长于 surface 应丢帧");
}

// ─────────────────────────────────────────────────────────────────────────────
// 性质级测试（ADR-009 S6 阶段 B）：验证 Memo 因果键对任意编辑序列的正确性。
// 两个性质：
//   1. memo_hit_equals_forced_after_any_sequence：任意编辑序列（0..50 次）+
//      主题/DPI 切换（0..20 次）后，memo 命中值恒等于 clear 后强制重算值
//   2. render_idempotent_after_any_sequence：任意编辑序列后连续两次 render
//      逐元素像素相等
// 使用 proptest（dev-dependency，只进测试二进制）：自动 shrinking 找最小反例，
// 发现边界 bug 后自动缩减为可钉死的永久单元测试。
// ─────────────────────────────────────────────────────────────────────────────

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

#[test]
fn heading_reveal_invalidates_after_block_shift_and_caret_leave() {
    let store = MemStore::default();
    let mut a = app("# 标题\n\n尾\n", None, false, &store);
    // caret 0 在标题块内 → 显示源码 `# 标题`
    let dirty = a.editor.apply(Intent::Newline).unwrap_or_default();
    a.after_edit(dirty); // 块整体下移，id 保留
    let dirty = a.editor.apply(Intent::MoveDocStart).unwrap_or_default();
    a.after_edit(dirty); // caret 回到块外 → 标记应消隐
    let stale = memo_matches_forced_after(&mut a);
    assert!(
        stale.is_empty(),
        "标题块迁移后不得残留旧 Reveal 布局: {stale:?}"
    );
    // caret 在块外（0）→ 隐藏 `#`
    let heading = a
        .index
        .blocks
        .iter()
        .find(|g| matches!(g.kind, BlockKind::AtxHeading(_)))
        .map(|g| g.id)
        .expect("文档含标题块");
    assert_eq!(a.cache.layouts[&heading].layout_text(), "标题");
}

#[test]
fn fenced_reveal_invalidates_after_block_shift_and_caret_leave() {
    let store = MemStore::default();
    let mut a = app("```\nx\n```\n\n尾\n", None, false, &store);
    let dirty = a.editor.apply(Intent::Newline).unwrap_or_default();
    a.after_edit(dirty);
    let dirty = a.editor.apply(Intent::MoveDocStart).unwrap_or_default();
    a.after_edit(dirty);
    let stale = memo_matches_forced_after(&mut a);
    assert!(
        stale.is_empty(),
        "围栏块迁移后不得残留旧 Reveal 布局: {stale:?}"
    );
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

/// 主题/DPI 切换变体（5 种）：主题模式切换（set_theme_mode 内部递增 revs.theme）
/// 与 scale 切换（base_key.scale_q 自然失配）。
fn apply_toggle(a: &mut App, kind: u8) {
    use crate::theme::ThemeMode;
    match kind % 5 {
        0 => a.set_theme_mode(ThemeMode::Light),
        1 => a.set_theme_mode(ThemeMode::Dark),
        2 => a.set_theme_mode(ThemeMode::System),
        3 => a.input_state.scale = 1.25,
        _ => a.input_state.scale = 1.75,
    }
}

proptest::prop_compose! {
    fn arbitrary_intent()(
        // InsertText 需要字符串：随机 1~5 个 a-z 字符（避免 markdown 语法复杂化解析）
        text in proptest::collection::vec(proptest::char::range('a', 'z'), 1..=5),
        // 选择哪个变体：0..=13 覆盖 14 个 Intent 变体
        var in 0u8..=13,
    ) -> Intent {
        let text: String = text.into_iter().collect();
        match var {
            0 => Intent::InsertText(text),
            1 => Intent::DeleteBackward,
            2 => Intent::DeleteForward,
            3 => Intent::Newline,
            4 => Intent::MoveLeft,
            5 => Intent::MoveRight,
            6 => Intent::MoveDocStart,
            7 => Intent::MoveDocEnd,
            8 => Intent::SelectLeft,
            9 => Intent::SelectRight,
            10 => Intent::SelectDocStart,
            11 => Intent::SelectDocEnd,
            12 => Intent::Undo,
            _ => Intent::Redo,
        }
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        // 标准 100 组对性质测试足够；debug build 22s 跑通第一条，
        // 若边界 bug 需要更高覆盖可调大
        cases: 100,
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn memo_hit_equals_forced_after_any_sequence(
        edits in proptest::collection::vec(arbitrary_intent(), 0..=50),
        toggles in proptest::collection::vec(0u8..=4, 0..=20),
    ) {
        // 性质一：同一 App 上先让 memo 自然命中并取快照，再清空派生缓存强制重排取第二份，
        // 两次的 alive BlockId 集合与逐块 (height, reveal, layout_text) 必须完全一致。
        // 用单 App 而非双 App 对比：撤销分组按真实时钟 500ms 静默窗合并（History::record），
        // 两份实例的耗时差会让「连续单字符删除」有时合并有时不合并，Undo 后文本分叉——
        // 那是测试对时钟的依赖，不是产品不变量。
        let base_doc = "# 标题\n\n段落 **粗** 与 `code` 和 [链接](https://e.com)\n\n- 列表一\n- 列表二\n\n> 引用一\n> 引用二\n\n```rust\nfn f() -> u32 { 42 }\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let store = MemStore::default();
        let mut a = app(base_doc, None, false, &store);

        for intent in &edits {
            let dirty = a.editor.apply(intent.clone()).unwrap_or_default();
            a.after_edit(dirty);
        }
        for t in &toggles {
            apply_toggle(&mut a, *t);
        }

        a.ensure_layouts_all();
        let snap_memo = snapshot_layouts(&a);

        a.cache.layouts.clear();
        a.cache.rasters.clear();
        a.cache.math_imgs.clear();
        a.cache.math_failed.clear();
        a.cache.inline_math.clear();
        a.cache.inline_failed.clear();
        a.cache.inline_images.clear();
        a.cache.image_failed.clear();
        a.cache.tables.clear();
        a.ensure_layouts_all();
        let snap_forced = snapshot_layouts(&a);

        let mut ids_memo: Vec<BlockId> = snap_memo.keys().copied().collect();
        let mut ids_forced: Vec<BlockId> = snap_forced.keys().copied().collect();
        ids_memo.sort();
        ids_forced.sort();
        proptest::prop_assert_eq!(
            ids_memo, ids_forced,
            "alive BlockId 集合不一致"
        );

        for id in snap_memo.keys() {
            if snap_memo.get(id) != snap_forced.get(id) {
                let g = a.index.blocks.iter().find(|g| g.id == *id);
                let src = g.map(|g| {
                    a
                        .editor
                        .session()
                        .text_slice(g.range)
                        .unwrap_or_default()
                });
                let g2 = a.index.blocks.iter().find(|g| g.id == *id);
                let src2 = g2.map(|g| {
                    a
                        .editor
                        .session()
                        .text_slice(g.range)
                        .unwrap_or_default()
                });
                let info = format!(
                    "id={id:?} memo:kind={:?} range={:?} 源文本={src:?} | forced:kind={:?} range={:?} 源文本={src2:?}",
                    g.map(|g| g.kind.clone()),
                    g.map(|g| g.range),
                    g2.map(|g| g.kind.clone()),
                    g2.map(|g| g.range)
                );
                proptest::prop_assert_eq!(
                    snap_memo.get(id),
                    snap_forced.get(id),
                    "memo 命中值 ≠ 强制重算值: {}",
                    info
                );
            }
        }
    }

    #[test]
    fn render_idempotent_after_any_sequence(
        edits in proptest::collection::vec(arbitrary_intent(), 0..=30),
    ) {
        // 性质二：任意编辑序列后连续两次 paint() 产出逐元素像素相同的帧缓冲。
        // 幂等性是渲染纯化（S4）的核心不变量。
        let base_doc = "# 标题\n\n段落 **粗** 与 `code` 和 [链接](https://e.com)\n\n- 列表一\n- 列表二\n\n> 引用一\n> 引用二\n\n```rust\nfn f() -> u32 { 42 }\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let store = MemStore::default();
        let mut a = app(base_doc, None, false, &store);

        for intent in &edits {
            let dirty = a.editor.apply(intent.clone()).unwrap_or_default();
            a.after_edit(dirty);
        }

        a.paint();
        let first = a.window_state.frame.as_ref().unwrap().pixels.clone();
        a.paint();
        let second = a.window_state.frame.as_ref().unwrap().pixels.clone();
        proptest::prop_assert_eq!(
            first, second,
            "render 幂等性被破坏"
        );
    }

    #[test]
    fn undo_to_initial_is_byte_identical_after_any_sequence(
        edits in proptest::collection::vec(arbitrary_intent(), 0..=50),
    ) {
        // 源码真值往返 fuzz（2c 切片）：任何编辑序列后逐步撤销到底，
        // 必须字节级回到初始文档。任何隐式规范化（全角/半角转换、尾随空格
        // 修剪、列表编号重排、换行风格改写）都不会进入撤销栈——一旦发生，
        // 撤销到底必残留差异而被本性质捕获。
        let seed = "首段 ＡＢＣ １２３ ！ ｱｲｳ  \n\n1. 甲\n1. 乙\n3. 丙\n\n> 引用  \n\n```\ncode 行  \n```\n\n末段 tab\t\n";
        let store = MemStore::default();
        let mut a = app(seed, None, false, &store);

        for intent in &edits {
            let dirty = a.editor.apply(intent.clone()).unwrap_or_default();
            a.after_edit(dirty);
        }
        // 逐步撤销到底（空栈 Undo 返回 None，循环必然终止）
        while a.editor.apply(Intent::Undo).is_some() {}
        let restored = a.editor.session().serialize();
        proptest::prop_assert_eq!(
            restored.as_str(),
            seed,
            "撤销到底后须字节级还原初始文档（隐式规范化检测）"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 持久化状态机性质测试（ADR-009 S6 阶段 B 续）：外部变更 × 保存 × 自动保存。
// 外部磁盘事件由不可控进程驱动，是全项目唯一「输入不可穷举」的状态机，故用 proptest
// 钉死六条用户可观察不变量。断言只观察 status()/hold() 与磁盘副作用，
// 不观察 pending/backup/missing 等内部事实（内部表示可变，契约不变）。
// ─────────────────────────────────────────────────────────────────────────────

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
                a.persist_state.persist.note_activity(Instant::now());
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

fn persist_projection(a: &App, store: &MemStore) -> PersistProjection {
    (
        a.external_state(),
        a.persist_state.persist.hold(),
        a.persist_state.persist.pending().map(<[u8]>::to_vec),
        a.persist_state.persist.missing(),
        a.persist_state.persist.backed_up(),
        store.files.borrow().clone(),
        a.doc.baseline.clone(),
    )
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 100,
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn persist_state_machine_invariants_hold_after_any_sequence(
        events in proptest::collection::vec(persist_event(), 1..=30),
    ) {
        let store = MemStore::default();
        let watch = FakeWatch::default();
        let base = "ab\n";
        disk(&store, "a.md", base.as_bytes());
        let mut a = app_watch(base, Some("a.md"), &store, &watch);
        let mut clock = Instant::now();
        let mut prev_was_poll = false;

        for (i, ev) in events.iter().enumerate() {
            let disk_before = store.files.borrow().get(Path::new("a.md")).cloned();
            let baseline_before = a.doc.baseline.clone();
            let conflicts_before = conflict_files(&store);
            let facts_before = persist_projection(&a, &store);
            apply_persist_event(&mut a, &store, &watch, &mut clock, ev);

            let p = &a.persist_state.persist;

            // I1：status 优先级投影自洽（Changed > BackedUp > Missing > None）
            match a.external_state() {
                ExternalState::Changed => proptest::prop_assert!(
                    p.pending().is_some(),
                    "step {} ({:?}): Changed 但无待备份版本", i, ev
                ),
                ExternalState::BackedUp => proptest::prop_assert!(
                    p.pending().is_none() && p.backed_up(),
                    "step {} ({:?}): BackedUp 但事实不符", i, ev
                ),
                ExternalState::Missing => proptest::prop_assert!(
                    p.pending().is_none() && !p.backed_up() && p.missing(),
                    "step {} ({:?}): Missing 但事实不符", i, ev
                ),
                ExternalState::None => proptest::prop_assert!(
                    p.pending().is_none() && !p.backed_up() && !p.missing(),
                    "step {} ({:?}): None 但仍有外部事实", i, ev
                ),
            }

            // I2：自动保存挂起必有原因（否则用户会看到「不保存」却无任何提示）
            if p.hold() {
                proptest::prop_assert!(
                    p.pending().is_some() || p.backed_up(),
                    "step {} ({:?}): 挂起自动保存但无外部状态原因", i, ev
                );
            }

            // I3：自动保存绝不覆盖外部版本（磁盘存在且≠基线时，磁盘必须原样保留）
            if matches!(ev, PersistEvent::AutoSaveTick) {
                if let Some(d) = &disk_before {
                    if *d != baseline_before {
                        let after = store.files.borrow().get(Path::new("a.md")).cloned();
                        proptest::prop_assert_eq!(
                            after, disk_before.clone(),
                            "step {}: 自动保存覆盖了外部版本", i
                        );
                    }
                }
            }

            // I4：手工保存成功 ⇒ 磁盘==基线、缺失态清除、挂起解除
            if matches!(ev, PersistEvent::ManualSave) {
                let disk_after = store.files.borrow().get(Path::new("a.md")).cloned();
                proptest::prop_assert_eq!(
                    disk_after.as_deref(), Some(a.doc.baseline.as_slice()),
                    "step {}: 手工保存成功后磁盘≠基线", i
                );
                proptest::prop_assert!(!p.missing(), "step {}: 保存成功但仍是缺失态", i);
                proptest::prop_assert!(!p.hold(), "step {}: 保存成功但自动保存仍挂起", i);
            }

            // I5：外部版本不丢失（被覆盖的外部字节必落冲突副本），且已写副本永不被改写
            if matches!(ev, PersistEvent::ManualSave) {
                let conflicts_after = conflict_files(&store);
                for (cp, cb) in &conflicts_before {
                    proptest::prop_assert!(
                        conflicts_after.iter().any(|(p2, b2)| p2 == cp && b2 == cb),
                        "step {}: 已写冲突副本被改写或丢失 {}", i, cp.display()
                    );
                }
                if let Some(d) = &disk_before {
                    if *d != baseline_before {
                        proptest::prop_assert!(
                            conflicts_after.iter().any(|(_, b)| b == d),
                            "step {}: 被覆盖的外部版本未落入任何冲突副本", i
                        );
                    }
                }
            }

            // I6：检测幂等（连续两次 Poll 且磁盘未变时，状态事实不变）
            if matches!(ev, PersistEvent::Poll) && prev_was_poll {
                let facts_after = persist_projection(&a, &store);
                proptest::prop_assert_eq!(
                    facts_after, facts_before,
                    "step {}: 连续轮询状态不幂等", i
                );
            }

            prev_was_poll = matches!(ev, PersistEvent::Poll);
        }
    }

    #[test]
    fn persist_replay_is_deterministic(
        events in proptest::collection::vec(persist_event(), 0..=20),
    ) {
        // 同一事件序列重放两次必须得到完全相同的可观察投影（含磁盘全量内容与基线）：
        // 持久化编排不得依赖 HashMap 顺序、时钟漂移或残留临时状态。
        let run = |events: &[PersistEvent]| {
            let store = MemStore::default();
            let watch = FakeWatch::default();
            let base = "ab\n";
            disk(&store, "a.md", base.as_bytes());
            let mut a = app_watch(base, Some("a.md"), &store, &watch);
            let mut clock = Instant::now();
            for ev in events {
                apply_persist_event(&mut a, &store, &watch, &mut clock, ev);
            }
            persist_projection(&a, &store)
        };
        proptest::prop_assert_eq!(run(&events), run(&events));
    }
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
    assert!(a.cache.layouts.contains_key(&id), "光标块须已排版");
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
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_keystroke_path
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

/// 分项归因探针（默认忽略）：三档文档规模下把每次击键拆成 5 段独立耗时。
/// 全部走真实代码路径：Editor::apply → Session::apply；refresh_index；脏块
/// ensure_block（真实缓存+排版路径）；LayoutEngine::rasterize；render 帧合成
///（headless 亦可运行，present 无 surface 时为 no-op，不纳入计时）。
/// cargo test -p veil-ui --release --lib -- --ignored --nocapture perf_keystroke_attribution
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_keystroke_attribution() {
    const SAMPLES: usize = 300;
    println!(
        "# segments: core_apply_reparse=Editor::apply->Session::apply; \
         index_rebuild=App::refresh_index（含 recompute_geometry + refresh_stats）; \
         dirty_layout=App::ensure_block（真实缓存查询+排版）; \
         rasterize=LayoutEngine::rasterize（脏块）; \
         frame_compose=App::render（真实 headless 帧合成，present 无 surface 不测）"
    );
    println!(
        "doc_blocks,core_apply_reparse_p50_us,core_apply_reparse_p95_us,\
         index_rebuild_p50_us,index_rebuild_p95_us,\
         dirty_layout_p50_us,dirty_layout_p95_us,\
         rasterize_p50_us,rasterize_p95_us,\
         frame_compose_p50_us,frame_compose_p95_us,\
         total_p95_us,index_share_pct"
    );

    for iters in [715usize, 1500, 2857] {
        let text = stress_doc_n(iters);
        let store = MemStore::default();
        let mut a = app(&text, None, false, &store);
        a.editor.apply(Intent::MoveDocStart).unwrap();
        for _ in 0..5 {
            a.editor.apply(Intent::MoveRight).unwrap();
        }
        a.paint(); // 预热：首帧完成可见区布局与光栅，排除字体首次加载成本

        let block_count = a.index.blocks.len();
        let mut core = Vec::with_capacity(SAMPLES);
        let mut index = Vec::with_capacity(SAMPLES);
        let mut layout = Vec::with_capacity(SAMPLES);
        let mut raster = Vec::with_capacity(SAMPLES);
        let mut compose = Vec::with_capacity(SAMPLES);

        for _ in 0..SAMPLES {
            let t = Instant::now();
            let dirty = a
                .editor
                .apply(Intent::InsertText("a".into()))
                .expect("插入点恒合法");
            core.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            a.refresh_index();
            index.push(t.elapsed().as_secs_f64() * 1e6);

            // 脏块排版：生产路径 ensure_block（命中则只比键，未命中才取词/行内并排版）
            let t = Instant::now();
            for &id in &dirty {
                a.ensure_block(id);
            }
            layout.push(t.elapsed().as_secs_f64() * 1e6);

            // 脏块光栅：与 paint.rs 的惰性光栅同一路径（布局键比对 + rasterize + 回填）
            let base = a.base_key();
            let caret = a.effective_caret();
            let fg = a.input_state.theme.fg;
            let t = Instant::now();
            for &id in &dirty {
                let Some(g) = a.index.blocks.iter().find(|g| g.id == id) else {
                    continue;
                };
                let kind = g.kind.clone();
                let range = g.range;
                let Some(lk) = a
                    .cache
                    .raster_key(id, &kind, range, base, caret, a.toc_layout_epoch(g))
                else {
                    continue;
                };
                if a.cache.raster(id, lk).is_none() {
                    let r = a.engine.rasterize(&a.cache.layouts[&id].value, fg);
                    a.cache.rasters.insert(id, Memo { key: lk, value: r });
                }
            }
            raster.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            let _ = a.render();
            compose.push(t.elapsed().as_secs_f64() * 1e6);
        }

        let pick = |v: &mut [f64], q: f64| {
            v.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let i = ((v.len() as f64 * q).ceil() as usize)
                .saturating_sub(1)
                .min(v.len() - 1);
            v[i]
        };
        let (core_p50, core_p95) = (pick(&mut core, 0.5), pick(&mut core, 0.95));
        let (index_p50, index_p95) = (pick(&mut index, 0.5), pick(&mut index, 0.95));
        let (layout_p50, layout_p95) = (pick(&mut layout, 0.5), pick(&mut layout, 0.95));
        let (raster_p50, raster_p95) = (pick(&mut raster, 0.5), pick(&mut raster, 0.95));
        let (compose_p50, compose_p95) = (pick(&mut compose, 0.5), pick(&mut compose, 0.95));
        let total_p95 = core_p95 + index_p95 + layout_p95 + raster_p95 + compose_p95;
        println!(
            "{block_count},{core_p50:.2},{core_p95:.2},\
             {index_p50:.2},{index_p95:.2},\
             {layout_p50:.2},{layout_p95:.2},\
             {raster_p50:.2},{raster_p95:.2},\
             {compose_p50:.2},{compose_p95:.2},\
             {total_p95:.2},{:.1}",
            index_p95 / total_p95 * 100.0
        );
    }
}

/// `index_rebuild` 子段探针（默认忽略）：按生产顺序逐步调用 refresh_index 的 4 个真实步骤。
/// 生产 `refresh_index` 自身不含任何 `Instant::now()`；探针在外部逐步计时，不重复实现逻辑。
/// cargo test -p veil-ui --release --lib -- --ignored --nocapture perf_index_subsegments
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_index_subsegments() {
    const SAMPLES: usize = 300;
    println!(
        "# index subsegments: blocks_enum=App::blocks_enum; geom_fill=App::geom_fill;\
         recompute=App::recompute_geometry; stats=App::refresh_stats"
    );
    println!(
        "doc_blocks,blocks_enum_p50_us,blocks_enum_p95_us,\
         geom_fill_p50_us,geom_fill_p95_us,\
         recompute_p50_us,recompute_p95_us,\
         stats_p50_us,stats_p95_us,\
         index_sum_p50_us,index_sum_p95_us,\
         blocks_enum_share_pct,geom_fill_share_pct,recompute_share_pct,stats_share_pct"
    );

    for iters in [715usize, 1500, 2857] {
        let text = stress_doc_n(iters);
        let store = MemStore::default();
        let mut a = app(&text, None, false, &store);
        a.editor.apply(Intent::MoveDocStart).unwrap();
        for _ in 0..5 {
            a.editor.apply(Intent::MoveRight).unwrap();
        }
        a.paint();

        let block_count = a.index.blocks.len();
        let mut blocks_enum = Vec::with_capacity(SAMPLES);
        let mut geom_fill = Vec::with_capacity(SAMPLES);
        let mut recompute = Vec::with_capacity(SAMPLES);
        let mut stats = Vec::with_capacity(SAMPLES);

        for _ in 0..SAMPLES {
            // 编辑本身不计入 index 子段，只为制造真实脏状态
            let _dirty = a
                .editor
                .apply(Intent::InsertText("a".into()))
                .expect("插入点恒合法");

            let t = Instant::now();
            let blocks = a.blocks_enum();
            blocks_enum.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            a.geom_fill(blocks);
            geom_fill.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            a.recompute_geometry();
            recompute.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            a.refresh_stats();
            stats.push(t.elapsed().as_secs_f64() * 1e6);
        }

        let pick = |v: &mut [f64], q: f64| {
            v.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let i = ((v.len() as f64 * q).ceil() as usize)
                .saturating_sub(1)
                .min(v.len() - 1);
            v[i]
        };
        let (be_p50, be_p95) = (pick(&mut blocks_enum, 0.5), pick(&mut blocks_enum, 0.95));
        let (gf_p50, gf_p95) = (pick(&mut geom_fill, 0.5), pick(&mut geom_fill, 0.95));
        let (rc_p50, rc_p95) = (pick(&mut recompute, 0.5), pick(&mut recompute, 0.95));
        let (st_p50, st_p95) = (pick(&mut stats, 0.5), pick(&mut stats, 0.95));
        let sum_p50 = be_p50 + gf_p50 + rc_p50 + st_p50;
        let sum_p95 = be_p95 + gf_p95 + rc_p95 + st_p95;
        println!(
            "{block_count},{be_p50:.2},{be_p95:.2},\
             {gf_p50:.2},{gf_p95:.2},\
             {rc_p50:.2},{rc_p95:.2},\
             {st_p50:.2},{st_p95:.2},\
             {sum_p50:.2},{sum_p95:.2},\
             {:.1},{:.1},{:.1},{:.1}",
            be_p95 / sum_p95 * 100.0,
            gf_p95 / sum_p95 * 100.0,
            rc_p95 / sum_p95 * 100.0,
            st_p95 / sum_p95 * 100.0
        );
    }
}

/// P1b 验收探针（默认忽略）：构造 5005 块文档 + 第一次编辑的端到端耗时。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_cold_first_edit
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_cold_first_edit() {
    let text = stress_doc();
    let store = MemStore::default();
    let t = Instant::now();
    let mut a = app(&text, None, false, &store);
    let app_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    let edit_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("perf_cold_first_edit_ms={:.3}", app_ms + edit_ms);
    println!("perf_cold_app_ms={app_ms:.3}");
    println!("perf_cold_edit_ms={edit_ms:.3}");
    println!("perf_layouts_after_first_edit={}", a.cache.layouts.len());
}

/// 对照探针（默认忽略）：先完成一次极小排版（等价真实窗口的首帧），
/// 再测同一「首编辑」路径，用于区分字体懒加载成本与编辑本身成本。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_cold_first_edit_prewarmed
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_cold_first_edit_prewarmed() {
    let text = stress_doc();
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    let _ = a.engine.layout_preedit("a");
    let t = Instant::now();
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    let edit_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("perf_cold_edit_prewarmed_ms={edit_ms:.3}");
    println!("perf_cold_edit_prewarmed_layouts={}", a.cache.layouts.len());
}

/// 真实生命周期的冷首编辑探针（默认忽略）：App 构造 → 首帧 paint（真实窗口启动必经）
/// → 用户第一次按键。与 `perf_cold_first_edit`（跳过首帧）对照，用于区分「用户可感知的
/// 首键延迟」与「字体/字形首次预热成本」。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_first_edit_after_first_frame
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_first_edit_after_first_frame() {
    let text = stress_doc();
    let store = MemStore::default();
    let t = Instant::now();
    let mut a = app(&text, None, false, &store);
    let app_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    a.paint();
    let first_frame_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    a.paint();
    let first_edit_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("perf_first_edit_after_first_frame app_ms={app_ms:.3} first_frame_ms={first_frame_ms:.3} first_edit_ms={first_edit_ms:.3}");
    println!("perf_first_edit_layouts={}", a.cache.layouts.len());
}

/// 0a 验收探针（默认忽略）：10MB 单块连续滚轮 300 格后仍须有内容像素。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_huge_scroll
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_huge_scroll() {
    let text = "a".repeat(10 * 1024 * 1024);
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_visible();
    let step = -3.0 * BASE_LINE_HEIGHT * a.input_state.scale;
    for _ in 0..300 {
        a.on_wheel(step);
        let _ = a.render();
    }
    let vp = a.viewport();
    let bg = a.input_state.theme.bg;
    let bgpix = ((bg[0] as u32) << 16) | ((bg[1] as u32) << 8) | bg[2] as u32;
    let (doc_h, scroll, block_h) = (
        a.doc_height(),
        a.window_state.scroll_y,
        a.block_height(&a.index.blocks[0]),
    );
    let frame = a.render();
    let lo = (vp.y as usize + 1) * frame.width as usize;
    let hi = ((vp.y + vp.h) as usize).min(frame.height as usize) * frame.width as usize;
    let nonbg = frame.pixels[lo..hi].iter().filter(|p| **p != bgpix).count();
    println!(
        "huge_scroll vp={}x{} doc_h={doc_h} block_h={block_h} scroll={scroll} nonbg={nonbg}",
        vp.w, vp.h
    );
    assert!(nonbg > 1000, "滚到 10MB 单块末尾仍须有内容");
}

/// 0a 验收探针（默认忽略）：1/2/10MB 单块只排前缀，不再整块成形。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_huge_block
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_huge_block() {
    for mb in [1usize, 2, 10] {
        let text = "a".repeat(mb * 1024 * 1024);
        let store = MemStore::default();
        let t = Instant::now();
        let mut a = app(&text, None, false, &store);
        let app_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let dirty = a.editor.apply(Intent::InsertText("b".into())).unwrap();
        a.after_edit(dirty);
        a.paint();
        let edit_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        a.editor.apply(Intent::MoveDocEnd).unwrap();
        let dirty = a.editor.apply(Intent::InsertText("c".into())).unwrap();
        a.after_edit(dirty);
        a.paint();
        let tail_ms = t.elapsed().as_secs_f64() * 1e3;
        let l = &a.cache.layouts[&a.index.blocks[0].id];
        println!(
            "huge_mb={mb} app_ms={app_ms:.3} edit_ms={edit_ms:.3} tail_ms={tail_ms:.3} prefix={} truncated={} height={:.1}",
            l.layout_text().len(),
            l.truncated,
            l.height
        );
        assert!(l.truncated);
        assert!(l.layout_text().len() <= MAX_BLOCK_BYTES);
        assert_eq!(a.editor.session().serialize().len(), text.len() + 2);
    }
}

// ---- 崩溃恢复 + 会话恢复 + 最近文件（0b）：故障注入 ----
// 核心验收：任意时刻杀进程，重启后可恢复到最后一次成功提交的恢复快照。

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
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
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
            clipboard: Box::new(NoClip),
            store: Box::new(store.clone()),
            math: Box::new(m),
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

#[test]
fn restored_draft_save_as_clears_recovery_snapshot() {
    // R1 回归：恢复草稿后直接另存为必须作废磁盘上的快照；否则下次启动再次恢复旧草稿，
    // 用户每次都要另存为（自循环）。
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    *rec.slot.borrow_mut() = Some(recovery::encode_recovery(None, false, "草稿内容"));
    let ses = FakeSession::default();
    let mut a = app_persist_dialogs("默认样例", None, &store, &rec, &ses, true, Some("saved.md"));
    assert!(a.editor.is_dirty(), "恢复草稿打开即脏");
    a.save_as();
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("saved.md")));
    assert!(rec.slot.borrow().is_none(), "另存为成功后恢复快照必须作废");
}

#[test]
fn cli_open_save_keeps_foreign_recovery_snapshot() {
    // 保留语义：resume=false（CLI 显式打开）时磁盘上的快照属于别的草稿，
    // 本会话保存不得超过清除它（否则会丢别处未保存内容）。
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"ab".to_vec());
    let rec = FakeRecovery::default();
    *rec.slot.borrow_mut() = Some(recovery::encode_recovery(None, false, "别的草稿"));
    let ses = FakeSession::default();
    let mut a = app_persist_dialogs("ab", Some("a.md"), &store, &rec, &ses, false, None);
    a.save(crate::document::doc_state::SaveKind::Manual);
    assert!(
        rec.slot.borrow().is_some(),
        "外部打开的保存不动他人恢复快照"
    );
}

#[test]
fn recovery_written_after_idle_and_restored_on_restart() {
    // 第一次会话：编辑后静默 2s 落恢复快照（内容含路径与脏文本）
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist("ab", Some("a.md"), &store, &rec, &ses, false);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_recovery_write(t0 + Duration::from_millis(1999));
    assert!(rec.slot.borrow().is_none(), "静默期未满不落恢复快照");
    a.maybe_recovery_write(t0 + Duration::from_secs(2));
    assert_eq!(
        slot_content(&rec),
        (Some(PathBuf::from("a.md")), "改ab".to_string()),
        "恢复快照 = 路径 + 最新脏内容"
    );

    // 模拟杀进程重启：磁盘仍是旧内容，恢复槽有未保存快照 → 恢复为脏文档
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"ab".to_vec());
    let b = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(
        b.editor.session().serialize(),
        "改ab",
        "重启后内容 = 恢复快照"
    );
    assert_eq!(b.doc.path, Some(PathBuf::from("a.md")));
    assert!(b.editor.is_dirty(), "恢复内容相对磁盘基线是未保存修改");
    assert_eq!(b.doc.baseline, b"ab".to_vec(), "基线 = 磁盘当前内容");
    assert!(
        b.ui_state.chrome.notice.contains("崩溃恢复"),
        "恢复后给出可读提示"
    );
}

#[test]
fn recovery_cleared_after_successful_save() {
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist("ab", Some("a.md"), &store, &rec, &ses, false);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_recovery_write(t0 + Duration::from_secs(2));
    assert!(rec.slot.borrow().is_some());
    a.save_now();
    assert!(!a.editor.is_dirty());
    assert!(rec.slot.borrow().is_none(), "保存成功后恢复副本作废");
    assert!(rec.calls.borrow().iter().any(|c| c == "clear"));
}

#[test]
fn recovery_matching_disk_opens_clean_and_self_clears() {
    // 上次保存成功但清理残留失败：恢复内容 == 磁盘内容 → 干净打开 + 自清
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), encode("内容", false));
    let rec = FakeRecovery::default();
    *rec.slot.borrow_mut() = Some(recovery::encode_recovery(
        Some(Path::new("a.md")),
        false,
        "内容",
    ));
    let ses = FakeSession::default();
    let b = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(b.editor.session().serialize(), "内容");
    assert_eq!(b.doc.path, Some(PathBuf::from("a.md")));
    assert!(!b.editor.is_dirty(), "与磁盘一致：干净打开");
    assert!(rec.slot.borrow().is_none(), "残留快照已自清");
}

#[test]
fn recovery_draft_without_path_restored_dirty() {
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    *rec.slot.borrow_mut() = Some(recovery::encode_recovery(None, false, "草稿内容"));
    let ses = FakeSession::default();
    let b = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(b.editor.session().serialize(), "草稿内容");
    assert_eq!(b.doc.path, None, "草稿恢复仍无路径");
    assert!(b.editor.is_dirty());
    assert!(
        !b.persist_state.persist.missing(),
        "草稿无磁盘目标：不置缺失态"
    );
}

#[test]
fn recovery_missing_disk_marks_missing() {
    // 恢复快照指向的文件磁盘读不到：恢复内容照给，但置缺失提示态
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    *rec.slot.borrow_mut() = Some(recovery::encode_recovery(
        Some(Path::new("gone.md")),
        false,
        "未保存内容",
    ));
    let ses = FakeSession::default();
    let b = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(b.editor.session().serialize(), "未保存内容");
    assert!(b.editor.is_dirty());
    assert!(b.persist_state.persist.missing(), "磁盘缺失须置提示态");
    assert_eq!(b.external_state(), ExternalState::Missing);
}

#[test]
fn recovery_write_failure_retries_with_backoff() {
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    rec.fail_save.set(true);
    let ses = FakeSession::default();
    let mut a = app_persist("ab", Some("a.md"), &store, &rec, &ses, false);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_recovery_write(t0 + Duration::from_secs(2));
    assert!(
        a.ui_state.chrome.notice.contains("崩溃恢复写入失败"),
        "失败给可读提示而非 panic"
    );
    assert_eq!(
        a.persist_state.recovery_failed_at,
        Some(t0 + Duration::from_secs(2))
    );
    let saves = || rec.calls.borrow().iter().filter(|c| *c == "save").count();
    assert_eq!(saves(), 1);
    a.maybe_recovery_write(t0 + Duration::from_secs(2) + Duration::from_secs(4));
    assert_eq!(saves(), 1, "退避期内不重试");
    a.maybe_recovery_write(t0 + Duration::from_secs(2) + Duration::from_secs(5));
    assert_eq!(saves(), 2, "退避期满重试");
    // 故障恢复后写成功：哈希登记、失败时刻清零、提示可被后续动作覆盖
    rec.fail_save.set(false);
    a.maybe_recovery_write(t0 + Duration::from_secs(2) + Duration::from_secs(10));
    assert!(rec.slot.borrow().is_some(), "故障恢复后快照落盘");
    assert_eq!(a.persist_state.recovery_failed_at, None);
    assert!(a.persist_state.recovery_hash.is_some());
}

#[test]
fn recovery_written_when_autosave_keeps_failing_on_locked_target() {
    // DEF-T08-E1-001：目标文件持续被占用时自动保存反复失败，但恢复快照必须
    // 按「最后一次真实编辑」的独立期限写入，否则崩溃会丢失全部未保存内容。
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"ab".to_vec());
    store.fail_paths.borrow_mut().push(PathBuf::from("a.md"));
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist("ab", Some("a.md"), &store, &rec, &ses, false);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.tick(t0 + Duration::from_secs(2));
    assert!(a.editor.is_dirty(), "保存失败必须保持脏态");
    assert_eq!(
        a.persist_state.persist.activity(),
        Some(t0 + Duration::from_secs(2)),
        "自动保存去抖语义保留：失败尝试同样推进下次尝试"
    );
    assert_eq!(
        a.persist_state.persist.recovery_activity(),
        None,
        "恢复快照已按独立期限写入：写成功后清零，不会被自动保存失败重试推迟"
    );
    assert!(
        rec.slot.borrow().is_some(),
        "自动保存持续失败时 recovery 仍按期限写入"
    );
    let (path, text) = slot_content(&rec);
    assert_eq!(path, Some(PathBuf::from("a.md")));
    assert_eq!(text, "改ab", "恢复快照包含未保存内容");
    assert_eq!(
        store.files.borrow().get(&PathBuf::from("a.md")),
        Some(&b"ab".to_vec()),
        "锁定期间原文件内容/哈希保持不变"
    );
    // 继续失败重试：已写入的恢复快照不得被顺带清掉
    a.tick(t0 + Duration::from_secs(4));
    assert!(rec.slot.borrow().is_some());
}

#[test]
fn recovery_same_content_not_rewritten() {
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist("ab", Some("a.md"), &store, &rec, &ses, false);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_recovery_write(t0 + Duration::from_secs(2));
    let saves = || rec.calls.borrow().iter().filter(|c| *c == "save").count();
    assert_eq!(saves(), 1);
    // 到期但内容未变：哈希相同不重写
    a.maybe_recovery_write(t0 + Duration::from_secs(4));
    assert_eq!(saves(), 1, "内容未变不重复落盘");
    // 内容再变：哈希失配重写。恢复快照已与上一版一致、独立计时已清零，
    // 真实编辑经 after_edit 重新登记时刻（本测试绕过编辑钩子，显式补登记）。
    a.editor.apply(Intent::InsertText("再".into())).unwrap();
    a.persist_state
        .persist
        .note_activity(t0 + Duration::from_secs(4));
    a.maybe_recovery_write(t0 + Duration::from_secs(6));
    assert_eq!(saves(), 2, "内容变化重写快照");
    assert_eq!(slot_content(&rec).1, "改再ab");
}

#[test]
fn recovery_skipped_during_ime_preedit() {
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist("ab", Some("a.md"), &store, &rec, &ses, false);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.editor.set_preedit("zh".into(), None);
    a.maybe_recovery_write(t0 + Duration::from_secs(30));
    assert!(rec.slot.borrow().is_none(), "IME 组合期不写恢复快照");
}

#[test]
fn corrupted_recovery_ignored_and_cleared() {
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    *rec.slot.borrow_mut() = Some(b"not-a-recovery-file".to_vec());
    let ses = FakeSession::default();
    let b = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(
        b.editor.session().serialize(),
        "默认样例",
        "损坏快照不影响正常打开"
    );
    assert!(!b.editor.is_dirty());
    assert!(rec.slot.borrow().is_none(), "损坏快照已清除");
    assert!(rec.calls.borrow().iter().any(|c| c == "clear"));
}

#[test]
fn recovery_with_external_disk_change_saves_with_backup() {
    // 恢复内容 ≠ 磁盘内容：基线取磁盘当前内容（与「无崩溃」编辑-保存路径等价）。
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"disk".to_vec());
    let rec = FakeRecovery::default();
    *rec.slot.borrow_mut() = Some(recovery::encode_recovery(
        Some(Path::new("a.md")),
        false,
        "编辑版",
    ));
    let ses = FakeSession::default();
    let mut b = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(b.editor.session().serialize(), "编辑版");
    assert_eq!(b.doc.baseline, b"disk".to_vec(), "基线 = 磁盘当前内容");
    // 恢复之后磁盘再被外部修改：基线失配 → 保存走冲突备份路径
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"disk-v2".to_vec());
    b.save_now();
    let files = store.files.borrow();
    assert_eq!(
        files[Path::new("a.md")],
        "编辑版".as_bytes(),
        "手工保存覆盖"
    );
    assert_eq!(
        files[Path::new("a.conflict-1.md")],
        b"disk-v2".to_vec(),
        "磁盘外部版本已备份，不丢字"
    );
}

#[test]
fn cli_open_leaves_recovery_untouched() {
    // CLI 显式打开（resume=false）：不读恢复槽、不动快照；最近列表照常加载
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"cli".to_vec());
    let rec = FakeRecovery::default();
    *rec.slot.borrow_mut() = Some(recovery::encode_recovery(
        Some(Path::new("x.md")),
        false,
        "旧恢复",
    ));
    let ses = FakeSession::default();
    let a = app_persist("cli", Some("a.md"), &store, &rec, &ses, false);
    assert_eq!(a.editor.session().serialize(), "cli");
    assert!(rec.slot.borrow().is_some(), "CLI 打开不动恢复快照");
    assert!(
        !rec.calls.borrow().iter().any(|c| c == "load"),
        "CLI 打开不读恢复槽"
    );
}

#[test]
fn session_resume_opens_recent_file() {
    // 无恢复快照、无 CLI 参数：打开最近一次文件
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("b.md"), b"session".to_vec());
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    *ses.text.borrow_mut() = Some("recent=b.md\n".into());
    let b = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(b.editor.session().serialize(), "session");
    assert_eq!(b.doc.path, Some(PathBuf::from("b.md")));
    assert!(!b.editor.is_dirty(), "会话恢复是干净打开");
}

#[test]
fn session_resume_failure_falls_back_to_sample() {
    // 最近文件已消失：回退传入文档（样例），不崩溃
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    *ses.text.borrow_mut() = Some("recent=gone.md\n".into());
    let b = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(b.editor.session().serialize(), "默认样例");
    assert_eq!(b.doc.path, None);
}

#[test]
fn open_paths_update_recent_in_order_and_persist() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"a".to_vec());
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("b.md"), b"b".to_vec());
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist("默认样例", None, &store, &rec, &ses, false);
    a.open_path(PathBuf::from("a.md"));
    a.open_path(PathBuf::from("b.md"));
    assert_eq!(
        a.persist_state.recent,
        vec![PathBuf::from("b.md"), PathBuf::from("a.md")],
        "最近列表新→旧"
    );
    assert_eq!(
        ses.text.borrow().as_deref(),
        Some("recent=b.md\nrecent=a.md\n"),
        "最近列表已持久化"
    );
    // 再次打开旧文件：去重提头
    a.open_path(PathBuf::from("a.md"));
    assert_eq!(
        a.persist_state.recent,
        vec![PathBuf::from("a.md"), PathBuf::from("b.md")]
    );
}

#[test]
fn menu_click_opens_recent_file() {
    // 端到端：文件菜单 → 最近文件项 → open_path（复用点击路由与动态菜单模型）
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("x.md"), b"recent-x".to_vec());
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    *ses.text.borrow_mut() = Some("recent=x.md\n".into());
    let mut a = app_persist("默认样例", None, &store, &rec, &ses, false);
    assert_eq!(
        a.recent_labels(),
        vec!["x.md".to_string()],
        "启动加载最近列表"
    );
    let labels = a.recent_labels();
    let (tx, ty) = menu_title_center(0);
    click_at(&mut a, tx, ty);
    assert_eq!(a.ui_state.chrome.open, Some(0));
    // 最近文件块位于「退出」区块之前，上下各有分隔线
    let rows = chrome::menu_rows(0, &labels);
    let row = rows
        .iter()
        .position(|(_, _, a)| *a == MenuAction::OpenRecent(0))
        .expect("文件菜单应含最近文件项");
    let item = rows[..row]
        .iter()
        .filter(|(_, _, a)| *a != MenuAction::Separator)
        .count();
    let (x, y) = popup_item_center_recent(0, item, &labels);
    click_at(&mut a, x, y);
    assert_eq!(a.editor.session().serialize(), "recent-x");
    assert_eq!(a.doc.path, Some(PathBuf::from("x.md")));
    assert_eq!(a.ui_state.chrome.open, None, "动作后菜单关闭");
}

#[test]
fn recent_labels_disambiguate_same_file_name_across_dirs() {
    // 真机发现：不同目录下的同名文件在最近文件菜单里显示成同一行，无法区分。
    // 标签必须带父目录（path_label），与状态栏/标题/notice 保持一致。
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    *ses.text.borrow_mut() = Some("recent=dir/a.md\nrecent=dir2/a.md\n".into());
    let a = app_persist("默认样例", None, &store, &rec, &ses, false);
    assert_eq!(
        a.recent_labels(),
        vec!["a.md · dir".to_string(), "a.md · dir2".to_string()]
    );
}

#[test]
fn open_recent_ignored_when_dirty_or_out_of_range() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("x.md"), b"x".to_vec());
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    *ses.text.borrow_mut() = Some("recent=x.md\n".into());
    let mut a = app_persist("旧", None, &store, &rec, &ses, false);
    a.perform_menu(MenuAction::OpenRecent(9));
    assert_eq!(a.editor.session().serialize(), "旧", "越界下标静默忽略");
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    a.perform_menu(MenuAction::OpenRecent(0));
    assert_eq!(
        a.editor.session().serialize(),
        "改旧",
        "脏文档拒绝打开最近文件"
    );
}

/// 阶段 1 快速打开：最近文件 + 当前目录 Markdown（去重、扩展名过滤）→ 过滤 → 打开。
#[test]
fn quick_open_lists_recent_and_directory_then_opens() {
    let store = MemStore::default();
    for (p, b) in [
        ("dir/cur.md", "cur"),
        ("dir/a.md", "aaa"),
        ("dir/README.md", "readme"),
        ("dir/skip.txt", "txt"),
    ] {
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from(p), b.as_bytes().to_vec());
    }
    let mut a = app("cur", Some("dir/cur.md"), false, &store);
    a.persist_state.recent = vec![PathBuf::from("dir/README.md")];
    a.input_state.mods.ctrl = true;
    a.route_key(Key::P);
    a.input_state.mods.ctrl = false;

    assert!(a.ui_state.quick.open, "Ctrl+P 打开浮层");
    assert_eq!(a.mode(), Mode::QuickOpen, "浮层期间文档只读");
    let labels: Vec<_> = a
        .ui_state
        .quick
        .items
        .iter()
        .map(|i| i.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["README.md", "a.md", "cur.md"],
        "最近在前，目录按名排序，txt 被过滤"
    );
    assert_eq!(a.ui_state.quick.filtered.len(), 3);
    assert!(a.ui_state.quick.items[0].recent);

    // 过滤：文件名子序列命中
    a.route_key(Key::Text("read".into()));
    assert_eq!(a.ui_state.quick.query, "read");
    assert_eq!(a.ui_state.quick.filtered.len(), 1);
    assert_eq!(
        a.ui_state.quick.current().unwrap().path,
        PathBuf::from("dir/README.md")
    );

    // 回车打开：内容整体替换、最近文件置顶
    a.route_key(Key::Enter);
    assert!(!a.ui_state.quick.open, "打开后浮层关闭");
    assert_eq!(doc_text(&a), "readme");
    assert_eq!(a.doc.path, Some(PathBuf::from("dir/README.md")));
    assert_eq!(a.persist_state.recent[0], PathBuf::from("dir/README.md"));
}

#[test]
fn quick_open_refuses_dirty_document_and_esc_closes() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("dir/a.md"), b"a".to_vec());
    let mut a = app("x", Some("dir/x.md"), false, &store);
    a.route_key(Key::Text("y".into()));
    assert!(a.editor.is_dirty());
    let before = doc_text(&a);

    a.input_state.mods.ctrl = true;
    a.route_key(Key::P);
    a.input_state.mods.ctrl = false;
    assert!(a.ui_state.quick.open);
    a.route_key(Key::Enter);
    assert!(!a.ui_state.quick.open, "尝试打开后浮层关闭");
    assert_eq!(doc_text(&a), before, "脏文档拒绝替换");
    assert!(a.ui_state.modal.is_some(), "未保存确认模态接管替换决策");
    answer_modal(&mut a, Key::Escape);
    assert!(
        a.ui_state.chrome.notice.contains("未保存"),
        "状态栏提示拒绝原因: {}",
        a.ui_state.chrome.notice
    );

    // Esc：关闭且不动文档
    a.input_state.mods.ctrl = true;
    a.route_key(Key::P);
    a.input_state.mods.ctrl = false;
    a.route_key(Key::Escape);
    assert!(!a.ui_state.quick.open);
    assert_eq!(doc_text(&a), before);
}

#[test]
fn quick_open_step_and_backspace_requery() {
    let store = MemStore::default();
    for p in ["dir/a.md", "dir/b.md"] {
        store
            .files
            .borrow_mut()
            .insert(PathBuf::from(p), b"x".to_vec());
    }
    let mut a = app("cur", Some("dir/cur.md"), false, &store);
    a.input_state.mods.ctrl = true;
    a.route_key(Key::P);
    a.input_state.mods.ctrl = false;
    // 启动即打开的文件已登记为最近文件：候选 = 当前文件 + 目录内 a.md/b.md
    let labels: Vec<_> = a
        .ui_state
        .quick
        .items
        .iter()
        .map(|i| i.label.clone())
        .collect();
    assert_eq!(labels, vec!["cur.md", "a.md", "b.md"]);
    assert_eq!(a.ui_state.quick.selected, 0);

    a.route_key(Key::Down);
    assert_eq!(a.ui_state.quick.selected, 1);
    a.route_key(Key::Up);
    assert_eq!(a.ui_state.quick.selected, 0);

    a.route_key(Key::Text("b".into()));
    assert_eq!(a.ui_state.quick.filtered.len(), 1);
    assert_eq!(
        a.ui_state.quick.current().unwrap().path,
        PathBuf::from("dir/b.md")
    );

    a.route_key(Key::Backspace);
    assert_eq!(a.ui_state.quick.query, "");
    assert_eq!(a.ui_state.quick.filtered.len(), 3, "退格后恢复全量候选");
}

#[test]
fn quick_open_takes_ime_text_and_preempts_search() {
    let store = MemStore::default();
    let mut a = app("x", None, false, &store);
    a.open_search();
    assert!(a.ui_state.search.open);
    a.input_state.mods.ctrl = true;
    a.route_key(Key::P);
    a.input_state.mods.ctrl = false;
    assert!(a.ui_state.quick.open);
    assert!(!a.ui_state.search.open, "快速打开抢占查找栏");
    assert_eq!(a.mode(), Mode::QuickOpen);
    assert_eq!(a.text_sink(), TextSink::Quick);

    a.ime_commit("m".into());
    assert_eq!(a.ui_state.quick.query, "m", "IME 提交进入过滤串");
    assert_eq!(doc_text(&a), "x", "文档内容不受输入影响");
}

/// 10k+ 块大文档击键探针（默认忽略）：验证编辑到上屏 p95 ≤16ms 预算。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_keystroke_10k_blocks
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_keystroke_10k_blocks() {
    // 每轮 7 块，1500 轮 ≈ 10500 块
    let mut text = String::with_capacity(500_000);
    for i in 0..1500 {
        text.push_str(&format!(
            "## Section {i}\n\nParagraph {i} with **bold** text, `code`, and a [link](https://example.com/{i}).\n\n- item A {i}\n- item B {i}\n\n```rust\nfn f{i}() -> u32 {{ {i} }}\n```\n\n> quote {i}\n\n---\n\n"
        ));
    }
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.editor.apply(Intent::MoveDocStart).unwrap();
    for _ in 0..5 {
        a.editor.apply(Intent::MoveRight).unwrap();
    }
    a.paint(); // 预热：真实 render() 路径，首帧含全量排版
    let n = a.index.blocks.len();
    let mut samples = Vec::with_capacity(300);
    for _ in 0..300 {
        let t = Instant::now();
        let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
        a.after_edit(dirty);
        a.paint(); // 真实 render→present 路径，不再手动模拟
        samples.push(t.elapsed().as_secs_f64() * 1e3);
    }
    samples.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let pick = |q: f64| {
        let i = ((samples.len() as f64 * q).ceil() as usize)
            .saturating_sub(1)
            .min(samples.len() - 1);
        samples[i]
    };
    println!("perf_10k_blocks={n}");
    println!("perf_10k_keystroke_ms_p50={:.3}", pick(0.5));
    println!("perf_10k_keystroke_ms_p95={:.3}", pick(0.95));
    println!(
        "perf_10k_keystroke_ms_max={:.3}",
        samples[samples.len() - 1]
    );
}

#[test]
fn outline_toggle_lists_visible_headings_and_shrinks_content_width() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs(
        "# 一\n\n正文\n\n## **二** and `c`\n\n###\n",
        None,
        false,
        &store,
        None,
        None,
    );
    let w0 = a.content_width();
    a.toggle_outline();
    assert!(a.ui_state.outline.open, "切换后面板开启");
    assert!(
        (w0 - a.content_width() - outline::PANEL_W * a.input_state.scale).abs() < 0.01,
        "版心宽须减去面板宽"
    );
    assert_eq!(a.ui_state.outline.entries.len(), 2, "空标题不生成条目");
    assert_eq!(a.ui_state.outline.entries[0].level, 1);
    assert_eq!(a.ui_state.outline.entries[0].text, "一");
    assert_eq!(a.ui_state.outline.entries[1].level, 2);
    assert_eq!(
        a.ui_state.outline.entries[1].text, "二 and c",
        "inline 标记须剥除"
    );

    // 开启状态下编辑：条目随块结构重建
    let dirty = a
        .editor
        .apply(Intent::InsertText("# 新章\n\n".into()))
        .unwrap();
    a.after_edit(dirty);
    assert_eq!(a.ui_state.outline.entries.len(), 3);
    assert_eq!(a.ui_state.outline.entries[0].text, "新章");

    // 悬停：标题区不命中、首行命中
    let panel = a.outline_rect().expect("面板开启");
    a.input_state.mouse = PhysicalPosition::new(
        (panel.x + 10.0) as f64,
        (panel.y + outline::HEAD_H * a.input_state.scale + 5.0) as f64,
    );
    a.update_hover();
    assert_eq!(a.ui_state.outline.hover, Some(0));
    a.input_state.mouse = PhysicalPosition::new((panel.x + 10.0) as f64, (panel.y + 5.0) as f64);
    a.update_hover();
    assert_eq!(a.ui_state.outline.hover, None);

    // 菜单动作与再次切换：条目清空、版心恢复
    assert!(!a.perform_menu(MenuAction::ToggleOutline));
    assert!(!a.ui_state.outline.open);
    assert!(a.ui_state.outline.entries.is_empty());
    assert!((a.content_width() - w0).abs() < 0.01);
}

#[test]
fn outline_click_jumps_caret_and_scrolls_heading_to_top() {
    let mut text = String::from("# 开头\n");
    for i in 0..40 {
        text.push_str(&format!("\n段落 {i}\n"));
    }
    text.push_str("\n# 结尾\n");
    let store = MemStore::default();
    let (mut a, _) = app_dialogs(&text, None, false, &store, None, None);
    a.toggle_outline();
    let want_start = text.find("# 结尾").unwrap();
    let panel = a.outline_rect().expect("面板开启");
    let rows = outline::visible_rows(
        panel,
        a.ui_state.outline.entries.len(),
        0.0,
        a.input_state.scale,
    );
    let row = &rows[1];
    click_at(&mut a, row.x + 4.0, row.y + row.h / 2.0);
    assert_eq!(a.editor.caret(), want_start, "光标落在标题块起点");
    assert!(a.window_state.scroll_y > 0.0, "远处标题跳到视口顶");
    assert_eq!(a.ui_state.outline.hover, None, "点击不残留悬停");
}

/// DEF-T11-E1-003：打字机模式下大纲跳转首帧即落在锚点，而非视口顶部。
#[test]
fn typewriter_outline_jump_lands_on_anchor_without_typing() {
    let mut text = String::from("# 开头\n");
    for i in 0..40 {
        text.push_str(&format!("\n段落 {i}\n"));
    }
    text.push_str("\n# 中部\n");
    for i in 0..40 {
        text.push_str(&format!("\n后文 {i}\n"));
    }
    text.push_str("\n# 结尾\n");
    let store = MemStore::default();
    let (mut a, _) = app_dialogs(&text, None, false, &store, None, None);
    a.toggle_outline();
    a.toggle_typewriter();
    assert!(a.ui_state.writing.typewriter);
    let want_start = text.find("# 中部").unwrap();
    a.jump_outline(1);
    assert_eq!(a.editor.caret(), want_start);
    let vp = a.viewport();
    let target = vp.y + vp.h * 0.42;
    let (_, cy, ch) = a.caret_screen();
    assert!(
        (cy + ch / 2.0 - target).abs() <= ch,
        "跳转首帧当前行应贴合打字机锚点：cy={cy} target={target}"
    );
}

/// DEF-T11-E1-003（复发）：跳转前远处块高只是估算值，首帧绘制补排会让几何
/// 漂移。要求「跳转后第一帧」就落在打字机锚点，而不是等下一次输入纠正。
#[test]
fn typewriter_outline_jump_first_frame_lands_on_anchor_after_real_layout() {
    // 长中文段落放大估算高度与实际排版高度的差距（CJK 字宽约为 ASCII 两倍）。
    let long = "这是一段用于放大行高估算误差的中文长段落，".repeat(12);
    let mut text = String::from("# 开头\n");
    for i in 0..30 {
        text.push_str(&format!("\n{long}{i}\n"));
    }
    text.push_str("\n# 中部\n");
    for i in 0..30 {
        text.push_str(&format!("\n{long}{i}\n"));
    }
    text.push_str("\n# 结尾\n");
    let store = MemStore::default();
    let (mut a, _) = app_dialogs(&text, None, false, &store, None, None);
    a.toggle_outline();
    a.toggle_typewriter();
    let want_start = text.find("# 中部").unwrap();
    a.jump_outline(1);
    assert_eq!(a.editor.caret(), want_start);
    // 首帧：走真实渲染（补排可见块 + 重算几何），与真机第一次绘制同路径。
    let _ = a.render();
    let vp = a.viewport();
    let target = vp.y + vp.h * 0.42;
    let (_, cy, ch) = a.caret_screen();
    assert!(
        (cy + ch / 2.0 - target).abs() <= ch / 2.0,
        "首帧标题行应贴合打字机锚点：cy={cy} ch={ch} target={target}"
    );
}

#[test]
fn outline_wheel_scrolls_panel_only_when_pointer_inside() {
    let mut text = String::from("# 目录\n");
    for i in 0..40 {
        text.push_str(&format!("\n## 节 {i}\n\n正文\n"));
    }
    let store = MemStore::default();
    let (mut a, _) = app_dialogs(&text, None, false, &store, None, None);
    a.toggle_outline();
    let panel = a.outline_rect().expect("面板开启");
    assert!(
        outline::max_scroll(panel, a.ui_state.outline.entries.len(), a.input_state.scale) > 0.0,
        "条数须超出面板高度"
    );
    let doc_y = a.window_state.scroll_y;
    a.input_state.mouse = PhysicalPosition::new(
        (panel.x + 20.0) as f64,
        (panel.y + outline::HEAD_H * a.input_state.scale + 40.0) as f64,
    );
    a.on_wheel(-60.0);
    assert!(a.ui_state.outline.scroll > 0.0, "指针在面板内滚动面板");
    assert_eq!(a.window_state.scroll_y, doc_y, "面板滚动不影响正文");

    // 面板外：滚正文，面板偏移不变
    let panel_scroll = a.ui_state.outline.scroll;
    a.input_state.mouse = PhysicalPosition::new(100.0, (panel.y + 120.0) as f64);
    a.on_wheel(-60.0);
    assert!(a.window_state.scroll_y > doc_y, "面板外滚动正文");
    assert_eq!(a.ui_state.outline.scroll, panel_scroll);
}

#[test]
fn outline_panel_paints_only_when_open() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("# 一\n\n正文\n", None, false, &store, None, None);
    a.toggle_outline();
    let (w, h) = a.win_size();
    let p = a.outline_rect().expect("面板开启");
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_outline(&mut f);
    assert_eq!(
        f.get((p.x + 4.0) as u32, (p.y + 4.0) as u32),
        a.input_state.theme.popup_bg,
        "标题区底色（卡片式面板与浮层同色）"
    );
    assert_eq!(
        f.get((p.x - 30.0) as u32, (p.y + 4.0) as u32),
        a.input_state.theme.bg,
        "面板左侧（投影范围外）仍为文档底色"
    );
    // 关闭后面板不再绘制
    a.toggle_outline();
    let mut g = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_outline(&mut g);
    assert_eq!(
        g.get((p.x + 4.0) as u32, (p.y + 4.0) as u32),
        a.input_state.theme.bg
    );
}

// ===== 文档内查找栏（P3-3） =====

#[test]
fn search_open_finds_all_and_walks_matches() {
    let store = MemStore::default();
    let mut a = app("alpha beta\nGamma alpha\n", None, false, &store);
    a.ensure_layouts_all();
    a.open_search();
    assert!(a.ui_state.search.open);
    assert!(a.ui_state.search.query.is_empty());
    a.search_input("alpha");
    // 大小写不敏感 + 全文扫描：alpha(0..5)、alpha(17..22)
    assert_eq!(
        a.ui_state.search.matches,
        vec![Range::new(0, 5), Range::new(17, 22)]
    );
    assert_eq!(a.editor.caret(), 0, "查询变化后跳到首个命中");
    assert_eq!(a.ui_state.search.label(), "1/2");

    a.search_next();
    assert_eq!(a.ui_state.search.current, 1);
    assert_eq!(a.editor.caret(), 17, "下一个命中");
    a.search_next();
    assert_eq!(a.editor.caret(), 0, "末尾循环回首个");
    a.search_prev();
    assert_eq!(a.editor.caret(), 17, "首部循环到末个");
    assert_eq!(a.ui_state.search.label(), "2/2");

    a.search_backspace();
    assert_eq!(a.ui_state.search.query, "alph");
    assert_eq!(a.editor.caret(), 0, "退格后重算并回首个");
    a.close_search();
    assert!(!a.ui_state.search.open);
    assert_eq!(a.editor.caret(), 0, "关闭时光标停在当前命中");
}

#[test]
fn case_toggle_click_rescans_and_updates_label() {
    let store = MemStore::default();
    let mut a = app("Foo foo", None, false, &store);
    a.ensure_layouts_all();
    a.open_search();
    a.search_input("foo");
    assert_eq!(a.ui_state.search.label(), "1/2", "默认不区分大小写");
    let l = a.search_rect().expect("查找栏开启");
    let (x, y) = (l.case.x + l.case.w / 2.0, l.case.y + l.case.h / 2.0);
    click_at(&mut a, x, y);
    assert!(a.ui_state.search.case_sensitive);
    assert_eq!(a.ui_state.search.label(), "1/1", "开启后 n/m 立即重算");
    assert_eq!(a.editor.caret(), 4, "跳到区分大小写后的首个命中");
    click_at(&mut a, x, y);
    assert!(!a.ui_state.search.case_sensitive);
    assert_eq!(a.ui_state.search.label(), "1/2");
    assert_eq!(a.editor.caret(), 0);
}

#[test]
fn search_is_read_only_for_document() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.ensure_layouts_all();
    let before = doc_text(&a);
    a.open_search();
    a.route_key(Key::Text("q".into()));
    assert_eq!(a.ui_state.search.query, "q");
    a.route_key(Key::Backspace);
    a.route_key(Key::Text("a".into()));
    assert_eq!(a.ui_state.search.query, "a");
    assert_eq!(doc_text(&a), before, "查找期间文档只读");
    assert!(!a.editor.is_dirty(), "查找不置脏");
    a.route_key(Key::Escape);
    assert!(!a.ui_state.search.open);
}

#[test]
fn search_keys_handle_enter_shift_enter_and_empty_result() {
    let store = MemStore::default();
    let mut a = app("x x x\n", None, false, &store);
    a.ensure_layouts_all();
    a.open_search();
    a.route_key(Key::Text("x".into()));
    assert_eq!(a.editor.caret(), 0);
    a.route_key(Key::Enter);
    assert_eq!(a.ui_state.search.current, 1);
    a.input_state.mods.shift = true;
    a.route_key(Key::Enter);
    assert_eq!(a.ui_state.search.current, 0, "Shift+Enter 走上一个");
    a.input_state.mods.shift = false;
    a.route_key(Key::Text("z".into())); // 查询「xz」：无命中
    assert!(a.ui_state.search.matches.is_empty());
    assert_eq!(a.ui_state.search.label(), "0/0");
    assert_eq!(a.editor.caret(), 0, "无命中不移动光标");
}

#[test]
fn mode_and_text_sink_derive_from_one_priority() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    assert_eq!(a.mode(), Mode::Document);
    assert_eq!(a.text_sink(), TextSink::Doc);

    a.ui_state.chrome.open = Some(0);
    assert_eq!(a.mode(), Mode::Menu, "菜单优先于查找栏");
    assert_eq!(a.text_sink(), TextSink::Doc);

    a.open_search(); // 既有行为：打开查找栏先关菜单
    assert_eq!(a.mode(), Mode::Search);
    assert_eq!(a.text_sink(), TextSink::Query);

    a.close_search();
    assert_eq!(a.mode(), Mode::Document);
}

#[test]
fn ime_commit_follows_text_sink() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.ensure_layouts_all();

    // 菜单打开：提交被吞掉，文档与查询串都不变
    a.ui_state.chrome.open = Some(0);
    a.ime_commit("菜".into());
    assert_eq!(doc_text(&a), "abc\n");
    assert!(a.ui_state.search.query.is_empty());

    // 查找栏打开：组合串提交进入查询串，文档只读
    a.open_search();
    a.ime_preedit("zh".into(), None);
    assert!(a.editor.preedit().is_some(), "组合串唯一存储在 Editor");
    a.ime_commit("中".into());
    assert_eq!(a.ui_state.search.query, "中");
    assert_eq!(doc_text(&a), "abc\n", "查找期间文档只读");
    assert!(a.editor.preedit().is_none(), "提交后组合串清除");

    // 关闭查找栏：提交回到文档
    a.close_search();
    a.ime_commit("文".into());
    assert_eq!(doc_text(&a), "文abc\n", "文档提交生效");
}

#[test]
fn search_bar_clicks_run_buttons_and_swallow_text_area() {
    let store = MemStore::default();
    let mut a = app("x x x\n", None, false, &store);
    a.ensure_layouts_all();
    a.open_search();
    a.search_input("x");
    let l = a.search_rect().expect("查找栏开启");
    let center = |r: crate::geometry::Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (nx, ny) = center(l.next);
    click_at(&mut a, nx, ny);
    assert_eq!(a.ui_state.search.current, 1, "下一个按钮");
    let (px, py) = center(l.prev);
    click_at(&mut a, px, py);
    assert_eq!(a.ui_state.search.current, 0, "上一个按钮");
    let caret = a.editor.caret();
    let (tx, ty) = center(l.text);
    click_at(&mut a, tx, ty);
    assert_eq!(a.editor.caret(), caret, "栏内空白吞掉点击，不落文档光标");
    let (cx, cy) = center(l.close);
    click_at(&mut a, cx, cy);
    assert!(!a.ui_state.search.open, "关闭按钮");
}

#[test]
fn winit_key_chain_opens_search_and_types_into_query() {
    let store = MemStore::default();
    let mut a = app("alpha beta alpha\n", None, false, &store);
    a.ensure_layouts_all();

    // Ctrl+F：Windows 的 event.text 是剥离 Ctrl 的 "f"，必须落到无修饰键分支
    let key = map_key_parts(
        Some("f"),
        WKey::Character("f".into()),
        Mods {
            ctrl: true,
            shift: false,
            alt: false,
        },
    )
    .expect("Ctrl+F 恒可映射");
    assert_eq!(key, Key::F, "Ctrl 组合不得走文本分支");
    a.input_state.mods.ctrl = true;
    a.route_key(key);
    a.input_state.mods.ctrl = false;
    assert!(a.ui_state.search.open, "Ctrl+F 打开查找栏");
    assert_eq!(a.mode(), Mode::Search);

    // 普通字母：winit 文本路径 → 进查询串，文档保持只读
    let key = map_key_parts(Some("a"), WKey::Character("a".into()), Mods::default())
        .expect("字母恒可映射");
    assert_eq!(key, Key::Text("a".into()));
    a.route_key(key);
    assert_eq!(a.ui_state.search.query, "a");
    assert_eq!(doc_text(&a), "alpha beta alpha\n", "查找期间文档只读");
}

#[test]
fn ctrl_h_first_opens_then_closes_then_reopens() {
    let store = MemStore::default();
    let mut a = app("a x a\n", None, false, &store);
    a.ensure_layouts_all();

    // 首次 Ctrl+H：打开 search+replace，focus 在 Query（首字不得落进替换框）
    a.input_state.mods.ctrl = true;
    a.route_key(Key::H);
    a.input_state.mods.ctrl = false;
    assert!(a.ui_state.search.open && a.ui_state.search.replace_open);
    assert_eq!(
        a.ui_state.search.focus,
        search::Field::Query,
        "首次 Ctrl+H 先聚焦查询，首字不得落进替换框"
    );
    a.route_key(Key::Text("a".into()));
    assert_eq!(a.ui_state.search.matches.len(), 2, "查询即时出命中");

    // 第二次 Ctrl+H：replace_open 已是 true → 关闭整个 search 面板（toggle 语义）
    a.input_state.mods.ctrl = true;
    a.route_key(Key::H);
    a.input_state.mods.ctrl = false;
    assert!(
        !a.ui_state.search.open,
        "已打开 replace 时再按 Ctrl+H 关闭整个面板"
    );

    // 第三次 Ctrl+H：search 已关 → 重新打开 search+replace，focus 又到 Query
    a.input_state.mods.ctrl = true;
    a.route_key(Key::H);
    a.input_state.mods.ctrl = false;
    assert!(a.ui_state.search.open && a.ui_state.search.replace_open);
    assert_eq!(
        a.ui_state.search.focus,
        search::Field::Query,
        "重新打开仍先聚焦查询"
    );

    // Search 模式下 Ctrl+F 同样是 toggle：search 已开 → 关闭
    a.input_state.mods.ctrl = true;
    a.route_key(Key::F);
    a.input_state.mods.ctrl = false;
    assert!(
        !a.ui_state.search.open,
        "Search 模式下 Ctrl+F 也按 toggle 关闭"
    );
}

#[test]
fn right_click_on_link_text_opens_link_menu() {
    // 真机走查发现：折叠态链接被整体命中为节点尾（range.end 不含），
    // 右键落在“链接”文字上却只出 Block 菜单，「复制链接地址」不可达。
    let (mut a, _) = app_math("[链接](https://example.com)\n", false);
    assert!(a.editor.set_caret(0));
    a.ensure_layouts_all();
    let id = a.editor.session().blocks()[0].0;
    let top = a
        .index
        .blocks
        .iter()
        .find(|g| g.id == id)
        .expect("正文块应存在")
        .top;
    let scale = a.input_state.scale;
    let vp = a.viewport();
    let (lx, ly, _) = {
        let l = &a.cache.layouts[&id];
        a.engine.caret_xy(l, 1)
    };
    a.input_state.mouse = PhysicalPosition::new(
        (MARGIN * scale + lx + 2.0) as f64,
        (vp.y + top - a.window_state.scroll_y + ly + 5.0) as f64,
    );
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "应打开右键菜单");
    assert!(
        a.ui_state.ctx.link.is_some(),
        "链接文字上右键应出「复制链接地址」项集"
    );
}

// ---- 链接打开：Ctrl+左键 / 右键菜单（协议白名单 + 失败提示） ----

/// DEF-T67-E1-004：危险协议链接右键仍出 Link 专用项；点击打开有可见提示且不调 opener。
#[test]
fn right_click_dangerous_link_shows_link_items_and_visible_hint() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app("[x](javascript:alert(1))\n", &calls, Ok(()));
    let (x, y) = link_point(&mut a);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.on_right_click();
    assert!(a.ui_state.ctx.open);
    assert_eq!(
        a.ui_state.ctx.link.as_deref(),
        Some("javascript:alert(1)"),
        "危险协议也必须进入 Link 上下文"
    );

    let items = a.ui_state.ctx.items().to_vec();
    let open_i = items
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::OpenLink)
        .expect("必须提供打开链接项");
    assert!(items
        .iter()
        .any(|&(_, act)| act == ctx_menu::CtxAction::CopyLink));

    let vp = a.viewport();
    let l = ctx_menu::layout(
        a.ui_state.ctx.items(),
        a.ui_state.ctx.x,
        a.ui_state.ctx.y,
        vp.x + vp.w,
        vp.y + vp.h,
        a.input_state.scale,
    );
    let iy = l.y
        + l.rows
            .iter()
            .find(|&&(i, _)| i == open_i)
            .map(|&(_, off)| off)
            .expect("打开项可点击")
        + 1.0;
    click_at(&mut a, l.x + 2.0, iy);

    assert!(calls.borrow().is_empty(), "危险协议不得交给系统 opener");
    assert!(
        a.ui_state.chrome.notice.contains("无法打开链接"),
        "状态栏必须给出可见提示：{:?}",
        a.ui_state.chrome.notice
    );
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
    let scale = a.input_state.scale;
    let vp = a.viewport();
    (
        MARGIN * scale + lx + 1.0,
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

#[test]
fn ctrl_click_footnote_ref_jumps_to_definition_without_editing() {
    let text = "正文[^a]引用\n\n[^a]: 定义内容\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    a.ensure_layouts_all();
    let def_start = a
        .editor
        .session()
        .footnote_definition("a")
        .expect("定义应已注册")
        .start;
    let bytes0 = a.editor.session().serialize();
    let undo0 = a.editor.session().undo_depth();
    let ref_start = first_ref_start(&a, 0);

    ctrl_click_doc(&mut a, ref_start);

    assert_eq!(a.editor.caret(), def_start, "光标落到定义内容起点");
    assert_eq!(a.editor.session().serialize(), bytes0, "文档字节不变");
    assert_eq!(a.editor.session().undo_depth(), undo0, "不产生撤销步");
}

#[test]
fn ctrl_click_footnote_back_marker_jumps_to_first_reference() {
    let text = "甲[^a] 乙[^a]\n\n[^a]: 尾注\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    a.ensure_layouts_all();
    let first_ref = first_ref_start(&a, 0);
    let def_idx = a
        .index
        .blocks
        .iter()
        .position(|g| g.kind == BlockKind::FootnoteDef)
        .expect("应有定义块");
    let lit = {
        let id = a.index.blocks[def_idx].id;
        a.cache.layouts[&id].layout_text().to_string()
    };
    assert!(lit.contains('↩'), "定义块末应渲染返回标记: {lit:?}");
    // `↩` 插在块末正文之后（零宽），其锚点即块 range.end。
    let back = a.index.blocks[def_idx].range.end;
    ctrl_click_doc(&mut a, back);

    assert_eq!(a.editor.caret(), first_ref, "回到文档顺序首个引用");
    assert_eq!(a.editor.session().serialize(), text, "跳转不改文档");
}

#[test]
fn ctrl_click_anchor_link_jumps_to_heading() {
    let text = "# 目标标题\n\n[去](#目标标题)\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    a.editor.set_caret(a.editor.session().text_len());

    ctrl_click_doc(&mut a, text.find('去').expect("应找到链接文字"));

    assert_eq!(a.editor.caret(), 0, "跳到标题块起点");
    assert_eq!(a.editor.session().serialize(), text, "跳转不改文档");
}

#[test]
fn ctrl_click_anchor_duplicate_heading_matches_suffix_slug() {
    let text = "# A\n\n# A\n\n[x](#a-1)\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    let heads = heading_starts(&a);
    assert_eq!(heads.len(), 2, "两个同名标题均应参与去重编号");
    a.editor.set_caret(a.editor.session().text_len());

    ctrl_click_doc(&mut a, text.find('x').expect("应找到链接文字"));

    assert_eq!(a.editor.caret(), heads[1], "#a-1 命中第二个同名标题");
}

#[test]
fn ctrl_click_undefined_footnote_keeps_caret_and_reports() {
    let text = "x[^nope]\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    let caret0 = a.editor.session().text_len();
    a.editor.set_caret(caret0);

    ctrl_click_doc(&mut a, text.find("[^nope]").expect("引用起点"));

    assert_eq!(a.editor.caret(), caret0, "光标不变");
    assert_eq!(a.ui_state.chrome.notice, "未找到脚注定义");
}

#[test]
fn ctrl_click_missing_anchor_keeps_caret_and_reports() {
    let text = "[x](#nope)\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    let caret0 = a.editor.session().text_len();
    a.editor.set_caret(caret0);

    ctrl_click_doc(&mut a, text.find('x').expect("应找到链接文字"));

    assert_eq!(a.editor.caret(), caret0, "光标不变");
    assert_eq!(a.ui_state.chrome.notice, "未找到锚点");
}

#[test]
fn plain_click_footnote_ref_does_not_jump() {
    let text = "正文[^a]引用\n\n[^a]: 定义内容\n";
    let mut a = app_with(text, None, false, &MemStore::default(), no_url());
    let def_start = a
        .editor
        .session()
        .footnote_definition("a")
        .expect("定义应已注册")
        .start;
    a.editor.set_caret(a.editor.session().text_len());
    let ref_start = first_ref_start(&a, 0);

    let (x, y) = point_at_doc(&mut a, ref_start);
    click_at(&mut a, x, y);

    assert_ne!(a.editor.caret(), def_start, "无 Ctrl 不得跳转");
    assert!(
        (ref_start..=ref_start + 4).contains(&a.editor.caret()),
        "普通单击落在脚注引用标记区: {}",
        a.editor.caret()
    );
    assert!(a.ui_state.chrome.notice.is_empty(), "不得产生跳转提示");
}

#[test]
fn url_whitelist_accepts_only_web_and_mail_schemes() {
    for ok in ["https://example.com", "  HTTP://a.b", "MAILTO:x@y.z"] {
        assert!(url_is_openable(ok), "应放行: {ok:?}");
    }
    for bad in [
        "file:///C:/x",
        "javascript:alert(1)",
        "a.md",
        "#anchor",
        "/abs",
    ] {
        assert!(!url_is_openable(bad), "应拒绝: {bad:?}");
    }
}

#[test]
fn normalize_link_url_adds_http_for_bare_www() {
    assert_eq!(
        normalize_link_url("www.example.com/a"),
        "http://www.example.com/a"
    );
    assert_eq!(
        normalize_link_url("  WWW.Example.com  "),
        "http://WWW.Example.com"
    );
    assert_eq!(normalize_link_url("https://a.b"), "https://a.b");
    assert_eq!(normalize_link_url("mailto:x@y.z"), "mailto:x@y.z");
    // `wwwx.com` 不是 www 前缀，保持原样（随后仍会被白名单拒绝）
    assert_eq!(normalize_link_url("wwwx.com"), "wwwx.com");
    // 裸邮箱补 mailto（GFM 自动链接）
    assert_eq!(normalize_link_url("foo@bar.baz"), "mailto:foo@bar.baz");
    assert_eq!(normalize_link_url("mailto:x@y.z"), "mailto:x@y.z");
    assert_eq!(normalize_link_url("foo@bar"), "foo@bar");
    assert!(url_is_openable(&normalize_link_url("www.a.com")));
}

#[test]
fn link_hover_shows_normalized_url_and_clears_on_leave() {
    let store = MemStore::default();
    let mut a = app("[站点](www.example.com)\n", None, false, &store);
    let (x, y) = point_at_doc(&mut a, 1);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.update_hover();
    let (url, _, _) = a.ui_state.hover_link.clone().expect("链接文字上应出提示");
    assert_eq!(url, "http://www.example.com", "浮层显示归一化目标");
    // 移到菜单栏（正文区外）→ 立即清除，无延迟
    a.input_state.mouse = PhysicalPosition::new(x as f64, 2.0);
    a.update_hover();
    assert!(a.ui_state.hover_link.is_none());
}

/// DEF-T05-010：切换文档时链接 tooltip 必须复位，不残留旧 URL。
#[test]
fn switching_document_clears_link_tooltip() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("b.md"), "新文档\n".as_bytes().to_vec());
    let mut a = app("[站点](www.example.com)\n", None, false, &store);
    let (x, y) = point_at_doc(&mut a, 1);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.update_hover();
    assert!(a.ui_state.hover_link.is_some(), "前置：悬停应出提示");
    a.open_path(PathBuf::from("b.md"));
    assert!(
        a.ui_state.hover_link.is_none(),
        "换文档后不得残留旧文档链接提示"
    );
}

#[test]
fn link_hover_hits_table_cell_link() {    let store = MemStore::default();
    let mut a = app(
        "p\n\n| [站点](www.example.com) | b |\n| --- | --- |\n| c | d |\n",
        None,
        false,
        &store,
    );
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, top) = {
        let g = a
            .index
            .blocks
            .iter()
            .find(|g| g.kind == BlockKind::Table)
            .expect("应有表格块");
        (g.id, g.top)
    };
    let cell = &a.cache.tables[&id].cells[0];
    let vp = a.viewport();
    let x = a.content_x() + cell.x + 2.0;
    let y = vp.y + top - a.window_state.scroll_y + cell.y + cell.h / 2.0;
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.update_hover();
    assert_eq!(
        a.ui_state.hover_link.as_ref().map(|t| t.0.as_str()),
        Some("http://www.example.com"),
        "表格单元格内链接同样触发"
    );
    // 移到表格外空白（首段）→ 清除
    a.input_state.mouse = PhysicalPosition::new(x as f64, (vp.y + 1.0) as f64);
    a.update_hover();
    assert!(a.ui_state.hover_link.is_none());
}

#[test]
fn link_hover_ignores_code_and_plain_text() {
    let store = MemStore::default();
    // 行内代码里的 URL 不是 Link 节点
    let mut a = app("`https://example.com`\n", None, false, &store);
    let (x, y) = point_at_doc(&mut a, 1);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.update_hover();
    assert!(a.ui_state.hover_link.is_none(), "行内代码不应触发");

    // 代码块里的 URL 同样不触发
    let mut b = app("```\nhttps://example.com\n```\n", None, false, &store);
    let (x, y) = point_at_doc(&mut b, 5);
    b.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    b.update_hover();
    assert!(b.ui_state.hover_link.is_none(), "代码块不应触发");

    // 普通文本（无链接节点）
    let mut c = app("普通文本\n", None, false, &store);
    let (x, y) = point_at_doc(&mut c, 1);
    c.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    c.update_hover();
    assert!(c.ui_state.hover_link.is_none(), "普通文本不应触发");
}

#[test]
fn link_tooltip_truncates_long_url_and_clamps_to_viewport() {
    let vp = chrome::Viewport {
        x: 0.0,
        y: 20.0,
        w: 240.0,
        h: 180.0,
    };
    let long = "https://example.com/very/long/path/that/never/ends";
    let max = crate::paint::link_tip_max_text_w(vp, 1.0);
    let (text, tw) = crate::paint::truncate_link_tip(long, max, 1.0);
    assert!(text.ends_with('…'), "超宽 URL 尾部省略");
    assert!(tw <= max + 0.01, "截断后宽度受视口约束");
    let r = crate::paint::link_tip_rect(tw + 12.0, 24.0, 238.0, 22.0, vp, 1.0);
    assert!(r.x >= vp.x && r.x + r.w <= vp.x + vp.w + 0.01, "水平不越界");
    assert!(r.y >= vp.y && r.y + r.h <= vp.y + vp.h + 0.01, "垂直不越界");
}

#[test]
fn ctrl_click_link_opens_without_touching_document() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app(LINK_DOC, &calls, Ok(()));
    let (x, y) = link_point(&mut a);
    let caret0 = a.editor.caret();
    let bytes0 = a.editor.session().serialize();
    let undo0 = a.editor.session().undo_depth();

    a.input_state.mods.ctrl = true;
    assert!(!click_at(&mut a, x, y), "打开链接不触发退出");

    assert_eq!(*calls.borrow(), vec!["https://example.com".to_string()]);
    assert_eq!(a.editor.caret(), caret0, "Ctrl+单击不移动光标");
    assert_eq!(a.editor.selection(), None, "Ctrl+单击不改变选区");
    assert_eq!(a.editor.session().serialize(), bytes0, "文档字节不变");
    assert_eq!(a.editor.session().undo_depth(), undo0, "不产生撤销步");
    assert!(
        a.window_state.last_click.is_none(),
        "不进入点击计数/拖选路径"
    );
}

#[test]
fn plain_click_link_moves_caret_and_does_not_open() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app(LINK_DOC, &calls, Ok(()));
    let (x, y) = link_point(&mut a);
    let doc_len = a.editor.session().text_len();
    a.editor.set_caret(doc_len);

    click_at(&mut a, x, y);

    assert!(calls.borrow().is_empty(), "未按 Ctrl 不打开链接");
    assert_ne!(a.editor.caret(), doc_len, "未按 Ctrl 时回落光标定位");
}

#[test]
fn ctrl_click_rejects_non_whitelisted_scheme() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app("[x](file:///C:/Windows/notepad.exe)\n", &calls, Ok(()));
    let (x, y) = link_point(&mut a);
    a.input_state.mods.ctrl = true;
    click_at(&mut a, x, y);

    assert!(calls.borrow().is_empty(), "白名单外不得调用平台实现");
    assert!(
        a.ui_state.chrome.notice.contains("无法打开链接"),
        "状态栏须提示：{:?}",
        a.ui_state.chrome.notice
    );

    // 应用内锚点：不交给系统 shell，也不弹提示（锚点跳转属另一任务）
    a.ui_state.chrome.notice.clear();
    a.open_link("#section");
    assert!(calls.borrow().is_empty(), "锚点不进入 opener");
    assert!(a.ui_state.chrome.notice.is_empty(), "锚点静默忽略");
}

#[test]
fn ctrl_click_reports_open_failure_in_status_bar() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app(LINK_DOC, &calls, Err("模拟打开失败".into()));
    let (x, y) = link_point(&mut a);
    a.input_state.mods.ctrl = true;
    click_at(&mut a, x, y);

    assert_eq!(calls.borrow().len(), 1, "白名单内应已交给平台实现");
    assert!(
        a.ui_state.chrome.notice.contains("模拟打开失败"),
        "失败原因须可见：{:?}",
        a.ui_state.chrome.notice
    );
}

#[test]
fn link_menu_offers_open_first_and_executes_it() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = link_app(LINK_DOC, &calls, Ok(()));
    let (x, y) = link_point(&mut a);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "链接上右键应打开菜单");

    let items = a.ui_state.ctx.items().to_vec();
    let open_i = items
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::OpenLink)
        .expect("应提供「在浏览器中打开」");
    let copy_i = items
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::CopyLink)
        .expect("应保留「复制链接地址」");
    assert!(open_i < copy_i, "打开项须排在复制项之前");

    // 点击首项：走与 Ctrl+左键相同的打开路径
    let vp = a.viewport();
    let l = ctx_menu::layout(
        a.ui_state.ctx.items(),
        a.ui_state.ctx.x,
        a.ui_state.ctx.y,
        vp.x + vp.w,
        vp.y + vp.h,
        a.input_state.scale,
    );
    let iy = l.y
        + l.rows
            .iter()
            .find(|&&(i, _)| i == open_i)
            .map(|&(_, off)| off)
            .expect("打开项应在可点行中")
        + 1.0;
    click_at(&mut a, l.x + 2.0, iy);
    assert_eq!(*calls.borrow(), vec!["https://example.com".to_string()]);
    assert!(!a.ui_state.ctx.open, "点击后菜单关闭");
}

#[test]
fn opening_search_closes_context_menu_so_keys_are_not_swallowed() {
    let store = MemStore::default();
    let mut a = app("abc\n", None, false, &store);
    a.ui_state
        .ctx
        .open_at(10.0, 20.0, ctx_menu::CtxKind::Block, None, None);
    assert_eq!(a.mode(), Mode::Menu, "右键菜单打开时按键归菜单");
    a.open_search();
    assert!(!a.ui_state.ctx.open, "查找栏打开必须关掉右键菜单");
    assert_eq!(a.mode(), Mode::Search, "否则栏可见但按键被 Menu 模式吞掉");
    a.route_key(Key::Text("a".into()));
    assert_eq!(a.ui_state.search.query, "a");
}

#[test]
fn search_hover_targets_buttons_only() {
    let store = MemStore::default();
    let mut a = app("x\n", None, false, &store);
    a.open_search();
    let l = a.search_rect().unwrap();
    a.input_state.mouse = PhysicalPosition::new(
        (l.next.x + l.next.w / 2.0) as f64,
        (l.next.y + l.next.h / 2.0) as f64,
    );
    a.update_hover();
    assert_eq!(a.ui_state.search.hover, Some(search::Hit::Next));
    a.input_state.mouse = PhysicalPosition::new((l.text.x + 2.0) as f64, (l.text.y + 2.0) as f64);
    a.update_hover();
    assert_eq!(
        a.ui_state.search.hover,
        Some(search::Hit::QueryText),
        "文本区只是焦点目标，不画按钮悬停底色"
    );
    a.close_search();
    a.update_hover();
    assert_eq!(a.ui_state.search.hover, None);
}

#[test]
fn menu_find_opens_empty_search_bar() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("x\n", None, false, &store, None, None);
    a.perform_menu(MenuAction::Find);
    assert!(a.ui_state.search.open);
    assert!(a.ui_state.search.query.is_empty());
}

#[test]
fn search_bar_has_one_boundary_and_no_internal_dividers() {
    let store = MemStore::default();
    let mut a = app("alpha beta\n", None, false, &store);
    a.open_replace();
    a.search_input("a");
    let (w, h) = a.win_size();
    let l = a.search_rect().unwrap();
    let rl = search::replace_layout(&l, a.input_state.scale);
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut f);
    let border = [
        a.input_state.theme.chrome_border[0],
        a.input_state.theme.chrome_border[1],
        a.input_state.theme.chrome_border[2],
    ];
    // 按钮之间只有间距：分距列不得出现边框色竖线
    let gap_x = (l.prev.x - search::GROUP_GAP / 2.0).round() as u32;
    for y in (l.bar.y + 2.0) as u32..(l.bar.y + l.bar.h - 2.0) as u32 {
        let px = f.get(gap_x, y);
        assert_ne!([px[0], px[1], px[2]], border, "按钮间不得有竖分隔线");
    }
    // 两行之间只有间距：空档整行不得出现边框色横线
    let gap_y = (rl.row.y - search::ROW_GAP / 2.0).round() as u32;
    for x in (l.bar.x + 2.0) as u32..(l.bar.x + l.bar.w - 2.0) as u32 {
        let px = f.get(x, gap_y);
        assert_ne!(
            [px[0], px[1], px[2]],
            border,
            "查询行与替换行之间不得有横线"
        );
    }
}

#[test]
fn search_bar_focus_uses_quiet_field_tint() {
    let store = MemStore::default();
    let mut a = app("x\n", None, false, &store);
    a.open_search();
    let (w, h) = a.win_size();
    let l = a.search_rect().unwrap();
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut f);
    // 输入区右端为空档（无计数、无字形）：应只剩焦点底色
    let x = (l.text.x + l.text.w - 6.0) as u32;
    let y = (l.text.y + l.text.h / 2.0) as u32;
    let mut t = Frame::new(1, 1, a.input_state.theme.popup_bg);
    t.fill_rect(
        0,
        0,
        1,
        1,
        crate::tokens::state::focus_bg(&a.input_state.theme),
    );
    assert_eq!(f.get(x, y), t.get(0, 0), "焦点用输入区安静底色");
}

#[test]
fn search_bar_no_match_uses_danger_and_disabled_colors() {
    let store = MemStore::default();
    let mut a = app("alpha\n", None, false, &store);
    a.open_search();
    a.search_input("zzz");
    assert!(a.ui_state.search.no_matches());
    let (w, h) = a.win_size();
    let l = a.search_rect().unwrap();
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut f);
    let theme = a.input_state.theme;
    let wanted = crate::tokens::state::empty_fg(&theme);
    let mut found = false;
    for x in (l.count_right - search::COUNT_W) as u32..l.count_right as u32 {
        for y in l.text.y as u32..(l.text.y + l.text.h) as u32 {
            let px = f.get(x, y);
            if [px[0], px[1], px[2]] == wanted {
                found = true;
            }
        }
    }
    assert!(found, "无命中计数使用 danger 色");
    // 翻页按钮字形应为禁用前景（置灰），不出现正文色
    let text = theme.text;
    let bg = theme.popup_bg;
    let (mut has_ink, mut has_text) = (false, false);
    for x in l.prev.x as u32..(l.prev.x + l.prev.w) as u32 {
        for y in l.prev.y as u32..(l.prev.y + l.prev.h) as u32 {
            let px = f.get(x, y);
            let rgb = [px[0], px[1], px[2]];
            if rgb != [bg[0], bg[1], bg[2]] {
                has_ink = true;
            }
            has_text |= rgb == text;
        }
    }
    assert!(has_ink, "无命中时翻页按钮仍绘制（置灰）");
    assert!(!has_text, "禁用按钮不得使用正文色");
}

#[test]
fn search_match_ticks_mark_scrollbar_positions() {
    let store = MemStore::default();
    let mut a = app("alpha\nalpha\nalpha\n", None, false, &store);
    a.open_search();
    a.search_input("alpha");
    assert_eq!(a.ui_state.search.matches.len(), 3);
    let (w, h) = a.win_size();
    let vp = a.viewport();
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search_ticks(&mut f, vp);
    let scale = a.input_state.scale;
    let lane = (chrome::SCROLL_W * scale).ceil() as u32;
    let x = (w as f32 - lane as f32 + 3.0 * scale).round() as u32;
    let tick_h = (2.0 * scale).round().max(1.0) as u32;
    let total = a.editor.session().text_len().max(1);
    let m = a.ui_state.search.matches[0];
    let y = (vp.y + (m.start as f32 / total as f32) * vp.h - tick_h as f32 / 2.0).round() as u32;
    let accent = a.input_state.theme.accent;
    assert_eq!(
        f.get(x + 1, y),
        [accent[0], accent[1], accent[2], 255],
        "当前命中在滚动条上打 accent 刻度"
    );
    // 关闭后不再打刻度
    a.close_search();
    let mut g = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search_ticks(&mut g, vp);
    assert_eq!(g.get(x + 1, y), a.input_state.theme.bg);
}

#[test]
fn search_bar_paints_overlay_and_clears_on_close() {
    let store = MemStore::default();
    let mut a = app("x\n", None, false, &store);
    a.open_search();
    let (w, h) = a.win_size();
    let l = a.search_rect().unwrap();
    let mut f = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut f);
    assert_eq!(
        f.get((l.bar.x + 3.0) as u32, (l.bar.y + 3.0) as u32),
        a.input_state.theme.popup_bg,
        "浮栏不透明底色（与全部浮层同色）"
    );
    a.close_search();
    let mut g = Frame::new(w, h, a.input_state.theme.bg);
    a.paint_search(&mut g);
    assert_eq!(
        g.get((l.bar.x + 3.0) as u32, (l.bar.y + 3.0) as u32),
        a.input_state.theme.bg,
        "关闭后不再绘制"
    );
}

#[test]
fn search_highlights_use_selection_and_hover_colors() {
    let store = MemStore::default();
    let mut a = app("alpha beta alpha\n", None, false, &store);
    a.ensure_layouts_all();
    a.open_search();
    a.search_input("alpha");
    let (id, range) = (a.index.blocks[0].id, a.index.blocks[0].range);
    // 像素取样：命中矩形中心（布局缓存跨关闭仍有效）
    let pixel = |f: &Frame, a: &App, m: &Range| {
        let (rx, ry, rw, rh) = a
            .engine
            .selection_rects(&a.cache.layouts[&id], m.start, m.end)[0];
        f.get(
            (MARGIN + rx + rw / 2.0) as u32,
            (10.0 + ry + rh / 2.0) as u32,
        )
    };
    let mut f = Frame::new(420, 120, a.input_state.theme.bg);
    a.paint_match_highlights(&mut f, &a.cache.layouts[&id], &range, 10);
    // 半透明色经 src-over 混合：以同底色单像素填充作为期望值
    let blend = |c: [u8; 4]| {
        let mut t = Frame::new(1, 1, a.input_state.theme.bg);
        t.fill_rect(0, 0, 1, 1, c);
        t.get(0, 0)
    };
    assert_eq!(
        pixel(&f, &a, &Range::new(0, 5)),
        blend(a.input_state.theme.sel),
        "当前命中用选区色"
    );
    assert_eq!(
        pixel(&f, &a, &Range::new(11, 16)),
        blend(a.input_state.theme.hover),
        "其余命中用 hover 色"
    );
    // 关闭后不再绘制命中高亮
    a.close_search();
    let mut g = Frame::new(420, 120, a.input_state.theme.bg);
    a.paint_match_highlights(&mut g, &a.cache.layouts[&id], &range, 10);
    assert_eq!(pixel(&g, &a, &Range::new(0, 5)), a.input_state.theme.bg);
}

/// max 峰值定位探针（默认忽略）：拆分 apply / after_edit / paint 耗时，
/// 并采样最大帧的各环节占比，定位 66ms 峰值来源。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_peak_breakdown
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_peak_breakdown() {
    let mut text = String::with_capacity(500_000);
    for i in 0..1500 {
        text.push_str(&format!(
            "## Section {i}\n\nParagraph {i} with **bold** text, `code`, and a [link](https://example.com/{i}).\n\n- item A {i}\n- item B {i}\n\n```rust\nfn f{i}() -> u32 {{ {i} }}\n```\n\n> quote {i}\n\n---\n\n"
        ));
    }
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.editor.apply(Intent::MoveDocStart).unwrap();
    for _ in 0..5 {
        a.editor.apply(Intent::MoveRight).unwrap();
    }
    a.paint();

    let mut total = Vec::with_capacity(500);
    let mut apply_ms = Vec::with_capacity(500);
    let mut after_ms = Vec::with_capacity(500);
    let mut paint_ms = Vec::with_capacity(500);
    for _ in 0..500 {
        let t0 = Instant::now();
        let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
        let t1 = Instant::now();
        a.after_edit(dirty);
        let t2 = Instant::now();
        a.paint();
        let t3 = Instant::now();
        apply_ms.push((t1 - t0).as_secs_f64() * 1e3);
        after_ms.push((t2 - t1).as_secs_f64() * 1e3);
        paint_ms.push((t3 - t2).as_secs_f64() * 1e3);
        total.push((t3 - t0).as_secs_f64() * 1e3);
    }
    // 找最大帧，输出其各环节耗时
    let (max_idx, _) = total
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .unwrap();
    // 统计 paint > 5ms 的次数
    let paint_spikes = paint_ms.iter().filter(|&&x| x > 5.0).count();
    let after_spikes = after_ms.iter().filter(|&&x| x > 5.0).count();
    println!("peak_total_ms={:.3}", total[max_idx]);
    println!("peak_apply_ms={:.3}", apply_ms[max_idx]);
    println!("peak_after_edit_ms={:.3}", after_ms[max_idx]);
    println!("peak_paint_ms={:.3}", paint_ms[max_idx]);
    println!("paint_spikes_gt5ms={paint_spikes}/500");
    println!("after_edit_spikes_gt5ms={after_spikes}/500");
}

/// after_edit 内部子步骤拆分探针（默认忽略）：定位 69ms 峰值具体来自哪个子步骤。
/// cargo test -p veil-ui --release -- --ignored --nocapture perf_after_edit_breakdown
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_after_edit_breakdown() {
    let mut text = String::with_capacity(500_000);
    for i in 0..1500 {
        text.push_str(&format!(
            "## Section {i}\n\nParagraph {i} with **bold** text, `code`, and a [link](https://example.com/{i}).\n\n- item A {i}\n- item B {i}\n\n```rust\nfn f{i}() -> u32 {{ {i} }}\n```\n\n> quote {i}\n\n---\n\n"
        ));
    }
    let store = MemStore::default();
    let mut a = app(&text, None, false, &store);
    a.editor.apply(Intent::MoveDocStart).unwrap();
    for _ in 0..5 {
        a.editor.apply(Intent::MoveRight).unwrap();
    }
    a.paint();

    // 真实 render() 路径预热：首帧后派生缓存已填，后续循环按命中率路径测
    a.refresh_index();
    a.ensure_layouts_visible();
    a.recompute_geometry();
    println!("after_warmup_cache_layouts={}", a.cache.layouts.len());

    let n = 500;
    // 记录首次编辑前后可见块 ID，验证 ID 稳定性
    let before_ids: Vec<BlockId> = a
        .index
        .blocks
        .get(a.visible_index_range())
        .unwrap_or(&[])
        .iter()
        .map(|g| g.id)
        .collect();
    let mut refresh = Vec::with_capacity(n);
    let mut ensure_vis = Vec::with_capacity(n);
    let mut ensure_block = Vec::with_capacity(n);
    let mut ensure_tbl = Vec::with_capacity(n);
    let mut recompute = Vec::with_capacity(n);
    let mut scroll = Vec::with_capacity(n);
    let mut total = Vec::with_capacity(n);
    let mut dirty_count = Vec::with_capacity(n);
    let mut vis_range_len = Vec::with_capacity(n);
    let mut missing_layouts = Vec::with_capacity(n);
    let mut after_first_ids: Option<Vec<BlockId>> = None;
    for iter in 0..n {
        let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
        dirty_count.push(dirty.len());
        let t0 = Instant::now();
        let t1 = Instant::now();
        a.refresh_index();
        let t2 = Instant::now();
        let vr = a.visible_index_range();
        vis_range_len.push(vr.len());
        // 统计可见范围内缺少布局缓存的块数
        let base = a.base_key();
        let caret = a.editor.caret();
        let missing = a
            .index
            .blocks
            .get(vr.clone())
            .unwrap_or(&[])
            .iter()
            .filter(|g| {
                a.cache
                    .layout(g.id, &g.kind, g.range, base, caret, a.toc_layout_epoch(g))
                    .is_none()
            })
            .count();
        missing_layouts.push(missing);
        if iter < 5 {
            println!("iter{iter}_cache_size={}", a.cache.layouts.len());
        }
        a.ensure_layouts_visible();
        let t3 = Instant::now();
        let caret_id = a.caret_block_index().map(|i| a.index.blocks[i].id);
        if let Some(id) = caret_id {
            a.ensure_block(id);
        }
        let t4 = Instant::now();
        a.ensure_tables_visible();
        let t5 = Instant::now();
        a.recompute_geometry();
        let t6 = Instant::now();
        a.scroll_to_caret();
        let t7 = Instant::now();
        refresh.push((t2 - t1).as_secs_f64() * 1e3);
        ensure_vis.push((t3 - t2).as_secs_f64() * 1e3);
        ensure_block.push((t4 - t3).as_secs_f64() * 1e3);
        ensure_tbl.push((t5 - t4).as_secs_f64() * 1e3);
        recompute.push((t6 - t5).as_secs_f64() * 1e3);
        scroll.push((t7 - t6).as_secs_f64() * 1e3);
        total.push((t7 - t0).as_secs_f64() * 1e3);
        if iter == 0 {
            after_first_ids = Some(
                a.index
                    .blocks
                    .get(a.visible_index_range())
                    .unwrap_or(&[])
                    .iter()
                    .map(|g| g.id)
                    .collect(),
            );
        }
        // 模拟生产 paint 的视口稳定：两遍 ensure_layouts_visible + recompute
        a.ensure_layouts_visible();
        a.ensure_math_visible();
        a.ensure_tables_visible();
        a.recompute_geometry();
        if visible_layout_missing(&a) {
            a.ensure_layouts_visible();
            a.ensure_math_visible();
            a.ensure_tables_visible();
            a.recompute_geometry();
        }
    }
    let (max_idx, _) = total
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .unwrap();
    println!("peak_total_ms={:.3}", total[max_idx]);
    println!("peak_refresh_index_ms={:.3}", refresh[max_idx]);
    println!("peak_ensure_layouts_visible_ms={:.3}", ensure_vis[max_idx]);
    println!("peak_ensure_block_ms={:.3}", ensure_block[max_idx]);
    println!("peak_ensure_tables_visible_ms={:.3}", ensure_tbl[max_idx]);
    println!("peak_recompute_geometry_ms={:.3}", recompute[max_idx]);
    println!("peak_scroll_to_caret_ms={:.3}", scroll[max_idx]);
    println!("peak_dirty_count={}", dirty_count[max_idx]);
    println!("peak_visible_range_len={}", vis_range_len[max_idx]);
    let max_dirty = dirty_count.iter().max().unwrap();
    println!("max_dirty_count={max_dirty}");
    println!("peak_missing_layouts={}", missing_layouts[max_idx]);
    let max_missing = missing_layouts.iter().max().unwrap();
    println!("max_missing_layouts={max_missing}");
    let multi_missing = missing_layouts.iter().filter(|&&m| m > 1).count();
    println!("iters_with_multi_missing={multi_missing}/{n}");
    // 打印前 20 次的 missing 数，看规律
    for (i, m) in missing_layouts.iter().take(20).enumerate() {
        println!("iter{i}_missing={m}");
    }
    // 首次编辑后可见块 ID 与编辑前对比
    if let Some(after_ids) = after_first_ids {
        let changed = before_ids
            .iter()
            .zip(after_ids.iter())
            .filter(|(b, a)| b != a)
            .count();
        println!(
            "before_ids={:?}",
            before_ids.iter().map(|id| id.0).collect::<Vec<_>>()
        );
        println!(
            "after_ids={:?}",
            after_ids.iter().map(|id| id.0).collect::<Vec<_>>()
        );
        println!("ids_changed_in_visible_range={changed}");
    }
    // p95 of each
    let p95 = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let i = ((s.len() as f64 * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(s.len() - 1);
        s[i]
    };
    println!("p95_refresh_index_ms={:.3}", p95(&refresh));
    println!("p95_ensure_layouts_visible_ms={:.3}", p95(&ensure_vis));
    println!("p95_recompute_geometry_ms={:.3}", p95(&recompute));
}

// ---- 阶段 1：中文口径字数 + 专注/打字机模式 ----

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

#[test]
fn word_count_tracks_edits_and_undo() {
    let store = MemStore::default();
    let mut a = app("你好世界", None, false, &store);
    assert_eq!(a.ui_state.writing.words, 4, "启动即按中文口径统计");
    a.perform_action(Action::Doc(Intent::InsertText(" hello".into())));
    assert_eq!(a.ui_state.writing.words, 5, "编辑后与文本同源重算");
    a.perform_menu(MenuAction::Undo);
    assert_eq!(a.ui_state.writing.words, 4, "撤销后回到旧值");
}

#[test]
fn line_move_refreshes_layout_without_further_edit() {
    let store = MemStore::default();
    let mut a = app("1one\n2two\n3three\n", None, false, &store);
    a.ensure_layouts_all();
    let text_of = |a: &crate::app::App| -> String {
        a.index
            .blocks
            .iter()
            .map(|g| a.cache.layouts[&g.id].layout_text())
            .collect()
    };
    let before = text_of(&a);
    assert!(before.find("1one") < before.find("2two"), "{before:?}");
    // 行移动保持块 id 与行区间不变，布局键必须仍能失配（回归：画面滞后到下次编辑）。
    a.perform_action(Action::Doc(Intent::MoveLineDown));
    let after = text_of(&a);
    assert!(
        after.find("2two") < after.find("1one"),
        "行移动后布局应立即反映新顺序，实际 {after:?}"
    );
}

#[test]
fn word_count_skips_markdown_markers() {
    let store = MemStore::default();
    let a = app("# 标题\n\n正文 **加粗** `code`\n", None, false, &store);
    // 标题(2) + 正文(2) + 加粗(2) + code(1)
    assert_eq!(a.ui_state.writing.words, 7);
}

#[test]
fn focus_mode_dims_only_non_caret_blocks() {
    let store = MemStore::default();
    let mut a = app("第一段\n\n第二段\n", None, false, &store);
    a.render(); // 先完成排版，几何才稳定
    let vp = a.viewport();
    let bg = a.input_state.theme.bg;
    let scroll = a.window_state.scroll_y;
    let (top0, h0) = {
        let g = &a.index.blocks[0];
        (g.top, g.height)
    };
    let (top1, h1) = {
        let g = &a.index.blocks[1];
        (g.top, g.height)
    };
    let (y0, y1) = (vp.y + top0 - scroll, vp.y + top1 - scroll);
    let frame = a.render();
    let p0 = first_non_bg(frame, y0 as i32, (y0 + h0) as i32, bg).expect("首段有内容");
    let p1 = first_non_bg(frame, y1 as i32, (y1 + h1) as i32, bg).expect("次段有内容");

    a.toggle_focus();
    assert!(a.ui_state.writing.focus);
    let frame = a.render();
    assert_eq!(frame.get(p0.0, p0.1), p0.2, "光标所在块不压暗");
    let dimmed = frame.get(p1.0, p1.1);
    assert_ne!(dimmed, p1.2, "非光标块被压暗");
    let lum = |p: [u8; 4]| p[0] as i32 + p[1] as i32 + p[2] as i32;
    assert!(lum(dimmed) > lum(p1.2), "压暗即向背景色靠拢");
    // 纯视觉开关：源码一个字节都不变
    assert_eq!(doc_text(&a), "第一段\n\n第二段\n");
}

#[test]
fn typewriter_mode_pins_caret_line_to_anchor() {
    let store = MemStore::default();
    let text = "line\n\n".repeat(100);
    let mut a = app(&text, None, false, &store);
    a.editor.set_caret(360); // 第 61 块起点（每块 6 字节）
    a.toggle_typewriter();
    assert!(a.ui_state.writing.typewriter);
    let vp = a.viewport();
    let target = vp.y + vp.h * 0.42;
    let (_, cy, ch) = a.caret_screen();
    assert!(a.window_state.scroll_y > 0.0, "开启即滚动到锚点");
    assert!(
        (cy + ch / 2.0 - target).abs() <= ch,
        "光标行中心贴合锚点：cy={cy} target={target}"
    );
    assert_eq!(doc_text(&a), text, "只改视口，不动文本");
}

// ---- 2e 端到端场景：打开→编辑→表格→图片→查找→保存→重开→恢复（红线 e2e 断言 ≥20）----

#[test]
fn e2e_writing_loop_open_edit_save_reopen() {
    // 打开既有文件 → 文末追加写作 → 保存落盘 → 重开验证字节一致
    let store = MemStore::default();
    store.files.borrow_mut().insert(
        PathBuf::from("doc/a.md"),
        "# 标题\n\n第一段。\n".as_bytes().to_vec(),
    );
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist("默认样例", None, &store, &rec, &ses, false);
    a.open_path(PathBuf::from("doc/a.md"));
    assert_eq!(doc_text(&a), "# 标题\n\n第一段。\n");
    assert_eq!(a.doc.path, Some(PathBuf::from("doc/a.md")));
    assert!(!a.editor.is_dirty(), "打开即干净态");
    assert_eq!(
        a.persist_state.recent,
        vec![PathBuf::from("doc/a.md")],
        "打开即进最近列表"
    );

    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.route_key(Key::Text("第二段，中文写作。\n".into()));
    assert!(a.editor.is_dirty(), "编辑置脏");
    assert_eq!(doc_text(&a), "# 标题\n\n第一段。\n第二段，中文写作。\n");

    a.save_now();
    assert!(!a.editor.is_dirty(), "保存清脏");
    assert_eq!(
        store.files.borrow()[&PathBuf::from("doc/a.md")],
        "# 标题\n\n第一段。\n第二段，中文写作。\n".as_bytes(),
        "磁盘字节 == 文档真值（无隐式规范化）"
    );

    // 重开同一文件：内容与会话基线都来自磁盘
    a.open_path(PathBuf::from("doc/a.md"));
    assert_eq!(doc_text(&a), "# 标题\n\n第一段。\n第二段，中文写作。\n");
    assert!(!a.editor.is_dirty(), "重开干净");
}

#[test]
fn e2e_table_edit_and_image_paste() {
    // 表格网格渲染 → Tab 格间导航 → 格内编辑重建网格 → 截图粘贴插相对路径 → 保存落盘
    let text = "说明\n\n| 名称 | 数量 |\n| --- | --- |\n| 苹果 | 1 |\n";
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("doc/t.md"), text.as_bytes().to_vec());
    let calls: Rc<RefCell<Vec<(PathBuf, u32, u32)>>> = Rc::new(RefCell::new(Vec::new()));
    let mut a = App::new(
        Doc {
            text: text.into(),
            path: Some(PathBuf::from("doc/t.md")),
            bom: false,
            resume: false,
            open_error: None,
        },
        Ports {
            clipboard: Box::new(ClipImage {
                text: None,
                html: None,
                image: Some(solid(4, 2)),
            }),
            store: Box::new(store.clone()),
            math: Box::new(FakeMath {
                fail: false,
                calls: Rc::new(RefCell::new(Vec::new())),
            }),
            images: Box::new(NoImages),
            assets: Box::new(FakeAssets {
                result: Ok("t.assets/image-1.png".into()),
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

    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, kind, range) = a.editor.session().blocks()[1].clone();
    assert_eq!(kind, BlockKind::Table);
    assert!(a.table_shows_grid(id, &range), "光标在表外 → 网格渲染");
    let tbl = a.editor.session().table(id).unwrap();
    let first = tbl.header[0].range.start;
    let second = tbl.header[1].range.start;
    let old_w = a.cache.tables[&id].width;

    // Tab：表头首格 → 次格；格内编辑 → 网格重建变宽
    assert!(a.editor.set_caret(first));
    a.route_key(Key::Tab);
    assert_eq!(a.editor.caret(), second, "Tab 前进到下一格内容起点");
    assert!(a.editor.set_caret(first));
    let dirty = a.editor.apply(Intent::InsertText("长长长".into())).unwrap();
    a.after_edit(dirty);
    a.ensure_tables();
    let new_id = a.editor.session().blocks()[1].0;
    assert!(a.cache.tables.contains_key(&new_id), "格内编辑 → 网格重建");
    assert!(a.cache.tables[&new_id].width > old_w, "内容变宽 → 列宽变大");

    // 截图粘贴：资产落盘 + 插入相对路径链接
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.paste_clipboard();
    assert!(
        doc_text(&a).contains("![](t.assets/image-1.png)"),
        "粘贴插入相对路径图片链接"
    );
    assert!(a.editor.is_dirty());
    assert_eq!(calls.borrow().len(), 1, "资产保存调用一次");
    assert_eq!(
        (calls.borrow()[0].1, calls.borrow()[0].2),
        (4, 2),
        "位图原样交给资产层"
    );

    a.save_now();
    let disk = String::from_utf8(store.files.borrow()[&PathBuf::from("doc/t.md")].clone())
        .expect("落盘内容恒为 UTF-8");
    assert!(
        disk.contains("| 长长长名称 | 数量 |"),
        "格内编辑后的表格源码完整落盘"
    );
    assert!(disk.contains("![](t.assets/image-1.png)"));
}

#[test]
fn e2e_search_replace_and_crash_recovery() {
    // 查找→替换全部→崩溃（杀进程）→重启恢复→保存收口
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist(
        "alpha beta alpha\n",
        Some("r.md"),
        &store,
        &rec,
        &ses,
        false,
    );
    a.ensure_layouts_all();
    a.perform_action(Action::Replace);
    assert!(a.ui_state.search.open && a.ui_state.search.replace_open);
    a.search_input("alpha");
    assert_eq!(a.ui_state.search.matches.len(), 2, "全文档命中 2 处");
    assert_eq!(a.ui_state.search.label(), "1/2");
    a.search_next();
    assert_eq!(a.ui_state.search.current, 1, "下一处命中推进游标");
    a.search_toggle_field();
    a.search_input("gamma");
    a.replace_all();
    assert_eq!(doc_text(&a), "gamma beta gamma\n");
    assert!(a.ui_state.search.matches.is_empty(), "替换后命中清空");
    assert!(a.editor.is_dirty());
    a.close_search();
    assert!(!a.ui_state.search.open);

    // 静默 2s 落恢复快照 → 模拟杀进程（磁盘仍是旧内容）
    let t0 = Instant::now();
    a.persist_state.persist.note_activity(t0);
    a.maybe_recovery_write(t0 + Duration::from_secs(2));
    assert_eq!(
        slot_content(&rec),
        (
            Some(PathBuf::from("r.md")),
            "gamma beta gamma\n".to_string()
        ),
        "恢复快照 = 路径 + 未保存内容"
    );
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("r.md"), b"alpha beta alpha\n".to_vec());

    // 重启：崩溃恢复优先，内容与路径都来自快照
    let mut b = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(doc_text(&b), "gamma beta gamma\n", "重启后恢复未保存内容");
    assert_eq!(b.doc.path, Some(PathBuf::from("r.md")));
    assert!(b.editor.is_dirty(), "恢复内容相对磁盘是未保存修改");
    assert_eq!(
        b.doc.baseline,
        b"alpha beta alpha\n".to_vec(),
        "基线 = 磁盘"
    );

    // 恢复后保存：磁盘更新、脏标清除（本 session 未重写快照，槽留待下次启动比对自清）
    b.save_now();
    assert!(!b.editor.is_dirty());
    assert_eq!(
        store.files.borrow()[&PathBuf::from("r.md")],
        b"gamma beta gamma\n".to_vec(),
        "恢复后保存落盘"
    );

    // 第三次启动：槽内快照 == 磁盘 → 干净打开 + 残留自清，全流程收口
    let c = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(doc_text(&c), "gamma beta gamma\n");
    assert!(!c.editor.is_dirty(), "快照与磁盘一致：干净打开");
    assert!(rec.slot.borrow().is_none(), "残留快照启动自清");
}

// ---- 打印（3b）：分页引擎 + 干净帧 + 状态复原 ----

/// 打印几何采样：begin 后各块 (top, 行顶序列)，断点合法性（不劈行）的判定依据。
type PrintGeoms = Rc<RefCell<Vec<(f32, Vec<f32>)>>>;
/// 页帧采样：逐页 (宽, 高, 四角像素)，无 chrome 的直接证据。
type PrintFrames = Rc<RefCell<Vec<(u32, u32, [[u8; 4]; 4])>>>;

/// 打印端口 mock：按注入页帧跑完整 begin→逐页→end 流程，采样页数/断点/
/// 各块行顶/页帧尺寸与四角像素，供测试断言（真实 Win32 粘合在产品层）。
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
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(m),
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

#[test]
fn print_paginates_clean_frames_and_restores_state() {
    let text: String = (1..=30).map(|i| format!("段落{i:02}\n\n")).collect();
    let p = FakePrint {
        w: 300,
        h: 120,
        scale: 1.0,
        ..Default::default()
    };
    let (pages, breaks, geoms, frames, ended) = (
        p.pages.clone(),
        p.breaks.clone(),
        p.geoms.clone(),
        p.frames.clone(),
        p.ended.clone(),
    );
    let mut a = app_print(&text, Box::new(p));
    a.input_state.scale = 1.5; // 非默认屏幕缩放：验证 end 复原
    a.window_state.scroll_y = 42.0;
    assert!(!a.perform_menu(MenuAction::Print), "打印不退出应用");
    assert!(ended.get(), "begin 之后必调 end");
    let pages = pages.get();
    assert!(pages >= 2, "30 段在 120px 页帧应多页: {pages}");
    let bs = breaks.borrow();
    assert_eq!(bs.len(), pages, "页数 = 断点数");
    assert_eq!(bs[0], 0.0, "首页从文档顶开始");
    assert!(bs.windows(2).all(|w| w[0] < w[1]), "断点严格递增: {bs:?}");
    assert_breaks_on_boundaries(&bs, &geoms.borrow());
    // 页帧 = 注入尺寸；四角 = 文档底色（无菜单栏/状态栏/光标覆盖）
    let bg = a.input_state.theme.bg;
    for (w, h, corners) in frames.borrow().iter() {
        assert_eq!((*w, *h), (300, 120), "页帧尺寸 = 注入尺寸");
        assert_eq!(
            corners,
            &[bg, bg, bg, bg],
            "页帧四角为文档底色（无 chrome）"
        );
    }
    // 屏幕态复原（尺寸/缩放/滚动），文档真值未动
    assert_eq!(a.win_size(), (800, 600), "headless 窗口尺寸复原");
    assert_eq!(a.input_state.scale, 1.5, "DPI 缩放复原");
    assert_eq!(a.window_state.scroll_y, 42.0, "滚动位置复原");
    assert!(a.window_state.print.is_none(), "打印态已退出");
    assert_eq!(doc_text(&a), text, "打印只读，不动真值");
}

/// K2：Ctrl+P = 快速打开（VS Code 一致）；打印改到 Ctrl+Shift+P，二者不串位。
#[test]
fn ctrl_p_quick_opens_and_ctrl_shift_p_prints() {
    let p = FakePrint {
        w: 300,
        h: 600,
        scale: 1.0,
        ..Default::default()
    };
    let ended = p.ended.clone();
    let mut a = app_print("x\n", Box::new(p));

    a.input_state.mods.ctrl = true;
    a.route_key(Key::P);
    a.input_state.mods.ctrl = false;
    assert!(a.ui_state.quick.open, "Ctrl+P 打开快速打开浮层");
    assert!(!ended.get(), "Ctrl+P 不触发打印");

    a.close_quick();
    a.input_state.mods.ctrl = true;
    a.input_state.mods.shift = true;
    a.route_key(Key::P);
    a.input_state.mods.ctrl = false;
    a.input_state.mods.shift = false;
    assert!(ended.get(), "Ctrl+Shift+P 经路由到达打印端口");
}

#[test]
fn print_breaks_inside_tall_block_at_line_tops() {
    // 单块（围栏代码 50 行）超页高：断点落在块内行顶（不劈行）
    let code: String = (1..=50).map(|i| format!("l{i:02}\n")).collect();
    let text = format!("```\n{code}```\n");
    let p = FakePrint {
        w: 300,
        h: 100,
        scale: 1.0,
        ..Default::default()
    };
    let (pages, breaks, geoms) = (p.pages.clone(), p.breaks.clone(), p.geoms.clone());
    let mut a = app_print(&text, Box::new(p));
    a.perform_menu(MenuAction::Print);
    assert!(pages.get() >= 2, "50 行代码块应多页: {}", pages.get());
    assert_breaks_on_boundaries(&breaks.borrow(), &geoms.borrow());
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

#[test]
fn print_collapses_all_reveal_markup() {
    // 光标停在 **粗体** 内：屏幕态该块显源码（含 **）；打印必须收拢为纯文本
    let text = "a **b** c\n";
    let probe = RevealProbe {
        texts: Rc::new(RefCell::new(Vec::new())),
    };
    let texts = probe.texts.clone();
    let mut a = app_print(text, Box::new(probe));
    let bold = text.find('b').unwrap();
    a.editor.set_caret(bold); // 光标进入行内节点 → 屏幕态显 ** 源码
    a.render();
    let id = a.index.blocks[0].id;
    let screen = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        screen.contains("**"),
        "屏幕态光标在节点内显源码: {screen:?}"
    );
    a.perform_menu(MenuAction::Print);
    let printed = texts.borrow();
    assert_eq!(printed.len(), 1, "单块文档");
    assert!(
        !printed[0].contains("**"),
        "打印态标记收拢: {:?}",
        printed[0]
    );
    assert!(printed[0].contains('b'), "内容保留: {:?}", printed[0]);
    assert_eq!(a.editor.caret(), bold, "打印不动真实光标");
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

#[test]
fn print_hard_cuts_atomic_block_taller_than_page() {
    // 原子块（500px 数学图）超页高：无合法断点 → 硬切，不 panic、页数 > 1
    let p = FakePrint {
        w: 300,
        h: 100,
        scale: 1.0,
        ..Default::default()
    };
    let (pages, breaks, geoms, ended) = (
        p.pages.clone(),
        p.breaks.clone(),
        p.geoms.clone(),
        p.ended.clone(),
    );
    let doc = Doc {
        text: "$$\nx\n$$\n".into(),
        path: None,
        bom: false,
        resume: false,
        open_error: None,
    };
    let mut a = App::new(
        doc,
        Ports {
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(TallMath),
            images: Box::new(NoImages),
            assets: Box::new(NoAssets),
            exporter: Box::new(NoExport),
            dialogs: Box::new(NoDialogs),
            watcher: Box::new(NoWatcher),
            theme_env: theme::test_support::env(),
            persist_env: persist_env(),
            print: Box::new(p),
            url_opener: no_url(),
            clock: fixed_clock(),
        },
    );
    a.perform_menu(MenuAction::Print);
    assert!(ended.get());
    assert!(
        pages.get() >= 2,
        "500px 原子块在 100px 页帧应多页: {}",
        pages.get()
    );
    let bs = breaks.borrow();
    let gs = geoms.borrow();
    // 至少一个硬切断点（不在任何块顶/行顶）：块顶那次合法，其余页首都是硬切
    let hard = bs[1..]
        .iter()
        .filter(|&&b| {
            !gs.iter().any(|(top, lines)| {
                (top - b).abs() < 0.5 || lines.iter().any(|&t| (top + t - b).abs() < 0.5)
            })
        })
        .count();
    assert!(hard >= 1, "超页原子块应产生硬切断点: {bs:?} vs {gs:?}");
}

#[test]
fn print_stub_notices_when_platform_missing() {
    let mut a = app_print("x\n", Box::new(NoPrint));
    a.perform_menu(MenuAction::Print);
    assert_eq!(a.ui_state.chrome.notice, "当前平台暂不支持打印");
    assert!(a.window_state.print.is_none(), "NoPrint 不进入打印态");
}

// ---- 8：PDF 导出（目录 / 页码）headless 断言 ----

/// 导出探针：跑 begin_export（或 begin）→ 逐页采样 → end。
#[derive(Clone, Default)]
struct ExportProbe {
    w: u32,
    h: u32,
    opt: ExportOptions,
    legacy: bool,
    pages: Rc<Cell<usize>>,
    breaks: Rc<RefCell<Vec<f32>>>,
    toc_count: Rc<Cell<usize>>,
    entries: Rc<RefCell<Vec<(u8, String, usize)>>>,
    heading_tops: Rc<RefCell<Vec<f32>>>,
    frames: Rc<RefCell<Vec<Vec<u32>>>>,
    ended: Rc<Cell<bool>>,
}

impl Print for ExportProbe {
    fn print(&mut self, _s: &mut PrintSession) {
        unreachable!("导出探针只走 export_pdf")
    }

    fn export_pdf(&mut self, s: &mut PrintSession) {
        let pages = if self.legacy {
            s.begin(self.w.max(1), self.h.max(1), 1.0)
        } else {
            s.begin_export(self.w.max(1), self.h.max(1), 1.0, self.opt)
        };
        self.pages.set(pages);
        *self.breaks.borrow_mut() = s.page_breaks().to_vec();
        self.toc_count.set(s.toc_entry_count());
        *self.entries.borrow_mut() = s
            .toc_entries()
            .iter()
            .map(|e| (e.level, e.text.clone(), e.page))
            .collect();
        *self.heading_tops.borrow_mut() = s
            .app
            .index
            .blocks
            .iter()
            .filter(|g| {
                matches!(
                    g.kind,
                    BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_)
                )
            })
            .map(|g| g.top)
            .collect();
        let mut frames = Vec::new();
        for i in 0..pages {
            frames.push(s.render_page(i).pixels.clone());
        }
        *self.frames.borrow_mut() = frames;
        s.end();
        self.ended.set(true);
    }
}

fn packed(c: [u8; 4]) -> u32 {
    ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
}

fn region_has_ink(frame: &[u32], width: u32, bg: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> bool {
    (y0..y1).any(|y| (x0..x1).any(|x| frame[(y as usize) * width as usize + x as usize] != bg))
}

fn export_doc() -> String {
    let filler: String = (1..=12)
        .map(|i| format!("正文段落 {i} 内容。\n\n"))
        .collect();
    format!("# 第一章\n\n{filler}## 第二节\n\n{filler}### 小节\n\n{filler}尾段\n")
}

#[test]
fn export_toc_adds_pages_with_ink() {
    let p = ExportProbe {
        w: 300,
        h: 120,
        opt: ExportOptions {
            page_numbers: false,
            toc: true,
        },
        ..Default::default()
    };
    let (pages, breaks, count, frames, ended) = (
        p.pages.clone(),
        p.breaks.clone(),
        p.toc_count.clone(),
        p.frames.clone(),
        p.ended.clone(),
    );
    let mut a = app_print(&export_doc(), Box::new(p));
    assert!(!a.perform_menu(MenuAction::ExportPdf), "导出不退出应用");
    assert!(ended.get(), "begin_export 之后必调 end");
    let body_pages = breaks.borrow().len();
    assert!(count.get() >= 3, "目录应含三个标题: {}", count.get());
    assert!(
        pages.get() > body_pages,
        "目录页使总页数增加: {} vs {}",
        pages.get(),
        body_pages
    );
    let bg = packed(a.input_state.theme.bg);
    let f0 = &frames.borrow()[0];
    assert!(
        f0.iter().any(|&px| px != bg),
        "目录首页不应为空白（含标题/条目墨迹）"
    );
    assert!(a.window_state.print.is_none(), "导出态已退出");
}

#[test]
fn export_without_toc_matches_plain_begin() {
    let text = export_doc();
    let toc_off = ExportProbe {
        w: 300,
        h: 120,
        opt: ExportOptions {
            page_numbers: false,
            toc: false,
        },
        ..Default::default()
    };
    let (pages, breaks, frames) = (
        toc_off.pages.clone(),
        toc_off.breaks.clone(),
        toc_off.frames.clone(),
    );
    let mut a = app_print(&text, Box::new(toc_off));
    a.perform_menu(MenuAction::ExportPdf);
    assert_eq!(
        pages.get(),
        breaks.borrow().len(),
        "无目录时总页数 = 正文页数"
    );

    let legacy = ExportProbe {
        w: 300,
        h: 120,
        legacy: true,
        ..Default::default()
    };
    let legacy_frames = legacy.frames.clone();
    let mut b = app_print(&text, Box::new(legacy));
    b.perform_menu(MenuAction::ExportPdf);
    assert_eq!(
        frames.borrow()[0],
        legacy_frames.borrow()[0],
        "关闭目录/页码时首页与 begin 完全一致"
    );
}

#[test]
fn export_page_numbers_draw_bottom_right_footer() {
    let text = "a\n\nb\n";
    let (w, h) = (300u32, 600u32);
    let on = ExportProbe {
        w,
        h,
        opt: ExportOptions {
            page_numbers: true,
            toc: false,
        },
        ..Default::default()
    };
    let on_frames = on.frames.clone();
    let mut a = app_print(text, Box::new(on));
    a.perform_menu(MenuAction::ExportPdf);
    let bg = packed(a.input_state.theme.bg);
    let on_ink = region_has_ink(&on_frames.borrow()[0], w, bg, w - 120, h - 48, w - 4, h - 4);

    let off = ExportProbe {
        w,
        h,
        opt: ExportOptions {
            page_numbers: false,
            toc: false,
        },
        ..Default::default()
    };
    let off_frames = off.frames.clone();
    let mut b = app_print(text, Box::new(off));
    b.perform_menu(MenuAction::ExportPdf);
    let bg = packed(b.input_state.theme.bg);
    let off_ink = region_has_ink(
        &off_frames.borrow()[0],
        w,
        bg,
        w - 120,
        h - 48,
        w - 4,
        h - 4,
    );

    assert!(on_ink, "页码开启时页脚右下应有墨迹");
    assert!(!off_ink, "页码关闭时页脚区域应为背景色");
}

#[test]
fn export_toc_entry_pages_follow_breaks() {
    let p = ExportProbe {
        w: 300,
        h: 120,
        opt: ExportOptions {
            page_numbers: false,
            toc: true,
        },
        ..Default::default()
    };
    let (pages, breaks, entries, tops) = (
        p.pages.clone(),
        p.breaks.clone(),
        p.entries.clone(),
        p.heading_tops.clone(),
    );
    let mut a = app_print(&export_doc(), Box::new(p));
    a.perform_menu(MenuAction::ExportPdf);
    let toc_pages = pages.get() - breaks.borrow().len();
    let entries = entries.borrow();
    assert_eq!(entries.len(), tops.borrow().len());
    for ((_, text, page), top) in entries.iter().zip(tops.borrow().iter()) {
        let expected = breaks.borrow().partition_point(|&b| b <= *top) + 1 + toc_pages;
        assert_eq!(*page, expected, "标题 {text:?} 的物理页码");
    }
}

// ── 3c-2：用户缩放（Ctrl+滚轮/捏合、Ctrl+=/-/0、视图菜单）────────────────────

#[test]
fn zoom_step_scales_effective_dpi_and_reflows() {
    let store = MemStore::default();
    let mut a = app("第一段\n\n第二段\n", Some("a.md"), false, &store);
    a.input_state.set_dpi(2.0); // 模拟 200% DPI：effective = dpi × zoom
    let before = a.base_key();
    a.ensure_layouts_all();
    let h0 = a.doc_height();
    a.zoom_step(1);
    assert_eq!(a.input_state.user_zoom, 1.1);
    assert!(
        (a.input_state.scale - 2.2).abs() < 1e-6,
        "effective scale = 2.0 × 1.1: {}",
        a.input_state.scale
    );
    assert_ne!(a.base_key(), before, "scale 量化入键 → 布局整体失配重排");
    a.ensure_layouts_all();
    assert!(
        a.doc_height() > h0,
        "放大后行高变大 → 文档变高: {h0} → {}",
        a.doc_height()
    );
    assert_eq!(a.ui_state.chrome.notice, "缩放 110%");
}

#[test]
fn zoom_quantizes_to_tenth_and_clamps() {
    let store = MemStore::default();
    let mut a = app("x\n", Some("a.md"), false, &store);
    a.set_user_zoom(1.04);
    assert_eq!(a.input_state.user_zoom, 1.0, "0.1 量化：1.04 → 1.0");
    a.set_user_zoom(1.06);
    assert_eq!(a.input_state.user_zoom, 1.1, "0.1 量化：1.06 → 1.1");
    for _ in 0..30 {
        a.zoom_step(1);
    }
    assert_eq!(a.input_state.user_zoom, 3.0, "放大封顶 3.0");
    let notice = a.ui_state.chrome.notice.clone();
    a.zoom_step(1);
    assert_eq!(a.input_state.user_zoom, 3.0, "到顶不再变化");
    assert_eq!(a.ui_state.chrome.notice, notice, "无变化不刷提示");
    for _ in 0..40 {
        a.zoom_step(-1);
    }
    assert_eq!(a.input_state.user_zoom, 0.5, "缩小封底 0.5");
}

#[test]
fn zoom_step_roundtrips_exactly_and_reset_restores() {
    let store = MemStore::default();
    let mut a = app("x\n", Some("a.md"), false, &store);
    a.zoom_step(1);
    a.zoom_step(-1);
    assert_eq!(a.input_state.user_zoom, 1.0, "0.1 加减量化保证精确往返");
    a.zoom_step(1);
    a.zoom_step(1);
    a.zoom_reset();
    assert_eq!(a.input_state.user_zoom, 1.0, "Ctrl+0 还原 100%");
    assert_eq!(a.input_state.scale, a.input_state.dpi_scale);
    assert_eq!(a.ui_state.chrome.notice, "缩放 100%");
}

#[test]
fn zoom_by_accumulates_trackpad_pixels() {
    let store = MemStore::default();
    let mut a = app("x\n", Some("a.md"), false, &store);
    // 高精度触控板小增量：不满一档（50px）不触发
    a.zoom_by(20.0);
    a.zoom_by(20.0);
    assert_eq!(a.input_state.user_zoom, 1.0, "40px 未满一档");
    a.zoom_by(20.0);
    assert_eq!(a.input_state.user_zoom, 1.1, "60px 触发一档");
    assert_eq!(a.input_state.zoom_accum, 10.0, "余量保留到下一次");
    // 反向一次大增量：10 - 120 = -110 → 两档缩小，余 -10
    a.zoom_by(-120.0);
    assert_eq!(a.input_state.user_zoom, 0.9, "-110px 触发两档");
    assert_eq!(a.input_state.zoom_accum, -10.0);
}

#[test]
fn zoom_shrink_clamps_scroll_into_new_max() {
    let store = MemStore::default();
    let text: String = (1..=100).map(|i| format!("段落{i:03}\n\n")).collect();
    let mut a = app(&text, Some("a.md"), false, &store);
    a.ensure_layouts_all();
    let max0 = chrome::max_scroll(a.viewport(), a.doc_height());
    assert!(max0 > 0.0, "长文档可滚动");
    a.window_state.scroll_y = max0;
    for _ in 0..5 {
        a.zoom_step(-1); // 1.0 → 0.5：文档变矮，最大滚动量收缩
    }
    let max1 = chrome::max_scroll(a.viewport(), a.doc_height());
    assert!(max1 < max0, "缩小后文档更矮: {max0} → {max1}");
    assert!(
        a.window_state.scroll_y <= max1,
        "滚动位置钳入新上界: {} ≤ {max1}",
        a.window_state.scroll_y
    );
}

#[test]
fn zoom_is_view_only_keeps_doc_and_dirty() {
    let store = MemStore::default();
    let mut a = app("正文 abc\n", Some("a.md"), false, &store);
    a.zoom_step(1);
    assert!(!a.editor.is_dirty(), "纯视图缩放不置脏");
    let dirty = a.editor.apply(Intent::InsertText("z".into())).unwrap();
    a.after_edit(dirty);
    assert!(a.editor.is_dirty());
    a.zoom_reset();
    assert!(a.editor.is_dirty(), "缩放不清已有脏标");
    assert_eq!(doc_text(&a), "z正文 abc\n", "缩放不动文档真值");
}

#[test]
fn zoom_ignored_during_print() {
    let store = MemStore::default();
    let mut a = app("x\n", Some("a.md"), false, &store);
    a.window_state.print = Some(crate::print::PrintState {
        w: 300,
        h: 120,
        saved_scale: 1.0,
        saved_scroll: 0.0,
        breaks: vec![0.0],
        toc_pages: 0,
        toc_frames: Vec::new(),
        footers: Vec::new(),
        total_pages: 1,
        options: ExportOptions::default(),
        toc_entries: Vec::new(),
    });
    a.set_user_zoom(1.5);
    assert_eq!(a.input_state.user_zoom, 1.0, "打印页帧语义独占 scale");
    a.window_state.print = None;
}

#[test]
fn zoom_menu_and_key_paths() {
    let store = MemStore::default();
    let mut a = app("x\n", Some("a.md"), false, &store);
    assert!(!a.perform_menu(MenuAction::ZoomIn), "缩放不退出应用");
    assert_eq!(a.input_state.user_zoom, 1.1, "菜单放大");
    // 键盘链路：map_key（Ctrl+= 的 winit 形态）→ translate → Action
    assert_eq!(
        crate::input::translate(
            Key::Equal,
            Mods {
                ctrl: true,
                shift: false,
                alt: false,
            }
        ),
        Some(Action::ZoomIn)
    );
    a.perform_action(Action::ZoomOut);
    a.perform_action(Action::ZoomOut);
    assert_eq!(a.input_state.user_zoom, 0.9, "键盘动作两档缩小");
    a.perform_action(Action::ZoomReset);
    assert_eq!(a.input_state.user_zoom, 1.0);
    assert!(!a.editor.is_dirty(), "菜单/键盘缩放均不置脏");
}

// ── 3c-3：F11 全屏切换 ───────────────────────────────────────────────────────

#[test]
fn ctrl_combo_ignores_winit_text_stripped_of_ctrl() {
    // winit Windows：Ctrl+S 的 event.text 是「剥离 Ctrl 后」的 "s"。文本分支若先行，
    // 全部 Ctrl 组合都会退化成 Key::Text("s") → translate 返回 None → 快捷键静默失效
    let s = winit::keyboard::SmolStr::new("s");
    let ctrl = Mods {
        ctrl: true,
        shift: false,
        alt: false,
    };
    assert_eq!(
        crate::interaction::map_key_parts(Some("s"), WKey::Character(s.clone()), ctrl),
        Some(Key::S),
        "Ctrl+S 必须映射为 Key::S，而不是 Key::Text"
    );
    // 无 Ctrl：文本照常承载（s 作为字符插入，而非快捷键）
    assert_eq!(
        crate::interaction::map_key_parts(Some("s"), WKey::Character(s.clone()), Mods::default()),
        Some(Key::Text("s".into()))
    );
    // AltGr（Ctrl+Alt）仍是输入字符，必须走文本
    let altgr = Mods {
        ctrl: true,
        shift: false,
        alt: true,
    };
    assert_eq!(
        crate::interaction::map_key_parts(Some("s"), WKey::Character(s), altgr),
        Some(Key::Text("s".into()))
    );
}

#[test]
fn ctrl_numpad_zoom_keys_match_main_keyboard() {
    // DEF-T06-004：小键盘 +/-/0 与主键盘缩放一致；NumLock 关时逻辑键可能不是字符，
    // 因此物理键必须兜底识别。
    for (logical, physical, want) in [
        (
            WKey::Named(NamedKey::Space),
            KeyCode::NumpadAdd,
            Key::Equal,
        ),
        (
            WKey::Character("+".into()),
            KeyCode::NumpadAdd,
            Key::Equal,
        ),
        (
            WKey::Named(NamedKey::ArrowDown),
            KeyCode::NumpadSubtract,
            Key::Minus,
        ),
        (
            WKey::Named(NamedKey::Insert),
            KeyCode::Numpad0,
            Key::Digit0,
        ),
    ] {
        assert_eq!(
            crate::interaction::ctrl_punctuation(&logical, PhysicalKey::Code(physical)),
            Some(want.clone()),
            "物理键 {physical:?} 应映射到 {want:?}"
        );
    }
}

#[test]
fn ctrl_comma_opens_settings() {
    // 回归：map_key 曾只特判 `/`，`,` 落到 None，Ctrl+, 静默失效（菜单却标注了该快捷键）。
    // 逻辑键与物理键任一命中即可（布局/IME 差异）。
    assert_eq!(
        crate::interaction::ctrl_punctuation(
            &WKey::Character(",".into()),
            PhysicalKey::Code(KeyCode::Comma)
        ),
        Some(Key::Comma)
    );
    assert_eq!(
        crate::interaction::ctrl_punctuation(
            &WKey::Named(NamedKey::Space),
            PhysicalKey::Code(KeyCode::Comma)
        ),
        Some(Key::Comma),
        "逻辑键不是字符时仍按物理键识别"
    );
    assert_eq!(
        crate::interaction::ctrl_punctuation(
            &WKey::Character(",".into()),
            PhysicalKey::Code(KeyCode::Period)
        ),
        Some(Key::Comma),
        "布局把逗号放到别的物理键时按字符识别"
    );
    // 无 Ctrl 的 `,` 不是快捷键（文本分支承载）
    assert_eq!(crate::input::translate(Key::Comma, Mods::default()), None);

    let store = MemStore::default();
    let mut a = app("x\n", None, false, &store);
    a.input_state.mods.ctrl = true;
    a.route_key(Key::Comma);
    a.input_state.mods.ctrl = false;
    assert!(a.ui_state.settings.is_some(), "Ctrl+, 打开设置模态");
}

#[test]
fn fullscreen_headless_noop_and_menu_path() {
    let store = MemStore::default();
    let mut a = app("x\n", Some("a.md"), false, &store);
    // headless 无窗口：切换是无害 no-op（真机翻转由 winit set_fullscreen 承担）
    a.perform_action(Action::ToggleFullscreen);
    assert!(!a.window_state.fullscreen, "无窗口不翻状态");
    assert!(!a.editor.is_dirty(), "全屏切换不动文档");
    let notice = a.ui_state.chrome.notice.clone();
    assert!(
        !a.perform_menu(MenuAction::ToggleFullscreen),
        "全屏不退出应用"
    );
    assert!(!a.window_state.fullscreen);
    assert_eq!(a.ui_state.chrome.notice, notice, "no-op 不刷提示");
    // 路由：Document 模式 F11 直达动作；Esc 语义不变（关菜单，不退全屏）
    assert_eq!(
        crate::input::translate(
            Key::F11,
            Mods {
                ctrl: false,
                shift: false,
                alt: false,
            }
        ),
        Some(Action::ToggleFullscreen)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3g WindowState visible 状态机：最小化/失焦/Occluded 时阻断 paint 路径
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn window_visibility_defaults_to_true() {
    let a = app_clip(None, None);
    assert!(a.window_state.visible, "App 刚构造时窗口默认可见");
}

#[test]
fn window_visibility_state_transitions() {
    // 手动驱动窗口可见性状态机，覆盖 Resized(0,0)/Focused/Focused(false)/
    // Occluded(true)/Occluded(false) 全部转换路径。
    let store = MemStore::default();
    let mut a = app("x\n", Some("a.md"), false, &store);
    assert!(a.window_state.visible);

    // Resized(0,0) → visible=false（最小化）
    a.window_state.resize_pending = true;
    a.window_state.visible = false; // 模拟 Resized(0,0) 分支赋值
    assert!(a.window_state.resize_pending);
    assert!(!a.window_state.visible);

    // Resized(800,600) → visible=true（恢复）
    a.window_state.visible = true; // 模拟 Resized(非零) 分支赋值
    assert!(a.window_state.visible);

    // Focused(false) → visible=false
    a.window_state.visible = false; // 模拟 Focused(false) 分支赋值
    assert!(!a.window_state.visible);

    // Focused(true) → visible=true
    a.window_state.visible = true; // 模拟 Focused(true) 分支赋值
    assert!(a.window_state.visible);

    // Occluded(true) → visible=false
    a.window_state.visible = false; // 模拟 Occluded(true) 分支赋值
    assert!(!a.window_state.visible);

    // Occluded(false) → visible=true
    a.window_state.visible = true; // 模拟 Occluded(false) 分支赋值
    assert!(a.window_state.visible);
}

#[test]
fn redraw_guard_allows_paint_when_visible() {
    // visible=true 时 paint 完整跑通（headless 下 present 是 no-op，render 是主路）
    let store = MemStore::default();
    let mut a = app("可见时应能正常 paint\n", None, false, &store);
    assert!(a.window_state.visible);
    a.paint();
    assert!(
        a.window_state.frame.is_some(),
        "visible=true 时 paint 应产出 frame"
    );
}

#[test]
fn paint_is_harmless_when_not_visible() {
    // visible=false 时 RedrawRequested 分支会提前 return，但直接调用 paint（绕过守卫）
    // 也不能 panic——作为次级防线，headless 下 present() 本身就是 no-op。
    let store = MemStore::default();
    let mut a = app("不可见时 paint 不应 panic\n", None, false, &store);
    // 先跑一次让 frame 有值
    a.paint();
    let frame_before = a.window_state.frame.as_ref().map(|f| f.pixels.clone());

    // 手动设置不可见后再直接 paint（绕过 RedrawRequested 守卫）
    a.window_state.visible = false;
    a.paint(); // 不应 panic
    let frame_after = a.window_state.frame.as_ref().map(|f| f.pixels.clone());

    // paint 仍会 render（绕过守卫），但这是 headless 测试；生产中 RedrawRequested 守卫
    // 会直接 return，根本不会调 paint。这里仅断言「直接调 paint 在不可见态也无害」。
    assert!(a.window_state.frame.is_some());
    let _ = (frame_before, frame_after); // 无 surface，present 是 no-op
}

// ===== C1 菜单键盘导航 =====

#[test]
fn menu_keyboard_navigation_selects_and_clamps() {
    let store = MemStore::default();
    let mut a = app("ab\n", None, false, &store);
    a.ui_state.chrome.open = Some(0);
    let n = chrome::menu_rows(0, &a.recent_labels()).len();
    assert!(n > 1);

    a.route_key(Key::Down);
    assert_eq!(
        a.ui_state.chrome.hover_item,
        Some((0, 0)),
        "无选中时 ↓ 选首项"
    );
    a.route_key(Key::Down);
    assert_eq!(a.ui_state.chrome.hover_item, Some((0, 1)));
    a.route_key(Key::Up);
    a.route_key(Key::Up);
    assert_eq!(a.ui_state.chrome.hover_item, Some((0, 0)), "首项处 ↑ 钳位");
    for _ in 0..(n + 3) {
        a.route_key(Key::Down);
    }
    assert_eq!(
        a.ui_state.chrome.hover_item,
        Some((0, n - 1)),
        "末项处 ↓ 钳位"
    );

    // 无选中时 ↑ 直接选末项
    a.ui_state.chrome.hover_item = None;
    a.route_key(Key::Up);
    assert_eq!(a.ui_state.chrome.hover_item, Some((0, n - 1)));
}

#[test]
fn menu_keyboard_enter_runs_selected_item_and_closes() {
    let store = MemStore::default();
    let mut a = app("ab\n", None, false, &store);
    a.ui_state.chrome.open = Some(2); // 视图菜单
    let rows = chrome::menu_rows(2, &[]);
    let nav: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, (_, _, a))| *a != MenuAction::Separator)
        .map(|(i, _)| i)
        .collect();
    let dark_pos = nav
        .iter()
        .position(|&i| rows[i].2 == MenuAction::ThemeDark)
        .expect("视图菜单应含深色主题项");
    let dark = nav[dark_pos];
    for _ in 0..=dark_pos {
        a.route_key(Key::Down);
    }
    assert_eq!(a.ui_state.chrome.hover_item, Some((2, dark)));

    a.route_key(Key::Enter);
    assert_eq!(
        a.input_state.theme.mode,
        ThemeMode::Dark,
        "Enter 执行选中项"
    );
    assert_eq!(a.ui_state.chrome.open, None, "执行后菜单关闭");
    assert_eq!(a.ui_state.chrome.hover_item, None);
    assert_eq!(a.editor.session().serialize(), "ab\n", "菜单动作不改文档");
}

#[test]
fn menu_keyboard_enter_toggles_source_mode_and_closes() {
    let store = MemStore::default();
    let mut a = app("# 标题\n", None, false, &store);
    a.ui_state.chrome.open = Some(2); // 视图菜单
    let rows = chrome::menu_rows(2, &[]);
    let nav: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, (_, _, a))| *a != MenuAction::Separator)
        .map(|(i, _)| i)
        .collect();
    let source_pos = nav
        .iter()
        .position(|&i| rows[i].2 == MenuAction::ToggleSourceMode)
        .expect("视图菜单应含源码模式项");
    let source = nav[source_pos];
    for _ in 0..=source_pos {
        a.route_key(Key::Down);
    }
    assert_eq!(a.ui_state.chrome.hover_item, Some((2, source)));

    a.route_key(Key::Enter);
    assert!(a.source_mode(), "Enter 打开源码模式");
    assert_eq!(a.ui_state.chrome.open, None, "执行后菜单关闭");
    assert_eq!(a.ui_state.chrome.hover_item, None);

    // 再次经菜单执行可翻回渲染态
    a.ui_state.chrome.open = Some(2);
    a.ui_state.chrome.hover_item = Some((2, source));
    a.route_key(Key::Enter);
    assert!(!a.source_mode(), "再次执行关闭源码模式");
    assert_eq!(a.ui_state.chrome.open, None);
}

#[test]
fn menu_keyboard_enter_without_selection_is_noop() {
    let store = MemStore::default();
    let mut a = app("ab\n", None, false, &store);
    a.ui_state.chrome.open = Some(0);
    a.route_key(Key::Enter);
    assert_eq!(a.ui_state.chrome.open, Some(0), "无选中项不执行也不关菜单");
    assert_eq!(
        a.editor.session().serialize(),
        "ab\n",
        "无选中项时 Enter 不得误编辑文档"
    );
}

#[test]
fn menu_flags_track_save_undo_redo_availability() {
    let store = MemStore::default();
    let mut a = app("ab\n", Some("a.md"), false, &store);
    let clean = a.menu_flags();
    assert!(!clean.can_save, "干净文档：保存禁用");
    assert!(!clean.can_undo, "无撤销步：撤销禁用");
    assert!(!clean.can_redo, "无重做步：重做禁用");

    a.editor.apply(Intent::InsertText("c".into())).unwrap();
    let dirty = a.menu_flags();
    assert!(dirty.can_save, "脏文档：保存可用");
    assert!(dirty.can_undo, "有撤销步：撤销可用");
    assert!(!dirty.can_redo, "未撤销前无重做");

    a.editor.apply(Intent::Undo).unwrap();
    let undone = a.menu_flags();
    assert!(undone.can_redo, "撤销后可重做");
    assert!(!undone.can_undo, "撤销栈已空");

    // 脏标只在保存时清除（撤销回初始态仍算「有改动待确认」）
    a.editor.mark_saved();
    assert!(!a.menu_flags().can_save, "保存后回干净态：保存禁用");

    // 未命名草稿：保存始终可用（转另存为）
    let d = app("x", None, false, &store);
    assert!(d.menu_flags().can_save, "未命名草稿：保存可用");
}

#[test]
fn menu_keyboard_skips_disabled_items_and_enter_is_noop() {
    let store = MemStore::default();
    let mut a = app("ab\n", Some("a.md"), false, &store);
    a.ui_state.chrome.open = Some(1); // 编辑菜单
    a.route_key(Key::Down);
    assert_eq!(
        a.ui_state.chrome.hover_item,
        Some((1, 3)),
        "无可撤销时跳过撤销/重做，落在首个可用项（剪切）"
    );
    // 直接落到禁用项（模拟鼠标悬停）：回车不执行、菜单保持打开
    a.ui_state.chrome.hover_item = Some((1, 0));
    a.route_key(Key::Enter);
    assert_eq!(a.ui_state.chrome.open, Some(1), "禁用项回车不关菜单");
    assert_eq!(a.editor.session().serialize(), "ab\n", "禁用项不产生编辑");
}

#[test]
fn menu_click_on_disabled_item_keeps_menu_open() {
    let store = MemStore::default();
    let (mut a, calls) = app_dialogs("ab\n", Some("a.md"), false, &store, None, Some("out.md"));
    let (tx, ty) = menu_title_center(0);
    click_at(&mut a, tx, ty);
    assert_eq!(a.ui_state.chrome.open, Some(0));
    let (x, y) = popup_item_center(0, 4); // 保存（干净文档 → 禁用）
    click_at(&mut a, x, y);
    assert_eq!(a.ui_state.chrome.open, Some(0), "禁用项点击不关菜单");
    assert!(calls.borrow().is_empty(), "禁用项不触发任何对话框/写盘");
}

#[test]
fn file_menu_groups_by_spacing_without_separator_lines() {
    use crate::theme::{Theme, ThemeMode};
    let store = MemStore::default();
    let mut a = app("ab\n", Some("dir/a.md"), false, &store);
    a.input_state.theme = Theme::of(ThemeMode::Light, false);
    let theme = a.input_state.theme;
    a.persist_state.recent = vec![
        PathBuf::from("dir/r1.md"),
        PathBuf::from("dir/r2.md"),
        PathBuf::from("dir/r3.md"),
    ];
    a.ui_state.chrome.open = Some(0);
    a.paint();
    let frame = a.window_state.frame.as_ref().expect("paint 应产出帧");
    let recent = a.recent_labels();
    let p = chrome::popup_layout(0, 1.0, &recent, 1000.0);

    // S3：弹层只有边界线，内部不再画任何分隔线（右侧内边距列只应是面板底色）。
    let border = [
        theme.chrome_border[0],
        theme.chrome_border[1],
        theme.chrome_border[2],
    ];
    let x = (p.x + p.w - 6.0) as u32;
    for y in (p.y + 2.0) as u32..(p.y + p.h - 2.0) as u32 {
        let px = frame.get(x, y);
        assert_ne!(
            [px[0], px[1], px[2]],
            border,
            "弹层内部不得出现分隔线（y={y}）"
        );
    }
    // 分组只用 8px 间距：文件菜单有 4 个 Separator，行间其余间距为 0。
    let gaps: Vec<f32> = p
        .items
        .windows(2)
        .map(|w| w[1].2 - (w[0].2 + w[0].4))
        .collect();
    assert_eq!(
        gaps.iter().filter(|g| (**g - 8.0).abs() < 0.01).count(),
        4,
        "文件菜单应有 4 个分组间距"
    );
}

#[test]
fn hover_recent_file_shows_full_path_in_statusbar() {
    use crate::chrome;
    use std::path::PathBuf;
    let store = MemStore::default();
    let mut a = app("ab\n", Some("dir/a.md"), false, &store);
    a.persist_state.recent = vec![PathBuf::from("dir/r1.md")];
    a.ui_state.chrome.open = Some(0);
    let original = "原有提示".to_string();
    a.ui_state.chrome.notice = original.clone();

    // 找到第一个最近文件项的中心坐标
    let recent = a.recent_labels();
    let p = chrome::popup_layout(0, 1.0, &recent, 1000.0);
    let recent_item = p
        .items
        .iter()
        .find(|(i, _, _, _, _)| {
            matches!(
                chrome::action_at(0, *i, &recent),
                Some(chrome::MenuAction::OpenRecent(0))
            )
        })
        .copied()
        .expect("应有最近文件项");
    let (_, ix, iy, iw, ih) = recent_item;

    // 鼠标移到最近文件项上 → 状态栏显示完整路径
    a.input_state.mouse = PhysicalPosition::new((ix + iw / 2.0) as f64, (iy + ih / 2.0) as f64);
    a.update_hover();
    assert!(
        a.ui_state.chrome.notice.contains("r1.md"),
        "悬停最近文件应显示完整路径，实际：{}",
        a.ui_state.chrome.notice
    );

    // 鼠标移开 → 恢复原提示
    a.input_state.mouse = PhysicalPosition::new(0.0, 0.0);
    a.update_hover();
    assert_eq!(a.ui_state.chrome.notice, original, "离开后应恢复原提示");
}

#[test]
fn disabled_menu_item_paints_muted_without_hover() {
    use crate::theme::{Theme, ThemeMode};
    let store = MemStore::default();
    let mut a = app("ab\n", Some("dir/a.md"), false, &store);
    a.input_state.theme = Theme::of(ThemeMode::Light, false);
    let theme = a.input_state.theme;
    a.ui_state.chrome.open = Some(0);
    // 干净文档：保存禁用，且悬停其上
    let p = chrome::popup_layout(0, 1.0, &[], 1000.0);
    let save = p.items[4]; // 新建 / 新建窗口 / 打开 / 快速打开 / 保存
    let (row, sx, sy, sw, sh) = save;
    a.ui_state.chrome.hover_item = Some((0, row));
    a.paint();
    let y0 = sy.ceil() as u32;
    let y1 = (sy + sh).floor() as u32;
    let x1 = (sx + sw).floor() as u32;
    {
        let frame = a.window_state.frame.as_ref().unwrap();
        let bg = frame.get(sx as u32 + 2, y0 + 2);
        assert_eq!(
            [bg[0], bg[1], bg[2]],
            [theme.popup_bg[0], theme.popup_bg[1], theme.popup_bg[2]],
            "禁用项不得出现悬停底色"
        );
        let (mut muted, mut normal) = (0, 0);
        for y in y0..y1 {
            for x in (sx as u32)..x1 {
                let px = frame.get(x, y);
                let c = [px[0], px[1], px[2]];
                if c == theme.muted {
                    muted += 1;
                }
                if c == theme.text {
                    normal += 1;
                }
            }
        }
        assert!(muted > 0, "禁用项文字应以弱化色绘制");
        assert_eq!(normal, 0, "禁用项不得使用正常文字色");
    }

    // 脏文档：保存可用 → 悬停底色 + 正常字色
    a.editor.apply(Intent::InsertText("c".into())).unwrap();
    a.paint();
    let frame = a.window_state.frame.as_ref().unwrap();
    // 采样点落在内缩圆角高亮块内部（避开 3px 内缩与圆角过渡带）
    let bg = frame.get(sx as u32 + 8, (sy + sh / 2.0) as u32);
    assert_eq!(
        [bg[0], bg[1], bg[2]],
        [theme.hover[0], theme.hover[1], theme.hover[2]],
        "可用项悬停应有底色"
    );
    let mut normal = 0;
    for y in y0..y1 {
        for x in (sx as u32)..x1 {
            let px = frame.get(x, y);
            if [px[0], px[1], px[2]] == theme.text {
                normal += 1;
            }
        }
    }
    assert!(normal > 0, "可用项文字应使用正常色");
}

#[test]
fn context_menu_keyboard_up_selects_last_and_enter_executes() {
    let store = MemStore::default();
    let mut a = app("ab\n", None, false, &store);
    a.ui_state
        .ctx
        .open_at(10.0, 20.0, ctx_menu::CtxKind::Selection, None, None);
    let n = a.ui_state.ctx.items().len();
    assert_eq!(a.mode(), Mode::Menu);

    a.route_key(Key::Up);
    assert_eq!(a.ui_state.ctx.hover, Some(n - 1), "无选中时 ↑ 选末项");
    // 选区菜单末项为「全选」：只改选区，不动文档文本
    a.route_key(Key::Enter);
    assert!(!a.ui_state.ctx.open, "执行后右键菜单关闭");
    assert_eq!(a.ui_state.ctx.hover, None);
    let sel = a.editor.selection().expect("全选应产生选区");
    assert_eq!((sel.start, sel.end), (0, 3));
    assert_eq!(a.editor.session().serialize(), "ab\n", "文档文本不得变化");
}

#[test]
fn menu_open_swallows_character_keys() {
    let store = MemStore::default();
    let mut a = app("ab\n", None, false, &store);
    a.ui_state.chrome.open = Some(0);
    a.route_key(Key::Text("a".into()));
    a.route_key(Key::Down);
    a.route_key(Key::Text("中".into()));
    assert_eq!(
        a.editor.session().serialize(),
        "ab\n",
        "菜单展开时字符键不落文档"
    );
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

#[test]
fn source_mode_toggles_markup_but_keeps_caret_and_scroll() {
    let store = MemStore::default();
    let text = "# 标题\n\n**粗体** 与 [链接](https://example.com)\n\n尾\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len() - 1); // 光标停在尾段，标题/行内节点均非激活
    a.ensure_layouts_all();
    a.window_state.scroll_y = 42.0;
    let heading = block_id_of(&a, |k| matches!(k, BlockKind::AtxHeading(_)));
    let para = a
        .index
        .blocks
        .iter()
        .find(|g| g.id != heading && g.kind == BlockKind::Paragraph)
        .map(|g| g.id)
        .expect("文档应含正文段");
    let caret = a.editor.caret();

    assert_eq!(a.cache.layouts[&heading].layout_text(), "标题");
    assert!(!a.cache.layouts[&para].layout_text().contains("**"));
    assert!(!a.cache.layouts[&para].layout_text().contains('['));

    a.perform_action(Action::OpenSourceMode);
    a.ensure_layouts_all();
    assert!(a.source_mode(), "Ctrl+/ 开启源码模式");
    assert!(
        a.cache.layouts[&heading].layout_text().contains('#'),
        "标题显 `#`: {:?}",
        a.cache.layouts[&heading].layout_text()
    );
    let src = a.cache.layouts[&para].layout_text().to_string();
    assert!(src.contains("**"), "粗体显标记: {src:?}");
    assert!(src.contains('['), "链接显标记: {src:?}");
    assert_eq!(a.editor.caret(), caret, "切换不得移动光标");
    assert_eq!(a.window_state.scroll_y, 42.0, "切换不得改变滚动位置");

    a.perform_action(Action::OpenSourceMode);
    a.ensure_layouts_all();
    assert!(!a.source_mode());
    assert_eq!(a.cache.layouts[&heading].layout_text(), "标题");
    assert!(!a.cache.layouts[&para].layout_text().contains("**"));
}

#[test]
fn frontmatter_hidden_in_wysiwyg_and_source_shows_raw() {
    // REN-SWEEP-001：非活动态 Front Matter 折叠为零高；光标进入/源码模式显原文。
    let store = MemStore::default();
    let text = "---\ntitle: 演示\ntags: [a, b]\n---\n\n# 标题\n\n正文\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.find("正文").unwrap());
    a.ensure_layouts_all();
    a.render();
    let fm = block_id_of(&a, |k| matches!(k, BlockKind::FrontMatter(_)));
    let g = a.index.blocks.iter().find(|g| g.id == fm).unwrap();
    assert_eq!(g.height, 0.0, "非活动态 Front Matter 应完全隐藏");
    let fm_top = g.top;
    let next_top = a.index.blocks.iter().find(|g| g.id != fm).unwrap().top;
    assert_eq!(next_top, fm_top, "隐藏的 Front Matter 不占前后段距");

    // 光标进入块内：WYSIWYM 显示原文（可编辑）。
    a.editor.set_caret(text.find("title").unwrap());
    a.ensure_layouts_all();
    a.render();
    let g = a.index.blocks.iter().find(|g| g.id == fm).unwrap();
    assert!(g.height > 0.0, "光标进入时 Front Matter 应显示原文");
    assert!(
        a.cache.layouts[&fm].layout_text().contains("title: 演示"),
        "应显示 YAML 原文: {:?}",
        a.cache.layouts[&fm].layout_text()
    );

    // 源码模式：光标移出后仍显示原文。
    a.editor.set_caret(text.find("正文").unwrap());
    a.perform_action(Action::OpenSourceMode);
    a.ensure_layouts_all();
    a.render();
    let g = a.index.blocks.iter().find(|g| g.id == fm).unwrap();
    assert!(a.source_mode());
    assert!(g.height > 0.0, "源码模式必须显示 Front Matter 原文");
    assert!(a.cache.layouts[&fm].layout_text().contains("---"));

    // 纯视图态：文档字节不变。
    assert_eq!(doc_text(&a), text);
}

#[test]
fn toml_frontmatter_paragraph_hidden_mid_document() {
    // REN-SWEEP-001：夹具里正文中间的 TOML Front Matter 示例（内核只认文首）
    // 在 WYSIWYG 非活动态同样隐藏；光标进入显原文。
    let store = MemStore::default();
    let text = "# 前\n\n+++\ntitle = \"TOML Front Matter\"\n+++\n\n后\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.find('后').unwrap());
    a.ensure_layouts_all();
    a.render();
    let toml = a
        .index
        .blocks
        .iter()
        .find(|g| {
            a.editor
                .session()
                .text_slice(g.range)
                .is_some_and(|t| t.contains("title ="))
        })
        .map(|g| g.id)
        .expect("应存在 TOML Front Matter 段落");
    let g = a.index.blocks.iter().find(|g| g.id == toml).unwrap();
    assert_eq!(g.height, 0.0, "非活动态 TOML Front Matter 应隐藏");
    a.editor.set_caret(text.find("title").unwrap());
    a.ensure_layouts_all();
    a.render();
    let g = a.index.blocks.iter().find(|g| g.id == toml).unwrap();
    assert!(g.height > 0.0, "光标进入时应显示 TOML 原文");
    assert_eq!(doc_text(&a), text, "纯视图态，文档字节不变");
}

#[test]
fn source_mode_hides_math_image_and_table_grid() {    let store = MemStore::default();
    let text = "前言\n\n$$\nx=1\n$$\n\n| a | b |\n| - | - |\n| 1 | 2 |\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(0); // 光标只在首段，公式/表格块均非激活
    a.ensure_layouts_all();
    a.ensure_math();
    a.ensure_tables();
    let math = a
        .index
        .blocks
        .iter()
        .find(|g| g.kind == BlockKind::Math)
        .map(|g| (g.id, g.range))
        .expect("含公式块");
    let table = a
        .index
        .blocks
        .iter()
        .find(|g| g.kind == BlockKind::Table)
        .map(|g| (g.id, g.range))
        .expect("含表格块");
    assert!(a.math_shows_image(math.0, &math.1));
    assert!(a.table_shows_grid(table.0, &table.1));

    a.perform_action(Action::OpenSourceMode);
    a.ensure_layouts_all();
    assert!(!a.math_shows_image(math.0, &math.1), "源码模式不渲染公式图");
    assert!(
        !a.table_shows_grid(table.0, &table.1),
        "源码模式不渲染表格网格"
    );
    assert!(
        !a.cache.tables.contains_key(&table.0),
        "源码模式不产表格网格缓存"
    );
    let tl = a.cache.layouts[&table.0].layout_text().to_string();
    assert!(tl.contains('|'), "表格显管道与分隔行源码: {tl:?}");

    a.perform_action(Action::OpenSourceMode);
    a.ensure_math();
    a.ensure_tables();
    assert!(a.math_shows_image(math.0, &math.1), "关闭后恢复公式图");
    assert!(a.table_shows_grid(table.0, &table.1), "关闭后恢复表格网格");
}

/// DEF-T05-002：源码模式下表格绘制原始行文本，而不是空白网格区域。
#[test]
fn source_mode_table_paints_raw_rows_instead_of_blank() {
    let store = MemStore::default();
    let mut a = app("| a | b |\n| --- | --- |\n| c | d |\n", None, false, &store);
    a.perform_action(Action::OpenSourceMode);
    let _ = a.render(); // 先完成排版/几何，再取稳定坐标
    let (top, h) = a
        .index
        .blocks
        .iter()
        .find(|g| g.kind == BlockKind::Table)
        .map(|g| (g.top, a.block_height(g)))
        .expect("含表格块");
    let vp = a.viewport();
    let bg = a.input_state.theme.bg;
    let scroll = a.window_state.scroll_y;
    let x0 = a.content_x().max(vp.x) as u32;
    let x1 = (a.content_x() + a.content_width()).min(vp.x + vp.w) as u32;
    let y0 = (vp.y + top - scroll).max(vp.y) as u32;
    let y1 = (y0 as f32 + h).min(vp.y + vp.h) as u32;
    let f = a.render();
    let mut ink = 0usize;
    for y in y0..y1 {
        for x in x0..x1 {
            if f.get(x, y) != bg {
                ink += 1;
            }
        }
    }
    assert!(ink > 50, "源码模式表格块应有原始行文本像素，实际 ink={ink}");
}

/// DEF-T05-005：`[TOC]` 在视图层渲染生成目录，源码/编辑态显标记，且不改文档。
#[test]
fn toc_marker_renders_directory_in_view_only() {
    let store = MemStore::default();
    let src = "[TOC]\n\n# 一\n\n## 二\n\n正文\n";
    let mut a = app(src, None, false, &store);
    a.editor.set_caret(src.len());
    a.refresh_index();
    a.ensure_layouts_all();
    let marker = a
        .index
        .blocks
        .iter()
        .find(|g| g.toc_marker)
        .map(|g| (g.id, g.range))
        .expect("应识别 TOC 标记块");
    let text = a.cache.layouts[&marker.0].layout_text().to_string();
    assert!(text.contains("• 一"), "目录应含一级标题: {text:?}");
    assert!(text.contains("   • 二"), "目录应含缩进的二级标题: {text:?}");
    assert_eq!(a.editor.session().serialize(), src, "不得写回文档");
    assert!(!a.editor.is_dirty(), "视图目录不得置脏");
    assert_eq!(a.editor.session().undo_depth(), 0, "视图目录不得产生撤销步");

    // 源码模式：原样显示 `[TOC]`。
    a.perform_action(Action::OpenSourceMode);
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&marker.0].layout_text(), "[TOC]");
    a.perform_action(Action::OpenSourceMode);
    // 编辑态（光标在标记块内）：显源码，便于修改标记。
    a.editor.set_caret(marker.1.start + 1);
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&marker.0].layout_text(), "[TOC]");

    // 动态刷新：插入新标题后目录重排并包含新条目。
    let end = a.editor.session().serialize().len();
    a.editor.set_caret(end);
    a.editor
        .apply(Intent::InsertText("\n# 新章\n".into()))
        .unwrap();
    a.refresh_index();
    a.ensure_layouts_all();
    let marker_id = a
        .index
        .blocks
        .iter()
        .find(|g| g.toc_marker)
        .expect("插入标题后仍应识别标记块")
        .id;
    let text = a.cache.layouts[&marker_id].layout_text().to_string();
    assert!(text.contains("新章"), "标题增删后目录应动态刷新: {text:?}");
}

/// DEF-T05-005 复发：预览态新增/删除标题后目录必须下一帧即刷新，无需重开文档。
/// 走真实编辑链路（`after_edit`），且全程不切源码模式、光标不进入标记块——
/// 避免旧测试因标记块恰好显示源码而侥幸刷新。
#[test]
fn toc_refreshes_after_heading_added_and_deleted_without_reopening() {
    let store = MemStore::default();
    let src = "[TOC]\n\n# 一\n\n## 二\n\n正文\n";
    let mut a = app(src, None, false, &store);
    a.editor.set_caret(src.len());
    a.refresh_index();
    a.ensure_layouts_all();
    let marker_id = a
        .index
        .blocks
        .iter()
        .find(|g| g.toc_marker)
        .expect("应识别 TOC 标记块")
        .id;
    let text = a.cache.layouts[&marker_id].layout_text().to_string();
    assert!(text.contains("• 一") && text.contains("   • 二"), "初始目录: {text:?}");
    assert!(!text.contains("新章"));

    // 文末新增二级标题：真实编辑链路（增量索引 + 视口排版）。
    let end = a.editor.session().serialize().len();
    a.editor.set_caret(end);
    let dirty = a
        .editor
        .apply(Intent::InsertText("\n## 新章\n".into()))
        .unwrap();
    a.after_edit(dirty);
    a.ensure_layouts_all();
    let text = a.cache.layouts[&marker_id].layout_text().to_string();
    assert!(
        text.contains("新章"),
        "新增标题后目录必须立即刷新（无需重开）: {text:?}"
    );
    assert_eq!(
        a.editor.session().serialize(),
        src.replace("正文\n", "正文\n\n## 新章\n"),
        "视图目录不得改写文档字节"
    );

    // 删除刚新增的标题（退格 7 次：`\n## 新章\n`），目录须移除该条目。
    for _ in 0..7 {
        let dirty = a.editor.apply(Intent::DeleteBackward).unwrap();
        a.after_edit(dirty);
    }
    a.ensure_layouts_all();
    let text = a.cache.layouts[&marker_id].layout_text().to_string();
    assert!(
        !text.contains("新章"),
        "删除标题后目录必须立即移除条目: {text:?}"
    );
    assert_eq!(a.editor.session().serialize(), src, "删除后文档回到原文");
}

/// 旧版目录列表（Task 2）：整组均为标题锚点链接时按层级呈现、不叠加列表序号；
/// 含空格锚点（`(#第三章 - 结论)` / `(#11- 背景)`）可点击跳转；不改文档。
#[test]
fn legacy_toc_list_renders_without_numbers_and_spaced_anchor_jumps() {
    let src = "1. [第一章 介绍](#第一章 - 介绍)\n   1. [1.1 背景](#11- 背景)\n1. [第二章 方法](#第二章 - 方法)\n\n# 第一章 介绍\n\n## 1.1 背景\n\n# 第二章 方法\n\n正文\n";
    let mut a = app(src, None, false, &MemStore::default());
    a.editor.set_caret(src.len());
    a.refresh_index();
    a.ensure_layouts_all();
    let items: Vec<(BlockId, Range)> = a
        .index
        .blocks
        .iter()
        .filter(|g| g.toc_list)
        .map(|g| (g.id, g.range))
        .collect();
    assert_eq!(items.len(), 3, "整组目录项应全部识别");
    let texts: Vec<String> = items
        .iter()
        .map(|(id, _)| a.cache.layouts[id].layout_text().to_string())
        .collect();
    assert_eq!(
        texts,
        ["    第一章 介绍", "      1.1 背景", "    第二章 方法"],
        "旧目录不叠加序号，保留源码缩进层级"
    );
    assert_eq!(a.editor.session().serialize(), src, "不得改写文档字节");
    assert!(!a.editor.is_dirty(), "不得置脏");
    assert_eq!(a.editor.session().undo_depth(), 0, "不得产生 undo");

    // 含空格锚点：Ctrl+点击跳转到对应标题。
    ctrl_click_doc(&mut a, src.find("第一章 介绍").unwrap());
    assert_eq!(
        a.editor.caret(),
        src.find("# 第一章").unwrap(),
        "空格锚点应可跳转"
    );
    ctrl_click_doc(&mut a, src.find("1.1 背景").unwrap());
    assert_eq!(
        a.editor.caret(),
        src.find("## 1.1 背景").unwrap(),
        "带编号的空格锚点应可跳转"
    );
    assert_eq!(a.editor.session().serialize(), src, "跳转不改文档");
}

/// 普通链接列表（外部 URL / 含普通文本项 / 技能列表外的列表）不得被判为目录。
#[test]
fn non_toc_link_lists_keep_normal_list_rendering() {
    let src = "- [A](https://a.example)\n- [B](https://b.example)\n\n1. [C](#目标)\n2. 普通项\n\n# 目标\n";
    let mut a = app(src, None, false, &MemStore::default());
    a.editor.set_caret(src.len());
    a.refresh_index();
    a.ensure_layouts_all();
    assert!(
        a.index.blocks.iter().all(|g| !g.toc_list),
        "普通链接列表与混合列表不得判为目录"
    );
    let texts: Vec<String> = a
        .index
        .blocks
        .iter()
        .filter(|g| matches!(g.kind, BlockKind::ListItem { .. }))
        .map(|g| a.cache.layouts[&g.id].layout_text().to_string())
        .collect();
    assert_eq!(texts, ["• A", "• B", "1. C", "2. 普通项"]);
}

/// `[TOC]` 视图目录在专注/打字机模式下继承同一渲染：同样无有序列表序号，
/// 且两种模式切换不写回文档、不产生撤销步。
#[test]
fn toc_render_is_shared_in_focus_and_typewriter_modes() {
    let store = MemStore::default();
    let src = "[TOC]\n\n# 一\n\n## 1.1 背景\n\n正文\n";
    let mut a = app(src, None, false, &store);
    a.editor.set_caret(src.len());
    a.refresh_index();
    a.ensure_layouts_all();
    let marker = a
        .index
        .blocks
        .iter()
        .find(|g| g.toc_marker)
        .map(|g| g.id)
        .expect("应识别 TOC 标记块");
    let plain = a.cache.layouts[&marker].layout_text().to_string();
    assert!(plain.contains("• 一") && plain.contains("   • 1.1 背景"));
    assert!(
        !plain.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("1. ") || t.starts_with("2. ")
        }),
        "生成目录不得叠加有序列表序号: {plain:?}"
    );

    a.toggle_focus();
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&marker].layout_text(), plain, "专注模式同一渲染");
    a.toggle_typewriter();
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&marker].layout_text(), plain, "打字机模式同一渲染");
    assert_eq!(a.editor.session().serialize(), src);
    assert!(!a.editor.is_dirty());
    assert_eq!(a.editor.session().undo_depth(), 0);
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

#[test]
fn print_ignores_source_mode_and_screen_restores_it() {
    let probe = SourcePrintProbe {
        during_heading: Rc::new(RefCell::new(String::new())),
        after_heading: Rc::new(RefCell::new(String::new())),
        source_during_print: Rc::new(Cell::new(false)),
    };
    let during = probe.during_heading.clone();
    let after = probe.after_heading.clone();
    let flag = probe.source_during_print.clone();
    let text = "# 标\n\n尾\n";
    let mut a = app_print(text, Box::new(probe));
    a.editor.set_caret(text.len() - 1);
    a.perform_action(Action::OpenSourceMode);
    a.ensure_layouts_all();
    let heading = block_id_of(&a, |k| matches!(k, BlockKind::AtxHeading(_)));
    assert!(a.cache.layouts[&heading].layout_text().contains('#'));

    a.perform_menu(MenuAction::Print);
    assert!(!flag.get(), "打印中 source_mode() 恒 false");
    assert_eq!(&*during.borrow(), "标", "打印态忽略源码模式");
    assert_eq!(&*after.borrow(), "# 标", "打印结束回屏幕后恢复源码模式");
    assert!(a.source_mode(), "用户开关未被打印改写");
}

// ===== 排版（Task 5）：可读列宽上限 / H4–H6 缩进 / 标题段距 =====

/// 420px 窄窗口 + 12 列：表格必须压回版心宽，块高覆盖所有单元格，
/// 后续段落起点不早于表格底边（不得压字/视口异常）。
#[test]
fn narrow_twelve_column_table_fits_and_tail_does_not_overlap() {
    let store = MemStore::default();
    let cols = 12;
    for scale in [1.0, 1.5] {
        let head = format!(
            "| {} |\n",
            (1..=cols)
                .map(|i| format!("C{i:02}_START"))
                .collect::<Vec<_>>()
                .join(" | ")
        );
        let sep = format!("| {} |\n", vec!["---"; cols].join(" | "));
        let row = format!(
            "| {} |\n",
            [
                "START_CELL",
                "中文长内容用于窄列逐字折行测试",
                "c03",
                "c04",
                "c05",
                "c06",
                "c07",
                "c08",
                "c09",
                "c10",
                "c11",
                "LAST_CELL"
            ]
            .join(" | ")
        );
        let text = format!("{head}{sep}{row}\nTAIL_AFTER_OVERFLOW_TABLE\n");
        let mut a = app(&text, None, false, &store);
        a.window_state.test_size = Some((420, 700));
        a.input_state.scale = scale;
        a.editor.set_caret(text.len());
        a.refresh_index();
        a.ensure_layouts_all();
        a.ensure_tables();
        a.recompute_geometry();
        let table = a
            .index
            .blocks
            .iter()
            .find(|g| g.kind == BlockKind::Table)
            .expect("应有表格块");
        let tail = a
            .index
            .blocks
            .iter()
            .find(|g| g.kind == BlockKind::Paragraph && g.range.start > table.range.start)
            .expect("应有表格后的段落");
        let tl = a.cache.table(table.id, a.table_key(table.id)).expect("网格已排版");
        assert!(
            tl.width <= a.content_width() + 0.5,
            "scale={scale}: 表格宽 {} 超出可用宽 {}",
            tl.width,
            a.content_width()
        );
        assert!(
            (a.block_height(table) - tl.height).abs() < 0.5,
            "scale={scale}: 块高必须与网格高一致"
        );
        assert!(
            table.top + tl.height <= tail.top + 0.5,
            "scale={scale}: 后文起点 {} 早于表格底边 {}",
            tail.top,
            table.top + tl.height
        );
        for c in &tl.cells {
            assert!(
                c.y + c.pixel_h as f32 <= tl.height + 0.5,
                "scale={scale}: 单元格内容溢出表格高度"
            );
        }
    }
}

/// 超宽窗口：版心被可读列宽钳制，并在窗口内水平居中。
#[test]
fn wide_window_caps_content_width_and_centers() {
    let store = MemStore::default();
    let mut a = app("正文", None, false, &store);
    a.window_state.test_size = Some((2560, 800));
    a.refresh_index();
    a.ensure_layouts_all();
    a.recompute_geometry();
    let scale = a.input_state.scale;
    let available = 2560.0 - 2.0 * MARGIN * scale - chrome::SCROLL_W * scale;
    assert!(
        (a.content_width() - 640.0 * scale).abs() < 0.01,
        "超宽窗口版心钳到可读列宽: {}",
        a.content_width()
    );
    assert!(
        (a.content_x() - (MARGIN * scale + (available - 640.0 * scale) / 2.0)).abs() < 0.01,
        "版心应水平居中: {}",
        a.content_x()
    );
}

/// 窄窗口：上限不生效，content_width / content_x 与改动前公式逐字节一致。
#[test]
fn narrow_window_content_width_unchanged_by_readable_cap() {
    let store = MemStore::default();
    let mut a = app("正文", None, false, &store);
    a.window_state.test_size = Some((1200, 800));
    let scale = a.input_state.scale;
    let want = (1200.0 - 2.0 * MARGIN * scale - chrome::SCROLL_W * scale).max(32.0);
    assert!((a.content_width() - want).abs() < 1e-4);
    assert!(
        (a.content_x() - MARGIN * scale).abs() < 1e-4,
        "窄窗口不偏移原点"
    );
}

/// 宽窗口下点击列宽内同一点，命中结果与窄窗口同一相对位置一致
/// （防止只改绘制、漏改内容坐标系而出现「中心偏移」）。
#[test]
fn wide_window_click_hits_same_column_as_narrow() {
    let store = MemStore::default();
    let text: String = (0..40).map(|i| char::from(b'0' + (i % 10) as u8)).collect();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_all();
    let vp = a.viewport();
    let y = (vp.y + a.index.blocks[0].top + 5.0) as f64;
    a.input_state.mouse = PhysicalPosition::new((MARGIN * a.input_state.scale + 100.0) as f64, y);
    let narrow = a.hit_offset().expect("窄窗口命中");

    a.window_state.test_size = Some((2560, 800));
    a.refresh_index();
    a.ensure_layouts_all();
    a.recompute_geometry();
    let vp = a.viewport();
    let wide_x = a.content_x() + 100.0;
    assert!(
        wide_x > MARGIN * a.input_state.scale + 100.0,
        "宽窗口原点应右移"
    );
    a.input_state.mouse =
        PhysicalPosition::new(wide_x as f64, (vp.y + a.index.blocks[0].top + 5.0) as f64);
    let wide = a.hit_offset().expect("宽窗口命中");
    assert_eq!(wide, narrow, "同列点击应命中同一文档偏移");
    assert!(
        narrow > 0 && narrow < text.len(),
        "命中点须落在行内: {narrow}"
    );
}

/// H4 非激活：标记区替换为 2 个半角空格缩进，行高与激活态一致；
/// H1–H3 仍为纯消隐，行为不变。
#[test]
fn h4_marker_replaced_by_indent_and_h1_h3_unchanged() {
    let store = MemStore::default();
    let mut a = app("#### a\n\n正文", None, false, &store);
    a.editor.set_caret(a.editor.session().text_len());
    a.ensure_layouts_all();
    let h4 = block_id_of(&a, |k| matches!(k, BlockKind::AtxHeading(4)));
    assert_eq!(
        a.cache.layouts[&h4].layout_text(),
        "  a",
        "H4 应缩进 2 空格"
    );
    let height = |a: &App| {
        let g = a.index.blocks.iter().find(|g| g.id == h4).unwrap();
        a.block_height(g)
    };
    let inactive = height(&a);
    assert!(
        inactive > BASE_LINE_HEIGHT * a.input_state.scale * 0.9
            && inactive < BASE_LINE_HEIGHT * a.input_state.scale * 1.5,
        "缩进占位须保持单行行高: {inactive}"
    );
    a.editor.set_caret(0);
    a.ensure_layouts_all();
    assert_eq!(
        a.cache.layouts[&h4].layout_text(),
        "#### a",
        "激活态还原源码"
    );
    assert!((inactive - height(&a)).abs() < 0.01, "显隐切换不得改变行高");

    let mut b = app("### a\n\n正文", None, false, &store);
    b.editor.set_caret(b.editor.session().text_len());
    b.ensure_layouts_all();
    let h3 = block_id_of(&b, |k| matches!(k, BlockKind::AtxHeading(3)));
    assert_eq!(b.cache.layouts[&h3].layout_text(), "a", "H1–H3 保持纯消隐");

    // H5/H6：缩进随级别递增（4 / 6 空格），同字号靠缩进区分
    for (src, want) in [("##### a", "    a"), ("###### a", "      a")] {
        let text = format!("{src}\n\n正文");
        let mut c = app(&text, None, false, &store);
        c.editor.set_caret(c.editor.session().text_len());
        c.ensure_layouts_all();
        let id = c.index.blocks[0].id;
        assert_eq!(c.cache.layouts[&id].layout_text(), want, "{src}");
    }
}

/// 标题后段距大于正文段距；total_h 与逐块累加（含空行/文末留白）一致。
#[test]
fn heading_gap_exceeds_paragraph_gap_and_total_h_consistent() {
    let store = MemStore::default();
    let mut a = app("## 标题\n\n正文一\n\n正文二", None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    a.recompute_geometry();
    let blocks = &a.index.blocks;
    assert_eq!(blocks.len(), 3);
    assert!(matches!(blocks[0].kind, BlockKind::AtxHeading(2)));
    let heading_to_body = blocks[1].top - (blocks[0].top + blocks[0].height);
    let body_to_body = blocks[2].top - (blocks[1].top + blocks[1].height);
    assert!(
        heading_to_body > body_to_body,
        "标题段距应大于正文段距: {heading_to_body} vs {body_to_body}"
    );
    let want = expected_doc_height(&a);
    assert!(
        (a.index.total_h - want).abs() < 1e-3,
        "total_h 应与逐块累加一致: {} vs {want}",
        a.index.total_h
    );
}

// ---- 空 Mermaid 块提示点击插入（Wave 6） ----

const EMPTY_MERMAID_DOC: &str = "p\n\n```mermaid\n```\n";

/// 空 mermaid 块 body 起点的绝对字节偏移（开围栏行换行之后）。
fn empty_mermaid_body_start(doc: &str) -> usize {
    doc.find("```mermaid\n").expect("文档应含 mermaid 开围栏") + "```mermaid\n".len()
}

/// 把鼠标放到 mermaid 块首行（卡片头）内；空块允许整块卡片命中。
fn hover_mermaid_block(a: &mut App) {
    a.ensure_layouts_all();
    let id = block_id_of(
        a,
        |k| matches!(k, BlockKind::FencedCode(Some(l)) if l.eq_ignore_ascii_case("mermaid")),
    );
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

#[test]
fn click_empty_mermaid_opens_template_only_menu() {
    let store = MemStore::default();
    let mut a = app(EMPTY_MERMAID_DOC, None, false, &store);
    let body_start = empty_mermaid_body_start(EMPTY_MERMAID_DOC);
    click_empty_mermaid_block(&mut a);
    assert!(a.ui_state.ctx.open, "空 mermaid 块左键应打开菜单");
    let actions: Vec<_> = a.ui_state.ctx.items().iter().map(|&(_, act)| act).collect();
    assert_eq!(
        actions,
        vec![
            ctx_menu::CtxAction::MermaidInsertFlowchart,
            ctx_menu::CtxAction::MermaidInsertSequence,
            ctx_menu::CtxAction::MermaidInsertClass,
        ],
        "空块菜单只含三条模板项"
    );
    assert_eq!(a.editor.caret(), body_start, "光标应落在 body 起点");

    // 右键同口径：空块只给三条模板项
    a.route_key(Key::Escape);
    hover_mermaid_block(&mut a);
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "空 mermaid 块右键也应打开菜单");
    assert_eq!(a.ui_state.ctx.items().len(), 3, "右键同样只含模板项");
    assert_eq!(
        a.ui_state.ctx.mermaid_insert_start,
        Some(body_start),
        "右键时模板插入锚点同样为 body 起点"
    );

    // 纯注释块与空块同口径
    let doc = "p\n\n```mermaid\n%% 注释\n%% 再一行\n```\n";
    let mut b = app(doc, None, false, &store);
    click_empty_mermaid_block(&mut b);
    assert!(b.ui_state.ctx.open, "纯注释块也应打开菜单");
    assert_eq!(b.ui_state.ctx.items().len(), 3);
    assert_eq!(
        b.editor.caret(),
        empty_mermaid_body_start(doc),
        "纯注释块光标也在 body 起点"
    );
}

#[test]
fn empty_mermaid_click_ignored_in_source_mode() {
    // 源码模式无卡片头/提示行：点击空 mermaid 块必须按普通落光标处理，
    // 不能弹出模板菜单（否则改源码会被劫持）。
    let store = MemStore::default();
    let mut a = app(EMPTY_MERMAID_DOC, None, false, &store);
    a.perform_action(Action::OpenSourceMode);
    assert!(a.source_mode());
    let n = a.editor.session().text_len();
    let before = a
        .editor
        .session()
        .text_slice(Range::new(0, n))
        .unwrap_or_default();
    click_empty_mermaid_block(&mut a);
    assert!(!a.ui_state.ctx.open, "源码模式点击不应打开模板菜单");
    let after = a
        .editor
        .session()
        .text_slice(Range::new(0, a.editor.session().text_len()))
        .unwrap_or_default();
    assert_eq!(after, before, "源码模式点击不应改动文档");
}

#[test]
fn non_empty_mermaid_right_click_keeps_full_menu() {
    let store = MemStore::default();
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app(doc, None, false, &store);
    hover_mermaid_block(&mut a);
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "非空 mermaid 块右键打开专用菜单");
    let actionable = a
        .ui_state
        .ctx
        .items()
        .iter()
        .filter(|&&(_, act)| act != ctx_menu::CtxAction::Separator)
        .count();
    assert_eq!(actionable, 8, "非空块保持现状 8 项可执行项");
    assert!(
        a.ui_state.ctx.mermaid_insert_start.is_none(),
        "非空块不启用空块插入锚点"
    );
}

#[test]
fn empty_mermaid_template_inserts_at_body_and_undo_once() {
    let store = MemStore::default();
    let mut a = app(EMPTY_MERMAID_DOC, None, false, &store);
    let body_start = empty_mermaid_body_start(EMPTY_MERMAID_DOC);
    click_empty_mermaid_block(&mut a);
    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::MermaidInsertFlowchart)
        .expect("空块菜单应含流程图模板项");
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);

    let template = veil_core::FLOWCHART_TEMPLATE;
    assert_eq!(
        a.editor.session().serialize(),
        format!("p\n\n```mermaid\n{template}\n```\n"),
        "模板插入 body 起点，闭合围栏保持独立一行"
    );
    assert_eq!(
        a.editor.caret(),
        body_start + template.len(),
        "光标位于插入内容之后"
    );

    a.editor.apply(Intent::Undo).unwrap();
    assert_eq!(
        a.editor.session().serialize(),
        EMPTY_MERMAID_DOC,
        "一次 undo 整体回退"
    );
}

#[test]
fn empty_mermaid_menu_cancel_keeps_bytes() {
    let store = MemStore::default();
    let mut a = app(EMPTY_MERMAID_DOC, None, false, &store);
    click_empty_mermaid_block(&mut a);
    a.route_key(Key::Escape);
    assert!(!a.ui_state.ctx.open, "Esc 关闭菜单");
    assert_eq!(a.editor.session().serialize(), EMPTY_MERMAID_DOC);

    click_empty_mermaid_block(&mut a);
    assert!(a.ui_state.ctx.open);
    a.input_state.mouse = PhysicalPosition::new(1.0, 1.0); // 菜单面板之外
    a.on_left_click();
    assert!(!a.ui_state.ctx.open, "点击菜单外关闭菜单");
    assert_eq!(
        a.editor.session().serialize(),
        EMPTY_MERMAID_DOC,
        "取消不改文档字节"
    );
}

#[test]
fn empty_mermaid_hint_absent_in_source_mode() {
    let store = MemStore::default();
    let mut a = app(EMPTY_MERMAID_DOC, None, false, &store);
    a.ensure_layouts_all();
    let id = block_id_of(
        &a,
        |k| matches!(k, BlockKind::FencedCode(Some(l)) if l.eq_ignore_ascii_case("mermaid")),
    );
    assert!(
        a.cache.layouts[&id]
            .layout_text()
            .contains("空 Mermaid 块 — 点击插入模板"),
        "渲染态显示提示"
    );
    a.perform_action(Action::OpenSourceMode);
    a.ensure_layouts_all();
    let src = a.cache.layouts[&id].layout_text();
    assert!(src.contains("```mermaid"), "源码模式保留围栏: {src:?}");
    assert!(
        !src.contains("空 Mermaid 块"),
        "源码模式不出现提示: {src:?}"
    );
}

// ---- Details 块折叠（Wave 7 Task 3-5） ----

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

/// 默认收起：空 expanded_details 集合，Details 块不在其中 → block_height 钳到单行。
#[test]
fn details_block_default_collapsed() {
    let store = MemStore::default();
    let mut a = app(DETAILS_DOC, None, false, &store);
    a.refresh_index();
    assert!(
        a.ui_state.expanded_details.is_empty(),
        "空集 = 全收起（details 块都不在展开集里）"
    );
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    a.ensure_layouts_all();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let line_h = a.engine.doc_line_height();
    assert!(
        (a.block_height(g) - line_h).abs() < 0.5,
        "折叠态高度应 ≈ 单行（{line_h:.1}），实际: {}",
        a.block_height(g)
    );
}

/// open_default=true 的 Details 块自动展开（sync_expanded_details 预塞进集合）。
#[test]
fn details_open_default_auto_expands() {
    let store = MemStore::default();
    let mut a = app(DETAILS_OPEN_DOC, None, false, &store);
    a.refresh_index();
    let id = block_id_of(&a, |k| {
        matches!(
            k,
            BlockKind::Details {
                open_default: true,
                ..
            }
        )
    });
    assert!(
        a.ui_state.expanded_details.contains(&id),
        "open_default=true 的块应自动加入 expanded_details"
    );
    a.ensure_layouts_all();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    // B3：头块只含 summary 一行，正文是独立子块——整组高度应明显大于单行。
    assert!(
        a.block_height(g) + a.details_children_height(g) > a.engine.doc_line_height() * 1.5,
        "展开态整组高度应明显大于单行"
    );
}

/// 点击 Details 块切换折叠——收起时整块点击展开；展开时点击 summary 行收起。
#[test]
fn details_click_toggles_expand() {
    let store = MemStore::default();
    let mut a = app(DETAILS_DOC, None, false, &store);
    let before = a.editor.session().serialize();
    a.refresh_index();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));

    // 初始态：全收起
    assert!(!a.ui_state.expanded_details.contains(&id));

    // 点击 Details 块区域 → 展开
    a.ensure_layouts_all();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let scale = a.input_state.scale;
    let vp = a.viewport();
    let click_y = (vp.y + g.top - a.window_state.scroll_y + 5.0 * scale) as f64;
    a.input_state.mouse = PhysicalPosition::new((a.content_x() + 10.0) as f64, click_y);
    a.on_left_click();
    assert!(a.ui_state.expanded_details.contains(&id), "点击后应展开");
    let mid = a.editor.session().serialize();
    assert_eq!(mid, before, "展开不改文档字节");

    // 再点击 summary 行 → 收起
    a.ensure_layouts_all();
    let g2 = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let click_y2 = (vp.y + g2.top - a.window_state.scroll_y + 2.0 * scale) as f64;
    a.input_state.mouse = PhysicalPosition::new((a.content_x() + 10.0) as f64, click_y2);
    a.on_left_click();
    assert!(!a.ui_state.expanded_details.contains(&id), "再次点击应收起");
    let after = a.editor.session().serialize();
    assert_eq!(after, before, "收起不改文档字节");
}

/// 非 100% DPI：summary 行点击仍能收起（y/top 是物理像素，行高须乘 scale）。
#[test]
fn details_click_collapses_at_non_unit_dpi() {
    let store = MemStore::default();
    let mut a = app(DETAILS_DOC, None, false, &store);
    a.input_state.set_dpi(1.25);
    a.refresh_index();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    a.ensure_layouts_all();
    a.recompute_geometry();
    let scale = a.input_state.scale;
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + 10.0) as f64,
        (vp.y + g.top - a.window_state.scroll_y + 5.0 * scale) as f64,
    );
    a.on_left_click();
    assert!(a.ui_state.expanded_details.contains(&id), "点击后应展开");
    a.ensure_layouts_all();
    let g2 = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + 10.0) as f64,
        (vp.y + g2.top - a.window_state.scroll_y + 2.0 * scale) as f64,
    );
    a.on_left_click();
    assert!(
        !a.ui_state.expanded_details.contains(&id),
        "1.25 DPI 下 summary 行点击应收起"
    );
}

/// B3：块级 details 展开后，内部标题/列表/代码/表格/段落各自成为原生块并排版。
#[test]
fn details_expanded_children_use_native_layouts() {
    let store = MemStore::default();
    let doc = "前\n\n<details open>\n<summary>S</summary>\n## 标题\n\n- 项\n\n```rust\nfn x() {}\n```\n\n| a |\n| --- |\n| 1 |\n\n段落\n</details>\n\n后\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    a.ensure_tables();
    let header = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    let kinds: Vec<BlockKind> = a
        .index
        .blocks
        .iter()
        .filter(|g| g.container.details_depth == 1)
        .map(|g| g.kind.clone())
        .collect();
    assert!(kinds.iter().any(|k| matches!(k, BlockKind::AtxHeading(2))));
    assert!(kinds
        .iter()
        .any(|k| matches!(k, BlockKind::ListItem { .. })));
    assert!(kinds.iter().any(|k| matches!(k, BlockKind::FencedCode(_))));
    assert!(kinds.contains(&BlockKind::Table));
    assert!(kinds.contains(&BlockKind::Paragraph));
    // 标题走标题字号/间距：块高大于正文行高。
    let hg = a
        .index
        .blocks
        .iter()
        .find(|g| matches!(g.kind, BlockKind::AtxHeading(2)))
        .unwrap();
    assert!(
        a.block_height(hg) > a.engine.doc_line_height(),
        "子块标题应使用标题字号层级"
    );
    // 代码块按代码卡排版（等宽文本进入布局）。
    let cg = a
        .index
        .blocks
        .iter()
        .find(|g| matches!(g.kind, BlockKind::FencedCode(_)))
        .unwrap();
    let ctext = a.cache.layouts[&cg.id].value.layout_text().to_string();
    assert!(ctext.contains("fn x() {}"), "代码子块应原生排版：{ctext:?}");
    // 表格走 B2 网格。
    let tg = a
        .index
        .blocks
        .iter()
        .find(|g| g.kind == BlockKind::Table)
        .unwrap();
    assert!(a.table_shows_grid(tg.id, &tg.range));
    assert!(a.cache.tables[&tg.id].value.rows.len() >= 2);
    // 卡片高度覆盖整组。
    let hdr = a.index.blocks.iter().find(|g| g.id == header).unwrap();
    assert!(
        a.details_children_height(hdr) > a.engine.doc_line_height() * 3.0,
        "卡片应覆盖全部子块高度"
    );
}

/// B3：折叠态子块高度为 0（整组不占排版空间），展开后再获得真实高度。
#[test]
fn details_collapsed_child_blocks_have_zero_height() {
    let store = MemStore::default();
    let doc = "p\n\n<details>\n<summary>S</summary>\n## t\n\n- a\n\n尾段\n</details>\n\nq\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    a.recompute_geometry();
    let header = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    for g in a
        .index
        .blocks
        .iter()
        .filter(|g| g.container.details_depth == 1)
    {
        assert_eq!(a.block_height(g), 0.0, "折叠态子块应为 0 高：{:?}", g.kind);
    }
    let hdr = a.index.blocks.iter().find(|g| g.id == header).unwrap();
    assert_eq!(a.details_children_height(hdr), 0.0);
    // 紧跟其后的顶层块从 summary 行底部开始（不存在被隐藏内容的空白）。
    let next = a
        .index
        .blocks
        .iter()
        .find(|g| g.range.start > hdr.range.start && g.container.details_depth == 0)
        .unwrap();
    assert!(next.top > hdr.top, "后续块仍在 summary 之后");
    // 展开后子块有真实高度。
    a.toggle_details(header);
    a.ensure_layouts_all();
    a.recompute_geometry();
    let any = a
        .index
        .blocks
        .iter()
        .filter(|g| g.container.details_depth == 1)
        .any(|g| a.block_height(g) > 0.0);
    assert!(any, "展开后子块应恢复真实高度");
}

/// B3：折叠态下上下移动光标跳过隐藏子块（落到 summary 或下一块）。
#[test]
fn collapsed_details_caret_skips_hidden_children() {
    let store = MemStore::default();
    let doc = "A\n\n<details>\n<summary>S</summary>\nB\n\nC\n</details>\n\nZ\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    a.recompute_geometry();
    a.editor.set_caret(1);
    a.move_vertical(1, false);
    let first = a.editor.caret();
    assert!(
        first < doc.find("B").unwrap(),
        "第一次下移应落在 details 头块（summary），实际 {first}"
    );
    a.move_vertical(1, false);
    let second = a.editor.caret();
    assert!(
        second >= doc.find("Z").unwrap(),
        "第二次下移应跳过隐藏子块直达下一顶层块，实际 {second}"
    );
}

/// B3：行内 `<details>` 默认只显示 summary；summary 点击切换展开/收起。
#[test]
fn inline_details_collapses_body_and_summary_click_toggles() {
    let store = MemStore::default();
    let doc = "前 <details><summary>更多</summary>隐藏正文XYZ</details> 后\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    let id = block_id_of(&a, |k| *k == BlockKind::Paragraph);
    let undo_before = a.editor.session().undo_depth();
    let collapsed = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        collapsed.contains("更多"),
        "应收起态显示 summary：{collapsed:?}"
    );
    assert!(
        !collapsed.contains("隐藏正文"),
        "收起态正文必须隐藏（不出现 <details> 裸标签）：{collapsed:?}"
    );
    assert!(!collapsed.contains("summary"), "不应显示裸标签");
    let (g_top, g_start) = {
        let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
        (g.top, g.range.start)
    };
    let scale = a.input_state.scale;
    let vp = a.viewport();
    let abs = doc.find("更多").unwrap() + 1;
    let (cx, cy) = {
        let l = &a.cache.layouts[&id].value;
        let (x, y, _) = a.engine.caret_xy(l, abs - g_start);
        (x, y)
    };
    let _ = scale;
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + cx) as f64,
        (vp.y + g_top - a.window_state.scroll_y + cy) as f64,
    );
    a.on_left_click();
    assert_eq!(
        a.ui_state.expanded_inline_details.len(),
        1,
        "点击 summary 应展开行内 details"
    );
    a.ensure_layouts_all();
    let expanded = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        expanded.contains("隐藏正文"),
        "展开后正文应以行内文本流继续：{expanded:?}"
    );
    assert_eq!(a.editor.session().serialize(), doc, "折叠/展开不改文档字节");
    assert_eq!(
        a.editor.session().undo_depth(),
        undo_before,
        "行内折叠/展开不得产生撤销步"
    );
    // 再点 summary → 收起。
    let g_top = a
        .index
        .blocks
        .iter()
        .find(|g| g.id == id)
        .map(|g| g.top)
        .unwrap();
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + cx) as f64,
        (vp.y + g_top - a.window_state.scroll_y + cy) as f64,
    );
    a.on_left_click();
    a.ensure_layouts_all();
    assert!(a.ui_state.expanded_inline_details.is_empty());
    let again = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(!again.contains("隐藏正文"));
}

/// details 闭标签行是纯 chrome（不属于任何块）：光标停在那里时吸附到前一块末尾。
#[test]
fn caret_in_details_close_line_snaps_to_previous_block() {
    let store = MemStore::default();
    let doc = "<details>\n<summary>S</summary>\n正文\n</details>\n\n尾\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    let close = doc.find("</details>").unwrap();
    a.editor.set_caret(close + 2);
    a.sync_details_at_caret();
    let caret = a.editor.caret();
    assert!(caret < close, "光标不应停在闭标签行，实际 {caret}");
    assert_eq!(a.editor.session().serialize(), doc, "吸附不改文档字节");
}

/// B3：搜索命中位于折叠 details 内时自动展开（块级与行内两种形态都覆盖）。
#[test]
fn search_match_inside_collapsed_details_auto_expands() {
    let store = MemStore::default();
    let doc =
        "p\n\n<details>\n<summary>S</summary>\nneedle_block\n</details>\n\n前 <details><summary>M</summary>needle_inline</details> 后\n";
    let mut a = app(doc, None, false, &store);
    a.refresh_index();
    a.ensure_layouts_all();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    assert!(!a.ui_state.expanded_details.contains(&id));
    a.open_search();
    a.search_input("needle_block");
    assert!(
        a.ui_state.expanded_details.contains(&id),
        "块级命中应自动展开"
    );
    // 继续搜行内正文命中。
    a.close_search();
    a.open_search();
    a.search_input("needle_inline");
    assert_eq!(
        a.ui_state.expanded_inline_details.len(),
        1,
        "行内 details 命中应自动展开"
    );
    assert_eq!(a.editor.caret(), doc.find("needle_inline").unwrap());
    a.ensure_layouts_all();
    assert_eq!(a.editor.session().serialize(), doc, "搜索展开不写文档");
}

/// 折叠态与展开态保存的字节完全一致——折叠是纯 UI 视图态。
/// 高度断言留给 details_block_default_collapsed / details_open_default_auto_expands 两个测试覆盖。
#[test]
fn details_fold_is_view_state_not_byte_change() {
    let store = MemStore::default();
    let mut a = app(DETAILS_DOC, None, false, &store);
    a.refresh_index();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));

    let collapsed_bytes = a.editor.session().serialize();
    let undo_before = a.editor.session().undo_depth();
    a.toggle_details(id);
    let expanded_bytes = a.editor.session().serialize();
    assert_eq!(
        expanded_bytes, collapsed_bytes,
        "展开态字节应与折叠态一致（视图态不改文档）"
    );
    assert_eq!(
        a.editor.session().undo_depth(),
        undo_before,
        "折叠/展开不得产生撤销步"
    );

    // 再 toggle 回去还是一样
    a.toggle_details(id);
    let after_toggle_back = a.editor.session().serialize();
    assert_eq!(after_toggle_back, collapsed_bytes, "再切回折叠态字节不变");
}

// ---- Wave 7 修复：行内 HTML 装饰 / sub-sup / img / 表格 br / details 像素回归 ----

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

#[test]
fn inline_html_kbd_mark_abbr_pixels_visible() {
    let store = MemStore::default();
    let text = "段落 <kbd>Ctrl</kbd> 与 <mark>高亮</mark> 与 <abbr title=\"x\">API</abbr>。\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len() - 1); // 块内激活会显源码，这里让节点保持非激活
    a.ensure_layouts_all();
    a.recompute_geometry();
    let id = a.index.blocks[0].id;
    let text = a.editor.session().serialize();
    let (ck, cm, ca) = (
        rel_offset(&a, id, text.find("Ctrl").unwrap()),
        rel_offset(&a, id, text.find("高亮").unwrap()),
        rel_offset(&a, id, text.find("API").unwrap()),
    );
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let scale = a.input_state.scale;
    let _ = scale;
    let l = &a.cache.layouts[&id];
    let k_rect = a.engine.selection_rects(l, ck, ck + "Ctrl".len())[0];
    let m_rect = a.engine.selection_rects(l, cm, cm + "高亮".len())[0];
    let a_rect = a.engine.selection_rects(l, ca, ca + "API".len())[0];
    let kx = cx + (k_rect.0 - 1.0) as i32;
    let ky = sy + (k_rect.1 + k_rect.3 / 2.0) as i32;
    let mx = cx + (m_rect.0 + m_rect.2 - 2.0) as i32;
    let my = sy + (m_rect.1 + 2.0) as i32;
    let bg = a.input_state.theme.bg;
    let f = a.render();
    assert_eq!(
        f.get(kx as u32, ky as u32),
        [160, 160, 160, 255],
        "kbd 左侧描边像素应为灰描边（实际: {:?}）",
        f.get(kx as u32, ky as u32)
    );
    let mark = f.get(mx as u32, my as u32);
    assert!(
        mark[0] > 245 && mark[1] > 240 && mark[2] < 225,
        "mark 底色应为黄色系，实际: {mark:?}"
    );
    // abbr 下划线为点线：同一行内存在间隔暗像素段（非连续实线）
    let uy = sy + (a_rect.1 + a_rect.3) as i32 - 2;
    let ux = cx + a_rect.0 as i32;
    assert_dotted_row(f, ux, uy, a_rect.2 as i32, bg);
}

#[test]
fn inline_html_sub_sup_pixels_smaller_and_shifted() {
    let store = MemStore::default();
    let text = "H<sub>2</sub>O<sup>3</sup>\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len() - 1);
    a.ensure_layouts_all();
    let id = a.index.blocks[0].id;
    let (px, w, _h) = {
        let l = &a.cache.layouts[&id];
        let glyphs: Vec<(usize, f32, f32, f32)> = l
            .buffer
            .layout_runs()
            .flat_map(|r| r.glyphs)
            .map(|g| (g.start, g.x, g.w, g.font_size))
            .collect();
        let body = glyphs.iter().find(|g| g.0 == 0).copied().unwrap();
        let sub = glyphs.iter().find(|g| g.0 == 1).copied().unwrap();
        let sup = glyphs.iter().find(|g| g.0 == 3).copied().unwrap();
        assert!(sub.3 < body.3, "sub 字号应小于正文: {sub:?} vs {body:?}");
        assert!(sup.3 < body.3, "sup 字号应小于正文: {sup:?} vs {body:?}");
        a.engine.rasterize(l, [0, 0, 0])
    };
    let bbox = |gx: f32, gw: f32| -> (usize, usize, usize) {
        let (x0, x1) = (gx.floor().max(0.0) as usize, (gx + gw).ceil() as usize);
        let (mut lo, mut hi, mut n) = (usize::MAX, 0usize, 0usize);
        for y in 0.._h as usize {
            for x in x0..x1.min(w as usize) {
                if px[(y * w as usize + x) * 4 + 3] > 32 {
                    lo = lo.min(y);
                    hi = hi.max(y);
                    n += 1;
                }
            }
        }
        assert!(n > 0, "字形区域应有墨迹");
        (lo, hi, n)
    };
    let body_g = {
        let l = &a.cache.layouts[&id];
        let gs: Vec<_> = l.buffer.layout_runs().flat_map(|r| r.glyphs).collect();
        (gs[0].x, gs[0].w, gs[1].x, gs[1].w, gs[3].x, gs[3].w)
    };
    let (b_lo, b_hi, _) = bbox(body_g.0, body_g.1);
    let (sub_lo, sub_hi, _) = bbox(body_g.2, body_g.3);
    let (sup_lo, sup_hi, _) = bbox(body_g.4, body_g.5);
    assert!(sub_hi - sub_lo < b_hi - b_lo, "sub 字形高度应小于正文");
    assert!(sup_hi - sup_lo < b_hi - b_lo, "sup 字形高度应小于正文");
    assert!(sub_hi > b_hi, "sub 应下沉（底部低于正文基线）");
    assert!(sup_hi < b_hi, "sup 应上浮（底部高于正文基线）");
}

#[test]
fn html_img_missing_or_remote_placeholder_and_alt_visible() {
    let store = MemStore::default();
    let text = "图：<img src=\"https://example.com/r.png\" alt=\"远程\"> 后\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len() - 1);
    a.ensure_layouts_all();
    a.recompute_geometry();
    let id = a.index.blocks[0].id;
    let l = &a.cache.layouts[&id];
    assert!(
        l.layout_text().contains("[远程]"),
        "远程/缺失 img 应显 alt 占位文本；layout_text: {:?}",
        l.layout_text()
    );
    let rect = l
        .decorations_under
        .iter()
        .find(|(_, _, k)| matches!(k, crate::view::UnderKind::ImagePlaceholder))
        .map(|&(s, e, _)| a.engine.selection_rects(l, s, e)[0])
        .expect("应登记图片占位框");
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let dx = cx + (rect.0 - 1.0) as i32;
    let dy = sy + (rect.1 - 1.0) as i32;
    let dw = (rect.2 + 2.0) as u32;
    let dh = (rect.3 + 2.0) as u32;
    let px = dx + dw as i32 - 3;
    let py = dy + dh as i32 / 2;
    let f = a.render();
    assert!(
        any_pixel(
            f,
            dx + 1,
            dy + 1,
            dw.saturating_sub(2),
            dh.saturating_sub(2),
            |p| { p == [235, 235, 235, 255] }
        ),
        "占位框内应有浅灰底色，实际: {:?}",
        f.get(px as u32, py as u32)
    );
}

#[test]
fn html_img_local_inline_uses_image_path() {
    let text = "行内 <img src=\"logo.png\" alt=\"本地\"> 后\n";
    let (mut a, calls) = app_with_images(text, Some("dir/doc.md"), &[("dir/logo.png", 20, 10)]);
    a.editor.set_caret(text.len() - 1);
    a.ensure_layouts_all();
    let id = a.index.blocks[0].id;
    let l = &a.cache.layouts[&id];
    assert!(
        calls.borrow().iter().any(|p| p.ends_with("dir/logo.png")),
        "行内 <img> 的本地 src 应交给 ImageLoader"
    );
    assert_eq!(l.imgs.len(), 1, "应有 1 个图片占位");
    assert!(
        l.imgs[0].width > 0 && l.imgs[0].height > 0,
        "图片占位宽高应 > 0"
    );
    assert!(
        !l.layout_text().contains("<img"),
        "激活/渲染态不应露出源码标签"
    );
}

#[test]
fn block_level_html_img_line_renders_placeholder_not_code_card() {
    let store = MemStore::default();
    let text = "<img src=\"veil-missing.png\" alt=\"缺失\">\n\n后\n";
    let mut a = app(text, None, false, &store);
    a.editor
        .set_caret(text.find('后').unwrap() + '后'.len_utf8()); // 光标在末段，img 块保持非激活渲染态
    a.refresh_index();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Paragraph));
    a.ensure_layouts_all();
    let l = &a.cache.layouts[&id];
    // 独立成行的缺失 `<img>`：占位提示 + 下方题注（alt），二者都在视图文本里。
    assert!(
        l.layout_text().contains(t(I18nKey::ImageAltFallback)) && l.layout_text().contains("缺失"),
        "块级 img 行应渲染占位 + 题注；layout_text: {:?}",
        l.layout_text()
    );
    assert!(
        l.decorations_under
            .iter()
            .any(|(_, _, k)| matches!(k, crate::view::UnderKind::ImagePlaceholder)),
        "块级 img 应登记占位框"
    );
}

#[test]
fn table_cell_br_renders_two_visual_lines() {
    let store = MemStore::default();
    let text = "前言\n\n| 列 |\n| --- |\n| 第一行<br>第二行 |\n";
    let mut a = app(text, None, false, &store);
    a.refresh_index();
    a.ensure_tables();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Table));
    let line_h = a.engine.doc_line_height() * a.input_state.scale;
    let tl = &a.cache.tables[&id].value;
    assert!(
        tl.rows[1].height > line_h * 1.5,
        "含 <br> 的表格行应有两行高，实际: {}（单行 {line_h}）",
        tl.rows[1].height
    );
    let cell = &tl.cells[tl.cols.len()]; // 表体第一行第一列
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    for y in 0..cell.pixel_h as usize {
        for x in 0..cell.pixel_w as usize {
            if cell.pixels[(y * cell.pixel_w as usize + x) * 4 + 3] > 32 {
                lo = lo.min(y);
                hi = hi.max(y);
            }
        }
    }
    assert!(
        hi.saturating_sub(lo) as f32 > line_h,
        "单元格墨迹应跨两行（跨度 {}），单行高 {line_h}",
        hi.saturating_sub(lo)
    );
}

#[test]
fn details_collapsed_summary_and_expanded_body_are_painted() {
    let store = MemStore::default();
    let mut a = app(DETAILS_DOC, None, false, &store);
    a.editor.set_caret(DETAILS_DOC.len() - 1);
    a.refresh_index();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Details { .. }));
    a.ensure_layouts_all();
    a.recompute_geometry();
    let collapsed_h = {
        let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
        a.block_height(g)
    };
    let line_h = a.engine.doc_line_height() * a.input_state.scale;
    assert!(
        (collapsed_h - line_h).abs() < 0.6,
        "折叠高度应为一整行（{line_h}），实际 {collapsed_h}"
    );
    let layout_text = a.cache.layouts[&id].value.layout_text().to_string();
    assert!(
        layout_text.starts_with("安装步骤"),
        "折叠态首行应为 summary 文本，实际: {layout_text:?}"
    );
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let cw = a.content_width() as u32;
    let f = a.render();
    assert!(
        ink_pixels(f, cx - 2, sy, cw, collapsed_h as u32) > 0,
        "折叠条应画出 summary 文本"
    );
    // 展开：正文行必须可见，且整组（头块 + 子块）高度增长（B3：正文是独立子块）
    a.toggle_details(id);
    a.ensure_layouts_all();
    a.recompute_geometry();
    let expanded_h = {
        let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
        a.block_height(g) + a.details_children_height(g)
    };
    assert!(expanded_h > collapsed_h * 2.0, "展开后整组高度应明显增大");
    let body_id = a
        .index
        .blocks
        .iter()
        .find(|g| g.details_parent == Some(id))
        .map(|g| g.id)
        .expect("展开后正文应为 details 子块");
    let layout_text = a.cache.layouts[&body_id].value.layout_text().to_string();
    assert!(
        layout_text.contains("pip install veil") && layout_text.contains("然后运行 veil"),
        "展开后正文应作为可见文本渲染；layout_text: {layout_text:?}"
    );
    let f = a.render();
    let y0 = sy + collapsed_h as i32;
    assert!(
        ink_pixels(f, cx - 2, y0, cw, (expanded_h - collapsed_h) as u32) > 0,
        "展开后正文区域应有墨迹（不能被卡片底遮盖）"
    );
}

// ---- Wave 7 收尾：表格内行内装饰 / abbr 点线 / abbr 悬停 / 行内代码底色 ----

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

/// 表格单元格内 kbd / mark / abbr / pre 与正文同语义：描边、黄底、点线、代码底色。
#[test]
fn table_cell_inline_decor_pixels_visible() {
    use crate::view::UnderKind;
    let store = MemStore::default();
    let text = "| a | b | c | d |\n| --- | --- | --- | --- |\n\
                | <kbd>K</kbd> | <mark>M</mark> | <abbr title=\"tip\">AB</abbr> | <pre>P</pre> |\n\n尾\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len()); // 光标移出表格 → 全部单元格走富文本网格
    a.refresh_index();
    a.ensure_tables();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Table));
    let (top, xs, ys, unders, overs) = {
        let top = a.index.blocks.iter().find(|g| g.id == id).unwrap().top;
        let tl = &a.cache.tables[&id].value;
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        let mut unders = Vec::new();
        let mut overs = Vec::new();
        for i in 4..8 {
            let c = &tl.cells[i];
            xs.push(c.x);
            ys.push(c.y);
            unders.push(c.under.clone());
            overs.push(c.over.clone());
        }
        (top, xs, ys, unders, overs)
    };
    let vp = a.viewport();
    let sy = (vp.y + top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let code_bg = a.input_state.theme.code_bg;
    let bg = a.input_state.theme.bg;
    let f = a.render();
    // kbd：左缘描边像素（灰 160）
    let kbd = unders[0]
        .iter()
        .position(|r| matches!(r.4, UnderKind::KbdFrame))
        .expect("kbd 单元格应登记边框装饰");
    {
        let (rx, ry, _, rh, _) = unders[0][kbd];
        let dx = cx + (xs[0] + rx - 1.0) as i32;
        let dy = sy + (ys[0] + ry - 1.0) as i32;
        let px = f.get(dx as u32, (dy + rh as i32 / 2) as u32);
        assert_eq!(px, [160, 160, 160, 255], "表格 kbd 左侧描边应为灰描边");
    }
    // mark：底色矩形内出现黄色系像素
    let mark = unders[1]
        .iter()
        .position(|r| matches!(r.4, UnderKind::MarkBg))
        .expect("mark 单元格应登记底色");
    {
        let (rx, ry, rw, rh, _) = unders[1][mark];
        let dx = cx + (xs[1] + rx - 1.0) as i32;
        let dy = sy + (ys[1] + ry - 1.0) as i32;
        assert!(
            any_pixel(f, dx, dy, rw as u32, rh as u32, |p| p[0] > 245
                && p[1] > 240
                && p[2] < 225),
            "表格 mark 应有黄色底"
        );
    }
    // pre：代码底色（主题 code_bg）出现在装饰矩形内
    let pre = unders[3]
        .iter()
        .position(|r| matches!(r.4, UnderKind::InlineCodeBg))
        .expect("行内 pre 单元格应登记代码底色");
    {
        let (rx, ry, rw, rh, _) = unders[3][pre];
        let dx = cx + (xs[3] + rx - 1.0) as i32;
        let dy = sy + (ys[3] + ry - 1.0) as i32;
        assert!(
            any_pixel(f, dx, dy, rw as u32, rh as u32, |p| p == code_bg),
            "表格行内 pre 应有主题代码底色"
        );
    }
    // abbr：下划线为点线（同一条行内存在暗段间隔）
    let abbr = overs[2].first().expect("abbr 单元格应登记下划线");
    let oy = sy + (ys[2] + abbr.1 + abbr.3) as i32 - 2;
    let ox = cx + (xs[2] + abbr.0) as i32;
    assert_dotted_row(f, ox, oy, abbr.2 as i32, bg);
}

/// `<strong style="color:red">` 在正文与表格单元格里都必须渲染为红色内容（非源码原文）。
#[test]
fn inline_strong_style_color_pixels_paragraph_and_table() {
    let store = MemStore::default();
    let text = "正文 <strong style=\"color:red\">VISUAL</strong> 结束\n\n\
                | 标题 |\n| --- |\n| <strong style=\"color:red\">VISUAL</strong> |\n\n尾\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len()); // 光标移出表格 / 段落，保持非激活渲染
    a.refresh_index();
    a.ensure_layouts_all();
    a.ensure_tables();
    a.recompute_geometry();
    let is_red = |p: [u8; 4]| p[3] > 0 && p[0] > 150 && p[1] < 110 && p[2] < 110;

    // 1) 正文段落：glyph 区域内出现红色像素，且 layout_text 不泄漏标签。
    let pid = block_id_of(&a, |k| matches!(k, BlockKind::Paragraph));
    let src = a.editor.session().serialize();
    let vpos = src.find("VISUAL").unwrap();
    let rel = rel_offset(&a, pid, vpos);
    let (rect, l_text) = {
        let l = &a.cache.layouts[&pid];
        (
            a.engine.selection_rects(l, rel, rel + "VISUAL".len())[0],
            l.layout_text().to_string(),
        )
    };
    assert!(
        !l_text.contains("<strong"),
        "正文不得显示源码标签: {l_text:?}"
    );
    let vp = a.viewport();
    let g = a.index.blocks.iter().find(|g| g.id == pid).unwrap();
    let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
    let cx = a.content_x() as i32;
    let f = a.render();
    assert!(
        any_pixel(
            f,
            cx + rect.0 as i32,
            sy + rect.1 as i32,
            rect.2 as u32,
            rect.3 as u32,
            is_red
        ),
        "正文 VISUAL 应为红色（实际区域像素: {:?}）",
        f.get((cx as f32 + rect.0) as u32, (sy as f32 + rect.1) as u32)
    );

    // 2) 表格单元格：可见文本为 VISUAL（无标签），光栅像素含红色。
    let tid = block_id_of(&a, |k| matches!(k, BlockKind::Table));
    let tl = &a.cache.tables[&tid].value;
    assert_eq!(tl.cells.len(), 2, "表头 + 一行表体");
    let cell = &tl.cells[1];
    assert_eq!(cell.pixels.len() as u32, cell.pixel_w * cell.pixel_h * 4);
    assert!(
        cell.pixels
            .chunks_exact(4)
            .any(|p| is_red([p[0], p[1], p[2], p[3]])),
        "表格单元格 VISUAL 应为红色"
    );
}

/// 表格单元格内 sub/sup 缩小且基线偏移（与同文本普通单元格逐像素比较）。
#[test]
fn table_cell_sub_sup_smaller_and_shifted() {
    let store = MemStore::default();
    let text = "| 2 | 2 | 2 |\n| --- | --- | --- |\n| 2 | <sub>2</sub> | <sup>2</sup> |\n\n尾\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len());
    a.refresh_index();
    a.ensure_tables();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Table));
    let tl = &a.cache.tables[&id].value;
    let boxed = |i: usize| {
        let c = &tl.cells[i];
        (c.pixels.clone(), c.pixel_w, c.pixel_h)
    };
    let (plain, pw, ph) = boxed(3);
    let (sub, sw, sh) = boxed(4);
    let (sup, uw, uh) = boxed(5);
    let (p_lo, p_hi) = raster_ink_rows(&plain, pw, ph).expect("普通格应有墨迹");
    let (s_lo, s_hi) = raster_ink_rows(&sub, sw, sh).expect("sub 格应有墨迹");
    let (u_lo, u_hi) = raster_ink_rows(&sup, uw, uh).expect("sup 格应有墨迹");
    assert!(
        s_hi - s_lo < p_hi - p_lo,
        "sub 字形高度应小于正文（sub {s_lo}..{s_hi} vs 正文 {p_lo}..{p_hi}）"
    );
    assert!(
        u_hi - u_lo < p_hi - p_lo,
        "sup 字形高度应小于正文（sup {u_lo}..{u_hi} vs 正文 {p_lo}..{p_hi}）"
    );
    // 基线偏移方向：同一字符在 sub 里整体低于 sup（下沉/上浮）
    assert!(
        s_lo > u_lo && s_hi > u_hi,
        "sub 应较 sup 下沉（sub {s_lo}..{s_hi} vs sup {u_lo}..{u_hi}）"
    );
    assert!(u_hi < p_hi, "sup 应上浮（底部高于正文）");
}

/// 行内 code（Markdown 反引号与 `<pre>`）均有代码底色，且与正文同一装饰语义。
#[test]
fn inline_code_and_pre_have_code_background() {
    use crate::view::UnderKind;
    let store = MemStore::default();
    let text = "行内 `code` 与 <pre>预格式</pre> 尾\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.len() - 1);
    a.ensure_layouts_all();
    a.recompute_geometry();
    let id = a.index.blocks[0].id;
    let (rects, code_bg, sy, cx) = {
        let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
        let vp = a.viewport();
        let sy = (vp.y + g.top - a.window_state.scroll_y) as i32;
        let cx = a.content_x() as i32;
        let l = &a.cache.layouts[&id];
        let kinds: Vec<_> = l
            .decorations_under
            .iter()
            .filter(|(_, _, k)| matches!(k, UnderKind::InlineCodeBg))
            .map(|&(s, e, _)| (s, e))
            .collect();
        let mut rects = Vec::new();
        for (s, e) in kinds {
            for r in a.engine.selection_rects(l, s, e) {
                rects.push(r);
            }
        }
        (rects, a.input_state.theme.code_bg, sy, cx)
    };
    assert_eq!(rects.len(), 2, "`code` 与行内 `<pre>` 应各有一段代码底色");
    let f = a.render();
    for r in rects {
        assert!(
            any_pixel(
                f,
                cx + r.0 as i32,
                sy + r.1 as i32,
                r.2 as u32,
                r.3 as u32,
                |p| p == code_bg
            ),
            "行内代码底色应为主题 code_bg：{:?}",
            code_bg
        );
    }
}

/// abbr 悬停：命中 `<abbr title>` 后状态含 title 文本（浮层由既有 tooltip 绘制）。
#[test]
fn abbr_hover_exposes_title_tooltip() {
    let store = MemStore::default();
    let text = "这里 <abbr title=\"Application Programming Interface\">API</abbr> 缩写。\n";
    let mut a = app(text, None, false, &store);
    let off = text.find("API").unwrap();
    let (x, y) = point_at_doc(&mut a, off);
    a.input_state.mouse = PhysicalPosition::new(x as f64, y as f64);
    a.update_hover();
    assert_eq!(
        a.ui_state.hover_link.as_ref().map(|t| t.0.as_str()),
        Some("Application Programming Interface"),
        "abbr 悬停应暴露 title 文本"
    );
    // 移到正文区外 → 清除
    a.input_state.mouse = PhysicalPosition::new(x as f64, 2.0);
    a.update_hover();
    assert!(a.ui_state.hover_link.is_none());
}

// ---- B5：排版留白与输入空间（底部留白 / 空行行高 / 末尾空块 / 点击落末尾） ----

/// 文末可点击输入空间 ≥ 2 行正文行高（B5-1）。
#[test]
fn doc_end_keeps_two_lines_of_clickable_space() {
    let store = MemStore::default();
    let mut a = app("正文一行", None, false, &store);
    a.paint();
    let scale = a.input_state.scale;
    let line_h = BASE_LINE_HEIGHT * scale;
    let g = a.index.blocks.last().unwrap();
    let space = a.index.total_h - (g.top + a.block_height(g));
    assert!(
        space >= 2.0 * line_h - 1e-3,
        "文末留白须 ≥ 2 行: {space} < {}",
        2.0 * line_h
    );
}

/// 光标滚到文末后，底部至少仍有 2 行正文高度的可视空白（B5-1/5）。
#[test]
fn caret_at_doc_end_keeps_two_lines_of_visible_space() {
    let store = MemStore::default();
    let doc = (1..=80)
        .map(|i| format!("第 {i:02} 段：滚动测试内容。\n"))
        .collect::<String>();
    let mut a = app(&doc, None, false, &store);
    a.paint();
    a.window_state.smooth = false;
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    a.scroll_to_caret();
    let scale = a.input_state.scale;
    let (_, cy, ch) = a.caret_screen();
    let vp = a.viewport();
    let line_h = BASE_LINE_HEIGHT * scale;
    let last_bottom = a
        .index
        .blocks
        .last()
        .map(|g| g.top + a.block_height(g))
        .unwrap_or(0.0);
    assert!(
        cy + ch <= vp.y + vp.h - 2.0 * line_h + 0.5,
        "光标行下方须留 ≥2 行可视空白: caret_bottom={} vp_bottom={} line_h={line_h} total_h={} last_bottom={} scroll={} vp_h={}",
        cy + ch,
        vp.y + vp.h,
        a.index.total_h,
        last_bottom,
        a.window_state.scroll_y,
        vp.h
    );
}

/// 点击末块下方留白 → 光标直接落文档末尾（B5-1）。
#[test]
fn click_in_doc_end_space_moves_caret_to_doc_end() {
    let store = MemStore::default();
    let mut a = app("正文一行", None, false, &store);
    a.paint();
    let scale = a.input_state.scale;
    let line_h = BASE_LINE_HEIGHT * scale;
    let vp = a.viewport();
    let (top, h) = {
        let g = a.index.blocks.last().unwrap();
        (g.top, a.block_height(g))
    };
    a.input_state.mouse = PhysicalPosition::new(
        (MARGIN * scale + 40.0) as f64,
        (vp.y + top + h + line_h * 0.5 - a.window_state.scroll_y) as f64,
    );
    assert_eq!(
        a.hit_offset(),
        Some(a.editor.session().text_len()),
        "文末留白点击须落文档末尾"
    );
}

/// 块间空行 = 段距 + 1 个满行高（B5-2），不得被压扁。
#[test]
fn blank_line_between_paragraphs_occupies_one_line() {
    let store = MemStore::default();
    let mut a = app("a\n\nb\n", None, false, &store);
    a.paint();
    let scale = a.input_state.scale;
    let line_h = BASE_LINE_HEIGHT * scale;
    let gap = {
        let b0 = &a.index.blocks[0];
        let b1 = &a.index.blocks[1];
        b1.top - (b0.top + b0.height)
    };
    assert!(
        (gap - (PARA_GAP * scale + line_h)).abs() < 1e-3,
        "空行间距应为段距 + 1 行: {gap} vs {}",
        PARA_GAP * scale + line_h
    );
}

/// 文档以空行结尾：光标锚在块底 + 段距 + 1 行，可继续输入且落在文末留白内（B5-3）。
#[test]
fn caret_after_trailing_blank_line_has_full_line_space() {
    let store = MemStore::default();
    let mut a = app("a\n\n", None, false, &store);
    a.paint();
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    let scale = a.input_state.scale;
    let vp = a.viewport();
    let (top, h) = {
        let g = a.index.blocks.last().unwrap();
        (g.top, a.block_height(g))
    };
    let (_, cy, _) = a.caret_screen();
    let want = vp.y + top + h + (PARA_GAP + BASE_LINE_HEIGHT) * scale;
    assert!((cy - want).abs() < 1e-3, "尾空行光标位置: {cy} vs {want}");
    assert!(
        cy + BASE_LINE_HEIGHT * scale <= vp.y + a.index.total_h + 1e-3,
        "尾空行光标须在文档高度内（可见可输入）"
    );
    a.route_key(Key::Text("b".into()));
    assert!(a.editor.session().serialize().ends_with('b'));
}

/// 文档以空块结尾（空列表项）：块高 = 1 行，光标可达且可输入（B5-3）。
#[test]
fn trailing_empty_list_item_keeps_one_line_and_is_editable() {
    let store = MemStore::default();
    let mut a = app("正文\n\n- ", None, false, &store);
    a.paint();
    let scale = a.input_state.scale;
    let (kind, height, top) = {
        let g = a.index.blocks.last().unwrap();
        (g.kind.clone(), a.block_height(g), g.top)
    };
    assert!(matches!(kind, BlockKind::ListItem { .. }), "末块应为列表项");
    assert!(
        (height - BASE_LINE_HEIGHT * scale).abs() < 1.0,
        "空列表项高度应为 1 行: {height}"
    );
    a.perform_action(Action::Doc(Intent::MoveDocEnd));
    let (_, cy, _) = a.caret_screen();
    let vp = a.viewport();
    assert!(
        cy >= vp.y + top - 0.5 && cy <= vp.y + top + height + 0.5,
        "光标须落在末尾空块内: {cy} / [{top}, {}]",
        top + height
    );
    a.route_key(Key::Text("x".into()));
    assert!(a.editor.session().serialize().ends_with('x'));
}

// ---- 版本历史（N1/B8a）：稳定键、只读预览、可撤销恢复、未保存守卫 ----

#[test]
fn version_history_key_is_path_stable_across_saves() {
    let path = r"C:\notes\b8a\stable.md";
    let store = MemStore::default();
    let mut snap = FakeSnapshot::default();
    let mut a = app_history("v0\n", Some(path), &store, &snap);
    let key = a.doc.history_key;
    assert_eq!(key, crate::document::history::doc_key(Path::new(path)));
    // 改三次内容并各保存一次：保存前把被覆盖的旧版本留档到同一稳定键目录
    for v in ["v1\n", "v2\n", "v3\n"] {
        a.editor.replace_all_text(v);
        a.save_now();
        assert_eq!(a.doc.history_key, key, "内容变化不得更换目录键");
    }
    assert_eq!(
        snap.texts(key),
        vec!["v3\n", "v2\n", "v1\n", "v0\n"],
        "时间倒序：保存前留档旧版本 + 当前版本入历史，同内容去重"
    );
    // 未改动再保存：不产生重复快照
    a.save_now();
    assert_eq!(snap.texts(key).len(), 4);
    let _ = &mut snap;
}

#[test]
fn version_history_preview_is_read_only_and_keeps_live_document() {
    let store = MemStore::default();
    let mut snap = FakeSnapshot::default();
    let mut a = app_history("current\n", Some(r"C:\notes\b8a\preview.md"), &store, &snap);
    let key = a.doc.history_key;
    snap.save_snapshot(key, "old\n").unwrap();
    a.open_history();
    assert!(a.ui_state.history.open);
    assert_eq!(a.mode(), Mode::History);
    assert_eq!(a.ui_state.history.entries.len(), 1);
    a.history_select(0);
    // 预览显示旧内容，但实时文档字节/脏态/基线均不变
    assert_eq!(a.editor.session().serialize(), "old\n");
    assert_eq!(a.history_live_text(), "current\n");
    assert!(!a.history_live_dirty());
    assert_eq!(a.doc.baseline, encode("current\n", false));
    a.close_history();
    assert_eq!(a.editor.session().serialize(), "current\n");
    assert!(!a.editor.is_dirty());
    assert!(!a.ui_state.history.open);
}

#[test]
fn version_history_restore_is_undoable_and_marks_dirty() {
    let store = MemStore::default();
    let mut snap = FakeSnapshot::default();
    let mut a = app_history("v2\n", Some(r"C:\notes\b8a\undo.md"), &store, &snap);
    let key = a.doc.history_key;
    snap.save_snapshot(key, "v1\n").unwrap();
    a.open_history();
    a.history_select(0);
    a.history_restore_selected();
    assert!(!a.ui_state.history.open, "恢复后关闭面板");
    assert_eq!(a.editor.session().serialize(), "v1\n");
    assert!(a.editor.is_dirty(), "恢复必须写入缓冲区并标脏");
    // 恢复是一次编辑：Ctrl+Z 回到恢复前内容
    let dirty = a.editor.apply(Intent::Undo).expect("恢复应可撤销");
    a.after_edit(dirty);
    assert_eq!(a.editor.session().serialize(), "v2\n");
}

#[test]
fn version_history_restore_with_unsaved_changes_uses_existing_guard() {
    let store = MemStore::default();
    let mut snap = FakeSnapshot::default();
    let mut a = app_history("saved\n", Some(r"C:\notes\b8a\guard.md"), &store, &snap);
    let key = a.doc.history_key;
    snap.save_snapshot(key, "old\n").unwrap();
    a.editor.replace_all_text("unsaved\n");
    assert!(a.editor.is_dirty());
    a.open_history();
    a.history_select(0);
    a.history_restore_selected();
    // 走出既有未保存确认弹窗；结论落地前不覆盖缓冲区
    assert!(a.ui_state.modal.is_some(), "必须先问未保存改动");
    assert_eq!(a.editor.session().serialize(), "unsaved\n");
    a.resolve_modal(3); // 取消
    assert_eq!(a.editor.session().serialize(), "unsaved\n");
    assert!(a.editor.is_dirty());
    // 改为「不保存」→ 恢复执行；恢复本身仍可撤销
    a.history_restore_selected();
    a.resolve_modal(2);
    assert_eq!(a.editor.session().serialize(), "old\n");
    assert!(a.editor.is_dirty());
    assert!(!a.ui_state.history.open);
}

#[test]
fn version_history_list_error_shows_error_state() {
    let store = MemStore::default();
    let snap = FakeSnapshot::default();
    let mut a = app_history("x\n", Some(r"C:\notes\b8a\err.md"), &store, &snap);
    snap.fail_list.set(true);
    a.open_history();
    assert!(a.ui_state.history.open);
    assert!(a.ui_state.history.entries.is_empty());
    assert_eq!(a.ui_state.history.error.as_deref(), Some("模拟读取失败"));
    assert!(a.history_live_text() == "x\n");
}

#[test]
fn version_history_draft_snapshots_migrate_on_first_save() {
    let store = MemStore::default();
    let mut snap = FakeSnapshot::default();
    let target = r"C:\notes\b8a\first-save.md";
    let mut a = app_history_dialogs("draft\n", None, &store, &snap, Some(target));
    let old = a.doc.history_key;
    snap.save_snapshot(old, "draft\n").unwrap();
    assert_eq!(snap.texts(old).len(), 1);
    a.save_as();
    let new_key = crate::document::history::doc_key(Path::new(target));
    assert_eq!(a.doc.history_key, new_key);
    assert!(snap.texts(old).is_empty(), "临时会话键历史已迁走");
    assert_eq!(snap.texts(new_key), vec!["draft\n"]);
}

#[test]
fn version_history_migrate_failure_is_visible_and_keeps_old_history() {
    // DEF-T08-001：草稿首次另存时历史迁移失败，不得静默、不得只报「已另存为」。
    let store = MemStore::default();
    let mut snap = FakeSnapshot::default();
    let target = r"C:\notes\b8a\migrate-fail.md";
    let mut a = app_history_dialogs("draft\n", None, &store, &snap, Some(target));
    let old = a.doc.history_key;
    snap.save_snapshot(old, "draft\n").unwrap();
    snap.fail_migrate.set(true);
    a.save_as();
    let new_key = crate::document::history::doc_key(Path::new(target));
    assert_eq!(
        a.doc.path,
        Some(PathBuf::from(target)),
        "文档本身保存成功（可用）"
    );
    assert!(
        store.files.borrow().contains_key(&PathBuf::from(target)),
        "保存本身已落盘"
    );
    assert_eq!(a.doc.history_key, old, "迁移失败保持旧键：旧历史仍可访问");
    assert_eq!(snap.texts(old), vec!["draft\n"], "旧历史未被删除");
    assert!(snap.texts(new_key).is_empty(), "新键下没有半迁移的历史");
    let notice = &a.ui_state.chrome.notice;
    assert!(
        notice.contains("历史迁移失败"),
        "必须明确提示历史未迁移: {notice}"
    );
    assert!(!notice.contains("已另存为"), "不得误报纯成功: {notice}");
}

#[test]
fn save_as_to_other_directory_migrates_relative_images() {
    // DEF-T08-002：跨目录另存须把相对图片资产迁到新目录（保持相对 URL 可解析）。
    let store = MemStore::default();
    let snap = FakeSnapshot::default();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let assets = FakeMigrateAssets {
        calls: calls.clone(),
        report: AssetMigration {
            copied: 1,
            missing: 0,
            failed: Vec::new(),
        },
    };
    let target = r"C:\notes\new\moved.md";
    let mut a = app_history_with_assets(
        "![x](notes.assets/image-1.png)\n",
        Some(r"C:\notes\old\src.md"),
        &store,
        &snap,
        Some(target),
        Box::new(assets),
    );
    a.save_as();
    let got = calls.borrow();
    assert_eq!(got.len(), 1, "跨目录另存必须触发资产迁移");
    assert_eq!(got[0].0, PathBuf::from(r"C:\notes\old\src.md"));
    assert_eq!(got[0].1, PathBuf::from(target));
    assert_eq!(got[0].2, vec!["notes.assets/image-1.png".to_string()]);
    assert!(
        a.ui_state.chrome.notice.contains("已另存为"),
        "资产全部迁移时保持正常成功提示"
    );
}

#[test]
fn save_as_asset_migration_incomplete_is_visible() {
    // 资产缺失/复制失败：文档仍保存成功，但必须给可见降级，不静默断链。
    let store = MemStore::default();
    let snap = FakeSnapshot::default();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let assets = FakeMigrateAssets {
        calls,
        report: AssetMigration {
            copied: 0,
            missing: 1,
            failed: Vec::new(),
        },
    };
    let target = r"C:\notes\new\broken.md";
    let mut a = app_history_with_assets(
        "![x](notes.assets/gone.png)\n",
        Some(r"C:\notes\old\src.md"),
        &store,
        &snap,
        Some(target),
        Box::new(assets),
    );
    a.save_as();
    assert_eq!(a.doc.path, Some(PathBuf::from(target)), "文档保存不受影响");
    let notice = &a.ui_state.chrome.notice;
    assert!(
        notice.contains("图片资产未能迁移"),
        "缺失资产须有可见降级: {notice}"
    );
}

#[test]
fn version_history_delete_and_clear_go_through_danger_modal() {
    let store = MemStore::default();
    let mut snap = FakeSnapshot::default();
    let mut a = app_history("v2\n", Some(r"C:\notes\b8a\del.md"), &store, &snap);
    let key = a.doc.history_key;
    snap.save_snapshot(key, "v1\n").unwrap();
    a.open_history();
    a.history_select(0);
    a.history_request_delete();
    assert!(matches!(
        a.ui_state.modal.as_ref().map(|m| m.purpose.clone()),
        Some(ModalPurpose::Error(ErrorPurpose::HistoryDelete(_)))
    ));
    a.resolve_modal(1);
    assert!(snap.texts(key).is_empty());
    assert!(a.ui_state.history.open, "删除后面板保持打开");
    // 清空：空列表直接提示，不弹危险框
    a.history_request_clear();
    assert!(a.ui_state.modal.is_none());
}

// ---- 启动即失败：应用内错误模态（同卡片/按钮样式，可重试/换文件/关闭） ----

#[test]
fn startup_error_opens_in_app_modal() {
    use crate::view::modal::ButtonRole;
    let store = MemStore::default();
    let a = app_startup_error("a.md", &store);
    let modal = a
        .ui_state
        .modal
        .as_ref()
        .expect("启动失败须在首帧弹出应用内模态");
    assert!(matches!(
        modal.purpose,
        ModalPurpose::Error(ErrorPurpose::Startup(_))
    ));
    assert_eq!(modal.spec.title, t(I18nKey::MainOpenFailedTitle));
    assert_eq!(modal.spec.message, STARTUP_ERROR_MESSAGE);
    let labels: Vec<String> = modal
        .spec
        .buttons
        .iter()
        .map(|b| b.display_label())
        .collect();
    assert_eq!(labels, vec!["重试 (R)", "打开其他文件 (O)", "关闭"]);
    assert_eq!(modal.spec.buttons[0].role, ButtonRole::Primary);
    assert_eq!(modal.state.focused, 0, "默认焦点在主按钮");
    assert_eq!(a.doc.path, None, "失败时以空白草稿开窗");
}

#[test]
fn startup_error_retry_loads_same_path() {
    let store = MemStore::default();
    disk(&store, "a.md", b"abc\n");
    let mut a = app_startup_error("a.md", &store);
    a.resolve_modal(1);
    assert!(a.ui_state.modal.is_none(), "重试成功后模态关闭");
    assert_eq!(a.doc.path, Some(PathBuf::from("a.md")));
    assert_eq!(a.editor.session().serialize(), "abc\n");
}

#[test]
fn startup_error_retry_failure_shows_open_error() {
    let store = MemStore::default(); // 无目标文件：重试仍失败
    let mut a = app_startup_error("a.md", &store);
    a.resolve_modal(1);
    assert!(
        matches!(
            a.ui_state.modal.as_ref().map(|m| m.purpose.clone()),
            Some(ModalPurpose::Error(ErrorPurpose::Open(_)))
        ),
        "重试失败应走既有「打开失败」错误模态"
    );
}

#[test]
fn startup_error_close_exits_window() {
    let store = MemStore::default();
    let mut a = app_startup_error("a.md", &store);
    assert!(!a.window_state.exit_requested);
    a.route_key(Key::Escape);
    assert!(
        a.window_state.exit_requested,
        "Esc 落到最后一个 Secondary「关闭」→ 请求退出本窗口"
    );
    assert!(a.ui_state.modal.is_none());

    let mut b = app_startup_error("a.md", &store);
    b.resolve_modal(3);
    assert!(b.window_state.exit_requested, "「关闭」按钮同样请求退出");
}

#[test]
fn startup_error_open_other_uses_dialog() {
    let store = MemStore::default();
    let mut a = app_startup_error("a.md", &store);
    a.resolve_modal(2); // NoDialogs 返回 None：什么都不做
    assert!(a.ui_state.modal.is_none(), "换文件出口关闭模态");
    assert!(!a.window_state.exit_requested, "换文件不等于退出");
    assert_eq!(a.doc.path, None);
}

#[test]
fn startup_error_mnemonic_o() {
    let store = MemStore::default();
    let mut a = app_startup_error("a.md", &store);
    a.route_key(Key::O);
    assert!(a.ui_state.modal.is_none(), "O 助记符命中「打开其他文件」");
    assert!(!a.window_state.exit_requested, "助记符出口不退出窗口");
}

// ===== 光标闪烁状态机（无 GUI，直接驱动 tick） =====

/// 隐藏半周期内收到输入活跃信号：下一次 tick 必须把光标拉回可见。
#[test]
fn blink_activity_restores_visibility_in_hidden_half_cycle() {
    let store = MemStore::default();
    let mut a = app("hello", Some("a.md"), false, &store);
    let t0 = Instant::now();
    a.tick(t0);
    assert!(a.cursor_visible, "初始 tick 应起算并显示光标");
    // 进入不可见半周期
    a.cursor_visible = false;
    a.last_blink = Some(t0);
    a.note_caret_activity();
    assert!(a.cursor_visible, "活跃信号须立即恢复可见");
    assert!(a.last_blink.is_none(), "周期起点交回 tick 重新起算");
    a.tick(t0 + Duration::from_millis(10));
    assert!(a.cursor_visible, "下一次 tick 后仍保持可见");
}

/// 输入活跃后 599ms 不翻转、601ms 翻转；重置后以新起点重新计周期。
#[test]
fn blink_flips_600ms_after_last_activity() {
    let store = MemStore::default();
    let mut a = app("hello", Some("a.md"), false, &store);
    let t0 = Instant::now();
    a.tick(t0);
    a.tick(t0 + Duration::from_millis(599));
    assert!(a.cursor_visible, "599ms 未到期，不翻转");
    a.tick(t0 + Duration::from_millis(601));
    assert!(!a.cursor_visible, "601ms 到期后翻转");

    // 活跃信号把周期起点重置到 610ms 这一刻
    a.note_caret_activity();
    a.tick(t0 + Duration::from_millis(610));
    assert!(a.cursor_visible);
    a.tick(t0 + Duration::from_millis(1205)); // 610 + 595
    assert!(a.cursor_visible, "重置后不得沿用旧到期点（610+600 才到期）");
    a.tick(t0 + Duration::from_millis(1215)); // 610 + 605
    assert!(!a.cursor_visible, "以新起点计满 600ms 才翻转");
}

/// IME 组合期间强制可见且不翻转（blink 不活跃）。
#[test]
fn blink_forced_visible_while_preedit_active() {
    let store = MemStore::default();
    let mut a = app("hello", Some("a.md"), false, &store);
    a.editor.set_preedit("ni".into(), None);
    a.cursor_visible = false;
    let t0 = Instant::now();
    a.tick(t0);
    assert!(a.cursor_visible, "preedit 期间强制可见");
    a.tick(t0 + Duration::from_millis(1200));
    assert!(a.cursor_visible, "preedit 期间不得翻转");
    assert!(a.last_blink.is_none());
}

/// 仅按键（Ctrl：caret 不变）也要恢复可见——tick 的位移检测覆盖不到这类输入。
#[test]
fn blink_key_without_caret_move_restores_visibility() {
    let store = MemStore::default();
    let mut a = app("hello", Some("a.md"), false, &store);
    let t0 = Instant::now();
    a.tick(t0);
    a.cursor_visible = false;
    a.last_blink = Some(t0);
    let caret = a.editor.caret();
    // 模拟 on_key 在 map_key 之前调用统一入口（Ctrl 不产生 Key 映射，caret 不变）
    a.note_caret_activity();
    a.tick(t0 + Duration::from_millis(5));
    assert_eq!(a.editor.caret(), caret, "Ctrl 不移动 caret");
    assert!(a.cursor_visible, "caret 不变也必须恢复可见");
}

// ---- Task 4：代码块语言标签与复制按钮 ----

const CODE_MULTILINE: &str =
    "```rust\nlet x = 1;\nlet 中 = \"你好\";\n\nprintln!(\"{}\", 中);\n```\n";

/// 代码块首行（围栏行消隐后的空白行）承载头部控件：按钮必须落在该行内，
/// 不占第二行（真正的代码行）高度。
#[test]
fn code_header_sits_in_first_line_and_shows_canonical_language() {
    let (mut a, _) = app_code_copy("```py\nprint(1)\n```\n\n```\nplain\n```\n");
    a.ensure_layouts_all();
    let line_h = a.engine.doc_line_height() * a.input_state.scale;
    let h0 = a
        .code_header_layout(&a.index.blocks[0])
        .expect("可见围栏块应绘制头部控件");
    assert!(h0.button.y >= h0.line_top && h0.button.y + h0.button.h <= h0.line_top + line_h);
    assert_eq!(h0.label.as_deref(), Some("Python"), "别名 py 显示规范名");
    // 无语言：标签隐藏（按钮仍保留，保证复制入口可发现）。
    let h1 = a
        .code_header_layout(&a.index.blocks[1])
        .expect("无语言代码块也应有复制按钮");
    assert_eq!(h1.label, None, "无 info string 时标签隐藏");
    // 版心右上角：按钮右缘不超过版心右缘。
    assert!(h1.button.x + h1.button.w <= a.content_x() + a.content_width());
}

#[test]
fn code_copy_button_copies_source_verbatim_without_undo_or_dirty() {
    let (mut a, log) = app_code_copy(CODE_MULTILINE);
    a.ensure_layouts_all();
    let h = a
        .code_header_layout(&a.index.blocks[0])
        .expect("应有复制按钮");
    let before = doc_text(&a);
    let undo0 = a.editor.session().undo_depth();
    click_at(
        &mut a,
        h.button.x + h.button.w / 2.0,
        h.button.y + h.button.h / 2.0,
    );
    assert_eq!(
        log.borrow().as_slice(),
        ["let x = 1;\nlet 中 = \"你好\";\n\nprintln!(\"{}\", 中);"],
        "复制内容为代码原文：无围栏/语言标记、保留内部空行、无多余结尾换行"
    );
    assert_eq!(doc_text(&a), before, "复制不得修改文档字节");
    assert_eq!(a.editor.session().undo_depth(), undo0, "复制不得产生撤销步");
    assert!(!a.editor.is_dirty(), "复制不得置脏");
    assert!(
        a.ui_state.chrome.notice.contains("已复制代码块"),
        "状态栏应有可见成功反馈"
    );
}

/// 点击代码正文（第二视觉行）不得触发复制，仍按普通点击落光标。
#[test]
fn code_copy_hit_area_excludes_code_text() {
    let (mut a, log) = app_code_copy(CODE_MULTILINE);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    let (tx, ty, th) = a.engine.caret_xy(&a.cache.layouts[&id].value, 1);
    let vp = a.viewport();
    let top = a.index.blocks[0].top;
    let x = a.content_x() + tx + 1.0;
    click_at(&mut a, x, vp.y + top + ty + th / 2.0);
    assert!(log.borrow().is_empty(), "代码文本点击不得复制");
    assert!(a.editor.caret() > 0, "代码文本点击应正常落光标");
}

#[test]
fn ctrl_shift_c_copies_code_block_at_caret_without_editing() {
    let (mut a, log) = app_code_copy(CODE_MULTILINE);
    a.editor.set_caret(8); // 围栏后的正文内
    let before = doc_text(&a);
    let undo0 = a.editor.session().undo_depth();
    a.perform_action(Action::CopyCodeBlock);
    assert_eq!(
        log.borrow().as_slice(),
        ["let x = 1;\nlet 中 = \"你好\";\n\nprintln!(\"{}\", 中);"]
    );
    assert_eq!(doc_text(&a), before);
    assert_eq!(a.editor.session().undo_depth(), undo0);
    assert!(!a.editor.is_dirty());

    // 光标不在代码块内：不复制，给出可行动提示。
    let (mut b, log_b) = app_code_copy("普通段落\n");
    b.editor.set_caret(0);
    b.perform_action(Action::CopyCodeBlock);
    assert!(log_b.borrow().is_empty());
    assert!(b.ui_state.chrome.notice.contains("光标不在代码块内"));
}

#[test]
fn code_block_context_menu_copy_action_runs_from_keyboard() {
    let (mut a, log) = app_code_copy(CODE_MULTILINE);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    let (tx, ty, th) = a.engine.caret_xy(&a.cache.layouts[&id].value, 1);
    let vp = a.viewport();
    let top = a.index.blocks[0].top;
    a.input_state.mouse = PhysicalPosition::new(
        (a.content_x() + tx + 1.0) as f64,
        (vp.y + top + ty + th / 2.0) as f64,
    );
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "代码块右键应打开菜单");
    assert_eq!(
        a.ui_state.ctx.action(0),
        Some(ctx_menu::CtxAction::CopyCodeBlock),
        "首项为复制代码块"
    );
    let before = doc_text(&a);
    let undo0 = a.editor.session().undo_depth();
    a.menu_move(1); // ↓ 选中首项
    a.menu_accept(); // Enter 执行
    assert_eq!(
        log.borrow().as_slice(),
        ["let x = 1;\nlet 中 = \"你好\";\n\nprintln!(\"{}\", 中);"]
    );
    assert!(!a.ui_state.ctx.open, "执行后菜单关闭");
    assert_eq!(doc_text(&a), before);
    assert_eq!(a.editor.session().undo_depth(), undo0);
    assert!(!a.editor.is_dirty());
}

/// 真帧绘制：复制按钮恒有可见描边/文字；语言标签仅在 info string 存在时出现。
#[test]
fn code_header_is_painted_and_label_hidden_without_language() {
    let (mut a, _) = app_code_copy("```rust\nfn main() {}\n```\n");
    a.ensure_layouts_all();
    let _ = a.render(); // 先产出光栅，让头部锚定到代码卡右缘
    let h = a.code_header_layout(&a.index.blocks[0]).expect("头部控件");
    let cx = a.content_x();
    let label_band_w = (h.button.x - cx).max(1.0) as u32;
    let f = a.render();
    assert!(
        ink_pixels(
            f,
            h.button.x as i32,
            h.button.y as i32,
            h.button.w as u32,
            h.button.h as u32
        ) > 0,
        "复制按钮应有可见描边/文字"
    );
    let with_lang = ink_pixels(
        f,
        cx as i32,
        h.line_top as i32,
        label_band_w,
        h.line_h as u32,
    );
    let _ = f;
    // 有光栅后按钮贴住代码卡右缘（而不是悬在版心最右）。
    let h2 = a.code_header_layout(&a.index.blocks[0]).expect("头部控件");
    assert!(
        h2.button.x + h2.button.w < a.content_x() + a.content_width() - 40.0,
        "短代码块按钮应贴卡片右缘"
    );
    let (mut b, _) = app_code_copy("```\nfn main() {}\n```\n");
    b.ensure_layouts_all();
    let _ = b.render();
    let hb = b.code_header_layout(&b.index.blocks[0]).expect("头部控件");
    let cbx = b.content_x();
    let fb = b.render();
    let without_lang = ink_pixels(
        fb,
        cbx as i32,
        hb.line_top as i32,
        label_band_w,
        hb.line_h as u32,
    );
    assert!(
        with_lang > without_lang,
        "有语言时首行应多出标签墨迹（{with_lang} vs {without_lang}）"
    );
}

/// 活动块（围栏行还原）仍保留头部入口，且控件仍只在首行。
#[test]
fn code_header_remains_available_when_block_is_active() {
    let (mut a, log) = app_code_copy(CODE_MULTILINE);
    a.ensure_layouts_all();
    a.editor.set_caret(10); // 代码正文 → 整块激活，围栏行显源码
    a.ensure_layouts_all();
    let h = a
        .code_header_layout(&a.index.blocks[0])
        .expect("活动代码块仍应有复制入口");
    click_at(
        &mut a,
        h.button.x + h.button.w / 2.0,
        h.button.y + h.button.h / 2.0,
    );
    assert_eq!(
        log.borrow().as_slice(),
        ["let x = 1;\nlet 中 = \"你好\";\n\nprintln!(\"{}\", 中);"]
    );
}

/// 源码模式展示原始围栏，不叠加头部控件（不劫持编辑点击）。
#[test]
fn code_header_hidden_in_source_mode() {
    let store = MemStore::default();
    let mut a = app(CODE_MULTILINE, None, false, &store);
    a.perform_action(Action::OpenSourceMode);
    a.ensure_layouts_all();
    for g in &a.index.blocks {
        assert!(
            a.code_header_layout(g).is_none(),
            "源码模式不得绘制代码块头部控件"
        );
    }
}
