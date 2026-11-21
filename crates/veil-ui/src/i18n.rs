//! 界面文案 key 化（N7/B8c）：默认 zh-CN 词条表 + `t()` 取词。
//!
//! 设计取舍：
//! - `Key` 枚举与 zh-CN 表由同一份宏生成，穷举性由构造保证（不会出现「有 key 无词条」）；
//! - 词条按语言分表（`zh-CN` / `en-US`），`t()` 按 `LOCALES` 顺序回退；zh-CN 全量，
//!   en-US 可按需补翻译（只需填表，不改调用点）；
//! - 缺失 key 回退显示 key 的稳定 id（如 `menu.file.new`），debug 构建断言提示；
//!   测试构建跳过断言，保证「回退不 panic」可被单测覆盖。
//!
//! 显示等价是硬约束：所有 zh-CN 词条与 key 化前的字面量逐字一致，包括空格与全角标点。

/// 界面文案 key。新增文案：在 `keys!` 中加一行（变体, 稳定 id, zh-CN 文本）。
macro_rules! keys {
    ($($variant:ident => $id:literal, $zh:literal;)*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Key {
            $($variant,)*
        }

        impl Key {
            /// 全部 key（与枚举定义同源，供穷举测试使用）。
            pub const ALL: &'static [Key] = &[$(Key::$variant,)*];

            /// 稳定 id：缺失词条时的回退显示，也用于日志/测试定位。
            pub const fn id(self) -> &'static str {
                match self {
                    $(Key::$variant => $id,)*
                }
            }
        }

        /// zh-CN 默认词条表（顺序与枚举一致）。
        pub const ZH_CN: &[(Key, &'static str)] = &[
            $((Key::$variant, $zh),)*
        ];
    };
}

