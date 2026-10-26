//! 跨域行为测试：以 headless App 驱动真实编辑/排版/绘制/持久化路径。

use crate::app::*;
use crate::document::block_index::BlockGeom;
use crate::paint::{fit_size, local_image_path};
use crate::chrome::{ExternalState, MenuAction};
use crate::editor::Intent;
use crate::view::InlineAsset;
use crate::image::{AssetStore, ImageData, ImageLoader};
use crate::math::{MathImage, MathRenderer};
use crate::theme::{self, ConfigStore, SystemTheme, ThemeEnv, ThemeMode};
use crate::view::BASE_LINE_HEIGHT;
use crate::chrome;
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
    let (id, _, range) = a.editor.session().blocks()[1];
    assert!(a.cache.math_failed.contains(&id));
    assert!(!a.math_shows_image(id, &range));
    let h = a.cache.layouts.get(&id).unwrap().height;
    assert_eq!(a.block_height(geom(&a, id)), h); // 降级为源码显示
}

#[test]
fn dirty_edit_clears_math_cache() {
    let (mut a, _calls) = app_math("a\n\n$$\nx\n$$\n", false);
    a.ensure_layouts_all();
    a.ensure_math();
    let (id, _, _) = a.editor.session().blocks()[1];
    assert!(a.cache.math_imgs.contains_key(&id));
    a.after_edit(vec![id]); // 块脏 → 图缓存失效，下帧重渲
    assert!(!a.cache.math_imgs.contains_key(&id));
}

// ---- 行内公式（切片 8）：非激活 Math 节点出图、激活显源码、tex 内容寻址缓存 ----
// 「a $x$ b」逐字节：a=0 ' '=1 $=2 x=3 $=4 ' '=5 b=6，Math range=[2,5) content=[3,4)

#[test]
fn inline_math_renders_image_when_caret_outside() {
    let (mut a, calls) = app_math("a $x$ b", false);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0];
    assert_eq!(a.cache.layouts[&id].imgs.len(), 1);
    assert_eq!(a.cache.layouts[&id].imgs[0].width, 8, "FakeMath 行内分支（8x4）");
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
    let (id, _, _) = a.editor.session().blocks()[0];
    assert!(a.cache.layouts[&id].imgs.is_empty());
    assert_eq!(a.cache.layouts[&id].layout_text(), "a $x$ b");
    assert!(calls.borrow().is_empty(), "源码态不渲染");
}

#[test]
fn inline_math_toggles_on_caret_enter_leave() {
    let (mut a, calls) = app_math("a $x$ b", false);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0];
    assert_eq!(a.cache.layouts[&id].imgs.len(), 1);
    // 进入节点：invalidate_reveal 失效 → 重排显源码
    let old = a.editor.caret();
    assert!(a.editor.set_caret(3));
    a.invalidate_reveal(old, 3);
    a.ensure_layouts_all();
    assert!(a.cache.layouts[&id].imgs.is_empty());
    // 离开节点：恢复出图，缓存命中不重渲
    assert!(a.editor.set_caret(0));
    a.invalidate_reveal(3, 0);
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].imgs.len(), 1);
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn inline_math_failure_degrades_and_memoizes() {
    let (mut a, calls) = app_math("a $x$ b", true);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0];
    assert!(a.cache.layouts[&id].imgs.is_empty(), "失败降级：占位不入排版");
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
    let (id, _, _) = a.editor.session().blocks()[0];
    // 默认版心宽 = 800 - 2*16 - 滚动条 12 = 756；1600×900 等比缩放
    assert_eq!(
        (a.cache.layouts[&id].imgs[0].width, a.cache.layouts[&id].imgs[0].height),
        (756, 425)
    );
}

#[test]
fn image_failure_degrades_to_alt_and_memoizes() {
    let (mut a, calls) = app_with_images("a ![alt](img/p.png) b", Some("dir/doc.md"), &[]);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0];
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
    let (id, _, _) = a.editor.session().blocks()[0];
    assert_eq!(a.cache.layouts[&id].layout_text(), "x a");

    let (mut b, calls2) = app_with_images("x ![a](p.png)", None, &[]);
    b.ensure_layouts_all();
    assert!(calls2.borrow().is_empty(), "草稿相对路径无解析基准");
    let (id, _, _) = b.editor.session().blocks()[0];
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
    let (id, _, _) = a.editor.session().blocks()[0];
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
    assert_eq!(a.block_height(geom(&a, id)), a.cache.tables[&id].height);
}

