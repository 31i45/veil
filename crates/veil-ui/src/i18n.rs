//! 界面文案 key 化（N7/B8c）：默认 zh-CN 词条表 + `t()` 取词。
//!
//! 设计取舍：
//! - `Key` 枚举与 zh-CN 表由同一份宏生成，穷举性由构造保证（不会出现「有 key 无词条」）；
//! - 词条按语言分表（`zh-CN` / `en-US`），`t()` 按 `LOCALES` 顺序回退；两张表均全量，
//!   新增 key 时需同时补齐英文（只需填表，不改调用点）；
//! - 运行时语言由产品层在启动时经 `init_locale()` 确定（未初始化时默认 zh-CN）；
//!   `t()` / `missing()` 读运行时语言，英文覆盖一律走纯函数 `translate()` 测试，
//!   不在测试里改写全局语言（避免并发用例串扰）；
//! - 缺失 key 回退显示 key 的稳定 id（如 `menu.file.new`），debug 构建断言提示；
//!   测试构建跳过断言，保证「回退不 panic」可被单测覆盖。
//!
//! 显示等价是硬约束：所有 zh-CN 词条与 key 化前的字面量逐字一致，包括空格与全角标点。

use std::sync::OnceLock;

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
    MenuEditCopyCode => "menu.edit.copy_code", "复制代码块";
    MenuEditFind => "menu.edit.find", "查找";
    MenuEditReplace => "menu.edit.replace", "替换";
    MenuEditInsertImage => "menu.edit.insert_image", "插入图片…";

    MenuViewOutline => "menu.view.outline", "大纲";
    MenuViewZoomIn => "menu.view.zoom_in", "放大";
    MenuViewZoomOut => "menu.view.zoom_out", "缩小";
    MenuViewZoomReset => "menu.view.zoom_reset", "还原缩放";
    MenuViewFullscreen => "menu.view.fullscreen", "全屏";
    MenuViewThemeSystem => "menu.view.theme_system", "跟随系统";
    MenuViewThemeLight => "menu.view.theme_light", "浅色";
    MenuViewThemeDark => "menu.view.theme_dark", "深色";
    MenuViewSourceMode => "menu.view.source_mode", "源码模式";
    MenuViewFocusMode => "menu.view.focus_mode", "专注模式";
    MenuViewTypewriter => "menu.view.typewriter", "打字机模式";
    MenuViewAllowRemoteImages => "menu.view.allow_remote_images", "允许远程图片";
    MenuViewSettings => "menu.view.settings", "设置…";
    MenuHelp => "menu.help", "帮助";
    MenuHelpAbout => "menu.help.about", "关于 Veil";
    MenuHelpExportDiagnostics => "menu.help.export_diagnostics", "导出诊断信息…";
    MenuHelpCheckUpdate => "menu.help.check_update", "检查更新";

    // ---- 快捷键提示（菜单右侧列） ----
    ShortcutNew => "shortcut.new", "Ctrl+N";
    ShortcutNewWindow => "shortcut.new_window", "Ctrl+Shift+N";
    ShortcutOpen => "shortcut.open", "Ctrl+O";
    ShortcutQuickOpen => "shortcut.quick_open", "Ctrl+P";
    ShortcutSave => "shortcut.save", "Ctrl+S";
    ShortcutSaveAs => "shortcut.save_as", "Ctrl+Shift+S";
    ShortcutExportHtml => "shortcut.export_html", "Ctrl+E";
    ShortcutExportPdf => "shortcut.export_pdf", "Ctrl+Shift+E";
    ShortcutPrint => "shortcut.print", "Ctrl+Alt+P";
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
    DisabledNoCodeBlock => "disabled.no_code_block", "光标不在代码块内";

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
    QuickOpenReadFailed => "quick_open.read_failed", "无法读取目录；按 Esc 关闭后重试";
    QuickOpenRecent => "quick_open.recent", "最近";

    // ---- 命令面板（Ctrl+Shift+P）：与快速打开共用浮层 ----
    CommandPalettePlaceholder => "command_palette.placeholder", "命令面板：输入命令，回车执行";
    CommandPaletteEmpty => "command_palette.empty", "无匹配命令";

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
    CtxAllowRemoteImages => "ctx.allow_remote_images", "允许远程图片（全局）…";
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
    MermaidLoading => "mermaid.loading", "渲染中…";

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
        "文件监听不可用: {}；下一步：重新打开该文件即可再次尝试";
    NoticeConflictNameExhausted => "notice.conflict_name_exhausted",
        "冲突副本名已耗尽；下一步：清理同目录下的冲突副本，或另存到其他位置";
    NoticeExternalBackupFailed => "notice.external_backup_failed",
        "外部版本备份失败：取消保存 {}；下一步：可检查磁盘空间或权限后重试保存，或另存为";
    NoticeSaved => "notice.saved", "已保存：{}";
    NoticeSaveFailed => "notice.save_failed",
        "保存失败：{}；下一步：可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为";
    ModalSaveFailedTitle => "modal.save_failed.title", "保存失败";
    ModalSaveFailedBody => "modal.save_failed.body",
        "{}\n\n原因：{}\n\n下一步：可重试本次保存，或用「另存为」写入其他位置；当前修改仍保留在编辑器中。";
    ModalExternalBackupFailBody => "modal.external_backup_fail.body",
        "{}\n\n原因：外部版本备份失败（{}）。\n\n下一步：为避免覆盖外部修改，本次保存已取消；可重试或「另存为」写入其他位置。";
    NoticeRecoveryWriteFailed => "notice.recovery_write_failed",
        "崩溃恢复写入失败：{}；下一步：可检查磁盘空间或权限后重试";
    NoticeFileRestored => "notice.file_restored", "文件已恢复：{}";
    NoticeExternalChanged => "notice.external_changed", "检测到外部修改：保存时将先备份磁盘版本";
    NoticeExternalDeleted => "notice.external_deleted",
        "文件已被外部删除；编辑器中内容仍在，下一步：可继续编辑，或用「另存为」保存到新位置";
    NoticeExternalUnreadable => "notice.external_unreadable",
        "文件暂不可读（可能被外部程序占用）；编辑器中内容仍在，下一步：可继续编辑或另存为";
    ModalExternalChangedTitle => "modal.external_changed.title", "「{}」已被外部修改";
    ModalExternalChangedBody => "modal.external_changed.body", "磁盘文件已被其他程序修改。重新加载将丢弃未保存的修改；保留当前可继续编辑，保存时会先备份磁盘版本。";
    NoticeExternalReloaded => "notice.external_reloaded", "已重新加载外部修改：{}";
    NoticeExternalKept => "notice.external_kept", "已保留当前内容；保存时将先备份磁盘版本";
    NoticeExternalDismissed => "notice.external_dismissed", "已取消：外部修改仍待处理";
    NoticeExportCancelled => "notice.export_cancelled", "导出已取消";
    NoticeOutputEmptyDocument => "notice.output_empty_document", "空文档没有可导出或打印的内容";
    NoticeExportedHtml => "notice.exported_html", "已导出 HTML：{}";
    NoticeExportFailed => "notice.export_failed",
        "导出失败：{}；下一步：可重试导出，或检查目标目录是否可写后换一个位置";
    NoticeErrorWhileModalOpen => "notice.error_while_modal_open",
        "{}：{}；下一步：请先关闭当前弹窗，再重试";
    ModalExportFailedTitle => "modal.export_failed.title", "导出失败";
    ModalExportFailedBody => "modal.export_failed.body",
        "{}\n\n原因：{}\n\n下一步：可重试导出，或取消后检查目标目录是否可写。";
    NoticeSaveAsCancelled => "notice.save_as_cancelled", "另存为已取消";
    NoticeSavedAs => "notice.saved_as", "已另存为：{}";
    NoticeHistoryMigrationFailed => "notice.history_migration_failed",
        "文档已保存到：{}；但版本历史迁移失败（{}），旧历史仍保留在临时会话目录；下一步：可检查磁盘空间或权限后重试保存";
    NoticeSnapshotFailed => "notice.snapshot_failed",
        "版本历史写入失败：{}（正文已保留）；下一步：可检查磁盘空间或写入权限后重试保存";
    NoticeAssetsMigrationIncomplete => "notice.assets_migration_incomplete",
        "已另存为：{}；有 {} 个相对图片资产未能迁移（源缺失或复制失败），新位置可能显示断图；下一步：请检查源图片是否仍存在，并把缺失图片复制到新位置";
    NoticeSaveAsFailed => "notice.save_as_failed",
        "另存为失败：{}；下一步：可 Ctrl+Shift+S 重试或换一个位置";
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
    NoticeSaveIncomplete => "notice.save_incomplete",
        "保存未完成（失败或取消），操作已取消；下一步：可重试保存，或检查失败原因后重试";
    NoticeCancelDirty => "notice.cancel_dirty", "已取消：当前文档仍有未保存的修改";
    NoticeOpenFailed => "notice.open_failed",
        "打开失败：{}；下一步：可确认文件存在且有读取权限后重试，或重新选择文件";
    NoticeOpened => "notice.opened", "已打开：{}";
    NoticeNewDraft => "notice.new_draft", "已新建空白文档";

    // ---- 快照 ----
    NoticeSnapshotReadFailed => "notice.snapshot_read_failed",
        "读取快照失败：{}；下一步：可关闭版本历史后重试，或检查快照文件是否被占用";
    NoticeSelectVersion => "notice.select_version", "请先选择一个版本";
    NoticeRestoredVersion => "notice.restored_version", "已恢复该版本（Ctrl+Z 可撤销）";
    NoticeSelectVersionToDelete => "notice.select_version_to_delete", "请先选择要删除的版本";
    ModalDeleteSnapshotTitle => "modal.delete_snapshot.title", "删除版本快照";
    ModalDeleteSnapshotBody => "modal.delete_snapshot.body",
        "{}（{}）\n\n将永久删除该版本，此操作不可撤销。\n\n下一步：确认删除，或取消保留。";
    NoticeDeletedSnapshot => "notice.deleted_snapshot", "已删除该版本快照";
    NoticeDeleteSnapshotFailed => "notice.delete_snapshot_failed",
        "删除快照失败：{}；下一步：可检查磁盘空间或文件是否被占用后重试";
    NoticeNoHistory => "notice.no_history", "暂无版本历史可清空";
    ModalClearHistoryTitle => "modal.clear_history.title", "清空版本历史";
    ModalClearHistoryBody => "modal.clear_history.body",
        "将永久删除本文档的全部 {} 份历史快照。\n\n此操作不可撤销；当前文档内容不受影响。\n\n下一步：确认清空，或取消保留。";
    NoticeHistoryCleared => "notice.history_cleared", "已清空版本历史（{} 份）";
    NoticeClearHistoryFailed => "notice.clear_history_failed",
        "清空历史失败：{}；下一步：可检查磁盘空间或文件是否被占用后重试";

    // ---- 启动 / 配置 ----
    NoticeConfigIgnored => "notice.config_ignored", "配置有 {} 处无法识别，已忽略";
    NoticeDiagnosticsSaved => "notice.diagnostics_saved", "已导出诊断信息：{}";
    NoticeDiagnosticsFailed => "notice.diagnostics_failed", "导出诊断信息失败：{}";
    NoticeUpdateOpenFailed => "notice.update_open_failed", "无法打开更新页面：{}";
    NoticeConfigInvalid => "notice.config_invalid",
        "配置文件无效，已用默认主题: {}；下一步：修正配置文件后重启应用，或删除配置文件以恢复默认设置";
    NoticeRecovered => "notice.recovered",
        "已从崩溃恢复「{}」的未保存内容；恢复时间 {}。当前内容尚未保存，按 Ctrl+S 保存（{}）";
    RecoveredSaveToDir => "notice.recovered.save_to_dir", "写回原文件所在目录「{}」";
    RecoveredSaveToFile => "notice.recovered.save_to_file", "写回原文件";
    RecoveredSaveChooseLocation => "notice.recovered.save_choose_location", "选择新的保存位置";
    NoticeTheme => "notice.theme", "主题：{}";
    NoticeConfigNotSaved => "notice.config_not_saved",
        "配置未保存：{}；下一步：可检查配置目录的写入权限或磁盘空间后重试";
    ErrEventLoopCreate => "error.event_loop_create",
        "创建事件循环失败: {}；请重启应用；若仍失败，检查图形环境或重新安装";
    ErrEventLoopRun => "error.event_loop_run",
        "事件循环错误: {}；请重启应用；若仍失败，检查图形环境或重新安装";
    FatalWindowCreate => "error.window_create",
        "无法创建窗口：图形环境不可用；请重启应用；若仍失败，检查图形环境或重新安装";
    FatalWindowCreateDetail => "error.window_create_detail", "创建窗口失败：{}";
    FatalWindowRenderDetail => "error.window_render_detail", "创建呈现上下文失败：{}";
    FatalWindowBusyNext => "error.window_busy_next",
        "请关闭一个窗口后重试；现有文档不会被关闭。";
    CliUsage => "cli.usage", "用法: veil [--new-window] [路径]";
    CliUnknownOption => "cli.unknown_option",
        "未知参数：{}；{}；下一步：请检查参数拼写，或按上面的用法重新运行";
    CliInvalidPath => "cli.invalid_path",
        "文档路径无效；下一步：请检查路径是否存在、是否为空，修正后重新运行";
    CliCurrentDir => "cli.current_dir",
        "无法确定当前目录: {}；下一步：请改用绝对路径重新运行";
    CliPathOutOfBounds => "cli.path_out_of_bounds",
        "文档路径越界；下一步：请检查路径中的「..」并使用绝对路径重新运行";

    // ---- 编辑交互提示 ----
    NoticeFullscreenOn => "notice.fullscreen_on", "全屏（F11 退出）";
    NoticeFullscreenOff => "notice.fullscreen_off", "已退出全屏";
    NoticeZoom => "notice.zoom", "缩放 {}%";
    NoticeImagePasteNeedsFile => "notice.image_paste_needs_file", "插入图片需要先保存文档";
    NoticeImageSaveFailed => "notice.image_save_failed",
        "图片保存失败：{}；下一步：检查目标目录是否可写，或另存文档后重试";
    NoticeMermaidSnapshotNeedsFile => "notice.mermaid_snapshot_needs_file", "Mermaid 快照未插入：请先保存文档，再写入本地图片";
    NoticeMermaidSnapshotSaveFailed => "notice.mermaid_snapshot_save_failed",
        "Mermaid 快照保存失败，未修改原文：{}；下一步：可检查目标目录是否可写后重新插入快照";
    NoticeMermaidSnapshotStale => "notice.mermaid_snapshot_stale",
        "Mermaid 源码已变化或文档已切换，快照未插入（正文未修改）；下一步：请重新预览图表并再次插入快照";
    NoticeImageReadFailed => "notice.image_read_failed",
        "图片读取失败：{}；下一步：检查文件是否存在、是否有读取权限，然后重新加载图片";
    NoticeInsertImageNeedsFile => "notice.insert_image_needs_file", "插入图片需要先保存文档";
    NoticeUnsupportedImage => "notice.unsupported_image",
        "所选文件不是支持的图片格式；下一步：改用 PNG/JPEG/WebP 等支持的格式";
    NoticeFootnoteDefMissing => "notice.footnote_def_missing",
        "未找到脚注定义；下一步：检查脚注名称是否与引用一致";
    NoticeFootnoteRefMissing => "notice.footnote_ref_missing",
        "未找到脚注引用；下一步：检查脚注名称是否与定义一致";
    NoticeAnchorMissing => "notice.anchor_missing",
        "未找到锚点；下一步：检查锚点名称是否与标题一致";
    NoticeLinkUnsupported => "notice.link_unsupported",
        "无法打开链接：不支持的链接地址 {}；下一步：改用 http/https 链接，或复制到浏览器打开";
    NoticeLinkFailed => "notice.link_failed",
        "无法打开链接：{}；下一步：检查系统默认浏览器，或复制链接手动打开";
    NoticeMermaidLiveOpenFailed => "notice.mermaid_live_open_failed",
        "无法打开 mermaid.live：{}。Mermaid 源码已复制；下一步：请手动打开 https://mermaid.live/ 并粘贴源码";
    NoticeImageMissing => "notice.image_missing",
        "图片文件缺失，已显示替代文本：{}；下一步：右键可重新加载图片或复制图片路径";
    NoticeCodeCopied => "notice.code_copied", "已复制代码块";
    NoticeCodeBlockRequired => "notice.code_block_required", "光标不在代码块内";
    CodeCopyButton => "code.copy_button", "复制";
    CodeCopiedButton => "code.copied_button", "已复制";
    FrontMatterCollapsed => "doc.frontmatter_collapsed", "Front Matter";
    NoticeFocusMode => "notice.focus_mode", "专注模式：{}";
    NoticeTypewriterMode => "notice.typewriter_mode", "打字机模式：{}";
    NoticeRemoteImageFailed => "notice.remote_image_failed",
        "部分远程图片加载失败，已显示替代文本；下一步：检查链接是否有效，或右键重新加载图片";
    NoticeMermaidRenderFailed => "notice.mermaid_render_failed",
        "图表渲染失败：{}；下一步：请检查 Mermaid 语法后重新预览";
    NoticeMermaidImageInserted => "notice.mermaid_image_inserted",
        "已插入本地图片快照；上方 Mermaid 源码块仍是活预览，快照不会随源码更新：{}";
    NoticeRemoteImagesBlocked => "notice.remote_images_blocked", "已阻止远程图片";
    NoticeRemoteImageBlockedRetry => "notice.remote_image_blocked_retry",
        "已阻止远程图片；下一步：在「视图 → 允许远程图片」（或设置）中开启后重试";
    NoticeRemoteImagesAllowed => "notice.remote_images_allowed", "已允许远程图片";
    ConfirmAllowRemoteImagesTitle => "confirm.allow_remote_images.title", "允许远程图片（全局）";
    ConfirmAllowRemoteImagesBody => "confirm.allow_remote_images.body",
        "允许后，文档中的远程图片会向外部服务器发起请求（可能暴露你的网络地址与访问时间）。\n\n这是一次全局设置；确定允许吗？";
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
    ImagePreviewUnavailable => "image.preview_unavailable", "图片无法显示";
    PrintTocTitle => "print.toc_title", "目录";
    ExportUntitledPdf => "export.untitled_pdf", "未命名.pdf";
    ExportUntitledHtml => "export.untitled_html", "未命名.html";
    ExportUntitledMarkdown => "export.untitled_markdown", "未命名.md";
    NoticePrintFailed => "notice.print_failed",
        "打印失败：{}；下一步：可重试打印，或检查打印机是否联机、驱动是否正常后重试";
    ModalPrintFailedTitle => "modal.print_failed.title", "打印失败";
    ModalPrintFailedBody => "modal.print_failed.body",
        "原因：{}\n\n下一步：可重试重新打开打印对话框，或检查打印机是否联机、驱动是否正常。";
    NoticeExportPdfFailed => "notice.export_pdf_failed",
        "导出 PDF 失败：{}；下一步：可重试导出，或检查目标目录是否可写、文件是否被占用后重试";
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
    DialogExportDiagnostics => "dialog.title.export_diagnostics", "导出诊断信息";
    DialogFilterText => "dialog.filter.text", "文本文件 (*.txt)";
    DialogFilterTextPattern => "dialog.filter.text_pattern", "*.txt";

    // ---- 关于 ----
    AboutTitle => "about.title", "关于 Veil";
    AboutOpenHomepage => "about.open_homepage", "打开项目主页";

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
    MathRenderFailedHint => "math.render_failed_hint", "公式渲染失败，已显示源码";
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
    SettingsUiLanguage => "settings.ui_language", "界面语言（重启生效）";
    SettingsLangAuto => "settings.lang_auto", "自动（跟随系统）";
    SettingsLangZhCn => "settings.lang_zh_cn", "中文";
    SettingsLangEnUs => "settings.lang_en_us", "English";
    SettingsDocFontSize => "settings.doc_font_size", "正文字号（10–32）";
    SettingsDocLineHeight => "settings.doc_line_height", "正文行高（字号–3×字号）";
    SettingsDocFontFamily => "settings.doc_font_family", "正文字体族";
    SettingsDocMargin => "settings.doc_margin", "版心";
    SettingsMarginCompact => "settings.margin_compact", "紧凑";
    SettingsMarginStandard => "settings.margin_standard", "标准";
    SettingsMarginRelaxed => "settings.margin_relaxed", "宽松";
    SettingsMarginFull => "settings.margin_full", "全宽";
    SettingsMarginCustom => "settings.margin_custom", "自定义";
    SettingsDocMarginPx => "settings.doc_margin_px", "自定义版心边距（4–200，仅自定义生效）";
    SettingsFontSystem => "settings.font_system", "系统默认";
    SettingsFontSans => "settings.font_sans", "无衬线";
    SettingsFontSerif => "settings.font_serif", "衬线";
    SettingsFontMono => "settings.font_mono", "等宽";
    SettingsFontSearch => "settings.font_search", "搜索字体（输入筛选，↑↓ 选择，Enter 确认）";
    SettingsFontNoMatch => "settings.font_no_match", "无匹配字体";
    SettingsFontMissingCjk => "settings.font_missing_cjk", "该字族缺少中文字形，正文将回退到系统字体";
    SettingsAllowRemoteImages => "settings.allow_remote_images", "允许远程图片";
    SettingsWindowGeom => "settings.window_geom", "上次窗口几何";
    SettingsWindowGeomNone => "settings.window_geom_none", "（未记录）";
    SettingsResetWindowGeom => "settings.reset_window_geom", "清除（下次启动重置）";
    SettingsExportPageNumbers => "settings.export_page_numbers", "导出 PDF 时显示页码";
    SettingsExportToc => "settings.export_toc", "导出 PDF 时包含目录页";
    SettingsExportMarginMm => "settings.export_margin_mm", "导出 PDF 页边距（毫米，5–50）";
    SettingsTabWidth => "settings.tab_width", "制表符宽度（2 或 4）";
    SettingsSourcePlainLines => "settings.source_plain_lines", "源码模式：纯文本行视图";
    NoticeSettingsSaved => "notice.settings_saved", "设置已保存";
    NoticeUiLanguageRestart => "notice.ui_language_restart", "界面语言已更改，重启后生效";
    NoticeSettingsSaveFailed => "notice.settings_save_failed",
        "设置保存失败：{}；下一步：可检查配置目录的写入权限或磁盘空间后重试";

    // ---- 首跑欢迎文档 ----
    DocEmptyHint => "doc.empty_hint", "从这里开始写，或按 Ctrl+O 打开已有的 Markdown 文件";
    SampleWelcome => "doc.sample_welcome", r#"# 欢迎使用 Veil

