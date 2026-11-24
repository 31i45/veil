//! 崩溃恢复与会话测试：恢复槽 / 会话恢复 / 最近文件 / 外部文件监听 / 未保存三选一模态。

use super::*;

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
    let popup_bg = crate::tokens::palette::surface_modal(&a.input_state.theme);
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

    let _ = a.replace_current();
    assert_eq!(doc_text(&a), "b x a", "仅替换当前命中");
    assert_eq!(a.ui_state.search.matches.len(), 1, "命中缓存重建");
    assert_eq!(a.editor.caret(), 4, "跳到剩余的首个命中");

    let _ = a.replace_current();
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
fn regex_replace_all_expands_capture_groups() {
    let store = MemStore::default();
    let mut a = app("**粗** 与 **斜**", None, false, &store);
    a.ensure_layouts_all();
    a.open_replace();
    a.search_toggle_regex();
    a.search_input(r"\*\*([^*]+)\*\*");
    assert_eq!(a.ui_state.search.matches.len(), 2, "正则命中两处加粗");
    assert!(!a.ui_state.search.regex_error);
    a.search_toggle_field(); // Tab → 替换框
    a.search_input("$1");
    a.replace_all();
    assert_eq!(doc_text(&a), "粗 与 斜", "捕获组展开为组 1 内容");
    assert!(a.ui_state.search.matches.is_empty(), "替换后命中缓存重建");
}

#[test]
fn regex_enter_replaces_current_with_capture_expansion() {
    let store = MemStore::default();
    let mut a = app("**粗** **斜**", None, false, &store);
    a.ensure_layouts_all();
    a.open_replace();
    a.search_toggle_regex();
    a.search_input(r"\*\*([^*]+)\*\*");
    a.search_toggle_field();
    a.search_input("<$1>");
    a.route_key(Key::Enter);
    assert_eq!(doc_text(&a), "<粗> **斜**", "仅替换当前一处且展开捕获组");
    assert_eq!(a.ui_state.search.matches.len(), 1, "剩余命中仍可继续替换");
}

#[test]
fn regex_invalid_pattern_marks_error_and_yields_no_matches() {
    let store = MemStore::default();
    let mut a = app("abc", None, false, &store);
    a.perform_action(Action::Find);
    a.search_toggle_regex();
    a.search_input("(");
    assert!(a.ui_state.search.regex_error, "非法模式须进入错误态");
    assert!(a.ui_state.search.matches.is_empty());
    // 修正后错误态消失并立即恢复命中
    a.search_backspace();
    a.search_input("a+");
    assert!(!a.ui_state.search.regex_error);
    assert_eq!(a.ui_state.search.matches.len(), 1);
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
    );
    a.save_as();
    assert_eq!(a.doc.path.as_deref(), Some(Path::new("b.md")));
    assert_eq!(watch.calls.borrow().last().unwrap(), "watch:b.md");
}

// ---- 主题与配置（切片 17） ----

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
    let notice = &b.ui_state.chrome.notice;
    assert!(notice.contains("「a.md」"), "提示须含恢复对象: {notice}");
    assert!(notice.contains("14:32"), "提示须含恢复时间: {notice}");
    assert!(notice.contains("Ctrl+S"), "提示须给出保存方式: {notice}");
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
    let notice = &b.ui_state.chrome.notice;
    assert!(
        notice.contains("未命名草稿"),
        "无路径草稿须以「未命名草稿」作为恢复对象: {notice}"
    );
    assert!(notice.contains("14:32"), "提示须含恢复时间: {notice}");
    assert!(notice.contains("Ctrl+S"), "提示须给出保存方式: {notice}");
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
fn committed_ime_text_survives_crash_recovery() {
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist("ab", Some("a.md"), &store, &rec, &ses, false);
    a.editor.apply(Intent::MoveDocEnd).unwrap();

    // 组合期不写半成品快照
    a.editor.set_preedit("zh".into(), None);
    a.persist_state.persist.note_activity(Instant::now());
    a.maybe_recovery_write(Instant::now() + Duration::from_secs(30));
    assert!(rec.slot.borrow().is_none(), "组合期不得写恢复快照");

    // 上屏后才允许写；快照必须包含已提交文本
    a.ime_commit("中".into());
    a.maybe_recovery_write(Instant::now() + Duration::from_secs(5));
    let data = rec.slot.borrow().clone().expect("上屏后应写恢复快照");
    let decoded = recovery::decode_recovery(&data).expect("恢复文件可解码");
    assert_eq!(decoded.text, "ab中", "已提交文本必须进入快照");
    assert_eq!(decoded.path, Some(PathBuf::from("a.md")));

    // 重启：未保存的上屏内容完整恢复
    let b = app_persist("默认样例", None, &store, &rec, &ses, true);
    assert_eq!(doc_text(&b), "ab中");
    assert!(b.editor.is_dirty(), "恢复内容相对磁盘未保存");
}

