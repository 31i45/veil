//! 应用外壳测试：菜单路由 / 对话框编排 / 视口滚动条 / 状态栏 / 窗口 / 光标闪烁 / 代码头 / 通知分级 / 首窗尺寸 / 焦点归还。

use super::*;

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
fn first_menu_open_clears_preedit() {
    let store = MemStore::default();
    let (mut a, _) = app_dialogs("ab\ncd", Some("a.md"), false, &store, None, None);
    a.editor.set_preedit("zh".into(), None);
    assert!(a.editor.preedit().is_some());

    let (tx, ty) = menu_title_center(0);
    click_at(&mut a, tx, ty);

    assert!(a.ui_state.chrome.is_open(), "点击菜单栏应打开菜单");
    assert!(
        a.editor.preedit().is_none(),
        "首次打开菜单必须立即清除组合态"
    );
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

// ---- Task 1：通知分级生命周期 / 首窗尺寸收敛 ----

/// 错误类通知不参与 6 秒超时：保留到被新通知替换或用户处理；成功提示照常短暂显示。
#[test]
fn error_notice_does_not_expire_automatically() {
    let store = MemStore::default();
    let mut a = app("正文", None, false, &store);
    a.ui_state
        .chrome
        .notify_error("保存失败：拒绝访问。 (os error 5)；可 Ctrl+S 重试");
    let t0 = Instant::now();
    a.tick(t0);
    a.tick(t0 + Duration::from_secs(60));
    assert!(
        a.ui_state.chrome.notice.contains("保存失败"),
        "错误通知不得自动消退: {:?}",
        a.ui_state.chrome.notice
    );
    assert_eq!(
        a.ui_state.chrome.current_notice_kind(),
        chrome::NoticeKind::Error
    );

    // 新通知替换旧通知后，生命周期回到「短暂显示」。
    a.ui_state
        .chrome
        .notify("已保存", chrome::NoticeKind::Success);
    a.tick(t0 + Duration::from_secs(61));
    a.tick(t0 + Duration::from_secs(75));
    assert!(
        a.ui_state.chrome.notice.is_empty(),
        "成功提示应短暂显示后消退: {:?}",
        a.ui_state.chrome.notice
    );
}

/// 直接替换 `notice` 文本时类别失效为 Info，避免旧错误通知的常驻语义污染新提示。
#[test]
fn direct_notice_assignment_resets_kind() {
    let store = MemStore::default();
    let mut a = app("正文", None, false, &store);
    a.ui_state.chrome.notify_error("失败");
    a.ui_state.chrome.notice = "已打开：a.md".into();
    assert_eq!(
        a.ui_state.chrome.current_notice_kind(),
        chrome::NoticeKind::Info
    );
}

/// 自动保存失败按错误分级：超过 TTL 仍保留；下一次成功通知替换后恢复短时行为。
#[test]
fn autosave_failure_notice_is_error_and_survives_ttl() {
    use crate::document::doc_state::SaveKind;
    let store = MemStore {
        fail_save: true,
        ..MemStore::default()
    };
    let mut a = dirty_app("ab", Some("a.md"), &store);
    a.save(SaveKind::Auto);
    assert!(
        a.ui_state.chrome.notice.contains("保存失败"),
        "自动保存失败必须可见：{}",
        a.ui_state.chrome.notice
    );
    assert_eq!(
        a.ui_state.chrome.current_notice_kind(),
        chrome::NoticeKind::Error
    );
    let t0 = Instant::now();
    a.tick(t0);
    a.tick(t0 + Duration::from_secs(60));
    assert!(
        a.ui_state.chrome.notice.contains("保存失败"),
        "自动保存失败不得在 TTL 后自动消失: {:?}",
        a.ui_state.chrome.notice
    );

    // 下一次成功通知替换错误提示，并回到「短暂显示」。
    a.ui_state
        .chrome
        .notify("已保存", chrome::NoticeKind::Success);
    a.tick(t0 + Duration::from_secs(61));
    a.tick(t0 + Duration::from_secs(75));
    assert!(
        a.ui_state.chrome.notice.is_empty(),
        "成功提示应短暂显示后消退: {:?}",
        a.ui_state.chrome.notice
    );
}

/// 版本历史写入失败按错误分级：超过 TTL 仍保留。
#[test]
fn snapshot_failure_notice_is_error_and_survives_ttl() {
    use crate::document::doc_state::SaveKind;
    let store = MemStore::default();
    let snap = FakeSnapshot::default();
    snap.fail_save.set(true);
    let mut a = dirty_app("ab", Some("a.md"), &store);
    a.ports.snapshot = Box::new(snap.clone());
    a.save(SaveKind::Auto);
    assert!(
        a.ui_state.chrome.notice.contains("版本历史写入失败"),
        "快照失败必须可见：{}",
        a.ui_state.chrome.notice
    );
    assert_eq!(
        a.ui_state.chrome.current_notice_kind(),
        chrome::NoticeKind::Error
    );
    let t0 = Instant::now();
    a.tick(t0);
    a.tick(t0 + Duration::from_secs(60));
    assert!(
        a.ui_state.chrome.notice.contains("版本历史写入失败"),
        "快照失败不得在 TTL 后自动消失: {:?}",
        a.ui_state.chrome.notice
    );
}

/// HTML 导出失败按错误分级：超过 TTL 仍保留。
#[test]
fn export_failure_notice_is_error_and_survives_ttl() {
    let store = MemStore::default();
    let mut a = app("正文", Some("a.md"), false, &store);
    a.ports.dialogs = Box::new(ExportPathDialog);
    a.export_now();
    assert!(
        a.ui_state.chrome.notice.contains("导出失败"),
        "导出失败必须可见：{}",
        a.ui_state.chrome.notice
    );
    assert_eq!(
        a.ui_state.chrome.current_notice_kind(),
        chrome::NoticeKind::Error
    );
    let t0 = Instant::now();
    a.tick(t0);
    a.tick(t0 + Duration::from_secs(60));
    assert!(
        a.ui_state.chrome.notice.contains("导出失败"),
        "导出失败不得在 TTL 后自动消失: {:?}",
        a.ui_state.chrome.notice
    );
}

/// 打印与 PDF 导出失败按错误分级：超过 TTL 仍保留。
#[test]
fn print_and_pdf_failure_notices_are_errors_and_survive_ttl() {
    let mut a = app_print("x\n", Box::new(FailingPrint));
    a.print_document();
    assert!(
        a.ui_state.chrome.notice.contains("打印失败"),
        "打印失败必须可见：{}",
        a.ui_state.chrome.notice
    );
    assert_eq!(
        a.ui_state.chrome.current_notice_kind(),
        chrome::NoticeKind::Error
    );
    let t0 = Instant::now();
    a.tick(t0);
    a.tick(t0 + Duration::from_secs(60));
    assert!(
        a.ui_state.chrome.notice.contains("打印失败"),
        "打印失败不得在 TTL 后自动消失: {:?}",
        a.ui_state.chrome.notice
    );

    let mut b = app_print("x\n", Box::new(FailingPrint));
    b.export_pdf();
    assert!(
        b.ui_state.chrome.notice.contains("导出 PDF 失败"),
        "PDF 导出失败必须可见：{}",
        b.ui_state.chrome.notice
    );
    assert_eq!(
        b.ui_state.chrome.current_notice_kind(),
        chrome::NoticeKind::Error
    );
    b.tick(t0);
    b.tick(t0 + Duration::from_secs(60));
    assert!(
        b.ui_state.chrome.notice.contains("导出 PDF 失败"),
        "PDF 导出失败不得在 TTL 后自动消失: {:?}",
        b.ui_state.chrome.notice
    );
}

/// 无打印机上下文属明确失败：错误分级、超过 TTL 仍保留，且文案含下一步。
#[test]
fn no_printer_context_notice_is_error_with_next_step_and_survives_ttl() {
    let mut a = app_print("x\n", Box::new(NoPrinterContextPrint));
    a.print_document();
    let notice = &a.ui_state.chrome.notice;
    assert!(
        notice.contains("无法获取打印机上下文"),
        "须保留失败原因：{notice}"
    );
    assert!(
        notice.contains("检查打印机连接") && notice.contains("重试"),
        "须提示检查打印机并重试：{notice}"
    );
    assert!(
        notice.contains("导出 PDF"),
        "须给出改用导出 PDF 的出口：{notice}"
    );
    assert_eq!(
        a.ui_state.chrome.current_notice_kind(),
        chrome::NoticeKind::Error,
        "无打印机上下文必须按错误分级：{notice}"
    );
    let t0 = Instant::now();
    a.tick(t0);
    a.tick(t0 + Duration::from_secs(60));
    assert!(
        a.ui_state.chrome.notice.contains("无法获取打印机上下文"),
        "错误通知不得在 TTL 后自动消失: {:?}",
        a.ui_state.chrome.notice
    );
}

/// 150% 小屏（工作区 1707×912 物理）：默认 900×700 逻辑必须收敛高度并居中不越界。
#[test]
fn initial_window_fits_small_150pct_screen() {
    let screens = [(
        ScreenRect {
            x: 0,
            y: 0,
            w: 1707,
            h: 912,
        },
        1.5_f32,
    )];
    let g = fit_initial_window(900, 700, &screens, None);
    assert_eq!(g.width, 900, "宽度仍放得下");
    assert!(g.height < 700, "高度必须收敛到工作区: {}", g.height);
    let phys_w = (g.width as f32 * 1.5).round() as i32;
    let phys_h = (g.height as f32 * 1.5).round() as i32;
    assert!(g.x >= 0 && g.x + phys_w <= 1707, "横向不得越界: {:?}", g);
    assert!(g.y >= 0 && g.y + phys_h <= 912, "纵向不得越界: {:?}", g);
}

/// 150% 小屏 + 任务栏：工作区 1707×864（任务栏占 48px），首窗必须整体落在
/// 工作区内并留边距，而不是按整屏 912 高收敛后压住任务栏。
#[test]
fn initial_window_fits_work_area_above_taskbar_on_150pct_screen() {
    let screens = [(
        ScreenRect {
            x: 0,
            y: 0,
            w: 1707,
            h: 912,
        },
        1.5_f32,
    )];
    let work_area = (
        ScreenRect {
            x: 0,
            y: 0,
            w: 1707,
            h: 864,
        },
        1.5_f32,
    );
    let g = fit_initial_window(900, 700, &screens, Some(work_area));
    assert_eq!(g.width, 900, "宽度仍放得下");
    assert!(g.height < 700, "高度必须收敛到工作区: {}", g.height);
    assert!(
        g.height < 554,
        "任务栏必须参与收敛（整屏可用高度为 554）: {}",
        g.height
    );
    let phys_w = (g.width as f32 * 1.5).round() as i32;
    let phys_h = (g.height as f32 * 1.5).round() as i32;
    // 外框 = 客户区 + 非客户区；上下左右各留 8px。
    assert!(
        g.x >= 8 && g.x + phys_w + 24 <= 1707 - 8,
        "横向不得贴边: {:?}",
        g
    );
    assert!(
        g.y >= 8 && g.y + phys_h + 64 <= 864 - 8,
        "纵向不得压住任务栏: {:?}",
        g
    );
}

/// 大屏（1920×1080 @100%）：默认尺寸保持，且在屏幕内居中。
#[test]
fn initial_window_keeps_default_on_large_screen() {
    let screens = [(
        ScreenRect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        1.0_f32,
    )];
    let g = fit_initial_window(900, 700, &screens, None);
    assert_eq!((g.width, g.height), (900, 700));
    assert!(g.x > 0 && g.x + 900 <= 1920);
    assert!(g.y > 0 && g.y + 700 <= 1080);
}

/// 无显示器信息：原样返回默认尺寸，不 panic。
#[test]
fn initial_window_without_screens_keeps_default() {
    let g = fit_initial_window(900, 700, &[], None);
    assert_eq!((g.width, g.height, g.x, g.y), (900, 700, 0, 0));
}

/// 最小尺寸必须小于默认 900×700（小屏不被强行放大），且在 150% DPI 下
/// 仍能完整显示菜单栏 + 4 行通知浮层 + 状态栏，而不是只剩首行。
#[test]
fn min_window_size_keeps_four_notice_lines_and_status_readable() {
    let (min_w, min_h) = MIN_WINDOW_SIZE;
    assert!(
        min_w > 0.0 && min_h > 0.0 && min_w <= 900.0 && min_h <= 700.0,
        "最小尺寸须为正且不大于默认 900×700: {min_w}×{min_h}"
    );
    let scale = 1.5_f32;
    let need_h = (chrome::MENU_H
        + chrome::STATUS_H
        + MIN_NOTICE_LINES * crate::tokens::typography::UI_LINE_HEIGHT
        + 2.0 * crate::tokens::space::S)
        * scale;
    assert!(
        min_h * scale >= need_h,
        "150% 下最小高度 {min_h} 逻辑须容纳菜单栏+状态栏+{MIN_NOTICE_LINES} 行通知（需 {need_h} 物理）"
    );
}

/// 极小工作区：首窗收敛也不得低于最小尺寸（最小尺寸是硬约束）。
#[test]
fn initial_window_never_falls_below_min_size_on_tiny_work_area() {
    let screens = [(
        ScreenRect {
            x: 0,
            y: 0,
            w: 200,
            h: 200,
        },
        1.0_f32,
    )];
    let g = fit_initial_window(900, 700, &screens, None);
    assert!(
        g.width >= MIN_WINDOW_SIZE.0 as u32,
        "宽度不得低于最小: {g:?}"
    );
    assert!(
        g.height >= MIN_WINDOW_SIZE.1 as u32,
        "高度不得低于最小: {g:?}"
    );
}

// ===== 键盘闭环与焦点归还（Task 4） =====

#[test]
fn keyboard_ctrl_o_dispatches_open_dialog_and_replaces_document() {
    let store = MemStore::default();
    disk(&store, "b.md", b"# new");
    let (mut a, calls) = app_dialogs("old", Some("a.md"), false, &store, Some("b.md"), None);
    ctrl_key(&mut a, Key::O);
    assert_eq!(
        calls.borrow().as_slice(),
        &["open".to_string()],
        "Ctrl+O 必须分发到系统打开对话框"
    );
    assert_eq!(doc_text(&a), "# new");
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));
}

