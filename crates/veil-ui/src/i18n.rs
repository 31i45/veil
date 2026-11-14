//! 界面文案 key 化（N7/B8c）：默认 zh-CN 词条表 + `t()` 取词。
//!
//! 设计取舍：
//! - `Key` 枚举与 zh-CN 表由同一份宏生成，穷举性由构造保证（不会出现「有 key 无词条」）；
//! - 词条按语言分表（`zh-CN` / `en-US`），`t()` 按 `LOCALES` 顺序回退；本次只填 zh-CN，
//!   en-US 留空（后续补翻译只需填表，不改调用点）；
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
    MenuFileOpen => "menu.file.open", "打开";
    MenuFileQuickOpen => "menu.file.quick_open", "快速打开";
    MenuFileSave => "menu.file.save", "保存";
    MenuFileSaveAs => "menu.file.save_as", "另存为";
    MenuFileExportHtml => "menu.file.export_html", "导出 HTML";
    MenuFileExportPdf => "menu.file.export_pdf", "导出 PDF";
    MenuFilePrint => "menu.file.print", "打印";
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
    MenuEditInsertImage => "menu.edit.insert_image", "插入图片";

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
    MenuViewTypewriterMode => "menu.view.typewriter_mode", "打字机模式";
    MenuViewBlockRemoteImages => "menu.view.block_remote_images", "阻止远程图片";
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

    // ---- 窗口标题 / 状态栏 ----
    WindowTitle => "title.window", "veil — {}";
    AppTitle => "title.app", "veil";
    TitleDirtyPrefix => "title.dirty_prefix", "* ";
    TitleExternalChanged => "title.external_changed", " (外部修改)";
    TitleExternalBackedUp => "title.external_backed_up", " (外部修改已备份)";
    TitleExternalMissing => "title.external_missing", " (文件缺失)";

    StatusDirtyMark => "status.dirty_mark", " *";
    StatusExternalChanged => "status.external_changed", "（外部修改）";
    StatusExternalBackedUp => "status.external_backed_up", "（外部修改已备份）";
    StatusExternalMissing => "status.external_missing", "（文件缺失）";
    StatusSourceMode => "status.source_mode", "（源码模式）";
    StatusUnsaved => "status.unsaved", " · 未保存";
    StatusSaved => "status.saved", " · 已保存 {}";
    StatusStats => "status.stats", "{} 字 · 行 {}，列 {}";

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
    CtxInsertImage => "ctx.insert_image", "插入图片";
    CtxCopyImagePath => "ctx.copy_image_path", "复制图片路径";
    CtxReloadImage => "ctx.reload_image", "重新加载图片";
    CtxImageMissing => "ctx.image_missing", "图片文件缺失（当前显示替代文本）";
    CtxMermaidPreview => "ctx.mermaid.preview", "在 mermaid.live 预览";
    CtxMermaidCopySource => "ctx.mermaid.copy_source", "复制源码";
    CtxMermaidCopyLink => "ctx.mermaid.copy_link", "复制预览链接";
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
    NoticeWatchUnavailable => "notice.watch_unavailable", "文件监听不可用: {}";
    NoticeConflictNameExhausted => "notice.conflict_name_exhausted", "冲突副本名已耗尽";
    NoticeExternalBackupFailed => "notice.external_backup_failed", "外部版本备份失败：取消保存 {}";
    NoticeSaved => "notice.saved", "已保存：{}";
    NoticeSaveFailed => "notice.save_failed", "保存失败：{}；可 Ctrl+S 重试，或 Ctrl+Shift+S 另存为";
    ModalSaveFailedTitle => "modal.save_failed.title", "保存失败";
    ModalSaveFailedBody => "modal.save_failed.body",
        "{}\n\n原因：{}\n\n下一步：可重试本次保存，或用「另存为」写入其他位置；当前修改仍保留在编辑器中。";
    ModalExternalBackupFailBody => "modal.external_backup_fail.body",
        "{}\n\n原因：外部版本备份失败（{}）。\n\n下一步：为避免覆盖外部修改，本次保存已取消；可重试或「另存为」写入其他位置。";
    NoticeRecoveryWriteFailed => "notice.recovery_write_failed", "崩溃恢复写入失败：{}";
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
    NoticeExportedHtml => "notice.exported_html", "已导出 HTML：{}";
    NoticeExportFailed => "notice.export_failed", "导出失败：{}";
    ModalExportFailedTitle => "modal.export_failed.title", "导出失败";
    ModalExportFailedBody => "modal.export_failed.body",
        "{}\n\n原因：{}\n\n下一步：可重试导出，或取消后检查目标目录是否可写。";
    NoticeSaveAsCancelled => "notice.save_as_cancelled", "另存为已取消";
    NoticeSavedAs => "notice.saved_as", "已另存为：{}";
    NoticeHistoryMigrationFailed => "notice.history_migration_failed",
        "文档已保存到：{}；但版本历史迁移失败（{}），旧历史仍保留在临时会话目录";
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
    ModalUnsavedBody => "modal.unsaved.body", "未保存的修改在关闭后将无法恢复。";
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
    NoticeSnapshotReadFailed => "notice.snapshot_read_failed", "读取快照失败：{}";
    NoticeSelectVersion => "notice.select_version", "请先选择一个版本";
    NoticeRestoredVersion => "notice.restored_version", "已恢复该版本（Ctrl+Z 可撤销）";
    NoticeSelectVersionToDelete => "notice.select_version_to_delete", "请先选择要删除的版本";
    ModalDeleteSnapshotTitle => "modal.delete_snapshot.title", "删除版本快照";
    ModalDeleteSnapshotBody => "modal.delete_snapshot.body",
        "{}（{}）\n\n将永久删除该版本，此操作不可撤销。\n\n下一步：确认删除，或取消保留。";
    NoticeDeletedSnapshot => "notice.deleted_snapshot", "已删除该版本快照";
    NoticeDeleteSnapshotFailed => "notice.delete_snapshot_failed", "删除快照失败：{}";
    NoticeNoHistory => "notice.no_history", "暂无版本历史可清空";
    ModalClearHistoryTitle => "modal.clear_history.title", "清空版本历史";
    ModalClearHistoryBody => "modal.clear_history.body",
        "将永久删除本文档的全部 {} 份历史快照。\n\n此操作不可撤销；当前文档内容不受影响。\n\n下一步：确认清空，或取消保留。";
    NoticeHistoryCleared => "notice.history_cleared", "已清空版本历史（{} 份）";
    NoticeClearHistoryFailed => "notice.clear_history_failed", "清空历史失败：{}";

    // ---- 启动 / 配置 ----
    NoticeConfigIgnored => "notice.config_ignored", "配置有 {} 处无法识别，已忽略";
    NoticeConfigInvalid => "notice.config_invalid", "配置文件无效，已用默认主题: {}";
    NoticeRecovered => "notice.recovered", "已从崩溃恢复未保存的内容；确认后请保存";
    NoticeTheme => "notice.theme", "主题：{}";
    NoticeConfigNotSaved => "notice.config_not_saved", "配置未保存：{}";
    ErrEventLoopCreate => "error.event_loop_create", "创建事件循环失败: {}";
    ErrEventLoopRun => "error.event_loop_run", "事件循环错误: {}";

    // ---- 编辑交互提示 ----
    NoticeFullscreenOn => "notice.fullscreen_on", "全屏（F11 退出）";
    NoticeFullscreenOff => "notice.fullscreen_off", "已退出全屏";
    NoticeZoom => "notice.zoom", "缩放 {}%";
    NoticeImagePasteNeedsFile => "notice.image_paste_needs_file", "图片粘贴需要关联文件（草稿无落盘目录）";
    NoticeImageSaveFailed => "notice.image_save_failed", "图片落盘失败：{}";
    NoticeImageReadFailed => "notice.image_read_failed", "图片读取失败：{}";
    NoticeInsertImageNeedsFile => "notice.insert_image_needs_file", "插入图片需要关联文件（草稿无相对路径基准）";
    NoticeUnsupportedImage => "notice.unsupported_image", "所选文件不是支持的图片格式";
    NoticeFootnoteDefMissing => "notice.footnote_def_missing", "未找到脚注定义";
    NoticeFootnoteRefMissing => "notice.footnote_ref_missing", "未找到脚注引用";
    NoticeAnchorMissing => "notice.anchor_missing", "未找到锚点";
    NoticeLinkUnsupported => "notice.link_unsupported", "无法打开链接：不支持的链接地址 {}";
    NoticeLinkFailed => "notice.link_failed", "无法打开链接: {}";
    NoticeImageMissing => "notice.image_missing", "图片文件缺失，已显示替代文本：{}";
    NoticeFocusMode => "notice.focus_mode", "专注模式：{}";
    NoticeTypewriterMode => "notice.typewriter_mode", "打字机模式：{}";
    NoticeRemoteImageFailed => "notice.remote_image_failed", "部分远程图片加载失败，已显示替代文本";
    NoticeRemoteImagesBlocked => "notice.remote_images_blocked", "已阻止远程图片";
    NoticeRemoteImagesAllowed => "notice.remote_images_allowed", "已允许远程图片";
    ToggleOn => "toggle.on", "开";
    ToggleOff => "toggle.off", "关";

    // ---- 拖放 ----
    DropImageNeedsDir => "drop.image_needs_dir", "「{}」无法插入：草稿没有落盘目录";
    DropInsertImage => "drop.insert_image", "松开以插入图片：{}";
    DropOpenDocument => "drop.open_document", "松开以打开文档：{}";

    // ---- 打印 / 导出 ----
    PrintUnsupportedPdf => "print.unsupported_pdf", "当前平台暂不支持导出 PDF";
    PrintUnsupportedPrint => "print.unsupported_print", "当前平台暂不支持打印";
    PrintTocTitle => "print.toc_title", "目录";
    PrintPageFooter => "print.page_footer", "{} / {}";
    ExportUntitledPdf => "export.untitled_pdf", "未命名.pdf";
    ExportUntitledHtml => "export.untitled_html", "未命名.html";
    ExportUntitledMarkdown => "export.untitled_markdown", "未命名.md";
    NoticePrintFailed => "notice.print_failed", "打印失败：{}";
    ModalPrintFailedTitle => "modal.print_failed.title", "打印失败";
    ModalPrintFailedBody => "modal.print_failed.body",
        "原因：{}\n\n下一步：可重试重新打开打印对话框，或检查打印机是否联机、驱动是否正常。";
    NoticeExportPdfFailed => "notice.export_pdf_failed", "导出 PDF 失败：{}";
    ModalExportPdfFailedTitle => "modal.export_pdf_failed.title", "导出 PDF 失败";
    ModalExportPdfFailedBody => "modal.export_pdf_failed.body",
        "原因：{}\n\n下一步：可重试导出，或取消后检查目标目录是否可写、PDF 驱动是否可用。";

    // ---- 平台层（打印 / PDF） ----
    PrintNoPrinterContext => "print.no_printer_context", "打印失败：无法获取打印机上下文";
    PrintSentPagesUnified => "print.sent_pages_unified", "已发送 {} 页到打印机（与 PDF 同一版面）";
    PrintUnifiedFallback => "print.unified_fallback", "统一版面不可用，回退屏幕渲染打印：{}";
    PrintSentPagesScreen => "print.sent_pages_screen", "已发送 {} 页到打印机（屏幕渲染）";
    PrintExportCancelled => "print.export_cancelled", "导出已取消";
    PrintNativePdfFallback => "print.native_pdf_fallback", "原生 PDF 导出失败，回退位图路径：{}";
    PrintNoPdfPrinterContext => "print.no_pdf_printer_context", "导出失败：无法创建 PDF 打印机上下文";
    PrintExportedPdfBitmap => "print.exported_pdf_bitmap", "已导出 PDF（位图）：{}";
    PrintExportPdfFellBack => "print.export_pdf_fell_back", "导出 PDF 失败：已回退打印对话框";
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
    ErrExportDraftNoPath => "error.export_draft_no_path", "草稿无关联文件，导出被忽略";
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
    SettingsSmoothScroll => "settings.smooth_scroll", "平滑滚动";
    SettingsAllowRemoteImages => "settings.allow_remote_images", "允许远程图片";
    SettingsBlockRemoteImages => "settings.block_remote_images", "阻止远程图片";
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

/// 语言表：`code` 为 BCP 47 风格标签；`entries` 为空表示尚未翻译，回退到下一份表。
pub struct Locale {
    pub code: &'static str,
    pub entries: &'static [(Key, &'static str)],
}

/// en-US 预留：本次不翻译（空表 → 回退 zh-CN）。
const EN_US: &[(Key, &str)] = &[];

/// 当前支持的语言（按回退顺序）。后续新增语言只需在此追加一份非空表。
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

/// 当前语言（本轮只做 key 化，固定 zh-CN）。
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

/// 按 `{}` 占位符顺序填充参数；参数不足时保留空串，绝不 panic。
/// 模板本身来自词条表，调用点只负责提供动态值。
pub fn tf(key: Key, args: &[&str]) -> String {
    let template = t(key);
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
        assert_eq!(t(Key::MenuFilePrint), "打印");
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
        assert_eq!(t(Key::CalloutNote), "Note");
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
        assert_eq!(tf(Key::StatusSaved, &[]), " · 已保存 ");
        assert_eq!(tf(Key::MenuFile, &["多余"]), "文件");
    }
}
