//! 媒体富内容测试：粘贴 / Mermaid / 行内与块级公式 / 文档内图片 / 远程图片 / 拖放。

use super::*;

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

/// 自动预览只向本地渲染器投递 request：label 是 `mermaid-auto` 稳定键，
/// 不含任何 URL；空块被跳过；不出现第三方提示。
#[test]
fn mermaid_auto_preview_requests_only_local_renderer() {
    let mermaid = RecordingMermaid::new();
    let doc =
        "前文\n\n```mermaid\n```\n\n```mermaid\n \n\t\n```\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_with_mermaid(doc, mermaid.clone());
    assert!(a.ui_state.chrome.notice.is_empty(), "启动时不应有提示");

    a.trigger_mermaid_auto();

    assert_eq!(mermaid.request_count(), 1, "只有非空块应投递 request");
    let label = mermaid.last_request_url().expect("request 未投递");
    assert!(label.starts_with("mermaid-auto:"), "{label}");
    assert!(!label.contains("http"), "label 不得包含网络地址：{label}");
    assert_eq!(
        mermaid.last_request_source().as_deref(),
        Some("graph TD\nA-->B")
    );
    assert!(
        a.ui_state.chrome.notice.is_empty(),
        "本地渲染不得出现第三方提示"
    );
}

/// 回归：投递的源码必须是围栏**内容**，不能带 ```mermaid 围栏行。
#[test]
fn mermaid_request_source_strips_fence() {
    let mermaid = RecordingMermaid::new();
    let doc = "前文\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_with_mermaid(doc, mermaid.clone());
    a.trigger_mermaid_auto();
    let source = mermaid.last_request_source().expect("request 未投递");
    assert!(!source.contains("```"), "源码仍含围栏: {source:?}");
    assert!(source.contains("graph TD"), "围栏内容丢失: {source:?}");
}

/// 同一 (source_hash, dark) 连续失败 ≥ 3 次后，trigger_mermaid_auto 跳过第 4 次 request。
/// 失败次数由 poll_remote_images 的 Err 分支累加，trigger_mermaid_auto 开头检查 ≥ 3 就 continue。
#[test]
fn mermaid_failed_count_reaches_three_skips_fourth_request() {
    let mermaid = RecordingMermaid::new();
    let doc = "前文\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_with_mermaid(doc, mermaid.clone());
    assert!(
        !a.config.allow_remote_images,
        "默认阻止普通远程图片，不影响本地 Mermaid"
    );

    // 辅助：一轮完整失败 → trigger 投递 → poll 消费 Err → failed 递增
    let run_fail_round = |a: &mut App, m: &RecordingMermaid| {
        a.trigger_mermaid_auto();
        // 把 poll_queue 填上与最近 request URL 匹配的 Err
        if let Some(url) = m.last_request_url() {
            m.push_err(url, "模拟渲染失败");
        }
        a.poll_remote_images();
    };

    run_fail_round(&mut a, &mermaid);
    assert_eq!(mermaid.request_count(), 1, "第 1 轮应投递并失败");

    run_fail_round(&mut a, &mermaid);
    assert_eq!(mermaid.request_count(), 2, "第 2 轮应再次投递并失败");

    run_fail_round(&mut a, &mermaid);
    assert_eq!(mermaid.request_count(), 3, "第 3 轮应再次投递并失败");

    // 第 4 次：failed 计数已达 3 → trigger_mermaid_auto 检查 ≥ 3 直接 continue
    a.trigger_mermaid_auto();
    assert_eq!(mermaid.request_count(), 3, "连续 3 次失败后应跳过新请求");
}

/// 源码 > 128 KiB → trigger_mermaid_auto 跳过投递并在状态栏设置 notice。
/// 超大源码不进 mermaid_failed（用户编辑源码变小后应能自动重试）。
#[test]
fn mermaid_source_too_large_skips_and_sets_notice() {
    let mermaid = RecordingMermaid::new();
    let huge_body = "x".repeat(128 * 1024 + 100);
    let doc = format!("前文\n\n```mermaid\n{}\n```\n", huge_body);
    let mut a = app_with_mermaid(&doc, mermaid.clone());

    a.trigger_mermaid_auto();
    assert_eq!(mermaid.request_count(), 0, "> 128 KiB 源码不应投递 request");
    assert!(
        a.ui_state.chrome.notice.contains("128 KiB"),
        "应设置源码超限 notice（实际: {:?}）",
        a.ui_state.chrome.notice
    );
}

/// 导出前置补齐：缓存缺失的 Mermaid 块应走 render_blocking 同步渲染并写入缓存；
/// 已补齐的块不重复渲染。
#[test]
fn export_ensure_renders_missing_mermaid_blocks() {
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(MermaidImage {
        data: vec![0; 4],
        width: 1,
        height: 1,
        svg_text: None,
        px_to_pt: 0.375,
    });
    let doc = "前文\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_with_mermaid(doc, mermaid.clone());
    assert!(a.cache.mermaid_imgs.is_empty(), "初始缓存应为空");

    a.ensure_mermaid_for_export();
    assert_eq!(mermaid.blocking_count(), 1, "缺失块应触发 1 次同步渲染");
    assert_eq!(a.cache.mermaid_imgs.len(), 1, "渲染结果应写入缓存");
    let id = a
        .editor
        .session()
        .blocks()
        .into_iter()
        .find(|(_, kind, _)| *kind == BlockKind::Mermaid)
        .map(|(id, _, _)| id)
        .expect("文档应含 Mermaid 块");
    assert!(a.cache.mermaid_imgs.contains_key(&id));

    // 幂等：缓存命中后不重复阻塞渲染
    a.ensure_mermaid_for_export();
    assert_eq!(mermaid.blocking_count(), 1, "缓存命中不应重复渲染");
}

