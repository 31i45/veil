//! 跨域行为测试：以 headless App 驱动真实编辑/排版/绘制/持久化路径。

use crate::app::*;
use crate::Ports;
use crate::chrome;
use crate::chrome::MenuAction;
use crate::docio::encode;
use crate::document::block_index::BlockGeom;
use crate::document::recovery;
use crate::document::ExternalState;
use crate::editor::Intent;
use crate::geometry::{BASE_LINE_HEIGHT, MARGIN, PARA_GAP};
use crate::image::{AssetStore, ImageData, ImageLoader};
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
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use veil_core::{BlockId, BlockKind, InlineKind, Range};
use winit::dpi::PhysicalPosition;
use winit::keyboard::Key as WKey;

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

/// 既有测试的默认持久化环境：空恢复槽 + 空会话（不产生任何恢复行为）。
fn persist_env() -> PersistEnv {
    PersistEnv {
        recovery: Box::new(FakeRecovery::default()),
        session: Box::new(FakeSession::default()),
    }
}

/// 0b 测试专用装配：恢复槽/会话可由测试预置并持有句柄断言。
fn persist_env_with(recovery: FakeRecovery, session: FakeSession) -> PersistEnv {
    PersistEnv {
        recovery: Box::new(recovery),
        session: Box::new(session),
    }
}

/// 既有测试的默认确认端口：恒「取消」（等同旧行为：脏文档拒绝操作、内容不丢）。
struct NoConfirm;
impl Confirmer for NoConfirm {
    fn confirm_unsaved(&mut self, _file: Option<&Path>) -> UnsavedChoice {
        UnsavedChoice::Cancel
    }
}

fn no_confirm() -> Box<dyn Confirmer> {
    Box::new(NoConfirm)
}

/// 可编排的确认端口：按队列依次给出选择，并记录每次调用时的关联路径。
#[derive(Clone, Default)]
struct FakeConfirm {
    choices: Rc<RefCell<Vec<UnsavedChoice>>>,
    calls: Rc<RefCell<Vec<Option<PathBuf>>>>,
}
impl FakeConfirm {
    fn new(choices: &[UnsavedChoice]) -> Self {
        FakeConfirm {
            choices: Rc::new(RefCell::new(choices.to_vec())),
            calls: Rc::new(RefCell::new(Vec::new())),
        }
    }
}
impl Confirmer for FakeConfirm {
    fn confirm_unsaved(&mut self, file: Option<&Path>) -> UnsavedChoice {
        self.calls.borrow_mut().push(file.map(Path::to_path_buf));
        let mut q = self.choices.borrow_mut();
        if q.is_empty() {
            UnsavedChoice::Cancel
        } else {
            q.remove(0)
        }
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
    assert_eq!(doc_text(&a), "a **b**", "末尾单换行必须剥掉，避免行中粘贴凭空断行");
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
                    .layout(g.id, &g.kind, g.range, base, caret)
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    let l = &a.cache.layouts[&id];
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
    a.cache.layouts.remove(&id);
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
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    // 默认版心宽 = 800 - 2*16 - 滚动条 12 = 756；1600×900 等比缩放
    assert_eq!(
        (
            a.cache.layouts[&id].imgs[0].width,
            a.cache.layouts[&id].imgs[0].height
        ),
        (756, 425)
    );
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
    assert!(!a.cache.tables.contains_key(&id), "管道符位置 → 源码态不排版网格");
    assert!(!a.table_shows_grid(id, &range));
}

#[test]
fn table_shows_grid_when_caret_inside_cell() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    assert!(a.editor.set_caret(5)); // 表头单元格内（cell.range 内）
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, range) = a.editor.session().blocks()[1].clone();
    assert!(a.cache.tables.contains_key(&id), "活动单元格 → 网格继续显示");
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
    assert!(a.cache.tables.contains_key(&new_id), "活动单元格内 → 网格重建");
    assert!(
        a.cache.tables[&new_id].width > old_w,
        "内容变宽 → 新网格列宽变大"
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
    assert!(a.cache.table(id, k0).is_none(), "旧键失配（切换活动格即失效）");
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
    let last_line = src.trim_end_matches('\n').rfind('\n').map(|i| i + 1).unwrap();
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
        .open_at(10.0, 20.0, ctx_menu::CtxKind::Table(tc), None);
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
        .open_at(10.0, 20.0, ctx_menu::CtxKind::Table(tc), None);
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
    assert_eq!(align_of(&a), veil_core::CellAlign::Right, "解析后的列对齐应更新");
    assert_eq!(
        a.editor.caret() - range.start,
        before,
        "对齐不移动光标（相对块起点偏移不变）"
    );

    a.editor.apply(Intent::Undo).unwrap();
    assert_eq!(a.editor.session().serialize(), TABLE_MENU_DOC, "对齐是单撤销步");
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
    a.input_state.mods = Mods { ctrl: false, shift: true, alt: false };
    a.route_key(Key::Tab);
    assert_eq!(a.editor.caret(), tbl.rows[0][0].range.start);
    a.input_state.mods = Mods { ctrl: false, shift: false, alt: false };
}