keys! {
    // ---- 通用 ----
    Empty => "common.empty", "";
    UntitledDraft => "common.untitled_draft", "未命名草稿";
    MenuCurrent => "common.current", "当前";

    // ---- 菜单 ----
    MenuFile => "menu.file", "文件";
    MenuEdit => "menu.edit", "编辑";
    MenuView => "menu.view", "视图";

    MenuFileNew => "menu.file.new", "新建";
    MenuFileNewWindow => "menu.file.new_window", "新建窗口";
    MenuFileOpen => "menu.file.open", "打开…";
    MenuFileQuickOpen => "menu.file.quick_open", "快速打开";
    MenuFileSave => "menu.file.save", "保存";
    MenuFileSaveAs => "menu.file.save_as", "另存为…";
    MenuFileExportHtml => "menu.file.export_html", "导出 HTML…";
    MenuFileExportPdf => "menu.file.export_pdf", "导出 PDF…";
    MenuFilePrint => "menu.file.print", "打印…";
    MenuFileVersionHistory => "menu.file.version_history", "版本历史";
    MenuFileQuit => "menu.file.quit", "关闭窗口";

    MenuEditUndo => "menu.edit.undo", "撤销";
    MenuEditRedo => "menu.edit.redo", "重做";
    MenuEditCut => "menu.edit.cut", "剪切";
    MenuEditCopy => "menu.edit.copy", "复制";
    MenuEditPaste => "menu.edit.paste", "粘贴";
    MenuEditSelectAll => "menu.edit.select_all", "全选";
    MenuEditBold => "menu.edit.bold", "加粗";
    MenuEditItalic => "menu.edit.italic", "斜体";
    MenuEditLink => "menu.edit.link", "链接";
    MenuEditFind => "menu.edit.find", "查找";
    MenuEditReplace => "menu.edit.replace", "替换";
    MenuEditInsertImage => "menu.edit.insert_image", "插入图片…";

    MenuViewOutline => "menu.view.outline", "大纲面板";
    MenuViewZoomIn => "menu.view.zoom_in", "放大";
    MenuViewZoomOut => "menu.view.zoom_out", "缩小";
    MenuViewZoomReset => "menu.view.zoom_reset", "还原缩放";
    MenuViewFullscreen => "menu.view.fullscreen", "全屏";
    MenuViewThemeSystem => "menu.view.theme_system", "跟随系统";
    MenuViewThemeLight => "menu.view.theme_light", "浅色";
    MenuViewThemeDark => "menu.view.theme_dark", "深色";
    MenuViewSourceMode => "menu.view.source_mode", "源码模式";
    MenuViewFocusMode => "menu.view.focus_mode", "专注模式";
    MenuViewAllowRemoteImages => "menu.view.allow_remote_images", "允许远程图片";
    MenuViewSettings => "menu.view.settings", "设置…";

    // ---- 快捷键提示（菜单右侧列） ----
    ShortcutNew => "shortcut.new", "Ctrl+N";
    ShortcutNewWindow => "shortcut.new_window", "Ctrl+Shift+N";
    ShortcutOpen => "shortcut.open", "Ctrl+O";
    ShortcutQuickOpen => "shortcut.quick_open", "Ctrl+P";
    ShortcutSave => "shortcut.save", "Ctrl+S";
    ShortcutSaveAs => "shortcut.save_as", "Ctrl+Shift+S";
    ShortcutExportHtml => "shortcut.export_html", "Ctrl+E";
    ShortcutExportPdf => "shortcut.export_pdf", "Ctrl+Shift+E";
    ShortcutPrint => "shortcut.print", "Ctrl+Shift+P";
    ShortcutVersionHistory => "shortcut.version_history", "Ctrl+Shift+H";
    ShortcutUndo => "shortcut.undo", "Ctrl+Z";
    ShortcutRedo => "shortcut.redo", "Ctrl+Shift+Z";
    ShortcutCut => "shortcut.cut", "Ctrl+X";
    ShortcutCopy => "shortcut.copy", "Ctrl+C";
    ShortcutCopyCode => "shortcut.copy_code", "Ctrl+Shift+C";
    ShortcutPaste => "shortcut.paste", "Ctrl+V";
    ShortcutSelectAll => "shortcut.select_all", "Ctrl+A";
    ShortcutBold => "shortcut.bold", "Ctrl+B";
    ShortcutItalic => "shortcut.italic", "Ctrl+I";
    ShortcutLink => "shortcut.link", "Ctrl+K";
    ShortcutFind => "shortcut.find", "Ctrl+F";
    ShortcutReplace => "shortcut.replace", "Ctrl+H";
    ShortcutInsertImage => "shortcut.insert_image", "Ctrl+Shift+I";
    ShortcutOutline => "shortcut.outline", "Ctrl+Shift+O";
    ShortcutZoomIn => "shortcut.zoom_in", "Ctrl+=";
    ShortcutZoomOut => "shortcut.zoom_out", "Ctrl+-";
    ShortcutZoomReset => "shortcut.zoom_reset", "Ctrl+0";
    ShortcutFullscreen => "shortcut.fullscreen", "F11";
    ShortcutSourceMode => "shortcut.source_mode", "Ctrl+/";
    ShortcutSettings => "shortcut.settings", "Ctrl+,";

    // ---- 菜单禁用原因（禁用项右侧 muted 说明，避免静默变灰） ----
    DisabledNoChanges => "disabled.no_changes", "没有需要保存的更改";
    DisabledEmptyDocument => "disabled.empty_document", "空文档没有可导出的内容";
    DisabledNothingToUndo => "disabled.nothing_to_undo", "没有可撤销的操作";
    DisabledNothingToRedo => "disabled.nothing_to_redo", "没有可重做的操作";

    // ---- 窗口标题 / 状态栏 ----
    WindowTitle => "title.window", "veil — {}";
    AppTitle => "title.app", "veil";
    TitleDirtyPrefix => "title.dirty_prefix", "* ";
    TitleExternalChanged => "title.external_changed", " (外部修改)";
    TitleExternalBackedUp => "title.external_backed_up", " (外部修改已备份)";
    TitleExternalMissing => "title.external_missing", " (文件缺失)";

    StatusExternalChanged => "status.external_changed", "（外部修改）";
    StatusExternalBackedUp => "status.external_backed_up", "（外部修改已备份）";
    StatusExternalMissing => "status.external_missing", "（文件缺失）";
    StatusSourceMode => "status.source_mode", "（源码模式）";
    StatusUnsaved => "status.unsaved", " · 未保存";
    StatusSaved => "status.saved", " · 已保存 {}";
    StatusStats => "status.stats", "{} 字 · 行 {}，列 {}";
    StatusWords => "status.words", "{} 字";

    // ---- 快速打开 ----
    QuickOpenPlaceholder => "quick_open.placeholder", "快速打开：输入文件名，回车打开";
    QuickOpenEmptyNoCandidates => "quick_open.empty.no_candidates", "暂无最近文件或同目录 Markdown";
    QuickOpenEmptyNoMatch => "quick_open.empty.no_match", "无匹配文件";
    QuickOpenRecent => "quick_open.recent", "最近";

    // ---- 查找替换 ----
    SearchPlaceholder => "search.placeholder", "查找";
    SearchReplacePlaceholder => "search.replace_placeholder", "替换为";
    SearchReplace => "search.replace", "替换";
    SearchReplaceAll => "search.replace_all", "全部";
    SearchCaseSensitive => "search.case_sensitive", "Aa";
    SearchRegex => "search.regex", ".*";
    SearchRegexError => "search.regex_error", "正则错误";
    SearchCount => "search.count", "{}/{}";

    // ---- 版本历史 ----
    HistoryTitle => "history.title", "版本历史";
    HistoryRestore => "history.action.restore", "恢复此版本";
    HistoryDelete => "history.action.delete", "删除快照";
    HistoryClear => "history.action.clear", "清空历史";
    HistoryClose => "history.action.close", "关闭";
    HistoryEmpty => "history.empty", "暂无版本历史";
    HistoryEmptyHint => "history.empty_hint", "保存前会自动留档旧版本";
    HistoryCurrentSuffix => "history.current_suffix", " · 当前";
    HistoryUndoHint => "history.undo_hint", "恢复后可 Ctrl+Z 撤销";
    HistoryErrorHeadline => "history.error.headline", "读取历史失败：{}";
    HistoryErrorHint => "history.error.hint", "可关闭面板后重试";

    // ---- 大纲 ----
    OutlineTitle => "outline.title", "大纲";
    OutlineEmpty => "outline.empty", "无标题";

    // ---- 右键菜单 ----
    CtxCut => "ctx.cut", "剪切";
    CtxCopy => "ctx.copy", "复制";
    CtxPaste => "ctx.paste", "粘贴";
    CtxSelectAll => "ctx.select_all", "全选";
    CtxOpenInBrowser => "ctx.open_in_browser", "在浏览器中打开";
    CtxCopyLink => "ctx.copy_link", "复制链接地址";
    CtxInsertImage => "ctx.insert_image", "插入图片…";
    CtxCopyImagePath => "ctx.copy_image_path", "复制图片路径";
    CtxCopyCode => "ctx.copy_code", "复制代码块";
    CtxReloadImage => "ctx.reload_image", "重新加载图片";
    CtxImageMissing => "ctx.image_missing", "图片文件缺失（当前显示替代文本）";
    CtxMermaidPreview => "ctx.mermaid.preview", "在 mermaid.live 打开（复制源码）";
    CtxMermaidPreviewImage => "ctx.mermaid.preview_image", "预览当前源码图";
    CtxMermaidInsertImage => "ctx.mermaid.insert_image", "插入本地图片快照（保留源码）";
    CtxMermaidCopySource => "ctx.mermaid.copy_source", "复制源码";
    CtxMermaidInsertFlowchart => "ctx.mermaid.insert_flowchart", "插入流程图模板";
    CtxMermaidInsertSequence => "ctx.mermaid.insert_sequence", "插入时序图模板";
    CtxMermaidInsertClass => "ctx.mermaid.insert_class", "插入类图模板";
    CtxFold => "ctx.fold", "折叠";
    CtxUnfold => "ctx.unfold", "展开";
    CtxTableInsertRowAbove => "ctx.table.insert_row_above", "上方插入行";
    CtxTableInsertRowBelow => "ctx.table.insert_row_below", "下方插入行";
    CtxTableDeleteRow => "ctx.table.delete_row", "删除行";
    CtxTableInsertColumnLeft => "ctx.table.insert_column_left", "左侧插入列";
    CtxTableInsertColumnRight => "ctx.table.insert_column_right", "右侧插入列";
    CtxTableDeleteColumn => "ctx.table.delete_column", "删除列";
    CtxTableAlignLeft => "ctx.table.align_left", "左对齐";
    CtxTableAlignCenter => "ctx.table.align_center", "居中";
    CtxTableAlignRight => "ctx.table.align_right", "右对齐";

    // ---- Callout（内置五类，显示值保持既有英文标签） ----
    CalloutNote => "callout.note", "Note";
    CalloutTip => "callout.tip", "Tip";
    CalloutImportant => "callout.important", "Important";
    CalloutWarning => "callout.warning", "Warning";
    CalloutCaution => "callout.caution", "Caution";

    // ---- Mermaid 卡片 ----
    MermaidFlowchart => "mermaid.flowchart", "流程图";
    MermaidSequence => "mermaid.sequence", "时序图";
    MermaidClass => "mermaid.class", "类图";
    MermaidState => "mermaid.state", "状态图";
    MermaidEr => "mermaid.er", "ER图";
    MermaidGantt => "mermaid.gantt", "甘特图";
    MermaidPie => "mermaid.pie", "饼图";
    MermaidUnknown => "mermaid.unknown", "mermaid";
    MermaidBadge => "mermaid.badge", "◈ {}";
    MermaidEmptyHint => "mermaid.empty_hint", "空 Mermaid 块 — 点击插入模板";

    // ---- 正文降级提示 ----
    BlockOversizeNotice => "doc.block_oversize", "该块约 {} MB，超出单块渲染上限，后续内容未渲染；源码完整保留";
    ImageAltFallback => "doc.image_alt_fallback", "[图片]";

    // ---- 通用按钮 ----
    BtnSave => "button.save", "保存";
    BtnDontSave => "button.dont_save", "不保存";
    BtnCancel => "button.cancel", "取消";
    BtnRetry => "button.retry", "重试";
    BtnOpenOther => "button.open_other", "打开其他文件";
    BtnClose => "button.close", "关闭";
    BtnSaveAs => "button.save_as", "另存为";
    BtnChangeLocation => "button.change_location", "换一个位置";
    BtnReselect => "button.reselect", "重新选择";
    BtnDelete => "button.delete", "删除";
    BtnClear => "button.clear", "清空";
    BtnConfirm => "button.confirm", "确定";
    BtnReload => "button.reload", "重新加载";
    BtnKeepCurrent => "button.keep_current", "保留当前";
    BtnRestoreDefaults => "button.restore_defaults", "恢复默认";

    // ---- 保存 / 打开 / 导出 失败与提示 ----
    NoticeWatchUnavailable => "notice.watch_unavailable",
        "文件监听不可用: {}；重新打开该文件即可再次尝试";
    NoticeConflictNameExhausted => "notice.conflict_name_exhausted", "冲突副本名已耗尽";
    NoticeExternalBackupFailed => "notice.external_backup_failed", "外部版本备份失败：取消保存 {}";
    NoticeSaved => "notice.saved", "已保存：{}";
    NoticeSaveFailed => "notice.save_failed", "保存失败：{}；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为";
    ModalSaveFailedTitle => "modal.save_failed.title", "保存失败";
    ModalSaveFailedBody => "modal.save_failed.body",
        "{}\n\n原因：{}\n\n下一步：可重试本次保存，或用「另存为」写入其他位置；当前修改仍保留在编辑器中。";
    ModalExternalBackupFailBody => "modal.external_backup_fail.body",
        "{}\n\n原因：外部版本备份失败（{}）。\n\n下一步：为避免覆盖外部修改，本次保存已取消；可重试或「另存为」写入其他位置。";
    NoticeRecoveryWriteFailed => "notice.recovery_write_failed",
        "崩溃恢复写入失败：{}；可检查磁盘空间或权限后重试";
    NoticeFileRestored => "notice.file_restored", "文件已恢复：{}";
    NoticeExternalChanged => "notice.external_changed", "检测到外部修改：保存时将先备份磁盘版本";
    NoticeExternalDeleted => "notice.external_deleted", "文件已被外部删除；保存将重建";
    NoticeExternalUnreadable => "notice.external_unreadable", "文件暂不可读（可能被外部程序占用）";
    ModalExternalChangedTitle => "modal.external_changed.title", "「{}」已被外部修改";
    ModalExternalChangedBody => "modal.external_changed.body", "磁盘文件已被其他程序修改。重新加载将丢弃未保存的修改；保留当前可继续编辑，保存时会先备份磁盘版本。";
    NoticeExternalReloaded => "notice.external_reloaded", "已重新加载外部修改：{}";
    NoticeExternalKept => "notice.external_kept", "已保留当前内容；保存时将先备份磁盘版本";
    NoticeExternalDismissed => "notice.external_dismissed", "已取消：外部修改仍待处理";
    NoticeExportCancelled => "notice.export_cancelled", "导出已取消";
    NoticeOutputEmptyDocument => "notice.output_empty_document", "空文档没有可导出或打印的内容";
    NoticeExportedHtml => "notice.exported_html", "已导出 HTML：{}";
    NoticeExportFailed => "notice.export_failed",
        "导出失败：{}；可重试导出，或检查目标目录是否可写后换一个位置";
    NoticeErrorWhileModalOpen => "notice.error_while_modal_open",
        "{}：{}（请先关闭当前弹窗，再重试）";
    ModalExportFailedTitle => "modal.export_failed.title", "导出失败";
    ModalExportFailedBody => "modal.export_failed.body",
        "{}\n\n原因：{}\n\n下一步：可重试导出，或取消后检查目标目录是否可写。";
    NoticeSaveAsCancelled => "notice.save_as_cancelled", "另存为已取消";
    NoticeSavedAs => "notice.saved_as", "已另存为：{}";
    NoticeHistoryMigrationFailed => "notice.history_migration_failed",
        "文档已保存到：{}；但版本历史迁移失败（{}），旧历史仍保留在临时会话目录";
    NoticeSnapshotFailed => "notice.snapshot_failed",
        "版本历史写入失败：{}（正文已保留；可检查磁盘空间或写入权限后重试保存）";
    NoticeAssetsMigrationIncomplete => "notice.assets_migration_incomplete",
        "已另存为：{}；有 {} 个相对图片资产未能迁移（源缺失或复制失败），新位置可能显示断图";
    NoticeSaveAsFailed => "notice.save_as_failed", "另存为失败：{}；可 Ctrl+Shift+S 重试或换一个位置";
    ModalSaveAsFailedTitle => "modal.save_as_failed.title", "另存为失败";
    ModalSaveAsFailedBody => "modal.save_as_failed.body",
        "{}\n\n原因：{}\n\n下一步：可重试写入同一位置，或换一个目录另存；当前修改仍保留在编辑器中。";
    ModalOpenFailedTitle => "modal.open_failed.title", "打开失败";
    ModalOpenFailedBody => "modal.open_failed.body",
        "{}\n\n原因：{}\n\n下一步：可重试读取同一文件，或重新选择其他文件；当前文档内容未被改动。";
    ModalUnsavedTitle => "modal.unsaved.title", "是否保存对「{}」的更改？";
    ModalUnsavedBody => "modal.unsaved.body",
        "「保存」：把本次修改写回原文件。\n「不保存」：丢弃本次修改，关闭后无法恢复。\n「取消」：返回文档继续编辑。";
    NoticeCancelled => "notice.cancelled", "已取消操作";
    NoticeCancelledWhat => "notice.cancelled_what", "已取消{}";
    ActionOpen => "action.open", "打开";
    ActionExportHtml => "action.export_html", "导出 HTML";
    ActionExportPdf => "action.export_pdf", "导出 PDF";
    ActionPrint => "action.print", "打印";
    NoticeSaveIncomplete => "notice.save_incomplete", "保存未完成（失败或取消），操作已取消";
    NoticeCancelDirty => "notice.cancel_dirty", "已取消：当前文档仍有未保存的修改";
    NoticeOpenFailed => "notice.open_failed", "打开失败：{}";
    NoticeOpened => "notice.opened", "已打开：{}";
    NoticeNewDraft => "notice.new_draft", "已新建空白文档";

    // ---- 快照 ----
    NoticeSnapshotReadFailed => "notice.snapshot_read_failed",
        "读取快照失败：{}；可关闭版本历史后重试，或检查快照文件是否被占用";
    NoticeSelectVersion => "notice.select_version", "请先选择一个版本";
    NoticeRestoredVersion => "notice.restored_version", "已恢复该版本（Ctrl+Z 可撤销）";
    NoticeSelectVersionToDelete => "notice.select_version_to_delete", "请先选择要删除的版本";
    ModalDeleteSnapshotTitle => "modal.delete_snapshot.title", "删除版本快照";
    ModalDeleteSnapshotBody => "modal.delete_snapshot.body",
        "{}（{}）\n\n将永久删除该版本，此操作不可撤销。\n\n下一步：确认删除，或取消保留。";
    NoticeDeletedSnapshot => "notice.deleted_snapshot", "已删除该版本快照";
    NoticeDeleteSnapshotFailed => "notice.delete_snapshot_failed",
        "删除快照失败：{}；可检查磁盘空间或文件是否被占用后重试";
    NoticeNoHistory => "notice.no_history", "暂无版本历史可清空";
    ModalClearHistoryTitle => "modal.clear_history.title", "清空版本历史";
    ModalClearHistoryBody => "modal.clear_history.body",
        "将永久删除本文档的全部 {} 份历史快照。\n\n此操作不可撤销；当前文档内容不受影响。\n\n下一步：确认清空，或取消保留。";
    NoticeHistoryCleared => "notice.history_cleared", "已清空版本历史（{} 份）";
    NoticeClearHistoryFailed => "notice.clear_history_failed",
        "清空历史失败：{}；可检查磁盘空间或文件是否被占用后重试";

    // ---- 启动 / 配置 ----
    NoticeConfigIgnored => "notice.config_ignored", "配置有 {} 处无法识别，已忽略";
    NoticeConfigInvalid => "notice.config_invalid", "配置文件无效，已用默认主题: {}";
    NoticeRecovered => "notice.recovered",
        "已从崩溃恢复「{}」的未保存内容；恢复时间 {}。当前内容尚未保存，按 Ctrl+S 保存（{}）";
    RecoveredSaveToDir => "notice.recovered.save_to_dir", "写回原文件所在目录「{}」";
    RecoveredSaveToFile => "notice.recovered.save_to_file", "写回原文件";
    RecoveredSaveChooseLocation => "notice.recovered.save_choose_location", "选择新的保存位置";
    NoticeTheme => "notice.theme", "主题：{}";
    NoticeConfigNotSaved => "notice.config_not_saved", "配置未保存：{}";
    ErrEventLoopCreate => "error.event_loop_create", "创建事件循环失败: {}";
    ErrEventLoopRun => "error.event_loop_run", "事件循环错误: {}";
    FatalWindowCreate => "error.window_create", "无法创建窗口：图形环境不可用";
    CliUsage => "cli.usage", "用法: veil [--new-window] [路径]";
    CliUnknownOption => "cli.unknown_option", "未知参数：{}；{}";
    CliInvalidPath => "cli.invalid_path", "文档路径无效";
    CliCurrentDir => "cli.current_dir", "无法确定当前目录: {}";
    CliPathOutOfBounds => "cli.path_out_of_bounds", "文档路径越界";

    // ---- 编辑交互提示 ----
    NoticeFullscreenOn => "notice.fullscreen_on", "全屏（F11 退出）";
    NoticeFullscreenOff => "notice.fullscreen_off", "已退出全屏";
    NoticeZoom => "notice.zoom", "缩放 {}%";
    NoticeImagePasteNeedsFile => "notice.image_paste_needs_file", "插入图片需要先保存文档";
    NoticeImageSaveFailed => "notice.image_save_failed",
        "图片保存失败：{}；下一步：检查目标目录是否可写，或另存文档后重试";
    NoticeMermaidSnapshotNeedsFile => "notice.mermaid_snapshot_needs_file", "Mermaid 快照未插入：请先保存文档，再写入本地图片";
    NoticeMermaidSnapshotSaveFailed => "notice.mermaid_snapshot_save_failed", "Mermaid 快照保存失败，未修改原文：{}";
    NoticeMermaidSnapshotStale => "notice.mermaid_snapshot_stale", "Mermaid 源码已变化或文档已切换，快照未插入（正文未修改）";
    NoticeImageReadFailed => "notice.image_read_failed",
        "图片读取失败：{}；下一步：检查文件是否存在、是否有读取权限，然后重新加载图片";
    NoticeInsertImageNeedsFile => "notice.insert_image_needs_file", "插入图片需要先保存文档";
    NoticeUnsupportedImage => "notice.unsupported_image",
        "所选文件不是支持的图片格式；下一步：改用 PNG/JPEG/WebP 等支持的格式";
    NoticeFootnoteDefMissing => "notice.footnote_def_missing", "未找到脚注定义";
    NoticeFootnoteRefMissing => "notice.footnote_ref_missing", "未找到脚注引用";
    NoticeAnchorMissing => "notice.anchor_missing", "未找到锚点";
    NoticeLinkUnsupported => "notice.link_unsupported", "无法打开链接：不支持的链接地址 {}";
    NoticeLinkFailed => "notice.link_failed", "无法打开链接: {}";
    NoticeMermaidLiveOpenFailed => "notice.mermaid_live_open_failed",
        "无法打开 mermaid.live：{}。Mermaid 源码已复制；请手动打开 https://mermaid.live/ 并粘贴源码";
    NoticeImageMissing => "notice.image_missing",
        "图片文件缺失，已显示替代文本：{}；下一步：右键可重新加载图片或复制图片路径";
    NoticeCodeCopied => "notice.code_copied", "已复制代码块";
    NoticeCodeBlockRequired => "notice.code_block_required", "光标不在代码块内";
    CodeCopyButton => "code.copy_button", "复制";
    CodeCopiedButton => "code.copied_button", "已复制";
    FrontMatterCollapsed => "doc.frontmatter_collapsed", "Front Matter";
    NoticeFocusMode => "notice.focus_mode", "专注模式：{}";
    NoticeRemoteImageFailed => "notice.remote_image_failed",
        "部分远程图片加载失败，已显示替代文本；下一步：检查链接是否有效，或右键重新加载图片";
    NoticeMermaidRenderFailed => "notice.mermaid_render_failed", "图表渲染失败：{}。请检查 Mermaid 语法";
    NoticeMermaidImageInserted => "notice.mermaid_image_inserted",
        "已插入本地图片快照；上方 Mermaid 源码块仍是活预览，快照不会随源码更新：{}";
    NoticeRemoteImagesBlocked => "notice.remote_images_blocked", "已阻止远程图片";
    NoticeRemoteImagesAllowed => "notice.remote_images_allowed", "已允许远程图片";
    NoticeMermaidSourceTooLarge => "notice.mermaid_source_too_large", "Mermaid 源码超过 128 KiB，已跳过渲染";
    ToggleOn => "toggle.on", "开";
    ToggleOff => "toggle.off", "关";

    // ---- 拖放 ----
    DropImageNeedsDir => "drop.image_needs_dir", "「{}」无法插入：请先保存文档";
    DropInsertImage => "drop.insert_image", "松开以插入图片：{}";
    DropOpenDocument => "drop.open_document", "松开以打开文档：{}";

    // ---- 打印 / 导出 ----
    PrintUnsupportedPdf => "print.unsupported_pdf",
        "当前平台暂不支持导出 PDF；可改用「导出 HTML」，或改在支持打印的平台上重试";
    PrintUnsupportedPrint => "print.unsupported_print",
        "当前平台暂不支持打印；可改用「导出 PDF」，保存后从阅读器打印";
    ImagePreviewTitle => "image.preview_title", "图片预览 · {} · {}";
    ImagePreviewHint => "image.preview_hint",
        "滚轮/方向键平移 · Ctrl+滚轮或 +/- 缩放 · 0 适应窗口 · Esc 或点击遮罩关闭";
    PrintTocTitle => "print.toc_title", "目录";
    ExportUntitledPdf => "export.untitled_pdf", "未命名.pdf";
    ExportUntitledHtml => "export.untitled_html", "未命名.html";
    ExportUntitledMarkdown => "export.untitled_markdown", "未命名.md";
    NoticePrintFailed => "notice.print_failed",
        "打印失败：{}；可重试打印，或检查打印机是否联机、驱动是否正常后重试";
    ModalPrintFailedTitle => "modal.print_failed.title", "打印失败";
    ModalPrintFailedBody => "modal.print_failed.body",
        "原因：{}\n\n下一步：可重试重新打开打印对话框，或检查打印机是否联机、驱动是否正常。";
    NoticeExportPdfFailed => "notice.export_pdf_failed",
        "导出 PDF 失败：{}；可重试导出，或检查目标目录是否可写、文件是否被占用后重试";
    ModalExportPdfFailedTitle => "modal.export_pdf_failed.title", "导出 PDF 失败";
    ModalExportPdfFailedBody => "modal.export_pdf_failed.body",
        "原因：{}\n\n下一步：可重试导出，或取消后检查系统是否安装可用的中文字体、目标目录是否可写、文件是否被占用；既有 PDF 不会被修改。";

    // ---- 平台层（打印 / PDF） ----
    PrintNoPrinterContext => "print.no_printer_context",
        "打印失败：无法获取打印机上下文；请检查打印机连接后重试，或改用导出 PDF";
    PrintSentPagesUnified => "print.sent_pages_unified", "已发送 {} 页到打印机（与 PDF 同一版面）";
    PrintUnifiedFallback => "print.unified_fallback", "统一版面不可用，回退屏幕渲染打印：{}";
    PrintSentPagesScreen => "print.sent_pages_screen", "已发送 {} 页到打印机（屏幕渲染）";
    PrintExportCancelled => "print.export_cancelled", "导出已取消";
    PrintErrStartJob => "print.err.start_job", "打印失败：无法开始打印作业";
    PrintErrInterruptedPartial => "print.err.interrupted_partial", "打印中断：部分页面未输出";
    PrintErrInterruptedNewPage => "print.err.interrupted_new_page", "打印中断：无法开始新页";
    PrintErrInterruptedEnd => "print.err.interrupted_end", "打印中断：无法结束打印作业";
    PrintErrBackupOld => "print.err.backup_old", "无法备份旧文件: {}";
    PrintErrWriteTarget => "print.err.write_target", "无法写入目标文件：{}";
    PrintErrNoCjkFont => "print.err.no_cjk_font", "系统无可用 CJK 字体";
    PrintErrDocEmpty => "print.err.doc_empty", "文档为空";
    PrintErrNoPrintablePages => "print.err.no_printable_pages", "没有可打印的页面";
    PrintErrNoExportPages => "print.err.no_export_pages", "没有可导出的页面";
    PrintErrPdfTooLarge => "print.err.pdf_too_large", "PDF 过大：{} MB";
    PrintErrWriteTemp => "print.err.write_temp", "写临时文件失败: {}";
    PrintErrReplaceTarget => "print.err.replace_target", "替换目标文件失败: {}";
    PrintExportedPdf => "print.exported_pdf", "已导出 PDF：{}（{} 字节，{} 页，{} 条书签，{} 张图片）";
    PrintDocumentName => "print.document_name", "Veil 文档";

    // ---- 文件关联 / 启动对话框 ----
    AssocTitle => "assoc.title", "Veil 文件关联";
    AssocFailedTitle => "assoc.failed_title", "Veil 文件关联失败";
    MainOpenFailedTitle => "main.open_failed_title", "Veil 无法打开文件";
    AssocLocateExe => "assoc.err.locate_exe", "无法定位当前程序路径：{}";
    AssocExeNotUtf8 => "assoc.err.exe_not_utf8", "程序路径不是有效的 UTF-8";
    AssocUnsupported => "assoc.err.unsupported", "当前平台不支持文件关联（仅 Windows）";
    AssocErrWithCode => "assoc.err.with_code", "{}：{}（错误码 {}）";
    AssocOpenKeyFailed => "assoc.err.open_key", "打开注册表键失败";
    AssocCreateKeyFailed => "assoc.err.create_key", "创建注册表键失败";
    AssocWriteValueFailed => "assoc.err.write_value", "写入注册表值失败";
    AssocReadValueFailed => "assoc.err.read_value", "读取注册表值失败";
    AssocDeleteValueFailed => "assoc.err.delete_value", "删除注册表值失败";
    AssocDeleteKeyFailed => "assoc.err.delete_key", "删除注册表键失败";
    AssocMarkdownDescription => "assoc.description", "Markdown 文档";
    AssocDefaultSet => "assoc.note.default_set", "已把 .md 的默认打开方式指向 Veil";
    AssocDefaultOther => "assoc.note.default_other",
        ".md 已有其他关联，未改写；如需 Veil 打开，请在「设置 → 应用 → 默认应用」中选择";
    AssocIconKept => "assoc.note.icon_kept", "已存在自定义文件图标，未改写";
    AssocRegisterKept => "assoc.note.register_kept",
        "以下 ProgID 值由其他程序写入，已保留未覆盖：{}";
    AssocUnregisterKept => "assoc.note.unregister_kept",
        "以下关联项不是本程序写入或已被改写，已保留：{}";
    AssocProgIdDescription => "assoc.item.prog_id_description", "ProgID 描述";
    AssocOpenCommand => "assoc.item.open_command", "打开命令";
    AssocAppIcon => "assoc.item.app_icon", "veil.exe 图标";
    AssocOpenWithEntry => "assoc.item.open_with_entry", "「打开方式」项";
    AssocMarkdownDefault => "assoc.item.markdown_default", ".md 默认值";
    AssocManualDefault => "assoc.note.manual_default",
        "\n\n要让 .md 双击默认由 Veil 打开，请在「设置 → 应用 → 默认应用」中选择 Veil：Windows 不允许程序自行修改默认应用。";
    AssocRegisteredBody => "assoc.registered_body",
        "已注册 Markdown 文件关联（仅当前用户）。\n\n- 文档图标已内嵌：设为 .md 默认应用后即显示\n- 右键 .md →「打开方式」中可选择 Veil";
    AssocUnregisteredBody => "assoc.unregistered_body",
        "已撤销 Veil 的 Markdown 文件关联（仅当前用户）。\n\n若资源管理器仍显示旧图标，注销并重新登录后即可刷新。";

    // ---- 系统对话框 ----
    DialogFilterMarkdown => "dialog.filter.markdown",
        "Markdown (*.md;*.markdown)\0*.md;*.markdown\0所有文件 (*.*)\0*.*\0\0";
    DialogFilterImage => "dialog.filter.image",
        "图片 (*.png;*.jpg;*.jpeg;*.jfif;*.bmp;*.gif;*.webp;*.tif;*.tiff;*.ico)\0*.png;*.jpg;*.jpeg;*.jfif;*.bmp;*.gif;*.webp;*.tif;*.tiff;*.ico\0所有文件 (*.*)\0*.*\0\0";
    DialogOpenMarkdown => "dialog.title.open_markdown", "打开 Markdown";
    DialogSaveMarkdown => "dialog.title.save_markdown", "另存为 Markdown";
    DialogInsertImage => "dialog.title.insert_image", "插入图片";
    DialogExportPdf => "dialog.title.export_pdf", "导出 PDF";
    DialogExportHtml => "dialog.title.export_html", "导出 HTML";
    DialogFilterPdf => "dialog.filter.pdf", "PDF (*.pdf)";
    DialogFilterHtml => "dialog.filter.html", "HTML (*.html)";
    DialogFilterAllFiles => "dialog.filter.all_files", "所有文件 (*.*)";
    DialogFilterMarkdownLabel => "dialog.filter.markdown_label", "Markdown (*.md;*.markdown)";
    DialogFilterMarkdownPattern => "dialog.filter.markdown_pattern", "*.md;*.markdown";
    DialogFilterPdfPattern => "dialog.filter.pdf_pattern", "*.pdf";
    DialogFilterHtmlPattern => "dialog.filter.html_pattern", "*.html";

    // ---- 打开失败说明 ----
    OpenFailNotUtf8 => "open_failure.not_utf8", "文件不是有效的 UTF-8 编码（{}）";
    OpenFailNotUtf8Next => "open_failure.not_utf8_next", "请将文件另存为 UTF-8 后重试。";
    OpenFailNotFound => "open_failure.not_found", "文件不存在或已被移动（{}）";
    OpenFailNotFoundNext => "open_failure.not_found_next", "请确认路径后重试，或从「文件 → 打开」重新选择。";
    OpenFailDenied => "open_failure.denied", "没有读取该文件的权限（{}）";
    OpenFailDeniedNext => "open_failure.denied_next", "请确认文件未被只读锁定或被安全软件拦截，取得权限后重试。";
    OpenFailOther => "open_failure.other", "读取失败（{}）";
    OpenFailOtherNext => "open_failure.other_next", "请确认文件可访问后重试。";
    OpenFailBody => "open_failure.body", "无法打开：{}\n\n原因：{}\n\n下一步：{}";
    ErrBitmapSizeMismatch => "error.bitmap_size_mismatch", "位图尺寸不匹配: {} 字节 ≠ {}×{}×4";
    ErrMathFontUnavailable => "error.math_font_unavailable", "数学字体不可用";
    ErrPngOutputOverflow => "error.png_output_overflow", "PNG 输出尺寸溢出";
    ErrPngBitDepth => "error.png_bit_depth", "非预期 PNG 位深: {}";
    ErrIndexedPng => "error.indexed_png", "索引色 PNG 未展开";
    ErrIoCreateDir => "error.io_create_dir", "{}（创建 {}）";
    ErrIoWriteFile => "error.io_write_file", "{}（写入 {}）";
    ErrNoConfigDir => "error.no_config_dir", "无法定位配置目录";
    ErrConfigNotUtf8 => "error.config_not_utf8", "配置文件不是合法 UTF-8";
    ErrNotUtf8 => "error.not_utf8", "文件不是有效的 UTF-8 编码";
    ErrNoDataDir => "error.no_data_dir", "无法定位数据目录";
    ErrUnsupportedImage => "error.unsupported_image", "当前平台仅支持 PNG 图片";
    ErrUrlUnsupported => "error.url_unsupported", "当前平台不支持打开链接";
    ErrSnapshotDirCreate => "error.snapshot_dir_create", "创建快照目录失败: {}";
    ErrSnapshotWrite => "error.snapshot_write", "写快照失败: {}";
    ErrSnapshotRead => "error.snapshot_read", "快照不存在或不可读（{}）: {}";
    ErrSnapshotMeta => "error.snapshot_meta", "快照元数据校验失败（文件已损坏）";
    ErrSnapshotUtf8 => "error.snapshot_utf8", "快照不是有效 UTF-8: {}";
    ErrSnapshotDelete => "error.snapshot_delete", "删除快照失败（{}）: {}";
    ErrHistoryClear => "error.history_clear", "清空历史失败: {}";
    ErrSnapshotMigrate => "error.snapshot_migrate", "迁移快照失败: {}";
    ErrFontRead => "error.font_read", "字体读取失败 {}: {}";
    ErrFontParse => "error.font_parse", "字体解析失败: {}";
    ErrExportDraftNoPath => "error.export_draft_no_path", "文档还没有保存位置，无法导出";
    ErrFontSubset => "error.font_subset", "字体子集化失败: {}";
    ErrShellProtocol => "error.shell_protocol", "不支持的链接协议: {}";
    ErrShellExecute => "error.shell_execute", "ShellExecuteW 返回 {}";
    ErrWatchInit => "error.watch_init", "文件监听初始化失败";
    ErrWatchPath => "error.watch_path", "{}（监听 {}）";
    ErrHresult => "error.hresult", "{}失败（HRESULT 0x{}）";
    ErrImageSizeInvalid => "error.image_size_invalid", "图片尺寸无效: {}×{}";
    ErrImageTooLarge => "error.image_too_large", "图片过大: {}×{}（上限 1 亿像素）";
    ErrPdfNoDocPath => "error.pdf_no_doc_path", "文档未保存，无法解析相对图片路径";
    ErrPdfImageEmpty => "error.pdf_image_empty", "图片尺寸为空";
    ErrMathRendererUnavailable => "error.math_renderer_unavailable", "公式渲染器不可用";
    ErrMathRendererInternal => "error.math_renderer_internal", "公式渲染器内部错误";
    ErrMathRenderEmpty => "error.math_render_empty", "公式渲染为空";
    WicCreateFactory => "wic.create_factory", "创建 WIC 工厂";
    WicComInit => "wic.com_init", "COM 初始化";
    WicFactoryNull => "wic.factory_null", "创建 WIC 工厂返回空指针";
    WicDecoderNull => "wic.decoder_null", "图片解码器返回空指针";
    WicOpenDecoder => "wic.open_decoder", "打开图片解码器";
    WicReadFrame => "wic.read_frame", "读取图片帧";
    WicFrameNull => "wic.frame_null", "图片帧返回空指针";
    WicCreateConverter => "wic.create_converter", "创建格式转换器";
    WicConverterNull => "wic.converter_null", "格式转换器返回空指针";
    WicConvertPixels => "wic.convert_pixels", "转换像素格式";
    WicReadSize => "wic.read_size", "读取图片尺寸";
    WicReadPixels => "wic.read_pixels", "读取图片像素";

    // ---- 平台层（启动 / IPC / HTTP） ----
    IpcPathAbsolute => "ipc.path_absolute", "IPC 文档路径必须是绝对路径";
    IpcPathUtf8 => "ipc.path_utf8", "IPC 文档路径不是有效 UTF-8";
    IpcPathInvalid => "ipc.path_invalid", "IPC 文档路径无效";
    IpcPathTooLong => "ipc.path_too_long", "IPC 文档路径过长";
    IpcConnectPrimary => "ipc.connect_primary", "无法连接 Veil 主进程: {}";
    IpcUnknown => "ipc.unknown", "未知错误";
    IpcUnsupported => "ipc.unsupported", "当前平台不支持单实例 IPC；将使用独立进程";
    HttpUrlEmpty => "http.url_empty", "URL 为空";
    HttpUrlTooLong => "http.url_too_long", "URL 过长（上限 {} 字符）";
    HttpUrlControl => "http.url_control", "URL 含控制字符";
    HttpUrlMissingScheme => "http.url_missing_scheme", "URL 缺少协议头（仅支持 http/https）";
    HttpUnsupportedScheme => "http.unsupported_scheme", "不支持的协议：{}";
    HttpUrlMissingHost => "http.url_missing_host", "URL 缺少主机名";
    HttpUrlUserInfo => "http.url_user_info", "URL 不允许包含用户信息";
    HttpIpv6MissingBracket => "http.ipv6_missing_bracket", "IPv6 主机缺少右括号";
    HttpHostTrailingChars => "http.host_trailing_chars", "主机名后存在非法字符";
    HttpIpv6NeedsBrackets => "http.ipv6_needs_brackets", "IPv6 主机必须用方括号包裹";
    HttpPortEmpty => "http.port_empty", "端口为空";
    HttpPortInvalid => "http.port_invalid", "端口非法：{}";
    HttpPortOutOfRange => "http.port_out_of_range", "端口超出范围：{}";
    HttpHostInvalidChars => "http.host_invalid_chars", "主机名含非法字符";
    HttpIpv6Invalid => "http.ipv6_invalid", "IPv6 地址非法：{}";
    HttpLocalhostBlocked => "http.localhost_blocked", "不允许访问本机地址";
    HttpPrivateAddressBlocked => "http.private_address_blocked", "不允许访问内网/保留地址：{}";
    HttpNumericHostBlocked => "http.numeric_host_blocked", "不允许数字/十六进制主机名：{}";
    HttpHostInvalid => "http.host_invalid", "主机名非法：{}";
    HttpRedirectEmpty => "http.redirect_empty", "重定向地址为空";
    HttpRedirectTooMany => "http.redirect_too_many", "重定向次数超过 {} 次";
    HttpRedirectMissingLocation => "http.redirect_missing_location", "重定向响应缺少 Location";
    HttpRedirectDowngrade => "http.redirect_downgrade", "拒绝从 https 跳转到 http";
    HttpErrWithCode => "http.err_with_code", "{}失败（错误码 {}）";
    HttpOpOpenSession => "http.op.open_session", "打开 WinHTTP 会话";
    HttpOpConnect => "http.op.connect", "连接远程主机";
    HttpOpCreateRequest => "http.op.create_request", "创建 HTTP 请求";
    HttpOpSetTimeouts => "http.op.set_timeouts", "设置 WinHTTP 超时";
    HttpOpConfigureRequest => "http.op.configure_request", "配置 HTTP 请求";
    HttpOpSendRequest => "http.op.send_request", "发送请求";
    HttpOpReceiveResponse => "http.op.receive_response", "接收响应";
    HttpOpReadStatus => "http.op.read_status", "读取响应状态";
    HttpOpReadBody => "http.op.read_body", "读取响应数据";
    HttpResponseTooLarge => "http.response_too_large", "远程图片超过 {} MiB 上限（{} 字节）";
    HttpResponseEmpty => "http.response_empty", "远程图片响应为空";
    HttpTempWriteFailed => "http.temp_write_failed", "写入远程图片临时文件失败：{}";

    // ---- 设置模态 ----
    ModalSettingsTitle => "modal.settings.title", "设置";
    SettingsThemeMode => "settings.theme_mode", "主题模式";
    SettingsFollowSystem => "settings.follow_system", "跟随系统";
    SettingsLight => "settings.light", "浅色";
    SettingsDark => "settings.dark", "深色";
    SettingsDocFontSize => "settings.doc_font_size", "正文字号（10–32）";
    SettingsDocFontFamily => "settings.doc_font_family", "正文字体族";
    SettingsFontSystem => "settings.font_system", "系统默认";
    SettingsFontSans => "settings.font_sans", "无衬线";
    SettingsFontSerif => "settings.font_serif", "衬线";
    SettingsFontMono => "settings.font_mono", "等宽";
    SettingsAllowRemoteImages => "settings.allow_remote_images", "允许远程图片";
    SettingsWindowGeom => "settings.window_geom", "上次窗口几何";
    SettingsWindowGeomNone => "settings.window_geom_none", "（未记录）";
    SettingsResetWindowGeom => "settings.reset_window_geom", "清除（下次启动重置）";
    SettingsExportPageNumbers => "settings.export_page_numbers", "导出 PDF 时显示页码";
    SettingsExportToc => "settings.export_toc", "导出 PDF 时包含目录页";
    SettingsExportMarginMm => "settings.export_margin_mm", "导出 PDF 页边距（毫米，0–50）";
    SettingsTabWidth => "settings.tab_width", "制表符宽度（2 或 4）";
    NoticeSettingsSaved => "notice.settings_saved", "设置已保存";
    NoticeSettingsSaveFailed => "notice.settings_save_failed", "设置保存失败：{}";
}