Veil：专注写作的 Markdown 编辑器，边写边排版。

这是你的第一页。Markdown 源码始终是唯一的内容。

## 现在可以做的三件事

- 打开文件：按 Ctrl+O，选择电脑里已有的 Markdown 文档。
- 直接开始写：把这段文字替换掉，或者接着往下写。
- 导出：按 Ctrl+E 导出 HTML，按 Ctrl+Shift+E 导出 PDF。

## 正文里的样式

**加粗**、*斜体* 和 `行内代码` 都可以直接输入，链接写成 [站点名称](https://example.com)，图片写成 `![说明](图片路径.png)`。

公式也可以写在句子里，例如 $x^2 + y^2 = z^2$；需要独立成行时：

$$
\frac{a + b}{\sqrt{c^2 + 1}} = \sum_{k=0}^{n} k^2
$$

```rust
fn main() {
    println!("Hello, Veil");
}
```
"#;

    // ---- 关于 ----
    AboutBody => "about.body",
        "Veil {}（构建 {}）\n许可证：MIT\n\
         隐私：无遥测、无后台更新检查、无后台网络请求；远程图片默认关闭。\n\
         配置目录：{}\n诊断目录：{}\n官方来源：{}\n\
         第三方组件：Mermaid（MIT）、QuickJS（MIT）、linkedom（MIT）以及全部 Rust 依赖；完整许可见随包 THIRD-PARTY-LICENSES.md。";
    AboutUnavailable => "about.unavailable", "不可用";

    // ---- 配置文件 ----
    ConfigHeader => "config.header",
        "# Veil 配置（自动生成）\n# mode: system（跟随系统）| light | dark";

    // ---- 导出兜底 ----
    ExportMermaidUnavailable => "export.mermaid_unavailable",
        "<!-- Mermaid 渲染数据不可用：导出时未预览过该块。可在浏览器中打开 https://mermaid.live 查看源码。 -->\n<pre><code class=\"language-mermaid\">{}</code></pre>\n";

    // ---- 致命启动错误 ----
    FatalErrorBody => "error.fatal_body",
        "{}\n\n请检查图形环境后重试；文档内容未被修改。";

    // ---- 文件关联列表分隔符 ----
    AssocListSeparator => "assoc.list_separator", "、";
}