/// 「允许远程图片」开关只作用于普通远程 `<img>`；Mermaid 导出补齐恒走本地渲染。
#[test]
fn export_ensure_renders_mermaid_locally_when_remote_images_off() {
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(MermaidImage {
        data: vec![0; 4],
        width: 1,
        height: 1,
        svg_text: None,
        px_to_pt: 0.375,
    });
    let doc = "前文\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_with_mermaid(doc, mermaid.clone());
    a.config.allow_remote_images = false;

    a.ensure_mermaid_for_export();
    assert_eq!(
        mermaid.blocking_count(),
        1,
        "本地导出补齐不受远程图片开关影响"
    );
    assert_eq!(a.cache.mermaid_imgs.len(), 1, "本地渲染结果应写入缓存");
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
    assert_eq!(
        a.math_failure_notice(id, &range),
        Some(I18nKey::MathRenderFailedHint),
        "渲染失败必须给出界面提示，不再只写 stderr"
    );
    assert_eq!(t(I18nKey::MathRenderFailedHint), "公式渲染失败，已显示源码");
    // 光标进入块内改源码时不再叠加提示（源码态是主动编辑，不是静默降级）。
    a.editor.set_caret(6);
    assert_eq!(a.math_failure_notice(id, &range), None);
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
    let (mut a, calls, _files) = app_with_images(
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
fn image_click_opens_preview_without_touching_document() {
    let (mut a, _calls, _files) = app_with_images(
        "前文\n\n![示例](img/p.png)",
        Some("dir/doc.md"),
        &[("dir/img/p.png", 120, 60)],
    );
    a.ensure_layouts_all();
    a.recompute_geometry();
    let before = doc_text(&a);
    let caret = a.editor.caret();
    let (id, _, _) = a.editor.session().blocks()[1].clone();
    let (ix, iy, iw, ih) = {
        let p = &a.cache.layouts[&id].imgs[0];
        (p.x, p.y, p.width, p.height)
    };
    let top = a
        .index
        .blocks
        .iter()
        .find(|g| g.id == id)
        .map(|g| g.top)
        .unwrap();
    let vp = a.viewport();
    let (x, y) = (
        a.content_x() + ix + iw as f32 * 0.5,
        vp.y + top - a.window_state.scroll_y + iy + ih as f32 * 0.5,
    );
    a.editor.set_preedit("zh".into(), None);
    click_at(&mut a, x, y);
    assert!(a.ui_state.image_preview.is_some(), "点击图片应打开放大预览");
    assert!(a.editor.preedit().is_none(), "打开预览必须立即清除组合态");
    assert_eq!(a.editor.caret(), caret, "预览不移动光标");
    assert_eq!(doc_text(&a), before, "预览不改文档字节");
    // 点击图片本身保持打开；点击遮罩关闭。
    click_at(&mut a, x, y);
    assert!(a.ui_state.image_preview.is_some(), "点图不关闭");
    click_at(&mut a, 2.0, 2.0);
    assert!(a.ui_state.image_preview.is_none(), "遮罩点击应关闭");
    assert_eq!(a.editor.caret(), caret);
    assert_eq!(doc_text(&a), before);
}

#[test]
fn image_preview_zoom_pan_and_escape_keep_document_intact() {
    let (mut a, _calls, _files) = app_with_images(
        "![示例](img/p.png)",
        Some("dir/doc.md"),
        &[("dir/img/p.png", 120, 60)],
    );
    a.ensure_layouts_all();
    a.recompute_geometry();
    let before = doc_text(&a);
    a.ui_state.image_preview = Some(ImagePreview {
        key: "dir/img/p.png".into(),
        title: "示例".into(),
        zoom: 1.0,
        pan: (0.0, 0.0),
    });
    a.route_key(Key::Equal);
    assert!(
        a.ui_state.image_preview.as_ref().unwrap().zoom > 1.0,
        "= 应放大"
    );
    let panned = a.ui_state.image_preview.as_ref().unwrap().pan.1;
    a.image_preview_wheel(winit::event::MouseScrollDelta::LineDelta(0.0, -2.0));
    assert!(
        a.ui_state.image_preview.as_ref().unwrap().pan.1 != panned,
        "普通滚轮应平移"
    );
    a.input_state.mods.ctrl = true;
    let zoom = a.ui_state.image_preview.as_ref().unwrap().zoom;
    a.image_preview_wheel(winit::event::MouseScrollDelta::LineDelta(0.0, 1.0));
    assert!(
        a.ui_state.image_preview.as_ref().unwrap().zoom > zoom,
        "Ctrl+滚轮应缩放"
    );
    a.input_state.mods.ctrl = false;
    a.route_key(Key::Escape);
    assert!(a.ui_state.image_preview.is_none(), "Esc 应关闭预览");
    assert_eq!(doc_text(&a), before, "缩放/平移/关闭都不改文档");
}

#[test]
fn reload_image_invalidates_layout_and_refreshes_size() {
    let (mut a, calls, files) = app_with_images(
        "x ![alt](img/p.png)",
        Some("dir/doc.md"),
        &[("dir/img/p.png", 100, 50)],
    );
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    let before = (
        a.cache.layouts[&id].imgs[0].width,
        a.cache.layouts[&id].imgs[0].height,
        a.cache.layouts[&id].height,
    );
    assert_eq!((before.0, before.1), (100, 50));
    let base_before = a.base_key();
    // 磁盘上的图片被替换为新尺寸：要点「重新加载图片」。
    files
        .borrow_mut()
        .insert(PathBuf::from("dir/img/p.png"), (120, 60));
    a.ui_state.ctx.open_at(
        10.0,
        20.0,
        ctx_menu::CtxKind::Image {
            path: "img/p.png".into(),
            missing: false,
        },
        None,
        None,
    );
    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::ReloadImage)
        .expect("图片菜单应含重新加载");
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);

    assert_ne!(a.base_key(), base_before, "重载后布局键必须失效");
    assert_eq!(calls.borrow().len(), 2, "重载必须重新读盘");
    let after = &a.cache.layouts[&id];
    assert_eq!(
        (after.imgs[0].width, after.imgs[0].height),
        (120, 60),
        "占位按新自然尺寸刷新"
    );
    assert_ne!(after.height, before.2, "块高随新图尺寸刷新");
}