#[test]
fn keyboard_search_esc_restores_caret_selection_and_scroll() {
    let store = MemStore::default();
    let text: String = (0..80).map(|i| format!("第 {i} 行 alpha\n")).collect();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_all();
    assert!(a.editor.set_caret(3) && a.editor.set_caret_extend(6));
    a.window_state.goal_y = 40.0;
    a.window_state.scroll_y = 40.0;
    let caret = a.editor.caret();
    let selection = a.editor.selection();
    let scroll = a.window_state.scroll_y;
    let before = doc_text(&a);

    ctrl_key(&mut a, Key::F);
    assert!(a.ui_state.search.open);
    a.route_key_with_focus(Key::Text("alpha".into()));
    assert_eq!(a.ui_state.search.query, "alpha");
    assert_ne!(
        a.window_state.scroll_y, scroll,
        "查找跳转改变了滚动，还原断言才有意义"
    );

    a.route_key_with_focus(Key::Escape);
    assert!(!a.ui_state.search.open, "Esc 必须一次关闭查找栏");
    assert_eq!(a.editor.caret(), caret);
    assert_eq!(a.editor.selection(), selection);
    assert_eq!(a.window_state.scroll_y, scroll);
    assert_eq!(doc_text(&a), before, "Esc 不得误删字符");
}

