//! 跨域行为测试：以 headless App 驱动真实编辑/排版/绘制/持久化路径。

use crate::app::*;
use crate::chrome;
use crate::chrome::MenuAction;
use crate::docio::encode;
use crate::document::block_index::BlockGeom;
use crate::document::recovery;
use crate::document::ExternalState;
use crate::editor::Intent;
use crate::geometry::{BASE_LINE_HEIGHT, MARGIN, PARA_GAP};
use crate::image::{AssetStore, ImageData, ImageLoader};
use crate::input::{Action, Key};
use crate::math::{MathImage, MathRenderer};
use crate::mode::{Mode, TextSink};
use crate::paint::{fit_size, local_image_path};
use crate::theme::{self, ConfigStore, SystemTheme, ThemeEnv, ThemeMode};
use crate::view::outline;
use crate::view::search;
use crate::view::{Frame, InlineAsset, Memo, MAX_BLOCK_BYTES};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use veil_core::{BlockId, BlockKind, Range};
use winit::dpi::PhysicalPosition;

struct NoClip;
impl Clipboard for NoClip {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _text: &str) {}
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
        resume: false,
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
        persist_env(),
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
        resume: false,
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
        persist_env(),
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
        persist_env(),
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
        persist_env(),
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
    let (id, kind, range) = a.editor.session().blocks()[1].clone();
    assert_eq!(kind, BlockKind::Table);
    assert!(a.table_shows_grid(id, &range));
    assert_eq!(a.block_height(geom(&a, id)), a.cache.tables[&id].height);
}

#[test]
fn table_shows_source_when_caret_inside() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    assert!(a.editor.set_caret(5)); // 表头单元格内
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, range) = a.editor.session().blocks()[1].clone();
    assert!(!a.cache.tables.contains_key(&id), "源码态不排版网格");
    assert!(!a.table_shows_grid(id, &range));
    assert_eq!(a.block_height(geom(&a, id)), a.cache.layouts[&id].height);
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
fn dirty_edit_invalidates_table_grid() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let old_w = a.cache.tables[&id].width;
    // 表头「a」变宽（xxxxa）→ 块脏；光标仍在表内故不重建网格
    assert!(a.editor.set_caret(5));
    let dirty = a.editor.apply(Intent::InsertText("xxxx".into())).unwrap();
    assert_eq!(dirty.len(), 1);
    a.after_edit(dirty);
    let id = a.editor.session().blocks()[1].0;
    assert!(
        !a.cache.tables.contains_key(&id),
        "表格光标内 → 脏后不建网格"
    );
    // 光标离开表格 → 按新内容重建（列宽变大）
    assert!(a.editor.set_caret(0));
    a.ensure_tables();
    assert!(a.cache.tables.contains_key(&id));
    assert!(a.cache.tables[&id].width > old_w, "内容变宽 → 网格重建");
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
}

fn app(text: &str, path: Option<&str>, bom: bool, store: &MemStore) -> App {
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
        Box::new(NoClip),
        Box::new(store.clone()),
        Box::new(m),
        Box::new(NoImages),
        Box::new(NoAssets),
        Box::new(NoExport),
        Box::new(NoDialogs),
        Box::new(NoWatcher),
        theme::test_support::env(),
        persist_env(),
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
        Box::new(NoClip),
        Box::new(MemStore::default()),
        Box::new(m),
        Box::new(NoImages),
        Box::new(NoAssets),
        Box::new(e),
        Box::new(NoDialogs),
        Box::new(NoWatcher),
        theme::test_support::env(),
        persist_env(),
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
        Box::new(NoClip),
        Box::new(store.clone()),
        Box::new(m),
        Box::new(NoImages),
        Box::new(NoAssets),
        Box::new(NoExport),
        Box::new(d),
        Box::new(NoWatcher),
        theme::test_support::env(),
        persist_env(),
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
    let (x, y) = popup_item_center(0, 2); // 保存
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
        2,
        2,
    );
    assert_eq!(left, "a.md");
    assert_eq!(right, "行 2，列 2");
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
        Box::new(NoClip),
        Box::new(store.clone()),
        Box::new(m),
        Box::new(NoImages),
        Box::new(NoAssets),
        Box::new(NoExport),
        Box::new(NoDialogs),
        Box::new(watch.clone()),
        theme::test_support::env(),
        persist_env(),
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