#[test]
fn reload_blocked_remote_image_points_to_allow_remote_images() {
    let mut a = app_remote(
        &format!("x ![a]({REMOTE_URL})"),
        Box::new(MemConfig::default()),
        RemoteImages::default(),
    );
    assert!(!a.config.allow_remote_images, "缺省阻止远程图片");
    a.ensure_layouts_all();
    a.ui_state.ctx.open_at(
        10.0,
        20.0,
        ctx_menu::CtxKind::Image {
            path: REMOTE_URL.into(),
            missing: false,
        },
        None,
        None,
    );
    let i = a
        .ui_state
        .ctx
        .items()
        .iter()
        .position(|&(_, act)| act == ctx_menu::CtxAction::ReloadImage)
        .expect("图片菜单应含重新加载");
    a.ui_state.ctx.hover = Some(i);
    a.route_key(Key::Enter);
    assert!(
        a.ui_state.chrome.notice.contains("允许远程图片"),
        "被阻止时重载应提示可操作入口: {}",
        a.ui_state.chrome.notice
    );
    assert!(!a.config.allow_remote_images, "提示不得擅自改变配置");
}

#[test]
fn image_wider_than_content_scales_proportionally() {
    // 前缀 "x "：默认光标 0 不在图片节点区间内（否则进入源码态不加载）
    // 宽 1600 且高 45（低于 2 行限高），确保本用例仍由「宽度上限」触发缩放。
    let (mut a, _, _files) = app_with_images(
        "x ![a](p.png)",
        Some("dir/doc.md"),
        &[("dir/p.png", 1600, 45)],
    );
    a.ensure_layouts_all();
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    // 版心宽受可读列宽上限约束（默认窗口 800 宽时封顶 640）；1600×45 等比缩放
    let want_w = a.content_width() as u32;
    let want_h = (45.0 * a.content_width() / 1600.0) as u32;
    assert_eq!(
        (
            a.cache.layouts[&id].imgs[0].width,
            a.cache.layouts[&id].imgs[0].height
        ),
        (want_w, want_h)
    );
}