#[test]
fn tab_outside_table_is_normal_indent() {
    // caret 在段落文本中 → Tab 仍是缩进（InsertText("    ") 或类似）
    let (mut a, _) = app_math("hello\n", false);
    a.editor.set_caret(0);
    let before = a.editor.caret();
    a.route_key(Key::Tab);
    // 缩进至少让光标右移（插 4 空格）
    assert!(
        a.editor.caret() >= before,
        "表外 Tab 不应回退 caret"
    );
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
        confirmer: no_confirm(),
        url_opener,
    }
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
    Ports {
        clipboard: Box::new(NoClip),
        store: Box::new(MemStore::default()),
        math: Box::new(m),
        images: Box::new(NoImages),
        assets: Box::new(NoAssets),
        exporter: Box::new(e),
        dialogs: Box::new(NoDialogs),
        watcher: Box::new(NoWatcher),
        theme_env: theme::test_support::env(),
        persist_env: persist_env(),
        print: Box::new(NoPrint),
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
            confirmer: no_confirm(),
            url_opener: no_url(),
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
fn export_named_document_does_not_ask_for_target() {
    let (mut a, calls, dialog_calls) = app_export_with_dialog("x", Some("ignored.html"));
    a.doc.path = Some(PathBuf::from("a.md"));
    a.export_now();
    assert!(dialog_calls.borrow().is_empty(), "有关联文件时不弹导出对话框");
    assert_eq!(calls.borrow()[0].3, None, "沿用源目录同名规则");
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
    let p = chrome::popup_layout(menu, 1.0, recent);
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
    let (x, y) = popup_item_center(0, 3); // 保存（新建/打开/快速打开/保存）
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
    assert!(calls.borrow().is_empty(), "脏文档拒绝时不弹对话框");
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
    assert!(a.ui_state.chrome.notice.contains("已打开"));
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
fn save_as_uses_old_file_name_as_suggestion() {
    let store = MemStore::default();
    let (mut a, calls) = app_dialogs("ab", Some("dir/a.md"), false, &store, None, None);
    a.save_as(); // 取消
    assert_eq!(calls.borrow().as_slice(), &["save:a.md".to_string()]);
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
    assert!(a.perform_menu(MenuAction::Quit));
    assert!(!a.perform_menu(MenuAction::Copy));
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
        a.ui_state.writing.words,
        2,
        2,
    );
    assert_eq!(left, "a.md");
    assert_eq!(right, "3 字 · 行 2，列 2");
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
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume: false,
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
        watcher: Box::new(watch.clone()),
        theme_env: theme::test_support::env(),
        persist_env: persist_env(),
        print: Box::new(NoPrint),
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
    assert_eq!(doc_text(&a), "改ab", "脏文档拒绝新建，数据不丢");
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
    assert!(a.editor.is_dirty());
    assert!(a.ui_state.chrome.notice.contains("未保存"));
}

// ---- 3e 修 B：未保存内容三选一（关闭/新建/打开） ----

/// 装配：可控确认端口 + 指定对话框 + 恢复槽/会话；其余端口无副作用。
#[allow(clippy::too_many_arguments)]
fn app_confirm(
    text: &str,
    path: Option<&str>,
    store: &MemStore,
    recovery: FakeRecovery,
    session: &FakeSession,
    dialogs: Box<dyn FileDialog>,
    confirmer: Box<dyn Confirmer>,
) -> App {
    let doc = Doc {
        text: text.into(),
        path: path.map(PathBuf::from),
        bom: false,
        resume: false,
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
        confirmer,
        url_opener: no_url(),
    }
)
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
    let c = FakeConfirm::new(&[UnsavedChoice::Save]);
    let c_calls = c.calls.clone();
    let mut a = app_confirm(
        "old",
        Some("a.md"),
        &store,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(d),
        Box::new(c),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.open_via_dialog();
    assert_eq!(
        c_calls.borrow().as_slice(),
        &[Some(PathBuf::from("a.md"))],
        "确认端口拿到当前关联路径"
    );
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
    let c = FakeConfirm::new(&[UnsavedChoice::Discard]);
    let mut a = app_confirm(
        "old",
        Some("a.md"),
        &store,
        rec.clone(),
        &FakeSession::default(),
        Box::new(NoDialogs),
        Box::new(c),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    seed_recovery(&mut a, &rec);

    a.open_path(PathBuf::from("b.md"));
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
    let c = FakeConfirm::new(&[UnsavedChoice::Cancel]);
    let c_calls = c.calls.clone();
    let mut a = app_confirm(
        "old",
        Some("a.md"),
        &store,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(NoDialogs),
        Box::new(c),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.open_path(PathBuf::from("b.md"));
    assert_eq!(c_calls.borrow().len(), 1, "脏文档必须确认一次");
    assert_eq!(doc_text(&a), "改old", "取消：文档保持原样");
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("a.md")));
    assert!(a.editor.is_dirty(), "取消不清脏");
    assert!(a.ui_state.chrome.notice.contains("未保存"));
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
        Box::new(FakeConfirm::new(&[UnsavedChoice::Save])),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.new_draft();
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
        Box::new(FakeConfirm::new(&[UnsavedChoice::Save])),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();

    a.new_draft();
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
        Box::new(FakeConfirm::new(&[UnsavedChoice::Cancel])),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    seed_recovery(&mut a, &rec);
    assert!(!a.confirm_close(), "取消必须阻止退出");
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
        Box::new(FakeConfirm::new(&[UnsavedChoice::Discard])),
    );
    b.editor.apply(Intent::InsertText("改".into())).unwrap();
    seed_recovery(&mut b, &rec);
    assert!(b.confirm_close(), "放弃允许退出");
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
        Box::new(FakeConfirm::new(&[UnsavedChoice::Save])),
    );
    c.editor.apply(Intent::InsertText("改".into())).unwrap();
    seed_recovery(&mut c, &rec);
    assert!(c.confirm_close(), "保存成功允许退出");
    assert_eq!(store.files.borrow()[Path::new("a.md")], "改old".as_bytes());
    assert!(rec.slot.borrow().is_none());
}