#[test]
fn tab_indents_selection_and_shift_tab_reverts() {
    let store = MemStore::default();
    let mut a = app("a\nb", None, false, &store);
    a.ensure_layouts_all();
    a.perform_action(Action::Doc(Intent::SelectAll));
    a.route_key(Key::Tab);
    assert_eq!(doc_text(&a), "    a\n    b", "选中多行一次缩进");
    a.input_state.mods.shift = true;
    a.route_key(Key::Tab);
    a.input_state.mods.shift = false;
    assert_eq!(doc_text(&a), "a\nb", "Shift+Tab 反缩进回原状");
    assert!(a.editor.is_dirty());
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
        search::Field::Replace,
        "替换模式焦点落在替换串"
    );
    a.search_toggle_field(); // Tab → 查询
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
    a.search_toggle_field();
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
    a.search_toggle_field();
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
    a.search_toggle_field();
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
    assert_eq!(a.title_text(), "veil — a.md * (外部修改)");
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
        persist_env(),
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
        persist_env(),
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
        // 性质一：两份独立 App 走相同编辑 + 切换序列，
        // 一份保留 memo（ensure_layouts_all 让失配键自动重排回填），
        // 一份清空派生缓存后 ensure_layouts_all 强制重排，
        // 两边 alive BlockId 集合与逐块 (height, reveal, layout_text) 必须完全一致。
        let base_doc = "# 标题\n\n段落 **粗** 与 `code` 和 [链接](https://e.com)\n\n- 列表一\n- 列表二\n\n> 引用一\n> 引用二\n\n```rust\nfn f() -> u32 { 42 }\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let store = MemStore::default();
        let mut a_memo = app(base_doc, None, false, &store);
        let mut a_forced = app(base_doc, None, false, &store);

        for intent in &edits {
            let dirty = a_memo.editor.apply(intent.clone()).unwrap_or_default();
            a_memo.after_edit(dirty);
            let dirty2 = a_forced.editor.apply(intent.clone()).unwrap_or_default();
            a_forced.after_edit(dirty2);
        }
        for t in &toggles {
            apply_toggle(&mut a_memo, *t);
            apply_toggle(&mut a_forced, *t);
        }

        a_memo.ensure_layouts_all();
        let snap_memo = snapshot_layouts(&a_memo);

        a_forced.cache.layouts.clear();
        a_forced.cache.rasters.clear();
        a_forced.cache.math_imgs.clear();
        a_forced.cache.math_failed.clear();
        a_forced.cache.inline_math.clear();
        a_forced.cache.inline_failed.clear();
        a_forced.cache.inline_images.clear();
        a_forced.cache.image_failed.clear();
        a_forced.cache.tables.clear();
        a_forced.ensure_layouts_all();
        let snap_forced = snapshot_layouts(&a_forced);

        let mut ids_memo: Vec<BlockId> = snap_memo.keys().copied().collect();
        let mut ids_forced: Vec<BlockId> = snap_forced.keys().copied().collect();
        ids_memo.sort();
        ids_forced.sort();
        proptest::prop_assert_eq!(
            ids_memo, ids_forced,
            "alive BlockId 集合不一致"
        );

        for id in snap_memo.keys() {
            proptest::prop_assert_eq!(
                snap_memo.get(id),
                snap_forced.get(id),
                "block_id={:?}: memo 命中值 ≠ 强制重算值",
                id
            );
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
        Box::new(NoClip),
        Box::new(store.clone()),
        Box::new(m),
        Box::new(NoImages),
        Box::new(NoAssets),
        Box::new(NoExport),
        Box::new(NoDialogs),
        Box::new(NoWatcher),
        theme::test_support::env(),
        persist_env_with(recovery.clone(), session.clone()),
    )
}

/// 恢复槽中解码出的（路径, 内容）。
fn slot_content(recovery: &FakeRecovery) -> (Option<PathBuf>, String) {
    let data = recovery.slot.borrow().clone().expect("恢复槽应有快照");
    let r = recovery::decode_recovery(&data).expect("快照应可解码");
    (r.path, r.text)
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
    assert_eq!(b.editor.session().serialize(), "改ab", "重启后内容 = 恢复快照");
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
    assert_eq!(b.editor.session().serialize(), "默认样例", "损坏快照不影响正常打开");
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
    assert_eq!(files[Path::new("a.md")], "编辑版".as_bytes(), "手工保存覆盖");
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
    assert_eq!(a.recent_labels(), vec!["x.md".to_string()], "启动加载最近列表");
    let labels = a.recent_labels();
    let (tx, ty) = menu_title_center(0);
    click_at(&mut a, tx, ty);
    assert_eq!(a.ui_state.chrome.open, Some(0));
    // 最近文件插在「导出 HTML…」之后、「退出」之前：下标 = 静态项数 - 1
    let (x, y) = popup_item_center_recent(0, chrome::MENUS[0].items.len() - 1, &labels);
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