/// 滚轮走 `handle_window_event` 生产路径后必须刷新焦点快照，
/// 随后查找→Esc 归还到用户最后滚到的位置。
#[test]
fn mouse_wheel_syncs_focus_snapshot_before_search_esc_restore() {
    let store = MemStore::default();
    let text: String = (0..200).map(|i| format!("第 {i} 行 alpha\n")).collect();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_all();
    a.handle_window_event(WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -120.0)),
        phase: TouchPhase::Moved,
    });
    let wheeled = a.window_state.scroll_y;
    assert!(wheeled > 0.0, "滚轮必须实际滚动正文: {wheeled}");
    assert_eq!(
        a.ui_state.focus_return.map(|f| f.scroll),
        Some(wheeled),
        "滚轮后快照必须同步到当前滚动位置"
    );

    ctrl_key(&mut a, Key::F);
    a.route_key_with_focus(Key::Text("alpha".into()));
    a.route_key_with_focus(Key::Escape);
    assert!(!a.ui_state.search.open);
    assert_eq!(
        a.window_state.scroll_y, wheeled,
        "Esc 关闭查找后必须回到滚轮后的位置"
    );
}

#[test]
fn keyboard_replace_tab_stays_in_bar_and_esc_restores_focus() {
    let store = MemStore::default();
    let text: String = (0..80).map(|i| format!("第 {i} 行 alpha\n")).collect();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_all();
    assert!(a.editor.set_caret(3) && a.editor.set_caret_extend(6));
    a.window_state.goal_y = 40.0;
    a.window_state.scroll_y = 40.0;
    let caret = a.editor.caret();
    let selection = a.editor.selection();
    let scroll = a.window_state.scroll_y;
    let before = doc_text(&a);

    ctrl_key(&mut a, Key::H);
    assert!(a.ui_state.search.open && a.ui_state.search.replace_open);
    assert_eq!(a.ui_state.search.focus, search::Field::Query);
    a.route_key_with_focus(Key::Text("alpha".into()));
    a.route_key_with_focus(Key::Tab);
    assert_eq!(
        a.ui_state.search.focus,
        search::Field::Replace,
        "Tab 只在查询/替换框之间切换"
    );
    assert_eq!(doc_text(&a), before, "Tab 不得逃出查找栏去缩进正文");
    a.route_key_with_focus(Key::Text("beta".into()));
    assert_eq!(a.ui_state.search.replace, "beta");
    assert_eq!(doc_text(&a), before, "未确认替换前正文不变");

    a.route_key_with_focus(Key::Escape);
    assert!(!a.ui_state.search.open);
    assert_eq!(a.editor.caret(), caret);
    assert_eq!(a.editor.selection(), selection);
    assert_eq!(a.window_state.scroll_y, scroll);
}

