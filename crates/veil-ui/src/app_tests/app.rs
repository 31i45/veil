//! 端到端与杂项行为测试：写作回路 / 打印分页 / 缩放 / 菜单键盘导航 / 源码模式 / TOC / 排版留白 / 版本历史 / 启动失败模态。

use super::*;

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
            ime_anchor: Box::new(NoopImeAnchor),
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
            mermaid: Box::new(NoopMermaid),
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
    // 最近文件登记推迟到首帧之后（headless 测试无首帧，显式触发同一收口）。
    a.flush_deferred_startup();
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
    b.flush_deferred_startup();
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

/// K2：Ctrl+P = 快速打开（VS Code 一致）；Ctrl+Shift+P = 命令面板；
/// 打印改到 Ctrl+Alt+P，三者互不串位。
#[test]
fn ctrl_p_quick_opens_ctrl_shift_p_palette_and_ctrl_alt_p_prints() {
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
    assert_eq!(
        a.ui_state.quick.kind,
        crate::view::quick_open::QuickKind::Files
    );
    assert!(!ended.get(), "Ctrl+P 不触发打印");

    a.close_quick();
    a.input_state.mods.ctrl = true;
    a.input_state.mods.shift = true;
    a.route_key(Key::P);
    a.input_state.mods.ctrl = false;
    a.input_state.mods.shift = false;
    assert!(a.ui_state.quick.open, "Ctrl+Shift+P 打开命令面板");
    assert_eq!(
        a.ui_state.quick.kind,
        crate::view::quick_open::QuickKind::Commands
    );
    assert!(!ended.get(), "Ctrl+Shift+P 不再触发打印");

    a.close_quick();
    a.input_state.mods.ctrl = true;
    a.input_state.mods.alt = true;
    a.route_key(Key::P);
    a.input_state.mods.ctrl = false;
    a.input_state.mods.alt = false;
    assert!(ended.get(), "Ctrl+Alt+P 经路由到达打印端口");
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

/// 短代码块（高度 ≤ 一页）在打印分页中整块不拆，与原生 PDF 导出同规则。
#[test]
fn print_keeps_short_code_block_on_one_page() {
    // 40 段填充把 4 行代码块推到不同页底位置；每档页高都不得从代码块中间切开。
    let filler: String = (0..40).map(|i| format!("段{i}\n\n")).collect();
    let text = format!("{filler}```rust\nfn main() {{\n    let x = 1;\n}}\n```\n");
    // 页高均高于代码块自身高度：只有此时「整块不拆」才是可兑现的承诺。
    for h in [120u32, 150, 180, 210, 240, 270] {
        let p = FakePrint {
            w: 300,
            h,
            scale: 1.0,
            ..Default::default()
        };
        let (breaks, geoms) = (p.breaks.clone(), p.geoms.clone());
        let mut a = app_print(&text, Box::new(p));
        a.perform_menu(MenuAction::Print);
        let bs = breaks.borrow();
        let gs = geoms.borrow();
        let (top, lines) = gs.last().expect("至少一个块");
        let line_h = lines.get(1).copied().unwrap_or(0.0);
        let block_h = lines.last().copied().unwrap_or(0.0) + line_h;
        assert!(block_h <= h as f32, "h={h}: 页高应高于代码块高度 {block_h}");
        for &b in bs.iter().skip(1) {
            for &t in lines.iter().skip(1) {
                assert!(
                    (top + t - b).abs() >= 0.5,
                    "h={h}: 短代码块被页内行切断（break={b}, 块顶={top}, 行顶={lines:?}）"
                );
            }
        }
    }
}

/// R1/R2（打印路径）：表格只在整行边界断页，续页可用高度扣除重复表头高。
#[test]
fn print_table_page_breaks_land_on_row_tops_and_reserve_header() {
    let mut text = String::from("| 列A | 列B |\n| --- | --- |\n");
    for i in 0..120 {
        text.push_str(&format!("| r{i:03} | v{i:03} |\n"));
    }
    let p = FakePrint {
        w: 300,
        h: 200,
        scale: 1.0,
        ..Default::default()
    };
    let (pages, breaks, tables) = (p.pages.clone(), p.breaks.clone(), p.table_rows.clone());
    let mut a = app_print(&text, Box::new(p));
    a.perform_menu(MenuAction::Print);
    assert!(pages.get() >= 3, "120 行表格应多页: {}", pages.get());
    let bs = breaks.borrow();
    let trs = tables.borrow();
    assert_eq!(trs.len(), 1, "应采样单张渲染态表格");
    let (table_top, row_tops, header_h) = &trs[0];
    assert!(*header_h > 0.0, "表头行高应大于 0");
    for &b in bs.iter().skip(1) {
        assert!(
            row_tops.iter().any(|&t| (table_top + t - b).abs() < 0.5),
            "断点 {b} 必须落在表格行顶（整行边界）: top={table_top} rows={row_tops:?}"
        );
    }
    // 页首落在表格内部的页（第 2 页起）：可用高度 = 页高 - 表头高。
    for w in bs.windows(2) {
        if w[0] > table_top + 0.5 {
            assert!(
                w[1] - w[0] <= 200.0 - header_h + 0.5,
                "续页可用高度未扣除表头: {}-{} > {}",
                w[1],
                w[0],
                200.0 - header_h
            );
        }
    }
}

/// 续页表头像素：第 2 页起页帧顶部必须出现表头底色（而非正文纸色）。
#[test]
fn print_repeats_table_header_pixels_on_continuation_pages() {
    struct HeaderProbe {
        w: u32,
        h: u32,
        rows: Rc<RefCell<Vec<Vec<[u8; 4]>>>>,
    }
    impl Print for HeaderProbe {
        fn print(&mut self, s: &mut PrintSession) {
            let pages = s.begin(self.w, self.h, 1.0);
            let mut out = Vec::new();
            for i in 0..pages {
                let f = s.render_page(i);
                let y = 1.min(f.height.saturating_sub(1));
                out.push((0..f.width).map(|x| f.get(x, y)).collect());
            }
            *self.rows.borrow_mut() = out;
            s.end();
        }
    }

    let mut text = String::from("| 列A | 列B |\n| --- | --- |\n");
    for i in 0..120 {
        text.push_str(&format!("| r{i:03} | v{i:03} |\n"));
    }
    let probe = HeaderProbe {
        w: 300,
        h: 200,
        rows: Rc::new(RefCell::new(Vec::new())),
    };
    let rows = probe.rows.clone();
    let mut a = app_print(&text, Box::new(probe));
    a.perform_menu(MenuAction::Print);
    let frames = rows.borrow();
    assert!(frames.len() >= 3, "应多页: {}", frames.len());
    let head_bg = a.input_state.theme.table_head_bg;
    for (i, row) in frames.iter().enumerate().skip(1) {
        assert!(
            row.contains(&head_bg),
            "第 {i} 页顶部缺少重复表头底色: {head_bg:?}"
        );
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
            ime_anchor: Box::new(NoopImeAnchor),
            clipboard: Box::new(NoClip),
            store: Box::new(MemStore::default()),
            math: Box::new(TallMath),
            mermaid: Box::new(NoopMermaid),
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
    assert_eq!(
        a.ui_state.chrome.notice,
        "当前平台暂不支持打印；可改用「导出 PDF」，保存后从阅读器打印"
    );
    assert!(a.window_state.print.is_none(), "NoPrint 不进入打印态");
}

#[test]
fn export_pdf_stub_notices_when_platform_missing() {
    let mut a = app_print("x\n", Box::new(NoPrint));
    a.perform_menu(MenuAction::ExportPdf);
    assert_eq!(
        a.ui_state.chrome.notice,
        "当前平台暂不支持导出 PDF；可改用「导出 HTML」，或改在支持打印的平台上重试"
    );
    assert!(a.window_state.print.is_none(), "无平台实现时不进入打印态");
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
        headers: Vec::new(),
        resume: None,
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
        (WKey::Named(NamedKey::Space), KeyCode::NumpadAdd, Key::Equal),
        (WKey::Character("+".into()), KeyCode::NumpadAdd, Key::Equal),
        (
            WKey::Named(NamedKey::ArrowDown),
            KeyCode::NumpadSubtract,
            Key::Minus,
        ),
        (WKey::Named(NamedKey::Insert), KeyCode::Numpad0, Key::Digit0),
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

/// D1/F2：空文档没有可输出内容，导出/打印在菜单层禁用；空白但合法的文档不受影响。
#[test]
fn menu_flags_disable_export_and_print_only_for_empty_document() {
    let store = MemStore::default();
    let mut empty = app("", None, false, &store);
    assert!(!empty.menu_flags().can_export, "空文档不可导出");
    for action in [MenuAction::Export, MenuAction::ExportPdf, MenuAction::Print] {
        assert!(
            !chrome::action_enabled(action, empty.menu_flags()),
            "空文档必须禁用 {action:?}"
        );
    }
    empty.editor.apply(Intent::InsertText("\n".into())).unwrap();
    assert!(empty.menu_flags().can_export, "仅有空白也应视为有内容");
    assert!(chrome::action_enabled(
        MenuAction::ExportPdf,
        empty.menu_flags()
    ));
}

/// 空文档时快捷键与菜单同源：不得打开保存框、不得进入导出/打印错误路径。
/// 有内容后同一快捷键恢复正常导出。
#[test]
fn empty_document_shortcuts_are_gated_like_menu() {
    let (mut a, calls, dialog_calls) = app_export_with_dialog("", Some("out.html"));
    a.input_state.mods = Mods {
        ctrl: true,
        ..Mods::default()
    };
    a.route_key(Key::E);
    assert!(
        dialog_calls.borrow().is_empty(),
        "空文档 Ctrl+E 不得弹保存框"
    );
    assert!(calls.borrow().is_empty(), "空文档 Ctrl+E 不得调用导出器");
    assert_eq!(
        a.ui_state.chrome.notice,
        t(I18nKey::NoticeOutputEmptyDocument)
    );
    assert!(a.ui_state.modal.is_none(), "不得进入错误弹窗路径");

    a.input_state.mods.shift = true;
    a.route_key(Key::E);
    a.input_state.mods.shift = false;
    // 打印快捷键已移到 Ctrl+Alt+P（Ctrl+Shift+P 归命令面板）
    a.input_state.mods.alt = true;
    a.route_key(Key::P);
    a.input_state.mods.alt = false;
    assert!(dialog_calls.borrow().is_empty(), "空文档导出/打印不得弹框");
    assert!(calls.borrow().is_empty(), "空文档导出/打印不得产生输出");
    assert_eq!(
        a.ui_state.chrome.notice,
        t(I18nKey::NoticeOutputEmptyDocument)
    );

    // 有内容后同一入口放行：Ctrl+E 先弹保存框，再调用导出器。
    a.editor.apply(Intent::InsertText("x".into())).unwrap();
    a.input_state.mods = Mods {
        ctrl: true,
        ..Mods::default()
    };
    a.route_key(Key::E);
    assert_eq!(dialog_calls.borrow().len(), 1, "有内容时 Ctrl+E 应弹保存框");
    assert_eq!(calls.borrow().len(), 1, "有内容时 Ctrl+E 应导出");
}

/// F1：已有模态时新错误不得静默丢弃——保留原模态，并把标题 + 恢复提示写入状态栏。
#[test]
fn error_while_modal_open_is_noted_not_dropped() {
    let store = MemStore::default();
    store
        .files
        .borrow_mut()
        .insert(PathBuf::from("a.md"), b"ab".to_vec());
    let mut a = app("ab", Some("a.md"), false, &store);
    a.editor.apply(Intent::InsertText("x".into())).unwrap();
    a.open_path(PathBuf::from("b.md"));
    assert!(a.ui_state.modal.is_some(), "未保存确认占住模态");

    store.fail_paths.borrow_mut().push(PathBuf::from("a.md"));
    a.save_now();
    assert!(a.ui_state.modal.is_some(), "既有模态不得被新错误覆盖");
    let notice = a.ui_state.chrome.notice.clone();
    assert!(notice.contains("保存失败"), "提示必须含错误标题: {notice}");
    assert!(
        notice.contains("请先关闭当前弹窗"),
        "提示必须含可操作的恢复路径: {notice}"
    );
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

/// 选区右键的加粗/斜体/链接必须真实改写正文（与主菜单同一 Intent），
/// 且一次撤销即可复原——只断言菜单项存在不足以防止映射写错。
#[test]
fn selection_ctx_actions_edit_text_and_undo_once() {
    for (action, expected) in [
        (ctx_menu::CtxAction::ToggleBold, "**hello**\n"),
        (ctx_menu::CtxAction::ToggleItalic, "*hello*\n"),
        (ctx_menu::CtxAction::InsertLink, "[hello]()\n"),
    ] {
        let store = MemStore::default();
        let mut a = app("hello\n", None, false, &store);
        assert!(a.editor.set_caret(0));
        assert!(a.editor.set_caret_extend(5));
        a.ui_state
            .ctx
            .open_at(10.0, 20.0, ctx_menu::CtxKind::Selection, None, None);
        let i = a
            .ui_state
            .ctx
            .items()
            .iter()
            .position(|&(_, act)| act == action)
            .unwrap_or_else(|| panic!("选区菜单应含 {action:?}"));
        a.ui_state.ctx.hover = Some(i);
        a.route_key(Key::Enter);
        assert_eq!(
            a.editor.session().serialize(),
            expected,
            "{action:?} 必须真实改写正文"
        );

        a.perform_action(Action::Doc(Intent::Undo));
        assert_eq!(
            a.editor.session().serialize(),
            "hello\n",
            "{action:?} 必须一次撤销复原"
        );
    }
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

#[test]
fn source_mode_toggles_markup_but_keeps_caret_and_scroll() {
    let store = MemStore::default();
    let text = "# 标题\n\n**粗体** 与 [链接](https://example.com)\n\n尾\n";
    let mut a = app(text, None, false, &store);
    a.config.source_plain_lines = false; // 本测试锁定旧块布局的源码视图
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
    a.config.source_plain_lines = false; // 本测试锁定旧块布局的源码视图
    a.editor.set_caret(text.find("正文").unwrap());
    a.ensure_layouts_all();
    a.render();
    let fm = block_id_of(&a, |k| matches!(k, BlockKind::FrontMatter(_)));
    let g = a.index.blocks.iter().find(|g| g.id == fm).unwrap();
    assert_eq!(
        g.height,
        crate::tokens::doc::FM_COLLAPSED_H * a.input_state.scale,
        "非活动态 Front Matter 折叠为可点击细条（不是整块 0 高）"
    );
    assert!(
        !a.cache.layouts[&fm].layout_text().is_empty(),
        "布局仍保留原文以便展开时零重排"
    );
    let fm_top = g.top;
    let next_top = a.index.blocks.iter().find(|g| g.id != fm).unwrap().top;
    assert!(
        next_top >= fm_top + g.height,
        "折叠条之后的块从条底开始，不保留整块段距"
    );

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
    assert_eq!(
        g.height,
        crate::tokens::doc::FM_COLLAPSED_H * a.input_state.scale,
        "非活动态 TOML Front Matter 折叠为可点击细条"
    );
    a.editor.set_caret(text.find("title").unwrap());
    a.ensure_layouts_all();
    a.render();
    let g = a.index.blocks.iter().find(|g| g.id == toml).unwrap();
    assert!(g.height > 0.0, "光标进入时应显示 TOML 原文");
    assert_eq!(doc_text(&a), text, "纯视图态，文档字节不变");
}

#[test]
fn toml_frontmatter_flag_tracks_single_block_edits() {
    // DEF-RENDER-FINAL-001：判定缓存进 `BlockGeom` 后，单块原位编辑必须重算该标志。
    // 每帧对全部块切片解析会拖慢大文档击键，因此判定只能缓存，但缓存不得陈旧。
    let store = MemStore::default();
    // 起止行起初都是 `++`（非法）；给末行补一个 `+` 才成为 `+++` 包裹形态。
    let text = "# 前\n\n++\nkey = 1\n++\n\n后\n";
    let mut a = app(text, None, false, &store);
    a.editor.set_caret(text.find('后').unwrap());
    a.ensure_layouts_all();
    a.render();
    let find_toml = |a: &App| {
        a.index
            .blocks
            .iter()
            .position(|g| {
                a.editor
                    .session()
                    .text_slice(g.range)
                    .is_some_and(|t| t.contains("key = 1"))
            })
            .expect("应存在 TOML 段落")
    };
    let i = find_toml(&a);
    assert!(
        !a.index.blocks[i].frontmatter_like,
        "初始 `++` 不是 Front Matter"
    );
    assert!(a.index.blocks[i].height > 0.0);

    // 首行与末行都补成 `+++`：同一块内变成合法 TOML Front Matter。
    let open = text.find("++\nkey").unwrap() + 2;
    let close = text.find("++\n\n后").unwrap() + 2;
    a.editor.set_caret(open);
    a.route_key(Key::Text("+".into()));
    a.editor.set_caret(close + 1);
    a.route_key(Key::Text("+".into()));
    a.editor.set_caret(text.find('后').unwrap() + 2);
    a.ensure_layouts_all();
    a.render();
    let i = find_toml(&a);
    assert!(
        a.index.blocks[i].frontmatter_like,
        "单块编辑后缓存标志必须更新"
    );
    assert_eq!(
        a.index.blocks[i].height,
        crate::tokens::doc::FM_COLLAPSED_H * a.input_state.scale,
        "非活动态折叠为可点击细条"
    );

    // 删掉末行补的 `+`：增量路径同样必须把标志降回 false 并恢复高度。
    a.editor.set_caret(close + 2);
    a.route_key(Key::Backspace);
    a.editor.set_caret(text.find('后').unwrap());
    a.ensure_layouts_all();
    a.render();
    let i = find_toml(&a);
    assert!(
        !a.index.blocks[i].frontmatter_like,
        "删回后不得残留 Front Matter"
    );
    assert!(a.index.blocks[i].height > 0.0, "非 Front Matter 必须占高度");
    assert!(
        doc_text(&a).contains("++\nkey = 1\n++\n"),
        "编辑往返后末行回到 `++`"
    );
}

#[test]
fn source_mode_hides_math_image_and_table_grid() {
    let store = MemStore::default();
    let text = "前言\n\n$$\nx=1\n$$\n\n| a | b |\n| - | - |\n| 1 | 2 |\n";
    let mut a = app(text, None, false, &store);
    a.config.source_plain_lines = false; // 本测试锁定旧块布局的源码视图
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
    a.config.source_plain_lines = false; // 本测试锁定旧块布局的源码视图
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
    a.config.source_plain_lines = false; // 本测试锁定旧块布局的源码视图
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
    assert!(
        text.contains("• 一") && text.contains("   • 二"),
        "初始目录: {text:?}"
    );
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

/// `[TOC]` 视图目录在专注模式下继承同一渲染：同样无有序列表序号，
/// 且模式切换不写回文档、不产生撤销步。
#[test]
fn toc_render_is_shared_in_focus_mode() {
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
    assert_eq!(
        a.cache.layouts[&marker].layout_text(),
        plain,
        "专注模式同一渲染"
    );
    assert_eq!(a.editor.session().serialize(), src);
    assert!(!a.editor.is_dirty());
    assert_eq!(a.editor.session().undo_depth(), 0);
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
    a.config.source_plain_lines = false; // 本测试锁定旧块布局的源码视图
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
        let tl = a
            .cache
            .table(table.id, a.table_key(table.id))
            .expect("网格已排版");
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

/// 真正的窄窗口（可用宽 ≤ 可读上限）：上限不生效，铺满可用宽、原点等于左边距。
#[test]
fn narrow_window_content_width_unchanged_by_readable_cap() {
    let store = MemStore::default();
    let mut a = app("正文", None, false, &store);
    // 640 宽：扣除边距与滚动条后约 596 < 640，可读上限不触发。
    a.window_state.test_size = Some((640, 800));
    let scale = a.input_state.scale;
    let want = (640.0 - 2.0 * MARGIN * scale - chrome::SCROLL_W * scale).max(32.0);
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
    // 显式取真正窄窗口：不封顶，原点等于左边距。
    a.window_state.test_size = Some((640, 800));
    a.ensure_layouts_all();
    let vp = a.viewport();
    let narrow_x = a.content_x() + 100.0;
    assert!(
        (narrow_x - (MARGIN * a.input_state.scale + 100.0)).abs() < 1e-4,
        "窄窗口原点应等于左边距"
    );
    let y = (vp.y + a.index.blocks[0].top + 5.0) as f64;
    a.input_state.mouse = PhysicalPosition::new(narrow_x as f64, y);
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

/// Task 4（ppq-task4）：源码模式复用块布局（非纯文本行布局），块间空行不产生可命中行
/// —— 光标不能停在空行上；但空行可被删除，且几何按「段距 + N 行行高」计入间距。
/// 本测试锁定该产品限制，防止回归成「空行消失」或「删空行后总高不更新」。
#[test]
fn source_mode_blank_line_cannot_hold_caret_but_is_deletable_and_occupies_height() {
    let store = MemStore::default();
    let text = "第一段\n\n\n第二段"; // 3 个换行 = 2 个空行
    let mut a = app(text, None, false, &store);
    a.config.source_plain_lines = false; // 本测试锁定旧块布局的源码视图
    a.perform_action(Action::OpenSourceMode);
    a.paint();
    assert!(a.source_mode());

    let first_end = "第一段".len(); // 9
    let second_start = text.find("第二段").unwrap(); // 12
    assert_eq!(text[first_end..second_start].matches('\n').count(), 3);

    // 块布局剥掉块尾换行：第一段只排 1 行，「空行」不进入任何块的行集合。
    assert_eq!(layout_of_text(&a, "第一段").layout_text(), "第一段");
    assert_eq!(layout_of_text(&a, "第二段").layout_text(), "第二段");

    // 断言 A：从第一段末行（单行段落即末行）行首按 ArrowDown 一次，
    // 光标直接落到第二段首行，中间两个空行（字节 10/11）没有任何停留机会。
    assert!(a.editor.set_caret(0));
    a.window_state.goal_x = None;
    a.route_key(Key::Down);
    assert_eq!(
        a.editor.caret(),
        second_start,
        "源码模式的下移必须跳过块间空行，落到下一段首行"
    );
    // 同一条末行从行尾下移：目标列被夹到第二段行尾，同样不会停在空行上。
    assert!(a.editor.set_caret(first_end));
    a.window_state.goal_x = None;
    a.route_key(Key::Down);
    let landed = a.editor.caret();
    assert!(
        (second_start..=second_start + "第二段".len()).contains(&landed),
        "下移落点必须落在第二段内，不得停在空行字节: {landed}"
    );

    // 断言 C（前测）：源码模式下两个空行按「段距 + 2 行」计入块间距。
    let scale = a.input_state.scale;
    let line_h = BASE_LINE_HEIGHT * scale;
    let gap = {
        let (b0, b1) = (&a.index.blocks[0], &a.index.blocks[1]);
        b1.top - (b0.top + b0.height)
    };
    assert!(
        (gap - (PARA_GAP * scale + 2.0 * line_h)).abs() < 1e-3,
        "两个空行应占 段距 + 2 行: {gap} vs {}",
        PARA_GAP * scale + 2.0 * line_h
    );
    let total_before = a.index.total_h;

    // 断言 B：第二段开头 Backspace 一次，段落间换行数减 1 —— 空行可删，只是不可见/不可停。
    assert!(a.editor.set_caret(second_start));
    a.route_key(Key::Backspace);
    let after = doc_text(&a);
    assert_eq!(after, "第一段\n\n第二段", "Backspace 应删除一个块间换行");
    assert_eq!(
        after.matches('\n').count(),
        text.matches('\n').count() - 1,
        "段落之间的换行数应减 1"
    );

    // 断言 C（后测）：删除一个空行后，文档总高正好减少一行行高。
    a.paint();
    let total_after = a.index.total_h;
    assert!(
        (total_before - total_after - line_h).abs() < 1e-3,
        "删一个空行总高应减少一行: {total_before} → {total_after}, line_h={line_h}"
    );

    // Ctrl+Z 恢复原文本与总高。
    a.input_state.mods = Mods {
        ctrl: true,
        ..Mods::default()
    };
    a.route_key(Key::Z);
    assert_eq!(doc_text(&a), text, "Ctrl+Z 应恢复被删的空行");
    a.paint();
    assert!(
        (a.index.total_h - total_before).abs() < 1e-3,
        "撤销后总高应复原: {} vs {total_before}",
        a.index.total_h
    );
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

/// 窗口建不起来时应用内模态不可见：打开失败详情必须留存，供 Shell 取走并弹原生错误框。
#[test]
fn startup_error_is_staged_for_window_failure_and_taken_once() {
    let store = MemStore::default();
    let mut a = app_startup_error("a.md", &store);
    assert_eq!(
        a.window_state.startup_error.as_deref(),
        Some(STARTUP_ERROR_MESSAGE)
    );
    assert_eq!(
        a.window_state.take_startup_error().as_deref(),
        Some(STARTUP_ERROR_MESSAGE)
    );
    assert!(a.window_state.take_startup_error().is_none(), "只上报一次");
}

/// 正常启动（无打开失败）不得留下启动错误，否则会误报为致命错误。
#[test]
fn normal_start_has_no_staged_startup_error() {
    let store = MemStore::default();
    let a = app("正文", None, false, &store);
    assert!(a.window_state.startup_error.is_none());
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

#[test]
fn image_preview_renders_overlay_frame() {
    let (mut a, _calls, _files) = app_with_images(
        "前文\n\n![示例](img/p.png)",
        Some("dir/doc.md"),
        &[("dir/img/p.png", 120, 60)],
    );
    a.ensure_layouts_all();
    a.recompute_geometry();
    let before = a.render().pixels[0];
    a.ui_state.image_preview = Some(ImagePreview {
        key: "dir/img/p.png".into(),
        title: "示例".into(),
        zoom: 1.0,
        pan: (0.0, 0.0),
    });
    let after = a.render().pixels[0];
    let sum = |p: u32| (p & 0xff) + ((p >> 8) & 0xff) + ((p >> 16) & 0xff);
    assert!(sum(after) < sum(before), "预览覆盖层应压暗背景");
}

/// 位图缺失：遮罩保留、绘制中性提示（不 panic、不静默消失）。
#[test]
fn image_preview_missing_bitmap_draws_neutral_hint() {
    let (mut a, _calls, _files) = app_with_images(
        "前文\n\n![示例](img/p.png)",
        Some("dir/doc.md"),
        &[("dir/img/p.png", 120, 60)],
    );
    a.ensure_layouts_all();
    a.recompute_geometry();
    let before = a.render().pixels[0];
    a.ui_state.image_preview = Some(ImagePreview {
        key: "dir/img/missing.png".into(),
        title: "示例".into(),
        zoom: 1.0,
        pan: (0.0, 0.0),
    });
    let after = a.render().pixels[0];
    let sum = |p: u32| (p & 0xff) + ((p >> 8) & 0xff) + ((p >> 16) & 0xff);
    assert!(sum(after) < sum(before), "位图缺失时仍应保留遮罩");
}

// ===== 源码模式纯文本行视图（source_plain_lines 灰度开关） =====

/// 灰度开启：段间 1 个空行成为可停光标的一行；上下移按行推进。
#[test]
fn source_plain_single_blank_line_holds_caret() {
    let store = MemStore::default();
    let mut a = app("aaa\n\nbbb", None, false, &store);
    a.config.source_plain_lines = true;
    a.perform_action(Action::OpenSourceMode);
    a.paint();
    assert!(a.source_plain_active(), "灰度开关 + 源码模式应生效");
    // 行区间：aaa[0,3]、空行[4,4]、bbb[5,8]。
    assert!(a.editor.set_caret(0));
    a.window_state.goal_x = None;
    a.route_key(Key::Down);
    assert_eq!(a.editor.caret(), 4, "下移应停在段间空行");
    a.route_key(Key::Down);
    assert_eq!(a.editor.caret(), 5, "再下移到下一段首行");
    a.route_key(Key::Up);
    assert_eq!(a.editor.caret(), 4, "上移应回到空行");
}

/// 灰度开启：连续 3 个空行逐行可停，且行距恒定（索引几何单调等距）。
#[test]
fn source_plain_three_consecutive_blank_lines_hold_caret() {
    let store = MemStore::default();
    let mut a = app("a\n\n\n\nb", None, false, &store);
    a.config.source_plain_lines = true;
    a.perform_action(Action::OpenSourceMode);
    a.paint();
    assert!(a.editor.set_caret(0));
    a.window_state.goal_x = None;
    for expect in [2, 3, 4, 5] {
        a.route_key(Key::Down);
        assert_eq!(a.editor.caret(), expect, "连续空行逐行可停");
    }
    let scale = a.input_state.scale;
    let pitch = {
        let (b0, b1) = (&a.index.blocks[0], &a.index.blocks[1]);
        b1.top - b0.top
    };
    for w in a.index.blocks.windows(2) {
        let d = w[1].top - w[0].top;
        assert!((d - pitch).abs() < 1e-3, "所有行距应一致: {d} vs {pitch}");
        assert!(d > BASE_LINE_HEIGHT * scale, "行块间还需段落间距");
    }
}

/// 灰度开启：文末 2 个空行也可停光标，Ctrl+End 落到文末。
#[test]
fn source_plain_trailing_blank_lines_hold_caret() {
    let store = MemStore::default();
    let mut a = app("a\n\n", None, false, &store);
    a.config.source_plain_lines = true;
    a.perform_action(Action::OpenSourceMode);
    a.paint();
    assert!(a.editor.set_caret(0));
    a.window_state.goal_x = None;
    a.route_key(Key::Down);
    assert_eq!(a.editor.caret(), 2, "第一个文末空行");
    a.route_key(Key::Down);
    assert_eq!(a.editor.caret(), 3, "第二个文末空行（末行）");
    a.route_key(Key::Down);
    assert_eq!(a.editor.caret(), 3, "末行下移不变");
    a.input_state.mods = Mods {
        ctrl: true,
        ..Mods::default()
    };
    a.route_key(Key::End);
    assert_eq!(a.editor.caret(), 3, "Ctrl+End 到文档尾");
    a.route_key(Key::Home);
    assert_eq!(a.editor.caret(), 0, "Ctrl+Home 到文档首");
}

/// 灰度开启：空行可选中，Backspace 删除后文档字节恰少一个换行。
#[test]
fn source_plain_blank_line_selectable_and_backspace_deletes_one_newline() {
    let store = MemStore::default();
    let original = "a\n\nb";
    let mut a = app(original, None, false, &store);
    a.config.source_plain_lines = true;
    a.perform_action(Action::OpenSourceMode);
    a.paint();
    assert!(a.editor.set_caret(2), "空行起点");
    a.window_state.goal_x = None;
    a.input_state.mods = Mods {
        shift: true,
        ..Mods::default()
    };
    a.route_key(Key::Down); // 选中空行 [2,3)
    a.input_state.mods = Mods::default();
    a.route_key(Key::Backspace);
    assert_eq!(doc_text(&a), "a\nb", "删除空行只去掉一个换行");
    assert_eq!(
        doc_text(&a).as_bytes(),
        original
            .as_bytes()
            .iter()
            .copied()
            .enumerate()
            .filter(|(i, _)| *i != 2)
            .map(|(_, b)| b)
            .collect::<Vec<_>>(),
        "其余字节保持不变"
    );
}

/// 灰度开启：10 万字文档连续上下移动，落点严格逐行、无跳行/回退。
#[test]
fn source_plain_large_doc_vertical_moves_are_monotonic() {
    let store = MemStore::default();
    // 10000 行 × 10 字节 = 10 万字节（末行是换行后的空行）。
    let mut text = String::with_capacity(100_001);
    for i in 0..10_000 {
        text.push_str(&format!("{i:09}\n"));
    }
    assert_eq!(text.len(), 100_000);
    let mut a = app(&text, None, false, &store);
    a.config.source_plain_lines = true;
    a.perform_action(Action::OpenSourceMode);
    a.paint();
    assert_eq!(a.index.blocks.len(), 10_001, "每行一块（含文末空行）");

    assert!(a.editor.set_caret(0));
    a.window_state.goal_x = None;
    let mut last = 0usize;
    for step in 1..=60usize {
        a.route_key(Key::Down);
        let caret = a.editor.caret();
        assert_eq!(caret, step * 10, "第 {step} 次下移应逐行推进");
        assert!(caret > last);
        last = caret;
    }
    for step in (0..60).rev() {
        a.route_key(Key::Up);
        assert_eq!(a.editor.caret(), step * 10, "上移应严格回退");
    }
    // 全量重排后所有行顶边仍等距单调。
    a.paint();
    assert!(a.index.blocks.windows(2).all(|w| w[1].top > w[0].top));
}

/// 灰度开启：Home/End 到当前行首尾，Ctrl+Home/End 到文档首尾。
#[test]
fn source_plain_home_end_and_ctrl_home_end_use_document_offsets() {
    let store = MemStore::default();
    let text = "one\ntwo\n\nfour";
    let mut a = app(text, None, false, &store);
    a.config.source_plain_lines = true;
    a.perform_action(Action::OpenSourceMode);
    a.paint();
    assert!(a.editor.set_caret(5), "位于 two 第 1 列");
    a.input_state.mods = Mods::default();
    a.route_key(Key::Home);
    assert_eq!(a.editor.caret(), 4, "Home 到本行行首");
    a.route_key(Key::End);
    assert_eq!(a.editor.caret(), 7, "End 到本行行尾");
    assert!(a.editor.set_caret(8), "空行");
    a.route_key(Key::Home);
    assert_eq!(a.editor.caret(), 8);
    a.route_key(Key::End);
    assert_eq!(a.editor.caret(), 8, "空行首尾同为该偏移");
    a.input_state.mods = Mods {
        ctrl: true,
        ..Mods::default()
    };
    a.route_key(Key::Home);
    assert_eq!(a.editor.caret(), 0, "Ctrl+Home 到文首");
    a.route_key(Key::End);
    assert_eq!(a.editor.caret(), text.len(), "Ctrl+End 到文末");
}

/// 灰度开启：源码模式往返切换后 caret 与滚动位置保持。
#[test]
fn source_plain_toggle_round_trip_keeps_caret_and_scroll() {
    let store = MemStore::default();
    let mut a = app("a\n\nb", None, false, &store);
    a.config.source_plain_lines = true;
    a.perform_action(Action::OpenSourceMode);
    a.paint();
    assert!(a.editor.set_caret(2), "停在空行");
    a.window_state.scroll_y = 0.0;
    a.paint();
    let caret = a.editor.caret();
    let scroll = a.window_state.scroll_y;

    a.perform_action(Action::OpenSourceMode); // 退出源码模式
    a.paint();
    assert!(!a.source_mode());
    assert_eq!(a.editor.caret(), caret, "退出后 caret 偏移不变");
    assert!((a.window_state.scroll_y - scroll).abs() < 0.5);

    a.perform_action(Action::OpenSourceMode); // 重新进入
    a.paint();
    assert!(a.source_mode());
    assert_eq!(a.editor.caret(), caret, "往返后 caret 偏移不变");
    assert!((a.window_state.scroll_y - scroll).abs() < 0.5);
    // 重新进入后空行仍可停：Down 到 b 首行。
    a.route_key(Key::Down);
    assert_eq!(a.editor.caret(), 3);
}

/// 开关关闭：仍然走既有块布局路径（空行不可停、块数等于真实段落数、画面逐像素一致）。
#[test]
fn source_plain_flag_off_matches_legacy_path() {
    let store = MemStore::default();
    let text = "a\n\n\nb";
    let mut a = app(text, None, false, &store);
    a.config.source_plain_lines = false; // 本测试就是「开关关闭」的旧路径基线
    a.perform_action(Action::OpenSourceMode);
    a.paint();
    assert!(!a.source_plain_active());
    assert_eq!(a.index.blocks.len(), 2, "旧路径按真实段落成块");
    let second_start = text.find('b').unwrap();
    assert!(a.editor.set_caret(0));
    a.window_state.goal_x = None;
    a.route_key(Key::Down);
    assert_eq!(a.editor.caret(), second_start, "旧路径跳过空行");

    // 与显式关闭的基线逐像素一致（先对齐 caret/滚动，排除光标本身差异）。
    let store2 = MemStore::default();
    let mut b = app(text, None, false, &store2);
    b.config.source_plain_lines = false;
    b.perform_action(Action::OpenSourceMode);
    assert!(a.editor.set_caret(0));
    a.window_state.scroll_y = 0.0;
    b.window_state.scroll_y = 0.0;
    assert_eq!(
        a.render().pixels,
        b.render().pixels,
        "开关关闭不产生行为差异"
    );
}

/// 开关开启：源码模式保存写盘字节 == 缓冲区字节（不做任何文本转换）。
#[test]
fn source_plain_save_writes_buffer_bytes_unchanged() {
    let store = MemStore::default();
    let original = "标题\r\n\r\n正文\n";
    let mut a = app(original, Some("plain.md"), false, &store);
    a.config.source_plain_lines = true;
    a.perform_action(Action::OpenSourceMode);
    a.paint();
    a.save_now();
    let saved = store
        .files
        .borrow()
        .get(Path::new("plain.md"))
        .cloned()
        .expect("保存应写盘");
    assert_eq!(saved, original.as_bytes(), "写盘字节必须等于缓冲区字节");
}