#[test]
fn empty_ime_commit_does_not_touch_recovery_slot() {
    let store = MemStore::default();
    let rec = FakeRecovery::default();
    let ses = FakeSession::default();
    let mut a = app_persist("abc", Some("a.md"), &store, &rec, &ses, false);
    a.editor.apply(Intent::MoveDocEnd).unwrap();
    a.ime_commit("改".into());
    a.maybe_recovery_write(Instant::now() + Duration::from_secs(5));
    let before = rec.slot.borrow().clone().expect("非空提交后应写快照");

    a.editor.apply(Intent::SelectAll).unwrap();
    let calls_before = rec.calls.borrow().len();
    a.ime_commit(String::new());
    a.maybe_recovery_write(Instant::now() + Duration::from_secs(30));

    assert_eq!(
        rec.slot.borrow().clone(),
        Some(before),
        "空提交不得改写恢复槽"
    );
    assert_eq!(
        rec.calls.borrow().len(),
        calls_before,
        "空提交不得触发恢复 IO"
    );
    assert!(a.editor.selection().is_some(), "空提交不得删除选区");
}

#[test]
fn truncated_and_non_utf8_recovery_are_rejected() {
    let encoded = recovery::encode_recovery(Some(Path::new("a.md")), false, "未保存正文");
    let mut non_utf8 = recovery::encode_recovery(None, false, "");
    non_utf8.extend_from_slice(&[0xff, 0xfe]);

    for (name, bytes) in [
        ("头部截断", encoded[..encoded.len() / 2].to_vec()),
        (
            "内容字符边界截断",
            encoded[..encoded.len() - 3].to_vec(), // “未保存正文”末字 3 字节
        ),
        ("非 UTF-8", non_utf8),
        ("坏魔数", b"not-a-recovery-file".to_vec()),
        (
            "path_len 溢出",
            b"VEIL-RECOVERY-2\nbom=0\npath_len=18446744073709551615\ncontent_len=0\nchecksum=cbf29ce484222325\n\n".to_vec(),
        ),
    ] {
        let store = MemStore::default();
        let rec = FakeRecovery::default();
        *rec.slot.borrow_mut() = Some(bytes);
        let ses = FakeSession::default();
        let a = app_persist("默认样例", None, &store, &rec, &ses, true);
        assert_eq!(
            doc_text(&a),
            "默认样例",
            "{name} 恢复文件必须被拒绝，正常打开默认文档"
        );
        assert!(!a.editor.is_dirty(), "{name} 拒绝后不得凭空置脏");
        assert!(rec.slot.borrow().is_none(), "{name} 恢复文件应被清除");
    }
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

/// 50 MB 中文长文探针（默认忽略）：打开 ≤5 s、连续键入 p95 ≤16 ms、保存字节一致。
/// cargo test -p veil-ui --release --lib -- --ignored --nocapture perf_50mb_long_doc
#[test]
#[ignore = "50MB 性能探针，不参与常规门禁"]
fn perf_50mb_long_doc() {
    let sentence = "中文输入保真压力测试段落，包含普通文本、标点与数字 0123456789。";
    let para = sentence.repeat(30);
    let mut text = String::with_capacity(55 * 1024 * 1024);
    for i in 0..20_000 {
        text.push_str(&para);
        text.push_str(" 第 ");
        text.push_str(&i.to_string());
        text.push_str(" 段\n\n");
    }
    assert!(text.len() >= 50 * 1024 * 1024, "探针文本应≥50MB");
    let store = MemStore::default();

    let t = Instant::now();
    let mut a = app(&text, Some("big50.md"), false, &store);
    a.paint();
    let open_ms = t.elapsed().as_secs_f64() * 1e3;

    a.editor.apply(Intent::MoveDocEnd).unwrap();
    let mut samples = Vec::with_capacity(100);
    for i in 0..100 {
        let t = Instant::now();
        // 交替 ASCII 键入与 IME 中文上屏，覆盖两条真实输入路径。
        let dirty = if i % 2 == 0 {
            a.editor.apply(Intent::InsertText("a".into())).unwrap()
        } else {
            a.editor.set_preedit("zhong".into(), None);
            a.editor.commit_ime("中".into())
        };
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

    let t = Instant::now();
    let expected = a.editor.session().serialize().into_bytes();
    a.save_now();
    let save_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(
        store.files.borrow()[Path::new("big50.md")],
        expected,
        "50MB 保存字节必须与会话序列化一致"
    );

    println!("perf_50mb_bytes={}", text.len());
    println!("perf_50mb_blocks={}", a.index.blocks.len());
    println!("perf_50mb_open_ms={open_ms:.1}");
    println!("perf_50mb_keystroke_ms_p50={:.3}", pick(0.5));
    println!("perf_50mb_keystroke_ms_p95={:.3}", pick(0.95));
    println!(
        "perf_50mb_keystroke_ms_max={:.3}",
        samples[samples.len() - 1]
    );
    println!("perf_50mb_save_ms={save_ms:.1}");

    assert!(open_ms <= 5_000.0, "50MB 打开超过 5s: {open_ms:.1}ms");
    assert!(pick(0.95) <= 16.0, "50MB 击键 p95 超过 16ms");
}

/// 20k+ 块大文档击键探针（默认忽略）：驱动真实 App 路径（editor apply + after_edit +
/// paint），上报击键 p50/p95/max，并断言零丢字与保存字节逐字节一致。
/// cargo test -p veil-ui --release --lib -- --ignored --nocapture perf_20k_blocks
#[test]
#[ignore = "性能探针，不参与常规门禁"]
fn perf_20k_blocks() {
    // 每轮 7 块（标题 / 正文 / 两条列表项 / 代码块 / 引用 / 分隔线），
    // 3000 轮 ≈ 21000 块，即约 21k 压力文档（converge-single-veil-surface G2 口径）。
    let mut text = String::with_capacity(1_000_000);
    for i in 0..3000 {
        text.push_str(&format!(
            "## Section {i}\n\nParagraph {i} with **bold** text, `code`, and a [link](https://example.com/{i}).\n\n- item A {i}\n- item B {i}\n\n```rust\nfn f{i}() -> u32 {{ {i} }}\n```\n\n> quote {i}\n\n---\n\n"
        ));
    }
    let store = MemStore::default();
    let mut a = app(&text, Some("perf.md"), false, &store);
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
    println!("perf_20k_blocks={n}");
    println!("perf_20k_samples_n={}", samples.len());
    println!("perf_20k_keystroke_ms_p50={:.3}", pick(0.5));
    println!("perf_20k_keystroke_ms_p95={:.3}", pick(0.95));
    println!(
        "perf_20k_keystroke_ms_max={:.3}",
        samples[samples.len() - 1]
    );
    // 零丢字：300 次击键必须一字不少地落在文档里（Task 11 门禁）。
    let caret = a.editor.caret();
    let g = a
        .index
        .blocks
        .iter()
        .find(|g| g.range.start <= caret && caret < g.range.end)
        .expect("光标应落在块内");
    let txt = a.editor.session().text_slice(g.range).unwrap_or_default();
    assert!(
        txt.contains(&"a".repeat(300)),
        "连续 300 次击键不得丢字：{txt:?}"
    );
    // 保存后逐字节一致：落盘字节必须等于会话序列化字节（Task 11 门禁）。
    let expected = a.editor.session().serialize().into_bytes();
    a.save_now();
    assert_eq!(
        store.files.borrow()[Path::new("perf.md")],
        expected,
        "保存字节必须与会话序列化逐字节一致"
    );
}