#[test]
fn keyboard_quick_open_esc_restores_focus_without_editing() {
    let store = MemStore::default();
    let text: String = (0..80).map(|i| format!("第 {i} 行\n")).collect();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_all();
    a.editor.set_caret(3);
    a.window_state.goal_y = 24.0;
    a.window_state.scroll_y = 24.0;
    let caret = a.editor.caret();
    let scroll = a.window_state.scroll_y;
    let before = doc_text(&a);

    ctrl_key(&mut a, Key::P);
    assert!(a.ui_state.quick.open);
    a.route_key_with_focus(Key::Text("zzz".into()));
    a.route_key_with_focus(Key::Tab);
    assert!(a.ui_state.quick.open, "Tab 不得关闭或逃出快速打开");
    assert_eq!(doc_text(&a), before, "快速打开过滤串不得写入正文");

    a.route_key_with_focus(Key::Escape);
    assert!(!a.ui_state.quick.open);
    assert_eq!(a.editor.caret(), caret);
    assert_eq!(a.window_state.scroll_y, scroll);
    assert_eq!(doc_text(&a), before);
}

#[test]
fn keyboard_settings_tab_cycles_inside_and_esc_restores_focus() {
    let store = MemStore::default();
    let text: String = (0..80).map(|i| format!("第 {i} 行\n")).collect();
    let mut a = app(&text, None, false, &store);
    a.ensure_layouts_all();
    a.editor.set_caret(3);
    a.window_state.goal_y = 24.0;
    a.window_state.scroll_y = 24.0;
    let caret = a.editor.caret();
    let scroll = a.window_state.scroll_y;
    let before = doc_text(&a);

    ctrl_key(&mut a, Key::Comma);
    assert!(a.ui_state.settings.is_some(), "Ctrl+, 打开设置");
    for _ in 0..8 {
        a.route_key_with_focus(Key::Tab);
        assert!(a.ui_state.settings.is_some(), "Tab 不得逃出设置模态");
    }
    assert_eq!(doc_text(&a), before, "设置内 Tab 不得触碰正文");

    a.route_key_with_focus(Key::Escape);
    assert!(a.ui_state.settings.is_none(), "Esc 取消设置");
    assert_eq!(a.editor.caret(), caret);
    assert_eq!(a.window_state.scroll_y, scroll);
    assert_eq!(doc_text(&a), before);
}