/// en-US 词条（全量，与 `Key::ALL` 一一对应）。
pub const EN_US: &[(Key, &str)] = &[
    (Key::MenuViewTypewriter, "Typewriter Mode"),
    (Key::NoticeTypewriterMode, "Typewriter mode: {}"),
    (
        Key::CtxMermaidPreview,
        "Open in mermaid.live (copies source)",
    ),
    (
        Key::NoticeMermaidLiveOpenFailed,
        "Could not open mermaid.live: {}. Mermaid source copied; next: manually open https://mermaid.live/ and paste the source",
    ),
    (
        Key::DocEmptyHint,
        "Start writing here, or press Ctrl+O to open an existing Markdown file",
    ),
    (
        Key::SampleWelcome,
        r#"# Welcome to Veil

Veil: a Markdown editor built for focused writing, typesetting as you type.

This is your first page. The Markdown source is always the single source of truth.

## Three things you can do now

- Open a file: press Ctrl+O and pick an existing Markdown document on your computer.
- Just start writing: replace this text, or keep going below it.
- Export: press Ctrl+E to export HTML, or Ctrl+Shift+E to export PDF.

## Styling in the body

You can type **bold**, *italic* and `inline code` directly; write links as [Site name](https://example.com) and images as `![Caption](image-path.png)`.

Formulas can sit inside a sentence, e.g. $x^2 + y^2 = z^2$; for a standalone display:

$$
\frac{a + b}{\sqrt{c^2 + 1}} = \sum_{k=0}^{n} k^2
$$

```rust
fn main() {
    println!("Hello, Veil");
}
```
"#,
    ),
    (
        Key::AboutBody,
        "Veil {} (build {})\nLicense: MIT\n\
         Privacy: no telemetry, no background update checks, no background network requests; remote images are off by default.\n\
         Config directory: {}\nDiagnostics directory: {}\nOfficial source: {}\n\
         Third-party components: Mermaid (MIT), QuickJS (MIT), linkedom (MIT) and all Rust dependencies; see THIRD-PARTY-LICENSES.md bundled with the app for the full texts.",
    ),
    (Key::AboutUnavailable, "Unavailable"),
    (
        Key::ConfigHeader,
        "# Veil configuration (auto-generated)\n# mode: system (follow OS) | light | dark",
    ),
    (
        Key::ExportMermaidUnavailable,
        "<!-- Mermaid render data unavailable: this block was not previewed before export. Open https://mermaid.live in a browser to view the source. -->\n<pre><code class=\"language-mermaid\">{}</code></pre>\n",
    ),
    (
        Key::FatalErrorBody,
        "{}\n\nCheck the graphics environment and try again; your document was not modified.",
    ),
    (Key::AssocListSeparator, ", "),
    (Key::ImageAltFallback, "[Image]"),

    // ---- 全量英文词条（按 zh-CN 表顺序；上排为早期先行词条） ----

    // ---- 通用 ----
    (Key::Empty, ""),
    (Key::UntitledDraft, "Untitled Draft"),
    (Key::MenuCurrent, "Current"),

    // ---- 菜单 ----
    (Key::MenuFile, "File"),
    (Key::MenuEdit, "Edit"),
    (Key::MenuView, "View"),
    (Key::MenuFileNew, "New"),
    (Key::MenuFileNewWindow, "New Window"),
    (Key::MenuFileOpen, "Open…"),
    (Key::MenuFileQuickOpen, "Quick Open"),
    (Key::MenuFileSave, "Save"),
    (Key::MenuFileSaveAs, "Save As…"),
    (Key::MenuFileExportHtml, "Export HTML…"),
    (Key::MenuFileExportPdf, "Export PDF…"),
    (Key::MenuFilePrint, "Print…"),
    (Key::MenuFileVersionHistory, "Version History"),
    (Key::MenuFileQuit, "Close Window"),
    (Key::MenuEditUndo, "Undo"),
    (Key::MenuEditRedo, "Redo"),
    (Key::MenuEditCut, "Cut"),
    (Key::MenuEditCopy, "Copy"),
    (Key::MenuEditPaste, "Paste"),
    (Key::MenuEditSelectAll, "Select All"),
    (Key::MenuEditBold, "Bold"),
    (Key::MenuEditItalic, "Italic"),
    (Key::MenuEditLink, "Link"),
    (Key::MenuEditCopyCode, "Copy Code Block"),
    (Key::MenuEditFind, "Find"),
    (Key::MenuEditReplace, "Replace"),
    (Key::MenuEditInsertImage, "Insert Image…"),
    (Key::MenuViewOutline, "Outline"),
    (Key::MenuViewZoomIn, "Zoom In"),
    (Key::MenuViewZoomOut, "Zoom Out"),
    (Key::MenuViewZoomReset, "Reset Zoom"),
    (Key::MenuViewFullscreen, "Fullscreen"),
    (Key::MenuViewThemeSystem, "Follow System"),
    (Key::MenuViewThemeLight, "Light"),
    (Key::MenuViewThemeDark, "Dark"),
    (Key::MenuViewSourceMode, "Source Mode"),
    (Key::MenuViewFocusMode, "Focus Mode"),
    (Key::MenuViewAllowRemoteImages, "Allow Remote Images"),
    (Key::MenuViewSettings, "Settings…"),
    (Key::MenuHelp, "Help"),
    (Key::MenuHelpAbout, "About Veil"),
    (Key::MenuHelpExportDiagnostics, "Export Diagnostics…"),
    (Key::MenuHelpCheckUpdate, "Check for Updates"),

    // ---- 快捷键提示 ----
    (Key::ShortcutNew, "Ctrl+N"),
    (Key::ShortcutNewWindow, "Ctrl+Shift+N"),
    (Key::ShortcutOpen, "Ctrl+O"),
    (Key::ShortcutQuickOpen, "Ctrl+P"),
    (Key::ShortcutSave, "Ctrl+S"),
    (Key::ShortcutSaveAs, "Ctrl+Shift+S"),
    (Key::ShortcutExportHtml, "Ctrl+E"),
    (Key::ShortcutExportPdf, "Ctrl+Shift+E"),
    (Key::ShortcutPrint, "Ctrl+Alt+P"),
    (Key::ShortcutVersionHistory, "Ctrl+Shift+H"),
    (Key::ShortcutUndo, "Ctrl+Z"),
    (Key::ShortcutRedo, "Ctrl+Shift+Z"),
    (Key::ShortcutCut, "Ctrl+X"),
    (Key::ShortcutCopy, "Ctrl+C"),
    (Key::ShortcutCopyCode, "Ctrl+Shift+C"),
    (Key::ShortcutPaste, "Ctrl+V"),
    (Key::ShortcutSelectAll, "Ctrl+A"),
    (Key::ShortcutBold, "Ctrl+B"),
    (Key::ShortcutItalic, "Ctrl+I"),
    (Key::ShortcutLink, "Ctrl+K"),
    (Key::ShortcutFind, "Ctrl+F"),
    (Key::ShortcutReplace, "Ctrl+H"),
    (Key::ShortcutInsertImage, "Ctrl+Shift+I"),
    (Key::ShortcutOutline, "Ctrl+Shift+O"),
    (Key::ShortcutZoomIn, "Ctrl+="),
    (Key::ShortcutZoomOut, "Ctrl+-"),
    (Key::ShortcutZoomReset, "Ctrl+0"),
    (Key::ShortcutFullscreen, "F11"),
    (Key::ShortcutSourceMode, "Ctrl+/"),
    (Key::ShortcutSettings, "Ctrl+,"),

    // ---- 菜单禁用原因 ----
    (Key::DisabledNoChanges, "No changes to save"),
    (Key::DisabledEmptyDocument, "Nothing to export in an empty document"),
    (Key::DisabledNothingToUndo, "Nothing to undo"),
    (Key::DisabledNothingToRedo, "Nothing to redo"),
    (Key::DisabledNoCodeBlock, "Cursor is not inside a code block"),

    // ---- 窗口标题 / 状态栏 ----
    (Key::WindowTitle, "veil — {}"),
    (Key::AppTitle, "veil"),
    (Key::TitleDirtyPrefix, "* "),
    (Key::TitleExternalChanged, " (externally modified)"),
    (Key::TitleExternalBackedUp, " (external change backed up)"),
    (Key::TitleExternalMissing, " (file missing)"),
    (Key::StatusExternalChanged, " (externally modified)"),
    (Key::StatusExternalBackedUp, " (external change backed up)"),
    (Key::StatusExternalMissing, " (file missing)"),
    (Key::StatusSourceMode, " (source mode)"),
    (Key::StatusUnsaved, " · Unsaved"),
    (Key::StatusSaved, " · Saved {}"),
    (Key::StatusStats, "{} words · line {}, col {}"),
    (Key::StatusWords, "{} words"),

    // ---- 快速打开 ----
    (Key::QuickOpenPlaceholder, "Quick Open: type a file name and press Enter"),
    (
        Key::QuickOpenEmptyNoCandidates,
        "No recent files or Markdown files in this folder",
    ),
    (Key::QuickOpenEmptyNoMatch, "No matching files"),
    (Key::QuickOpenReadFailed, "Could not read folder; Esc to retry"),
    (Key::QuickOpenRecent, "Recent"),
    (
        Key::CommandPalettePlaceholder,
        "Command palette: type a command and press Enter",
    ),
    (Key::CommandPaletteEmpty, "No matching commands"),

    // ---- 查找替换 ----
    (Key::SearchPlaceholder, "Find"),
    (Key::SearchReplacePlaceholder, "Replace with"),
    (Key::SearchReplace, "Replace"),
    (Key::SearchReplaceAll, "All"),
    (Key::SearchCaseSensitive, "Aa"),
    (Key::SearchRegex, ".*"),
    (Key::SearchRegexError, "Invalid regex"),
    (Key::SearchCount, "{}/{}"),

    // ---- 版本历史 ----
    (Key::HistoryTitle, "Version History"),
    (Key::HistoryRestore, "Restore This Version"),
    (Key::HistoryDelete, "Delete Snapshot"),
    (Key::HistoryClear, "Clear History"),
    (Key::HistoryClose, "Close"),
    (Key::HistoryEmpty, "No version history yet"),
    (
        Key::HistoryEmptyHint,
        "Old versions are archived automatically before each save",
    ),
    (Key::HistoryCurrentSuffix, " · Current"),
    (Key::HistoryUndoHint, "Press Ctrl+Z to undo the restore"),
    (Key::HistoryErrorHeadline, "Failed to read history: {}"),
    (Key::HistoryErrorHint, "Close the panel and try again"),

    // ---- 大纲 ----
    (Key::OutlineTitle, "Outline"),
    (Key::OutlineEmpty, "No headings"),

    // ---- 右键菜单 ----
    (Key::CtxCut, "Cut"),
    (Key::CtxCopy, "Copy"),
    (Key::CtxPaste, "Paste"),
    (Key::CtxSelectAll, "Select All"),
    (Key::CtxOpenInBrowser, "Open in Browser"),
    (Key::CtxCopyLink, "Copy Link Address"),
    (Key::CtxInsertImage, "Insert Image…"),
    (Key::CtxCopyImagePath, "Copy Image Path"),
    (Key::CtxCopyCode, "Copy Code Block"),
    (Key::CtxReloadImage, "Reload Image"),
    (Key::CtxAllowRemoteImages, "Allow Remote Images (Global)…"),
    (Key::CtxImageMissing, "Image file missing (showing alt text)"),
    (Key::CtxMermaidPreviewImage, "Preview Current Source Diagram"),
    (
        Key::CtxMermaidInsertImage,
        "Insert Local Image Snapshot (Keeps Source)",
    ),
    (Key::CtxMermaidCopySource, "Copy Source"),
    (Key::CtxMermaidInsertFlowchart, "Insert Flowchart Template"),
    (
        Key::CtxMermaidInsertSequence,
        "Insert Sequence Diagram Template",
    ),
    (Key::CtxMermaidInsertClass, "Insert Class Diagram Template"),
    (Key::CtxFold, "Fold"),
    (Key::CtxUnfold, "Unfold"),
    (Key::CtxTableInsertRowAbove, "Insert Row Above"),
    (Key::CtxTableInsertRowBelow, "Insert Row Below"),
    (Key::CtxTableDeleteRow, "Delete Row"),
    (Key::CtxTableInsertColumnLeft, "Insert Column Left"),
    (Key::CtxTableInsertColumnRight, "Insert Column Right"),
    (Key::CtxTableDeleteColumn, "Delete Column"),
    (Key::CtxTableAlignLeft, "Align Left"),
    (Key::CtxTableAlignCenter, "Center"),
    (Key::CtxTableAlignRight, "Align Right"),

    // ---- Callout ----
    (Key::CalloutNote, "Note"),
    (Key::CalloutTip, "Tip"),
    (Key::CalloutImportant, "Important"),
    (Key::CalloutWarning, "Warning"),
    (Key::CalloutCaution, "Caution"),

    // ---- Mermaid 卡片 ----
    (Key::MermaidFlowchart, "Flowchart"),
    (Key::MermaidSequence, "Sequence Diagram"),
    (Key::MermaidClass, "Class Diagram"),
    (Key::MermaidState, "State Diagram"),
    (Key::MermaidEr, "ER Diagram"),
    (Key::MermaidGantt, "Gantt Chart"),
    (Key::MermaidPie, "Pie Chart"),
    (Key::MermaidUnknown, "mermaid"),
    (Key::MermaidBadge, "◈ {}"),
    (
        Key::MermaidEmptyHint,
        "Empty Mermaid block — click to insert a template",
    ),
    (Key::MermaidLoading, "Rendering…"),
    // ---- 正文降级提示 ----
    (
        Key::BlockOversizeNotice,
        "This block is about {} MB, exceeding the per-block render limit; later content was not rendered. The source is preserved in full.",
    ),

    // ---- 通用按钮 ----
    (Key::BtnSave, "Save"),
    (Key::BtnDontSave, "Don't Save"),
    (Key::BtnCancel, "Cancel"),
    (Key::BtnRetry, "Retry"),
    (Key::BtnOpenOther, "Open Another File"),
    (Key::BtnClose, "Close"),
    (Key::BtnSaveAs, "Save As"),
    (Key::BtnChangeLocation, "Change Location"),
    (Key::BtnReselect, "Choose Again"),
    (Key::BtnDelete, "Delete"),
    (Key::BtnClear, "Clear"),
    (Key::BtnConfirm, "Confirm"),
    (Key::BtnReload, "Reload"),
    (Key::BtnKeepCurrent, "Keep Current"),
    (Key::BtnRestoreDefaults, "Restore Defaults"),

    // ---- 保存 / 打开 / 导出 失败与提示 ----
    (
        Key::NoticeWatchUnavailable,
        "File watching unavailable: {}. Next: reopen the file to try again.",
    ),
    (
        Key::NoticeConflictNameExhausted,
        "Conflict copy names exhausted. Next: clean up conflict copies in the same folder, or use Save As to save elsewhere.",
    ),
    (
        Key::NoticeExternalBackupFailed,
        "External version backup failed: save canceled for {}. Next: check disk space or permissions, then save again, or use Save As.",
    ),
    (Key::NoticeSaved, "Saved: {}"),
    (
        Key::NoticeSaveFailed,
        "Save failed: {}. Next: press Ctrl+S to retry, or Ctrl+Shift+S to save as.",
    ),
    (Key::ModalSaveFailedTitle, "Save Failed"),
    (
        Key::ModalSaveFailedBody,
        "{}\n\nReason: {}\n\nNext: retry this save, or use Save As to write elsewhere; your changes are still in the editor.",
    ),
    (
        Key::ModalExternalBackupFailBody,
        "{}\n\nReason: external version backup failed ({}).\n\nNext: to avoid overwriting the external change, this save was canceled; retry or use Save As to write elsewhere.",
    ),
    (
        Key::NoticeRecoveryWriteFailed,
        "Crash recovery write failed: {}. Next: check disk space or permissions and try again.",
    ),
    (Key::NoticeFileRestored, "File restored: {}"),
    (
        Key::NoticeExternalChanged,
        "External change detected: the disk version will be backed up before saving.",
    ),
    (
        Key::NoticeExternalDeleted,
        "The file was deleted externally; the content is still in the editor. Next: keep editing, or use Save As to save to a new location.",
    ),
    (
        Key::NoticeExternalUnreadable,
        "The file is temporarily unreadable (it may be locked by another program); the content is still in the editor. Next: keep editing or use Save As.",
    ),
    (
        Key::ModalExternalChangedTitle,
        "\"{}\" was changed externally",
    ),
    (
        Key::ModalExternalChangedBody,
        "The file on disk was modified by another program. Reloading discards unsaved changes; Keep Current lets you keep editing, and the disk version will be backed up before saving.",
    ),
    (Key::NoticeExternalReloaded, "Reloaded the external change: {}"),
    (
        Key::NoticeExternalKept,
        "Kept the current content; the disk version will be backed up before saving.",
    ),
    (
        Key::NoticeExternalDismissed,
        "Canceled: the external change is still pending.",
    ),
    (Key::NoticeExportCancelled, "Export canceled"),
    (
        Key::NoticeOutputEmptyDocument,
        "An empty document has nothing to export or print.",
    ),
    (Key::NoticeExportedHtml, "Exported HTML: {}"),
    (
        Key::NoticeExportFailed,
        "Export failed: {}. Next: retry the export, or check that the target folder is writable and choose another location.",
    ),
    (
        Key::NoticeErrorWhileModalOpen,
        "{}: {}. Next: close the current dialog, then try again.",
    ),
    (Key::ModalExportFailedTitle, "Export Failed"),
    (
        Key::ModalExportFailedBody,
        "{}\n\nReason: {}\n\nNext: retry the export, or cancel and check that the target folder is writable.",
    ),
    (Key::NoticeSaveAsCancelled, "Save As canceled"),
    (Key::NoticeSavedAs, "Saved as: {}"),
    (
        Key::NoticeHistoryMigrationFailed,
        "Document saved to: {}; version history migration failed ({}), and the old history remains in the temporary session folder. Next: check disk space or permissions and save again.",
    ),
    (
        Key::NoticeSnapshotFailed,
        "Failed to write version history: {} (your content is preserved). Next: check disk space or write permissions and save again.",
    ),
    (
        Key::NoticeAssetsMigrationIncomplete,
        "Saved as: {}; {} relative image assets could not be migrated (source missing or copy failed), so broken images may appear in the new location. Next: check that the source images still exist and copy the missing images to the new location.",
    ),
    (
        Key::NoticeSaveAsFailed,
        "Save As failed: {}. Next: press Ctrl+Shift+S to retry, or choose another location.",
    ),
    (Key::ModalSaveAsFailedTitle, "Save As Failed"),
    (
        Key::ModalSaveAsFailedBody,
        "{}\n\nReason: {}\n\nNext: retry writing to the same location, or choose another folder; your changes are still in the editor.",
    ),
    (Key::ModalOpenFailedTitle, "Open Failed"),
    (
        Key::ModalOpenFailedBody,
        "{}\n\nReason: {}\n\nNext: retry reading the same file, or choose a different file; the current document is unchanged.",
    ),
    (Key::ModalUnsavedTitle, "Save changes to \"{}\"?"),
    (
        Key::ModalUnsavedBody,
        "\"Save\": write these changes back to the original file.\n\"Don't Save\": discard these changes; they cannot be recovered after closing.\n\"Cancel\": return to the document and keep editing.",
    ),
    (Key::NoticeCancelled, "Canceled"),
    (Key::NoticeCancelledWhat, "Canceled: {}"),
    (Key::ActionOpen, "Open"),
    (Key::ActionExportHtml, "Export HTML"),
    (Key::ActionExportPdf, "Export PDF"),
    (Key::ActionPrint, "Print"),
    (
        Key::NoticeSaveIncomplete,
        "Save did not complete (failed or canceled), so the action was canceled. Next: retry the save, or check the failure reason and try again.",
    ),
    (
        Key::NoticeCancelDirty,
        "Canceled: the document still has unsaved changes.",
    ),
    (
        Key::NoticeOpenFailed,
        "Open failed: {}. Next: confirm the file exists and is readable, then try again, or choose another file.",
    ),
    (Key::NoticeOpened, "Opened: {}"),
    (Key::NoticeNewDraft, "Created a new blank document"),

    // ---- 快照 ----
    (
        Key::NoticeSnapshotReadFailed,
        "Failed to read snapshot: {}. Next: close Version History and try again, or check whether the snapshot file is in use.",
    ),
    (Key::NoticeSelectVersion, "Select a version first"),
    (
        Key::NoticeRestoredVersion,
        "Version restored (press Ctrl+Z to undo)",
    ),
    (
        Key::NoticeSelectVersionToDelete,
        "Select a version to delete first",
    ),
    (Key::ModalDeleteSnapshotTitle, "Delete Version Snapshot"),
    (
        Key::ModalDeleteSnapshotBody,
        "{} ({})\n\nThis version will be permanently deleted; this cannot be undone.\n\nNext: confirm the deletion, or cancel to keep it.",
    ),
    (Key::NoticeDeletedSnapshot, "Version snapshot deleted"),
    (
        Key::NoticeDeleteSnapshotFailed,
        "Failed to delete snapshot: {}. Next: check disk space or whether the file is in use, then try again.",
    ),
    (Key::NoticeNoHistory, "No version history to clear"),
    (Key::ModalClearHistoryTitle, "Clear Version History"),
    (
        Key::ModalClearHistoryBody,
        "This will permanently delete all {} historical snapshots of this document.\n\nThis cannot be undone; the current document content is unaffected.\n\nNext: confirm clearing, or cancel to keep them.",
    ),
    (Key::NoticeHistoryCleared, "Version history cleared ({} snapshots)"),
    (
        Key::NoticeClearHistoryFailed,
        "Failed to clear history: {}. Next: check disk space or whether files are in use, then try again.",
    ),
    // ---- 启动 / 配置 ----
    (
        Key::NoticeConfigIgnored,
        "Ignored {} unrecognized configuration entries",
    ),
    (Key::NoticeDiagnosticsSaved, "Diagnostics exported: {}"),
    (Key::NoticeDiagnosticsFailed, "Failed to export diagnostics: {}"),
    (
        Key::NoticeUpdateOpenFailed,
        "Could not open the update page: {}",
    ),
    (
        Key::NoticeConfigInvalid,
        "The configuration file is invalid; using the default theme: {}. Next: fix the configuration file and restart the app, or delete it to restore defaults.",
    ),
    (
        Key::NoticeRecovered,
        "Recovered unsaved content for \"{}\" from a crash; recovery time {}. The current content is not saved yet: press Ctrl+S to save ({})",
    ),
    (
        Key::RecoveredSaveToDir,
        "write back to the original file's folder \"{}\"",
    ),
    (Key::RecoveredSaveToFile, "write back to the original file"),
    (
        Key::RecoveredSaveChooseLocation,
        "choose a new save location",
    ),
    (Key::NoticeTheme, "Theme: {}"),
    (
        Key::NoticeConfigNotSaved,
        "Configuration not saved: {}. Next: check write permissions or disk space for the config directory and try again.",
    ),
    (
        Key::ErrEventLoopCreate,
        "Failed to create the event loop: {}. Restart the app; if it still fails, check the graphics environment or reinstall.",
    ),
    (
        Key::ErrEventLoopRun,
        "Event loop error: {}. Restart the app; if it still fails, check the graphics environment or reinstall.",
    ),
    (
        Key::FatalWindowCreate,
        "Could not create a window: the graphics environment is unavailable. Restart the app; if it still fails, check the graphics environment or reinstall.",
    ),
    (Key::FatalWindowCreateDetail, "Failed to create window: {}"),
    (
        Key::FatalWindowRenderDetail,
        "Failed to create rendering context: {}",
    ),
    (
        Key::FatalWindowBusyNext,
        "Close a window and try again; your open documents will stay open.",
    ),
    (Key::CliUsage, "Usage: veil [--new-window] [path]"),
    (
        Key::CliUnknownOption,
        "Unknown option: {}; {}; Next: check the option spelling, or rerun with the usage above.",
    ),
    (
        Key::CliInvalidPath,
        "Invalid document path. Next: check that the path exists and is not empty, then rerun.",
    ),
    (
        Key::CliCurrentDir,
        "Could not determine the current directory: {}. Next: rerun with an absolute path.",
    ),
    (
        Key::CliPathOutOfBounds,
        "Document path is out of bounds. Next: check for \"..\" in the path and rerun with an absolute path.",
    ),

    // ---- 编辑交互提示 ----
    (Key::NoticeFullscreenOn, "Fullscreen (press F11 to exit)"),
    (Key::NoticeFullscreenOff, "Exited fullscreen"),
    (Key::NoticeZoom, "Zoom {}%"),
    (
        Key::NoticeImagePasteNeedsFile,
        "Save the document before inserting an image",
    ),
    (
        Key::NoticeImageSaveFailed,
        "Failed to save image: {}. Next: check that the target folder is writable, or save the document elsewhere and try again.",
    ),
    (
        Key::NoticeMermaidSnapshotNeedsFile,
        "Mermaid snapshot not inserted: save the document first, then write the local image.",
    ),
    (
        Key::NoticeMermaidSnapshotSaveFailed,
        "Failed to save the Mermaid snapshot; the source was not modified: {}. Next: check that the target folder is writable and insert the snapshot again.",
    ),
    (
        Key::NoticeMermaidSnapshotStale,
        "The Mermaid source changed or the document switched, so the snapshot was not inserted (content unchanged). Next: preview the diagram again and insert the snapshot once more.",
    ),
    (
        Key::NoticeImageReadFailed,
        "Failed to read image: {}. Next: check that the file exists and is readable, then reload the image.",
    ),
    (
        Key::NoticeInsertImageNeedsFile,
        "Save the document before inserting an image",
    ),
    (
        Key::NoticeUnsupportedImage,
        "The selected file is not a supported image format. Next: use a supported format such as PNG, JPEG, or WebP.",
    ),
    (
        Key::NoticeFootnoteDefMissing,
        "Footnote definition not found. Next: check that the footnote name matches its reference.",
    ),
    (
        Key::NoticeFootnoteRefMissing,
        "Footnote reference not found. Next: check that the footnote name matches its definition.",
    ),
    (
        Key::NoticeAnchorMissing,
        "Anchor not found. Next: check that the anchor name matches the heading.",
    ),
    (
        Key::NoticeLinkUnsupported,
        "Could not open link: unsupported link address {}. Next: use an http/https link, or copy it into a browser.",
    ),
    (
        Key::NoticeLinkFailed,
        "Could not open link: {}. Next: check the system default browser, or copy the link and open it manually.",
    ),
    (
        Key::NoticeImageMissing,
        "Image file missing; showing alt text: {}. Next: right-click to reload the image or copy the image path.",
    ),
    (Key::NoticeCodeCopied, "Code block copied"),
    (Key::NoticeCodeBlockRequired, "Cursor is not inside a code block"),
    (Key::CodeCopyButton, "Copy"),
    (Key::CodeCopiedButton, "Copied"),
    (Key::FrontMatterCollapsed, "Front Matter"),
    (Key::NoticeFocusMode, "Focus mode: {}"),
    (
        Key::NoticeRemoteImageFailed,
        "Some remote images failed to load; showing alt text. Next: check that the links are valid, or right-click to reload the images.",
    ),
    (
        Key::NoticeMermaidRenderFailed,
        "Diagram rendering failed: {}. Next: check the Mermaid syntax and preview again.",
    ),
    (
        Key::NoticeMermaidImageInserted,
        "Inserted a local image snapshot; the Mermaid source block above remains a live preview and the snapshot will not update with the source: {}",
    ),
    (Key::NoticeRemoteImagesBlocked, "Remote images blocked"),
    (
        Key::NoticeRemoteImageBlockedRetry,
        "Remote images are blocked; next: enable View → Allow Remote Images (or Settings) and retry",
    ),
    (Key::NoticeRemoteImagesAllowed, "Remote images allowed"),
    (
        Key::ConfirmAllowRemoteImagesTitle,
        "Allow Remote Images (Global)",
    ),
    (
        Key::ConfirmAllowRemoteImagesBody,
        "Once allowed, remote images in the document will contact external servers (which may reveal your network address and access time).\n\nThis is a global setting. Allow it?",
    ),
    (
        Key::NoticeMermaidSourceTooLarge,
        "Mermaid source exceeds 128 KiB; rendering skipped",
    ),
    (Key::ToggleOn, "On"),
    (Key::ToggleOff, "Off"),

    // ---- 拖放 ----
    (
        Key::DropImageNeedsDir,
        "\"{}\" cannot be inserted: save the document first",
    ),
    (Key::DropInsertImage, "Release to insert image: {}"),
    (Key::DropOpenDocument, "Release to open document: {}"),

    // ---- 打印 / 导出 ----
    (
        Key::PrintUnsupportedPdf,
        "PDF export is not supported on this platform; use Export HTML instead, or retry on a platform that supports printing.",
    ),
    (
        Key::PrintUnsupportedPrint,
        "Printing is not supported on this platform; use Export PDF instead and print from a reader.",
    ),
    (Key::ImagePreviewTitle, "Image Preview · {} · {}"),
    (
        Key::ImagePreviewHint,
        "Scroll wheel/arrow keys to pan · Ctrl+wheel or +/- to zoom · 0 to fit window · Esc or click the backdrop to close",
    ),
    (Key::ImagePreviewUnavailable, "Image cannot be displayed"),
    (Key::PrintTocTitle, "Table of Contents"),
    (Key::ExportUntitledPdf, "Untitled.pdf"),
    (Key::ExportUntitledHtml, "Untitled.html"),
    (Key::ExportUntitledMarkdown, "Untitled.md"),
    (
        Key::NoticePrintFailed,
        "Print failed: {}. Next: retry printing, or check that the printer is online and its driver is working.",
    ),
    (Key::ModalPrintFailedTitle, "Print Failed"),
    (
        Key::ModalPrintFailedBody,
        "Reason: {}\n\nNext: retry by reopening the print dialog, or check that the printer is online and its driver is working.",
    ),
    (
        Key::NoticeExportPdfFailed,
        "PDF export failed: {}. Next: retry the export, or check that the target folder is writable and the file is not in use.",
    ),
    (Key::ModalExportPdfFailedTitle, "PDF Export Failed"),
    (
        Key::ModalExportPdfFailedBody,
        "Reason: {}\n\nNext: retry the export, or cancel and check that a usable CJK font is installed, the target folder is writable, and the file is not in use; existing PDFs will not be modified.",
    ),

    // ---- 平台层（打印 / PDF） ----
    (
        Key::PrintNoPrinterContext,
        "Print failed: could not obtain a printer context. Check the printer connection and try again, or use Export PDF instead.",
    ),
    (
        Key::PrintSentPagesUnified,
        "Sent {} pages to the printer (same layout as PDF)",
    ),
    (
        Key::PrintUnifiedFallback,
        "Unified layout unavailable; falling back to screen-rendered printing: {}",
    ),
    (
        Key::PrintSentPagesScreen,
        "Sent {} pages to the printer (screen rendering)",
    ),
    (Key::PrintExportCancelled, "Export canceled"),
    (
        Key::PrintErrStartJob,
        "Print failed: could not start the print job",
    ),
    (
        Key::PrintErrInterruptedPartial,
        "Print interrupted: some pages were not output",
    ),
    (
        Key::PrintErrInterruptedNewPage,
        "Print interrupted: could not start a new page",
    ),
    (
        Key::PrintErrInterruptedEnd,
        "Print interrupted: could not end the print job",
    ),
    (Key::PrintErrBackupOld, "Could not back up the old file: {}"),
    (Key::PrintErrWriteTarget, "Could not write the target file: {}"),
    (Key::PrintErrNoCjkFont, "No usable CJK font on the system"),
    (Key::PrintErrDocEmpty, "The document is empty"),
    (Key::PrintErrNoPrintablePages, "No pages to print"),
    (Key::PrintErrNoExportPages, "No pages to export"),
    (Key::PrintErrPdfTooLarge, "PDF too large: {} MB"),
    (Key::PrintErrWriteTemp, "Failed to write temporary file: {}"),
    (
        Key::PrintErrReplaceTarget,
        "Failed to replace the target file: {}",
    ),
    (
        Key::PrintExportedPdf,
        "Exported PDF: {} ({} bytes, {} pages, {} bookmarks, {} images)",
    ),
    (Key::PrintDocumentName, "Veil Document"),

    // ---- 文件关联 / 启动对话框 ----
    (Key::AssocTitle, "Veil File Associations"),
    (Key::AssocFailedTitle, "Veil File Association Failed"),
    (
        Key::MainOpenFailedTitle,
        "Veil Could Not Open the File",
    ),
    (
        Key::AssocLocateExe,
        "Could not locate the current executable path: {}",
    ),
    (Key::AssocExeNotUtf8, "The executable path is not valid UTF-8"),
    (
        Key::AssocUnsupported,
        "File associations are not supported on this platform (Windows only)",
    ),
    (Key::AssocErrWithCode, "{}: {} (error code {})"),
    (Key::AssocOpenKeyFailed, "Failed to open the registry key"),
    (Key::AssocCreateKeyFailed, "Failed to create the registry key"),
    (Key::AssocWriteValueFailed, "Failed to write the registry value"),
    (Key::AssocReadValueFailed, "Failed to read the registry value"),
    (
        Key::AssocDeleteValueFailed,
        "Failed to delete the registry value",
    ),
    (Key::AssocDeleteKeyFailed, "Failed to delete the registry key"),
    (Key::AssocMarkdownDescription, "Markdown Document"),
    (
        Key::AssocDefaultSet,
        "Set Veil as the default app for .md",
    ),
    (
        Key::AssocDefaultOther,
        ".md already has another association, so it was left unchanged; to open .md files with Veil, choose it in Settings → Apps → Default apps",
    ),
    (
        Key::AssocIconKept,
        "A custom file icon already exists; it was left unchanged",
    ),
    (
        Key::AssocRegisterKept,
        "The following ProgID values were written by another program and were kept unchanged: {}",
    ),
    (
        Key::AssocUnregisterKept,
        "The following associations were not written by this app or have been modified, so they were kept: {}",
    ),
    (Key::AssocProgIdDescription, "ProgID Description"),
    (Key::AssocOpenCommand, "Open Command"),
    (Key::AssocAppIcon, "veil.exe Icon"),
    (Key::AssocOpenWithEntry, "\"Open with\" Entry"),
    (Key::AssocMarkdownDefault, ".md Default"),
    (
        Key::AssocManualDefault,
        "\n\nTo make Veil the default app for double-clicking .md files, choose Veil in Settings → Apps → Default apps. Windows does not allow apps to change the default app themselves.",
    ),
    (
        Key::AssocRegisteredBody,
        "Registered Markdown file associations (current user only).\n\n- Document icon embedded: shown once Veil is the default .md app\n- Right-click a .md file → \"Open with\" lists Veil",
    ),
    (
        Key::AssocUnregisteredBody,
        "Removed Veil's Markdown file associations (current user only).\n\nIf Explorer still shows the old icon, sign out and back in to refresh it.",
    ),
    // ---- 系统对话框 ----
    (
        Key::DialogFilterMarkdown,
        "Markdown (*.md;*.markdown)\0*.md;*.markdown\0All files (*.*)\0*.*\0\0",
    ),
    (
        Key::DialogFilterImage,
        "Images (*.png;*.jpg;*.jpeg;*.jfif;*.bmp;*.gif;*.webp;*.tif;*.tiff;*.ico)\0*.png;*.jpg;*.jpeg;*.jfif;*.bmp;*.gif;*.webp;*.tif;*.tiff;*.ico\0All files (*.*)\0*.*\0\0",
    ),
    (Key::DialogOpenMarkdown, "Open Markdown"),
    (Key::DialogSaveMarkdown, "Save As Markdown"),
    (Key::DialogInsertImage, "Insert Image"),
    (Key::DialogExportPdf, "Export PDF"),
    (Key::DialogExportHtml, "Export HTML"),
    (Key::DialogFilterPdf, "PDF (*.pdf)"),
    (Key::DialogFilterHtml, "HTML (*.html)"),
    (Key::DialogFilterAllFiles, "All files (*.*)"),
    (
        Key::DialogFilterMarkdownLabel,
        "Markdown (*.md;*.markdown)",
    ),
    (Key::DialogFilterMarkdownPattern, "*.md;*.markdown"),
    (Key::DialogFilterPdfPattern, "*.pdf"),
    (Key::DialogFilterHtmlPattern, "*.html"),
    (Key::DialogExportDiagnostics, "Export Diagnostics"),
    (Key::DialogFilterText, "Text files (*.txt)"),
    (Key::DialogFilterTextPattern, "*.txt"),

    // ---- 关于 ----
    (Key::AboutTitle, "About Veil"),
    (Key::AboutOpenHomepage, "Open Project Homepage"),

    // ---- 打开失败说明 ----
    (Key::OpenFailNotUtf8, "The file is not valid UTF-8 ({})"),
    (
        Key::OpenFailNotUtf8Next,
        "Save the file as UTF-8 and try again.",
    ),
    (
        Key::OpenFailNotFound,
        "The file does not exist or has been moved ({})",
    ),
    (
        Key::OpenFailNotFoundNext,
        "Check the path and try again, or choose the file again via File → Open.",
    ),
    (Key::OpenFailDenied, "No permission to read this file ({})"),
    (
        Key::OpenFailDeniedNext,
        "Make sure the file is not read-only or blocked by security software, then try again once you have permission.",
    ),
    (Key::OpenFailOther, "Read failed ({})"),
    (
        Key::OpenFailOtherNext,
        "Make sure the file is accessible and try again.",
    ),
    (
        Key::OpenFailBody,
        "Could not open: {}\n\nReason: {}\n\nNext: {}",
    ),

    // ---- 错误明细 ----
    (
        Key::ErrBitmapSizeMismatch,
        "Bitmap size mismatch: {} bytes ≠ {}×{}×4",
    ),
    (Key::ErrMathFontUnavailable, "Math font unavailable"),
    (Key::ErrPngOutputOverflow, "PNG output size overflow"),
    (Key::ErrPngBitDepth, "Unexpected PNG bit depth: {}"),
    (Key::ErrIndexedPng, "Indexed PNG was not expanded"),
    (Key::ErrIoCreateDir, "{} (creating {})"),
    (Key::ErrIoWriteFile, "{} (writing {})"),
    (Key::ErrNoConfigDir, "Could not locate the config directory"),
    (
        Key::ErrConfigNotUtf8,
        "The configuration file is not valid UTF-8",
    ),
    (Key::ErrNotUtf8, "The file is not valid UTF-8"),
    (Key::ErrNoDataDir, "Could not locate the data directory"),
    (
        Key::ErrUnsupportedImage,
        "Only PNG images are supported on this platform",
    ),
    (
        Key::ErrUrlUnsupported,
        "Opening links is not supported on this platform",
    ),
    (
        Key::ErrSnapshotDirCreate,
        "Failed to create the snapshot directory: {}",
    ),
    (Key::ErrSnapshotWrite, "Failed to write snapshot: {}"),
    (
        Key::ErrSnapshotRead,
        "Snapshot does not exist or is unreadable ({}): {}",
    ),
    (
        Key::ErrSnapshotMeta,
        "Snapshot metadata validation failed (the file is corrupted)",
    ),
    (Key::ErrSnapshotUtf8, "Snapshot is not valid UTF-8: {}"),
    (Key::ErrSnapshotDelete, "Failed to delete snapshot ({}): {}"),
    (Key::ErrHistoryClear, "Failed to clear history: {}"),
    (Key::ErrSnapshotMigrate, "Failed to migrate snapshot: {}"),
    (Key::ErrFontRead, "Failed to read font {}: {}"),
    (Key::ErrFontParse, "Failed to parse font: {}"),
    (
        Key::ErrExportDraftNoPath,
        "The document has no save location yet, so it cannot be exported",
    ),
    (Key::ErrFontSubset, "Font subsetting failed: {}"),
    (Key::ErrShellProtocol, "Unsupported link protocol: {}"),
    (Key::ErrShellExecute, "ShellExecuteW returned {}"),
    (Key::ErrWatchInit, "Failed to initialize file watching"),
    (Key::ErrWatchPath, "{} (watching {})"),
    (Key::ErrHresult, "{} failed (HRESULT 0x{})"),
    (Key::ErrImageSizeInvalid, "Invalid image size: {}×{}"),
    (
        Key::ErrImageTooLarge,
        "Image too large: {}×{} (limit 100 million pixels)",
    ),
    (
        Key::ErrPdfNoDocPath,
        "The document is not saved, so relative image paths cannot be resolved",
    ),
    (Key::ErrPdfImageEmpty, "Image has zero size"),
    (Key::ErrMathRendererUnavailable, "Math renderer unavailable"),
    (Key::ErrMathRendererInternal, "Internal math renderer error"),
    (Key::ErrMathRenderEmpty, "Math render is empty"),
    (Key::MathRenderFailedHint, "Math render failed; showing source"),
    // ---- 平台层（WIC 图片解码） ----
    (Key::WicCreateFactory, "Create WIC factory"),
    (Key::WicComInit, "COM initialization"),
    (
        Key::WicFactoryNull,
        "Creating the WIC factory returned a null pointer",
    ),
    (
        Key::WicDecoderNull,
        "Image decoder returned a null pointer",
    ),
    (Key::WicOpenDecoder, "Open image decoder"),
    (Key::WicReadFrame, "Read image frame"),
    (Key::WicFrameNull, "Image frame returned a null pointer"),
    (Key::WicCreateConverter, "Create format converter"),
    (
        Key::WicConverterNull,
        "Format converter returned a null pointer",
    ),
    (Key::WicConvertPixels, "Convert pixel format"),
    (Key::WicReadSize, "Read image size"),
    (Key::WicReadPixels, "Read image pixels"),

    // ---- 平台层（启动 / IPC / HTTP） ----
    (
        Key::IpcPathAbsolute,
        "IPC document path must be absolute",
    ),
    (Key::IpcPathUtf8, "IPC document path is not valid UTF-8"),
    (Key::IpcPathInvalid, "Invalid IPC document path"),
    (Key::IpcPathTooLong, "IPC document path is too long"),
    (
        Key::IpcConnectPrimary,
        "Could not connect to the main Veil process: {}",
    ),
    (Key::IpcUnknown, "Unknown error"),
    (
        Key::IpcUnsupported,
        "Single-instance IPC is not supported on this platform; a separate process will be used",
    ),
    (Key::HttpUrlEmpty, "URL is empty"),
    (
        Key::HttpUrlTooLong,
        "URL is too long (limit {} characters)",
    ),
    (Key::HttpUrlControl, "URL contains control characters"),
    (
        Key::HttpUrlMissingScheme,
        "URL is missing a scheme (only http/https are supported)",
    ),
    (Key::HttpUnsupportedScheme, "Unsupported scheme: {}"),
    (Key::HttpUrlMissingHost, "URL is missing a host name"),
    (
        Key::HttpUrlUserInfo,
        "URL must not contain user information",
    ),
    (
        Key::HttpIpv6MissingBracket,
        "IPv6 host is missing the closing bracket",
    ),
    (
        Key::HttpHostTrailingChars,
        "Invalid characters after the host name",
    ),
    (
        Key::HttpIpv6NeedsBrackets,
        "IPv6 hosts must be wrapped in brackets",
    ),
    (Key::HttpPortEmpty, "Port is empty"),
    (Key::HttpPortInvalid, "Invalid port: {}"),
    (Key::HttpPortOutOfRange, "Port out of range: {}"),
    (
        Key::HttpHostInvalidChars,
        "Host name contains invalid characters",
    ),
    (Key::HttpIpv6Invalid, "Invalid IPv6 address: {}"),
    (
        Key::HttpLocalhostBlocked,
        "Access to localhost is not allowed",
    ),
    (
        Key::HttpPrivateAddressBlocked,
        "Access to private/reserved addresses is not allowed: {}",
    ),
    (
        Key::HttpNumericHostBlocked,
        "Numeric/hexadecimal host names are not allowed: {}",
    ),
    (Key::HttpHostInvalid, "Invalid host name: {}"),
    (Key::HttpRedirectEmpty, "Redirect location is empty"),
    (
        Key::HttpRedirectTooMany,
        "Too many redirects (more than {})",
    ),
    (
        Key::HttpRedirectMissingLocation,
        "Redirect response is missing Location",
    ),
    (
        Key::HttpRedirectDowngrade,
        "Refused redirect from https to http",
    ),
    (Key::HttpErrWithCode, "{} failed (error code {})"),
    (Key::HttpOpOpenSession, "Open WinHTTP session"),
    (Key::HttpOpConnect, "Connect to remote host"),
    (Key::HttpOpCreateRequest, "Create HTTP request"),
    (Key::HttpOpSetTimeouts, "Set WinHTTP timeouts"),
    (Key::HttpOpConfigureRequest, "Configure HTTP request"),
    (Key::HttpOpSendRequest, "Send request"),
    (Key::HttpOpReceiveResponse, "Receive response"),
    (Key::HttpOpReadStatus, "Read response status"),
    (Key::HttpOpReadBody, "Read response body"),
    (
        Key::HttpResponseTooLarge,
        "Remote image exceeds the {} MiB limit ({} bytes)",
    ),
    (Key::HttpResponseEmpty, "Remote image response is empty"),
    (
        Key::HttpTempWriteFailed,
        "Failed to write the remote image temporary file: {}",
    ),

    // ---- 设置模态 ----
    (Key::ModalSettingsTitle, "Settings"),
    (Key::SettingsThemeMode, "Theme Mode"),
    (Key::SettingsFollowSystem, "Follow System"),
    (Key::SettingsLight, "Light"),
    (Key::SettingsDark, "Dark"),
    (Key::SettingsUiLanguage, "Interface Language"),
    (Key::SettingsLangAuto, "Automatic (Follow System)"),
    (Key::SettingsLangZhCn, "Chinese"),
    (Key::SettingsLangEnUs, "English"),
    (Key::SettingsDocFontSize, "Body Font Size (10–32)"),
    (
        Key::SettingsDocLineHeight,
        "Body Line Height (font size–3×)",
    ),
    (Key::SettingsDocFontFamily, "Body Font Family"),
    (Key::SettingsDocMargin, "Body Column"),
    (Key::SettingsMarginCompact, "Compact"),
    (Key::SettingsMarginStandard, "Standard"),
    (Key::SettingsMarginRelaxed, "Relaxed"),
    (Key::SettingsMarginFull, "Full Width"),
    (Key::SettingsMarginCustom, "Custom"),
    (
        Key::SettingsDocMarginPx,
        "Custom Body Margin (4–200, Custom Only)",
    ),
    (Key::SettingsFontSystem, "System Default"),
    (Key::SettingsFontSans, "Sans Serif"),
    (Key::SettingsFontSerif, "Serif"),
    (Key::SettingsFontMono, "Monospace"),
    (
        Key::SettingsFontSearch,
        "Search fonts (type to filter, ↑↓ to select, Enter to apply)",
    ),
    (Key::SettingsFontNoMatch, "No matching fonts"),
    (
        Key::SettingsFontMissingCjk,
        "This family lacks Chinese glyphs; body text falls back to a system font",
    ),
    (Key::SettingsAllowRemoteImages, "Allow Remote Images"),
    (Key::SettingsWindowGeom, "Last Window Geometry"),
    (Key::SettingsWindowGeomNone, "(Not Recorded)"),
    (
        Key::SettingsResetWindowGeom,
        "Clear (Resets on Next Launch)",
    ),
    (
        Key::SettingsExportPageNumbers,
        "Show Page Numbers in PDF Export",
    ),
    (
        Key::SettingsExportToc,
        "Include Table of Contents in PDF Export",
    ),
    (
        Key::SettingsExportMarginMm,
        "PDF Export Margin (mm, 5–50)",
    ),
    (Key::SettingsTabWidth, "Tab Width (2 or 4)"),
    (
        Key::SettingsSourcePlainLines,
        "Source Mode: Plain-Text Line View",
    ),
    (Key::NoticeSettingsSaved, "Settings saved"),
    (
        Key::NoticeUiLanguageRestart,
        "Interface language changed; restart to apply",
    ),
    (
        Key::NoticeSettingsSaveFailed,
        "Failed to save settings: {}. Next: check write permissions or disk space for the config directory and try again.",
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

/// 未初始化时的默认语言。
const DEFAULT_LOCALE: &str = "zh-CN";

/// 进程级运行时语言；只在启动时初始化一次（`init_locale`），之后只读。
static CURRENT_LOCALE: OnceLock<&'static str> = OnceLock::new();

/// 当前运行时语言：未初始化（库调用方/单测）时默认 zh-CN。
pub fn current_locale() -> &'static str {
    CURRENT_LOCALE.get().copied().unwrap_or(DEFAULT_LOCALE)
}

/// 初始化运行时语言。只接受已支持的语言码（`zh-CN` / `en-US`），其他值忽略；
/// 重复调用不生效（首次成功者胜），保证启动后语言稳定。
///
/// 成功后同步设置 veil-core 层 `<img>` 无 alt 时的降级占位文本——这是 UI 层
/// 唯一的「反向注入」：core 是纯策略层，不持有语言状态，需要上层在启动时
/// 把和当前 UI 语言对应的占位字符串（如 `[Image]`）传进去。OnceLock 保证两边
/// 都只设一次，不会因重试而错乱。
pub fn init_locale(code: &'static str) {
    if matches!(code, "zh-CN" | "en-US") && CURRENT_LOCALE.set(code).is_ok() {
        veil_core::set_image_alt_fallback(t(Key::ImageAltFallback).to_string());
    }
}

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
    debug_assert!(false, "i18n 缺失词条: {} ({})", key.id(), current_locale());
    key.id()
}

/// 当前语言优先；当前语言缺失词条或未配置时，按语言表顺序回退。
pub(crate) fn translate(locales: &[Locale], current: &str, key: Key) -> Option<&'static str> {
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
    translate(LOCALES, current_locale(), key).unwrap_or_else(|| missing(key))
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
        assert_eq!(t(Key::ShortcutPrint), "Ctrl+Alt+P");
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

    /// en-US 覆盖项必须非空（`Empty` 本身即空串），且 `{}` 占位符数量与 zh-CN 一致
    /// （`tf` 才能同参填充）。
    #[test]
    fn en_us_entries_keep_placeholder_parity() {
        for &(key, text) in EN_US {
            let zh = lookup(ZH_CN, key).unwrap_or_else(|| panic!("en-US 词条无 zh 对应: {key:?}"));
            if key != Key::Empty {
                assert!(!text.is_empty(), "en-US 词条不得为空: {:?}", key);
            }
            assert_eq!(
                text.matches("{}").count(),
                zh.matches("{}").count(),
                "en-US 与 zh-CN 占位符数量不一致: {}",
                key.id()
            );
        }
    }

    /// 穷举：每个 Key 都有非空 en-US 词条，占位符数量一致，且无重复/多余词条。
    #[test]
    fn every_key_has_en_us_entry_with_placeholder_parity() {
        assert_eq!(EN_US.len(), Key::ALL.len(), "en-US 词条数与 key 数不一致");
        for &key in Key::ALL {
            let en = lookup(EN_US, key)
                .unwrap_or_else(|| panic!("缺少 en-US 词条: {:?} ({})", key, key.id()));
            let zh = lookup(ZH_CN, key).expect("zh-CN 全量词条");
            if key != Key::Empty {
                assert!(!en.is_empty(), "en-US 词条不得为空: {:?}", key);
            }
            assert_eq!(
                en.matches("{}").count(),
                zh.matches("{}").count(),
                "en-US 与 zh-CN 占位符数量不一致: {}",
                key.id()
            );
            // 占位符相对位置的结构特征：首尾是否为占位符必须与 zh 一致，避免参数语义错位。
            assert_eq!(
                en.starts_with("{}"),
                zh.starts_with("{}"),
                "首部占位符位置与 zh-CN 不一致: {}",
                key.id()
            );
            assert_eq!(
                en.ends_with("{}"),
                zh.ends_with("{}"),
                "尾部占位符位置与 zh-CN 不一致: {}",
                key.id()
            );
        }
        for (i, &(key, _)) in EN_US.iter().enumerate() {
            assert!(
                !EN_US[..i].iter().any(|(other, _)| *other == key),
                "en-US 词条重复: {:?}",
                key
            );
        }
    }

    /// 英文词条不得混入中文或全角标点（`Veil` / `Markdown` 等专名不受影响）。
    #[test]
    fn en_us_entries_use_halfwidth_punctuation_only() {
        for &(key, text) in EN_US {
            for ch in text.chars() {
                let code = ch as u32;
                let is_cjk = (0x3000..=0x303F).contains(&code)
                    || (0x4E00..=0x9FFF).contains(&code)
                    || (0xFF00..=0xFFEF).contains(&code);
                assert!(!is_cjk, "en-US 词条含中文/全角字符 {ch:?}: {}", key.id());
            }
        }
    }

    /// 代表性词条抽查：菜单 / notice / dialog / settings / history / search / outline。
    #[test]
    fn representative_en_us_entries_are_translated() {
        let en = |key| translate(LOCALES, "en-US", key);
        assert_eq!(en(Key::MenuFile), Some("File"));
        assert_eq!(en(Key::MenuViewTypewriter), Some("Typewriter Mode"));
        assert_eq!(en(Key::NoticeSaved), Some("Saved: {}"));
        assert_eq!(en(Key::ModalOpenFailedTitle), Some("Open Failed"));
        assert_eq!(en(Key::SettingsThemeMode), Some("Theme Mode"));
        assert_eq!(en(Key::HistoryTitle), Some("Version History"));
        assert_eq!(en(Key::SearchPlaceholder), Some("Find"));
        assert_eq!(en(Key::OutlineTitle), Some("Outline"));
    }

    #[test]
    fn typewriter_mode_entries_are_localized() {
        assert_eq!(t(Key::MenuViewTypewriter), "打字机模式");
        assert_eq!(t(Key::NoticeTypewriterMode), "打字机模式：{}");
        assert_eq!(
            translate(LOCALES, "en-US", Key::MenuViewTypewriter),
            Some("Typewriter Mode")
        );
        assert_eq!(
            translate(LOCALES, "en-US", Key::NoticeTypewriterMode),
            Some("Typewriter mode: {}")
        );
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

    /// 运行时语言默认 zh-CN；英文覆盖只经纯函数 translate 验证，不改写全局语言
    /// （改写会让并发用例互相串扰）。
    #[test]
    fn default_runtime_locale_is_zh_cn_and_english_uses_pure_translate() {
        assert_eq!(current_locale(), "zh-CN");
        assert_eq!(t(Key::MenuFile), "文件");
        // en-US 已翻译条目：纯函数路径可见英文
        assert_eq!(
            translate(LOCALES, "en-US", Key::CtxMermaidPreview),
            Some("Open in mermaid.live (copies source)")
        );
        // en-US 全量覆盖后不再回退中文
        assert_eq!(translate(LOCALES, "en-US", Key::MenuFile), Some("File"));
        // 未知语言仍按表顺序回退 zh-CN
        assert_eq!(translate(LOCALES, "fr-FR", Key::MenuFile), Some("文件"));
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

    /// 链接 / 脚注 / 锚点提示：现状之外必须给出用户真能执行的下一步。
    #[test]
    fn link_and_reference_notices_offer_next_step() {
        let unsupported = tf(Key::NoticeLinkUnsupported, &["file:///tmp/a.md"]);
        assert!(
            unsupported.contains("不支持的链接地址") && unsupported.contains("file:///tmp/a.md"),
            "须保留现状与地址: {unsupported}"
        );
        assert!(unsupported.contains("下一步"), "须给下一步: {unsupported}");
        assert!(
            unsupported.contains("http/https") && unsupported.contains("复制到浏览器打开"),
            "须说明改用 http/https 或复制到浏览器: {unsupported}"
        );

        let failed = tf(Key::NoticeLinkFailed, &["未找到可用程序"]);
        assert!(failed.contains("未找到可用程序"), "须保留原因: {failed}");
        assert!(failed.contains("下一步"), "须给下一步: {failed}");
        assert!(
            failed.contains("默认浏览器") && failed.contains("复制链接手动打开"),
            "须说明检查默认浏览器或复制链接: {failed}"
        );

        for (key, current, hint) in [
            (Key::NoticeFootnoteDefMissing, "未找到脚注定义", "引用一致"),
            (Key::NoticeFootnoteRefMissing, "未找到脚注引用", "定义一致"),
            (Key::NoticeAnchorMissing, "未找到锚点", "标题一致"),
        ] {
            let text = t(key);
            assert!(text.contains(current), "须保留现状: {text}");
            assert!(text.contains("下一步"), "须给下一步: {text}");
            assert!(
                text.contains("检查") && text.contains(hint),
                "须说明核对方式: {text}"
            );
        }
    }

    /// 外部文件异常：说明文件现状、编辑器内容仍在，并给出另存为出口。
    #[test]
    fn external_file_notices_state_content_survives() {
        let deleted = t(Key::NoticeExternalDeleted);
        assert!(deleted.contains("已被外部删除"), "须说明现状: {deleted}");
        assert!(
            deleted.contains("内容仍在"),
            "须说明编辑器内容仍在: {deleted}"
        );
        assert!(deleted.contains("另存为"), "须给出另存为出口: {deleted}");
        assert!(
            deleted.contains("新位置"),
            "须说明可保存到新位置: {deleted}"
        );

        let unreadable = t(Key::NoticeExternalUnreadable);
        assert!(unreadable.contains("暂不可读"), "须说明现状: {unreadable}");
        assert!(
            unreadable.contains("内容仍在"),
            "须说明编辑器内容仍在: {unreadable}"
        );
        assert!(
            unreadable.contains("继续编辑") && unreadable.contains("另存为"),
            "须给出继续编辑或另存为出口: {unreadable}"
        );
    }

    /// 失败文案穷举：A 类（可恢复失败）与 D 类（致命启动错误）都必须保留现状，
    /// 并给出用户可执行的下一步；B 类（纯引导）与 C 类（成功/状态）不得混入。
    ///
    /// 清单显式列出，避免按前缀自动分类把内部诊断片段（如 `ErrIoWriteFile`，
    /// 由上层 Notice 附带下一步）误判为面向用户的失败文案。
    #[test]
    fn all_failure_notices_offer_next_step() {
        // A 类可恢复失败 + D 类致命启动错误。
        let failures: &[(Key, &str)] = &[
            (Key::NoticeWatchUnavailable, "文件监听不可用"),
            (Key::NoticeConflictNameExhausted, "冲突副本名已耗尽"),
            (Key::NoticeExternalBackupFailed, "外部版本备份失败"),
            (Key::NoticeSaveFailed, "保存失败"),
            (Key::ModalSaveFailedBody, "当前修改仍保留在编辑器中"),
            (Key::ModalExternalBackupFailBody, "外部版本备份失败"),
            (Key::NoticeRecoveryWriteFailed, "崩溃恢复写入失败"),
            (Key::NoticeExternalDeleted, "已被外部删除"),
            (Key::NoticeExternalUnreadable, "暂不可读"),
            (Key::NoticeExportFailed, "导出失败"),
            (Key::NoticeErrorWhileModalOpen, "请先关闭当前弹窗"),
            (Key::ModalExportFailedBody, "可重试导出"),
            (Key::ModalPrintFailedBody, "重试重新打开打印对话框"),
            (Key::ModalExportPdfFailedBody, "中文字体"),
            (Key::NoticeHistoryMigrationFailed, "版本历史迁移失败"),
            (Key::NoticeSnapshotFailed, "版本历史写入失败"),
            (Key::NoticeAssetsMigrationIncomplete, "未能迁移"),
            (Key::NoticeSaveAsFailed, "另存为失败"),
            (Key::ModalSaveAsFailedBody, "当前修改仍保留在编辑器中"),
            (Key::ModalOpenFailedBody, "当前文档内容未被改动"),
            (Key::NoticeSaveIncomplete, "保存未完成"),
            (Key::NoticeOpenFailed, "打开失败"),
            (Key::NoticeSnapshotReadFailed, "读取快照失败"),
            (Key::NoticeDeleteSnapshotFailed, "删除快照失败"),
            (Key::NoticeClearHistoryFailed, "清空历史失败"),
            (Key::NoticeConfigInvalid, "配置文件无效"),
            (Key::NoticeConfigNotSaved, "配置未保存"),
            (Key::ErrEventLoopCreate, "创建事件循环失败"),
            (Key::ErrEventLoopRun, "事件循环错误"),
            (Key::FatalWindowCreate, "无法创建窗口"),
            (Key::CliUnknownOption, "未知参数"),
            (Key::CliInvalidPath, "文档路径无效"),
            (Key::CliCurrentDir, "无法确定当前目录"),
            (Key::CliPathOutOfBounds, "文档路径越界"),
            (Key::NoticeImageSaveFailed, "图片保存失败"),
            (Key::NoticeMermaidSnapshotSaveFailed, "Mermaid 快照保存失败"),
            (Key::NoticeMermaidSnapshotStale, "快照未插入"),
            (Key::NoticeImageReadFailed, "图片读取失败"),
            (Key::NoticeUnsupportedImage, "不是支持的图片格式"),
            (Key::NoticeFootnoteDefMissing, "未找到脚注定义"),
            (Key::NoticeFootnoteRefMissing, "未找到脚注引用"),
            (Key::NoticeAnchorMissing, "未找到锚点"),
            (Key::NoticeLinkUnsupported, "无法打开链接"),
            (Key::NoticeLinkFailed, "无法打开链接"),
            (Key::NoticeMermaidLiveOpenFailed, "无法打开 mermaid.live"),
            (Key::NoticeImageMissing, "图片文件缺失"),
            (Key::NoticeRemoteImageFailed, "远程图片加载失败"),
            (Key::NoticeMermaidRenderFailed, "图表渲染失败"),
            (Key::NoticePrintFailed, "打印失败"),
            (Key::NoticeExportPdfFailed, "导出 PDF 失败"),
            (Key::NoticeSettingsSaveFailed, "设置保存失败"),
        ];
        for &(key, current) in failures {
            let text = t(key);
            assert!(
                text.contains(current),
                "须保留现状「{current}」({key:?}): {text}"
            );
            assert!(
                text.contains("下一步") || text.contains("重启应用"),
                "失败文案须给出可执行下一步 ({key:?}): {text}"
            );
        }

        // D 类：致命启动错误除重启外，还要说明仍失败时的排查方向。
        for key in [
            Key::ErrEventLoopCreate,
            Key::ErrEventLoopRun,
            Key::FatalWindowCreate,
        ] {
            let text = t(key);
            assert!(
                text.contains("重启应用"),
                "须建议重启应用 ({key:?}): {text}"
            );
            assert!(
                text.contains("检查图形环境") && text.contains("重新安装"),
                "须说明仍失败时的排查方向 ({key:?}): {text}"
            );
        }

        // B 类：纯引导（需先保存、光标不在代码块、全屏/缩放提示等）。
        // C 类：成功/状态（已保存、已复制、主题、字数等）。
        let guidance_or_status = [
            Key::NoticeExternalChanged,
            Key::NoticeOutputEmptyDocument,
            Key::ModalExternalChangedBody,
            Key::ModalUnsavedTitle,
            Key::ModalUnsavedBody,
            Key::NoticeSelectVersion,
            Key::NoticeSelectVersionToDelete,
            Key::NoticeRecovered,
            Key::NoticeFullscreenOn,
            Key::NoticeImagePasteNeedsFile,
            Key::NoticeMermaidSnapshotNeedsFile,
            Key::NoticeInsertImageNeedsFile,
            Key::NoticeCodeBlockRequired,
            Key::NoticeMermaidSourceTooLarge,
            Key::DropImageNeedsDir,
            Key::ErrExportDraftNoPath,
            Key::NoticeSaved,
            Key::NoticeFileRestored,
            Key::NoticeExternalReloaded,
            Key::NoticeExternalKept,
            Key::NoticeExternalDismissed,
            Key::NoticeExportCancelled,
            Key::NoticeExportedHtml,
            Key::NoticeSaveAsCancelled,
            Key::NoticeSavedAs,
            Key::NoticeCancelled,
            Key::NoticeCancelledWhat,
            Key::NoticeCancelDirty,
            Key::NoticeOpened,
            Key::NoticeNewDraft,
            Key::NoticeRestoredVersion,
            Key::NoticeDeletedSnapshot,
            Key::NoticeNoHistory,
            Key::NoticeHistoryCleared,
            Key::NoticeConfigIgnored,
            Key::NoticeTheme,
            Key::CliUsage,
            Key::NoticeFullscreenOff,
            Key::NoticeZoom,
            Key::NoticeCodeCopied,
            Key::NoticeFocusMode,
            Key::NoticeMermaidImageInserted,
            Key::NoticeRemoteImagesBlocked,
            Key::NoticeRemoteImagesAllowed,
            Key::NoticeSettingsSaved,
        ];
        for key in guidance_or_status {
            let text = t(key);
            assert!(
                !text.contains("下一步"),
                "B/C 类不应混入下一步 ({key:?}): {text}"
            );
        }
    }

    /// 双语布局预算门禁：英文词条不得宽于弹窗能提供的单行宽度。
    ///
    /// 预算来源（与 `tokens::surface`、`view/settings.rs::layout` 同源）：
    /// - 弹窗最大宽 `DIALOG_MAX_WIDTH = 760`，左右内边距 `2 × DIALOG_PADDING(20)`
    ///   → 内宽 720px；
    /// - 标签列硬顶 320px（内宽 50%），控件列 380px + 列间距 10px，合计 710 ≤ 720。
    ///
    /// 字宽估算：13px UI 字号（`typography::UI_SIZE`）下，Segoe UI 混合大小写英文的
    /// 平均前进宽度按 0.62 em 计（≈ 8.06px/字符）。系数依据：`view/settings.rs` 实测
    /// 最长英文标签「Include Table of Contents in PDF Export」（39 字符）约占 312px，
    /// 折合 8.0px/字符 ≈ 0.615em；向上取 0.62em 作保守上界，高分屏回退到更宽字体时
    /// 仍不越界。
    #[test]
    fn longest_english_strings_fit_dialog_budget() {
        use crate::tokens::surface;
        use crate::tokens::typography;

        // zh-CN / en-US 的 key 集合一致性由既有穷举用例保证：直接复用，不重复造断言。
        every_key_has_en_us_entry_with_placeholder_parity();

        const AVG_CHAR_EM: f32 = 0.62;
        const LABEL_COL_BUDGET_PX: f32 = 320.0;
        let avg_char_px = typography::UI_SIZE * AVG_CHAR_EM;
        let label_budget: usize = (LABEL_COL_BUDGET_PX / avg_char_px).floor() as usize;
        // 320 / (13 × 0.62) = 39.7 → 39 字符。预算基准变化时必须同步更新
        // docs/design/terminology.md 的「双语布局预算」一节。
        assert_eq!(label_budget, 39, "标签列字符预算基准变化，需同步文档");

        // 设置弹窗标签列的全部字段（与 view/settings.rs::SettingsForm::new 的 label 一一对应）。
        const DIALOG_LABELS: &[Key] = &[
            Key::SettingsThemeMode,
            Key::SettingsUiLanguage,
            Key::SettingsDocFontSize,
            Key::SettingsDocLineHeight,
            Key::SettingsDocFontFamily,
            Key::SettingsDocMargin,
            Key::SettingsDocMarginPx,
            Key::SettingsAllowRemoteImages,
            Key::SettingsSourcePlainLines,
            Key::SettingsWindowGeom,
            Key::SettingsExportPageNumbers,
            Key::SettingsExportToc,
            Key::SettingsExportMarginMm,
            Key::SettingsTabWidth,
        ];
        let mut widest = (0usize, Key::Empty);
        for &key in DIALOG_LABELS {
            let en = lookup(EN_US, key).expect("en-US 全量词条");
            let n = en.chars().count();
            assert!(
                n <= label_budget,
                "英文标签超出弹窗标签列预算（{} 字符 > {}）：{} = {:?}",
                n,
                label_budget,
                key.id(),
                en
            );
            if n > widest.0 {
                widest = (n, key);
            }
        }
        let widest_px = widest.0 as f32 * avg_char_px;
        assert!(
            widest_px <= LABEL_COL_BUDGET_PX,
            "最长英文标签 {} 需 {:.1}px，超出标签列 {}px",
            widest.1.id(),
            widest_px,
            LABEL_COL_BUDGET_PX
        );

        // 单行控件文案（菜单 / 右键菜单 / 按钮 / 历史动作 / 查找 / 大纲 / 开关 / 设置）
        // 允许占满弹窗内宽（720px），同样不得折行；提示与错误正文由绘制层换行，不在本预算内。
        let inner_budget: usize = ((surface::DIALOG_MAX_WIDTH - 2.0 * surface::DIALOG_PADDING)
            / avg_char_px)
            .floor() as usize;
        assert_eq!(inner_budget, 89, "内宽字符预算基准变化，需同步文档");
        const SINGLE_LINE_PREFIXES: &[&str] = &[
            "common.",
            "menu.",
            "button.",
            "ctx.",
            "history.action.",
            "search.",
            "outline.",
            "toggle.",
            "settings.",
            "disabled.",
        ];
        for &(key, en) in EN_US {
            if !SINGLE_LINE_PREFIXES
                .iter()
                .any(|prefix| key.id().starts_with(prefix))
            {
                continue;
            }
            let n = en.chars().count();
            assert!(
                n <= inner_budget,
                "英文单行控件文案超出弹窗内宽预算（{} 字符 > {}）：{} = {:?}",
                n,
                inner_budget,
                key.id(),
                en
            );
        }
    }
}