/// 已翻译的 en-US 词条；缺失项继续回退 zh-CN。
pub const EN_US: &[(Key, &str)] = &[
    (
        Key::CtxMermaidPreview,
        "Open in mermaid.live (copies source)",
    ),
    (
        Key::NoticeMermaidLiveOpenFailed,
        "Could not open mermaid.live: {}. Mermaid source copied; manually open https://mermaid.live/ and paste the source",
    ),
];

/// 语言表：`code` 为 BCP 47 风格标签；`entries` 为空表示尚未翻译，回退到下一份表。
pub struct Locale {
    pub code: &'static str,
    pub entries: &'static [(Key, &'static str)],
}

/// 默认 zh-CN；en-US 已翻译条目优先，未翻译条目回退 zh-CN。
pub const LOCALES: &[Locale] = &[
    Locale {
        code: "zh-CN",
        entries: ZH_CN,
    },
    Locale {
        code: "en-US",
        entries: EN_US,
    },
];

/// 当前语言（固定 zh-CN；多语言支持时改为可注入）。
pub const CURRENT_LOCALE: &str = "zh-CN";

/// 词条查找：命中返回词条，缺失返回 None（纯函数，便于单测缺失回退）。
pub fn lookup(entries: &'static [(Key, &'static str)], key: Key) -> Option<&'static str> {
    entries
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, text)| *text)
}