#[test]
fn keyboard_modal_tab_cycles_inside_and_esc_cancels() {
    let store = MemStore::default();
    disk(&store, "a.md", b"old");
    let mut a = app("old", Some("a.md"), false, &store);
    a.editor.apply(Intent::InsertText("改".into())).unwrap();
    a.confirm_close();
    assert!(a.ui_state.modal.is_some());
    let n = a.ui_state.modal.as_ref().unwrap().spec.buttons.len();
    for _ in 0..(n * 2 + 1) {
        a.route_key_with_focus(Key::Tab);
        let m = a.ui_state.modal.as_ref().expect("Tab 不得逃出模态");
        assert!(m.state.focused < n, "焦点下标越界");
    }
    assert!(!a.window_state.exit_requested, "Tab 不得触发默认动作");
    a.route_key_with_focus(Key::Escape);
    assert!(a.ui_state.modal.is_none(), "Esc 取消并关闭模态");
    assert!(!a.window_state.exit_requested, "取消不得退出窗口");
}

#[test]
fn search_focus_ring_is_visible_against_popup() {
    let store = MemStore::default();
    let mut a = app("alpha\n", None, false, &store);
    a.open_search();
    let (w, h) = a.win_size();
    let l = a.search_rect().unwrap();
    let theme = a.input_state.theme;
    let popup = crate::tokens::palette::surface_popup(&theme);
    let ring = crate::view::modal::focus_ring_color(&theme, [popup[0], popup[1], popup[2]]);
    let mut f = Frame::new(w, h, theme.bg);
    a.paint_search(&mut f);

    let x0 = l.text.x.round() as u32;
    let mut found = false;
    for x in x0.saturating_sub(1)..=x0 + 2 {
        for y in (l.text.y + 2.0) as u32..(l.text.y + l.text.h - 2.0) as u32 {
            let px = f.get(x, y);
            if [px[0], px[1], px[2]] == ring {
                found = true;
            }
        }
    }
    assert!(found, "焦点字段必须绘制可见焦点环");
    let c = ring_contrast(ring, [popup[0], popup[1], popup[2]]);
    assert!(c >= 3.0, "焦点环与 popup 底对比不足: {c:.2}");
}