/// B4 回归：整块仅图片（含尾随换行）→ 按版心宽/视口高放大，不再受 4 行限高。
#[test]
fn standalone_markdown_image_uses_viewport_height() {
    let (mut a, _, _files) = app_with_images(
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
    let (mut a, _, _files) = app_with_images(
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
    let (mut a, _, _files) = app_with_images(
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
    let (mut a, _, _files) = app_with_images(
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
    let (mut a, _, _files) = app_with_images(
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

/// B4 回归：独立图片块只占图片实际显示高度，后接标题的间距与普通段落一致。
/// 回归：占位 span 重复拼接 → NBSP 翻倍折行 → 块高 = 2×图高（半屏空白）。
#[test]
fn standalone_image_block_height_is_displayed_height() {
    let (mut a, _, _files) = app_with_images(
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
    let (mut a, _, _files) = app_with_images(
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
    a.config.allow_remote_images = true;
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
    assert!(
        after_h > before_h,
        "加载后块高应增大：{before_h} -> {after_h}"
    );
    assert_eq!(after_h, shown, "加载后块高 = 图片实际显示高度");
    assert!(
        (after_top - before_top - (after_h - before_h)).abs() < 0.5,
        "后续块顶应同步下移：{before_top} -> {after_top}（块高差 {after_h}-{before_h}）"
    );
}

/// 窗口缩放后独立图片块高度按新视口重算，块高仍等于显示高度。
#[test]
fn standalone_image_block_height_recomputed_after_resize() {
    let (mut a, _, _files) = app_with_images(
        "前文\n\n![](p.png)\n\n## 1.1 背景\n",
        Some("dir/doc.md"),
        // 宽高比足够大：宽 640 时显示高 320，超过矮窗口的视口高上限，才能验证重算。
        &[("dir/p.png", 1600, 800)],
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
    let (mut a, calls, _files) = app_with_images("a ![alt](img/p.png) b", Some("dir/doc.md"), &[]);
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
    let (mut a, _, _files) = app_with_images(
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
    let (mut a, _, _files) = app_with_images(
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
    let (mut a, _, _files) = app_with_images(
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
    let (mut a, _, _files) =
        app_with_images("前文\n\n![找不到](missing.png)\n", Some("dir/doc.md"), &[]);
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
    let (mut with, _, _files) = app_with_images(
        "前文\n\n![题注](p.png)\n",
        Some("dir/doc.md"),
        &[("dir/p.png", 200, 100)],
    );
    with.ensure_layouts_all();
    with.recompute_geometry();
    let (mut without, _, _files) = app_with_images(
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
    let (mut a, _, _files) = app_with_images(
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
    let (mut a, calls, _files) =
        app_with_images("x ![a](https://example.com/x.png)", Some("dir/doc.md"), &[]);
    a.ensure_layouts_all();
    assert!(calls.borrow().is_empty());
    let (id, _, _) = a.editor.session().blocks()[0].clone();
    assert_eq!(a.cache.layouts[&id].layout_text(), "x a");

    let (mut b, calls2, _files) = app_with_images("x ![a](p.png)", None, &[]);
    b.ensure_layouts_all();
    assert!(calls2.borrow().is_empty(), "草稿相对路径无解析基准");
    let (id, _, _) = b.editor.session().blocks()[0].clone();
    assert_eq!(b.cache.layouts[&id].layout_text(), "x a");
}

#[test]
fn image_shows_source_when_caret_inside() {
    let (mut a, calls, _files) = app_with_images(
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
            ime_anchor: Box::new(NoopImeAnchor),
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
            mermaid: Box::new(NoopMermaid),
            images: Box::new(FakeImages {
                files: Rc::new(RefCell::new(files)),
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

// ---- 远程图片（v1.4）：默认阻止 + 用户显式允许开关 ----

#[test]
fn remote_image_requested_once_when_allowed() {
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let mut a = app_remote(
        &format!("前 ![a]({REMOTE_URL}) 后\n"),
        Box::new(MemConfig::default()),
        images,
    );
    a.config.allow_remote_images = true;
    a.ensure_layouts_all();
    assert_eq!(requests.borrow().as_slice(), &[REMOTE_URL.to_string()]);
    assert!(a.cache.remote_pending.contains(REMOTE_URL));
    // 再次布局/重绘：布局已缓存且 pending 去重，不得重复请求
    a.ensure_layouts_all();
    a.request_redraw();
    assert_eq!(requests.borrow().len(), 1, "同一 URL 只请求一次");
}

#[test]
fn remote_image_not_requested_by_default() {
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let mut a = app_remote(
        &format!("前 ![a]({REMOTE_URL}) 后\n"),
        Box::new(MemConfig::default()),
        images,
    );
    assert!(!a.config.allow_remote_images, "缺省配置必须阻止远程图片");
    a.ensure_layouts_all();
    assert!(requests.borrow().is_empty(), "默认阻止时不得发起请求");
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
    a.config.allow_remote_images = true;
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
    a.config.allow_remote_images = true;
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
    a.config.allow_remote_images = true;
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
    a.config.allow_remote_images = true;
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
    // 菜单与设置同文案、同极性：勾选 = 允许。
    assert_eq!(
        t(I18nKey::MenuViewAllowRemoteImages),
        t(I18nKey::SettingsAllowRemoteImages),
        "菜单与设置必须使用同一「允许远程图片」文案"
    );
    a.config.allow_remote_images = false;
    let flags = a.menu_flags();
    assert!(!flags.allow_remote_images, "阻止态进入菜单标志");
    assert!(!chrome::action_checked(
        MenuAction::ToggleRemoteImages,
        flags
    ));
    a.perform_menu(MenuAction::ToggleRemoteImages);
    assert!(a.config.allow_remote_images, "切换后恢复允许");
    assert!(a.menu_flags().allow_remote_images);
    assert!(chrome::action_checked(
        MenuAction::ToggleRemoteImages,
        a.menu_flags()
    ));
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

/// Task 6.3 反向同步：设置里改主题/远程图片后，View 菜单的勾选状态必须立即跟随，
/// 不能只有「菜单 → 设置」单向同步。
#[test]
fn settings_changes_update_view_menu_state_immediately() {
    use crate::view::settings::{FieldValue, BTN_OK};
    let cfg = MemConfig::default();
    let mut a = app_remote("abc", Box::new(cfg), RemoteImages::default());
    assert!(!a.config.allow_remote_images, "缺省配置必须阻止远程图片");

    a.open_settings();
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        for f in &mut s.form.fields {
            match f.label {
                I18nKey::SettingsThemeMode => f.value = FieldValue::ThemeMode(ThemeMode::Dark),
                I18nKey::SettingsAllowRemoteImages => f.value = FieldValue::Toggle(false),
                _ => {}
            }
        }
    }
    a.resolve_settings(BTN_OK);

    assert!(!a.config.allow_remote_images, "设置必须落到 Config");
    assert!(
        !chrome::action_checked(MenuAction::ToggleRemoteImages, a.menu_flags()),
        "取消允许后 View 菜单必须立即变为未勾选"
    );
    assert!(
        chrome::theme_action_matches(MenuAction::ThemeDark, a.config.mode),
        "切到深色后主题菜单「当前」标记必须立即跟随"
    );

    // 反向：允许远程 + 浅色，菜单同步回勾选态。
    a.open_settings();
    {
        let s = a.ui_state.settings.as_mut().expect("设置已打开");
        for f in &mut s.form.fields {
            match f.label {
                I18nKey::SettingsThemeMode => f.value = FieldValue::ThemeMode(ThemeMode::Light),
                I18nKey::SettingsAllowRemoteImages => f.value = FieldValue::Toggle(true),
                _ => {}
            }
        }
    }
    a.resolve_settings(BTN_OK);
    assert!(a.config.allow_remote_images);
    assert!(chrome::action_checked(
        MenuAction::ToggleRemoteImages,
        a.menu_flags()
    ));
    assert!(chrome::theme_action_matches(
        MenuAction::ThemeLight,
        a.config.mode
    ));
}

// ---- 拖放（阶段 1）：图片插入落点；非图片按文档打开 ----

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
    assert!(
        f.app.ui_state.chrome.notice.contains("先保存文档"),
        "提示应给出可操作的下一步"
    );
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
    assert!(f.app.ui_state.chrome.notice.contains("保存失败"));
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
    a.config.source_plain_lines = false; // 本测试锁定旧块布局的源码视图
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
    assert_eq!(actionable, 9, "本地预览/插入与复制源码共 9 个可执行项");
    assert!(
        a.ui_state.ctx.mermaid_insert_start.is_none(),
        "非空块不启用空块插入锚点"
    );
}

/// 预览/插入共用本地渲染：点击「预览」直接同步渲染并打开浮层，且不改文档。
#[test]
fn mermaid_preview_renders_locally_without_network() {
    let store = MemStore::default();
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(test_mermaid_image(12, 8));
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_remote_assets(
        doc,
        None,
        &store,
        images,
        FakeAssets {
            result: Ok("d.assets/mermaid-1.png".into()),
            calls: Rc::new(RefCell::new(Vec::new())),
        },
    );
    a.ports.mermaid = Box::new(mermaid.clone());

    exec_mermaid_action(&mut a, ctx_menu::CtxAction::MermaidPreviewImage);

    assert!(requests.borrow().is_empty(), "本地预览不得发起远程请求");
    assert_eq!(mermaid.blocking_count(), 1, "本地预览应同步渲染一次");
    assert!(
        a.ui_state.image_preview.is_some(),
        "本地渲染后应打开图片浮层"
    );
    let n = a.editor.session().text_len();
    assert_eq!(
        a.editor.session().text_slice(Range::new(0, n)).unwrap(),
        doc,
        "预览不得修改文档"
    );
}

/// 下载插入：写入 assets 并在闭合围栏之后自成段落；一次事务（undo_depth 恰 +1）。
/// 断言完整文本，而不是 `contains`——图片进入围栏内部时必须失败。
#[test]
fn mermaid_local_insert_writes_asset_as_one_undo_step() {
    let store = MemStore::default();
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(test_mermaid_image(20, 10));
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n尾段\n";
    let mut a = app_remote_assets(
        doc,
        Some("C:\\tmp\\d.md"),
        &store,
        images,
        FakeAssets {
            result: Ok("d.assets/mermaid-1.png".into()),
            calls: calls.clone(),
        },
    );
    a.ports.mermaid = Box::new(mermaid.clone());
    let depth0 = a.editor.session().undo_depth();

    exec_mermaid_action(&mut a, ctx_menu::CtxAction::MermaidInsertImage);

    assert!(requests.borrow().is_empty(), "本地插入不得发起远程请求");
    assert_eq!(mermaid.blocking_count(), 1);
    assert_eq!(calls.borrow().len(), 1, "应落盘一次资产");
    assert_eq!(calls.borrow()[0].0, PathBuf::from("C:\\tmp\\d.md"));
    let text = a.editor.session().serialize();
    assert_eq!(
        text, "p\n\n```mermaid\ngraph TD\nA-->B\n```\n![](d.assets/mermaid-1.png)\n尾段\n",
        "图片必须位于闭合围栏之后、自成段落"
    );
    assert_eq!(
        text.matches("![](").count(),
        1,
        "一次动作只新增一个图片引用"
    );
    assert_eq!(
        a.editor.session().undo_depth(),
        depth0 + 1,
        "插入必须是一条 undo 步骤"
    );
    assert_eq!(
        t(I18nKey::CtxMermaidInsertImage),
        "插入本地图片快照（保留源码）"
    );
    assert!(
        a.ui_state
            .chrome
            .notice
            .contains("上方 Mermaid 源码块仍是活预览"),
        "提示需说明活预览与快照的关系：{}",
        a.ui_state.chrome.notice
    );
    assert!(a.ui_state.chrome.notice.contains("不会随源码更新"));
    assert!(a.ui_state.chrome.notice.contains("d.assets/mermaid-1.png"));
}

/// 旧配置 `mermaid_insert_url = true` 仍只写本地资产：旧键在解析层静默忽略，
/// 快照动作不再有 URL 分支。
#[test]
fn mermaid_snapshot_ignores_legacy_insert_url_config() {
    let store = MemStore::default();
    let images = RemoteImages::default();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(test_mermaid_image(20, 10));
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n尾段\n";
    let mut env = theme::test_support::env();
    env.config = Box::new(MemConfig {
        text: Rc::new(RefCell::new(Some(
            "mermaid_insert_url = true\nallow_remote_images = true\n".into(),
        ))),
        saves: Rc::new(RefCell::new(Vec::new())),
    });
    let mut a = app_remote_assets_env(
        doc,
        Some("C:\\tmp\\d.md"),
        &store,
        images,
        FakeAssets {
            result: Ok("d.assets/mermaid-1.png".into()),
            calls: calls.clone(),
        },
        env,
    );
    a.ports.mermaid = Box::new(mermaid);
    assert!(
        a.ui_state.chrome.notice.is_empty(),
        "旧键不得产生无法识别提示：{}",
        a.ui_state.chrome.notice
    );

    exec_mermaid_action(&mut a, ctx_menu::CtxAction::MermaidInsertImage);

    assert_eq!(calls.borrow().len(), 1, "旧配置 true 也必须本地落盘");
    let text = a.editor.session().serialize();
    assert_eq!(
        text,
        "p\n\n```mermaid\ngraph TD\nA-->B\n```\n![](d.assets/mermaid-1.png)\n尾段\n"
    );
}

/// 同文两个完全相同的 Mermaid 块：右键第二个插入快照，必须落在第二个块之后，
/// 第一个块保持不变。
#[test]
fn mermaid_snapshot_targets_second_identical_block() {
    let store = MemStore::default();
    let images = RemoteImages::default();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(test_mermaid_image(20, 10));
    let doc =
        "前\n\n```mermaid\ngraph TD\nA-->B\n```\n中\n\n```mermaid\ngraph TD\nA-->B\n```\n尾\n";
    let mut a = app_remote_assets(
        doc,
        Some("C:\\tmp\\d.md"),
        &store,
        images,
        FakeAssets {
            result: Ok("d.assets/mermaid-1.png".into()),
            calls: calls.clone(),
        },
    );
    a.ports.mermaid = Box::new(mermaid);

    exec_nth_mermaid_action(&mut a, 1, ctx_menu::CtxAction::MermaidInsertImage);

    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(
        a.editor.session().serialize(),
        "前\n\n```mermaid\ngraph TD\nA-->B\n```\n中\n\n```mermaid\ngraph TD\nA-->B\n```\n![](d.assets/mermaid-1.png)\n尾\n",
        "快照必须落在第二个块之后，第一个块不得被改动"
    );
}

#[test]
fn mermaid_offline_insert_stays_local_when_remote_blocked() {
    let store = MemStore::default();
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(test_mermaid_image(20, 10));
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_remote_assets(
        doc,
        Some("C:\\tmp\\d.md"),
        &store,
        images,
        FakeAssets {
            result: Ok("d.assets/mermaid-1.png".into()),
            calls: calls.clone(),
        },
    );
    a.ports.mermaid = Box::new(mermaid.clone());
    a.config.allow_remote_images = false;

    exec_mermaid_action(&mut a, ctx_menu::CtxAction::MermaidInsertImage);

    assert!(requests.borrow().is_empty(), "Mermaid 快照不得发起远程请求");
    assert_eq!(mermaid.blocking_count(), 1);
    assert_eq!(calls.borrow().len(), 1, "离线版固定本地落盘");
    let inserted = a.editor.session().serialize();
    assert!(
        inserted.contains("![](d.assets/mermaid-1.png)"),
        "{inserted}"
    );
    assert!(!inserted.contains("https://mermaid.ink"), "{inserted}");
    assert!(inserted.contains("graph TD\nA-->B"), "活源码必须保留");
    assert!(a.ui_state.chrome.notice.contains("不会随源码更新"));

    // 后续编辑只改活源码；独立快照引用保持原样，也不触发第二次落盘。
    let source_edit = inserted.find("A-->B").unwrap() + "A-->B".len();
    a.editor.set_caret(source_edit);
    a.insert_paste("\nA-->C");
    let edited = a.editor.session().serialize();
    assert!(edited.contains("A-->B\nA-->C"));
    assert!(edited.contains("![](d.assets/mermaid-1.png)"));
    assert_eq!(calls.borrow().len(), 1, "编辑源码不得重写独立快照");
}

#[test]
fn mermaid_offline_preview_works_when_remote_images_blocked() {
    let store = MemStore::default();
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(test_mermaid_image(12, 8));
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_remote_assets(
        doc,
        None,
        &store,
        images,
        FakeAssets {
            result: Ok("d.assets/mermaid-1.png".into()),
            calls: Rc::new(RefCell::new(Vec::new())),
        },
    );
    a.ports.mermaid = Box::new(mermaid.clone());
    a.config.allow_remote_images = false;

    exec_mermaid_action(&mut a, ctx_menu::CtxAction::MermaidPreviewImage);

    assert!(requests.borrow().is_empty());
    assert_eq!(mermaid.blocking_count(), 1);
    let preview = a.ui_state.image_preview.as_ref().expect("本地预览应打开");
    assert!(preview.key.starts_with("mermaid-manual:"));
    assert!(a.cache.remote_image(&preview.key).is_some());
    assert_eq!(a.editor.session().serialize(), doc, "预览不得修改原文");
}

#[test]
fn mermaid_local_snapshot_draft_failure_preserves_document() {
    let store = MemStore::default();
    let images = RemoteImages::default();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(test_mermaid_image(20, 10));
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_remote_assets(
        doc,
        None,
        &store,
        images,
        FakeAssets {
            result: Ok("d.assets/mermaid-1.png".into()),
            calls: calls.clone(),
        },
    );
    a.ports.mermaid = Box::new(mermaid);

    exec_mermaid_action(&mut a, ctx_menu::CtxAction::MermaidInsertImage);

    assert_eq!(a.editor.session().serialize(), doc);
    assert!(!a.editor.is_dirty());
    assert!(calls.borrow().is_empty(), "草稿没有路径，不应尝试落盘");
    assert_eq!(
        a.ui_state.chrome.notice,
        t(I18nKey::NoticeMermaidSnapshotNeedsFile)
    );
}

#[test]
fn mermaid_offline_snapshot_save_failure_has_no_url_fallback() {
    let store = MemStore::default();
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(test_mermaid_image(20, 10));
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_remote_assets(
        doc,
        Some("C:\\tmp\\d.md"),
        &store,
        images,
        FakeAssets {
            result: Err("只读目录".into()),
            calls: calls.clone(),
        },
    );
    a.ports.mermaid = Box::new(mermaid);

    exec_mermaid_action(&mut a, ctx_menu::CtxAction::MermaidInsertImage);

    assert!(requests.borrow().is_empty());
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(a.editor.session().serialize(), doc, "失败不得改坏原文");
    assert!(!a.editor.is_dirty());
    assert!(!a.editor.session().serialize().contains("mermaid.ink"));
    assert!(a.ui_state.chrome.notice.contains("未修改原文"));
    assert!(a.ui_state.chrome.notice.contains("只读目录"));
}

#[test]
fn mermaid_offline_render_failure_preserves_source_without_url_fallback() {
    let store = MemStore::default();
    let images = RemoteImages::default();
    let requests = images.requests.clone();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mermaid = RecordingMermaid::new();
    *mermaid.blocking_result.borrow_mut() = Some(Err("本地渲染失败".into()));
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_remote_assets(
        doc,
        Some("C:\\tmp\\d.md"),
        &store,
        images,
        FakeAssets {
            result: Ok("d.assets/mermaid-1.png".into()),
            calls: calls.clone(),
        },
    );
    a.ports.mermaid = Box::new(mermaid);

    exec_mermaid_action(&mut a, ctx_menu::CtxAction::MermaidInsertImage);

    assert!(requests.borrow().is_empty());
    assert!(calls.borrow().is_empty());
    assert_eq!(a.editor.session().serialize(), doc);
    assert!(!a.editor.is_dirty());
    assert!(a.ui_state.chrome.notice.contains("本地渲染失败"));
    assert!(
        !a.ui_state.chrome.notice.contains("mermaid.ink"),
        "渲染失败提示不得指向第三方服务：{}",
        a.ui_state.chrome.notice
    );
    assert!(
        a.ui_state.chrome.notice.contains("请检查 Mermaid 语法"),
        "失败提示应给出本地可操作建议：{}",
        a.ui_state.chrome.notice
    );
    assert!(!a.editor.session().serialize().contains("mermaid.ink"));
}

#[test]
fn mermaid_snapshot_relative_path_survives_document_move() {
    let store = MemStore::default();
    let images = RemoteImages::default();
    let mermaid = RecordingMermaid::new();
    mermaid.set_blocking_ok(test_mermaid_image(20, 10));
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_remote_assets(
        doc,
        Some("C:\\old\\d.md"),
        &store,
        images,
        FakeAssets {
            result: Ok("d.assets/mermaid-1.png".into()),
            calls: Rc::new(RefCell::new(Vec::new())),
        },
    );
    a.ports.mermaid = Box::new(mermaid);
    exec_mermaid_action(&mut a, ctx_menu::CtxAction::MermaidInsertImage);
    let inserted = a.editor.session().serialize();
    assert!(inserted.contains("![](d.assets/mermaid-1.png)"));

    let moved_doc = PathBuf::from("C:\\moved\\d.md");
    let moved_png = PathBuf::from("C:\\moved\\d.assets\\mermaid-1.png");
    assert_eq!(
        local_image_path("d.assets/mermaid-1.png", Some(&moved_doc)),
        Some(moved_png.clone())
    );
    let (mut moved, loads, _files) = app_with_images(
        &inserted,
        Some("C:\\moved\\d.md"),
        &[("C:\\moved\\d.assets/mermaid-1.png", 20, 10)],
    );
    moved.ensure_layouts_all();

    assert_eq!(
        loads.borrow().as_slice(),
        &[moved_png],
        "移动后重新打开应按新文档目录加载相对快照"
    );
    assert_eq!(
        moved.editor.session().serialize(),
        inserted,
        "移动不得改写 Markdown 字节"
    );
}

/// 右键「预览」是显式外部动作：只复制源码并打开 mermaid.live 首页，
/// URL 不携带源码；不改文档字节。
#[test]
fn mermaid_preview_copies_source_and_opens_site_without_source_url() {
    let store = MemStore::default();
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = app_with(
        doc,
        None,
        false,
        &store,
        Box::new(FakeUrl {
            calls: calls.clone(),
            result: Ok(()),
        }),
    );
    let log = Rc::new(RefCell::new(Vec::new()));
    a.ports.clipboard = Box::new(ClipLog(log.clone()));

    exec_mermaid_preview(&mut a);

    assert_eq!(
        calls.borrow().as_slice(),
        &["https://mermaid.live/".to_string()],
        "URL 必须是站点首页，不携带源码"
    );
    assert_eq!(
        log.borrow().as_slice(),
        &["graph TD\nA-->B".to_string()],
        "源码必须复制到剪贴板"
    );
    assert_eq!(a.editor.session().serialize(), doc, "外部动作不得修改文档");
}

#[test]
fn mermaid_preview_open_failure_keeps_source_and_shows_recovery_notice() {
    let store = MemStore::default();
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = app_with(
        doc,
        None,
        false,
        &store,
        Box::new(FakeUrl {
            calls: calls.clone(),
            result: Err("模拟浏览器失败".into()),
        }),
    );
    let log = Rc::new(RefCell::new(Vec::new()));
    a.ports.clipboard = Box::new(ClipLog(log.clone()));

    exec_mermaid_preview(&mut a);

    assert_eq!(
        calls.borrow().as_slice(),
        &["https://mermaid.live/".to_string()]
    );
    assert_eq!(log.borrow().as_slice(), &["graph TD\nA-->B".to_string()]);
    assert!(a
        .ui_state
        .chrome
        .notice
        .contains("无法打开 mermaid.live：模拟浏览器失败"));
    assert!(a.ui_state.chrome.notice.contains("Mermaid 源码已复制"));
    assert!(a
        .ui_state
        .chrome
        .notice
        .contains("手动打开 https://mermaid.live/ 并粘贴源码"));
    assert_eq!(a.editor.session().serialize(), doc, "失败不得修改文档");
}

#[test]
fn mermaid_non_active_image_caret_reveal_and_code_background_preserve_source() {
    let mermaid = RecordingMermaid::new();
    let doc = "前文\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let mut a = app_with_mermaid(doc, mermaid.clone());
    a.trigger_mermaid_auto();
    let url = mermaid.last_request_url().expect("应请求自动预览");
    mermaid
        .poll_queue
        .borrow_mut()
        .push((url, Ok(test_mermaid_image(20, 10))));
    a.poll_remote_images();
    a.ensure_layouts_all();

    let id = block_id_of(&a, |k| matches!(k, BlockKind::Mermaid));
    assert!(
        a.cache.layouts[&id]
            .imgs
            .iter()
            .any(|img| matches!(img.key, InlineAsset::Mermaid(_))),
        "非激活块应显示自动渲染图"
    );

    let body = doc.find("graph TD").unwrap();
    a.editor.set_caret(body);
    a.ensure_layouts_all();
    let layout = &a.cache.layouts[&id];
    assert!(layout.layout_text().contains("```mermaid"));
    assert!(layout.layout_text().contains("graph TD\nA-->B"));
    assert!(
        !layout
            .imgs
            .iter()
            .any(|img| matches!(img.key, InlineAsset::Mermaid(_))),
        "caret 进入后必须 Reveal 源码而不是永久显示图片"
    );
    assert_eq!(a.editor.session().serialize(), doc, "Reveal 不得改写原文");

    a.recompute_geometry();
    let g = a.index.blocks.iter().find(|g| g.id == id).unwrap();
    let scale = a.input_state.scale;
    let pad_x = (crate::tokens::doc::CODE_PAD_X * scale).round() as i32;
    let pad_y = (crate::tokens::doc::CODE_PAD_Y * scale).round() as i32;
    let x = a.content_x() as i32 - pad_x;
    let y = (a.viewport().y + g.top - a.window_state.scroll_y) as i32 - pad_y;
    let code_bg = a.input_state.theme.code_bg;
    let sample_h = (a.engine.doc_line_height() * scale).max(1.0) as u32;
    let frame = a.render();
    assert!(
        any_pixel(frame, x, y, pad_x.max(1) as u32, sample_h, |pixel| pixel
            == code_bg,),
        "Mermaid 源码态应沿用代码背景"
    );
    assert_eq!(
        a.editor.session().serialize(),
        doc,
        "绘制不得改变 Markdown 往返字节"
    );
}

#[test]
fn mermaid_async_bitmap_stays_inside_own_block_before_and_after_scroll() {
    let mermaid = RecordingMermaid::new();
    let doc = "# Mermaid 上下文走查\n\n## 空 Mermaid\n\n```mermaid\n```\n\n## 非空 Mermaid\n\n```mermaid\nflowchart TD\n    A[开始] --> B[结束]\n```\n\n尾部正文。\n";
    let mut a = app_with_mermaid(doc, mermaid.clone());
    a.trigger_mermaid_auto();
    let url = mermaid.last_request_url().expect("非空块应请求自动预览");
    let mut bitmap = test_mermaid_image(96, 120);
    for px in bitmap.data.chunks_exact_mut(4) {
        px.copy_from_slice(&[237, 12, 160, 255]);
    }
    mermaid.poll_queue.borrow_mut().push((url, Ok(bitmap)));
    a.poll_remote_images();
    a.ensure_layouts_all();
    a.recompute_geometry();

    let source_at = doc.find("flowchart TD").unwrap();
    let heading_at = doc.find("非空 Mermaid").unwrap();
    let image_id = a
        .index
        .blocks
        .iter()
        .find(|g| {
            g.kind == BlockKind::Mermaid && g.range.start <= source_at && source_at <= g.range.end
        })
        .unwrap()
        .id;
    let heading_id = a
        .index
        .blocks
        .iter()
        .find(|g| g.range.start <= heading_at && heading_at <= g.range.end)
        .unwrap()
        .id;
    let empty_id = a
        .index
        .blocks
        .iter()
        .find(|g| g.kind == BlockKind::Mermaid && g.id != image_id)
        .unwrap()
        .id;
    assert!(
        a.cache.layouts[&empty_id].imgs.is_empty(),
        "空块不得借用后续块位图"
    );

    let (px, py, pw, ph, layout_h) = {
        let layout = &a.cache.layouts[&image_id].value;
        let p = layout
            .imgs
            .iter()
            .find(|p| matches!(p.key, InlineAsset::Mermaid(_)))
            .expect("异步回包后应重排出 Mermaid 位图");
        (p.x, p.y, p.width, p.height, layout.height)
    };
    assert!(py >= 0.0, "位图不得向上越出自己的块：y={py}");
    assert!(
        py + ph as f32 <= layout_h + 0.01,
        "块高必须包含完整位图：y={py}, h={ph}, layout_h={layout_h}"
    );
    let (image_top, image_h, heading_top, heading_h, tail_top) = {
        let image = a.index.blocks.iter().find(|g| g.id == image_id).unwrap();
        let heading = a.index.blocks.iter().find(|g| g.id == heading_id).unwrap();
        let tail = a.index.blocks.last().unwrap();
        (
            image.top,
            image.height,
            heading.top,
            heading.height,
            tail.top,
        )
    };
    assert!(
        heading_top + heading_h <= image_top,
        "相邻标题与图块几何不得重叠"
    );
    assert!(
        image_top + image_h <= tail_top,
        "位图底缘不得越过后续正文起点"
    );

    for scroll in [0.0, 17.0] {
        a.window_state.scroll_y = scroll;
        let vp = a.viewport();
        let cx = a.content_x() as i32;
        let cw = a.content_width() as u32;
        let heading_y = (vp.y + heading_top - scroll) as i32;
        let image_y = (vp.y + image_top - scroll + py) as i32;
        let frame = a.render();
        assert!(
            ink_pixels(frame, cx, heading_y, cw, heading_h as u32) > 0,
            "滚动 {scroll} 后相邻标题仍应可见"
        );
        assert!(
            !any_pixel(frame, cx, heading_y, cw, heading_h as u32, |p| p[0] > 200
                && p[1] < 80
                && p[2] > 100),
            "滚动 {scroll} 后 Mermaid 像素不得覆盖相邻标题"
        );
        assert!(
            any_pixel(frame, cx + px as i32, image_y, pw, ph, |p| p
                == [237, 12, 160, 255]),
            "滚动 {scroll} 后位图应绘制在自己的块内"
        );
    }
}

#[test]
fn mermaid_async_wide_bitmap_is_clamped_to_content_width() {
    // 回归：位图是 CSS 尺寸 ×2 光栅副本，≥2000px 宽图曾按原始像素当逻辑尺寸，
    // 越出版心并与相邻块重叠。修复后按 DPI 还原 CSS 尺寸并限制在版心内。
    let mermaid = RecordingMermaid::new();
    let doc = "# 宽图\n\n```mermaid\nflowchart LR\n    A[开始] --> B[结束]\n```\n\n尾部正文。\n";
    let mut a = app_with_mermaid(doc, mermaid.clone());
    a.trigger_mermaid_auto();
    let url = mermaid.last_request_url().expect("应请求自动预览");
    mermaid
        .poll_queue
        .borrow_mut()
        .push((url, Ok(test_mermaid_image(2400, 1200))));
    a.poll_remote_images();
    a.ensure_layouts_all();
    a.recompute_geometry();

    let source_at = doc.find("flowchart LR").unwrap();
    let image_id = a
        .index
        .blocks
        .iter()
        .find(|g| {
            g.kind == BlockKind::Mermaid && g.range.start <= source_at && source_at <= g.range.end
        })
        .unwrap()
        .id;
    let (px, pw, ph, layout_h) = {
        let layout = &a.cache.layouts[&image_id].value;
        let p = layout
            .imgs
            .iter()
            .find(|p| matches!(p.key, InlineAsset::Mermaid(_)))
            .expect("回包后应排出版图");
        (p.x, p.width, p.height, layout.height)
    };
    let content_w = a.content_width();
    assert!(
        px >= -0.01 && px + pw as f32 <= content_w + 0.5,
        "宽位图不得越出版心：x={px}, w={pw}, content_w={content_w}"
    );
    assert!(
        pw as f32 <= content_w + 0.5,
        "宽位图宽度必须压到版心内：w={pw}, content_w={content_w}"
    );
    assert!(
        ph as f32 <= layout_h + 0.01,
        "块高必须包含完整位图：h={ph}, layout_h={layout_h}"
    );

    let image = a.index.blocks.iter().find(|g| g.id == image_id).unwrap();
    let next = a
        .index
        .blocks
        .iter()
        .find(|g| g.range.start >= image.range.end && g.id != image_id)
        .expect("应有后续块");
    assert!(
        image.top + image.height <= next.top,
        "图块底缘不得越过后续块起点：{} + {} > {}",
        image.top,
        image.height,
        next.top
    );
}

#[test]
fn mermaid_copy_source_copies_body_without_changing_document() {
    let store = MemStore::default();
    let doc = "p\n\n```mermaid\ngraph TD\nA-->B\n```\n";
    let copied = Rc::new(RefCell::new(Vec::new()));
    let mut a = app(doc, None, false, &store);
    a.ports.clipboard = Box::new(ClipLog(copied.clone()));

    exec_mermaid_action(&mut a, ctx_menu::CtxAction::MermaidCopySource);

    assert_eq!(copied.borrow().as_slice(), &["graph TD\nA-->B".to_string()]);
    assert_eq!(a.editor.session().serialize(), doc);
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
    a.config.source_plain_lines = false; // 本测试锁定旧块布局的源码视图
    a.ensure_layouts_all();
    let id = block_id_of(&a, |k| matches!(k, BlockKind::Mermaid));
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