#[test]
fn table_shows_source_when_caret_inside() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    assert!(a.editor.set_caret(5)); // 表头单元格内
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, _, range) = a.editor.session().blocks()[1];
    assert!(!a.cache.tables.contains_key(&id), "源码态不排版网格");
    assert!(!a.table_shows_grid(id, &range));
    assert_eq!(a.block_height(geom(&a, id)), a.cache.layouts[&id].height);
}

#[test]
fn table_grid_hit_maps_to_cell_content_start() {
    let (mut a, _) = app_math(TABLE_DOC, false);
    a.ensure_layouts_all();
    a.ensure_tables();
    let (id, kind, range) = a.editor.session().blocks()[1];
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
    let (id, _, _) = a.editor.session().blocks()[1];
    let old_w = a.cache.tables[&id].width;
    // 表头「a」变宽（xxxxa）→ 块脏；光标仍在表内故不重建网格
    assert!(a.editor.set_caret(5));
    let dirty = a.editor.apply(Intent::InsertText("xxxx".into())).unwrap();
    assert_eq!(dirty.len(), 1);
    a.after_edit(dirty);
    let id = a.editor.session().blocks()[1].0;
    assert!(!a.cache.tables.contains_key(&id), "表格光标内 → 脏后不建网格");
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
    let (id, kind, _) = a.editor.session().blocks()[1];
    assert_eq!(
        kind,
        BlockKind::ListItem {
            ordered: false,
            depth: 0
        }
    );
    assert_eq!(a.cache.layouts[&id].layout_text(), "• a");
    // 进入块：失效 → 重排显源码
    assert!(a.editor.set_caret(4));
    a.invalidate_reveal(0, 4);
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "- a");
    // 离开块：恢复符号
    assert!(a.editor.set_caret(0));
    a.invalidate_reveal(4, 0);
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "• a");
    assert_eq!(a.block_height(geom(&a, id)), a.cache.layouts[&id].height);
}

#[test]
fn blockquote_line_reveal_reflows_only_across_lines() {
    // 「p\n\n> a\n> b\n」：引用块 bytes [3,11)，行 1 [3,7)、行 2 [7,11)
    let (mut a, _) = app_math("p\n\n> a\n> b\n", false);
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[1];
    assert_eq!(a.cache.layouts[&id].layout_text(), "  a\n  b");
    // 块内跨行移动（0 → 9，行 2 内容）：逐行激活变化 → 重排
    assert!(a.editor.set_caret(9));
    a.invalidate_reveal(0, 9);
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "  a\n> b");
    // 同一行内移动（9 → 10）：激活行不变 → 布局缓存保留（不重排）
    assert!(a.editor.set_caret(10));
    a.invalidate_reveal(9, 10);
    assert!(a.cache.layouts.contains_key(&id), "行内移动不得失效引用块布局");
}

#[test]
fn thematic_break_elides_and_keeps_line_height() {
    // 「p\n\n---\n」：分隔线块 bytes [3,7)，初始 caret=0 在块外
    let (mut a, _) = app_math("p\n\n---\n", false);
    a.ensure_layouts_all();
    let (id, kind, _) = a.editor.session().blocks()[1];
    assert_eq!(kind, BlockKind::ThematicBreak);
    assert_eq!(a.cache.layouts[&id].layout_text(), "");
    // 消隐后仍占一行高，块高不塌缩
    assert_eq!(
        a.block_height(geom(&a, id)),
        BASE_LINE_HEIGHT * a.scale
    );
    // 进入块：显源码，块高不变
    assert!(a.editor.set_caret(5));
    a.invalidate_reveal(0, 5);
    a.ensure_layouts_all();
    assert_eq!(a.cache.layouts[&id].layout_text(), "---");
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
    a.cache.rasters.insert(id, (vec![0; 4], 1, 1));
    a.cache.inline_math.insert(
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
        a.cache.layouts.len() < 100,
        "首次编辑只应排版视口内块，实际 {} / {} 块",
        a.cache.layouts.len(),
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
    assert!(!a.cache.layouts.contains_key(&id), "远块初始不应已排版");
    a.ensure_block(id);
    let h = a.cache.layouts.get(&id).expect("按需排版应写入布局").height;
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
    let dirty = a.editor.apply(Intent::InsertText("a".into())).unwrap();
    a.after_edit(dirty);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    println!("perf_cold_first_edit_ms={ms:.3}");
    println!("perf_layouts_after_first_edit={}", a.cache.layouts.len());
}