// ===== 帮助菜单 / 关于 / 导出诊断 / 检查更新（P0-1 / P0-2） =====

/// 导出诊断文本专用对话框：只有 save_text 返回目标，其余一律取消。
struct TextSaveDialog {
    target: Option<PathBuf>,
    calls: Rc<RefCell<Vec<String>>>,
}

impl FileDialog for TextSaveDialog {
    fn open_markdown(&mut self) -> Option<PathBuf> {
        None
    }
    fn save_markdown(&mut self, _suggested: &str) -> Option<PathBuf> {
        None
    }
    fn save_text(&mut self, suggested: &str, ext: &str) -> Option<PathBuf> {
        self.calls
            .borrow_mut()
            .push(format!("text:{ext}:{suggested}"));
        self.target.clone()
    }
}

#[test]
fn about_modal_shows_identity_and_primary_opens_official_release_page() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let store = MemStore::default();
    let mut a = app_with(
        "正文",
        None,
        false,
        &store,
        Box::new(FakeUrl {
            calls: calls.clone(),
            result: Ok(()),
        }),
    );
    a.perform_menu(MenuAction::About);
    let modal = a.ui_state.modal.as_ref().expect("应弹出关于弹窗");
    assert!(matches!(modal.purpose, ModalPurpose::About));
    assert!(modal.spec.title.contains("关于 Veil"));
    for needle in [
        crate::diag::APP_VERSION,
        crate::diag::BUILD_COMMIT,
        "MIT",
        "https://github.com/31i45/veil/releases",
        "Mermaid",
        "QuickJS",
        "linkedom",
        "THIRD-PARTY-LICENSES.md",
    ] {
        assert!(
            modal.spec.message.contains(needle),
            "关于正文缺少 `{needle}`: {}",
            modal.spec.message
        );
    }
    assert_eq!(modal.spec.buttons.len(), 2);
    assert_eq!(modal.spec.buttons[0].id, 1);
    assert_eq!(modal.spec.buttons[1].id, 2);

    // 弹窗存在时输入不得泄露到编辑器。
    let before = doc_text(&a);
    a.route_key(Key::Text("x".into()));
    assert_eq!(doc_text(&a), before, "关于弹窗必须吞掉正文输入");

    a.resolve_modal(1);
    assert!(a.ui_state.modal.is_none(), "主按钮关闭弹窗");
    assert_eq!(
        calls.borrow().as_slice(),
        &[crate::app::RELEASES_URL.to_string()],
        "主按钮只调用一次 URL 端口"
    );
}

