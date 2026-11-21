//! 文件 IO 与持久化状态测试：导出 / 保存 / 打开 / 自动保存 / 冲突备份 / 保存态信任提示。

use super::*;

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

/// 版本历史写入失败：状态栏可见提示，正文/磁盘照常保存（不因历史失败丢数据）。
#[test]
fn snapshot_failure_is_visible_and_does_not_lose_data() {
    use crate::document::doc_state::SaveKind;
    let store = MemStore::default();
    let snap = FakeSnapshot::default();
    snap.fail_save.set(true);
    let mut a = dirty_app("ab", Some("a.md"), &store);
    a.ports.snapshot = Box::new(snap.clone());
    a.save(SaveKind::Auto);
    assert!(!a.editor.is_dirty(), "正文仍走正常保存流程");
    assert_eq!(
        store.files.borrow()[Path::new("a.md")],
        "改ab".as_bytes(),
        "磁盘内容已写入，历史失败不影响正文"
    );
    assert!(
        a.ui_state.chrome.notice.contains("版本历史写入失败"),
        "失败必须可见：{}",
        a.ui_state.chrome.notice
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