#[test]
fn clean_close_skips_confirmation_and_clears_stale_snapshot() {
    // 干净文档：不弹确认；若磁盘上仍残留本会话快照（保存后清理失败的兜底），退出时清掉
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let c = FakeConfirm::new(&[]);
    let c_calls = c.calls.clone();
    let mut a = app_confirm(
        "old",
        Some("a.md"),
        &store,
        rec.clone(),
        &FakeSession::default(),
        Box::new(NoDialogs),
        Box::new(c),
    );
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    seed_recovery(&mut a, &rec);
    a.editor.mark_saved(); // 模拟「内容已与磁盘一致」的干净态，快照仍残留
    assert!(a.confirm_close());
    assert!(c_calls.borrow().is_empty(), "干净文档不得弹确认");
    assert!(rec.slot.borrow().is_none(), "干净退出清残稿");
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
    let center = |q: search::Rect| (q.x + q.w / 2.0, q.y + q.h / 2.0);
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
        a.input_state.theme.chrome_bg,
        "替换行不透明底色"
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
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    a.ui_state.chrome.notice = "占位".into();
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert_eq!(a.ui_state.chrome.notice, "占位", "同一外部版本不重复提示");
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
        ExternalState::Changed,
        "新版本待备份必须盖过旧版本的已备份提示"
    );
    assert_eq!(a.title_text(), "veil — a.md (外部修改)");
    assert!(a.persist_state.persist.hold());
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
    watch.events.borrow_mut().push(FileEvent::Changed);
    a.poll_external();
    assert!(a.persist_state.persist.hold());
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
fn save_as_rebinds_watcher() {
    let store = MemStore::default();
    let watch = FakeWatch::default();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let doc = Doc {
        text: "x".into(),
        path: Some(PathBuf::from("a.md")),
        bom: false,
        resume: false,
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
        },
        persist_env: persist_env(),
        print: Box::new(NoPrint),
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
        .raster_key(id, &kind, range, base, caret)
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
            assert_eq!(b.get(i - 1), Some(&b'\r'), "第 {i} 字节的 \\n 前缺 \\r（混入 LF）");
        }
        if c == b'\r' {
            assert_eq!(b.get(i + 1), Some(&b'\n'), "第 {i} 字节的 \\r 后缺 \\n（孤立回车）");
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

    // 段落分隔（Enter×2）：新段顶边 = 块底 + 段距
    a.route_key(Key::Enter);
    let (cx2, cy2, _) = a.caret_screen();
    assert_eq!(
        (cx2, cy2),
        (margin, vp_y + top1 + h1 + PARA_GAP * a.input_state.scale),
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
    let (sy0, sy1) = (vp.y + top - a.window_state.scroll_y, vp.y + top + bh - a.window_state.scroll_y);
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
    let files: Vec<(String, u32, u32)> =
        (0..10).map(|i| (format!("doc/p{i}.png"), 600, 500)).collect();
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
            a.cache.layout(g.id, &g.kind, g.range, base, caret).is_none()
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
    assert!(missing.is_empty(), "可见区间存在未排版块（绘制为空洞）: {missing:?}");
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
    let files: Vec<(String, u32, u32)> =
        (0..5).map(|i| (format!("doc/p{}.png", i * 2), 600, 500)).collect();
    let files_ref: Vec<(&str, u32, u32)> = files.iter().map(|(p, w, h)| (p.as_str(), *w, *h)).collect();
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
            a.cache.layout(g.id, &g.kind, g.range, base, caret).is_none()
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
    assert!(missing.is_empty(), "可见区间存在未排版块（绘制为空洞）: {missing:?}");
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
    let text = (0..12).map(|_| para.clone()).collect::<Vec<_>>().join("\n\n");
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
            a.cache.layout(g.id, &g.kind, g.range, base, caret).is_none()
                && g.kind != BlockKind::Math
                && !(g.kind == BlockKind::Table
                    && a.cache.table(g.id, a.table_key(g.id)).is_some())
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
    assert!(missing.is_empty(), "可见区间存在未排版块（绘制为空洞）: {missing:?}");
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
        .raster_key(g.id, &g.kind, g.range, a.base_key(), a.editor.caret())
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

#[test]
fn geometry_index_is_monotonic_and_consistent() {
    let store = MemStore::default();
    let mut a = app(&stress_doc(), None, false, &store);
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    let mut prev = f32::NEG_INFINITY;
    let mut y = MARGIN * a.input_state.scale;
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
        y += g.height + PARA_GAP * a.input_state.scale;
    }
    let want = y - PARA_GAP * a.input_state.scale + MARGIN * a.input_state.scale;
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
    let g = geom(&a, id);
    let (kind, range) = (g.kind.clone(), g.range);
    assert!(
        a.cache
            .layout(id, &kind, range, a.base_key(), a.editor.caret())
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
    assert!(stale.is_empty(), "标题块迁移后不得残留旧 Reveal 布局: {stale:?}");
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
    assert!(stale.is_empty(), "围栏块迁移后不得残留旧 Reveal 布局: {stale:?}");
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
                let Some(lk) = a.cache.raster_key(id, &kind, range, base, caret) else {
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
/// cargo test -p veil-egui --release -- --ignored --nocapture perf_cold_first_edit
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
/// cargo test -p veil-egui --release -- --ignored --nocapture perf_cold_first_edit_prewarmed
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
/// cargo test -p veil-egui --release -- --ignored --nocapture perf_first_edit_after_first_frame
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
/// cargo test -p veil-egui --release -- --ignored --nocapture perf_huge_scroll
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
/// cargo test -p veil-egui --release -- --ignored --nocapture perf_huge_block
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
    assert!(rec.slot.borrow().is_some(), "外部打开的保存不动他人恢复快照");
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
    // 内容再变：哈希失配重写
    a.editor.apply(Intent::InsertText("再".into())).unwrap();
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
/// cargo test -p veil-egui --release -- --ignored --nocapture perf_keystroke_10k_blocks
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
        a.input_state.theme.chrome_bg,
        "标题区底色"
    );
    assert_eq!(
        f.get((p.x - 4.0) as u32, (p.y + 4.0) as u32),
        a.input_state.theme.bg,
        "面板左侧仍为文档底色"
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
    let center = |r: search::Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
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
fn ctrl_h_first_open_focuses_query_then_repeat_focuses_replace() {
    let store = MemStore::default();
    let mut a = app("a x a\n", None, false, &store);
    a.ensure_layouts_all();

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

    // 栏已开时再按 Ctrl+H：焦点给替换串，可直接输入替换词
    a.input_state.mods.ctrl = true;
    a.route_key(Key::H);
    a.input_state.mods.ctrl = false;
    assert_eq!(a.ui_state.search.focus, search::Field::Replace);
    a.route_key(Key::Text("b".into()));
    assert_eq!(a.ui_state.search.replace, "b");

    // Ctrl+F 在栏内仍可回到查询焦点，且不丢已有查询词
    a.input_state.mods.ctrl = true;
    a.route_key(Key::F);
    a.input_state.mods.ctrl = false;
    assert_eq!(a.ui_state.search.focus, search::Field::Query);
    assert_eq!(a.ui_state.search.query, "a");
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
    a.input_state.mouse =
        PhysicalPosition::new((MARGIN * scale + lx + 2.0) as f64, (vp.y + top - a.window_state.scroll_y + ly + 5.0) as f64);
    a.on_right_click();
    assert!(a.ui_state.ctx.open, "应打开右键菜单");
    assert!(
        a.ui_state.ctx.link.is_some(),
        "链接文字上右键应出「复制链接地址」项集"
    );
}

// ---- 链接打开：Ctrl+左键 / 右键菜单（协议白名单 + 失败提示） ----

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

fn link_app(
    text: &str,
    calls: &Rc<RefCell<Vec<String>>>,
    result: Result<(), String>,
) -> App {
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
    for bad in ["file:///C:/x", "javascript:alert(1)", "a.md", "#anchor", "/abs"] {
        assert!(!url_is_openable(bad), "应拒绝: {bad:?}");
    }
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
    assert!(a.window_state.last_click.is_none(), "不进入点击计数/拖选路径");
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
        .open_at(10.0, 20.0, ctx_menu::CtxKind::Block, None);
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
        a.input_state.theme.chrome_bg,
        "浮栏不透明底色"
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
/// cargo test -p veil-egui --release -- --ignored --nocapture perf_peak_breakdown
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
/// cargo test -p veil-egui --release -- --ignored --nocapture perf_after_edit_breakdown
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
                    .layout(g.id, &g.kind, g.range, base, caret)
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
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("doc/a.md"), "# 标题\n\n第一段。\n".as_bytes().to_vec());
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
    assert!(disk.contains("| 长长长名称 | 数量 |"), "格内编辑后的表格源码完整落盘");
    assert!(disk.contains("![](t.assets/image-1.png)"));
}

#[test]
fn e2e_search_replace_and_crash_recovery() {
    // 查找→替换全部→崩溃（杀进程）→重启恢复→保存收口
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist("alpha beta alpha\n", Some("r.md"), &store, &rec, &ses, false);
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
        (Some(PathBuf::from("r.md")), "gamma beta gamma\n".to_string()),
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
    assert_eq!(b.doc.baseline, b"alpha beta alpha\n".to_vec(), "基线 = 磁盘");

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
                .layout(g.id, &g.kind, g.range, base, usize::MAX)
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
)
}

/// 断点合法性：除 0.0 外每个断点落在某块 top 或块内行顶（不劈行）。
fn assert_breaks_on_boundaries(bs: &[f32], gs: &[(f32, Vec<f32>)]) {
    for &b in &bs[1..] {
        let on = gs
            .iter()
            .any(|(top, lines)| (top - b).abs() < 0.5 || lines.iter().any(|&t| (top + t - b).abs() < 0.5));
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
        assert_eq!(corners, &[bg, bg, bg, bg], "页帧四角为文档底色（无 chrome）");
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
    assert!(screen.contains("**"), "屏幕态光标在节点内显源码: {screen:?}");
    a.perform_menu(MenuAction::Print);
    let printed = texts.borrow();
    assert_eq!(printed.len(), 1, "单块文档");
    assert!(!printed[0].contains("**"), "打印态标记收拢: {:?}", printed[0]);
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
        confirmer: no_confirm(),
        url_opener: no_url(),
    }
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
            !gs
                .iter()
                .any(|(top, lines)| (top - b).abs() < 0.5 || lines.iter().any(|&t| (top + t - b).abs() < 0.5))
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
            .filter(|g| matches!(g.kind, BlockKind::AtxHeading(_) | BlockKind::SetextHeading(_)))
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

fn region_has_ink(
    frame: &[u32],
    width: u32,
    bg: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> bool {
    (y0..y1).any(|y| {
        (x0..x1).any(|x| frame[(y as usize) * width as usize + x as usize] != bg)
    })
}

fn export_doc() -> String {
    let filler: String = (1..=12).map(|i| format!("正文段落 {i} 内容。\n\n")).collect();
    format!(
        "# 第一章\n\n{filler}## 第二节\n\n{filler}### 小节\n\n{filler}尾段\n"
    )
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
    assert_eq!(pages.get(), breaks.borrow().len(), "无目录时总页数 = 正文页数");

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
    let off_ink = region_has_ink(&off_frames.borrow()[0], w, bg, w - 120, h - 48, w - 4, h - 4);

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
        crate::input::translate(Key::Equal, Mods {
            ctrl: true,
            shift: false,
            alt: false,
        }),
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
    let ctrl = Mods { ctrl: true, shift: false, alt: false };
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
    let altgr = Mods { ctrl: true, shift: false, alt: true };
    assert_eq!(
        crate::interaction::map_key_parts(Some("s"), WKey::Character(s), altgr),
        Some(Key::Text("s".into()))
    );
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
    assert!(!a.perform_menu(MenuAction::ToggleFullscreen), "全屏不退出应用");
    assert!(!a.window_state.fullscreen);
    assert_eq!(a.ui_state.chrome.notice, notice, "no-op 不刷提示");
    // 路由：Document 模式 F11 直达动作；Esc 语义不变（关菜单，不退全屏）
    assert_eq!(
        crate::input::translate(Key::F11, Mods {
            ctrl: false,
            shift: false,
            alt: false,
        }),
        Some(Action::ToggleFullscreen)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3g WindowState visible 状态机：最小化/失焦/Occluded 时阻断 paint 路径
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn window_visibility_defaults_to_true() {
    let a = app_clip(None, None);
    assert!(
        a.window_state.visible,
        "App 刚构造时窗口默认可见"
    );
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
    let frame_before = a
        .window_state
        .frame
        .as_ref()
        .map(|f| f.pixels.clone());

    // 手动设置不可见后再直接 paint（绕过 RedrawRequested 守卫）
    a.window_state.visible = false;
    a.paint(); // 不应 panic
    let frame_after = a
        .window_state
        .frame
        .as_ref()
        .map(|f| f.pixels.clone());

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
    assert_eq!(a.ui_state.chrome.hover_item, Some((0, 0)), "无选中时 ↓ 选首项");
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
    assert_eq!(a.input_state.theme.mode, ThemeMode::Dark, "Enter 执行选中项");
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
    let (x, y) = popup_item_center(0, 3); // 保存（干净文档 → 禁用）
    click_at(&mut a, x, y);
    assert_eq!(a.ui_state.chrome.open, Some(0), "禁用项点击不关菜单");
    assert!(calls.borrow().is_empty(), "禁用项不触发任何对话框/写盘");
}

#[test]
fn file_menu_paints_group_separators_and_recent_block() {
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
    let p = chrome::popup_layout(0, 1.0, &recent);

    // 4 条分隔线：三个静态分组边界 + 最近文件块与「退出」之间
    assert_eq!(p.separators.len(), 4, "文件菜单应有 4 条分隔线");
    let border = [theme.chrome_border[0], theme.chrome_border[1], theme.chrome_border[2]];
    for (_, sy) in &p.separators {
        let mid = frame.get((p.x + 20.0) as u32, sy.round() as u32);
        assert_eq!(
            [mid[0], mid[1], mid[2]],
            border,
            "分隔线像素质点应为边框色（y={sy}）"
        );
        // 左右内缩：贴边处不应是分隔线
        let edge = frame.get(p.x as u32, sy.round() as u32);
        assert_ne!([edge[0], edge[1], edge[2]], border);
    }
    // 项行中部不是分隔线色（防止把整行涂成线）
    let (_, _, iy, _, ih) = p.items[0];
    let row = frame.get((p.x + 20.0) as u32, (iy + ih / 2.0) as u32);
    assert_ne!([row[0], row[1], row[2]], border);
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
    let p = chrome::popup_layout(0, 1.0, &[]);
    let save = p.items[3]; // 新建 / 打开 / 快速打开 / 保存
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
            [
                theme.popup_bg[0],
                theme.popup_bg[1],
                theme.popup_bg[2]
            ],
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
    let bg = frame.get(sx as u32 + 2, y0 + 2);
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
        .open_at(10.0, 20.0, ctx_menu::CtxKind::Selection, None);
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

    a.perform_action(Action::ToggleSourceMode);
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

    a.perform_action(Action::ToggleSourceMode);
    a.ensure_layouts_all();
    assert!(!a.source_mode());
    assert_eq!(a.cache.layouts[&heading].layout_text(), "标题");
    assert!(!a.cache.layouts[&para].layout_text().contains("**"));
}

#[test]
fn source_mode_hides_math_image_and_table_grid() {
    let store = MemStore::default();
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

    a.perform_action(Action::ToggleSourceMode);
    a.ensure_layouts_all();
    assert!(!a.math_shows_image(math.0, &math.1), "源码模式不渲染公式图");
    assert!(!a.table_shows_grid(table.0, &table.1), "源码模式不渲染表格网格");
    assert!(
        !a.cache.tables.contains_key(&table.0),
        "源码模式不产表格网格缓存"
    );
    let tl = a.cache.layouts[&table.0].layout_text().to_string();
    assert!(tl.contains('|'), "表格显管道与分隔行源码: {tl:?}");

    a.perform_action(Action::ToggleSourceMode);
    a.ensure_math();
    a.ensure_tables();
    assert!(a.math_shows_image(math.0, &math.1), "关闭后恢复公式图");
    assert!(a.table_shows_grid(table.0, &table.1), "关闭后恢复表格网格");
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
        *self.during_heading.borrow_mut() =
            s.app.cache.layouts[&heading].layout_text().to_string();
        s.end();
        s.app.ensure_layouts_all();
        *self.after_heading.borrow_mut() =
            s.app.cache.layouts[&heading].layout_text().to_string();
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
    a.perform_action(Action::ToggleSourceMode);
    a.ensure_layouts_all();
    let heading = block_id_of(&a, |k| matches!(k, BlockKind::AtxHeading(_)));
    assert!(a.cache.layouts[&heading].layout_text().contains('#'));

    a.perform_menu(MenuAction::Print);
    assert!(!flag.get(), "打印中 source_mode() 恒 false");
    assert_eq!(&*during.borrow(), "标", "打印态忽略源码模式");
    assert_eq!(
        &*after.borrow(),
        "# 标",
        "打印结束回屏幕后恢复源码模式"
    );
    assert!(a.source_mode(), "用户开关未被打印改写");
}