#[test]
fn about_modal_close_button_does_not_open_url_and_failure_notifies() {
    let store = MemStore::default();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = app_with(
        "正文",
        None,
        false,
        &store,
        Box::new(FakeUrl {
            calls: calls.clone(),
            result: Ok(()),
        }),
    );
    a.perform_menu(MenuAction::About);
    a.resolve_modal(2);
    assert!(a.ui_state.modal.is_none());
    assert!(calls.borrow().is_empty(), "关闭按钮不得打开 URL");

    // 打开失败：写通知，不崩溃。
    let mut b = app_with(
        "正文",
        None,
        false,
        &store,
        Box::new(FakeUrl {
            calls: Rc::new(RefCell::new(Vec::new())),
            result: Err("未找到默认浏览器".into()),
        }),
    );
    b.perform_menu(MenuAction::About);
    b.resolve_modal(1);
    assert!(
        b.ui_state.chrome.notice.contains("无法打开链接"),
        "失败必须通知: {}",
        b.ui_state.chrome.notice
    );
    assert!(b.ui_state.modal.is_none());
}

#[test]
fn export_diagnostics_writes_whitelisted_fields_without_paths() {
    let store = MemStore::default();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let d = TextSaveDialog {
        target: Some(PathBuf::from("diag.txt")),
        calls: calls.clone(),
    };
    let mut a = app_confirm(
        "hello",
        Some(r"C:\Users\alice\secret\a.md"),
        &store,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(d),
    );
    a.perform_menu(MenuAction::ExportDiagnostics);

    assert_eq!(
        calls.borrow().as_slice(),
        &[format!(
            "text:txt:veil-diagnostics-{}.txt",
            crate::diag::APP_VERSION
        )]
    );
    let files = store.files.borrow();
    let raw = files.get(Path::new("diag.txt")).expect("应写出诊断文件");
    let text = String::from_utf8(raw.clone()).expect("诊断文本为 UTF-8");
    assert!(text.contains("=== Veil 诊断信息 ==="));
    for needle in [
        "版本",
        "构建提交",
        "配置存在",
        "文档已关联",
        "未保存修改",
        "文档字符数",
        "主题",
        "允许远程图片",
        "界面缩放",
        "崩溃报告数",
    ] {
        assert!(text.contains(needle), "缺少白名单字段 `{needle}`: {text}");
    }
    assert!(!text.contains("C:\\"), "诊断文本不得含完整路径: {text}");
    assert!(!text.contains("alice"), "诊断文本不得含路径片段: {text}");
    assert!(
        a.ui_state.chrome.notice.contains("已导出诊断信息"),
        "成功必须写通知: {}",
        a.ui_state.chrome.notice
    );
}

#[test]
fn export_diagnostics_cancel_and_failure_do_not_leave_files() {
    // 取消：无副作用。
    let store = MemStore::default();
    let d = TextSaveDialog {
        target: None,
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let mut a = app_confirm(
        "body",
        None,
        &store,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(d),
    );
    a.perform_menu(MenuAction::ExportDiagnostics);
    assert!(store.files.borrow().is_empty(), "取消不得落盘");

    // 写失败：只通知。
    let failing = MemStore {
        fail_save: true,
        ..Default::default()
    };
    let d = TextSaveDialog {
        target: Some(PathBuf::from("diag.txt")),
        calls: Rc::new(RefCell::new(Vec::new())),
    };
    let mut b = app_confirm(
        "body",
        None,
        &failing,
        FakeRecovery::default(),
        &FakeSession::default(),
        Box::new(d),
    );
    b.perform_menu(MenuAction::ExportDiagnostics);
    assert!(failing.files.borrow().is_empty());
    assert!(
        b.ui_state.chrome.notice.contains("导出诊断信息失败"),
        "失败必须通知: {}",
        b.ui_state.chrome.notice
    );
}

#[test]
fn check_update_calls_url_port_once_and_failure_notifies() {
    let store = MemStore::default();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut a = app_with(
        "body",
        None,
        false,
        &store,
        Box::new(FakeUrl {
            calls: calls.clone(),
            result: Ok(()),
        }),
    );
    a.perform_menu(MenuAction::CheckUpdate);
    assert_eq!(
        calls.borrow().as_slice(),
        &[crate::app::RELEASES_URL.to_string()],
        "检查更新只应调用一次 URL 端口"
    );

    let mut b = app_with(
        "body",
        None,
        false,
        &store,
        Box::new(FakeUrl {
            calls: Rc::new(RefCell::new(Vec::new())),
            result: Err("ShellExecuteW 返回 2".into()),
        }),
    );
    b.perform_menu(MenuAction::CheckUpdate);
    assert!(
        b.ui_state.chrome.notice.contains("无法打开更新页面"),
        "打开失败必须通知: {}",
        b.ui_state.chrome.notice
    );
}