/// 缺失 key 的 debug 断言：测试构建跳过，保证「回退不 panic」可被单测覆盖。
fn missing(key: Key) -> &'static str {
    #[cfg(not(test))]
    debug_assert!(false, "i18n 缺失词条: {} ({})", key.id(), CURRENT_LOCALE);
    key.id()
}

/// 当前语言优先；当前语言缺失词条或未配置时，按语言表顺序回退。
fn translate(locales: &[Locale], current: &str, key: Key) -> Option<&'static str> {
    if let Some(text) = locales
        .iter()
        .find(|locale| locale.code == current)
        .and_then(|locale| lookup(locale.entries, key))
    {
        return Some(text);
    }
    locales
        .iter()
        .filter(|locale| locale.code != current)
        .find_map(|locale| lookup(locale.entries, key))
}

/// 取当前语言词条；所有语言均缺失时回退显示 key id（不 panic）。
pub fn t(key: Key) -> &'static str {
    translate(LOCALES, CURRENT_LOCALE, key).unwrap_or_else(|| missing(key))
}

/// 按 `{}` 占位符顺序填充参数；参数不足时返回词条原文（占位符原样保留），
/// 绝不静默吞成空串，也绝不 panic。多余参数忽略。
/// 模板本身来自词条表，调用点只负责提供动态值。
pub fn tf(key: Key, args: &[&str]) -> String {
    let template = t(key);
    let needed = template.matches("{}").count();
    if args.len() < needed {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    let mut i = 0usize;
    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        if let Some(arg) = args.get(i) {
            out.push_str(arg);
        }
        i += 1;
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

/// 错误信息单行裁剪：仅取首行，超过 `MAX_ERR_CHARS` 字符时截断并追加省略号。
///
/// 远程渲染/RPC 错误可能携带 URL、SVG 片段或反序列化细节，直接拼进状态栏会撑爆
/// 单行提示。这里按字符（非字节）裁剪，保证 UTF-8 边界安全；完整信息仍由调用点
/// 写 stderr 供排查。
pub fn clip_error(msg: &str) -> String {
    const MAX_ERR_CHARS: usize = 120;
    let first_line = msg.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= MAX_ERR_CHARS {
        return first_line.to_string();
    }
    let mut out: String = first_line.chars().take(MAX_ERR_CHARS).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 穷举：每个 Key 都必须有 zh-CN 词条，且词条非空（含空串 key 的显式例外）。
    #[test]
    fn every_key_has_zh_cn_entry() {
        for &key in Key::ALL {
            let text = lookup(ZH_CN, key)
                .unwrap_or_else(|| panic!("缺少 zh-CN 词条: {:?} ({})", key, key.id()));
            if key != Key::Empty {
                assert!(!text.is_empty(), "词条不得为空: {:?}", key);
            }
        }
    }

    /// 词条表不得含未定义的 key / 重复 key：ALL 与 ZH_CN 一一对应。
    #[test]
    fn key_and_entry_tables_are_in_sync() {
        assert_eq!(ZH_CN.len(), Key::ALL.len(), "词条数与 key 数不一致");
        for (i, &key) in Key::ALL.iter().enumerate() {
            assert_eq!(ZH_CN[i].0, key, "词条顺序/内容与枚举不一致: {:?}", key);
        }
    }

    /// 稳定 id 是日志与后续翻译资产的契约：不得为空或重复。
    #[test]
    fn stable_ids_are_nonempty_and_unique() {
        for (i, &key) in Key::ALL.iter().enumerate() {
            assert!(!key.id().is_empty(), "稳定 id 不得为空: {:?}", key);
            assert!(
                !Key::ALL[..i].iter().any(|other| other.id() == key.id()),
                "稳定 id 重复: {}",
                key.id()
            );
        }
    }

    /// 当前语言优先；空翻译、缺失词条和未知语言均按表顺序回退。
    #[test]
    fn locale_fallback_prefers_current_then_table_order() {
        const ZH: &[(Key, &str)] = &[(Key::MenuFile, "文件")];
        const EN: &[(Key, &str)] = &[(Key::MenuFile, "File")];
        const EMPTY: &[(Key, &str)] = &[];
        const WITH_EN: &[Locale] = &[
            Locale {
                code: "zh-CN",
                entries: ZH,
            },
            Locale {
                code: "en-US",
                entries: EN,
            },
        ];
        const WITH_EMPTY_EN: &[Locale] = &[
            Locale {
                code: "zh-CN",
                entries: ZH,
            },
            Locale {
                code: "en-US",
                entries: EMPTY,
            },
        ];
        assert_eq!(translate(WITH_EN, "en-US", Key::MenuFile), Some("File"));
        assert_eq!(
            translate(WITH_EMPTY_EN, "en-US", Key::MenuFile),
            Some("文件")
        );
        assert_eq!(translate(WITH_EN, "fr-FR", Key::MenuFile), Some("文件"));
        assert_eq!(translate(WITH_EN, "en-US", Key::MenuEdit), None);
    }

    /// 缺失 key：回退显示 id、不 panic（测试构建跳过 debug 断言）。
    #[test]
    fn missing_key_falls_back_to_id_without_panic() {
        let empty: &'static [(Key, &'static str)] = &[];
        assert_eq!(lookup(empty, Key::MenuFile), None);
        assert_eq!(missing(Key::MenuFile), "menu.file");
        // t() 对每个合法 key 恒不 panic。
        for &key in Key::ALL {
            let _ = t(key);
        }
    }

    /// 抽查：菜单 / 错误弹窗 / 空态 / tooltip 的取词与旧字面量逐字一致。
    #[test]
    fn spot_check_matches_legacy_literals() {
        assert_eq!(t(Key::MenuFile), "文件");
        assert_eq!(t(Key::MenuFilePrint), "打印…");
        assert_eq!(t(Key::ShortcutPrint), "Ctrl+Shift+P");
        assert_eq!(t(Key::MenuCurrent), "当前");
        assert_eq!(t(Key::ModalOpenFailedTitle), "打开失败");
        assert_eq!(
            t(Key::ModalOpenFailedBody),
            "{}\n\n原因：{}\n\n下一步：可重试读取同一文件，或重新选择其他文件；当前文档内容未被改动。"
        );
        assert_eq!(t(Key::HistoryEmpty), "暂无版本历史");
        assert_eq!(t(Key::QuickOpenEmptyNoMatch), "无匹配文件");
        assert_eq!(
            t(Key::QuickOpenPlaceholder),
            "快速打开：输入文件名，回车打开"
        );
        assert_eq!(t(Key::StatusExternalChanged), "（外部修改）");
        assert_eq!(t(Key::MermaidEmptyHint), "空 Mermaid 块 — 点击插入模板");
        assert_eq!(
            t(Key::CtxMermaidPreview),
            "在 mermaid.live 打开（复制源码）"
        );
        assert_eq!(t(Key::CalloutNote), "Note");
    }

    #[test]
    fn mermaid_live_wording_is_consistent_in_zh_and_en() {
        assert_eq!(
            translate(LOCALES, "en-US", Key::CtxMermaidPreview),
            Some("Open in mermaid.live (copies source)")
        );
        let en_error = translate(LOCALES, "en-US", Key::NoticeMermaidLiveOpenFailed)
            .expect("英文失败提示已翻译");
        assert!(en_error.contains("Mermaid source copied"));
        assert!(en_error.contains("manually open https://mermaid.live/ and paste the source"));
    }

    /// 参数化模板：顺序填充、缺参不 panic、无占位符原样返回。
    #[test]
    fn format_fills_placeholders_in_order() {
        assert_eq!(
            tf(Key::StatusStats, &["12", "3", "7"]),
            "12 字 · 行 3，列 7"
        );
        assert_eq!(tf(Key::WindowTitle, &["a.md"]), "veil — a.md");
        assert_eq!(tf(Key::StatusSaved, &["09:30"]), " · 已保存 09:30");
        // 缺参：返回词条原文（占位符保留），不静默吞成空串
        assert_eq!(tf(Key::StatusSaved, &[]), " · 已保存 {}");
        assert_eq!(tf(Key::MenuFile, &["多余"]), "文件");
    }

    /// 错误类提示统一「原因 + 下一步」：导出失败必须给出可执行出口。
    #[test]
    fn export_failed_notice_has_actionable_next_step() {
        let notice = tf(Key::NoticeExportFailed, &["磁盘已满"]);
        assert!(notice.contains("磁盘已满"), "须保留原因: {notice}");
        assert!(notice.contains("重试"), "须给重试出口: {notice}");
        assert!(
            notice.contains("目标目录"),
            "须提示检查目标目录是否可写: {notice}"
        );
    }

    /// 打印 / PDF 失败提示：保留原因 + 可执行下一步，且与成功态文案可区分。
    #[test]
    fn print_failures_have_actionable_next_step() {
        let unsupported_print = t(Key::PrintUnsupportedPrint);
        assert!(
            unsupported_print.contains("不支持打印"),
            "须保留原因: {unsupported_print}"
        );
        assert!(
            unsupported_print.contains("导出 PDF") && unsupported_print.contains("打印"),
            "须给出改用导出 PDF 的出口: {unsupported_print}"
        );

        let unsupported_pdf = t(Key::PrintUnsupportedPdf);
        assert!(
            unsupported_pdf.contains("不支持导出 PDF"),
            "须保留原因: {unsupported_pdf}"
        );
        assert!(
            unsupported_pdf.contains("导出 HTML"),
            "须给出改用导出 HTML 的出口: {unsupported_pdf}"
        );

        let print_failed = tf(Key::NoticePrintFailed, &["设备离线"]);
        assert!(
            print_failed.contains("设备离线"),
            "须保留原因: {print_failed}"
        );
        assert!(
            print_failed.contains("打印机") && print_failed.contains("重试"),
            "须提示检查打印机并重试: {print_failed}"
        );

        let pdf_failed = tf(Key::NoticeExportPdfFailed, &["拒绝访问"]);
        assert!(pdf_failed.contains("拒绝访问"), "须保留原因: {pdf_failed}");
        assert!(
            pdf_failed.contains("目标目录") && pdf_failed.contains("重试"),
            "须提示检查目标目录后重试: {pdf_failed}"
        );

        let no_context = t(Key::PrintNoPrinterContext);
        assert!(
            no_context.contains("无法获取打印机上下文"),
            "须保留原因: {no_context}"
        );
        assert!(
            no_context.contains("检查打印机连接") && no_context.contains("重试"),
            "须提示检查打印机并重试: {no_context}"
        );
        assert!(
            no_context.contains("导出 PDF"),
            "须给出改用导出 PDF 的出口: {no_context}"
        );

        // 成功态是「已发送 / 已导出」，失败态不得与其中任一条混同。
        for success in [
            t(Key::PrintSentPagesUnified),
            t(Key::PrintSentPagesScreen),
            t(Key::PrintExportedPdf),
        ] {
            assert!(
                success.starts_with("已"),
                "成功态文案必须以「已」开头: {success}"
            );
            assert!(!print_failed.contains(success), "失败文案混入成功态");
            assert!(!pdf_failed.contains(success), "失败文案混入成功态");
            assert!(!no_context.contains(success), "失败文案混入成功态");
        }
        assert!(
            print_failed.contains("失败")
                && pdf_failed.contains("失败")
                && no_context.contains("失败")
        );
    }

    /// 未保存关闭对话框：正文逐项说明三个按钮各自的后果，缺一不可。
    #[test]
    fn unsaved_body_explains_each_button_consequence() {
        let body = t(Key::ModalUnsavedBody);
        assert!(
            body.contains("「保存」") && body.contains("原文件"),
            "保存=写回原文件: {body}"
        );
        assert!(
            body.contains("「不保存」") && body.contains("丢弃"),
            "不保存=丢弃本次修改: {body}"
        );
        assert!(
            body.contains("「取消」") && body.contains("继续编辑"),
            "取消=返回文档继续编辑: {body}"
        );
    }

    /// 崩溃恢复提示不硬编码文件名：对象、时间、保存去向都由调用点以参数注入。
    #[test]
    fn recovered_notice_renders_object_time_and_save_hint() {
        assert_eq!(
            t(Key::NoticeRecovered).matches("{}").count(),
            3,
            "恢复对象、时间与保存去向必须都是占位符参数"
        );
        let save_hint = tf(Key::RecoveredSaveToDir, &["资料归档"]);
        let notice = tf(Key::NoticeRecovered, &["a.md", "14:32", &save_hint]);
        assert!(notice.contains("崩溃恢复"), "须说明来源: {notice}");
        assert!(notice.contains("「a.md」"), "须含恢复对象: {notice}");
        assert!(notice.contains("14:32"), "须含恢复时间: {notice}");
        assert!(notice.contains("尚未保存"), "须说明当前未保存: {notice}");
        assert!(notice.contains("Ctrl+S"), "须给出保存方式: {notice}");
        assert!(
            notice.contains("原文件所在目录「资料归档」"),
            "有原路径时须说明保存回原位置: {notice}"
        );
        // 无原路径：保存去向应说明可选择新位置，而不是照抄文件名。
        let choose = t(Key::RecoveredSaveChooseLocation);
        let notice = tf(Key::NoticeRecovered, &["未命名草稿", "14:32", choose]);
        assert!(
            notice.contains("选择新的保存位置"),
            "无路径时须说明可选择保存位置: {notice}"
        );
        // 有路径但取不到父目录名（如盘符根目录）：说明写回原文件，不编造位置。
        let file = t(Key::RecoveredSaveToFile);
        let notice = tf(Key::NoticeRecovered, &["a.md", "14:32", file]);
        assert!(
            notice.contains("写回原文件") && !notice.contains("目录"),
            "取不到目录名时只说写回原文件: {notice}"
        );
    }

    /// 快照读写删、清空历史与文件监听失败：保留原因，并给出可执行的下一步。
    #[test]
    fn storage_and_watch_failures_offer_actionable_next_step() {
        let write = tf(Key::NoticeSnapshotFailed, &["磁盘已满"]);
        assert!(write.contains("磁盘已满"), "须保留原因: {write}");
        assert!(
            write.contains("磁盘空间") && write.contains("权限") && write.contains("重试"),
            "写入失败须给可执行出口: {write}"
        );
        let read = tf(Key::NoticeSnapshotReadFailed, &["快照已损坏"]);
        assert!(read.contains("快照已损坏"), "须保留原因: {read}");
        assert!(
            read.contains("重试") && read.contains("关闭版本历史"),
            "读取失败须说明如何重试: {read}"
        );
        for (key, reason) in [
            (Key::NoticeDeleteSnapshotFailed, "拒绝访问"),
            (Key::NoticeClearHistoryFailed, "拒绝访问"),
        ] {
            let text = tf(key, &[reason]);
            assert!(text.contains(reason), "须保留原因: {text}");
            assert!(
                text.contains("磁盘空间") && text.contains("重试"),
                "删除类失败须给可执行出口: {text}"
            );
        }
        let watch = tf(Key::NoticeWatchUnavailable, &["没有权限"]);
        assert!(watch.contains("没有权限"), "须保留原因: {watch}");
        assert!(
            watch.contains("重新打开该文件"),
            "监听失败须说明如何再次尝试: {watch}"
        );
        let recovery = tf(Key::NoticeRecoveryWriteFailed, &["磁盘已满"]);
        assert!(recovery.contains("磁盘已满"), "须保留原因: {recovery}");
        assert!(
            recovery.contains("重试"),
            "恢复写入失败须给可执行出口: {recovery}"
        );
    }

    /// 图片类失败提示：保留原因之外，必须给出用户真能执行的下一步。
    #[test]
    fn image_failures_offer_actionable_next_step() {
        let save = tf(Key::NoticeImageSaveFailed, &["磁盘已满"]);
        assert!(save.contains("磁盘已满"), "须保留原因: {save}");
        assert!(save.contains("下一步"), "须给下一步: {save}");
        assert!(
            save.contains("目标目录是否可写") && save.contains("另存"),
            "保存失败须说明检查目标目录或另存: {save}"
        );

        let read = tf(Key::NoticeImageReadFailed, &["文件不存在"]);
        assert!(read.contains("文件不存在"), "须保留原因: {read}");
        assert!(read.contains("下一步"), "须给下一步: {read}");
        assert!(
            read.contains("读取权限") && read.contains("重新加载图片"),
            "读取失败须说明检查权限并重新加载: {read}"
        );

        let unsupported = t(Key::NoticeUnsupportedImage);
        assert!(
            unsupported.contains("不是支持的图片格式"),
            "须保留原因: {unsupported}"
        );
        assert!(unsupported.contains("下一步"), "须给下一步: {unsupported}");
        assert!(
            unsupported.contains("PNG")
                && unsupported.contains("JPEG")
                && unsupported.contains("WebP"),
            "须说明可改用的格式: {unsupported}"
        );

        let missing = tf(Key::NoticeImageMissing, &["img/a.png"]);
        assert!(missing.contains("img/a.png"), "须保留原因: {missing}");
        assert!(missing.contains("下一步"), "须给下一步: {missing}");
        assert!(
            missing.contains("重新加载图片") && missing.contains("复制图片路径"),
            "须指向真实存在的右键项: {missing}"
        );

        let remote = t(Key::NoticeRemoteImageFailed);
        assert!(remote.contains("远程图片加载失败"), "须保留原因: {remote}");
        assert!(remote.contains("下一步"), "须给下一步: {remote}");
        assert!(
            remote.contains("链接") && remote.contains("重新加载图片"),
            "须说明检查链接或右键重新加载: {remote}"
        );
    }

    /// 错误裁剪：只取首行；超长按字符截断并加省略号；UTF-8 安全。
    #[test]
    fn clip_error_keeps_first_line_and_caps_length() {
        // 多行错误只留首行（去掉尾部空白）
        assert_eq!(
            clip_error("HTTP: connection reset by peer\n  at line 2\n  at line 3"),
            "HTTP: connection reset by peer"
        );
        // 短错误原样保留
        assert_eq!(clip_error("timeout"), "timeout");
        // 超长错误截到 120 字符 + 省略号，且不破坏中文字符边界
        let long = "错".repeat(200);
        let clipped = clip_error(&long);
        assert_eq!(clipped.chars().count(), 121, "120 字符 + 省略号");
        assert!(clipped.ends_with('…'));
        // 恰好 120 字符不加省略号
        let exact = "a".repeat(120);
        assert_eq!(clip_error(&exact), exact);
    }

    /// 参数不足：整体回退为词条原文，不半填、不空串、不 panic。
    #[test]
    fn insufficient_args_return_template_verbatim() {
        // StatusStats 有 3 个占位符，只给 1 个 → 整体回退原文
        assert_eq!(tf(Key::StatusStats, &["1"]), t(Key::StatusStats));
        // 0 参数同理
        assert_eq!(tf(Key::StatusSaved, &[]), t(Key::StatusSaved));
        assert!(
            tf(Key::StatusSaved, &[]).contains("{}"),
            "缺失占位符须原样可见，不能蒸发"
        );
        // 参数恰好/多余时行为不变
        assert_eq!(tf(Key::StatusSaved, &["09:30"]), " · 已保存 09:30");
        assert_eq!(tf(Key::MenuFile, &["多余"]), "文件");
    }
}
